# Plan: blueprints (room layouts that travel)

**Status: stages 0 to 6 built (PR #215), and the server half of stages 7 and 8; the Revit side is written
(the Apply tasks button) but has never run in Revit. Revised 2026-10-20 after the author's review of the Revit task
design and of how sheets are stored; first written 2026-10-19 from the stage 0 results
([PLAN-blueprints-stage0.md](PLAN-blueprints-stage0.md)).**

Built so far (PR #215): FF&E carries `family_name` and `host` (stage 1, read from Revit by the
extractor, **not yet run in Revit**); the blueprint store with append-only revisions, pending pushed
revisions, import and export between projects, and its HTTP and MCP surface (stage 2); capture from a
room (stage 3, a service for the Revit push); the resolver (stage 4), which on RHH's real sibling rooms lands the items within
150 mm of the real ones for 90% of ensuite pairs, 68% of `1 BED B` and 58% of `1 BED A`; the blueprint
viewer at `/blueprints/` (stage 5: list, Import and Export ("New from a room" is to be removed, decision 18), revision timeline with
pending revisions, the room's outline, doors and FF&E, an inspector for dimension constraints, and a
manual edge map); and **stage 6 as a mode of that page, not in the main viewer** ("Assign to rooms"):
pick a room, see the resolver's answer drawn in it with the room's doors and where the blueprint's
land, choose between the rotations that fit (best by doors first) or map the edges by hand, and save
an assignment (one per room, pinning a revision; the request is stored, never the placements).
**Several rooms can be assigned at once**: the resolver is asked about each ticked room at the best
fit and a verdict (fits, to look at, cannot be placed) is shown before assigning. Doors are captured
with the blueprint for orientation only, and break ties of shape.

Track B's first slice is built too: the sheet library on its own endpoint with identity matching,
pending renumbered pushes and `suggest`, plus the Sheets and Blueprint pyRevit buttons (**not yet run
in Revit**). The 2026-10-20 decisions are built on the server and in the buttons (still **not run in Revit**): the
Blueprint button pushes a room and its sheets to `POST .../blueprints/from-revit` (decisions 18 and 19),
and the library stores a summary of each sheet's room (decision 25).

Built since (2026-10-21 and 22): the tag-rule store and the tag export (decision 24); the `update_room` task kind
(server and tasks page; the plug-in does not know it, so it aborts one); a page for the sheet library and the tag
rules (`/library/`); blueprints that need attention are flagged; **per-item overrides** (a room leaves out, moves or
turns single items, kept with its assignment, applied by the resolver before its findings); and the **Blueprint tool
in the main viewer** (a ghost, a best fit and a box laid by hand: see stage 6).

Not built: a mirrored copy (left out on purpose, decision 7); view template in the sheet export; the Revit side
of the update kind; and running any of the Revit side in Revit.

## What exists, and what does not

Grounded in the code as of 2026-10-18; rows marked *now* were true then and have since changed:

| Need | State |
|---|---|
| Room outline, level, properties | Have. |
| FF&E: type, category, `insertion_point` (plan, mm), `facing`, owner room | Have. |
| FF&E footprint (`loops`, the instance's oriented box in plan) | Have: 15,826 of 16,161 items (98%) in RHH `HOS-INT_L1-L2` (probed 2026-10-18). The "empty on every item" comments in `renderer/types.ts` predate it and are stale. |
| FF&E instance and type **properties** (every parameter, as exported) | Have. The probe found `Elevation from Level` on 100% of items, `Host` on 99.9%, `Offset from Host` on 99.4%, `Mounting Height` on 59%. |
| FF&E **vertical extent and sizes** | **Not needed as separate fields** (decided): they are part of the type and instance properties, which a blueprint item carries as exported. |
| FF&E **hosting kind** | Weak: `Host` is a display string (`Level : LEVEL 1`); which wall, and where on it, is not stated. *Now:* `host` is read from Revit by the extractor (built, not yet run in Revit). |
| Family name | **Not exported.** `type_name` is on every item; a `Family` key exists on 2 of 16,161 and holds something else. *Now:* read from Revit by the extractor (`item_facts`), built, not yet run in Revit. |
| Units of parameter values | Display strings (`"0.0"`, `"420.0"`), not typed. The trap `CLAUDE.md` records for the ceiling height offset. |
| Doors: position, `through_wall_normal`, width | Have. |
| Authored, per-project, user-made data | Precedent: connections document (zones, passages, saved routes, reaches): one JSON per project, optimistic concurrency, replace-whole-list, "authored data outlives geometry; a stale reference is a finding, never an error". |
| Revit extractor | pyRevit, **one-way** (Revit → server). |
| Anything that sends work **to** Revit | **Does not exist** (still true). A new channel, not an extension. |
| Room layout sheet data | duHast has rudimentary IronPython exports, surveyed in B0 (below). *Now:* the sheet library holds them as exported, on its own endpoint. |

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
7. **No mirroring, at assignment or by copy** (decided 2026-10-19, confirmed 2026-10-20).
   Handed items are most likely a different family, and a reflection would place the wrong
   one in a way that looks right. Rotation is supported; mirror is not offered. Stage 0
   shows handedness is common (37% and 43% of ensuite pairs on RHH are mirror images), so the
   resolver reports "this room is the mirror image of the blueprint" instead of resolving
   badly, and the other hand is **authored by a person as its own blueprint**. No mirrored
   copy is generated. A link between the two ("handed pair", so the finding can offer the
   other blueprint) is possible later.
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
13. **The Revit side is pull-based, and a task is checked as a whole before anything is written**
    (revised 2026-10-20 from the "Revit interactions" tab). The server holds tasks; a Revit
    button asks for the tasks of one model, validates the entire list (decision 22), reports
    anything aborted before work starts, shows warnings with *abort* or *ignore and proceed*,
    and reports a status per task afterwards. One transaction per room. The earlier "dry-run
    diff by default" is kept for the update task type; the populate task's safety is its
    validation, since the room must be empty in the phase (or overwrite is chosen).
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
18. **Sheets live in two places, and they are two storage methods** (clarified 2026-10-20; **replaced by decision 27, built in step 2: one store, the library, with a `blueprint` binding. The rest of this decision describes what was**).
    - **Inside a blueprint.** A blueprint push always carries the room and its sheets (the
      plugin enforces both, and the server refuses a blueprint push without sheets), so a
      revision holds outline, items, doors **and sheets**. The sheets are layout only:
      number and size, each viewport's type, position and extent. The blueprint is
      self-contained, so export and import carry its sheets, and an assignment that pins a
      revision pins its sheets. A sheet arriving again is a pending revision (decision 10).
    - **In the library** (its own endpoint, decided 2026-10-19), for sheets that belong to no
      blueprint: one record per sheet, kept as exported. It is what "suggest" searches.
    **A blueprint comes only from Revit** (decided 2026-10-20). A layout borrowed from another
    room's sheet would carry that room's viewports and sizes, so a blueprint is created only by
    the Revit push, which always carries the room data and its sheets; the server refuses a
    create or a push without sheets. The viewer has no "New from a room". Import and export stay
    (an exported blueprint carries its sheets), and an edit in the viewer makes a new revision
    that copies the previous revision's sheets unchanged; sheets change only by a Revit
    re-push, which arrives pending. A blueprint made before this rule has no sheets, stays
    valid and is shown as having none; a task from it cannot create sheets and says why.
    **Neither holds tags** (decision 24). A sheet in a blueprint is a copy and stays
    independent of the library.
19. **The user chooses which sheets are exported, and which room each belongs to.** Nothing is
    inferred from sheet names or parameters: the export has no marker for a room layout sheet
    and no link from a sheet to a room. In Revit the user picks rooms and, per room, its
    sheets. Through the **blueprint button** they go into a blueprint (with the room data, as a
    pending revision of the blueprint of that name, or a new blueprint; the same rule as
    import). Through the **Sheets button** they go to the library, where the room is named at
    export or afterwards in the viewer.
20. **A sheet reaches a room in one of two ways.** *Through a blueprint:* an assignment pins a
    revision and the revision holds the sheets (no separate association record; the earlier
    "associate sheets with a blueprint" step is dropped). *Suggested:* in sheet creation mode
    only, the server ranks library sheets by how close their room is to the target room (size,
    proportions, number of items), the three weighed **equally** (decided 2026-10-19). A
    suggestion is offered, never applied, and never made outside sheet creation.
21. **Revit tasks (design from the user's "Revit interactions" tab, 2026-10-20).** Once a
    blueprint is assigned to rooms the user can create tasks to be actioned in Revit. Kinds
    (decision 26 adds the second): **populate room from blueprint**, **create RLS from a
    library sheet**, and **update room from blueprint** (items update or location update after a
    room-size or design change; not now). **Populate is built first.** Its properties:
    - `phase`: the phase to create content in.
    - `overwrite_layout`: true deletes the room's existing content in that phase; false aborts the
      task if the room already has content in it.
    - `create_rls` and `overwrite_rls` (default false): kept on the record, but the Apply button no
      longer makes sheets; room layout sheets come from a `create_rls` task and the RLS create button.
    - **items**: family name, family type name, placement type, related data.
    - **RLS data**, from the pinned revision's sheets (decision 18): number and size of sheets,
      each viewport's type and location. **Tags are not in it** (decision 24): they come from the
      tag rules, per family and view type, applied to the items placed.
    A task is a record, **unlike an assignment**: it holds the resolver's placements and the
    sheet data as they were when the task was made, so what Revit is told cannot change
    underneath it. The resolver stays on the server (decision 2); the plugin only validates and
    writes.
22. **Task validation, all before work starts.** The whole list is checked; any abort is
    reported to the server first.
    - Phase name does not exist in the model: **abort**.
    - Room (found by its number) no longer exists: **abort**.
    - Room already has content in the phase and `overwrite_layout` is false: **abort**.
    - A specified family or type is in neither the model nor the library location: **warning**.
    - A populate task that asks for sheets: **warning** (this button places items only).
    - The sheet checks (number taken, view name taken, no title block for the size, a missing view
      type or schedule, incomplete settings) belong to the RLS create button, which **aborts** on each.
    A warning is shown to the person with two choices: abort, or ignore and proceed.
23. **Task transport and states.** A new endpoint, `roomtasks`, takes a **model name**
    and returns that model's queued tasks and marks them *in progress*, so another Revit
    session is not given the same ones. The plugin then posts a status per task: *completed*,
    *completed with warnings*, *failed* or *aborted* (an abort from the validation above is
    reported before work, with its reason). On the server the person can read the task list,
    **dismiss** a task, or **resend** one (back into the queue, which is also the way out of a
    session that died holding tasks *in progress*). Ideally a room's last task status is
    drawn on the plan viewer through a colouring option.
24. **Tags are not part of a sheet.** (Their key and scope are to be replaced by decision 27.) A task may put a library sheet into a room whose items
    differ from those the sheet was made for, so tags tied to items cannot travel with it. Tag
    placement is a **separate store, future work**, keyed by family name and type name plus
    viewport type, holding where the tag goes relative to the element. A task applies those
    rules to whichever items it places or finds, whether the sheets came from a blueprint or
    the library. Until duHast exports tags nothing is stored, and the plugin drops tags from a
    sheet before pushing. A sheet's name, number and id are not used on assignment: the Revit
    side generates them from the target room.

    **Tag data (decided 2026-10-21).** The export holds raw observations as duHast data classes
    (`DataTag`, `DataTaggedElement`, `DataTagType`, in `Data/Objects/Collectors`): positions in mm in the
    model's own coordinates, the tag's category, family and type name, and the tagged element's category,
    family and type name. **The ids in them are provenance, never the lookup**: a blueprint places new
    elements with new ids, so a rule is found by the item's type code (built under decision 27; it was names
    until then). **One rule per key** (type code, tag family + type, viewport type): several observations of one key are reduced to a single rule
    (median offset) when the rules are made, so the same blueprint always tags the same way. Where a tag
    is placed relative to its element is derived from the raw observation, never stored in the export.
    **Built (2026-10-21):** observations endpoint, the rules store (paper mm in the element's own frame, median of
    samples, a re-pushed sheet replaces its samples), rules carried in a task, and the extractor sending tags. The
    plug-in does not place tags yet (the apply button), and none of it has run in Revit.
25. **A library sheet carries a stored summary of its room** (kept by decision 27) (decided 2026-10-20, replacing
    "worked out at read time, never stored"): the room's bounding box X and Y dimensions and
    its number of items, as at push. They make a sheet comparable with a task's room without
    the sheet's own room being on the server, or unchanged. No room geometry is stored. The
    server works them out from the rooms and FF&E at push time (the one source of both);
    dimensions are of the smallest enclosing rectangle, so a rotated room reads the same, and
    the item count is counted as capture counts. Suggest ranks on these figures against the
    target room's current ones. A blueprint needs no such summary: it holds the room itself.
26. **Two task kinds read different inputs** (decided 2026-10-20), both validated as in
    decision 22 where the check applies:
    - **Create RLS from a sheet.** Input: a library sheet, the target room, the phase and
      `overwrite_rls`. No items are placed, so there is no layout check; the stored room
      summary against the target room's size is a warning when they differ widely, and tags
      come from the tag rules applied to the items found in the room.
    - **Populate room from blueprint, and create RLS** (decision 21). Input: an assignment
      (revision and mapping), the phase, `overwrite_layout`, `create_rls`, `overwrite_rls`; the
      sheets come from the pinned revision.

27. **Three kinds of blueprint, one scope rule** (decided 2026-10-31; **not built**, replaces decisions 18, 24 and 25 where
    they differ). Supersedes the embedded sheets and the name-keyed tag rules.
    - **RBP (room blueprint)**: outline, items, doors, as now. It holds no sheets and no tags. An RBP is never deleted, only
      marked **not in use** (it leaves suggestions and new assignments; what is bound to it stays bound).
    - **SBP (sheet blueprint)**: the sheet library record, kept by sheet identity, with `blueprint: Option<id>`. `None` is
      generic. A sheet belongs to at most one RBP. Size, viewports and their boxes are as exported.
    - **TBP (tag blueprint)**, two kinds, each with the same `blueprint: Option<id>`:
      - **FFE tag**: keyed by the item's **type code** (a project setting names the type parameter; every instance of the
        type shares it), the tag type and the viewport type. Location and orientation are in paper mm in the item's frame
        (the rule store as built). A **bound** tag also names its **instance** (the blueprint item id, which is the Revit
        element id and survives a re-capture), so a bound tag holds exactly one sample; a generic tag has no instance and
        holds the median of its samples. **Code only, no fall-back to family and type names**: an item with no code is
        untagged and reported (`skipped`, `items_without_tags`).
      - **Room tag**: keyed by the viewport type; its location is relative to the room **boundary** in plan.
    - **Invariant**: bound means `blueprint` set and, for an FFE tag, `instance` set; generic means both empty.
    - **A re-push of an RBP is pending, as ever (decision 10); its sheets and observations wait in the pending revision.**
      **Accepting reconciles**: the bound SBPs and TBPs become the pushed ones; one that is no longer in the push is
      **unlinked** (`blueprint` and `instance` cleared) and **merged into the generic rule with its key**; a sheet that
      disappears is unlinked the same way. A first push has nothing pending and reconciles at once. Rejecting drops the
      payload. A sample's identity is its source sheet plus the tagged element, so a sheet pushed again replaces its own
      samples wherever they now are and a merged orphan is never counted twice.
    - **Looking tags up for an RLS**: bound room tags (the chosen sheet is bound; they need no assignment); bound FFE
      tags by instance, for the items an assignment placed; generic FFE tags by type code for every other item. A room with
      no assignment of the sheet's RBP gets generic tags for its items.
    - **A pin covers an assignment's ITEMS only.** Sheets and tags follow the latest accepted push; a task copies them when
      made, so a task is unaffected.
    - **Import and export carry the bound sheets and tags**, and an import goes through the same reconcile.
    - **Room tags** are relative to the room boundary in plan: bound ones use the wall recipe (edge by the cyclic shift that
      fits, fraction along, distance from it: decision 24's elevation marker rule), generic ones the nearest side of the room's
      smallest enclosing rectangle (decided 2026-10-31, built in step 3).
    - **A generic sheet is identified by its room's bounding rectangle and item count** (`room_key`, built in step 3).
    - **Built in four steps** (1, 2 and 3 are built): (1) the type-code setting and rules keyed by it (old rules discarded); (2) `blueprint` on the
      sheet record, embedded sheets and `offer_to_library` removed; (3) scope and instance on tag rules and the lookup
      order above; (4) pending payload, reconcile, import and export.

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
revisions, current revision, reference outline, items with excluded flag, doors, and sheets
built, B3b), assignment records (room ref, pinned revision, mapping), pending pushed
revisions. Optimistic concurrency, MCP tool parity per
the one-tool-per-read-route rule. **Import and export between projects on the same server**
(cross-server later). *Built; the UI is stage 5.*

**Stage 3 — Capture from a room.** `service::blueprints::capture(room)` reads the room's
outline, FF&E and doors and proposes recipes: each item snaps to the edge it lies beside
(the nearest edge whose span contains it; nearest-segment distance failed on notched rooms),
records along/off distance, and picks fixed or percentage by the decision 4 and grammar
rules. Nested components are excluded, wall devices get their own class, and abutting items
become runs. Both outlines are stored (decision 14). The capture is a service and a read
(MCP `capture_blueprint`) that the Revit push uses; there is no "Create blueprint" action in the
viewer (decision 18). *Value on its own: a catalogue of the project's repeated layouts.*

**Stage 4 — Resolver.** Blueprint revision + target room + transform (rotation, edge
mapping by shape, bounding-box scale) to placements plus findings, including the
mirror-image case (decision 7). Pure and heavily unit-tested with
the stage 0 fixtures. The core: everything after depends on it being right.

**Stage 5 — Blueprint viewer (a new view in RoomMate).**
- **Left pane:** the list of blueprints, single selection, like routes and reaches.
  Import and export of blueprints (same-server projects for now). Blueprints are created only
  by the Revit push (decision 18), so the earlier "New from a room" is to be removed.
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
Built as its own page in SVG, not on the Pixi renderer: one room and a few hundred rectangles
need the hit-testing and labels SVG gives for free. *The biggest UI piece; the first with a real
brief.*

**Stage 6 — Assign UI. Built:** as a mode of the blueprint page (one room at a time with rotations and a hand
mapping, or several rooms at once from a solver pass, saving assignments), and as the **Blueprint tool in the main
viewer** (a fourth per-zone tool beside route, reach and connections). **The box is the room reference.** Rooms are
never exactly the blueprint's shape, so a person moves, turns and stretches the blueprint's own outline over the room
by hand (handles on the plan, or numbers), and the recipes resolve against the box, not the room (decision 6: items
keep their size, a fixed distance keeps its mm, a percentage stretches with the box). The room's real outline only
judges the findings. It is stored on the assignment as `mapping.frame` (relative to the room, so a room that moves is
followed; stretch between 0.2 and 5, never a mirror) and is one of three mappings with a shift and an edge map. The
best fit by shape (click a room, Rotate through the rotations that fit) is the starting point, "Adjust by hand" turns
it into a box. Not in the SVG export. **Per-item overrides are built** (leave out, nudge in whole mm, turn in whole
degrees; one per item, this room's only, applied to the resolver's poses before the `outside` and `overlap`
findings, so a follower keeps its place unless it is overridden itself).
*Value without Revit: planning and coverage ("which rooms follow which blueprint").*

**Stage 7 — Revit write, thin slice (the walking skeleton).** **Server half built:** the task model and its
states, making populate and create-RLS tasks from assignments or a library sheet (placements in the target
model's own frame), the `roomtasks` ask and report channel, dismiss and resend, MCP reads. The task list is a page, `/tasks/` (filters, a task's contents and attempts, dismiss, resend, making
tasks), and the main viewer colours rooms by their last task's state (a choice in the zone's colour picker; not in
the SVG export). The plug-in has the decision 22 checking as a pure, CPython-tested planner (`room_m/task_plan.py`) and the `roomtasks` transport. **The apply button is written** (`room_m/apply_*.py`: the sequence, the facts the planner asks, the writer, the
arithmetic) and not run in Revit. **Still to do:** run it, and give the plug-in the `update_room` kind.
Original scope: task model on the server
(decisions 21 to 23: a task is a resolved snapshot plus a status), the `roomtasks` endpoint,
and a task list for the person (dismiss, resend). A new pyRevit button "Apply blueprint
tasks": fetch for the open model, validate the whole list, report aborts, ask about warnings,
then place **one family instance type** with location, rotation and marker, and report a status
per task. Populate only, no RLS creation yet. Prove the transaction, the marker, idempotent
re-run and the results channel before the rest. Fake-Revit interface so the executor is
testable in CI. *The Revit write loop is the largest unknown and is deferred behind the
viewer. Platform is pyRevit (decision 17).*

