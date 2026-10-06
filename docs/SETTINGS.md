# RoomMate — Settings reference

Every setting a project file or the server file accepts, with a sample of each.
This is a **reference for people editing TOML**: what a setting is called, what
values it takes and what it looks like. Why a setting behaves as it does lives
in the type's doc comment (`crates/roommate-shared/src/settings/mod.rs`) and in
[Architecture & Strategy](STRATEGY.md) — this page does not restate it.

Everything on this page is also editable at `/settings/`, which holds the whole
settings object it read and sends it back, so saving there never drops a section
it does not show. Each save keeps the file it replaces under `.backups/` beside
the project files (newest 20 per project).

## Where settings live

| File | Holds | Passed as |
|---|---|---|
| Server file | Storage location and dev seeding — properties of the running server | `--server-settings` |
| Project files | One TOML per project, any file name, `project_id` is the identity | `--project-settings <dir>` |

```
roommate --server-settings settings/server.toml --project-settings settings/projects
```

Relative paths inside a settings file resolve against **that file's own
directory**, not the working directory. A project file is validated in full at
startup and on every save, and a bad one fails loudly naming the mistake.
Settings are **not file-watched**: a hand edit takes effect at the next start,
while a save through `/settings/` is applied live.

**TOML ordering.** Scalars and arrays of plain values (`name`, `room_label`,
`room_models`) go *above* the first `[table]` of their section. A value written
after a table header belongs to that table.

---

## Server file

### `[storage]`

Where snapshots are written. Omit the section to run in memory (nothing survives
a restart).

```toml
[storage]
root = "../data/snapshots"
```

### `[test_data]`

Dev only: seeds the server with one snapshot at startup, in the same JSON shape
a push sends. Leave it out in production.

```toml
[test_data]
snapshot_path = "test_snapshot.json"
```

---

## Project file

### Identity

| Key | Type | Default |
|---|---|---|
| `project_id` | string, required | — |
| `name` | string | none (the id is shown) |
| `is_default` | bool | `false` |

`project_id` must be safe as a file name and cannot change once saved. At most
one project may set `is_default = true`; it answers for ids that have no file of
their own.

```toml
project_id = "campus"
name = "Sample Campus"
is_default = false
```

### Room label and comparison

| Key | Type | Default |
|---|---|---|
| `room_label` | list of property names | `["$name", "$id"]` |
| `comparison_key` | property name | none |
| `comparison_properties` | list of property names | `[]` |

`$name` and `$id` are the room's own fields; anything else is a property name,
including a joined one such as `schedule.FireRating`. A name that resolves to
nothing for a room just contributes nothing. `comparison_key` is what rooms are
matched on between two milestones, and `comparison_properties` are the ones
reported as changed.

```toml
room_label = ["$name", "RoomNumber"]
comparison_key = "RoomNumber"
comparison_properties = ["Area", "Department", "SubDepartment"]
```

### `anchor_model`

Which model defines the project-local coordinate frame that every read places
its geometry into. Absent means the lexicographically smallest model id. It
moves the whole plan as one piece and never changes how models sit relative to
each other; it matters only for exported coordinates.

```toml
anchor_model = "ARCH-MAIN"
```

### `[[hierarchy]]` — classification tiers

Ordered, outermost first. Each tier names the room property holding its code,
its display name, or both (at least one is required).

```toml
[[hierarchy]]
name = "Building"
name_property = "Building"

[[hierarchy]]
name = "Department"
code_property = "Department Number"
name_property = "Department"
```

### `[[hierarchy_exclusions]]` — keep things out of the area tiers

Two kinds, told apart by `match`.

- `group` — a resolved group at `tier` is still computed and reported but is
  withheld from its parent's total (for example outdoor areas).
- `rooms` — the listed room ids never enter any total at all.

`value` matches the tier's resolved code or name.

```toml
[[hierarchy_exclusions]]
match = "group"
tier = "Department"
value = "External"

[[hierarchy_exclusions]]
match = "rooms"
ids = ["12345", "67890"]
```

### `[[builtin_properties]]` — one name over several producers

Maps a stable name to whatever each producer calls the property, so a
classification tier or reference link can use the stable name. With a single
producer you do not need any.

```toml
[[builtin_properties]]
canonical = "Area"

[builtin_properties.by_source]
revit = "Area"
ifc = "NetFloorArea"
```

### `[sources.reference.<name>]` — reference data joined onto an entity

The name is the **join namespace**: `schedule.FireRating` in a filter, label or
comparison refers to column `FireRating` of source `schedule`. Names are unique
across entities.

