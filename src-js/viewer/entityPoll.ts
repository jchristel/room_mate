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
  init: { cache: RequestCache; headers: Record<string, string> },
) => Promise<PollResponse>;

export interface EntityPollOptions<P> {
  /** The request URL for the page's CURRENT scope, asked afresh every poll. */
  url: () => string;
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
   * Neither needs clearing on a scope change: the tag is computed over the scope
   * it was issued for, so a tag from the old scope cannot match and the server
   * answers 200. `invalidate` exists for a caller that wants to say so anyway.
   */
  private etag: string | null = null;
  private revision: string | null = null;

  constructor(private readonly options: EntityPollOptions<P>) {}

  /**
   * Ask once. Never throws: a layer that fails must not take the rooms, or any
   * other layer, down with it — a model with rooms and no furniture is
   * legitimate and the plan is still worth drawing.
   */
  async poll(): Promise<PollOutcome> {
    const { enabled, onPayload, url } = this.options;
    if (enabled && !enabled()) return "skipped";
    const doFetch = this.options.fetch ?? ((u, init) => fetch(u, init));
    try {
      const res = await doFetch(url(), { cache: "no-store", headers: conditionalHeaders(this.etag) });
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
      if (incoming === this.revision) return "unchanged";
      this.revision = incoming;
      this.payload = payload;
      onPayload?.(payload);
      return "changed";
    } catch {
      // The rooms read already reports connection loss; one message is enough.
      this.fetchState = "error";
      return "error";
    }
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