**Stage 8 — Revit write, full content.** `overwrite_layout` (delete then place), creating the RLS (sheets and viewports, then tags from the tag rules; needs track B's tag and view-template export), the create-RLS-from-a-sheet task (decision 26), the update task type, and all item kinds including wall-hosted and
face-hosted (needs stage 1's hosting kind), doors (by host wall and offset), moves and
removals of previously placed items, missing-type mapping, per-room results with reasons.

**Stage 9 — Elevation and 3D preview in the browser (optional).** Boxes from item
properties; only if track B wants to show a sheet before generating it.

## Track B: room layout sheets (separate task, own plan)

Not part of stages 0 to 9 and not blocking them. Blueprints are useful without it
(catalogue, planning, Revit placement). It joins the main track at one point: a blueprint
carries its sheets (decision 18), which is stage 2's reserved slot, now designed (B3b).

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

**B1 — Export buttons (extractor).** In Revit the user picks rooms and, per room, its sheets
(decision 19). **Blueprint button:** pushes the room and its sheets into a blueprint; both are
required (B3b). **Sheets button:** pushes the sheets to the library, with the room named at export.
Each chosen sheet goes through duHast's `convert_revit_sheet`, and tags are dropped before pushing
(decision 24). It can be pushed in the same run as FF&E, since a run already exports several
entities at once. The export carries the sheet's identity (Revit element id and sheet number), its
properties, viewports with sheet positions, crop boxes and schedules. Not carried, because duHast
does not export it: tags and their positions, view scale, view template, view names.

**B2 — Sheet library (server).** A `sheets` collection on its own endpoint, one record per
sheet, scoped to a project. Each record holds the sheet as exported and the room the user
named for it (a reference, never validated against the model on save; it may be set later in
the viewer). **A sheet is identified by its Revit sheet id plus the document's title** (decided
2026-10-19). A sheet pushed again that matches on both is the same sheet and is updated; **if it
matches but its sheet number has changed, the push is stashed as pending and the user is asked
to confirm** (accept or reject, as a pushed blueprint is, decision 10), so a renumbering cannot
silently rewrite a sheet someone relies on. A sheet that matches on neither is a new sheet, even if its number matches an existing one.
Both readings confirmed 2026-10-19.
Reads: list, one sheet, and **suggest**. Each record also holds a summary of its room as at
push (B3c).

**B3 — Suggest, in sheet creation mode only.** `GET .../sheets/suggest?room=` ranks library
sheets by the closeness of their room to the target room: area, proportions and number of
items, **weighed equally**, each compared with the target room's current figures against the
sheet's stored summary (B3c): area as the stored X times Y, proportions as long over short side. It returns the ranking with the figures, so a person sees why a sheet was offered.
Nothing applies it.

**B3b — Sheets inside a blueprint (replaces "associate sheets", 2026-10-20). Built (server and button; not run in Revit).** A blueprint
revision holds `sheets` beside outline, items and doors; the blueprint push carries both
and is refused without sheets; export and import carry them. Same sheet shape as the library
without tags, and without the sheet's name, number or id being used on assignment (decisions
18 and 24). Needs `sheets` in the revision and a blueprint-push endpoint that captures the
room on the server and attaches the sheets.

