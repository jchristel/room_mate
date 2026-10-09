// The reads and writes of a project's connections document, shared by the editor
// bar, the route bar and the saved-routes layer. The record is one document, so one
// pair of calls and one failure shape: the server's own sentence on a refusal,
// which is worth showing as it is (a stale base, an id two models share).

import type { ConnectionsDoc, DocBody } from "../connections.js";

export type Reply = { ok: true; body: ConnectionsDoc } | { ok: false; status: number; message: string };

export async function request(url: string, init?: RequestInit): Promise<Reply> {
  try {
    const res = await fetch(url, { cache: "no-store", ...init });
    if (!res.ok) return { ok: false, status: res.status, message: (await res.text()).trim() || `${url} -> ${res.status}` };
    return { ok: true, body: (await res.json()) as ConnectionsDoc };
  } catch (err) {
    return { ok: false, status: 0, message: `Could not reach ${url}: ${String(err)}` };
  }
}

/** Replace the document with `body`, naming the version it was read at. A 409 means
 *  somebody saved since, and the reply says so. */
export function put(url: string, base: string, body: DocBody): Promise<Reply> {
  return request(url, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ base, ...body }),
  });
}
