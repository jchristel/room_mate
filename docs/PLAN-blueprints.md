# Plan: blueprints (room layouts that travel)

**Status: stages 0 to 5 built and a first cut of stage 6 (PR #215); the box over the main plan and everything toward Revit are not. Revised 2026-10-19 after the author's review comments and
the stage 0 results ([PLAN-blueprints-stage0.md](PLAN-blueprints-stage0.md)).**

Built so far (PR #215): FF&E carries `family_name` and `host` (stage 1, read from Revit by the
extractor, **not yet run in Revit**); the blueprint store with append-only revisions, pending pushed
revisions and its HTTP and MCP surface (stage 2, without import/export); capture from a room (stage 3's
service and endpoint, without the "Create blueprint" button or list panel); and track B's first
slice: the sheet library on its own endpoint with identity matching, pending renumbered pushes and
`suggest`, plus the Sheets and Blueprint pyRevit buttons (also **not yet run in Revit**). The resolver (stage 4) is built too: measured on RHH's real siblings, 90% of ensuite pairs
land the items within 150 mm of the real ones, `1 BED B` 68%, `1 BED A` 58%. The blueprint viewer (stage 5) is built as a page at `/blueprints/`: list, revision timeline with
pending revisions, the room's outline and FF&E, and an inspector for dimension constraints; the
resolver also accepts a manual edge map. **Stage 6 is built as a mode of that page, not in the main viewer**
("Assign to rooms"): pick a room, see the resolver's answer drawn in it with the room's doors and where the
blueprint's land, choose between the rotations that fit (best by doors first) or map the edges by hand, and
save an assignment (one per room, pinning a revision; the request is stored, never the placements). Doors are
captured with the blueprint for orientation only, and break ties of shape. Not built: dragging, rotating or
scaling a box over the main plan, per-item overrides, assigning several rooms at once, a mirrored copy,
the blueprint's own sheet association, tags/scale/template in
the sheet export, and anything toward Revit.

## What exists, and what does not

Grounded in the code as of 2026-10-18:

| Need | State |
|---|---|
| Room outline, level, properties | Have. |
| FF&E: type, category, `insertion_point` (plan, mm), `facing`, owner room | Have. |
| FF&E footprint (`loops`, the instance's oriented box in plan) | Have: 15,826 of 16,161 items (98%) in RHH `HOS-INT_L1-L2` (probed 2026-10-18). The "empty on every item" comments in `renderer/types.ts` predate it and are stale. |
| FF&E instance and type **properties** (every parameter, as exported) | Have. The probe found `Elevation from Level` on 100% of items, `Host` on 99.9%, `Offset from Host` on 99.4%, `Mounting Height` on 59%. |
| FF&E **vertical extent and sizes** | **Not needed as separate fields** (decided): they are part of the type and instance properties, which a blueprint item carries as exported. |
| FF&E **hosting kind** | Weak: `Host` is a display string (`Level : LEVEL 1`); which wall, and where on it, is not stated. **To be added to the exporter** (decided), via duHast. |
| Family name | **Not exported.** `type_name` is on every item; a `Family` key exists on 2 of 16,161 and holds something else. Comes from Revit (`FamilyInstance.Symbol.FamilyName`), so from duHast. |
| Units of parameter values | Display strings (`"0.0"`, `"420.0"`), not typed. The trap `CLAUDE.md` records for the ceiling height offset. |
| Doors: position, `through_wall_normal`, width | Have. |
| Authored, per-project, user-made data | Precedent: connections document (zones, passages, saved routes, reaches): one JSON per project, optimistic concurrency, replace-whole-list, "authored data outlives geometry; a stale reference is a finding, never an error". |
| Revit extractor | pyRevit, **one-way** (Revit → server). |
| Anything that sends work **to** Revit | **Does not exist.** A new channel, not an extension. |
| Room layout sheet data | duHast has **rudimentary IronPython exports** (not yet surveyed; first task of track B). Nothing is in the server contract. |

So blueprints are three products in one: a constraint model, an editor, and the first
write path into Revit. They have very different risk, which drives the staging.

## Design decisions

1. **A blueprint is stored as a recipe, not as coordinates.** Every placed thing is
   described in the room's own frame: *which edge, how far along it (a fixed distance from
   either end, or a percentage), how far off it, which way it faces relative to the edge
   normal*. The room's current outline plus a recipe gives coordinates; nothing derived is
   stored (the rule `reach` and `passage` follow).
2. **One resolver, in Rust under `service/`.** The blueprint viewer, the assign UI and the
   Revit task all call the same function. Two copies of "where does the bed go" is how a
   preview and a Revit result disagree. The viewer POSTs and draws the answer, as it does
   for reach.
3. **Placement findings, never refusals.** An item that falls outside the room, overlaps
   another, or whose edge no longer exists is a reported state on that item. The resolver
   always returns the placements it could make.
4. **Anchors are edges and doors, plus runs.** v1 relates items to room edges and door
   positions, and to one item-to-item relation only: an item may *follow* the previous item
   in a run (abut it) or be a child of a containing item. Stage 0: 183 of 256 new overlaps
   when a room shrank 15% were between items that had been within 50 mm of each other
   (casework runs, units inside wardrobes), which independent percentages squash. Every
   other item-to-item relation stays out.
5. **Items are the FF&E record as exported** (decided 2026-10-18): category, type name,
   instance and type properties verbatim, plus location and rotation relative to the room.
   Matched in another room by **family, then type name, then instance properties** (type
   properties expected equal), never by `type_id`, which is per Revit document. A missing
   type is a reported state with a mapping table the user can fill.
6. **Scaling is the room's size and proportions, never the items'** (decided 2026-10-19).
   Items keep their exported size. A *fixed* constraint keeps its distance in mm; a
   *percentage* constraint stretches with the room, measured along the edge it is anchored to, not the
   assign box's axes (decided 2026-10-19).
7. **No mirroring at assignment.** A mirrored layout is a different blueprint: handed items
   are most likely a different family, and mirroring them would cause placement problems.
   Rotation is supported; mirror is not offered when assigning (decided 2026-10-19). Stage 0
   shows handedness is common (37% and 43% of ensuite pairs on RHH are mirror images), so the
   blueprint viewer offers **Create mirrored copy**, which writes a second blueprint the user
   can then correct (swap the handed families), and the resolver reports "this room is the
   mirror image of the blueprint" instead of resolving badly.
8. **Blueprints are a separate endpoint**, not part of the connections document
   (decided 2026-10-19). Same write rules as connections (optimistic concurrency), own
   routes, own storage.
9. **Revisions.** A blueprint has an ordered timeline of revisions, one marked *current*.
   Constraint edits are stored against the selected revision only. A revision can be made
   current by the user. An assignment pins a revision and shows "newer revision available".
10. **A pushed blueprint never overwrites an active one.** A push (import, or later a
    Revit-originated one) is stored as a pending revision on the timeline; the user
    reviews it and either accepts it as the new current or rejects it.
11. **Removing an item from a blueprint marks it excluded; it is never deleted.** It can be
    added back, and an earlier revision's settings can be imported onto it.
12. **Revit stays the source of truth for the model; the blueprint is the intent.** Items
    written from a blueprint carry a marker (blueprint id, revision, item id), so a re-run
    modifies instead of duplicating, and **drift from the blueprint becomes detectable**
    (a QA report falls out of this, and may be the first thing people value).
13. **The Revit side is pull-based and defaults to a dry run.** The server holds tasks; a
    Revit button fetches, shows a diff (adds, moves, removes), and only writes on
    confirmation, in one transaction per room, reporting results back per task.
14. **The reference outline is a general polygon, held at two tolerances** (stage 0).
    The standardised repeated rooms on RHH (patient rooms, ensuites) are 5 to 9-edge angled
    polygons, so rectangles are a case, not the scope. A blueprint stores a **coarse outline**
    (jogs under about 350 mm removed) for matching and edge mapping, and the **true outline**
    (about 30 mm) for checking findings: on the coarse outline items near column jogs fall
    outside it (1% of `1 BED B` items on the fine outline, 21% on the coarse).
15. **A blueprint fits a room by shape, not edge count.** Edge lengths plus turn angles,
    cyclic shifts only (no reflection), with the door position choosing between rotations
    that score the same. Even among same-named patient rooms 14 to 54% of pairs differ in
    edge count because of jogs at columns and risers.
16. **Wall devices are their own class of item.** Items under 100 mm thick (43% of House A's
    usable items: switches, sockets, rails) are placed by wall, along-distance and height,
    carry no footprint check, and are left out of overlap findings. Nested components
    (`super_component_id`) are excluded from capture; they move with their parent.
17. **Platform: pyRevit** (decided 2026-10-19). The Revit side stays in the existing
    pyRevit extension; no C# add-in and no platform spike. Revisit only if a stage 7 or 8
    obstacle forces it.
18. **Room layout sheets are a library with their own endpoint** (decided 2026-10-19), a
    collection of sheet objects beside blueprints, not inside them. Sheets saved there are
    kept as exported and are stored separately from the sheet association a blueprint
    carries (confirmed 2026-10-19). The library exists so the server can offer the closest
    sheet for a room.
19. **The user chooses which sheets are exported, and which room each belongs to.** Nothing is
    inferred from sheet names or parameters: the export has no marker for a room layout sheet
    and no link from a sheet to a room. The room is named **at export** when the sheet goes out
    through the blueprint route (the blueprint already knows its room), and **afterwards, in the
    viewer,** for rooms no blueprint covers, where the UI suggests a match and the user makes
    the choice.
20. **A sheet reaches a room in one of two ways.** *Direct:* the user associates sheets with a
    blueprint, and that association is exported with the blueprint. *Suggested:* in sheet
    creation mode only, the server ranks library sheets by how close their room is to the
    target room (size, proportions, number of items, as duHast's `RoomLayoutSheet` sketch
    intended), the three weighed **equally** (decided 2026-10-19). A suggestion is offered,
    never applied, and never made outside sheet creation.

### Constraint grammar (sketch; stage 0 added `follows` and `class`)

```
Placement {
  edge:    index into the blueprint's reference outline
  along:   Fixed(mm, from start) | Fixed(mm, from end) | Percent(0..100)
  off:     Flush | Fixed(mm) | Percent(0..100)   // distance into the room
  facing:  ToEdge | AwayFromEdge | Angle(deg, relative to edge normal)
  z:       FromFloor(mm)      // mounting height; not constrained to the room
  follows: None | ItemRef(gap mm)   // runs: abut the previous item, or sit inside a parent
  class:   Free | WallDevice        // wall devices: no footprint, no overlap check
}
```

Every dimension can be switched between a fixed value and a percentage by the user, and a
change moves the object. An item may carry a second anchor ("2 m from this corner" is
`Fixed(2000, from start)` on an edge) and a list of edges it may move to when the room
does not match. There is no mirror field. Defaults for the fixed-or-percent rule, from stage 0:
an item within 300 mm of a corner keeps that gap, an item within 150 mm of its wall keeps that
gap, anything else is a percentage (both thresholds user-adjustable).

## Stages

Each stage ends in something usable and reviewable. The blueprint viewer (the editor)
moves ahead of the Revit write path from the earlier draft: it is where constraints are
authored and checked, and the author has now specified it concretely.

**Stage 0 — Prove the model on real data (no product code). Done 2026-10-19** on House A and
RHH: [PLAN-blueprints-stage0.md](PLAN-blueprints-stage0.md). Capture and resolve with throwaway
scripts reproduce a captured layout exactly on its own room and, on RHH patient rooms and
ensuites, land the median item within 27 to 97 mm of the real item in a sibling room (74 to
96% of shape-compatible pairs within 150 mm). Findings folded into decisions 4, 7 and 14 to
16. The platform question is decided (pyRevit), so there is no spike.

**Stage 1 — Close the export gaps.** What a blueprint item holds is decided (the item as
exported). Two small requests go to duHast, not `room_m`: **family name** and **hosting
kind** (which wall or face, and where). Until the family name lands, matching is on type
name. Probe on House A as ceilings and floors had before relying on either. *Needed by 3
and 4.*

**Stage 2 — Blueprint store and API (own endpoint).** Blueprint document (id, name, ordered
revisions, current revision, reference outline, items with excluded flag, doors, a reserved
and undesigned slot for sheet data), assignment records (room ref, pinned revision,
placement of the box), pending pushed revisions. Optimistic concurrency, MCP tool parity per
the one-tool-per-read-route rule. **Import and export between projects on the same server**
(cross-server later). *No UI yet.*

**Stage 3 — Capture from a room.** `service::blueprints::capture(room)` reads the room's
outline, FF&E and doors and proposes recipes: each item snaps to the edge it lies beside
(the nearest edge whose span contains it; nearest-segment distance failed on notched rooms),
records along/off distance, and picks fixed or percentage by the decision 4 and grammar
rules. Nested components are excluded, wall devices get their own class, and abutting items
become runs. Both outlines are stored (decision 14). A "Create blueprint" action on a
selected room. *Value on its own: a catalogue of the project's repeated layouts.*

**Stage 4 — Resolver.** Blueprint revision + target room + transform (rotation, edge
mapping by shape, bounding-box scale) to placements plus findings, including the
mirror-image case (decision 7). Pure and heavily unit-tested with
the stage 0 fixtures. The core: everything after depends on it being right.

**Stage 5 — Blueprint viewer (a new view in RoomMate).**
- **Left pane:** the list of blueprints, single selection, like routes and reaches.
  Import and export of blueprints (same-server projects for now).
- **Centre:** the room outline drawn as a polygon with a circle at each vertex, the
  FF&E and the doors, with an on/off toggle for FF&E tags.
- **Top:** a timeline of the blueprint's revisions. Select another revision and set it
  current. Pending pushed revisions appear here to accept or reject.
- **Right pane:** appears on selecting an item. Remove from blueprint (marks excluded), add
  back, and the item's dimension constraints. **Create mirrored copy** of the blueprint.
- **Constraints:** shown only for the selected item. Each can be switched between fixed
  and percentage, and a change moves the object (the stage 4 resolver, live). Stored against
  the selected revision only. Import the dimension settings from another revision, with
  multi-select for bulk updates.
Built on the Pixi renderer already in use. *The biggest UI piece; the first with a real
brief.*

**Stage 6 — Assign UI in the main viewer.** Flow: select the rooms a blueprint applies to,
select the blueprint, then place a bounding box that shows the blueprint's outline and door
for orientation; the user drags, rotates and scales it to match the real room, with snap
to endpoints. Ghost items drawn over the plan, findings listed and clickable, assignment
saved. Scaling the box scales the room reference only (decision 6). Reuses the zone/overlay
machinery. *Value without Revit: planning and coverage ("which rooms follow which
blueprint").*

**Stage 7 — Revit write, thin slice (the walking skeleton).** Task model on the server (a
task is an assignment plus a snapshot of resolved placements and a state). A new pyRevit
button "Apply blueprint tasks": fetch, dry-run diff, confirm, place **one family instance
type** with location, rotation and marker, report back. Prove the transaction, the marker,
idempotent re-run and the results channel before the rest. Fake-Revit interface so the
executor is testable in CI. *The Revit write loop is the largest unknown and is deferred
behind the viewer. Platform is pyRevit (decision 17).*

**Stage 8 — Revit write, full content.** All item kinds including wall-hosted and
face-hosted (needs stage 1's hosting kind), doors (by host wall and offset), moves and
removals of previously placed items, missing-type mapping, per-room results with reasons.

**Stage 9 — Elevation and 3D preview in the browser (optional).** Boxes from item
properties; only if track B wants to show a sheet before generating it.

## Track B: room layout sheets (separate task, own plan)

Not part of stages 0 to 9 and not blocking them. Blueprints are useful without it
(catalogue, planning, Revit placement). It joins the main track at one point: a blueprint
may carry a sheet specification, and stage 2 reserves the slot without designing it.

**Scope for now, decided 2026-10-19:** review the duHast code with a view to using the
sheet export code in RoomMate, and make it a **one-way proof of concept, Revit to
RoomMate**, into a **sheet library with its own endpoint** (decision 18). Creating sheets in
Revit (B4) stays in the plan but is not scheduled.

**B0 — Review the duHast sheet exports.** Read the existing IronPython sheet code and record
exactly what it returns: sheets, views and view types, viewport positions and extents, view
templates, tags and their positions, schedules. Output: a note of what is real, what is a
stub and what is missing, in the style of the door and ceiling probes (run against a House A
sheet set; "which duHast" means the one loaded in memory, see `CLAUDE.md`). *Everything
else in this track depends on it.*

**B0 findings so far (read 2026-10-19, SampleCodeRevitBatchProcessor `370038c6`, code only,
nothing run in Revit):**

- **Real:** `sheets_to_data.get_all_sheet_data` exports per sheet its instance properties,
  its viewports and its schedule instances. A viewport carries its **sheet position**
  (`GetBoxOutline` min/max and `GetBoxCenter`, as 2D points), a type (floor plan,
  elevation, 3D or schedule; other view types are dropped), and per view: plan, elevation
  and 3D crop box; elevation marker index and view direction; 3D eye, forward and up.
  A schedule instance carries its position, row count and segment heights. A title-block
  size name exists (`get_title_block_size`), matched against supported sizes.
- **Stub:** `Data/Objects/BluePrints/room_layout_sheets.py` (`RoomLayoutSheet`: room size,
  proportions, item count, sheets) parses nothing and its `__eq__` compares attributes it
  does not have. It is a sketch of matching a blueprint to a room by size and proportions.
- **Missing:** tags and their positions (nothing exists; a comment in the plan-view
  export marks the place); view scale; view template; view name and view type name;
  viewport type name; the schedule's bounding box (`# TODO`); the viewport's `view_id`
  is never set by the converter, so it stays `-1`. Units of the 2D points are not yet
  confirmed (feet from the API unless converted).
- **Implication:** enough for a one-way proof of concept that records where each view sits
  on a room layout sheet. Creating sheets (B4) additionally needs view type, scale and
  template names, and tag capture.

**B1 — Export button (extractor).** A pyRevit button lists the document's sheets; the user picks
the ones to export, and names the room when it is exporting through the blueprint route
(decision 19). Each chosen sheet goes through
duHast's `convert_revit_sheet`. It can be pushed in the same run as FF&E, since a run already
exports several entities at once. The export carries the sheet's identity (Revit element id
and sheet number), its properties, viewports with sheet positions, crop boxes and schedules.
Not carried, because duHast does not export it: tags and their positions, view scale, view
template, view names.

**B2 — Sheet library (server).** A `sheets` collection on its own endpoint, one record per
sheet, scoped to a project. Each record holds the sheet as exported and the room the user
named for it (a reference, never validated against the model on save; it may be set later in
the viewer). **A sheet is identified by its Revit sheet id plus the document's title** (decided
2026-10-19). A sheet pushed again that matches on both is the same sheet and is updated; **if it
matches but its sheet number has changed, the push is stashed as pending and the user is asked
to confirm** (accept or reject, as a pushed blueprint is, decision 10), so a renumbering cannot
silently rewrite a sheet someone relies on. A sheet that matches on neither is a new sheet, even if its number matches an existing one.
Both readings confirmed 2026-10-19.
Reads: list, one sheet, and **suggest**.

**B3 — Suggest, in sheet creation mode only.** `GET .../sheets/suggest?room=` ranks library
sheets by the closeness of their room to the target room: area, proportions (the outline's
aspect) and number of items, **weighed equally**, each worked out at read time from the rooms
and FF&E, never stored. It returns the ranking with the figures, so a person sees why a sheet was offered.
Nothing applies it.

**B3b — Associate sheets with a blueprint.** A blueprint carries a list of sheet references (the
direct route, decision 20), and exporting a blueprint carries the sheets it names. Needs the
blueprint export, which is not built.

**B4 — Revit creation (not scheduled).** A task kind on the stage-7 channel: views from the
project's own templates and title block (named in the spec, never invented), viewports,
tags, schedule. Reuses the dry-run, marker and per-task result rules.

**Open for this track:** whether title block, view templates and sheet naming are common
across projects; whether the duHast exports are a base to extend or a prototype to
replace. (The platform is pyRevit; sheet creation weighs on it more than placement
does, so B4 is where that choice would be tested.)

## Critique

**What is most likely to go wrong, in order:**

1. **Shape variation is the main matching risk, not rectangles.** Stage 0 showed the recipe
   model works on the angled 5 to 9-edge rooms that actually repeat, so non-rectangular is not
   the problem. What is: same-named rooms differ in small jogs (edge count differs on 14 to
   54% of patient-room pairs), a third to a half of ensuite pairs are mirror images, and
   same-shape rooms of non-standard types (toilets, consult rooms) do not share a layout at
   all. Mitigation: shape matching on the coarse outline (decisions 14, 15), the mirrored
   copy action, and findings for every case that does not fit.

2. **The data is thick, with three known gaps.** Footprints and the common elevation
   parameters are there; sizes and heights are in the properties by decision. What is
   missing is the family name and the hosting kind, and the units of parameter values are
   display strings. Any exporter change must come from duHast or the API (`CLAUDE.md`: do
   not re-derive in `room_m`); give stage 1 its own probe on House A. (An earlier draft said
   the export lacked footprints and properties; that claim was wrong and is corrected.)

3. **The Revit write side is a different kind of software.** Everything so far reads. Writing
   means transactions, worksharing (elements checked out by others), families not loaded,
   hosted families needing a host, and a mistake changes a real model. Mitigations are in
   the design (dry-run default, markers, one transaction per room, per-task results). The
   cost of moving the viewer ahead of it (stages 5 and 6 before 7) is that this risk is found
   later. Staying on pyRevit (decision 17) keeps the Revit side in the extension already
   shipped, which is the cheaper way to find out.

4. **Handed items and rotation.** With mirroring removed, the remaining hazard is a blueprint
   applied to a room whose geometry only fits it rotated by something other than a right
   angle. Rotation is supported; the resolver must report items that land outside or on
   another after rotation, never silently clip them.

5. **Platform is pyRevit, by decision.** pyRevit (IronPython) is what the repo ships and where the
   buttons live; the cost is weaker support for transactions at scale and for sheet
   automation than a C# add-in. Accepted for now (2026-10-19). Reopen if a viewport or tag
   cannot be created reliably, which is B4's question, not a prerequisite.

6. **Scope.** Several months and three audiences (planner, designer, documenter). The
   staging gives value at 3 (catalogue), 5 (viewer and revisions), 6 (planning) and 7/8
   (Revit automation) independently, so it can stop at any of them. Sheets are split into
   track B for the same reason: a second product with its own unknowns.

7. **Authored data is unreproducible.** Blueprints are exactly the kind of thing
   `STRATEGY-AUTHORED.md` warns about. Revisions are immutable once assigned or pushed,
   backups use the existing mechanism, and the connections store is already flagged as
   provisional. Blueprints get their own endpoint (decision 8), which makes the storage
   question sharper, not smaller: decide it once for both, rather than adding a second
   provisional store. Raise it at stage 2.

8. **Pushes and revisions are a small version-control system.** Pending revisions, current
   marker, per-revision constraint settings and cross-revision import is more state than
   it looks. Keep revisions append-only, never edit one in place, and make the timeline the
   only way to change which is current.

9. **Security.** The server tells a Revit user's machine to change a model. Tasks come from
   authored data, so the plugin must validate everything it is told (types, ids, ranges),
   never execute free-form content, and always show the diff. Cross-check with
   `STRATEGY-SECURITY.md` before stage 7 ships.

10. **Testing.** Revit cannot run in CI. The executor is split into a pure planner (task +
    current model facts to operations) and a thin Revit adapter; the planner is tested
    against JSON fixtures, the adapter by hand against a sample model. Without this split
    stages 7 to 9 are untestable.

11. **Naming and expectations.** "Follow to some degree" is a promise a solver must keep
    predictably. Prefer explicit rules a person can read ("2 m from corner A") over
    inference; show every resolved placement with the rule that produced it.

**Things deliberately left out of v1:** mirroring when assigning, item-to-item relationships
other than runs, automatic
fitting of a blueprint to a room (suggesting which blueprint suits which room),
cross-server and cross-project libraries beyond same-server import/export, nested
components, and editing Revit items already in the model that were not placed by a
blueprint.

## Questions for review

1. ~~**"Scaling" in the assign UI.**~~ **Closed 2026-10-19:** room size and proportions
   only; items keep their size (decision 6).
2. ~~**Rooms in scope for v1.**~~ **Closed 2026-10-19 by stage 0:** general polygons. The
   rooms that repeat on RHH are angled 5 to 9-edge rooms, not rectangles (decision 14).
3. **Scope of a blueprint**: same-server import/export is planned. Reusable across
   projects as a shared library, or only copied between them?
4. ~~**Revit platform.**~~ **Closed 2026-10-19:** pyRevit (decision 17).
5. **Who may write to Revit**: anyone using the button, or does a person approve tasks in the
   viewer first (a task state of *approved*)?
6. **Sheets (track B)**: are the title block, view templates and sheet naming standard across
   your projects, or per project? How far were the duHast sheet exports taken (B0 will read
   them, but your view helps)?
7. ~~**Percentage constraints on rotated boxes.**~~ **Closed 2026-10-19:** a percentage
   constraint follows the edge it is anchored to, not the box's axes (decision 6).
8. ~~**Sheet library, three details.**~~ **Closed 2026-10-19:** a re-pushed sheet matches on Revit
   sheet id plus document title, and a changed sheet number is stashed for confirmation; the
   room is chosen at export through the blueprint route and afterwards in the viewer for rooms no
   blueprint covers; "closest" weighs size, proportions and item count equally.
