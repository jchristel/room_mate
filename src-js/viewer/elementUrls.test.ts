import { describe, expect, it } from "vitest";

import { coversStorey, elementUrl, matchSuffix, visibleStoreys } from "./elementUrls.js";
import type { Level } from "../renderer/types.js";

const level = (id: string, name: string, elevation: number): Level => ({ id, name, elevation });
const scope = { projectId: "RHH", building: null, milestone: null };

describe("elementUrl", () => {
  /** The case that stops a load fetching every storey once: until the rooms
   *  payload says what levels exist, the poll must not ask at all. */
  it("is null while the storeys are unknown", () => {
    expect(elementUrl("doors", scope, null)).toBeNull();
  });

  it("scopes by storey elevation and id", () => {
    const url = elementUrl("doors", scope, [level("7", "LEVEL 1", 12.5)]);
    expect(url).toBe("/doors?project=RHH&storey_elevations=12.5&storey_level_ids=7");
  });

  /** A payload that declares no levels is the show-everything case, so the
   *  read is unscoped rather than asking for no storeys and getting nothing. */
  it("asks unscoped when the payload declares no levels", () => {
    expect(elementUrl("ceilings", scope, [])).toBe("/ceilings?project=RHH");
  });

  it("carries building and milestone", () => {
    const url = elementUrl("ffe", { projectId: "RHH", building: "B1", milestone: "Issue 3" }, []);
    expect(url).toBe("/ffe?project=RHH&building=B1&milestone=Issue+3");
  });

  it("takes a model for spaces only, which is the one entity that serves it", () => {
    expect(elementUrl("spaces", scope, [], { model: "MECH" })).toBe("/spaces?project=RHH&model=MECH");
    expect(elementUrl("ceilings", scope, [], { model: "MECH" })).toBe("/ceilings?project=RHH");
  });
});

describe("visibleStoreys", () => {
  const levels = [level("1", "L1", 0), level("2", "L2", 8)];

  it("is null with no payload, which is what defers the polls", () => {
    expect(visibleStoreys(null, ["1"])).toBeNull();
  });

  it("dedupes a level two zones both show", () => {
    expect(visibleStoreys(levels, ["1", "1"])?.map((l) => l.id)).toEqual(["1"]);
  });

  it("collects every zone's level", () => {
    expect(visibleStoreys(levels, ["2", "1"])?.map((l) => l.id)).toEqual(["2", "1"]);
  });

  it("skips a zone with no level and one the payload does not have", () => {
    expect(visibleStoreys(levels, [null, "gone"])).toEqual([]);
  });
});

describe("matchSuffix", () => {
  /** Silence on an exact match is what keeps the suffix meaningful when it
   *  does appear. */
  it("says nothing when the storey resolved exactly", () => {
    expect(matchSuffix("exact")).toBe("");
    expect(matchSuffix("none")).toBe("");
  });

  it("names a fallback, because an invisible one is the failure mode here", () => {
    expect(matchSuffix("elevation")).toBe(" (by elevation)");
    expect(matchSuffix("all")).toBe(" (all levels)");
  });
});

describe("coversStorey", () => {
  const url = (ids: string, extra = "") => `/doors?project=RHH${extra}&storey_elevations=0&storey_level_ids=${ids}`;

  it("covers a zone whose storey the held read already asked for", () => {
    expect(coversStorey(url("7,9"), url("7,9,11"), "9")).toBe(true);
  });

  it("does not cover the zone that switched to a new storey", () => {
    expect(coversStorey(url("7,9"), url("7,11"), "11")).toBe(false);
  });

  it("covers nothing before anything is held", () => {
    expect(coversStorey(null, url("7"), "7")).toBe(false);
  });

  /** A building or milestone change leaves the storeys alone and changes every
   *  answer, so a held payload for the old scope must not vouch for the new. */
  it("covers nothing when the scope outside the storeys moved", () => {
    expect(coversStorey(url("7"), url("7", "&building=B1"), "7")).toBe(false);
  });

  it("treats an unscoped held read as covering every storey", () => {
    expect(coversStorey("/doors?project=RHH", url("7"), "7")).toBe(true);
  });
});