| Key | Values | Default |
|---|---|---|
| `type` | `"upload"` (the only live origin) | required |
| `entity` | `rooms`, `doors`, `windows`, `ffe`, `spaces` | `rooms` |
| `fields` | list of column declarations | none (every column a string) |

A column declaration:

| Key | Values |
|---|---|
| `label` | the CSV column name |
| `type` | `string` (default), `numeric`, `date` |
| `format` | strftime pattern; **required** for `date`, rejected otherwise |
| `revit_format` | strftime pattern for the Revit side of a date comparison |
| `qa` | `exact` (compare as text) or `ignore` (leave out of QA) |

`type = "file"` is gone: data arrives by upload, which keeps dated history a
milestone can pin.

```toml
[sources.reference.schedule]
type = "upload"
entity = "rooms"

[[sources.reference.schedule.fields]]
label = "Programmed Area"
type = "numeric"
qa = "ignore"

[[sources.reference.schedule.fields]]
label = "Last Updated"
type = "date"
format = "%-m/%-d/%Y %-I:%M:%S %p"
qa = "ignore"

[sources.reference.door_schedule]
type = "upload"
entity = "doors"
```

### `[areas]`

| Key | Values | Default |
|---|---|---|
| `boundary_location` | `centreline`, `finish_face` | taken from each push, else `finish_face` |
| `max_wall_thickness` | feet, above 0 and at most 5 | `1.5` |
| `measurement_standard` | `IPMS1`, `IPMS2`, `IPMS3`, `DIN277`, `SIA416`, `BOMA`, `RICS` | none |

`boundary_location` declares whether rooms already contain their half-walls
(`centreline`) or float inside them (`finish_face`). `max_wall_thickness` is the
largest gap treated as a wall; it feeds area totals, adjacency and the geometric
room lookup together. Read [Area calculation](STRATEGY-AREA-CALCULATION.md)
before setting `measurement_standard`: it labels a figure, it does not change
the geometry, and it is a claim that has to hold for the project.

```toml
[areas]
boundary_location = "centreline"
max_wall_thickness = 0.5
measurement_standard = "IPMS3"
```

### `[doors]` and `[windows]`

The two share one shape.

| Key | Values | Default |
|---|---|---|
| `comparison_key` | property name or `$id` | none |
| `comparison_properties` | list of property names | `[]` |
| `room_attribution` | `to_room_then_from_room`, `to_room`, `from_room`, `both`, `none` | `to_room_then_from_room` |
| `room_resolution` | `off`, `same_model`, `project` | `off` |
| `room_reference_property` | property name | none (the check is off) |

`room_attribution` decides which room an opening belongs to when the model names
both sides; it is read-time, so changing it rewrites nothing.
`room_resolution` fills in a missing room reference from geometry — `same_model`
looks only among the opening's own model's rooms, `project` across every model
in the project (needed when openings live in a separate model from the rooms).
Authored references always win; geometry fills an absent side and never
overwrites. `room_reference_property` names an authored property to reconcile
against the attributed room; absent means that check is off, not that it passed.

```toml
[doors]
comparison_key = "$id"
comparison_properties = ["$to_room", "$from_room", "Mark"]
room_attribution = "to_room_then_from_room"
room_resolution = "same_model"
room_reference_property = "Door Room Reference"

[windows]
comparison_key = "$id"
room_resolution = "project"
```

### `[ffe]`

| Key | Values | Default |
|---|---|---|
| `comparison_key` | property name | none |
| `comparison_properties` | list of property names | `[]` |
| `nested_components` | `exclude`, `include` | `exclude` |
| `room_resolution` | `off`, `same_model`, `project` | `off` |
| `room_reference_property` | property name | none |

`nested_components` decides whether furniture nested inside another family (a
chair inside casework) is counted as its own item.

```toml
[ffe]
comparison_key = "Asset Tag"
comparison_properties = ["Type Name", "Manufacturer"]
nested_components = "exclude"
room_resolution = "same_model"
```

### `[spaces]`

Links a space (from a services model) to the room it describes, and optionally
compares properties on each matched pair. With nothing set, spaces still report
which models hold none and which are unenclosed; the match and comparison stay
silent until configured. Unknown keys are rejected.

| Key | Meaning |
|---|---|
| `comparison_key` | space property holding the link value |
| `room_key` | room property to match it against, when spelled differently |
| `room_models` | models that supply the rooms; empty means every model holding rooms |
| `compared_properties` | pairs to compare on a matched space and room |

A `compared_properties` entry:

