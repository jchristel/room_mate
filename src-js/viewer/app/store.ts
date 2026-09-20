// The page's state, in one place, with subscribers.
//
// **Deliberately not a library, and deliberately not React state.** What the
// old page has is module-scope globals plus explicit listeners
// (`selectionListeners`), and the port keeps that shape: the poll loop, the
// renderer host and the pickers all read and write the same values, and most
// of them are not inside a component. This is that, typed, with one
// `useSyncExternalStore` at the edge (`useViewer`).
//
// One snapshot object, replaced whole on every change, so React's identity
// check is the change check. The state is small — scope, three server lists,
// one payload reference and a status string — and the payload is never copied,
// only pointed at.

import type { BuildingRow, MilestoneRow, ProjectRow, RoomsPayload } from "./api.js";
import type { Scope } from "../scope.js";

/** What the page is doing, in the words the old page's zone meta used. Not an
 *  enum of HTTP states: "waiting for data" (a 204 — the project exists and
 *  holds no rooms) and "connection lost" are different answers to the reader,
 *  and both are ordinary rather than errors. */
export type Status = "starting" | "ready" | "waiting for data" | "connection lost";

export interface ViewerState {
  scope: Scope;
  projects: readonly ProjectRow[];
  buildings: readonly BuildingRow[];
  milestones: readonly MilestoneRow[];
  /** The one `/rooms` payload every zone will render from (B3). */
  payload: RoomsPayload | null;
  status: Status;
  /** When the last changed payload landed, for the meta line. */
  updatedAt: Date | null;
}

const initial: ViewerState = {
  scope: { projectId: null, building: null, milestone: null },
  projects: [],
  buildings: [],
  milestones: [],
  payload: null,
  status: "starting",
  updatedAt: null,
};

let state: ViewerState = initial;
const listeners = new Set<() => void>();

export function getState(): ViewerState {
  return state;
}

/** Merge a patch and notify. A no-op patch still notifies, which is cheap and
 *  keeps callers from having to prove a change; the snapshot identity is what
 *  React compares, and a redundant render of a header is not worth a guard. */
export function setState(patch: Partial<ViewerState>): void {
  state = { ...state, ...patch };
  for (const listener of listeners) listener();
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Test seam: the poll loop and the pickers are the only writers, and a test
 *  that ran either needs the page back as it found it. */
export function resetState(): void {
  state = initial;
  for (const listener of listeners) listener();
}
