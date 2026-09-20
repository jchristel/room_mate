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
import type { ColourPlan } from "../colour.js";
import type { ElementKind } from "../../renderer/seam.js";
import type { FilterState } from "../properties.js";
import type { AreasData } from "../areas.js";
import type { ValidationReport } from "../validation.js";
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
  /** This zone's footprint overlay: on/off and which tier. Per zone because
   *  both are presentation — two zones showing one level at two tiers is a
   *  comparison the overlay exists to make. */
  areasMode: boolean;
  areasTier: number;
  /** The level this zone shows. `null` until a payload says what there is. */
  levelId: string | null;
  /** The colour plan applied to THIS zone, by name, or null for none.
   *
   *  Per zone because it is presentation: two zones showing one level under two
   *  plans is the comparison the picker exists for. The plans themselves are
   *  page state below — they are a property of the project. */
  colourPlan: string | null;
  /** Which element layers THIS zone draws. Per zone since C1, where the page
   *  had one set for all of them: two zones on one storey, one showing
   *  ceilings and one not, is the comparison that makes possible. The READS
   *  stay scope-wide — a layer is fetched while any zone shows it — so this is
   *  a drawing decision, never a cost control. */
  layers: Readonly<Record<ElementEntity, boolean>>;
  /** The rooms themselves, and their labels. Rooms default ON, and that is the
   *  only toggle that hides the layer every other one is drawn over: the
   *  overlays are read AGAINST the rooms, and sometimes the rooms are what is
   *  in the way. */
  showRooms: boolean;
  showLabels: boolean;
  /** Which services model's spaces this zone draws; `""` is all of them.
   *
   *  **Presentation, not a scope.** The read is unscoped and the filtering
   *  happens at paint, because two zones may choose differently and one
   *  request cannot serve both. The default was already unscoped, so nothing
   *  fetches more than it did. */
  spacesModel: string;
}

/**
 * What is selected, page-wide.
 *
 * PAGE state, not zone state, and that is the decision the old page made and
 * this keeps: one room can legitimately appear in two zones showing the same
 * level, so the mark is applied in EVERY zone that draws it, while `zoneId`
 * records where the click came from. That id is a LABEL the panel shows, never
 * focus machinery — selection is explicit, never inferred from where the reader
 * last interacted.
 *
 * `kind` is what makes this more than an id: ids are unique only within an
 * entity, so a door and a room may share one.
 */
/** What can be selected: every kind the plan can pick, plus `area` — a
 *  hierarchy FOOTPRINT, which is page geometry drawn over the plan rather than
 *  an element the renderer knows about. */
export type SelectionKind = ElementKind | "area";

export interface Selection {
  kind: SelectionKind;
  id: string;
  zoneId: string | null;
}

export interface ViewerState {
  scope: Scope;
  selection: Selection | null;
  /** The inspector's property filters. Page state, and deliberately kept
   *  ACROSS selection changes: the common use is comparing one field over
   *  several rooms by clicking each in turn. Not persisted across reloads,
   *  like every other view preference here. */
  inspector: FilterState;
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
  /** Every model that has spaces, learned from an unscoped payload. */
  spacesModels: readonly string[];
  /** The room search: one query, one field set, one match set for the page.
   *
   *  `matches` is `null` for NO QUERY and an empty set for a query that matched
   *  nothing — opposite states, and only the second dims the plan. `seen`
   *  records which fields the picker has already offered, so a field arriving
   *  with a new project can default ON without re-ticking one a reader
   *  deliberately turned off. */
  search: {
    query: string;
    fields: ReadonlySet<string>;
    seen: ReadonlySet<string>;
    matches: ReadonlySet<string> | null;
    active: boolean;
  };
  /** The dissolved footprints for the scope, fetched when some zone first
   *  switches its overlay on. One dataset serves every zone's overlay AND the
   *  band's figures: it is scope-derived, so a second copy per zone could only
   *  disagree with itself. */
  areas: AreasData | null;
  /** The tier the BAND's figures are at. Separate from a zone's overlay tier
   *  on purpose: the overlay answers "what does this floor look like by
   *  department", the figures "what do the departments total across the job". */
  areasBandTier: number;
  /** The QA report for the scope, or null before one has been read. */
  validation: ValidationReport | null;
  /** Whether flagged rooms are marked on the plan. Follows the QA block's
   *  expansion rather than the report: a reader opens QA to look at the
   *  flagged rooms, and a plan lighting up on a background refresh would be a
   *  change nobody asked for. */
  showErrors: boolean;
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
  /** The project's colour plans, fetched with its appearance from the one
   *  settings read. */
  colourPlans: readonly ColourPlan[];
}

