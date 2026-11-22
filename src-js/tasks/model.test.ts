import { describe, expect, it } from "vitest";

import { canDismiss, canResend, countByState, findingText, matchesTask, shownTasks, modelsOf, needsAttention, newestFirst, roomsLabel, shortTime } from "./model.js";
import type { TaskSummary } from "./types.js";

const row = (id: string, state: TaskSummary["state"], model = "m", at = "2026-10-20T09:00:00Z"): TaskSummary => ({
  id,
  kind: "populate_room",
  dry_run: false,
  state,
  model_id: model,
  room: { room_id: id, name: id },
  phase: "P",
  overwrite_layout: false,
  create_rls: false,
  overwrite_rls: false,
  items: 0,
  sheets: 0,
  tag_rules: 0,
  findings: 0,
  created_at: at,
  updated_at: at,
  attempts: 0,
});

describe("task states", () => {
  it("lets a person dismiss anything not dismissed and resend anything not queued", () => {
    expect(canDismiss("in_progress")).toBe(true);
    expect(canDismiss("dismissed")).toBe(false);
    expect(canResend("queued")).toBe(false);
    expect(canResend("in_progress")).toBe(true);
    expect(canResend("dismissed")).toBe(true);
  });

  it("flags the results that need a look", () => {
    expect(["failed", "aborted", "completed_with_warnings"].every((s) => needsAttention(s as never))).toBe(true);
    expect(needsAttention("completed")).toBe(false);
    expect(needsAttention("queued")).toBe(false);
  });

  it("counts every state, including those with none", () => {
    const c = countByState([row("a", "queued"), row("b", "queued"), row("c", "failed")]);
    expect(c.queued).toBe(2);
    expect(c.failed).toBe(1);
    expect(c.completed).toBe(0);
  });

  it("lists the models once, sorted, and the newest task first", () => {
    const rows = [row("a", "queued", "z", "2026-10-20T09:00:00Z"), row("b", "queued", "m", "2026-10-20T10:00:00Z"), row("c", "queued", "z")];
    expect(modelsOf(rows)).toEqual(["m", "z"]);
    expect(newestFirst(rows).map((r) => r.id)).toEqual(["b", "a", "c"]);
  });
});

describe("findings and times", () => {
  const task = { items: [{ item_id: "7", type_name: "Bed" }] } as never;

  it("names an item by its type, and shows a message as it is", () => {
    expect(findingText({ kind: "outside", item: "7" }, task)).toContain("Bed is outside");
    expect(findingText({ kind: "outside", item: "9" }, task)).toContain("item 9");
    expect(findingText({ kind: "room_size_differs", message: "different" }, task)).toBe("different");
  });

  it("shortens a timestamp", () => {
    expect(shortTime("2026-10-20T09:10:22.430142Z")).toBe("2026-10-20 09:10");
  });

  it("names a set by its lead room and how many more", () => {
    const room = (name: string) => ({ room_id: name, name });
    expect(roomsLabel({ room: room("OFFICE") })).toBe("OFFICE");
    expect(roomsLabel({ room: room("OFFICE"), set_rooms: [room("OFFICE"), room("WC")] })).toBe("OFFICE + 1 room");
    expect(roomsLabel({ room: room("OFFICE"), set_rooms: [room("OFFICE"), room("WC"), room("HALL")] })).toBe("OFFICE + 2 rooms");
  });
});

describe("filtering tasks", () => {
  const t = { ...row("1", "queued", "House_A"), kind: "create_rls" as const, room: { room_id: "9", number: "00.01", name: "RUMPUS" }, blueprint: { name: "Rumpus bp", revision: 2 } } as TaskSummary;

  it("needs every word, in any of the task's texts", () => {
    expect(matchesTask(t, "")).toBe(true);
    expect(matchesTask(t, "rumpus 00.01")).toBe(true);
    expect(matchesTask(t, "RLS queued")).toBe(true);
    expect(matchesTask(t, "rumpus kitchen")).toBe(false);
  });
});

describe("the tasks the list shows", () => {
  const rows = [row("a", "queued", "m1", "2026-10-20T09:00:00Z"), row("b", "failed", "m1", "2026-10-21T09:00:00Z"), row("c", "queued", "m2", "2026-10-22T09:00:00Z")];
  const want = { model: "", states: new Set<TaskSummary["state"]>(), onlyWarnings: false, text: "", warns: () => false };

  it("narrow by model, state, warning and words, newest first", () => {
    expect(shownTasks(rows, want).map((t) => t.id)).toEqual(["c", "b", "a"]);
    expect(shownTasks(rows, { ...want, model: "m1" }).map((t) => t.id)).toEqual(["b", "a"]);
    expect(shownTasks(rows, { ...want, states: new Set(["queued"]) }).map((t) => t.id)).toEqual(["c", "a"]);
    expect(shownTasks(rows, { ...want, onlyWarnings: true, warns: (id) => id === "b" }).map((t) => t.id)).toEqual(["b"]);
    expect(shownTasks(rows, { ...want, text: "no such room" })).toEqual([]);
  });
});
