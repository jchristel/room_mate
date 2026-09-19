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
page described below: the report type dropdown, the four forms, the filter
builder and the preview, over invented sample rows. Open the file in a browser.
It is a picture of the design, not a prototype: it reads nothing, saves
nothing, and none of its code is meant to be reused. It does follow the decided
filter semantics -- set-wise conditions on the associated side, and text that
ignores case unless a condition says otherwise -- because a mockup that
contradicts the rules is worse than none. Delete it when the page ships.

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
| Floors → rooms | The same rule, the same code — a floor is a `Surface` too | Nothing, for the same reason |
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
is a **star** — every association is an edge into rooms, and there are six of
them. A node editor would be a dependency, a layout problem, a saved-graph
format and a test surface, spent on choosing one of six options. And the one
thing a graph would make easy — wiring an arbitrary property on one side to an
arbitrary property on the other — is exactly what the section above rules out.

**What would reopen it:** an association that is genuinely user-authored rather
than modelled. The manual room connections in [Authored](STRATEGY-AUTHORED.md)
are the nearest candidate, and even they are an edge set, not a join rule.

## Report types are a registry, and a type is a list of sections

**There will be many more report types than the first association reports**, so
the page is not built around any one of them. A report opens on a single
**report type** dropdown, grouped (`<optgroup>`) by family:

- **Schedules** — one entity, one row per element: room, door, window, ceiling,
  floor, space, FF&E. All seven in v1.
- **By room** — the associations below, all six of them in v1.
- **Between milestones** — room changes, which is today's comparison page; door
  changes later.
- **Checks** — QA findings served as reports, all in v1: the reference-data
  check, rooms without a ceiling, unmatched spaces and rooms, and openings whose
  room reference does not resolve.

A type that is not built yet sits in the list disabled, marked "later".

**Doors, windows and floors are all in v1** (decided 2026-09-19), and each
costs a registry row rather than a design. Doors and windows are both
`Opening`, both already carry `owner_rooms_qualified`, and `service::openings`
answers the read. A floor is a `Surface`, so it arrives on the ceilings stack
verbatim — same `service::surfaces`, same attribution, same tolerance — which
is the entity split paying out exactly as `STRATEGY-ENTITIES.md` predicted.
What each adds that no other association has is below.

**Choosing a type redraws the form below it, and the form is assembled from a
small vocabulary of sections, never hand-built per type.** Each type declares
which sections it needs:

| Form | Sections |
|---|---|
| Schedule | Columns · Filter · Scope · Preview |
| Association | Join rule · Columns and measures · Filter · Scope · Rows · Unmatched · Preview |
| Comparison | Comparison key · Compared properties · Milestones · Preview |
| Check | What counts as a finding · Scope · Preview |

Adding a report type is then a registry entry plus, at most, one new section.
A type needing a screen of its own is the signal that the section vocabulary is
missing something, and that is what gets fixed.

**The registry belongs on the server**, as `GET /projects/{id}/reports/types`.
It is resolved per project because a type's join rule text depends on project
settings (the space key, `room_attribution`, `room_resolution`). The page
renders what it is sent, so the rule is stated in one place, the same place it
is enforced. That route is a read, so it gets its MCP tool too.

## The QA CSV moves here, and the viewer keeps a link

**The viewer's QA band builds its CSV in the browser** (`buildValidationCsv`
in `static/index.html`, behind the `qaDownload` button). That is precisely the
second formatter the CSV decision above rules out: the same findings, rendered
by different code from the report that will serve them, drifting the first time
either is fixed.

So the **Checks** family carries it, and `qaDownload`, `downloadValidationCsv`
and `buildValidationCsv` are **deleted in the same change that ships the checks
reports** — the `settings.html` precedent again, and the reason to do it in one
change rather than two: a viewer that has lost the download before the reports
page can serve it is a regression, however short-lived.

