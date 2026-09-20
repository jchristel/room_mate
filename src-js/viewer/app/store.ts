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
import type { ElementEntity } from "../elementUrls.js";
import type { ViewerAppearance } from "./appearance.js";
import type { Scope } from "../scope.js";

/** What the page is doing, in the words the old page's zone meta used. Not an
 *  enum of HTTP states: "waiting for data" (a 204 — the project exists and
 *  holds no rooms) and "connection lost" are different answers to the reader,
 *  and both are ordinary rather than errors. */
export type Status = "starting" | "ready" | "waiting for data" | "connection lost";

/** One zone, as far as RENDERING it goes. The view rect is deliberately not
 *  here — see `zoneRegistry.ts`: it changes on every pointer move, and holding
 *  it in the store would re-render the page per frame to update a number only
 *  the renderer reads. */
export interface ZoneRow {
  id: string;
  /** The level this zone shows. `null` until a payload says what there is. */
  levelId: string | null;
}

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
  /** The zones on screen, left to right. Never empty: the page always has one,
   *  which is why `removeZone` stops at one rather than at zero. */
  zones: readonly ZoneRow[];
  /** Pan and zoom one zone, move them all. Page state, not per zone: it is a
   *  property of the strip rather than of any one panel. */
  linkViews: boolean;
  /** Which element layers are drawn. Page state like the toggles above: the
   *  question "are doors shown" has one answer for the page, and two zones
   *  disagreeing about it would make a comparison between them meaningless. */
  layers: Readonly<Record<ElementEntity, boolean>>;
  /** The rooms themselves, and their labels. Rooms default ON, and this is the
   *  only toggle that hides the layer every other one is drawn over: the
   *  overlays are read AGAINST the rooms, and sometimes that is the problem —
   *  a ceiling ring sits inches inside the room outline beneath it. */
  showRooms: boolean;
  showLabels: boolean;
  /** Which services model's spaces to draw; "" is all of them. RHH keeps one
   *  services file per service, so "all" stacks four near-identical outlines on
   *  every room in one colour — which is why the picker exists. */
  spacesModel: string;
  /** Every model that has spaces, learned from an unscoped payload. */
  spacesModels: readonly string[];
  /** Bumped whenever a layer's payload changes.
   *
   *  The element payloads live on their polls, not in this store -- they are
   *  large and only the paint reads them -- so nothing would tell React that a
   *  doors push landed. This counter is that signal, and it is what the paint
   *  effect depends on. Without it the plan drew rooms and no elements until
   *  something else happened to repaint, which is exactly the bug the old
   *  page's `redrawAllZones()` on a changed element poll exists to prevent. */
  layersRevision: number;
  /** The project's `[appearance]` block, or `{}` — which is the ordinary state
   *  and means "every layer follows the theme". */
  appearance: ViewerAppearance;
}

const initial: ViewerState = {
  scope: { projectId: null, building: null, milestone: null },
  projects: [],
  buildings: [],
  milestones: [],
  payload: null,
  status: "starting",
  updatedAt: null,
  zones: [{ id: "zone-0", levelId: null }],
  linkViews: false,
  layers: { doors: true, windows: true, ffe: true, spaces: false, ceilings: false, floors: false },
  showRooms: true,
  showLabels: true,
  spacesModel: "",
  spacesModels: [],
  layersRevision: 0,
  appearance: {},
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

// ---- zones ------------------------------------------------------------------
//
// A hard ceiling on open zones, and it exists for WebGL specifically: browsers
// cap live contexts (commonly ~16) and silently kill the OLDEST past the limit,
// so an unbounded "+ zone" would blank an earlier zone with no error and
// nothing a reader could act on. 8 leaves headroom for the adjacency canvas and
// for other tabs on the same GPU.
export const MAX_ZONES = 8;

let zoneSeq = 1;

export function addZone(): void {
  const { zones } = state;
  if (zones.length >= MAX_ZONES) return;
  // The new zone starts on the level the LAST one shows, so "+ zone" opens a
  // copy of what is on screen and the reader changes one of them -- rather than
  // jumping to the lowest level and making them find their way back.
  const levelId = zones.length ? zones[zones.length - 1]!.levelId : null;
  setState({ zones: [...zones, { id: `zone-${zoneSeq++}`, levelId }] });
}

export function removeZone(): void {
  const { zones } = state;
  if (zones.length <= 1) return;
  setState({ zones: zones.slice(0, -1) });
}

export function setZoneLevel(id: string, levelId: string | null): void {
  setState({ zones: state.zones.map((z) => (z.id === id ? { ...z, levelId } : z)) });
}

export function setLinkViews(on: boolean): void {
  setState({ linkViews: on });
}

// ---- layers -----------------------------------------------------------------

export function setLayer(entity: ElementEntity, on: boolean): void {
  setState({ layers: { ...state.layers, [entity]: on } });
}

export function setShowRooms(on: boolean): void {
  setState({ showRooms: on });
}

export function setShowLabels(on: boolean): void {
  setState({ showLabels: on });
}

export function setSpacesModel(model: string): void {
  setState({ spacesModel: model });
}

/** Tell the page a layer's data moved. See `layersRevision`. */
export function bumpLayers(): void {
  setState({ layersRevision: state.layersRevision + 1 });
}
