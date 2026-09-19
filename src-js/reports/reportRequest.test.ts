import { describe, expect, it } from "vitest";

import { summarise, toBody, type FormState } from "./reportRequest.js";

const form: FormState = {
  entityId: "ceilings",
  byRoom: true,
  columns: ["$type_name"],
  roomColumns: ["Number"],
  measures: ["overlap_area"],
  shape: "per_room",
  includeRoomsWithout: true,
  includeUnattributed: true,
  limit: 500,
};

describe("toBody", () => {
  it("sends the by-room half of the form for a by-room report", () => {
    expect(toBody(form)).toMatchObject({
      entity: "ceilings",
      by_room: true,
      columns: ["$type_name"],
      room_columns: ["Number"],
      measures: ["overlap_area"],
      shape: "per_room",
      include_rooms_without: true,
      include_unattributed: true,
    });
  });

  // A schedule has no room side, so it must not ask for one — the server would
  // otherwise have to decide to ignore columns it was sent, which is a
  // decision nobody can see.
  it("drops the room side for a schedule, whatever the form still holds", () => {
    const body = toBody({ ...form, byRoom: false });
    expect(body.room_columns).toEqual([]);
    expect(body.measures).toEqual([]);
    expect(body.shape).toBe("per_match");
    expect(body.include_rooms_without).toBe(false);
  });

  // The element columns are the one thing a schedule keeps.
  it("keeps the element columns either way", () => {
    expect(toBody({ ...form, byRoom: false }).columns).toEqual(["$type_name"]);
  });
});

describe("summarise", () => {
  const entity = { one: "ceiling", many: "ceilings" };

  // A capped preview that said "500 rows" would read as the whole answer.
  it("says what the cap hid", () => {
    expect(summarise(500, 39412, 0, entity).shown).toBe("first 500 of 39,412 rows");
  });

  it("says the plain count when nothing was capped", () => {
    expect(summarise(12, 12, 0, entity).shown).toBe("12 rows");
    expect(summarise(1, 1, 0, entity).shown).toBe("1 row");
  });

  // Unmatched rows are reported, never filtered: the total includes them and
  // the reader has to know that before adding a column up.
  it("names the unmatched rows when there are any", () => {
    const summary = summarise(30, 30, 8, entity);
    expect(summary.unmatched).toContain("8 of them matched nothing");
    expect(summary.unmatched).toContain("a room with no ceilings");
  });

  it("says nothing about unmatched rows when there are none", () => {
    expect(summarise(30, 30, 0, entity).unmatched).toBeUndefined();
  });
});
