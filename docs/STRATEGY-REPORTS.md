# RoomMate — Reports

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[Area calculation](STRATEGY-AREA-CALCULATION.md) ·
[Browser](STRATEGY-BROWSER.md) · [MCP](STRATEGY-MCP.md) ·
[Authored](STRATEGY-AUTHORED.md) · [Entities](STRATEGY-ENTITIES.md) ·
[Security](STRATEGY-SECURITY.md)

**Nothing here is built** (as of 2026-09-13). The design is settled; read this
before building any of it. When a piece ships, its section is deleted from here
and the rationale moves to the module header — see "Code documents what is
built" in [Coding Conventions](CODING-CONVENTIONS.md).

**[reports-mockup.html](reports-mockup.html)** is a clickable mockup of the
page described below: the report type dropdown, the three forms, the filter
builder and the preview, over invented sample rows. Open the file in a browser.
It is a picture of the design, not a prototype. It reads nothing, saves
nothing, and none of its code is meant to be reused. Delete it when the page
ships.

The ask is a page where a user builds tabular reports over the entities — and,
above all, **reports through an association**: ceilings by room, spaces by
room, FF&E by room. The rest of this document is mostly about why the obvious
way to let a user express that is the wrong one.

## The page

**A new React page at `/reports/`, which absorbs milestone comparison and then
replaces `comparison.html`.** Comparison *is* a report — rooms across milestones
— so it belongs under the same roof, but it is 492 lines of vanilla JS over its
own diff endpoint, and porting it first would put the feature that was asked for
behind a migration nobody asked for. So the order is:

1. `/reports/` as a third Vite build beside the renderer and the settings page —
   an app build with its own `index.html`, so a `vite.reports.config.ts` on the
   `vite.settings.config.ts` pattern, emitting into `static/reports/`, committed,
   and covered by the same frontend CI gate.
2. The association reports below.
3. Comparison ported as a report kind; `comparison.html` **deleted in the same
   change**, the way `settings.html` was. Two live comparison pages is the state
   to never be in.

A separate page rather than a mode on the viewer, for the reason
[Browser](STRATEGY-BROWSER.md) gives for a multi-project comparator.

**The save trap carries over.** `comparison.html` PUTs back the *whole* settings
JSON it loaded, verbatim except its own fields. Rebuilding that JSON field by
field is the bug that silently emptied every milestone's pins. Anything this
page saves either keeps the whole-object round-trip or goes through
`settings_api::merge_over_stored`.

## The association is chosen, never defined

The first design instinct — a table whose first column is a room property and
whose second is "the ceiling property used to link" — assumes the user supplies
the join. **For three of the four associations there is no property to supply**,
and the fourth already has one in project settings:

| Association | How the server already joins it | What a user could pick |
|---|---|---|
| Ceilings → rooms | Geometry only, area-weighted, many-to-many, model-scoped (`service::surface_attribution`, `MIN_OVERLAP_MEAN_WIDTH_FT`) | Nothing — a ceiling has no room parameter |
| FF&E → rooms | Authored room id, model-scoped, geometry fallback under `room_resolution` (`ItemResponse::owner_rooms_qualified`) | Nothing — it is an id, not a property |
| Spaces → rooms | Property key, project-scoped (`[spaces] comparison_key` / `room_key` / `room_models`) | The key — already a setting, and `SpaceReport` reads it |
| Doors, windows → rooms | `to_room` / `from_room` under `room_attribution` | The policy — already a setting |

**A user-defined join would be a second implementation of each rule, and every
one of them has a recorded way to be wrong.** FF&E joined on a "Room Number"
property project-wide reintroduces the project-scoped false match the
model-scoped rule exists to prevent — a wrong answer that looks right. A
ceiling join that skips the tolerance counts touching edges as coverage. A second
space-key picker gives the report and `SpaceReport` two answers to one
question.

So the user chooses **which association** from a fixed list, and the page
**states the rule** it runs under, in words, beside the choice — "overlap at
least 10 mm wide", "`Number` = `Number`, project-wide, rooms from
the ARCH models". Where the rule is a project setting, the statement links to
it. The report never edits a join; the settings page does.

