import { describe, expect, it } from "vitest";

import { defaultSettings } from "./defaults.js";

describe("defaultSettings", () => {
  it("starts a project with every section present at its real default", () => {
    const s = defaultSettings();
    expect(s.areas.max_wall_thickness).toBe(1.5);
    expect(s.routing.room_height_property).toBe("Ceiling Height");
    expect(s.routing.door_frame_allowance_mm).toBe(150);
    expect(s.routing.door_height_properties).toEqual(["Height", "Door Height"]);
    expect(s.doors.room_attribution).toBe("to_room_then_from_room");
    expect(s.ffe.nested_components).toBe("exclude");
    expect(s.room_label).toEqual(["$name", "$id"]);
  });

  it("hands out fresh lists, so two new projects never share one", () => {
    const a = defaultSettings();
    a.routing.door_height_properties?.push("x");
    a.doors.comparison_properties?.push("y");
    const b = defaultSettings();
    expect(b.routing.door_height_properties).toEqual(["Height", "Door Height"]);
    expect(b.doors.comparison_properties).toEqual([]);
  });
});
