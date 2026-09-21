// The six element layers: their polls, their toggles, and which of their
// elements stand on the storey a zone is showing.
//
// **One table, six layers.** Exactly five things vary per layer — the entity it
// reads, what its toggle is called, whether it starts on, whether it is polled
// while switched off, and (for spaces) a model picker — and nothing else does.
// The old page had six hand-written `EntityPoll`s and six `…OnLevel` functions
// beside them; four of them had already drifted once, which is what the shared
// `EntityPoll` module was extracted to stop.
//
// **Three layers are not polled while off**, and that is not symmetry for its
// own sake: spaces, ceilings and floors each overlay the rooms they sit on, so
// each starts off and a reader switches it on to answer a specific question.
// Polling a hidden overlay would cost every viewer of every project a read —
// `/ceilings` is 9 MB on RHH — for a question most of them are not asking.

import { EntityPoll, onStorey } from "./planRenderer.js";
import { coversStorey, elementUrl, matchSuffix, visibleStoreys, type ElementEntity } from "../elementUrls.js";
import { bumpLayers, getState, layerWanted, markLayerLoading, setState, type ZoneRow } from "./store.js";
import type { Ceiling, Door, Floor, Item, Level, Space, WindowOpening } from "../../renderer/types.js";

/** How a layer's storey match resolved, so its toggle can say when the answer
 *  was a guess. */
export type StoreyMatch = "exact" | "elevation" | "all" | "none";

export interface LayerSpec {
  entity: ElementEntity;
  /** The key its elements arrive under, which is the entity name for all six. */
  label: string;
  /** Whether the layer starts on. The three overlays do not. */
  defaultOn: boolean;
  /** Whether it is polled while switched off. Only the glyph layers are. */
  pollWhenOff: boolean;
}

export const LAYERS: readonly LayerSpec[] = [
  { entity: "doors", label: "Doors", defaultOn: true, pollWhenOff: true },
  { entity: "windows", label: "Windows", defaultOn: true, pollWhenOff: true },
  { entity: "ffe", label: "FF&E", defaultOn: true, pollWhenOff: true },
  { entity: "spaces", label: "Spaces", defaultOn: false, pollWhenOff: false },
  { entity: "ceilings", label: "Ceilings", defaultOn: false, pollWhenOff: false },
  { entity: "floors", label: "Floors", defaultOn: false, pollWhenOff: false },
];

/** What each entity's list holds. The paint request types every layer
 *  separately, so this map is what keeps `elementsOnStorey` honest instead of
 *  a cast at each of the six call sites. */
export interface ElementOf {
  doors: Door;
  windows: WindowOpening;
  ffe: Item;
  spaces: Space;
  ceilings: Ceiling;
  floors: Floor;
}

export interface ElementPayload {
  [key: string]: unknown;
  levels_by_model?: unknown;
}

/** The storeys every zone is showing, from the current state. */
function storeysOnScreen(): Level[] | null {
  const { payload, zones } = getState();
  return visibleStoreys(
    payload?.levels ?? null,
    zones.map((z) => z.levelId),
  );
}

const polls = new Map<ElementEntity, InstanceType<typeof EntityPoll<ElementPayload>>>();

for (const layer of LAYERS) {
  polls.set(
    layer.entity,
    new EntityPoll<ElementPayload>({
      // **Spaces are read UNSCOPED**, with no `?model=`, since C1 made the
      // model a per-ZONE choice: two zones may pick different services models
      // and one request cannot serve both, so the filtering moved to the paint.
      // Nothing fetches more than it did — the picker's default was already
      // "all models".
      url: () => elementUrl(layer.entity, getState().scope, storeysOnScreen()),
      // ANY zone showing it is what makes a layer worth fetching. The read is
      // scope-wide, so it cannot be per zone — a menu is a drawing decision,
      // never a cost control.
      enabled: () => layer.pollWhenOff || layerWanted(layer.entity),
      ...(layer.entity === "spaces"
        ? { onPayload: (payload: ElementPayload) => refreshSpacesModels(payload) }
        : {}),
    }),
  );
}

/** Every model that has spaces, learned from the payload's `phase_by_model` —
 *  which lists exactly the models that contributed. The picker exists because
 *  RHH keeps one services file per service, so "all" stacks four
 *  near-identical outlines on every room in one colour.
 *
 *  Unconditional since C1: the read is always unscoped now, so the list can
 *  never be narrowed to the one model a zone happens to be showing. */
function refreshSpacesModels(payload: ElementPayload): void {
  const { scope } = getState();
  const byProject = (payload["phase_by_model"] ?? {}) as Record<string, Record<string, unknown>>;
  const models = Object.keys(byProject[scope.projectId ?? ""] ?? {}).sort();
  if (models.length) setState({ spacesModels: models });
}

/** Poll every layer once. Returns whether any of them changed what is drawn. */
export async function pollLayers(): Promise<boolean> {
  // Every layer that will read a new scope is marked BEFORE the first await:
  // the polls run in series, and a zone drawing only FF&E is waiting while the
  // doors read is still going.
  for (const [entity, poll] of polls) markLayerLoading(entity, poll.newScopeUrl());
  let changed = false;
  for (const [entity, poll] of polls) {
    const outcome = await poll.poll();
    if (outcome === "changed" || outcome === "cleared") changed = true;
    // Still wanting a new scope after its poll means the scope moved while it
    // was in flight, so the next read is the one to wait for. A failure is not
    // loading: the bar would otherwise run forever against a dead layer.
    markLayerLoading(entity, outcome === "error" ? null : poll.newScopeUrl());
  }
  // One bump for the tick, not one per layer: a tick that moved three layers
  // should repaint once.
  if (changed) bumpLayers();
  return changed;
}

