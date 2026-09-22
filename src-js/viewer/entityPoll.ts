// One element entity's poll: the conditional request, the two cursors it keeps,
// and what the last read actually said.
//
// **Extracted because the copies had already drifted, not to tidy them.**
// `index.html` carried this exchange four times — doors, windows, FF&E, spaces —
// and `poll()` carried a second, shorter path for the tick where the rooms read
// answers 304. That path called `pollDoors` alone and threw its answer away: it
// was written when doors were the only other entity and never learned about the
// three that followed. So on a quiet project — every tick between rooms pushes —
// windows, FF&E and spaces were not asked again, and a doors push was fetched
// but not repainted until something else happened to redraw. Five copies of one
// exchange is exactly the "same state in several places, drifting" signal
// STRATEGY-BROWSER.md names; this module is the one copy.
//
// **What stays in the page, and why.** The URL builders read the page's scope
// globals and are injected; the payloads' *consumers* (storey matching, the room
// contents panel) are page code and read `payload`. And the ROOMS read is not an
// `EntityPoll`: its payload is `currentPayload`, owned by `ingestAll`, and its
// failures are the page's failures ("connection lost", "waiting for data")
// rather than one layer's. Folding it in would give the rooms two names that can
// disagree after a 204, which is the drift this file exists to remove.
//
// No DOM here. A change to what one layer does with its payload happens through
// `onPayload`, so the exchange itself is testable without a page.

/** What the last read of this entity SAID — which a null payload cannot tell
 *  you. Three different things leave `payload` null: a poll that has not run, a
 *  204 saying the scope holds none, and a read that failed. The room contents
 *  panel keeps them apart, because "this room has no doors" and "the doors have
 *  not arrived" are opposite answers and only one of them is a finding. */
export type FetchState = "pending" | "loaded" | "empty" | "error";

/**
 * What one `poll()` did. Finer than the boolean the page repaints on, because
 * the distinctions are cheap to keep and the tests are what pin them:
 *
 * - `skipped` — the layer is not being polled (spaces while hidden).
 * - `not-modified` — 304; the server built no body.
 * - `unchanged` — 200 carrying the revision already held.
 * - `changed` — 200 carrying a new revision; `payload` replaced.
 * - `cleared` — 204 while a payload was held; `payload` is now null.
 * - `empty` — 204 with nothing held; nothing to repaint.
 * - `error` — a non-2xx/304 status, a network failure, or a body that would not
 *   parse. `payload` is left as it was.
 */
export type PollOutcome = "skipped" | "not-modified" | "unchanged" | "changed" | "cleared" | "empty" | "error";

/** The subset of `Response` a poll reads. Structural, so the real `fetch`
 *  satisfies it and a test needs no `Response` global. */
export interface PollResponse {
  readonly status: number;
  readonly ok: boolean;
  readonly headers: { get(name: string): string | null };
  json(): Promise<unknown>;
}

export type PollFetch = (
  url: string,
  init: { cache: RequestCache; headers: Record<string, string>; signal?: AbortSignal },
) => Promise<PollResponse>;

export interface EntityPollOptions<P> {
  /** The request URL for the page's CURRENT scope, asked afresh every poll.
   *  `null` when the scope cannot be known yet -- the element layers before the
   *  rooms say which storeys are showing -- which skips the poll rather than
   *  asking for everything. */
  url: () => string | null;
  /** `false` skips the poll entirely. Spaces is the one layer that uses it: it
   *  overlays the rooms it sits on, so it starts off and does not cost every
   *  viewer a read for a question most of them are not asking. */
  enabled?: () => boolean;
  /** Called once per accepted new payload, never on `unchanged`. */
  onPayload?: (payload: P) => void;
  /** Injected by tests. Defaults to the global `fetch`. */
  fetch?: PollFetch;
}

/**
 * `If-None-Match` for a poll, or no header at all when there is no tag.
 *
 * Sent by hand with `cache: "no-store"` rather than left to `cache: "no-cache"`
 * to revalidate, and the difference matters: the browser cache would answer a
 * 304 with its stored 200, so the page would re-parse 73 MB of doors JSON to
 * learn that nothing had changed. Setting the header explicitly is what lets the
 * 304 reach the caller as a 304, with no body and no parse.
 */