**B3c — Library room summary. Built.** Store the room's bounding box X and Y and item count with
each library sheet at push (decision 25), and have suggest use them.

**B4 — Revit creation (not scheduled).** A task kind on the stage-7 channel: views from the
project's own templates and title block (named in the spec, never invented), viewports,
tags, schedule. Reuses the validation, marker and per-task status rules (decisions 22 and 23).

**B5 — Tag rules (future).** A store of tag placement by family name, type name and viewport
type (decision 24), applied by tasks.

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
   all. Mitigation: shape matching on the coarse outline (decisions 14, 15), the resolver's
   mirror-image finding (the other hand is its own blueprint), and findings for every case that does not fit.

2. **The data is thick, with known gaps.** Footprints and the common elevation
   parameters are there; sizes and heights are in the properties by decision. The family name
   and the hosting kind are now read from Revit by the extractor but not yet run in Revit, and
   the units of parameter values are display strings. Tags, view scale and view templates are
   not in the sheet export. Any exporter change must come from duHast or the API (`CLAUDE.md`: do
   not re-derive in `room_m`); give stage 1 its own probe on House A. (An earlier draft said
   the export lacked footprints and properties; that claim was wrong and is corrected.)

3. **The Revit write side is a different kind of software.** Everything so far reads. Writing
   means transactions, worksharing (elements checked out by others), families not loaded,
   hosted families needing a host, and a mistake changes a real model. Mitigations are in
   the design (whole-list validation before any write, markers, one transaction per room, per-task results). The
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
   never execute free-form content, and validate the whole list and report aborts before writing. Cross-check with
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
   viewer first (a task state of *approved*)? (parked; the tasks tab has the person who runs the
   button seeing every warning, and tasks come from what a person created on the server)