**The band itself stays.** It answers "is this project clean right now" at a
glance, next to the plan, which is not what a report is for. What it loses is
the export, and what it gains is a line under it linking to the reports page —
the same "a fallback nobody can see" instinct the layer toggles follow.

**Four checks, and each already has its finding logic server-side** except the
ceilings one, which is the report `STRATEGY-ENTITIES.md` has been holding open:

| Check | Where the findings come from |
|---|---|
| Reference data | `service::validation`, what the QA band shows today |
| Openings with unresolved rooms | `door_report`, keeping **pending** distinct from **dangling** |
| Unmatched spaces and rooms | `SpaceReport`, both directions plus ambiguous keys |
| Rooms without a ceiling | Unbuilt. The probe already said what it must not say: 12 of House A's 32 rooms have no ceiling and nearly all are POOL, DECK or DRIVEWAY, so a classification filter is what makes it readable, and unattributed ceilings cluster by type rather than scattering |

**A check's options are what it refuses to call a finding**, and they are the
report's own, not project settings: "ignore rooms classified External",
"include pending references", "ignore ceilings under 5 sqft". A check with no
options would either bury its signal in the expected (the ceilings case) or
report a legitimate state as a fault (the pending case).

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

**Text comparison ignores case unless the condition says otherwise** (decided
2026-09-18). Every text operator — is, is not, contains, starts with, ends
with — is case-insensitive by default, with a **Match case** toggle on the
condition for the rare time someone means `CLG` and not `clg`. Stored as a
`case_sensitive` flag on the predicate, defaulting false, so the default is
also what an unset field means. A modeller's capitalisation is not data, and a
report that silently misses `Plasterboard` because the user typed
`plasterboard` is wrong in the way that never gets noticed.

That is one rule everywhere, so **`=` in `?filter=` becomes case-insensitive
too**, matching the `~` beside it. It is a behaviour change for programmatic
callers, and the honest one: the alternative is two spellings of equality that
differ invisibly. The query string has no way to *ask* for case sensitivity;
`==` is the obvious spelling if anything ever needs one, and nothing does yet.

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

**That last rule stays for a ROOM property, and the form has to show it**,
because a user will not expect it: **`Department is not Living` also drops
every room with no Department.** It is the right rule, since a missing value is
not evidence of anything. But a "does not" condition in the builder carries a
one-line note and a **Keep blanks too** action. The action rewrites the
condition into `any(is not Living, is blank)`; the rule itself does not change.

### A condition on the associated side asks about the SET, not the row

Decided 2026-09-18, and it is the rule the missing-value rule cannot give:

> "Ceiling type is X" and "ceiling type is not X" are not each other's
> complement over one row. They are `EXISTS` and `NOT EXISTS` over the
> **room's matched ceilings**.

- **`Ceiling.Type is X`** keeps a room when at least one of its ceilings is X,
  and keeps that ceiling's rows. **A room with no ceilings is not a match** —
  it cannot have one of type X. This holds even with "include rooms with no
  ceilings" ticked: an explicit condition outranks a default.
- **`Ceiling.Type is not X`** keeps a room when **none** of its ceilings is X,
  and **a room with no ceilings is a match**: nothing it has is X. So the room
  is dropped whole if *any* of its ceilings is X — the negation quantifies over
  the set, and per-row negation would answer a different question, keeping the
  room because of its other ceilings.

**Per-row evaluation is the trap here.** "Rooms without a type X ceiling" is
the question people actually ask of this data, and a per-row filter answers
"ceilings that are not X", which lists almost every room. The two differ only
on the rooms the user cares about.

So the two unmatched-row switches state the **default**, and a condition on the
associated side overrides it for the rooms it speaks about: a positive
condition excludes rooms with none, a negative one includes them. The preview
says which is in force, for the reason every fallback in this codebase
announces itself.

