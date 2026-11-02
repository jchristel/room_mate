import { describe, expect, it } from "vitest";

import { NO_DATA_COLOUR } from "./colour.js";
import { indexTasks, lastTaskState, taskColourForRoom, TASK_STATE_COLOUR, type TaskRow } from "./taskStatus.js";
import type { Room } from "../renderer/types.js";

const room = (id: string, number?: string): Room =>
  ({ id, ...(number ? { properties: { Number: { value: number } } } : {}) }) as unknown as Room;
const task = (room_id: string, state: TaskRow["state"], updated_at: string, number?: string): TaskRow => ({
  state,
  updated_at,
  room: { room_id, ...(number ? { number } : {}) },
});

describe("task status colouring", () => {
  it("takes the newest task of a room", () => {
    const index = indexTasks([task("1", "failed", "2026-10-01T00:00:00Z"), task("1", "queued", "2026-10-02T00:00:00Z")]);
    expect(lastTaskState(room("1"), index)).toBe("queued");
  });

  it("lets a dismissed task fall back to the one before it", () => {
    const index = indexTasks([task("1", "completed", "2026-10-01T00:00:00Z"), task("1", "dismissed", "2026-10-02T00:00:00Z")]);
    expect(taskColourForRoom(room("1"), index)).toBe(TASK_STATE_COLOUR.completed);
  });

  it("colours a room with no live task as no data", () => {
    const index = indexTasks([task("1", "dismissed", "2026-10-01T00:00:00Z")]);
    expect(taskColourForRoom(room("1"), index)).toBe(NO_DATA_COLOUR);
    expect(taskColourForRoom(room("2"), index)).toBe(NO_DATA_COLOUR);
  });

  it("refuses a task whose number disagrees with the room's", () => {
    const index = indexTasks([task("7", "failed", "2026-10-01T00:00:00Z", "01.06")]);
    expect(lastTaskState(room("7", "02.01"), index)).toBeNull();
    expect(lastTaskState(room("7", "01.06"), index)).toBe("failed");
    expect(lastTaskState(room("7"), index)).toBe("failed");
  });
});