6. **Sheets (track B)**: are the title block, view templates and sheet naming standard across
   your projects, or per project? (parked; B0 has read the duHast exports, see above)
7. ~~**Percentage constraints on rotated boxes.**~~ **Closed 2026-10-19:** a percentage
   constraint follows the edge it is anchored to, not the box's axes (decision 6).
8. ~~**Sheet library, three details.**~~ **Closed 2026-10-19:** a re-pushed sheet matches on Revit
   sheet id plus document title, and a changed sheet number is stashed for confirmation; the
   room is chosen at export (through the Sheets button, for library sheets) and afterwards in the
   viewer; "closest" weighs size, proportions and item count equally.
9. **Room identity in a task.** *Built as proposed:* the task carries the id and the room's `Number`
   property (a room has no number field of its own). Still to confirm: the number is unique within a model, or say what to do when it is not. The tab finds the room *by number*, the assignment holds model
   plus room id. A task should carry both (id for the server, number for Revit's check), and
   the check "does the room still exist" should compare the number *and* warn if the id
   moved. Confirm the number is unique within a model, or say what to do when it is not.
10. **Frame of the placements.** *Built:* placements leave the server in the target model's own frame
    (the inverse of its placement, unit-tested; not yet checked against a linked model with an offset
    origin). The resolver answers in the project's plan frame; Revit needs
    the target model's own frame. The server should put it back through the model's placement
    (the inverse of `service::placement`) when it makes the task. To confirm against a linked
    model with an offset origin before stage 7.
11. **`overwrite_layout` deletes.** What counts as the room's content in the phase: all FF&E
    whose room (or space) is this one, or only items carrying a RoomMate marker? Deleting
    unmarked items is the destructive reading. Proposed: only marked items are deleted
    without a warning; unmarked ones are listed and need the person's go-ahead.
12. **Tag and viewport data is not exported yet** (track B "missing"), so the RLS half of a
    populate task has nothing to read until the sheet export carries tags, view scale and
    view templates. The populate task can ship without RLS.
13. ~~**How a blueprint and its sheets are stored.**~~ **Closed 2026-10-20:** two methods. A blueprint
    revision holds its sheets (always pushed with the room data); the library holds sheets that
    belong to no blueprint; no sheet holds tags (decisions 18, 24 to 26).