An **any** group mixing a room condition with a negative associated one is the
one shape whose reading is genuinely ambiguous. The builder keeps negative
associated conditions in **all** groups and says so, rather than picking a
quantifier the user cannot see.

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
- **A room lies on SEVERAL floors, and that is correct data.** `OST_Floors` is
  a room's whole build-up — joist zone, timber build-up, finish, insulation,
  and outside the building lawn, paving, driveway, kerb — and `Structural` does
  not separate them. House A also hosts its roof build-ups on LEVEL 01, so BED
  01 01.06 lies on 7 floors, 3 of which are its roof. **Which one is "the
  floor" is the report's question, not the join's**, so a floors report carries
  `Type` and `height_offset` as columns by default and expects the filter to do
  the rest. Anything that picked one floor per room here would be re-deriving
  the storey rule the entity deliberately does not re-derive.
- **Spaces are one room to several spaces.** One services file per discipline
  means a room legitimately matches a mechanical and an electrical space.
  `model_id` must be available as a column or a grouping, for the reason
  `SpaceResponse::model_id` gives: without it, two disciplines read as a
  duplicate.
- **FF&E is zero-or-one**, the easy case.
- **An opening is zero-to-two, and which room it is under is a policy** —
  `to_room_then_from_room` by default, and `both` attributes the same door
  twice. So a doors report's row count moves when `[doors] room_attribution`
  moves, with nothing pushed and nothing stored; the join rule line has to
  state the live policy, and the `to`/`from` side belongs on the row as a
  measure. **An external door or a facade-package window is homeless**, which
  is the ordinary case rather than an edge one: RHH's facade model holds no
  rooms at all, so every window in it reports no room and matches no
  `?building=`. That is what "include openings with no room" is for, and why a
  doors report defaults it on where a ceilings report does not.
- **"Never pushed" is not "no rows".** `/windows?project=RHH` answers 200 with
  an empty list because `windows_export_entry` has never run there, and a
  report that renders that as an empty table tells the reader their filter is
  wrong. The entity reads already distinguish the two — nothing ever pushed is
  the service's `None`, a 204 — so the report says "no windows have been pushed
  for this project" and never dresses it as a result.
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

### CSV is rendered here too (decided 2026-09-19)

**A `format` parameter on the report read, not a second route and not a
client-side writer.** `format=csv` renders the same rows the JSON form
returns.

- **One formatter, one answer.** A CSV built in the browser is a second
  implementation of quoting, number formatting, column order and how "no room"
  is spelled, and the two drift the first time one is fixed. The MCP host and
  the download would then disagree about the same report.
- **MCP gets it by being one route.** The one-tool-per-read-route rule means a
  second CSV route would mean a second tool; a `format` argument on the report
  tool keeps it at one, and a host can ask for CSV directly. The MCP process
  reads the store itself, so this works with no HTTP server running — and it
  returns the CSV as text, since that process writes nothing.
- **It can stream**, which the browser cannot do while building a string in
  memory. That is what takes 39,000 FF&E rows off the volume question below.
- **A saved report has an id, so its CSV has a plain URL** —
  `GET /projects/{id}/reports/{report_id}/rows.csv` — bookmarkable, and
  fetchable by a script that never opens the page. An unsaved definition still
  goes through the POST, and the page turns that response into a download.
- **What has to be decided once, in one place:** quoting and escaping, the
  header row's names, how a blank and an unattributed row differ, and how many
  decimals an area carries. Those are the choices that make two formatters
  drift, which is the argument above, so write them down where the formatter
  lives.

## Saved reports are documents, not settings

**Server-side**, which is [Browser](STRATEGY-BROWSER.md)'s standing answer to
"users re-pick the same columns" rather than `localStorage`. **But one JSON
document per report, beside the project settings — not a `[[reports]]` block
inside them** (decided 2026-09-18).

**The line is what a thing does to an answer.** A *setting* changes what every
read means: `room_attribution` re-owns every door, the space key re-matches
every space, the area policy re-measures every room. A saved report changes
nothing — delete one and every other answer in the system is identical. That is
the test for the next thing that wants a home: if removing it would change an
answer somewhere else, it is a setting; if it only stops a question being asked
twice, it is a document.

