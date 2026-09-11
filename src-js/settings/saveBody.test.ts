import { describe, expect, it } from "vitest";

import { saveBody } from "./saveBody.js";
import type { Settings } from "./types.js";

/** A settings object as `GET /api/settings/projects/{id}` returns one.
 *  Duplicated rather than shared, per the house rule on per-module helpers. */
const settings = (extra: Partial<Settings> = {}): Settings => ({
  project_id: "p1",
  is_default: false,
  comparison_properties: [],
  sources: {},
  hierarchy: [],
  builtin_properties: [],
  room_label: ["$name", "$id"],
  milestones: [],
  colour_plans: [],
  ...extra,
});

/** What actually goes over the wire, which is where an `undefined` disappears. */
const wire = (s: Settings): Record<string, unknown> => JSON.parse(JSON.stringify(saveBody(s))) as Record<string, unknown>;

describe("saveBody", () => {
  it("sends every cleared scalar as null, so the server's merge cannot restore it", () => {
    const body = wire(settings({ name: "", comparison_key: "   ", anchor_model: null }));
    for (const key of ["name", "comparison_key", "anchor_model"]) {
      expect(Object.hasOwn(body, key), key).toBe(true);
      expect(body[key], key).toBeNull();
    }
  });

  it("still sends null for a field the project never set", () => {
    const body = wire(settings());
    expect(Object.hasOwn(body, "anchor_model")).toBe(true);
    expect(body["anchor_model"]).toBeNull();
  });

  it("trims the values that are set", () => {
    const body = wire(settings({ name: "  House A ", anchor_model: " ARCH-01 " }));
    expect(body["name"]).toBe("House A");
    expect(body["anchor_model"]).toBe("ARCH-01");
  });

  it("carries whole sections back exactly as they came", () => {
    // Including the ones this page has no control for -- the reason the body is
    // the object as read rather than one rebuilt field by field.
    const body = wire(
      settings({
        areas: { max_wall_thickness: 1.5, measurement_standard: "IPMS3" },
        doors: {
          room_attribution: "to_room",
          room_resolution: "same_model",
          comparison_properties: ["$to_room"],
        },
        milestones: [
          {
            name: "Freeze",
            date: "2026-06-30",
            reference_snapshots: {},
            attachments: { "model-1": "2026-06-29T10:00:00Z" },
            door_attachments: { "model-1": "2026-06-28T09:00:00Z" },
            window_attachments: {},
            ffe_attachments: {},
            space_attachments: {},
            ceiling_attachments: {},
          },
        ],
      }),
    );
    expect(body["areas"]).toEqual({ max_wall_thickness: 1.5, measurement_standard: "IPMS3" });
    expect((body["doors"] as Record<string, unknown>)["room_attribution"]).toBe("to_room");
    // The pins the JavaScript page drops on every save: a door pin survives here
    // because the whole milestone goes back, not four rebuilt fields of it.
    const milestone = (body["milestones"] as Record<string, unknown>[])[0] as Record<string, unknown>;
    expect(milestone["door_attachments"]).toEqual({ "model-1": "2026-06-28T09:00:00Z" });
  });
});
