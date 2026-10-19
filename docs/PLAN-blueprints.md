# Plan: blueprints (room layouts that travel)

**Status: proposed, nothing built. Revised 2026-10-19 after the author's review comments.**
A blueprint is a room, its FF&E, its doors and its room layout sheets, stored as
*relationships to the room boundary* so the same layout can be placed in another room,
rotated, or fitted to a differently sized or proportioned room, and eventually written into
Revit. **Room layout sheets are a separate track** (below): they take the blueprint as an
input but are planned, staged and built on their own.

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
4. **Anchors are edges, to start.** Item-to-item relations are real but multiply the
   solver's difficulty. v1 relates items to room edges and to door positions only.
5. **Items are the FF&E record as exported** (decided 2026-10-18): category, type name,
   instance and type properties verbatim, plus location and rotation relative to the room.
   Matched in another room by **family, then type name, then instance properties** (type
   properties expected equal), never by `type_id`, which is per Revit document. A missing
   type is a reported state with a mapping table the user can fill.
6. **Scaling is the room's size and proportions, never the items'** (decided 2026-10-19).
   Items keep their exported size. A *fixed* constraint keeps its distance in mm; a
   *percentage* constraint stretches with the room.
7. **No mirroring.** A mirrored layout is a different blueprint: handed items are most
   likely a different family, and mirroring them would cause placement problems. Rotation
   is supported; mirror is not offered anywhere (decided 2026-10-19).
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

### Constraint grammar (sketch, to be proven in stage 0)

```
Placement {
  edge:    index into the blueprint's reference outline
  along:   Fixed(mm, from start) | Fixed(mm, from end) | Percent(0..100)
  off:     Flush | Fixed(mm) | Percent(0..100)   // distance into the room
  facing:  ToEdge | AwayFromEdge | Angle(deg, relative to edge normal)
  z:       FromFloor(mm)      // mounting height; not constrained to the room
}
```

Every dimension can be switched between a fixed value and a percentage by the user, and a
change moves the object. An item may carry a second anchor ("2 m from this corner" is
`Fixed(2000, from start)` on an edge) and a list of edges it may move to when the room
does not match. There is no mirror field.

## Stages

Each stage ends in something usable and reviewable. The blueprint viewer (the editor)
moves ahead of the Revit write path from the earlier draft: it is where constraints are
authored and checked, and the author has now specified it concretely.

**Stage 0 — Prove the model on real data (no product code).**
Pick 3 to 5 repeated room types from House A / RHH (patient room, WC, store). Write them as
recipes by hand, resolve them onto their siblings with a throwaway script, and measure:
how many siblings share edge topology, how often an item lands outside or on another, how
much real layouts vary under size and proportion changes. Decide: the grammar above, how to
handle non-rectangular rooms, and the pyRevit-versus-add-in question (see critique 5).
*Output: a short findings note and a fixed grammar. The cheapest way to kill the biggest
risk.*

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
outline, FF&E and doors and proposes recipes: each item snaps to its nearest edge, records
along/off distance, and picks fixed or percentage by a rule (flush against a wall and near
a corner is fixed; central on an edge is a percentage). A "Create blueprint" action on a
selected room. *Value on its own: a catalogue of the project's repeated layouts.*

**Stage 4 — Resolver.** Blueprint revision + target room + transform (rotation, edge
mapping, bounding-box scale) to placements plus findings. Pure and heavily unit-tested with
the stage 0 fixtures. The core: everything after depends on it being right.

**Stage 5 — Blueprint viewer (a new view in RoomMate).**
- **Left pane:** the list of blueprints, single selection, like routes and reaches.
  Import and export of blueprints (same-server projects for now).
- **Centre:** the room outline drawn as a polygon with a circle at each vertex, the
  FF&E and the doors, with an on/off toggle for FF&E tags.
- **Top:** a timeline of the blueprint's revisions. Select another revision and set it
  current. Pending pushed revisions appear here to accept or reject.
- **Right pane:** appears on selecting an item. Remove from blueprint (marks excluded), add
  back, and the item's dimension constraints.
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
executor is testable in CI. *The Revit write loop is the largest unknown and is only
deferred behind the viewer, not behind the whole product: if stage 0 flags the platform
question as risky, pull a spike of this forward.*

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
RoomMate**. Creating sheets in Revit (B4) stays in the plan but is not scheduled.

