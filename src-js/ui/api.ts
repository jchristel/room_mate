// The calls every page makes the same way. Writes never throw on an HTTP error: the server's own text is shown to the
// person, verbatim.

import { apiSend } from "./common.js";

/** A write's outcome: the answer, or the server's own text for why not. */
export type Sent<T> = { ok: true; value: T } | { ok: false; error: string };

/** `/projects/{id}`, the root of every per-project route. */
export const projectBase = (project: string): string => `/projects/${encodeURIComponent(project)}`;

/** Send JSON; an HTTP error is a `Sent` with the server's text (or `HTTP <status>`), never a throw. */
export async function send<T>(method: string, url: string, body: unknown): Promise<Sent<T>> {
  const r = await apiSend(method, url, body);
  if (!r.ok) return { ok: false, error: r.text || `HTTP ${r.status}` };
  try {
    return { ok: true, value: (r.text ? JSON.parse(r.text) : null) as T };
  } catch {
    return { ok: false, error: "the server's answer was not JSON" };
  }
}

/** The text of whatever a rejected promise or a catch holds. */
export const message = (e: unknown): string => String((e as Error).message ?? e);
