# Plan: blueprints (room layouts that travel)

**Status: proposed, nothing built. For review.** A blueprint is a room, its FF&E, its
doors and its room layout sheets, stored as *relationships to the room boundary* so the
same layout can be placed in another room, mirrored, rotated or in a differently sized
room, and eventually written into Revit. **Room layout sheets are a separate track** (below):
they take the blueprint as an input but are planned, staged and built on their own.

## What exists, and what does not

Grounded in the code as of 2026-10-18:

| Need | State |
|---|---|
| Room outline, level, properties | Have. |
| FF&E: type, category, `insertion_point` (plan, mm), `facing`, owner room | Have. |
| FF&E footprint | **Mostly empty** (`loops` is absent on most items). |
| FF&E **height above floor, width/depth/height, hosting** (wall-hosted, face-hosted) | **Not in the contract.** Elevations, schedules-by-size and wall-mounted rules all need these. |
| Doors: position, `through_wall_normal`, width | Have. |
| Authored, per-project, user-made data | Have a precedent: connections document (zones, passages, saved routes, reaches): one JSON per project, optimistic concurrency, replace-whole-list, "authored data outlives geometry; a stale reference is a finding, never an error". |
| Revit extractor | pyRevit, **one-way** (Revit → server). |
| Anything that sends work **to** Revit | **Does not exist.** This is a new channel, not an extension. |
| Room layout sheet data | duHast has **rudimentary IronPython exports** of it (not yet surveyed here; first task of track B). Nothing is in the server contract. |

So blueprints are three products in one: a constraint model, an editor, and the first
write path into Revit. They have very different risk, which drives the staging.

## Design decisions (proposed)

1. **A blueprint is stored as a recipe, not as coordinates.** Every placed thing is
   described in the room's own frame: *which edge, how far along it (fixed from either end
   or a fraction), how far off it (fixed or flush), which way it faces relative to the
   edge normal*. The room's current outline plus a recipe gives coordinates; nothing
   derived is stored (the rule `reach` and `passage` follow).
2. **One resolver, in Rust under `service/`.** Preview in the browser, the assignment UI
   and the Revit task all call the same function. Two copies of "where does the bed go"
   is how a preview and a Revit result disagree. The viewer POSTs and draws the answer, as
   it does for reach.
3. **Placement findings, never refusals.** An item that falls outside the room, overlaps
   another, or whose edge no longer exists is a reported state on that item. The resolver
   always returns the placements it could make.
