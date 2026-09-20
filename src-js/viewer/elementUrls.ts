// The element reads' URLs: scope, plus the storeys some zone is SHOWING.
//
// **Storey-scoped, and that is a size decision with a measurement behind it.**
// These reads used to ship every storey and let the browser discard most of it:
// on RHH, 38,913 FF&E items to draw the few hundred on the floor being looked
// at — 133 MB against 23.6 MB for one storey. The server keeps a SUPERSET of
// what the viewer's own storey rule can keep (`service::entity_scope::
// StoreyScope`), so every storey decision and every "(by elevation)" label
// comes out as it did.
//
// `null` means DO NOT ASK YET: the storeys are unknowable until a rooms payload
// says what levels exist, and asking unscoped in the meantime would fetch every
// storey exactly once per load — the cost this scoping exists to avoid.

import type { Level } from "../renderer/types.js";
import type { Scope } from "./scope.js";

/** The entities served per storey. `spaces` also takes `?model=`. */
export type ElementEntity = "doors" | "windows" | "ffe" | "spaces" | "ceilings" | "floors";

/**
 * One element read's URL, or `null` while the storeys are unknown.
 *
 * `building` scopes an element THROUGH the room it is attributed to, so a
 * homeless one — every window and external door in a facade package — matches
 * no `?building=` and vanishes while a building is selected. That is the
 * server's documented behaviour and the reason "All buildings" is the default,
 * not something to paper over here.
 */
export function elementUrl(
  entity: ElementEntity,
  scope: Scope,
  storeys: readonly Level[] | null,
  opts: { model?: string | null } = {},
): string | null {
  if (storeys === null) return null;
  const params = new URLSearchParams();
  if (scope.projectId) params.set("project", scope.projectId);
  if (scope.building) params.set("building", scope.building);
  if (scope.projectId && scope.milestone) params.set("milestone", scope.milestone);
  if (entity === "spaces" && opts.model) params.set("model", opts.model);
  // An EMPTY storey list is not "no storeys": it is a payload that declares no
  // levels at all, which the viewer's storey rule treats as show-everything. So
  // it asks unscoped rather than for nothing.
  if (storeys.length) {
    params.set("storey_elevations", storeys.map((l) => l.elevation).join(","));
    params.set("storey_level_ids", storeys.map((l) => l.id).join(","));
  }
  const qs = params.toString();
  return qs ? `/${entity}?${qs}` : `/${entity}`;
}

/**
 * The storeys on screen, deduped by id — several zones may show one.
 *
 * `null` when there is no rooms payload, which is what stops every element
 * poll asking before the levels are known.
 */
export function visibleStoreys(
  levels: readonly Level[] | null,
  zoneLevelIds: readonly (string | null)[],
): Level[] | null {
  if (!levels) return null;
  const shown = new Map<string, Level>();
  for (const id of zoneLevelIds) {
    if (!id) continue;
    const level = levels.find((l) => String(l.id) === String(id));
    if (level) shown.set(String(level.id), level);
  }
  return [...shown.values()];
}

/** How a layer's toggle reads, including a storey match that was a GUESS.
 *
 *  The suffixes are the point: a fallback nobody can see is the failure mode
 *  this area keeps producing — a reader switches a layer on, sees the wrong
 *  thing, and cannot tell the layer from the data. */
export function toggleLabel(name: string, on: boolean, match: "exact" | "elevation" | "all" | "none"): string {
  if (!on) return `${name}: off`;
  if (match === "all") return `${name}: on (all levels)`;
  if (match === "elevation") return `${name}: on (by elevation)`;
  return `${name}: on`;
}
