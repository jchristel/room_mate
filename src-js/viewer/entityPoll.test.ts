import { describe, expect, it } from "vitest";

import { EntityPoll, conditionalHeaders, pollInOrder, repaints } from "./entityPoll.js";
import type { PollFetch, PollOutcome } from "./entityPoll.js";

/** One scripted server answer. `bodyFails` is a response whose headers arrived
 *  and whose body did not — the truncated-transfer case. */
interface Reply {
  status: number;
  etag?: string;
  body?: unknown;
  bodyFails?: boolean;
  networkFails?: boolean;
}

/** A server that answers from a script and records the headers each request
 *  sent. Duplicated rather than shared with another test file, per the house
 *  rule on per-module helpers. */
function scriptedServer(replies: Reply[]) {
  const sent: Record<string, string>[] = [];
  const fetch: PollFetch = async (_url, init) => {
    sent.push(init.headers);
    const reply = replies.shift();
    if (!reply) throw new Error("test asked the server more often than it scripted");
    if (reply.networkFails) throw new TypeError("Failed to fetch");
    return {
      status: reply.status,
      ok: reply.status >= 200 && reply.status < 300,
      headers: { get: (name: string) => (name === "ETag" ? (reply.etag ?? null) : null) },
      json: async () => {
        if (reply.bodyFails) throw new SyntaxError("Unexpected end of JSON input");
        return reply.body;
      },
    };
  };
  return { fetch, sent };
}

const doors = (revision: string, ids: string[] = ["d1"]) => ({ revision, doors: ids.map((id) => ({ id })) });

describe("EntityPoll", () => {
  it("fetches unconditionally first, then sends back the tag it was given", async () => {
    const server = scriptedServer([
      { status: 200, etag: '"t1"', body: doors("r1") },
      { status: 304, etag: '"t1"' },
    ]);
    const poll = new EntityPoll({ url: () => "/doors", fetch: server.fetch });

    expect(await poll.poll()).toBe("changed");
    expect(poll.payload).toEqual(doors("r1"));
    expect(poll.fetchState).toBe("loaded");

    expect(await poll.poll()).toBe("not-modified");
    expect(server.sent).toEqual([{}, { "If-None-Match": '"t1"' }]);
    expect(poll.payload).toEqual(doors("r1"));
  });

  it("does not repaint a body carrying the revision it already holds", async () => {
    const first = doors("r1");
    const server = scriptedServer([
      { status: 200, etag: '"t1"', body: first },
      // A new tag with the same content: the ETag is deliberately more
      // conservative than the revision, so this is an ordinary answer.
      { status: 200, etag: '"t2"', body: doors("r1") },
      { status: 304 },
    ]);
    const poll = new EntityPoll({ url: () => "/doors", fetch: server.fetch });

    await poll.poll();
    expect(await poll.poll()).toBe("unchanged");
    expect(poll.payload).toBe(first);
    await poll.poll();
    expect(server.sent[2]).toEqual({ "If-None-Match": '"t2"' });
  });

  it("clears a held payload on 204, and stops sending its tag", async () => {
    const server = scriptedServer([
      { status: 200, etag: '"t1"', body: doors("r1") },
      { status: 204 },
      { status: 204 },
    ]);
    const poll = new EntityPoll({ url: () => "/doors", fetch: server.fetch });

    await poll.poll();
    expect(await poll.poll()).toBe("cleared");
    expect(poll.payload).toBeNull();
    expect(poll.fetchState).toBe("empty");

    // Nothing held any more, so a second 204 has nothing to repaint.
    expect(await poll.poll()).toBe("empty");
    expect(server.sent[2]).toEqual({});
  });

  it("keeps the payload and its tag through a failing status", async () => {
    const server = scriptedServer([
      { status: 200, etag: '"t1"', body: doors("r1") },
      { status: 500 },
      { status: 304 },
    ]);
    const poll = new EntityPoll({ url: () => "/doors", fetch: server.fetch });

    await poll.poll();
    expect(await poll.poll()).toBe("error");
    expect(poll.fetchState).toBe("error");
    expect(poll.payload).toEqual(doors("r1"));

    // The held payload is still current, and a 304 is what says so -- the state
    // must not keep reading "error" until the next push.
    expect(await poll.poll()).toBe("not-modified");
    expect(server.sent[2]).toEqual({ "If-None-Match": '"t1"' });
    expect(poll.fetchState).toBe("loaded");
  });

  it("does not keep the tag of a body that failed to arrive", async () => {
    // The stall this pins: with the tag stored before the parse, every later
    // poll sent it, got a 304, and the layer never loaded until the next push.
    const server = scriptedServer([
      { status: 200, etag: '"t1"', bodyFails: true },
      { status: 200, etag: '"t1"', body: doors("r1") },
    ]);
    const poll = new EntityPoll({ url: () => "/ffe", fetch: server.fetch });

    expect(await poll.poll()).toBe("error");
    expect(poll.payload).toBeNull();
    expect(poll.fetchState).toBe("error");

    expect(await poll.poll()).toBe("changed");
    expect(server.sent[1]).toEqual({});
    expect(poll.payload).toEqual(doors("r1"));
  });

  it("reports a network failure as an error rather than throwing", async () => {
    const server = scriptedServer([{ status: 0, networkFails: true }]);
    const poll = new EntityPoll({ url: () => "/doors", fetch: server.fetch });

    expect(await poll.poll()).toBe("error");
    expect(poll.fetchState).toBe("error");
  });

  it("starts pending, which is not the same as empty", () => {
    const poll = new EntityPoll({ url: () => "/doors", fetch: scriptedServer([]).fetch });
    expect(poll.fetchState).toBe("pending");
    expect(poll.payload).toBeNull();
  });

  it("does not ask at all while disabled", async () => {
    let on = false;
    const server = scriptedServer([{ status: 200, etag: '"t1"', body: { revision: "s1", spaces: [] } }]);
    const poll = new EntityPoll({ url: () => "/spaces", enabled: () => on, fetch: server.fetch });

    expect(await poll.poll()).toBe("skipped");
    expect(server.sent).toHaveLength(0);

    on = true;
    expect(await poll.poll()).toBe("changed");
  });

  it("refetches and repaints unchanged content after invalidate, keeping the payload meanwhile", async () => {
    const server = scriptedServer([
      { status: 200, etag: '"t1"', body: doors("r1") },
      { status: 200, etag: '"t1"', body: doors("r1") },
    ]);
    const poll = new EntityPoll({ url: () => "/spaces", fetch: server.fetch });

    await poll.poll();
    poll.invalidate();
    expect(poll.payload).toEqual(doors("r1"));
    expect(await poll.poll()).toBe("changed");
    expect(server.sent[1]).toEqual({});
  });

  it("hands each new payload to onPayload once, and never an unchanged one", async () => {
    const seen: unknown[] = [];
    const server = scriptedServer([
      { status: 200, etag: '"t1"', body: doors("r1") },
      { status: 200, etag: '"t2"', body: doors("r1") },
      { status: 200, etag: '"t3"', body: doors("r2") },
    ]);
    const poll = new EntityPoll({ url: () => "/doors", fetch: server.fetch, onPayload: (p) => seen.push(p) });

    await poll.poll();
    await poll.poll();
    await poll.poll();
    expect(seen).toEqual([doors("r1"), doors("r2")]);
  });

  it("falls back to the whole body when an older server sends no revision", async () => {
    const server = scriptedServer([
      { status: 200, body: { doors: [{ id: "d1" }] } },
      { status: 200, body: { doors: [{ id: "d1" }] } },
      { status: 200, body: { doors: [{ id: "d2" }] } },
    ]);
    const poll = new EntityPoll({ url: () => "/doors", fetch: server.fetch });

    expect(await poll.poll()).toBe("changed");
    expect(await poll.poll()).toBe("unchanged");
    expect(await poll.poll()).toBe("changed");
  });

  it("asks for the URL afresh on every poll", async () => {
    let scope = "A";
    const urls: string[] = [];
    const server = scriptedServer([{ status: 204 }, { status: 204 }]);
    const poll = new EntityPoll({
      url: () => `/doors?project=${scope}`,
      fetch: (url, init) => {
        urls.push(url);
        return server.fetch(url, init);
      },
    });

    await poll.poll();
    scope = "B";
    await poll.poll();
    expect(urls).toEqual(["/doors?project=A", "/doors?project=B"]);
  });
});