4. **Anchors are edges, to start.** Item-to-item relations ("bedside table 100 mm from the
   bed") are real but multiply the solver's difficulty. v1 relates items to room edges and
   to door positions only; item-to-item is a later, separate decision.
5. **Items are identified by family and type name plus category, never `type_id`**, which
   is per Revit document. A blueprint made in one model must find its types in another;
   a missing type is a reported state with a mapping table the user can fill.
6. **Blueprints are versioned and an assignment pins a revision.** Editing a blueprint
   must not silently move rooms already assigned; the assignment shows "newer revision
   available".
7. **Revit stays the source of truth for the model; the blueprint is the intent.** Items
   written from a blueprint carry a marker (blueprint id, revision, item id), so a re-run
   modifies instead of duplicating, and **drift from the blueprint becomes detectable**
   (a QA report falls out of this, for free, and may be the first thing people value).
8. **The Revit side is pull-based and defaults to a dry run.** The server holds tasks; a
   Revit button fetches, shows a diff (adds, moves, removes), and only writes on
   confirmation, in one transaction per room, reporting results back per task.

### Constraint grammar (sketch, to be proven in stage 0)

```
Placement {
  edge:    index into the blueprint's reference outline
  along:   FromStart(mm) | FromEnd(mm) | Fraction(0..1)
  off:     Flush | Offset(mm)              // distance into the room
  facing:  ToEdge | AwayFromEdge | Angle(deg, relative to edge normal)
  mirror:  Allow | Never                   // handed families, e.g. a left-hand fixture
  z:       FromFloor(mm)                   // mounting height; not constrained to the room
}
```

An item may carry a second anchor (corner pin: "2 m from this corner" is `FromStart(2000)`
on an edge) and a list of edges it is allowed to move to when the room does not match.

## Stages

Each stage ends in something usable and reviewable. Dependencies are marked.

**Stage 0 — Prove the model on real data (no product code).**
Pick 3 to 5 repeated room types from House A / RHH (patient room, WC, store). Write them as
recipes by hand, resolve them onto their siblings with a throwaway script, and measure:
how many siblings share edge topology, how often an item lands outside or on another, how
much real layouts vary. Decide: the grammar above, how to handle non-rectangular rooms,
and the pyRevit-versus-add-in question for stage 7 (see critique). *Output: a short
findings note and a fixed grammar. This is the cheapest way to kill the biggest risk.*

**Stage 1 — Contract gaps.** Extend the FF&E and door exports with height above floor,
overall dimensions, host kind and host element, and a room-boundary-relative flag if the
export can supply one. Schema version bump, ingest, storage, `/ffe` response. Old
snapshots read with the fields absent (degrade, never guess). *Needed by 5, 8, 9.*

**Stage 2 — Blueprint store and API.** Blueprint document (id, name, revision, reference
outline, items, doors; a reserved, undesigned slot for sheet data, see track B), assignment records (room ref, blueprint
revision, transform). Storage beside connections with the same write rules. CRUD,
optimistic concurrency, MCP tool parity per the one-tool-per-read-route rule. *No UI yet.*

**Stage 3 — Capture from a room.** `service::blueprints::capture(room)` reads the room's
outline, FF&E and doors and proposes recipes: each item snaps to its nearest edge, records
along/off distance, picks "fixed" or "fraction" by a rule (flush against a wall and near a
corner is fixed; central on an edge is a fraction). Viewer: "Create blueprint" on a selected
room, a blueprint list panel, a read-only overlay of a blueprint's items. *Value on its
own: a catalogue of the project's repeated layouts.*

**Stage 4 — Resolver.** Blueprint + target room + transform (mirror, rotation in 90° steps,
edge mapping) to placements plus findings. Pure, heavily unit-tested with fixtures from
stage 0. Mirror respects `Never`. "Scaling" is room-size adaptation (see questions), not
item scaling. *The core; everything after depends on it being right.*

**Stage 5 — Assign UI.** Pick a blueprint for a selected room; ghost items drawn over the
plan; mirror/rotate/edge-map controls; findings listed and clickable; save the assignment.
Reuses the zone/overlay machinery (a new layer, like the reach heat map). *Value without
Revit: planning and coverage ("which rooms follow which blueprint").*

**Stage 6 — Revit write, thin slice (the walking skeleton).** Task model on the server
(a task is an assignment plus a snapshot of resolved placements and a state). New pyRevit
button "Apply blueprint tasks": fetch, dry-run diff, confirm, place **one family instance
type** with location, rotation and marker, report back. Prove the transaction, the marker,
idempotent re-run, and the results channel before any of the rest. Fake-Revit interface so
the executor is testable in CI. *Deliberately placed before the editor: the Revit write
loop is the largest unknown, and the editor is the largest UI. Do the unknown first.*

**Stage 7 — Revit write, full content.** All item kinds including wall-hosted and
face-hosted, doors (by host wall and offset), moves and removals of previously placed
items, missing-type mapping, per-room results with reasons.

**Stage 8 — Blueprint editor (the "new viewer").** A room-normalised canvas (the reference
outline drawn square-on, not at its place on the plan): add/remove items, drag with edge
snapping, an inspector for the grammar above, live resolve of the blueprint onto 2 or 3
sample rooms to see the effect of a constraint. Built on the Pixi renderer already in use.
*The biggest UI piece; after 5 it has a clear brief, because the assign UI has already
shown what is awkward about recipes.*

**Stage 9 — Elevation and 3D preview in the browser (optional).** Boxes from item dimensions
and heights; only if track B wants to show a sheet before generating it.

## Track B: room layout sheets (separate task, own plan)

Not part of stages 0 to 9 and not blocking them. Blueprints are useful without it
(catalogue, planning, Revit placement). It joins the main track at one point: a blueprint
may carry a sheet specification, and stage 2 reserves the slot without designing it.

**B0 — Survey what duHast already exports.** Read the existing IronPython sheet exports and
record exactly what they return: sheets, views and view types, viewport positions and
extents, view templates, tags and their positions, schedules. Output: a note of what is
real, what is a stub and what is missing, in the style of the door and ceiling probes
(run against a House A sheet set; "which duHast" means the one loaded in memory, see
`CLAUDE.md`). *Everything else in this track depends on it.*

**B1 — Contract and storage for sheet data.** If B0 shows the export is worth keeping, a
sheets entity on the existing entity stack (contract, ingest, storage, read route, MCP);
"adding an entity is a variant, not a copy". Read-only first.

**B2 — Capture a layout-sheet specification from existing sheets.** Which views (plan,
elevations, 3D, schedule), their types and templates by name, viewport positions relative
to the sheet, tag positions relative to their elements. Same recipe principle as
blueprints: relationships, not absolute coordinates.

**B3 — Attach a spec to a blueprint and preview it.** Resolver output feeds it (which walls,
which items to tag).

**B4 — Revit creation.** A task kind on the stage-6 channel: views from the project's own
templates and title block (named in the spec, never invented), viewports, tags, schedule.
Reuses the dry-run, marker and per-task result rules.

**Open for this track:** whether title block, view templates and sheet naming are common
across projects; whether the duHast exports are a base to extend or a prototype to replace;
and the platform question (critique 5), which sheet automation weighs more than placement.

## Critique

**What is most likely to go wrong, in order:**

1. **The constraint grammar will meet non-rectangular rooms and break.** "The edge" is
   well-defined on a rectangle and ambiguous on a T. A room that changes shape can gain or
   lose an edge, and then every anchor is stale. Mitigation: stage 0 measures it;
   v1 requires the target room to share the blueprint's edge count and the user maps edges
   explicitly (with a sensible default); anything else is a finding. If stage 0 shows most
   real rooms are non-rectangular, the model needs rethinking *before* stage 2, not after.

2. **The data is thinner than the feature assumes.** No heights, no sizes, mostly no
   footprints, no hosting. Elevations, wall-mounted rules and a size-aware overlap check
   all hang on stage 1, which touches the Revit exporter (and, per `CLAUDE.md`, duHast
   decides what an export means; fields must come from duHast or the API, not be re-derived
   in `room_m`). Treat stage 1 as a gate with its own probe on House A, as ceilings and
   floors had.

3. **The Revit write side is a different kind of software.** Everything so far reads. Writing
   means transactions, worksharing (elements checked out by others), families not loaded,
   hosted families needing a host, and a mistake changes a real model. Mitigations are in the
   design (dry-run default, markers, one transaction per room, per-task results) but the
   real answer is stage 6 coming early so the risk is found in weeks, not at the end.

4. **Mirroring is not geometry only.** Handed families (left/right fixtures, door swings,
   furniture with a "front") do not mirror safely. `mirror: Never` helps but needs setting
   per item, and capture cannot know. Default for capture should be *Never* for anything
   with a facing, and the user opts in.

5. **Platform choice for stage 6 onwards is not made.** pyRevit (IronPython, what the
   repo already ships and where the buttons live) versus a C# add-in. pyRevit is faster to
   build and matches the installer; a C# add-in is sturdier for transactions, sheet
   automation and distribution. Decide in stage 0 with a one-day spike: can pyRevit create
   a viewport and a tag reliably on the Revit versions in use?

6. **Scope.** This is realistically several months and three audiences (planner, designer,
   documenter). The staging gives value at 3 (catalogue), 5 (planning) and 6/7 (Revit
   automation) independently, so it can stop at any of them. The risk is starting 8
   before 6 has shaken out the model. Sheets are split into track B for the same reason:
   a second product with its own unknowns, which bundling would hide.

7. **Authored data is unreproducible.** Blueprints are exactly the kind of thing
   `STRATEGY-AUTHORED.md` warns about. Revisions are immutable once assigned, backups use
   the existing mechanism, and the connections store is already flagged as provisional:
   decide that storage question once, for both, rather than adding a second provisional
   store. Raise it at stage 2.

8. **Security.** The server tells a Revit user's machine to change a model. Tasks come
   from authored data, so the plugin must validate everything it is told (types, ids,
   ranges), never execute free-form content, and always show the diff. Cross-check with
   `STRATEGY-SECURITY.md` before stage 6 ships.

9. **Testing.** Revit cannot run in CI. The executor is split into a pure planner
   (task + current model facts to operations) and a thin Revit adapter; the planner is
   tested against JSON fixtures, the adapter by hand against a sample model. Without this
   split stages 6 to 9 are untestable.

10. **Naming and expectations.** "Follow to some degree" is a promise a solver must keep
    predictably. Prefer explicit rules a person can read ("2 m from corner A") over
    inference; show every resolved placement with the rule that produced it.

**Things deliberately left out of v1:** item-to-item relationships, automatic fitting of a
blueprint to a room (suggesting which blueprint suits which room), cross-project libraries,
nested components, and editing Revit items already in the model that were not placed by a
blueprint.

## Questions for review

1. **"Scaling" in the assign UI**: do you mean adapting the layout to a *larger or smaller
   room* (fixed offsets stay, fractions stretch, items keep their size)? Or scaling items too?
   The plan assumes the first.
2. **Rooms in scope for v1**: rectangular and orthogonal only, or L and T from the start?
   Stage 0 will inform this, but your sense of how common each is on real projects matters.
3. **Scope of a blueprint**: project-only, or reusable across projects (a "patient room"
   library)? Cross-project changes the identity rules in decision 5 and the store.
4. **Revit platform**: stay with pyRevit, or is a C# add-in acceptable?
5. **Who may write to Revit**: anyone using the button, or does a person approve tasks in the
   viewer first (a task state of *approved*)?
6. **Sheets (track B)**: are the title block, view templates and sheet naming standard across
   your projects, or per project? And how far were the duHast sheet exports taken (B0 will
   read them, but your view helps)?
