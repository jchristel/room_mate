import { describe, expect, it } from "vitest";
import { holeClassName, resolveRoomAppearance, roomClassName } from "./appearance.js";
import type { Room } from "./types.js";

const room = (id: string): Room => ({
  id,
  loops: [{ points: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 1, y: 1 }] }],
});

describe("resolveRoomAppearance", () => {
  it("returns no fill when no colour plan is supplied", () => {
    // null, NOT "". An empty string would emit `style="fill: "` and override
    // the stylesheet with nothing.
    expect(resolveRoomAppearance(room("a"), {}).fill).toBeNull();
  });

  it("takes the fill from the injected colour plan", () => {
    const a = resolveRoomAppearance(room("a"), { colourFor: () => "#123456" });
    expect(a.fill).toBe("#123456");
  });

  it("keeps the colour-plan fill on an error room -- the plan wins", () => {
    // The precedence that matters most: an active plan beats the error
    // highlight. `error` is still reported, because it drives the CSS class,
    // but the inline fill is the plan's and that is what survives into an
    // exported .svg.
    const a = resolveRoomAppearance(room("a"), {
      colourFor: () => "#123456",
      errorRoomIds: new Set(["a"]),
      showErrors: true,
    });
    expect(a.fill).toBe("#123456");
    expect(a.error).toBe(true);
    expect(roomClassName(a)).toBe("room error");
  });

  it("reports no error while showErrors is off, even for a listed room", () => {
    // showErrors follows the QA panel's expansion, so a level can genuinely
    // have errors and correctly show none.
    const a = resolveRoomAppearance(room("a"), {
      errorRoomIds: new Set(["a"]),
      showErrors: false,
    });
    expect(a.error).toBe(false);
  });

  describe("search state", () => {
    it("marks a match and does not dim it", () => {
      const a = resolveRoomAppearance(room("a"), {
        searchActive: true,
        matchRoomIds: new Set(["a"]),
      });
      expect(a).toMatchObject({ match: true, dim: false });
    });

    it("dims every non-match while a search runs", () => {
      const a = resolveRoomAppearance(room("b"), {
        searchActive: true,
        matchRoomIds: new Set(["a"]),
      });
      expect(a).toMatchObject({ match: false, dim: true });
    });

    it("dims nothing when no search is active, whatever the match set says", () => {
      // The match set outlives the search box being cleared, so this guard is
      // the difference between a cleared search and a level stuck at 15%
      // opacity.
      const a = resolveRoomAppearance(room("b"), {
        searchActive: false,
        matchRoomIds: new Set(["a"]),
      });
      expect(a).toMatchObject({ match: false, dim: false });
    });

    it("dims a non-matching room even when the match set is empty", () => {
      const a = resolveRoomAppearance(room("a"), {
        searchActive: true,
        matchRoomIds: new Set(),
      });
      expect(a.dim).toBe(true);
    });
  });

  it("composes error, match and dim independently", () => {
    // These are three orthogonal states that routinely co-occur; a room can be
    // an erroring search match, and each must survive the others.
    const a = resolveRoomAppearance(room("a"), {
      errorRoomIds: new Set(["a"]),
      showErrors: true,
      searchActive: true,
      matchRoomIds: new Set(["a"]),
    });
    expect(a).toMatchObject({ error: true, match: true, dim: false });
  });
});

describe("roomClassName", () => {
  // The token ORDER is serialized into every exported .svg and pinned by the
  // golden file. Reordering would diff every room in every export.
  it("emits tokens in the order room -> error -> match -> dim", () => {
    expect(
      roomClassName({ fill: null, error: true, match: true, dim: true }),
    ).toBe("room error match dim");
  });

  it("emits a bare `room` when nothing applies", () => {
    expect(roomClassName({ fill: null, error: false, match: false, dim: false })).toBe("room");
  });
});

describe("holeClassName", () => {
  it("carries dim but never error or match", () => {
    // A hole is a subtraction from a room, so `error`/`match` are statements
    // about the room and do not belong on it. `dim` is opacity and does.
    expect(holeClassName({ fill: null, error: true, match: true, dim: false })).toBe("hole");
    expect(holeClassName({ fill: null, error: true, match: true, dim: true })).toBe("hole dim");
  });
});