Four consequences follow, and each is a cost the settings file would carry for
no gain:

- **Validation blast radius.** A settings file is validated through
  `bootstrap::load_project_bundle` on save *and* on every boot, which is what
  makes "a file this API accepts can never fail the next boot" true. Putting
  report definitions in it means a malformed report can fail a project's
  settings — so a saved question could stop rooms being served. As a separate
  document, a broken report is one broken row on one page.
- **Read-modify-write.** Every settings save rewrites the whole file and
  hot-swaps the registry. That is the exact path that silently emptied every
  milestone's pins, and `merge_over_stored` exists because of it. One file per
  report means a report save touches one report, and two people saving
  different reports cannot clobber each other.
- **The settings page's exhaustiveness rule keeps its teeth.** A field on a
  settings type with no control there is a type error *on purpose* — the page
  cannot silently stop exposing a setting. Reports would be the first
  deliberate exemption, and an exemption is how that rule starts eroding. The
  reports page is their editor; the settings page never needs to know they
  exist.
- **TOML is the wrong shape for a filter tree.** Arbitrary nesting is where
  array-of-tables syntax turns unreadable, and the TOML ordering footgun in
  [Coding Conventions](CODING-CONVENTIONS.md) lives in exactly that shape. JSON
  holds a tree natively and round-trips what the builder produced.

**Where:** beside the project settings, in the settings directory — not in the
snapshot store. The store's discipline is append-only history, and a report is
edited in place; a mutable document in an immutable store is a rule waiting to
be broken. Sketch: `<projects_dir>/reports/<project-id>/<report-id>.json`,
written temp-then-rename, id checked with `is_path_safe_component` like every
other path component from a request. Its CRUD is the reports page's own routes
(list, read, save, delete), not `/api/settings`.

**The definition type still lives in `roommate-shared` with ts-rs**, so the
reports page's TypeScript is generated and the committed-copy CI gate covers
it. What it does *not* do is join `Settings`.

A sketch of one document:

```json
{
  "name": "Ceilings by room",
  "type": "by_room.ceilings",
  "shape": "per_match",
  "include_rooms_without": false,
  "include_unattributed": true,
  "room_columns": ["Number", "Name", "Level"],
  "associated_columns": ["Type", "Height Offset From Level"],
  "measures": ["overlap_area", "fraction_of_room"],
  "filter": {
    "mode": "all",
    "items": [
      { "side": "room", "field": "Level", "op": "eq", "value": "LEVEL 00" },
      { "mode": "any", "items": [
        { "side": "ceilings", "field": "Type", "op": "contains", "value": "plasterboard" },
        { "side": "join", "field": "fraction_of_room", "op": "ge", "value": "0.9" }
      ] }
    ]
  }
}
```

## Open questions

- **Row volume: the preview is capped, the CSV is not** (decided 2026-09-19).
  One row per match over RHH FF&E is ~39,000 rows. The page asks for the first
  N and says what it is showing -- "first 500 of 39,412" -- and anyone who
  wants all of it takes the streamed CSV. Honest about what the cap buys:
  payload and browser memory, not server work, since the join still runs to
  produce the count.

  **Streaming the JSON instead is the answer to reach for last.** An MCP tool
  returns one `CallToolResult`, so the consumer that justified rendering CSV
  server-side cannot consume a stream at all; the preview holds every row in
  the DOM regardless unless it virtualises; and no read streams today --
  `assemble_rooms` and `assemble_items` are value-in, value-out, and the 304
  path wants the cursor before any row. (Ingest's `put_streaming` is not the
  precedent it looks like: it writes bytes through and assembles nothing.)
  What would reopen it: a projected body measured past ~20 MB, or a streaming
  read arriving for another reason, at which point reports ride it rather than
  lead it.

  Still unmeasured, and worth measuring before building even the cap: the read
  projects to the requested columns, so those rows are a few MB rather than the
  273 MB `/ffe` taught everyone to fear.