### Rejected: a node-based editor

A node graph earns its cost when topology is arbitrary and multi-hop. This graph
is a **star** — every association is an edge into rooms, and there are five of
them. A node editor would be a dependency, a layout problem, a saved-graph
format and a test surface, spent on choosing one of five options. And the one
thing a graph would make easy — wiring an arbitrary property on one side to an
arbitrary property on the other — is exactly what the section above rules out.

**What would reopen it:** an association that is genuinely user-authored rather
than modelled. The manual room connections in [Authored](STRATEGY-AUTHORED.md)
are the nearest candidate, and even they are an edge set, not a join rule.

## Report types are a registry, and a type is a list of sections

**There will be many more report types than the first association reports**, so
the page is not built around any one of them. A report opens on a single
**report type** dropdown, grouped (`<optgroup>`) by family:

- **Schedules** — one entity, one row per element (room, ceiling, space, FF&E,
  later doors and windows).
- **By room** — the associations below.
- **Between milestones** — room changes, which is today's comparison page; door
  changes later.
- **Checks** — rooms without a ceiling, unmatched spaces and rooms: the QA
  findings that `STRATEGY-ENTITIES.md` still lists as unbuilt, served as reports
  rather than as a separate page.

A type that is not built yet sits in the list disabled, marked "later".

**Choosing a type redraws the form below it, and the form is assembled from a
small vocabulary of sections, never hand-built per type.** Each type declares
which sections it needs:

| Form | Sections |
|---|---|
| Schedule | Columns · Filter · Scope · Preview |
| Association | Join rule · Columns and measures · Filter · Scope · Rows · Unmatched · Preview |
| Comparison | Comparison key · Compared properties · Milestones · Preview |

Adding a report type is then a registry entry plus, at most, one new section.
A type needing a screen of its own is the signal that the section vocabulary is
missing something, and that is what gets fixed.

**The registry belongs on the server**, as `GET /projects/{id}/reports/types`.
It is resolved per project because a type's join rule text depends on project
settings (the space key, `room_attribution`, `room_resolution`). The page
renders what it is sent, so the rule is stated in one place, the same place it
is enforced. That route is a read, so it gets its MCP tool too.

## The association form

Three parts, left to right, under the report type:

- **Room columns** — any room property (the tiered lookup, so a blank instance
  value does not shadow a type value), plus joined reference fields in the flat
  `<source>.<label>` namespace.
- **Associated columns** — the same, over the associated entity, tiered
  instance-then-type where the entity has tiers.
- **Measures** — what the *join itself* produced, which is per association and
  not a property of either side: `overlap_area`, `fraction_of_element`,
  `fraction_of_room` for ceilings; `room_origin` for FF&E; `model_id` for
  spaces; the side (`to` / `from`) for openings.

Then the filter below, scope (building, level, latest or a milestone), row
shape, and the two unmatched-row switches.

## Filters: a tree, not the query string

The filter is a builder: **conditions** (field, operator, value) inside
**groups** that each match **all** or **any** of their children. Groups nest,
capped at three levels because deeper logic stops being readable in a form. A
plain-language line under the builder reads the whole tree back, e.g.
`Room.Level is "LEVEL 00" AND (Ceiling.Type contains "plasterboard" OR
Join.% of room ≥ 90%)`. That line is how a user checks the logic they built,
and a nested form alone does not let them.

**Operators depend on the field's type:**

- **Text:** is, is not, contains, does not contain, starts with, ends with, is
  blank, has a value.
- **Number:** =, ≠, <, ≤, >, ≥, between, is blank, has a value.

A field's type comes from its declared `FieldType` where it has one, and
otherwise from its values. An incomplete condition (no value yet) is left out
of evaluation rather than failing, so a half-built filter never empties the
preview.

**In an association report, every field names its side**: room, the associated
entity, or the join (`overlap_area`, `% of room`). The side is a separate
field in the stored definition, never a prefix in the field name. The
`<source>.<label>` dot is already taken by joined reference fields, and a
`room.` prefix would collide with any reference source that happened to be
named `room`.

### What the existing grammar gives, and what it lacks

