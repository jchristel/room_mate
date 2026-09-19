// The schedule and by-room report types, and what each one opens with.
//
// **Defaults matter more than they look.** A report that opens empty asks the
// reader to know the vocabulary before it shows them anything, so each entity
// opens on columns every model measured so far carries — intrinsics (`$id`,
// `$type_name`) and the handful of Revit names that are not project
// conventions.
//
// What a picker OFFERS is no longer here: `/projects/{id}/reports/columns`
// serves this project's actual property names, with the value type Revit
// stated for each. This file keeps only what the server cannot decide — where
// a report starts.

export interface EntityDef {
  /** Wire spelling, as `service::reports::Entity::parse` reads it. */
  id: string;
  label: string;
  /** Singular noun for a sentence: "3 doors with no room". */
  one: string;
  many: string;
  columns: string[];
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