/** One layer's elements on one storey, and how that match resolved.
 *
 * The join is `onStorey`'s — NAME plus ELEVATION, never a raw level id. A
 * `Level.id` is per document, so comparing ids directly drops every element in
 * a model that pushes no rooms: RHH's facade package has 78 external doors, and
 * they were invisible on every level. */
export function elementsOnStorey<E extends ElementEntity>(
  entity: E,
  levelId: string | null,
): { kept: readonly ElementOf[E][]; match: StoreyMatch } {
  return storeyElements(entity, polls.get(entity)?.payload ?? null, levelId);
}

/**
 * The same join over ANY payload of the layer, not just the poll's.
 *
 * The SVG export needs it for storeys no zone is showing: the polls hold only
 * the storeys on screen, so the export fetches each level itself and must then
 * keep exactly what the screen would. One function for both is what makes an
 * exported level agree with the same level drawn.
 */
export function storeyElements<E extends ElementEntity>(
  entity: E,
  payload: ElementPayload | null,
  levelId: string | null,
): { kept: readonly ElementOf[E][]; match: StoreyMatch } {
  const { payload: rooms } = getState();
  if (!payload || !rooms) return { kept: [], match: "none" };
  const levels = rooms.levels ?? [];
  const level = levels.find((l) => String(l.id) === String(levelId)) ?? null;
  const result = onStorey(
    (payload[entity] ?? []) as ElementOf[E][],
    payload.levels_by_model,
    level,
    // The whole displayed level list, not just the target: whether elevation
    // alone can be trusted is a fact about the two vocabularies, not about one
    // level.
    levels,
  );
  return { kept: result.kept, match: result.match as StoreyMatch };
}

/** A layer's menu text: its name, plus how its storey resolved for THIS zone.
 *  A layer that fell back to elevation, or to every level, says so where the
 *  reader switches it on. */
export function layerToggleLabel(layer: LayerSpec, levelId: string | null): string {
  return layer.label + matchSuffix(elementsOnStorey(layer.entity, levelId).match);
}

/** One layer's raw payload, for the console handle. See `main.tsx`. */
export function layerPayload(entity: ElementEntity): unknown {
  return polls.get(entity)?.payload ?? null;
}

/** One element by id, with the payload it came from — which the type-property
 *  lookup needs, since a row indexes its OWN response only. */
export function findElement<E extends ElementEntity>(
  entity: E,
  id: string,
): { element: ElementOf[E]; payload: ElementPayload } | null {
  const payload = polls.get(entity)?.payload;
  const list = (payload?.[entity] ?? []) as ElementOf[E][];
  const element = list.find((e) => (e as { id: string }).id === id);
  return element && payload ? { element, payload } : null;
}

/**
 * An element's family-type properties.
 *
 * An element read sends each distinct bag ONCE, in `type_property_sets`, and
 * each element names its row in `type_properties_ref` — the same bag on 1,000
 * chairs used to arrive 1,000 times, which on RHH was most of a 293 MB `/ffe`
 * body. The row indexes THIS payload only, so it is always resolved against the
 * payload the element came from; an element with no type properties names no
 * row.
 */
export function typePropertiesOf(
  payload: ElementPayload,
  element: { type_properties_ref?: number | null },
): Record<string, { value?: string } | undefined> {
  const row = element.type_properties_ref;
  const sets = payload["type_property_sets"] as Record<string, unknown>[] | undefined;
  if (row == null || !sets) return {};
  return (sets[row] ?? {}) as Record<string, { value?: string } | undefined>;
}

/** What a layer's last read SAID, which a null payload cannot tell you: not
 *  asked yet, a 204 meaning the scope holds none, or a failure. The room
 *  contents panel keeps them apart, because "this room has no doors" and "the
 *  doors have not arrived" are opposite answers and only one is a finding. */
export function layerState(entity: ElementEntity): "pending" | "loaded" | "empty" | "error" {
  return polls.get(entity)?.fetchState ?? "pending";
}

/**
 * Whether one zone is waiting on an element layer it DRAWS.
 *
 * Per zone, though the reads are shared: a read in flight counts only if what
 * the layer holds does not already cover this zone's storey (`coversStorey`),
 * so one zone switching level does not light every zone's bar. A layer that
 * has never answered counts too -- an overlay switched on is not asked until
 * the next tick, and the reader should see it coming from the click.
 */
export function zoneAwaitsLayers(zone: ZoneRow, levelId: string | null): boolean {
  const { loadingLayers, payload } = getState();
  return LAYERS.some(({ entity }) => {
    if (!zone.layers[entity]) return false;
    const poll = polls.get(entity);
    if (!poll) return false;
    const target = loadingLayers[entity];
    if (target) return !coversStorey(poll.acceptedUrl, target, levelId);
    return payload !== null && poll.neverAnswered() && poll.newScopeUrl() !== null;
  });
}