const initial: ViewerState = {
  scope: { projectId: null, building: null, milestone: null },
  selection: null,
  inspector: { filter: "", hideEmpty: true },
  projects: [],
  buildings: [],
  milestones: [],
  payload: null,
  status: "starting",
  updatedAt: null,
  zones: [newZone("zone-0")],
  linkViews: false,
  search: { query: "", fields: new Set(), seen: new Set(), matches: null, active: false },
  areas: null,
  areasBandTier: 0,
  validation: null,
  showErrors: false,
  spacesModels: [],
  layersRevision: 0,
  appearance: {},
  colourPlans: [],
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

/** A zone's starting presentation, or a COPY of another zone's.
 *
 *  The three overlay layers start off, for the reason they are not polled while
 *  off: each covers the rooms it sits on, so a reader switches one on to ask a
 *  specific question. */
function newZone(id: string, from?: ZoneRow): ZoneRow {
  return {
    id,
    levelId: from?.levelId ?? null,
    colourPlan: from?.colourPlan ?? null,
    areasMode: from?.areasMode ?? false,
    areasTier: from?.areasTier ?? 0,
    layers: from
      ? { ...from.layers }
      : { doors: true, windows: true, ffe: true, spaces: false, ceilings: false, floors: false },
    showRooms: from?.showRooms ?? true,
    showLabels: from?.showLabels ?? true,
    spacesModel: from?.spacesModel ?? "",
  };
}

export function addZone(): void {
  const { zones } = state;
  if (zones.length >= MAX_ZONES) return;
  // The new zone starts on the level the LAST one shows, so "+ zone" opens a
  // copy of what is on screen and the reader changes one of them -- rather than
  // jumping to the lowest level and making them find their way back.
  // A new zone opens as a COPY of the last one -- same level, same layers --
  // so "+ zone" gives a reader something to change rather than a blank panel
  // they have to configure back to what they were looking at.
  setState({ zones: [...zones, newZone(`zone-${zoneSeq++}`, zones[zones.length - 1])] });
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

// ---- layers, per zone -------------------------------------------------------

/** Change one zone's visibility. `layers` is merged, so a caller names only the
 *  entity it is toggling. */
export function setZoneLayer(
  id: string,
  patch: { layers?: Partial<Record<ElementEntity, boolean>>; showRooms?: boolean; showLabels?: boolean },
): void {
  setState({
    zones: state.zones.map((z) =>
      z.id === id
        ? {
            ...z,
            ...(patch.showRooms === undefined ? {} : { showRooms: patch.showRooms }),
            ...(patch.showLabels === undefined ? {} : { showLabels: patch.showLabels }),
            ...(patch.layers ? { layers: { ...z.layers, ...patch.layers } } : {}),
          }
        : z,
    ),
  });
}

export function setZoneSpacesModel(id: string, model: string): void {
  setState({ zones: state.zones.map((z) => (z.id === id ? { ...z, spacesModel: model } : z)) });
}

/** Whether ANY zone draws this layer — what decides whether it is fetched. The
 *  read is scope-wide, so it cannot be per zone. */
export function layerWanted(entity: ElementEntity): boolean {
  return state.zones.some((z) => z.layers[entity]);
}

/** Tell the page a layer's data moved. See `layersRevision`. */
export function bumpLayers(): void {
  setState({ layersRevision: state.layersRevision + 1 });
}

/** Apply a colour plan to one zone, by name. `null` is no colour. */
export function setZoneColourPlan(id: string, plan: string | null): void {
  setState({ zones: state.zones.map((z) => (z.id === id ? { ...z, colourPlan: plan } : z)) });
}

// ---- selection --------------------------------------------------------------

/** Select one element, or nothing. `zoneId` is where the click came from, and
 *  is null for a selection made anywhere else (the grid, a search result). */
export function select(kind: SelectionKind, id: string, zoneId: string | null = null): void {
  setState({ selection: { kind, id, zoneId } });
}

export function clearSelection(): void {
  if (state.selection) setState({ selection: null });
}

export function setInspectorFilter(patch: Partial<FilterState>): void {
  setState({ inspector: { ...state.inspector, ...patch } });
}

/** Update the search, from a function of what it was — the field picker reads
 *  the previous set to add a newly discovered field without re-ticking one a
 *  reader turned off. */
export function setSearch(patch: (prev: ViewerState["search"]) => Partial<ViewerState["search"]>): void {
  setState({ search: { ...state.search, ...patch(state.search) } });
}

export function setValidation(report: ValidationReport | null): void {
  setState({ validation: report });
}

export function setShowErrors(on: boolean): void {
  setState({ showErrors: on });
}

export function setAreas(data: AreasData | null): void {
  setState({ areas: data });
}

export function setAreasBandTier(depth: number): void {
  setState({ areasBandTier: depth });
}

export function setZoneAreas(id: string, patch: { areasMode?: boolean; areasTier?: number }): void {
  setState({ zones: state.zones.map((z) => (z.id === id ? { ...z, ...patch } : z)) });
}