export function conditionalHeaders(etag: string | null): Record<string, string> {
  return etag ? { "If-None-Match": etag } : {};
}

/** Whether an outcome changed what the plan should draw. */
export function repaints(outcome: PollOutcome): boolean {
  return outcome === "changed" || outcome === "cleared";
}

/** The server's one-value content revision, or — from a server too old to send
 *  one — the whole body stringified, so the viewer still updates against it. */
function revisionOf(payload: unknown): string {
  const revision = payload !== null && typeof payload === "object" ? (payload as { revision?: unknown }).revision : null;
  return revision != null ? String(revision) : JSON.stringify(payload);
}

export class EntityPoll<P> {
  /** The last accepted body, or null — see `fetchState` for which null. */
  payload: P | null = null;
  fetchState: FetchState = "pending";

  /**
   * Two cursors, and they answer different questions, so they must not be
   * collapsed into one. `revision` rides in the body and says whether what we
   * hold differs from what we drew — it decides whether to repaint. The ETag is
   * sent back as `If-None-Match` and lets the SERVER decide not to build a body
   * at all; it is deliberately more conservative than the revision (see
   * `service::scope_cursor`).
   *
   * **Both are only meaningful for the URL that produced them**, which is why
   * `acceptedUrl` is kept beside them. The tag is safe across a scope change --
   * it hashes the scope, so an old one cannot match -- but the revision is NOT:
   * it hashes which snapshots contributed, and `?building=` or a storey switch
   * leaves those unchanged while changing the body completely. Compared across
   * scopes, it answered "unchanged" to a body that was not, and the layer kept
   * drawing the previous scope. So a poll whose URL differs from the accepted
   * one is a new scope: no tag sent, and whatever arrives is accepted.
   */
  private etag: string | null = null;
  private revision: string | null = null;
  private accepted: string | null = null;
  /** The read in flight, if any, so a scope change can cancel it -- see
   *  `abortIfStale`. */
  private inFlight: { target: string; controller: AbortController } | null = null;

  constructor(private readonly options: EntityPollOptions<P>) {}

  /** The URL the held payload (or 204) answered, or null before any. */
  get acceptedUrl(): string | null {
    return this.accepted;
  }

  /** Whether what is held answers the page's CURRENT scope. False while a scope
   *  change is in flight -- the room contents panel says "not loaded yet" then
   *  rather than reporting the previous scope's answer as this one's. */
  isCurrent(): boolean {
    return this.accepted !== null && this.accepted === this.options.url();
  }

  /** The URL a poll would read if it would read a NEW scope -- nothing held
   *  for it yet -- else null. Null too while the layer is not polled at all, so
   *  a hidden overlay never reads as loading. This is the distinction the
   *  loading bar needs and `isCurrent` cannot make: a same-scope revalidation
   *  is a read in flight, and not something the reader is waiting for. */
  newScopeUrl(): string | null {
    const { enabled, url } = this.options;
    if (enabled && !enabled()) return null;
    const target = url();
    return target !== null && target !== this.accepted ? target : null;
  }

  /** Whether this layer has never answered anything -- not a payload, not a
   *  204, not a failure. A layer switched on between ticks is in this state
   *  until the next one asks. */
  neverAnswered(): boolean {
    return this.accepted === null && this.fetchState === "pending";
  }

