// The schedule and by-room report types, and what each one opens with.
//
// **Defaults matter more than they look.** A report that opens empty asks the
// reader to know the property vocabulary before it will show them anything, so
// each entity opens on columns that exist in every model measured so far —
// intrinsics (`$id`, `$type_name`) and the handful of Revit names that are not
// project conventions. Everything else is typed in; see the open question on
// column discovery in docs/STRATEGY-REPORTS.md.
//
// The measures are not preferences: they are the fields the JOIN produced, and
// they only exist for the entities whose attribution measures anything.

export interface EntityDef {
  /** Wire spelling, as `service::reports::Entity::parse` reads it. */
  id: string;
  label: string;
  /** Singular noun for a sentence: "3 doors with no room". */
  one: string;
  many: string;
  columns: string[];
  /** Offered in the picker beside whatever the reader types. */
  suggestions: string[];
  /** What the join measured, empty where it measured nothing. */
  measures: string[];
  defaultMeasures: string[];
  /** Whether this entity can be reported by room — see `Entity::joins_rooms`. */
  byRoom: boolean;
  /** Why not, for the entities that cannot. */
  byRoomNote?: string;
}

const SURFACE_MEASURES = ["overlap_area", "fraction_of_room", "fraction_of_element", "mean_width"];

export const ENTITIES: EntityDef[] = [
  {
    id: "rooms",
    label: "Rooms",
    one: "room",
    many: "rooms",
    columns: ["$id", "$name", "Number"],
    suggestions: ["$id", "$name", "$level_id", "Number", "Name", "Department", "Area"],
    measures: [],
    defaultMeasures: [],
    byRoom: false,
    byRoomNote: "Rooms are the other side of the join.",
  },
  {
    id: "doors",
    label: "Doors",
    one: "door",
    many: "doors",
    columns: ["$id", "$type_name", "Mark"],
    suggestions: ["$id", "$type_name", "$type_id", "$level_id", "$from_room", "$to_room", "Mark", "Comments"],
    measures: [],
    defaultMeasures: [],
    byRoom: true,
  },
  {
    id: "windows",
    label: "Windows",
    one: "window",
    many: "windows",
    columns: ["$id", "$type_name", "Mark"],
    suggestions: ["$id", "$type_name", "$type_id", "$level_id", "$from_room", "$to_room", "Mark", "Comments"],
    measures: [],
    defaultMeasures: [],
    byRoom: true,
  },
  {
    id: "ceilings",
    label: "Ceilings",
    one: "ceiling",
    many: "ceilings",
    columns: ["$id", "$type_name", "$height_offset"],
    suggestions: ["$id", "$type_name", "$type_id", "$level_id", "$height_offset", "Mark", "Comments"],
    measures: SURFACE_MEASURES,
    defaultMeasures: ["overlap_area", "fraction_of_room"],
    byRoom: true,
  },
  {
    id: "floors",
    label: "Floors",
    one: "floor",
    many: "floors",
    // A floor report without the height offset cannot tell a finish from the
    // roof build-up hosted on the same level, which is the first question
    // House A's data raises.
    columns: ["$id", "$type_name", "$height_offset"],
    suggestions: ["$id", "$type_name", "$type_id", "$level_id", "$height_offset", "Mark", "Comments"],
    measures: SURFACE_MEASURES,
    defaultMeasures: ["overlap_area", "fraction_of_room"],
    byRoom: true,
  },
  {
    id: "ffe",
    label: "FF&E",
    one: "item",
    many: "items",
    columns: ["$id", "$category", "$type_name"],
    suggestions: ["$id", "$category", "$type_name", "$type_id", "$level_id", "$room", "Mark", "Manufacturer"],
    measures: ["room_origin"],
    defaultMeasures: ["room_origin"],
    byRoom: true,
  },
  {
    id: "spaces",
    label: "Spaces",
    one: "space",
    many: "spaces",
    columns: ["$id", "$name", "Number"],
    suggestions: ["$id", "$name", "$level_id", "Number", "Name", "Space Type"],
    measures: [],
    defaultMeasures: [],
    byRoom: false,
    byRoomNote:
      "A space matches a room on a key, project-wide, and that match lives in the QA report rather than on the rows. Use the Unmatched spaces and rooms check.",
  },
];

export const entityById = (id: string): EntityDef => ENTITIES.find((e) => e.id === id) as EntityDef;

/** Room columns a by-room report opens with, and what it offers beside them. */
export const ROOM_COLUMNS = ["Number", "$name"];
export const ROOM_SUGGESTIONS = ["Number", "Name", "$name", "$id", "$level_id", "Department", "Area"];
