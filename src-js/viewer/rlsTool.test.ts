import { describe, expect, it } from "vitest";

import { chooseSheet, madeText, newRlsTool, pickRoom, roomsText, whyNot } from "./rlsTool.js";

describe("the RLS tool", () => {
  it("adds a room on one click and takes it off on the next", () => {
    const a = pickRoom(newRlsTool(), "r1");
    const b = pickRoom(a, "r2");
    expect(b.roomIds).toEqual(["r1", "r2"]);
    expect(pickRoom(b, "r1").roomIds).toEqual(["r2"]);
  });

  it("needs a sheet and a room, and says which", () => {
    expect(whyNot(newRlsTool())).toBe("choose a sheet");
    expect(whyNot(chooseSheet(newRlsTool(), "s"))).toBe("click the rooms to make sheets for");
    expect(whyNot(pickRoom(chooseSheet(newRlsTool(), "s"), "r"))).toBeNull();
  });

  it("counts rooms", () => {
    expect(roomsText(1)).toBe("1 room");
    expect(roomsText(2)).toBe("2 rooms");
  });

  it("reports what was queued and the first refusal", () => {
    expect(madeText([{ subject: "1" }, { subject: "2", room_name: "BED", refused: "a task is already queued" }, { subject: "3", refused: "x" }])).toBe(
      "Queued 1 room. Refused: BED: a task is already queued (+1 more)",
    );
    expect(madeText([{ subject: "1" }])).toBe("Queued 1 room.");
  });
});