`service::rooms::RoomFilter` is the matcher every entity read shares, through
`FilterTarget`. It is **a flat AND**: comma-separated predicates, operators
`= != > >= < <= ~`. The builder needs three things it lacks:

1. **OR, and nesting.** `RoomFilter` becomes a tree (all / any / predicate). The
   comma form parses to a single all-group, so `?filter=` on every existing
   read, and the MCP array form, keep their meaning without a migration.
2. **Operators:** does not contain, starts with, ends with, between, is blank,
   has a value. "Between" is stored as its own operator, so the form can load a
   saved report back exactly as it was built.
3. **Explicit blank tests.** Today an empty value is a parse error, and an
   absent or blank property matches no operator at all.

**That last rule stays, and the form has to show it**, because a user will not
expect it: **`Department is not Living` also drops every room with no
Department.** It is the right rule, since a missing value is not evidence of
anything. But a "does not" condition in the builder carries a one-line note and
a **Keep blanks too** action. The action rewrites the condition into
`any(is not Living, is blank)`; the rule itself does not change.

## Cardinality is the design problem, not the picker

Each association has a different cardinality, and a report that ignores it
produces a total that is wrong without looking wrong.

- **Ceilings are many-to-many.** 419 of RHH's 1,833 ceilings cover more than
  one room at a sliver threshold the read has since dropped (2026-09-16), so it
  now lists slivers too and a report that means "the rooms this ceiling is
  in" states its own line over `fraction_of_element` or `fraction_of_room`.
  Summing *ceiling area* per room counts a
  ceiling once per room it touches. **So ceiling area is not offered as a
  summable measure in the grouped shape; `overlap_area` is**, and
  `fraction_of_room` answers "how much of this room is ceiled" — the finishes
  question the attribution rule must not answer for it.
- **Spaces are one room to several spaces.** One services file per discipline
  means a room legitimately matches a mechanical and an electrical space.
  `model_id` must be available as a column or a grouping, for the reason
  `SpaceResponse::model_id` gives: without it, two disciplines read as a
  duplicate.
- **FF&E is zero-or-one**, the easy case, and **openings are zero-to-two** (or
  two attributions under the `both` policy).
- **Unmatched rows are reported states, never gaps** — "signal, not error".
  Unattributed ceilings (17 on RHH), homeless items, unmatched spaces, rooms
  with none. Two explicit switches, in plain words rather than join vocabulary:
  **"include rooms with no ⟨ceilings⟩"** and **"include ⟨ceilings⟩ with no
  room"**, the second rendering as a `(no room)` group. Without them totals do
  not reconcile and nothing says so.
- **"Rooms with no ceiling" is mostly noise** unfiltered — 12 of House A's 32,
  nearly all external (POOL, DECK, the `EX` suffix). The switch is useful only
  beside the scope filter, which is why both sit in the same form rather than
  the switch being on by default.

### Row shapes

- **One row per match** — a room and one associated element per row, room
  columns repeated. The export shape, and the default.
- **Grouped by room** — one row per room, associated columns aggregated:
  `count`, `sum` (numeric measures only, subject to the ceilings rule above),
  `distinct` (a value list).

### Rejected: several associations in one flat table

"Rooms with their ceilings *and* their FF&E" as one flat table is a cross
product — ceilings × items per room — and every count in it is multiplied.
**When more than one association is wanted, the shape is nested**: one room,
one section per association. Deferred until single-association reports are in
use; it is also where a tree-shaped view would earn its cost, which a node
editor would not.

## The join runs on the server

A client-side join over `/rooms` + `/ffe` + `/ceilings` is not viable at RHH
scale: `/ffe` is 273 MB / 94 s and `/ceilings` 33 s (see `CLAUDE.md`, "Open").

- **`service::reports`**, transport-agnostic like the rest of `service/`, takes
  a report definition and returns rows carrying **only the requested columns**.
  Dropping the full per-item property maps is also a partial answer to the
  `/ffe` payload problem, for the one consumer that never needed them.
