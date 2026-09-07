// Which of an entity's elements belong on the storey the viewer is showing.
//
// **One answer, because four entities have to agree on it.** Doors, windows,
// FF&E and spaces each used to carry their own version, and the two versions
// that existed disagreed: the openings matched a raw `level_id` against the
// level picker's id, while `/spaces` matched on elevation. The id comparison is
// the wrong one and had been quietly losing whole models -- see `sameStorey`
// below for what it costs. This module is the spaces answer, generalised,
// tightened, and given the tests the inline versions never had.
//
// **Nothing here draws.** It is a pure filter over already-fetched payloads, so
// a level switch pays a scan and no fetch, which is what the viewer's poll loop
// assumes.

import type { Level } from "./types.js";

/** How far apart two levels' elevations may be and still be one storey, in
 *  millimetres.
 *
 *  Matches `LEVEL_EPS_MM` in `service::room_locator` and
 *  `rooms::elevation_match`'s rounding, so the plan, the geometric resolver and
 *  the server's own level dedup all mean the same thing by "the same storey".
 *  Real drift is a few parts in 1e11 (a level authored at 74000 arrives as
 *  73999.99999999999 in one file and 74000.0 in another); the tolerance is this
 *  wide because a *modelled* level is sometimes a millimetre or two out and
 *  nobody would call that a different floor. */
export const LEVEL_EPS_MM = 50;

/** The minimum an element needs to be placed on a storey. */
export interface Placed {
  model_id?: string;
  level_id?: string;
}

/** How a level list was resolved, so a filter that could not do the precise
 *  thing can say so rather than looking confident. */
export type StoreyMatch =
  /** Name and elevation both agreed -- the level dedup's own rule. */
  | "exact"
  /** Elevation agreed and no name did, so the elevation alone was used. */
  | "elevation"
  /** Nothing could be resolved; every element is being shown. */
  | "all";

export interface StoreyResult<T> {
  kept: T[];
  match: StoreyMatch;
}

/** Fold the whitespace and case out of a level name before comparing.
 *
 *  Linked models are authored by different people in different offices, and
 *  `"LEVEL 1"` / `"Level 1"` / `"LEVEL  1"` are one floor in every one of them.
 *  The server's `dedup_levels` compares raw, which is stricter than this; being
 *  looser here can only merge storeys the picker already showed separately, and
 *  never splits one it showed as whole. */
function normaliseName(name: string): string {
  return name.split(/\s+/).join(" ").trim().toLowerCase();
}

/**
 * Is `candidate` -- a level as its OWN model declares it -- the same storey as
 * the displayed level `target`?
 *
 * **Not an id comparison, and that is the whole point of this module.** A
 * `Level.id` is a Revit `ElementId`, unique only inside one document. The level
 * picker is built from `/rooms`, which has already merged its linked models'
 * levels onto one canonical id per storey (`rooms::dedup_levels`). An element's
 * `level_id` is neither of those things -- it is the raw id its own model used.
 * Comparing the two directly is wrong in two ways that both fail silently:
 *
 *   - a model whose id lost the dedup race has EVERY element dropped, on every
 *     level (RHH: 5 doors on LEVEL 9, 1358 items on GROUND);
 *   - a model that pushed no rooms contributes no canonical id at all, so none
 *     of its elements can ever match (RHH's facade package: 78 external doors,
 *     invisible everywhere, which is the report this module was written for).
 *
 * **Name AND elevation, because either alone is wrong on real data.** Elevation
 * alone merges storeys that genuinely share one -- RHH's car park stacks "C 00"
 * at the hospital's "GROUND" elevation, and matching on the number alone pulls
 * 74 car-park spaces onto hospital floors. Name alone merges a "LEVEL 1" in one
 * building with an unrelated "LEVEL 1" 40 m higher in another. Requiring both is
 * exactly `dedup_levels`' rule, which is what makes an element land on the
 * picker entry its own storey produced.
 */
export function sameStorey(candidate: Level, target: Level): boolean {
  return (
    normaliseName(candidate.name) === normaliseName(target.name) &&
    Math.abs(candidate.elevation - target.elevation) <= LEVEL_EPS_MM
  );
}

