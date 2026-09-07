import { describe, expect, it } from "vitest";

import { LEVEL_EPS_MM, onStorey, sameStorey } from "./storey.js";
import type { Level } from "./types.js";

const lvl = (id: string, name: string, elevation: number): Level => ({ id, name, elevation });

/** An element as the four entity reads present one: its own model, its own
 *  model's level id. Duplicated rather than shared with another test file, per
 *  the house rule on per-module helpers. */
const el = (id: string, model_id: string, level_id: string) => ({ id, model_id, level_id });

describe("sameStorey", () => {
  it("matches on name and elevation, not on id", () => {
    // The facade model's LEVEL 3 and the interior model's LEVEL 3 are one
    // storey with two ElementIds -- the case that made 78 external doors
    // invisible.
    expect(sameStorey(lvl("16667039", "LEVEL 3", 69000), lvl("8698294", "LEVEL 3", 69000))).toBe(true);
  });

  it("tolerates cross-file float drift", () => {
    expect(sameStorey(lvl("a", "LEVEL 4", 73999.99999999999), lvl("b", "LEVEL 4", 74000.0))).toBe(true);
  });

  it("separates two storeys that share an elevation", () => {
    // RHH's car park stacks C 00 at the hospital GROUND's elevation. Matching
    // on elevation alone pulls car-park elements onto hospital floors.
    expect(sameStorey(lvl("8662438", "C 00", 55500), lvl("7977087", "GROUND", 55500))).toBe(false);
  });

  it("separates two storeys that share a name", () => {
    expect(sameStorey(lvl("a", "LEVEL 1", 60000), lvl("b", "LEVEL 1", 100600))).toBe(false);
  });

  it("ignores case and internal whitespace in the name", () => {
    expect(sameStorey(lvl("a", "Level  1", 60000), lvl("b", "LEVEL 1", 60000))).toBe(true);
  });

  it("rejects an elevation just outside the tolerance", () => {
    expect(sameStorey(lvl("a", "L", 0), lvl("b", "L", LEVEL_EPS_MM + 0.001))).toBe(false);
    expect(sameStorey(lvl("a", "L", 0), lvl("b", "L", LEVEL_EPS_MM))).toBe(true);
  });
});

