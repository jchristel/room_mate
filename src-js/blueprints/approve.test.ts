import { describe, expect, it } from "vitest";

import { canTick, isOverridden, planText, tickable } from "./approve.js";
import type { AssignmentRow, RoomImpact } from "./types.js";

const room = (id: string, status: RoomImpact["status"]): RoomImpact => ({
  assignment_id: id,
  room: { model_id: "m", room_id: id },
  room_name: id,
  pinned_revision: 1,
  status,
  held: [],
  findings: [],
  item_changes: [],
  plan: [],
  plan_counts: {},
});

const row = (id: string, extra: Partial<AssignmentRow> = {}): AssignmentRow =>
  ({ id, blueprint_id: "b", revision: 1, room: { model_id: "m", room_id: id }, room_name: id, mapping: {}, taken_at: "", blueprint_name: "B", current_revision: 2, newer_revision: true, blueprint_missing: false, ...extra }) as AssignmentRow;

describe("approving an update", () => {
  it("lets through only a room with something to do or nothing wrong", () => {
    expect(canTick("changes") && canTick("no_change")).toBe(true);
    for (const s of ["error", "room_gone", "cannot_judge", "needs_decision"] as const) expect(canTick(s)).toBe(false);
  });

  it("ticks every room with changes, or only those with none of their own", () => {
    const rooms = [room("a", "changes"), room("b", "changes"), room("c", "needs_decision"), room("d", "no_change")];
    const byId = new Map([["a", row("a")], ["b", row("b", { overrides: [{ item_id: "1", along_mm: 5 }] })], ["c", row("c")], ["d", row("d")]]);
    expect([...tickable(rooms, byId, false)].sort()).toEqual(["a", "b"]);
    expect([...tickable(rooms, byId, true)]).toEqual(["a"]);
  });

  it("counts an addition as the room's own change", () => {
    expect(isOverridden(row("a", { additions: [{ id: "add-1", clone_of: "1", edge: 0, along_mm: 0, off_mm: 0 }] }))).toBe(true);
    expect(isOverridden(row("a", { overrides: [] }))).toBe(false);
    expect(isOverridden(undefined)).toBe(false);
  });

  it("says the plan in words", () => {
    expect(planText({})).toBe("nothing to do");
    expect(planText({ add: 2, move: 1, report: 3 })).toBe("2 to add, 1 to move, 3 to look at");
  });
});
