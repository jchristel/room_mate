// What a NEW project starts from: every setting present, at the value the server
// would use if it were absent.
//
// A project file written by hand or by an older page omits whatever is default
// (`skip_serializing_if`), and each section copes by falling back to these same
// values -- so an untouched box meant "the default" and showed nothing. Starting a
// project from the whole set puts every setting on the page with its real value, and
// saves it, so nothing is a silent fallback the author never saw.
//
// **The Rust defaults, spelled once on this side**, and the sections import them
// from here: a second copy in a section is how the page and the server drift.
// `RoutingPolicy::DEFAULT_*` and `AreaPolicy::DEFAULT_MAX_WALL_THICKNESS_FT` are the
// authority. A setting whose default is *none* (a comparison key, the anchor model, a
// hover property, a colour) stays blank, because blank is its default.

import type { AreaPolicy, FfePolicy, OpeningPolicy, RoutingPolicy, Settings } from "./types.js";

export const DEFAULT_MAX_WALL_THICKNESS_FT = 1.5;
export const DEFAULT_ROOM_HEIGHT_PROPERTY = "Ceiling Height";
export const DEFAULT_FRAME_ALLOWANCE_MM = 150;
export const DEFAULT_DOOR_HEIGHT_PROPERTIES = ["Height", "Door Height"];

export const AREA_DEFAULT: AreaPolicy = { max_wall_thickness: DEFAULT_MAX_WALL_THICKNESS_FT };

export const OPENING_DEFAULT: OpeningPolicy = {
  room_attribution: "to_room_then_from_room",
  room_resolution: "off",
  comparison_properties: [],
};

export const FFE_DEFAULT: FfePolicy = {
  nested_components: "exclude",
  room_resolution: "off",
  comparison_properties: [],
};

export const ROUTING_DEFAULT: RoutingPolicy = {
  room_height_property: DEFAULT_ROOM_HEIGHT_PROPERTY,
  room_height_from_ceilings: true,
  door_height_from_type_name: true,
  door_frame_allowance_mm: DEFAULT_FRAME_ALLOWANCE_MM,
  door_height_properties: [...DEFAULT_DOOR_HEIGHT_PROPERTIES],
};

/** The settings whose default is NONE, so leaving them out is the default. Every
 *  other key must be listed below: `Required` makes a field added in Rust, and not
 *  here, a type error -- the page cannot silently start a project without it. */
type NoDefault = "name" | "comparison_key" | "anchor_model";

/** A project that does not exist yet. Fresh copies each call: the editor edits what
 *  it is given, and two new projects must not share a list. */
export const defaultSettings = (): Required<Omit<Settings, NoDefault>> & Pick<Settings, NoDefault> => ({
  project_id: "",
  is_default: false,
  comparison_properties: [],
  sources: {},
  areas: { ...AREA_DEFAULT },
  doors: { ...OPENING_DEFAULT, comparison_properties: [] },
  windows: { ...OPENING_DEFAULT, comparison_properties: [] },
  ffe: { ...FFE_DEFAULT, comparison_properties: [] },
  spaces: { room_models: [], compared_properties: [] },
  routing: { ...ROUTING_DEFAULT, door_height_properties: [...DEFAULT_DOOR_HEIGHT_PROPERTIES] },
  hierarchy: [],
  builtin_properties: [],
  room_label: ["$name", "$id"],
  milestones: [],
  colour_plans: [],
  hierarchy_exclusions: [],
  appearance: {},
  hover: {},
});