describe("repaints", () => {
  it("is true exactly when what the plan draws changed", () => {
    const all: PollOutcome[] = ["skipped", "not-modified", "unchanged", "changed", "cleared", "empty", "error"];
    expect(all.filter(repaints)).toEqual(["changed", "cleared"]);
  });
});

describe("pollInOrder", () => {
  const stub = (outcome: PollOutcome, log: string[], name: string) => ({
    poll: async () => {
      log.push(name);
      return outcome;
    },
  });

  it("asks every layer even when the earlier ones report nothing", async () => {
    // The regression: the page's quiet-tick path asked doors alone and dropped
    // the answer, so windows, FF&E and spaces went unasked.
    const log: string[] = [];
    const changed = await pollInOrder([
      stub("not-modified", log, "doors"),
      stub("not-modified", log, "windows"),
      stub("changed", log, "ffe"),
      stub("skipped", log, "spaces"),
    ]);
    expect(log).toEqual(["doors", "windows", "ffe", "spaces"]);
    expect(changed).toBe(true);
  });

  it("is false when no layer changed", async () => {
    const log: string[] = [];
    expect(await pollInOrder([stub("unchanged", log, "a"), stub("error", log, "b")])).toBe(false);
  });

  it("asks one at a time, never concurrently", async () => {
    let inFlight = 0;
    let peak = 0;
    const slow = {
      poll: async (): Promise<PollOutcome> => {
        inFlight += 1;
        peak = Math.max(peak, inFlight);
        await new Promise((resolve) => setTimeout(resolve, 1));
        inFlight -= 1;
        return "unchanged";
      },
    };
    await pollInOrder([slow, slow, slow]);
    expect(peak).toBe(1);
  });
});

describe("conditionalHeaders", () => {
  it("sends If-None-Match only when there is a tag", () => {
    expect(conditionalHeaders(null)).toEqual({});
    expect(conditionalHeaders('"abc"')).toEqual({ "If-None-Match": '"abc"' });
  });
});