/**
 * The elements standing on one storey, resolved through each model's OWN level
 * list.
 *
 * `levelsByModel` is the `levels_by_model` map every element read now carries
 * (`service::entity_scope::levels_by_model`); `target` is the displayed level,
 * off the `/rooms` payload the picker was built from.
 *
 * **Degrades in announced steps rather than silently emptying the layer**, which
 * is the failure this whole area keeps producing -- a reader who switches a
 * layer on and sees nothing concludes the data is missing, and is wrong. So:
 *
 *   1. `exact` -- some element's own level agrees on name and elevation. The
 *      answer, and the overwhelmingly common case.
 *   2. `elevation` -- the contributing models share **no level name at all**
 *      with the displayed list, so the two sides genuinely speak different
 *      vocabularies ("L01" against "LEVEL 1") and elevation alone is the best
 *      available answer. Reported, because it is a guess.
 *
 *      **A property of the whole payload, not of one storey**, which is why
 *      `levels` (the full displayed list) is a parameter. Deciding it per
 *      storey -- "nothing matched here, try elevation" -- duplicates floors on
 *      real data, twice over on RHH: its interior model carries a decoy
 *      "LEVEL 6" mis-elevated onto LEVEL 4's height with no doors on it, and
 *      its car park has level names no FF&E-bearing model uses. Both would
 *      answer with the neighbouring storey's contents. Once the vocabularies
 *      are known to overlap, a storey with no exact match is EMPTY, and empty
 *      is an answer.
 *   3. `all` -- no element resolved to a level at all, which is a payload whose
 *      models pushed no levels (every probe capture, and any snapshot older
 *      than `levels_by_model`). Show everything and say so.
 *
 * An element whose own model does not declare its `level_id` is unresolvable and
 * is not kept -- unless nothing else resolved either, which is case 3. FF&E is
 * where this is visible: duHast writes `-1` for an item it found no solid
 * geometry to measure, and RHH has 842 of them. They were already invisible
 * before this module (`"-1"` matched no picker id either); what changes is that
 * they no longer count as evidence that the level data is missing.
 *
 * **A model that declares NO levels falls back to comparing ids**, which is
 * what this module replaced and is still the only thing available for such a
 * model. Snapshots predate `levels` on the element envelopes -- RHH's car park
 * pushed its 122 doors in August with none -- and those ids do resolve against
 * the picker whenever that model also pushed rooms, which is how they were
 * drawn before. Dropping them to enforce the better rule would fix the facade
 * by breaking the car park. The distinction is per model and deliberate: a
 * model that declared its levels and still cannot place an element has an
 * unplaceable element, not a missing level list.
 */
export function onStorey<T extends Placed>(
  elements: readonly T[],
  levelsByModel: Record<string, Level[] | undefined> | undefined,
  target: Level | null,
  levels: readonly Level[] = [],
): StoreyResult<T> {
  const all = elements.slice();
  if (!elements.length) return { kept: all, match: "exact" };
  if (!target) return { kept: all, match: "all" };

  // model id -> level id -> that model's own Level. Built once per call; the
  // alternative is a linear scan of one model's levels per element, and RHH's
  // services models declare ~40 levels each against 38k items.
  const byModel = new Map<string, Map<string, Level>>();
  for (const [modelId, levels] of Object.entries(levelsByModel ?? {})) {
    const map = new Map<string, Level>();
    for (const level of levels ?? []) map.set(level.id, level);
    byModel.set(modelId, map);
  }

  // Whether the payload carries level information AT ALL. This, and not "did
  // anything match", is what separates "we cannot filter" from "we filtered and
  // this storey is empty" -- the distinction the show-everything fallback turns
  // on, and the one that is easy to get wrong: an element pointing at a level id
  // its own model does not declare is corrupt data, not absent level data, and
  // must not tip the whole layer into showing every storey.
  const haveLevels = [...byModel.values()].some((levels) => levels.size > 0);

  // Do the contributing models and the displayed level list share ANY storey
  // name? See step 2: this is what separates "the disciplines spell storeys
  // differently" -- where elevation is the only bridge -- from "this storey is
  // simply empty". Computed against the whole displayed list rather than the one
  // target, because it is a fact about the two vocabularies, not about a floor.
  const displayed = new Set(levels.map((l) => normaliseName(l.name)));
  const vocabulariesOverlap =
    displayed.size > 0 &&
    [...byModel.values()].some((own) => [...own.values()].some((l) => displayed.has(normaliseName(l.name))));

  const exact: T[] = [];
  const loose: T[] = [];
  for (const element of elements) {
    const declared = byModel.get(element.model_id ?? "");
    if (!declared || declared.size === 0) {
      // This model declared no levels, so elevation is not available for it and
      // the id is all there is. See the doc comment: the id resolves whenever
      // the model also pushed rooms, which is the pre-`levels_by_model` world.
      if ((element.level_id ?? "") === target.id) {
        exact.push(element);
        loose.push(element);
      }
      continue;
    }
    const own = declared.get(element.level_id ?? "");
    if (!own) continue;
    if (Math.abs(own.elevation - target.elevation) > LEVEL_EPS_MM) continue;
    loose.push(element);
    if (normaliseName(own.name) === normaliseName(target.name)) exact.push(element);
  }

  if (exact.length) return { kept: exact, match: "exact" };
  if (!vocabulariesOverlap && loose.length) return { kept: loose, match: "elevation" };
  // No level information anywhere and no id hit either: there is nothing to
  // filter on. Drawing every storey at once is wrong, but it is VISIBLY wrong,
  // and the caller announces it -- where drawing nothing reads as "the data is
  // missing" and is indistinguishable from a genuinely empty floor.
  if (!haveLevels) return { kept: all, match: "all" };
  // Level information exists and none of it puts an element here. An empty floor
  // is a real answer -- GS.0 has no doors -- and must not fall back to showing all.
  return { kept: [], match: "exact" };
}
