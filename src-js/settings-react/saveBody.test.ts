import { describe, expect, it } from "vitest";

import { saveBody } from "./saveBody.js";
import type { Settings } from "./types.js";

/** A settings object as `GET /api/settings/projects/{id}` returns one.
 *  Duplicated rather than shared, per the house rule on per-module helpers. */
const settings = (extra: Partial<Settings> = {}): Settings => ({
  project_id: "p1",
  is_default: false,
  room_label: ["$name", "$id"],
  ...extra,
});

/** What actually goes over the wire, which is where an `undefined` disappears. */
const wire = (s: Settings): Record<string, unknown> => JSON.parse(JSON.stringify(saveBody(s))) as Record<string, unknown>;

describe("saveBody", () => {
  it("sends a cleared name as null, so the server's merge cannot restore it", () => {
    const body = wire(settings({ name: "" }));
    expect(Object.hasOwn(body, "name")).toBe(true);
    expect(body["name"]).toBeNull();
  });

  it("treats a whitespace-only name as cleared", () => {
    expect(wire(settings({ name: "   " }))["name"]).toBeNull();
  });

  it("still sends null when the project never had a name", () => {
    const body = wire(settings());
    expect(Object.hasOwn(body, "name")).toBe(true);
    expect(body["name"]).toBeNull();
  });

  it("trims a name that is set", () => {
    expect(wire(settings({ name: "  House A " }))["name"]).toBe("House A");
  });

  it("carries sections the page has no control for back exactly as they came", () => {
    const body = wire(settings({ comparison_key: "Number", doors: { room_resolution: "same_model" } }));
    expect(body["comparison_key"]).toBe("Number");
    expect(body["doors"]).toEqual({ room_resolution: "same_model" });
  });
});