**B0 — Review the duHast sheet exports.** Read the existing IronPython sheet code and record
exactly what it returns: sheets, views and view types, viewport positions and extents, view
templates, tags and their positions, schedules. Output: a note of what is real, what is a
stub and what is missing, in the style of the door and ceiling probes (run against a House A
sheet set; "which duHast" means the one loaded in memory, see `CLAUDE.md`). *Everything
else in this track depends on it.*

**B1 — One-way proof of concept: contract and storage for sheet data.** If B0 shows the
export is worth keeping, a sheets entity on the existing entity stack (contract, ingest,
storage, read route, MCP): "adding an entity is a variant, not a copy". Read-only.

**B2 — Capture a layout-sheet specification from existing sheets.** Which views (plan,
elevations, 3D, schedule), their types and templates by name, viewport positions relative
to the sheet, tag positions relative to their elements. Same recipe principle as
blueprints: relationships, not absolute coordinates.

**B3 — Attach a spec to a blueprint and preview it.** Resolver output feeds it (which
walls, which items to tag).

**B4 — Revit creation (not scheduled).** A task kind on the stage-7 channel: views from the
project's own templates and title block (named in the spec, never invented), viewports,
tags, schedule. Reuses the dry-run, marker and per-task result rules.

**Open for this track:** whether title block, view templates and sheet naming are common
across projects; whether the duHast exports are a base to extend or a prototype to
replace; and the platform question (critique 5), which sheet automation weighs more than
placement.

## Critique

**What is most likely to go wrong, in order:**

1. **The constraint grammar will meet non-rectangular rooms and break.** "The edge" is
   well-defined on a rectangle and ambiguous on a T. A room that changes shape can gain or
   lose an edge, and then every anchor is stale. Mitigation: stage 0 measures it;
   v1 requires the target room to share the blueprint's edge count and the user maps edges
   explicitly (with a sensible default); anything else is a finding. If stage 0 shows most
   real rooms are non-rectangular, the model needs rethinking *before* stage 2, not after.
   Scaling the box in stage 6 makes this worse: stretching proportions is only well-defined
   when the edges correspond.

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
   later; the one-day platform spike in stage 0 is what keeps that acceptable.

4. **Handed items and rotation.** With mirroring removed, the remaining hazard is a blueprint
   applied to a room whose geometry only fits it rotated by something other than a right
   angle. Rotation is supported; the resolver must report items that land outside or on
   another after rotation, never silently clip them.

5. **Platform choice for stage 7 onwards is not made.** pyRevit (IronPython, what the repo
   already ships and where the buttons live) versus a C# add-in. pyRevit is faster to build
   and matches the installer; a C# add-in is sturdier for transactions, sheet automation and
   distribution. Decide in stage 0 with a one-day spike: can pyRevit create a viewport and a
   tag reliably on the Revit versions in use?

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

**Things deliberately left out of v1:** mirroring, item-to-item relationships, automatic
fitting of a blueprint to a room (suggesting which blueprint suits which room),
cross-server and cross-project libraries beyond same-server import/export, nested
components, and editing Revit items already in the model that were not placed by a
blueprint.

## Questions for review

1. ~~**"Scaling" in the assign UI.**~~ **Closed 2026-10-19:** room size and proportions
   only; items keep their size (decision 6).
2. **Rooms in scope for v1**: rectangular and orthogonal only, or L and T from the start?
   Stage 0 will inform this, but your sense of how common each is on real projects matters.
3. **Scope of a blueprint**: same-server import/export is planned. Reusable across
   projects as a shared library, or only copied between them?
4. **Revit platform**: stay with pyRevit, or is a C# add-in acceptable?
5. **Who may write to Revit**: anyone using the button, or does a person approve tasks in the
   viewer first (a task state of *approved*)?
6. **Sheets (track B)**: are the title block, view templates and sheet naming standard across
   your projects, or per project? How far were the duHast sheet exports taken (B0 will read
   them, but your view helps)?
7. **Percentage constraints on rotated boxes** (new): when the assign box is rotated and
   scaled non-uniformly, should a percentage constraint follow the edge it is anchored to
   (the plan's assumption), or the box's axes?