| Key | Meaning |
|---|---|
| `space` | property name on the space |
| `room` | property name on the room, when different |
| `type` | `string`, `numeric`, `date` |
| `qa` | `exact` or `ignore` |
| `tolerance_pct` | flag only past this percentage of the **room's** value |
| `tolerance_min` | and past this absolute difference, in the property's units; both must be exceeded |

```toml
[spaces]
comparison_key = "Number"
room_key = "Number"
room_models = ["ARCH-L1-L2", "ARCH-L3-L5"]

[[spaces.compared_properties]]
space = "Area"
room = "Area"
type = "numeric"
tolerance_pct = 30.0
tolerance_min = 2.0

[[spaces.compared_properties]]
space = "Room Name"
room = "Name"
```

### `[[milestones]]`

A named date with the snapshots pinned to it. Pins are separate maps per entity
because each is pushed independently and their snapshot ids do not correspond;
a model with no entry contributes nothing to that milestone. A snapshot id is
the push's UTC timestamp. The name is the identity and is unique per project;
`date` is `YYYY-MM-DD` or a full RFC 3339 date-time.

```toml
[[milestones]]
name = "Design Freeze"
date = "2026-06-30"

[milestones.attachments]
"ARCH-MAIN" = "2026-06-29T10:00:00.123456Z"

[milestones.door_attachments]
"ARCH-MAIN" = "2026-06-29T10:05:00.000000Z"

[milestones.reference_snapshots]
schedule = "2026-06-29T17:00:00Z"
```

The other pin maps are `window_attachments`, `ffe_attachments`,
`space_attachments`, `ceiling_attachments` and `floor_attachments`.

### `[[colour_plans]]`

Named room colourings the viewer switches between. A joined property such as
`schedule.Last Updated` must name a source declared in the same file. The server stores them and
never computes a colour. At most one plan may be `active`. There are three
kinds, chosen by `mode.kind`.

**`hierarchy`** — one hue per group at the listed tiers:

```toml
[[colour_plans]]
name = "By department"
active = true

[colour_plans.mode]
kind = "hierarchy"
tiers = ["Department"]
scheme = "Set2"
```

**`daterange`** — shade by how near a date property is to a reference date:

```toml
[[colour_plans]]
name = "Recently changed"
active = false

[colour_plans.mode]
kind = "daterange"
property = "schedule.Last Updated"
near_date = "2026-06-30"
scheme = "RdYlGn"
format = "%-m/%-d/%Y"
```

**`propertycompare`** — colour by the difference (`op = "diff"`) or ratio
(`op = "ratio"`) of two properties, with one of three colourings:

- `style = "match"` with `tolerance`
- `style = "diverging"` with `scheme`
- `style = "bands"` with a list of bands, each `[lo, hi)`; bands must be sorted
  and non-overlapping, only the first may omit `lo`, only the last may omit
  `hi`, and a gap renders grey.

```toml
[[colour_plans]]
name = "Area vs programme"
active = false

[colour_plans.mode]
kind = "propertycompare"
property_a = "Area"
property_b = "schedule.Programmed Area"
op = "diff"

[colour_plans.mode.colouring]
style = "bands"

[[colour_plans.mode.colouring.bands]]
hi = 0.0
colour = "#d7191c"

[[colour_plans.mode.colouring.bands]]
lo = 0.0
hi = 10.0
colour = "#1a9641"

[[colour_plans.mode.colouring.bands]]
lo = 10.0
colour = "#fdae61"
```

### `[appearance]` — plan colours

Any CSS colour string; an unset key keeps the theme default. What can be set
differs by what each entity draws.

| Section | Keys |
|---|---|
| `[appearance.rooms]` | `line`, `fill`, `selection`, `hover` |
| `[appearance.doors]`, `.windows`, `.ffe` | `line`, `fill`, `selection` |
| `[appearance.spaces]`, `.ceilings`, `.floors` | `line`, `selection` |

```toml
[appearance.rooms]
line = "#444444"
fill = "#f4f1ea"
selection = "#0b6bcb"
hover = "#dbe8f7"

[appearance.doors]
line = "#8a4b08"
fill = "#f3d9b1"

[appearance.ceilings]
line = "#555555"
```

### `[hover]` — what a hover on the plan reads out

One property name per entity: `rooms`, `doors`, `windows`, `ffe`, `spaces`,
`ceilings`, `floors`. The lookup reads the instance parameter, then the type;
a name nothing carries falls back to the element's name, then its id, so a typo
costs the feature and never the tooltip.

```toml
[hover]
rooms = "Department"
doors = "Door Leaf Thickness"
ffe = "Type Name"
```
