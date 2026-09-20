import { describe, expect, it } from "vitest";

import { buildingLabel, keepBuilding, keepMilestone, resolveProject, roomsUrl } from "./scope.js";

describe("roomsUrl", () => {
  it("is unscoped with nothing selected", () => {
    expect(roomsUrl({ projectId: null, building: null, milestone: null })).toBe("/rooms");
  });

  it("carries project, building and milestone", () => {
    const url = roomsUrl({ projectId: "House A", building: "B1", milestone: "Issue 3" });
    expect(url).toBe("/rooms?project=House+A&building=B1&milestone=Issue+3");
  });

  /** A milestone without a project has nothing to resolve against, so it is
   *  dropped rather than sent — the old page's rule, kept. */
  it("drops a milestone with no project", () => {
    expect(roomsUrl({ projectId: null, building: null, milestone: "Issue 3" })).toBe("/rooms");
  });
});

describe("resolveProject", () => {
  const projects = [{ id: "House A" }, { id: "RHH" }];

  it("keeps a selection the server still lists", () => {
    expect(resolveProject(projects, "RHH", "House A", true)).toBe("RHH");
  });

  it("takes the seed on the first resolve", () => {
    expect(resolveProject(projects, null, "RHH", false)).toBe("RHH");
  });

  /** A deep link to a project the server no longer serves must not leave the
   *  page empty and unexplained. */
  it("falls through to the first project when the seed is gone", () => {
    expect(resolveProject(projects, null, "deleted", false)).toBe("House A");
  });

  it("ignores the seed once seeding has happened", () => {
    expect(resolveProject(projects, null, "RHH", true)).toBe("House A");
  });

  it("is null when the server lists nothing", () => {
    expect(resolveProject([], "House A", "House A", false)).toBeNull();
  });
});

describe("keepBuilding / keepMilestone", () => {
  it("keeps a selection the list still offers", () => {
    expect(keepBuilding([{ key: "B1" }], "B1")).toBe("B1");
    expect(keepMilestone([{ name: "Issue 3" }], "Issue 3")).toBe("Issue 3");
  });

  it("drops one the list no longer offers", () => {
    expect(keepBuilding([{ key: "B2" }], "B1")).toBeNull();
    expect(keepMilestone([{ name: "Issue 4" }], "Issue 3")).toBeNull();
  });

  it("never picks one on its own", () => {
    expect(keepBuilding([{ key: "B1" }], null)).toBeNull();
    expect(keepMilestone([{ name: "Issue 3" }], null)).toBeNull();
  });
});

describe("buildingLabel", () => {
  it("names the building", () => {
    expect(buildingLabel({ key: "B1", name: "Ward Block" })).toBe("Ward Block");
    expect(buildingLabel({ key: "B1", code: "WB" })).toBe("WB");
    expect(buildingLabel({ key: "B1" })).toBe("B1");
  });

  it("disambiguates with the code, which is when the name stopped being an answer", () => {
    expect(buildingLabel({ key: "B1", name: "Ward", code: "WB", ambiguous: true })).toBe("Ward (WB)");
  });

  it("names the server's no-building bucket", () => {
    expect(buildingLabel({ key: "", unclassified: true, name: "ignored" })).toBe("Unclassified");
  });
});