  /**
   * Ask once. Never throws: a layer that fails must not take the rooms, or any
   * other layer, down with it — a model with rooms and no furniture is
   * legitimate and the plan is still worth drawing.
   */
  async poll(): Promise<PollOutcome> {
    const { enabled, onPayload, url } = this.options;
    if (enabled && !enabled()) return "skipped";
    const target = url();
    if (target === null) return "skipped";
    const sameScope = target === this.accepted;
    const doFetch = this.options.fetch ?? ((u, init) => fetch(u, init));
    const controller = new AbortController();
    const flight = { target, controller };
    this.inFlight = flight;
    try {
      const res = await doFetch(target, {
        cache: "no-store",
        headers: conditionalHeaders(sameScope ? this.etag : null),
        signal: controller.signal,
      });
      // The scope moved while this was in flight -- a storey switch, typically,
      // whose own poll may already have landed. Accepting this answer would put
      // the older scope back on screen, so it is dropped unread; the next poll
      // asks for the current one.
      if (url() !== target) return "skipped";
      if (res.status === 304) {
        // A tag is only ever held alongside an accepted payload (see below), so
        // a 304 means what we hold is current — including after a transient
        // error, which would otherwise leave the state reading "error" until
        // the next push.
        this.fetchState = "loaded";
        return "not-modified";
      }
      if (res.status === 204) {
        // Clear rather than keep the last set: stale doors over a new project's
        // plan is worse than none. The tag goes with the payload, because a 204
        // carries none and holding the old one would send `If-None-Match` for an
        // entity we no longer have.
        this.etag = null;
        this.fetchState = "empty";
        this.accepted = target;
        if (this.payload === null) return "empty";
        this.payload = null;
        this.revision = null;
        return "cleared";
      }
      if (!res.ok) {
        this.fetchState = "error";
        return "error";
      }
      const payload = (await res.json()) as P;
      // The tag is stored only AFTER the body parsed, and that order is the
      // fix for a stall. Stored first — as the page did — a body that failed
      // mid-transfer left us holding the tag for data we never received, so
      // every later poll got a 304 and the layer never loaded until the next
      // push. `/ffe` is 273 MB over ~94 s on RHH; a dropped body is not
      // hypothetical at that size.
      this.etag = res.headers.get("ETag");
      this.fetchState = "loaded";
      const incoming = revisionOf(payload);
      // Only a revision from the SAME scope can vouch for the body -- see the
      // cursors' doc comment.
      if (sameScope && incoming === this.revision) return "unchanged";
      this.accepted = target;
      this.revision = incoming;
      this.payload = payload;
      onPayload?.(payload);
      return "changed";
    } catch {
      // Cancelled because the scope moved (`abortIfStale`), or failed for a
      // scope nobody is looking at any more: either way the answer was never
      // going to be used, and reporting it as this layer's failure would put
      // "error" on a layer whose current read has not even been asked yet.
      if (url() !== target) return "skipped";
      // The rooms read already reports connection loss; one message is enough.
      this.fetchState = "error";
      return "error";
    } finally {
      if (this.inFlight === flight) this.inFlight = null;
    }
  }

  /**
   * Cancel the read in flight if the page no longer wants its answer.
   *
   * `poll` already drops such an answer unread; this stops the transfer and
   * the parse as well, which on RHH is up to 24 MB of FF&E for a storey the
   * reader has just left. The server may still finish building the body it
   * had started -- cancelling cannot reach into that -- but it is not sent,
   * and the next read in the lane starts at once instead of waiting for it.
   */
  abortIfStale(): void {
    const flight = this.inFlight;
    if (flight && flight.target !== this.options.url()) flight.controller.abort();
  }

  /** Drop both cursors, so the next poll fetches a body and repaints from it
   *  even if the content is unchanged. The payload is kept until then, so the
   *  plan does not flash empty between the two. */
  invalidate(): void {
    this.etag = null;
    this.revision = null;
  }
}

/**
 * Poll every layer, in order, and say whether any of them changed.
 *
 * **Every poll is asked, whatever an earlier one answered** — the property the
 * page's hand-written sequence lost. Typed structurally rather than as
 * `EntityPoll<unknown>[]` so polls of different payload types can share a list.
 *
 * Sequential rather than `Promise.all`, deliberately: each read that does build
 * a body costs the server a full assemble, and concurrent entity reads were
 * measured taking it from 245 MB to 904 MB resident. A 304 costs a header
 * exchange, so an idle tick stays cheap in series.
 */
export async function pollInOrder(polls: readonly { poll(): Promise<PollOutcome> }[]): Promise<boolean> {
  let changed = false;
  for (const p of polls) if (repaints(await p.poll())) changed = true;
  return changed;
}