describe("onStorey", () => {
  const target = lvl("8698294", "LEVEL 3", 69000);

  it("keeps an element whose own model names the storey differently by id", () => {
    const levels = {
      facade: [lvl("16667039", "LEVEL 3", 69000)],
      interior: [lvl("8698294", "LEVEL 3", 69000)],
    };
    const result = onStorey([el("d1", "facade", "16667039"), el("d2", "interior", "8698294")], levels, target);
    expect(result.match).toBe("exact");
    expect(result.kept.map((e) => e.id)).toEqual(["d1", "d2"]);
  });

  it("drops an element on a different storey of the same model", () => {
    const levels = { m: [lvl("l3", "LEVEL 3", 69000), lvl("l4", "LEVEL 4", 74000)] };
    const result = onStorey([el("a", "m", "l3"), el("b", "m", "l4")], levels, target);
    expect(result.kept.map((e) => e.id)).toEqual(["a"]);
  });

  it("excludes a level that shares the elevation but not the name", () => {
    const levels = {
      hospital: [lvl("h", "LEVEL 3", 69000)],
      carpark: [lvl("c", "C 05", 69000)],
    };
    const result = onStorey([el("a", "hospital", "h"), el("b", "carpark", "c")], levels, target);
    expect(result.match).toBe("exact");
    expect(result.kept.map((e) => e.id)).toEqual(["a"]);
  });

  it("falls back to elevation when the vocabularies share no name at all", () => {
    // A discipline that spells every storey differently still draws rather than
    // vanishing -- but the caller is told the match was a guess.
    const byModel = { mep: [lvl("x", "L03", 69000)] };
    const result = onStorey([el("a", "mep", "x")], byModel, target, [target, lvl("z", "LEVEL 4", 74000)]);
    expect(result.match).toBe("elevation");
    expect(result.kept.map((e) => e.id)).toEqual(["a"]);
  });

  it("does not fall back once the vocabularies overlap anywhere", () => {
    // RHH's car park declares "C 00"/"C 05" and no model that pushed FF&E uses
    // those names -- but both sides agree on GROUND and LEVEL 3, so the names
    // are comparable and "C 05 has no furniture" is the answer. Falling back
    // here painted LEVEL 3's 2382 items onto the car park.
    const byModel = { interior: [lvl("g", "GROUND", 55500), lvl("l3", "LEVEL 3", 69000)] };
    const displayed = [lvl("g", "GROUND", 55500), lvl("l3", "LEVEL 3", 69000), lvl("c5", "C 05", 69000)];
    const result = onStorey([el("a", "interior", "l3")], byModel, lvl("c5", "C 05", 69000), displayed);
    expect(result.match).toBe("exact");
    expect(result.kept).toEqual([]);
  });

  it("does not fall back to elevation when a model declares the name and is empty", () => {
    // RHH's interior model carries a decoy "LEVEL 6" mis-elevated to LEVEL 4's
    // height and puts no doors on it. Falling back would answer with LEVEL 4's
    // doors and duplicate a whole storey.
    const decoy = lvl("51029993", "LEVEL 6", 74000);
    const real = lvl("8698299", "LEVEL 4", 74000);
    const result = onStorey([el("a", "m", "8698299")], { m: [real, decoy] }, decoy, [real, decoy]);
    expect(result.match).toBe("exact");
    expect(result.kept).toEqual([]);
  });

  it("shows everything when no element resolves to a level at all", () => {
    // A probe capture, or any snapshot older than levels_by_model.
    const result = onStorey([el("a", "m", "l1"), el("b", "m", "l2")], {}, target);
    expect(result.match).toBe("all");
    expect(result.kept).toHaveLength(2);
  });

  it("returns an empty storey rather than falling back to everything", () => {
    // GS.0 has no doors. That is an answer, not missing data.
    const levels = { m: [lvl("l4", "LEVEL 4", 74000)] };
    const result = onStorey([el("a", "m", "l4")], levels, target);
    expect(result.match).toBe("exact");
    expect(result.kept).toEqual([]);
  });

  it("does not let unresolvable elements trigger the show-everything fallback", () => {
    // duHast writes level -1 for an item it could not measure; RHH has 842.
    // They are unplaceable, but they are not evidence the levels are missing.
    const levels = { m: [lvl("l3", "LEVEL 3", 69000)] };
    const result = onStorey([el("a", "m", "l3"), el("b", "m", "-1")], levels, target);
    expect(result.match).toBe("exact");
    expect(result.kept.map((e) => e.id)).toEqual(["a"]);
  });

  it("falls back to the canonical id for a model that declared no levels", () => {
    // RHH's car park pushed its doors before `levels` rode the element
    // envelopes. Its ids DO resolve against the picker, because it also pushed
    // rooms — dropping them to enforce the better rule would fix the facade by
    // breaking the car park.
    const levels = { facade: [lvl("16667039", "LEVEL 3", 69000)], carpark: [] };
    const result = onStorey(
      [el("a", "facade", "16667039"), el("b", "carpark", "8698294"), el("c", "carpark", "other")],
      levels,
      target,
    );
    expect(result.match).toBe("exact");
    expect(result.kept.map((e) => e.id)).toEqual(["a", "b"]);
  });

  it("does not use the id fallback for a model that DID declare levels", () => {
    // Such a model has an unplaceable element, not a missing level list — and
    // an id from another document must never be trusted to mean this storey.
    const levels = { m: [lvl("l4", "LEVEL 4", 74000)] };
    const result = onStorey([el("a", "m", "8698294")], levels, target);
    expect(result.kept).toEqual([]);
  });

  it("shows everything when the displayed level cannot be resolved", () => {
    const result = onStorey([el("a", "m", "l1")], { m: [lvl("l1", "LEVEL 3", 69000)] }, null);
    expect(result.match).toBe("all");
    expect(result.kept).toHaveLength(1);
  });

  it("treats an empty element list as a resolved empty storey", () => {
    // No layer data is not a level-resolution failure, and must not make the
    // toggle claim a fallback.
    expect(onStorey([], {}, target)).toEqual({ kept: [], match: "exact" });
  });
});
