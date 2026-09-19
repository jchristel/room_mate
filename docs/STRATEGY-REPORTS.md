# RoomMate — Reports

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[Area calculation](STRATEGY-AREA-CALCULATION.md) ·
[Browser](STRATEGY-BROWSER.md) · [MCP](STRATEGY-MCP.md) ·
[Authored](STRATEGY-AUTHORED.md) · [Entities](STRATEGY-ENTITIES.md) ·
[Security](STRATEGY-SECURITY.md)

**The page, its reports and the filter are built** (as of 2026-09-19).
`/reports/` serves milestone comparison, three QA checks, a schedule of every
entity, the by-room reports and the filter builder over `service::reports`.
Those document themselves, and the pickers now read a served vocabulary rather
than guessing. What is left here is saved report definitions, the debts under
"What the server still owes a report", and the design of the parts nobody has
built yet. When a piece ships, its section is deleted from here and
the rationale moves to the module header — see "Code documents what is built"
in [Coding Conventions](CODING-CONVENTIONS.md).

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

## The page, and what slice 1 left behind

`/reports/` is a React app (`src-js/reports/`, `vite.reports.config.ts`) beside
the renderer and the settings page, built by the same `npm run build` and
covered by the same committed-bundle gate. It replaced `comparison.html`, which
is deleted, and took the QA CSV export off the viewer's band.

**Slice 1 renders in the browser, and that is staging rather than the design.**
Comparison and the checks each read one already-computed endpoint
(`/projects/{id}/comparison`, `/projects/{id}/validation`), so nothing is joined
client-side and nothing waits on `service::reports`. The schedules and the
by-room reports are different: those read whole entity payloads, which is where
the server-side section below stops being optional — `/ffe` is 133 MB for every
storey on RHH. **Take that as the trigger, not the date**: the first report type
that needs an entity payload is the one that needs the endpoint.

**What is still owed to the checks**: the ceilings check (no server-side report
exists — see [Entities](STRATEGY-ENTITIES.md)), and CSV rendered server-side so
an MCP host and a download cannot differ. The client-side `csv.ts` that shipped
is one renderer where there were two, which is the improvement; it is not the
end state.

**The save trap did carry over, and is handled.** `comparison.html` PUT back the
*whole* settings JSON it had read, changing only its own two fields, because
rebuilding that JSON field by field is what silently emptied every milestone's
pins. `comparison.tsx` does the same. Anything else this page saves either keeps
the whole-object round-trip or goes through `settings_api::merge_over_stored`.

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

## What the filter cost, and what it left open

The builder ships: `rooms::FilterNode` is the tree every entity read now
matches against, `reports::FilterWire` is how it arrives, and
`src-js/reports/filter.ts` is the editing model. The rules it enforces —
set-wise conditions on the associated side, case folding unless a condition
says otherwise, `is blank` as the only way to ask about a missing value — are
documented where they are enforced.

**One behaviour changed outside reports, and it is the only migration in it:**
`=` in `?filter=` folds case now, matching the `~` beside it. Two equalities
that differ invisibly was the alternative. The query string still has no way to
*ask* for an exact comparison; `==` is the obvious spelling if anything ever
needs one, and nothing does yet.

Left open:

- **Three levels of nesting** is a cap the builder enforces and nothing tests at
  the server, which accepts any depth. It has not mattered; a form deeper than
  that stops being readable long before it stops parsing.
- **A reference label's type is still assumed to be text.** The entity's own
  properties carry Revit's storage type through the dictionary, but a joined
  source's do not: `ReferenceFieldConfig` has a `FieldType` for the fields QA
  compares, and nothing reads it here yet. A numeric dRofus column therefore
  offers text operators.
- **An `any` group holding a negative associated condition** has no single
  reading, so the builder does not offer one there. The server evaluates
  whatever it is sent, which is the looser contract of the two; if a caller ever
  sends one, it means NOT EXISTS.

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

## What the server still owes a report

`service::reports` builds the rows and `POST /projects/{id}/reports` serves
them, with `format=csv` rendering the same rows and a `build_report` MCP tool
beside it. The module documents the three rules it keeps. What is left:

- **Streaming the CSV.** It is built in memory today, which is fine for the
  projected rows a report asks for and is not the streamed export the row-volume
  entry below imagined. Measure before building it.
- **Saved reports**, as designed below, and with them the plain `GET` URL a
  script can fetch:
  `GET /projects/{id}/reports/{report_id}/rows.csv`.
- **Spaces by room.** A space matches a room on a key, project-wide, and that
  match lives in `SpaceReport` rather than on the `/spaces` rows. Re-deriving it
  in a report would give the report and the QA check two answers to one
  question, so the by-room type is disabled until the match is exposed on the
  read. The Unmatched spaces and rooms check answers the question meanwhile.

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