- It reuses, never re-derives: `entity_scope` for scoping and milestone pins,
  `service::surfaces` for attribution, the items and openings reads for owners,
  and the spaces match for spaces. A report that computed its own attribution
  would be the extractor-footprint lesson in a new place.
- **`POST /projects/{id}/reports`**, body the definition. A POST read has
  precedent in `/comparison`, and a definition does not fit a query string.
- **An MCP tool beside it**, per the one-tool-per-read-route rule — which also
  lets a host ask "ceilings by room" directly. Update the tool count in
  `bin/mcp.rs`.
- **Its ETag cursor covers rooms and the associated entity**, for the reason
  `/ceilings`' already covers rooms: attribution derives from the rooms in
  scope, so a rooms push alone changes the answer.

## Saved reports

**Server-side, in project settings** — shareable and per project, which is
[Browser](STRATEGY-BROWSER.md)'s standing answer to "users re-pick the same
columns" rather than `localStorage`. A sketch, scalars first for the TOML
ordering rule:

```toml
[[reports]]
name = "Ceilings by room"
type = "by_room.ceilings"         # a registry id: schedule.rooms, cmp.rooms, ...
shape = "per_match"               # per_match | per_room
include_rooms_without = false
include_unattributed = true
room_columns = ["Number", "Name", "Level"]
associated_columns = ["Type", "Height Offset From Level"]
measures = ["overlap_area", "fraction_of_room"]

[reports.filter]                  # Level 00 AND (plasterboard OR >= 90% of room)
mode = "all"
[[reports.filter.items]]
side = "room"
field = "Level"
op = "eq"
value = "LEVEL 00"
[[reports.filter.items]]
mode = "any"
items = [
  { side = "ceilings", field = "Type", op = "contains", value = "plasterboard" },
  { side = "join", field = "fraction_of_room", op = "ge", value = "0.9" },
]
```

**The filter tree is where TOML starts to hurt.** Two levels already read
badly, and a tree is exactly the nested table-in-array shape the TOML ordering
rule in [Coding Conventions](CODING-CONVENTIONS.md) guards against. That is an
argument for storing a report's definition as a JSON document beside the
project settings rather than inside them. It is an open question below, not a
decision.

## Open questions

- **Who owns editing `[[reports]]`.** A field added to a settings type in
  `roommate-shared` is a type error on the settings page until it has a control
  there — deliberately. Either the settings page gets a control, or the reports
  page is the sole editor and that exclusion is made explicit. Decide before
  adding the field, not after the type error.
- **Does a changed rule move the ETag?** Changing `room_attribution` or the
  space key changes every report row without any push. Check how the existing
  entity reads' cursors treat a settings change before assuming the report's
  can copy them.
- **Row volume.** One-row-per-match FF&E on RHH is ~39,000 rows. Projected
  columns make that small per row, but whether it wants paging or a streamed
  CSV route is unmeasured — measure it, per [Index](STRATEGY.md)'s caveat.
- **CSV export client-side or server-side.** Client-side from the rows already
  fetched is free until the volume question says otherwise.
- **Report definitions: TOML in settings, or JSON beside them.** Filter trees
  push hard towards JSON. Settings, though, are where "shareable per project"
  already works, milestone pins included. Decide together with the
  `[[reports]]` ownership question above.
- **Should text "is" ignore case?** The builder is friendlier if it does. But
  `=` in `?filter=` is exact today, and `numeric_match` tolerates `25.50` =
  `25.5`. Recommended: the builder's text operators ignore case, while the query
  string's `=` keeps its meaning and gains no new one. That means the tree needs
  a case-insensitive equality operator of its own, not a change to `Op::Eq`.
- **What a filter does to an unmatched row.** A room with no ceilings has no
  ceiling fields, so under the missing-value rule any AND'd ceiling condition
  drops it, even with "include rooms with no ceilings" ticked. That is
  consistent, but probably not what someone asking "Living rooms and their
  plasterboard ceilings, including rooms with none" means. The likely answer is
  that conditions on the associated side filter *matches* and never remove a
  room outright. Settle it against a real request before building.
- **Doors and windows in v1.** Not asked for. They fit the same definition and
  cost a row in the association list, so they are in the shape but out of the
  first build.
