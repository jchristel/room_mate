# RoomMate — working notes

Revit → Rust → browser room data pipeline. The **reasoning** lives in
[`docs/`](docs/README.md) — this file holds only what is expensive to get wrong
and impossible to infer from the code. Don't duplicate the docs here; link them.

## Doors are built — the rules that outlive the build

Doors ship end to end (contract, ingest, storage, `/doors`, QA, milestone
comparison, pyRevit exporter). What is expensive to rediscover:

- **Property lookup is tiered, and a tier wins only when it is `Present`.**
  `lookup_property`/`property_presence` take `&impl PropertyTiers`; a door
  yields instance-then-type. A *blank* instance parameter does not shadow a real
  type value — `Door Leaf Thickness` is blank on 22 of 26 sample doors while the
  type says `40.0`. A name in both tiers is **not** a finding: `Workset` and
  `Edited by` collide on all 26.
- **The store takes bytes plus a `SnapshotMeta`, never a payload type.** Serde
  lives in a thin layer on `AppState`. Don't add a typed `put_doors` beside it —
  that is the exact parallel-method-set failure R1 was written to prevent.
  `AppState` holds `Box<dyn SnapshotStore>`, so the trait must stay
  **object-safe**: a generic `put<T>` is out.
- **Ingest streams to disk; the JSON framing lives in `state.rs`, not the
  store.** `put_streaming` hands back an append-only byte sink that publishes
  atomically, and `StreamingSnapshot` is what knows a snapshot is an object with
  an element array in it. A store that parsed the payload in order to write it
  would be the first crack in the bytes-at-the-boundary rule. The streamed file
  is **one element per line, then the property dictionary as the last line**;
  the buffered path (`encode_snapshot`) writes the same layout on fewer lines.
- **A stored snapshot names its properties by index, and the dictionary is its
  LAST line** (`contract::property_codec`, 2026-09-17). Each element's
  `properties` are `[key, storage_type, value]` triples and its
  `type_properties` an index into a table of distinct bags, keyed by *content*
  (never `type_id` — nothing enforces one bag per type). Last because ingest
  streams; a reader looks only at the last line, so a legacy snapshot costs
  nothing to detect and still reads. **The push wire is unchanged**: the field
  codecs write plain maps unless a thread-local `Encoder` is active, so never
  serialize a stored snapshot any way but `encode_snapshot`/`StreamingSnapshot`
  — `to_vec_pretty` would write maps with no trailer, which still reads, but
  silently gives up the format. In memory, keys are `Arc<str>` and type bags
  `Arc<PropertyMap>`; that sharing, not the file, is most of the speed (RHH
  FF&E parse + clone: 29 s owned strings, 9 s dictionary expanded back to
  `String`, 4.3 s shared).
- **Element responses send each type bag once, too** (`service::type_table`,
  2026-09-17): `/doors`, `/windows`, `/ffe`, `/ceilings` and `/floors` carry a
  top-level `type_property_sets` and each element a `type_properties_ref` row,
  never an inline `type_properties`. **Built last** — when the wire result is
  made from `Assembled` for openings and surfaces, at the very end of
  `assemble_items` for FF&E — and the bag is *moved* out of the element, so anything
  that reads the type tier (the property filter, milestone comparison) must run
  on `Assembled`, before that step. A row indexes its own response only; the
  viewer resolves it with `typePropertiesOf(payload, element)`.
- **A writer dropped without committing must leave no trace**, and dropping is
  the *ordinary* path, not an error one: a rooms push is only known to be empty
  once its rooms have been counted, which is after writing has started. That is
  why `finish_rooms` checks every model before committing any — a stray temp
  file, a manifest entry, or half a run going live would turn a correct refusal
  into corrupt data.
- **A room id is unique only within a model, and a door's `from_room`/`to_room`
  are room ids.** So the door→room join is model-scoped *everywhere*: QA resolves
  references per model, and every `/doors` row carries `model_id`. A
  project-scoped shortcut anywhere here turns a dangling reference into a false
  clean bill.
- **Ingest does NOT require a model to have rooms before its doors** — and the
  gate that used to is deliberately gone, not missing. Doors may be pushed first.
  The question moved to `door_report`, which separates **pending** (no rooms yet
  — expected, not a finding) from **dangling** (the named room is not among the
  ones the model has — a finding). Re-adding an ingest-time gate would refuse
  data that becomes resolvable the moment the rooms land, and would put the one
  "signal, not error" exception back into the server.
- **Doors never re-phase a lineage.** A rooms push that disagrees on phase is
  quarantined and promotable; a doors push is **refused**. Promoting it would
  move the lineage while the rooms stayed behind. An *unphased* lineage is the
  exception and is phased by whichever push reaches it first — which, now that
  doors may arrive before rooms, is sometimes the doors.
- **Reference sources are entity-scoped (R4, 2026-08-05).** Each declares
  `entity`, defaulting to `rooms`. A source scoped to one entity never joins the
  other, even when the key would match. The join namespace stays **flat** —
  `schedule.FireRating`, never `doors.schedule.FireRating` — because the entity
  is already known from the endpoint, so source names are unique across
  entities. That uniqueness is free: the sources map is keyed by name.
- **Door ownership: a door belongs to the room it opens *into*, else the room it
  opens *from*, else it is homeless.** `[doors] room_attribution`, default
  `to_room_then_from_room`. Derived at read time, never stored, so changing the
  policy changes every answer and rewrites nothing. `owner_rooms` on `/doors` is
  a **list** (the `both` policy attributes twice) and **empty means homeless** —
  a reported state, which is also why a homeless door matches no `?building=`.
  Trust it exactly as far as the model is consistent: Revit's `to_room` follows
  the door's *orientation*, not the leaf swing, so flipping a door swaps it.
  That is why it is policy with an override, never a rule in code.
- **A door's room is what it *serves*, not what it opens into** — and the two
  differ on purpose. A cupboard off a long corridor swings into the corridor and
  belongs to the cupboard; 2 of the 26 House A doors are deliberately that shape
  (element ids `2618110`, `2626240`). So `to_room` is a **modeller's
  assignment**, not a geometric consequence, and a door whose
  `through_wall_normal` points away from its own `to_room` is correct data, not
  a finding. Do not add a check that "reconciles" the two — they answer
  different questions, which is also why both are on the wire.
- **The geometric resolver does not violate that, and the distinction is easy to
  lose.** `service::room_locator` answers "which room is this door physically
  beside", which is *not* "does this door open into that room". The cupboard
  door's insertion point is still on the cupboard/corridor boundary, so it
  passes; what fails is a room that has **moved away** from its door. Authored
  references always win — geometry fills an absent side and reports a
  disagreement, never overwrites one (`[doors] room_resolution`, default off).
- **`room_reference_property` reconciles the modeller against the geometry**,
  and finds real disagreements: 4 of the 26 House A doors, mostly where the
  geometry picks an exterior or circulation space over the served room. Absent
  means the check is **off**, not clean — the QA response says which.

## Level-scoped pushes and the single Export window

A streamed push (`/<entity>/stream`, all seven) may name `"scope": {"levels": ["<level id>", ...]}` on a model block: what it sends REPLACES
that model's elements on those levels and every other level is kept (`scoped_push`, a middleware in front of the stream routes). It appends
the latest snapshot's elements on the OTHER levels to the request body as ordinary lines (`AppState::carried_elements`: decoded, so the
property triples expand and re-encode), so the handler is unchanged and cannot store what an unscoped push would refuse, and the stored result
is a complete snapshot: reads, milestones and the dictionary never learn it was partial. **Level ids, not names** (per document, stable between
pushes of one model). A scoped model may send elements only on its scoped levels (else 400: it would duplicate a carried one). A model's first
push carries nothing. Two scoped pushes to one model race (the last stored wins whole); not guarded.

**The extractor's side**: the **Export** button (`RoomMate` panel, `export_all_entry`, `export.xaml` / `export_window.py`, every rule in
`export_form.py`) asks once for what to push (a check box per entity; FF&E by room or by space, not both), the project, the phase, and the
whole model or ticked levels. A level choice is by NAME across the selected models and becomes, per model, a `scope` of that model's own ids
(`export_form.scope_for_doc`); a model with none of the chosen levels is left out and said so, never pushed unscoped (that would replace its
whole snapshot with nothing). Each `post_*` stream drops an element on any other level (`phase_filter.in_scope`), and a scoped run does not
refuse an empty push (a level with no doors is a legitimate clear; `is_scoped`). The single-entity buttons are unchanged and share
`_run_export`. **The level choice also narrows what Revit reads**: each exporter hands `level_ids` to duHast's `get_all_*_data` (`duHast.Revit.Common.level_filter`),
which converts only elements on those levels, before the slow per-element conversion; the count guards in the spaces and surfaces exporters compare
against the same filtered set. An UNSCOPED push passes duHast no new argument (`export_form.level_kwargs`), so an older duHast still works for
every push that does not choose levels; a scoped one needs the duHast with the filter (**`duhast.lock` is not bumped**: build with `-DuHastPath`).
FF&E is filtered on the level the modeller assigned before conversion and on the geometric fallback level after it, so the level an item reports is
unchanged. Levels are the per-document ids, which each element's own `LevelId` reports (a room's level, a door's host level, a ceiling's host level).
**Not run in Revit.**

**The panel**: the `RoomMate` panel on the duHast tab holds Export, Blueprint, Sheets, Apply tasks, RLS settings, RLS create and Server. The older `Export` panel (Rooms + Doors, By category)
was removed: the Export window covers every entity.

## Ceilings: the entity where geometry is the only answer

Ceilings ship end to end: contract, ingest, storage, `/ceilings`, MCP,
exporter, pyRevit button and the plan layer. No QA report yet.

**Probed on RHH 2026-09-12** (1,833 ceilings across 9 documents, against House
A's 30). Everything below is measured on both unless it says otherwise. The
store's RHH ceilings for the four `HOS-INT_*` models were re-exported
2026-09-14 (1,806; 17 fewer, all on one level of `INT_L1-L2` bar one); `CPB-MAIN`'s
10 were not, so its latest is still the 2026-09-12 set.

- **A ceiling has no room parameter and a room has no ceiling parameter.** So
  the join is geometric or it does not exist. **Model-scoped is correct, and
  RHH confirms it**: every document holding ceilings also holds rooms, so
  ceilings are NOT a second exception to the model-scoped rule. A document
  holding rooms and no ceilings (a base-build package, a site model) is ordinary
  and says nothing about scope; what would break it is ceilings with no rooms in
  their own document, and there are none. The 26 cross-document overlaps RHH's
  geometry shows are what a project-scoped join would wrongly attribute — a
  ceiling over the one enormous site room another model declares. That inverts the precedence rule
  every other entity follows — doors, windows and FF&E each carry an authored
  reference and `room_resolution` is opt-in *because* there is something
  authored to disagree with. `service::surfaces` therefore passes
  `RoomResolution::SameModel` unconditionally and never reads the project's
  setting: reading it would let a project switch the entity off and report every
  ceiling as homeless, which is a wrong answer dressed as a disabled feature.
- **Attribution is many-to-many, area-weighted, and derived at read time.**
  `rooms` on a `/ceilings` row is a list ordered largest-overlap-first, so
  `rooms[0]` is a usable single owner. **Empty means unattributed, which is a
  reported state** — 8 of House A's 30, being external soffits, a level with no
  rooms, and two degenerate exports; 17 of RHH's 1,833. **Many-to-many is real,
  not defensive**: 419 of RHH's ceilings genuinely cover more than one room at a
  sliver-filtering threshold, where House A's single apparent case was an
  artefact. Nothing is stored, so changing the rule
  changes every answer and rewrites nothing.
- **One tolerance, and it is for imprecision, not for slivers** (decided
  2026-09-16). An overlap counts when its mean width (`2A/P`) is at least
  `MIN_OVERLAP_MEAN_WIDTH_FT`, **10 mm**, for ceilings and floors alike. It
  exists for the case duHast guards in shapely -- `intersects()` is true for
  polygons that only touch, and the boolean op returns rounding -- plus a
  finish edge a few mm past a room boundary. **A width, because an area
  cannot**: that strip grows with the wall's length and its width does not.
  Measured on RHH with a 0.001 sqm area tolerance instead, 243 admitted pairs
  were 0.001-0.01 sqm at a median 3-4 mm wide.
- **The sliver rules are gone, and re-adding one is re-opening a closed
  question.** Ceilings dropped overlaps under 0.5% of the ceiling and floors
  had a 1.5 ft width rule with two escapes, over a 1 sqft minimum. Every one
  drew a line through a population with no gap in it, and each needed another
  escape for the case the last one missed; on RHH the ceiling rule dropped 17
  rooms at least half covered by a ceiling they were under 0.5% of. Which real
  overlaps matter is the consumer's policy: `fraction_of_element`,
  `fraction_of_room` and `mean_width` ride every row. Removing them added 222
  RHH ceiling pairs and 557 floor pairs, **lost none, and changed `rooms[0]`
  for no element**. Two costs, accepted: a strip under a wall IS listed (behind
  the room it serves), and a degenerate export (House A's 0.33 and 0.18 sqft
  ceilings) attributes -- surfacing that is QA's job. duHast's own test divides
  by the ROOM, so a sliver into a SMALL room passes it and a small real
  overlap in a LARGE room fails it.
- **A ceiling is a LIST of polygons and the consumer unions them. Never take
  one piece, and never sum.** This rule was wrong once and the wrong version
  shipped, so the numbers are here rather than in a commit message. duHast
  exports one polygon per *horizontal face* and a slab has two, which on House A
  was the whole story: its 7 multi-polygon ceilings were the same face twice
  (IoU above 0.98) and the largest piece — which happened to be `polygon[0]` —
  was the entire ceiling. **RHH's 48 multi-polygon ceilings are genuinely
  DISJOINT.** Measured against the union: taking `polygon[0]` loses **44.0%** of
  ceiling area on average and **99.2%** at worst (a 1,457 sqft ceiling whose
  first piece is 11 sqft); taking the LARGEST piece still loses **25.7%**, with
  31 of the 48 over 5%. The union is the one operation correct on both — it
  collapses House A's duplicates (`A ∪ A = A`) and keeps RHH's pieces.
- **A ratio of summed area to largest area cannot tell a duplicated face from
  two equal disjoint pieces** — both give `sum ≈ 2 × largest`. Only a real
  overlap test separates them. That heuristic is exactly what made the House A
  reading look general, and it is why `analyse_ceilings_probe.py` now classifies
  on the union and says so when shapely is absent rather than guessing.
- **Holes are subtracted from a ceiling, unlike a room.** `room_locator::
  outline_of` drops a room's holes so a probe landing on a column still
  resolves; a room's hole is a column or a shaft. A ceiling's hole is a light
  well or a void over an atrium and is routinely large — RHH exports 173 holed
  polygons — so it comes out of both the overlap and the `fraction_of_element`
  denominator.
- **Phase is the DOORS range test**, `PHASE_CREATED` / `PHASE_DEMOLISHED`, never
  the rooms equality test on `ROOM_PHASE`. No ceiling on House A is demolished,
  so the two agree there by accident — which is why this is written down rather
  than inferred from a passing run.
- **A disagreeing phase is quarantined, not refused.** The openings refusal
  exists because promoting would strand `from_room`/`to_room`; a ceiling carries
  no room reference for a promotion to strand. An empty push is accepted by the
  server and refused by the producer, per run — the doors asymmetry verbatim.
- **The height offset is read from Revit; the level id is duHast's.** duHast
  carries the offset as the display STRING `"2700"` (rounded millimetres) where
  the contract wants decimal feet, so `utils/surfaces.host_offsets` reads the
  parameter itself. That is not the re-derivation this file forbids — there is
  no measurement to disagree about, only a rendering not to parse. The *level*
  is duHast's and must stay so; that one is the FF&E trap.
- **The plan layer is a DASHED INK ring, and the contrast with spaces is the
  point.** A space draws solid in the accent because it is a *services*
  boundary being held up against the architecture; a ceiling IS the
  architecture, so it draws in the room ink and is told apart from the room
  outline directly beneath it by the dash rather than by colour. Ceilings sit
  BELOW the spaces overlay in paint order: where both are on, the reader turned
  spaces on to compare them, so the services line is the one that must stay
  legible where the two coincide. Like spaces it defaults OFF and its poll is
  gated on the toggle, because it overlays the rooms it sits on and polling a
  hidden overlay would cost every viewer a read for a question most are not
  asking. Unlike spaces it has no model picker -- that exists because RHH keeps
  one services file per service, and `/ceilings` takes no `?model=` at all.
- **The SVG export draws the zone's layers, and fetches them itself**
  (2026-09-21). The polls hold only the storeys on screen, so `svgExport`
  re-reads each chosen level's layers and runs them through the same
  `storeyElements` join; an export built from the polls would silently drop
  every overlay on every level nobody is looking at. How a layer LOOKS --
  dashes, footprint alpha, theme fallback, the override test -- is
  `renderer/style.ts`, read by both painters: change a ceiling's dash anywhere
  else and the file and the screen disagree. **Saved routes are the one layer that
  is NOT fetched at export**: a route's path already covers every level it passes
  through and sits in the page store (`savedRoutes`), so the export reads it from
  there. What a route draws on a level is `routeDrawingsOnLevel` in `route.ts`,
  read by the screen overlay and by `routeExport.ts` alike -- change it there, not
  in either painter. Open zones are deliberately not exported.
- **`/ceilings`' ETag cursor covers ROOMS as well as ceilings**, which no other
  entity read needs. Attribution derives from the rooms in scope, so a rooms
  push alone changes every answer and a ceilings-only cursor would serve a stale
  304.

## Floors: the second slab, built before it was probed

Floors ship end to end on the ceilings stack (2026-09-13): contract, ingest,
storage, `/floors`, MCP, exporter, pyRevit button and the plan layer. No QA
report. **Probed on House A** (85 floors, building and site documents) **and
measured on RHH's first export** (2026-09-14: 1,749 floors in the four
`HOS-INT_*` models, against their 2,858 rooms; the server attributed it under
the sliver rules since removed, and again after).

- **One stack, not two copies.** duHast's `to_data_floor` is `to_data_ceiling`
  with the category and offset parameter swapped, so everything the ceilings
  bullets above say about the footprint (a list, unioned, holes subtracted,
  empty is a reported state), the phase test, quarantine and the offset read
  holds for floors verbatim. The code is shared to match: `contract::Surface`
  is both records, `service::surfaces` both reads, one ingest path in
  `handlers.rs`, `post_surfaces` / `exporters.surfaces` on the producer.
  What differs is a lookup on `SurfaceKind` or a thin per-entity module --
  list key, schema version, milestone pins. **Adding a third slab
  is a variant, not a copy**; writing `service::floors` beside it is the R1
  failure. `fraction_of_ceiling` was renamed `fraction_of_element` on the wire
  when floors made it a lie; it was never stored, so nothing migrated.
- **Why floors never had a working threshold of their own** -- the history
  behind the single tolerance above. A slab can be a whole plate, where a
  fraction of the element drops every small room: RHH has 25 rooms 80-100%
  covered by a floor they are 0.08-0.49% of (fire hose reel cupboards, 4-7 sqft
  bays under corridor finishes). The 1.5 ft width rule that replaced it needed
  an escape for a narrow floor wholly in one room (House A's hob, stair
  surround, step), and still dropped 43% of a 9 sqft water-fountain niche
  (`FLVY-007.E` into `WATER IMG003`); RHH's overlaps ran continuously through
  the line. `analyse_floors_probe.py` still re-runs those retired rules, for the
  comparison it was written to make.
- **The storey is the host `Level`, never elevation plus offset** (decided
  2026-09-13). The export carries it -- `level.id` equals the host Level on all
  85 -- and the join uses it. That has a visible cost, accepted: House A hosts
  its roof build-ups (`RafterZone_200mm`, `INS-104`, `INS-106`) on LEVEL 01 at
  +11.3 to +12.5 ft, so BED 01 01.06 "lies on" 7 floors and 3 are its roof; 18 of
  93 attributions come from them. Re-deriving the storey would fix those and
  break ordinary ones -- 20 floors, joist zones and build-ups among them, sit
  up to 2 ft below their level -- and it is the FF&E trap in another field. `height_offset`
  rides every row; filter on it, do not reassign.
- **`OST_Floors` is a room's whole build-up, not its floor.** Joist zone,
  timber build-up, finish, insulation, lawn, paving, driveway, kerb, hob and
  roof layers are all floors on House A, and `Structural` does not separate
  them (the rafter zone and `INS-106` are structural). Which one is "the
  floor" is the consumer's question; nothing here answers it.
- **Model-scoped holds on House A and on RHH's fit-out models.** House A's
  site document holds 16 landscaping floors and no rooms, which report no room
  correctly. RHH's export put every floor in a `HOS-INT_*` model beside its
  rooms, and 1,744 of 1,749 attribute; the 5 roomless are all `FLGE-004`. None
  is `Structural` -- these are finishes. **The base-build question is still
  open**: the export did not include `HOS-BASE`, so whether RHH's slabs live
  there, beside no rooms, is unmeasured.
- **The footprint is wrong on some floors, upstream, measured.** Check which
  duHast the extension runs before touching `room_m`:
  - **Holes exported as islands** on 2 floors, which the union fills back in
    (a travertine pool surround at 558.6 against Revit's 289.1 sqft). Cause
    confirmed by re-running duHast's own classifier on the exported rings:
    `geometry.adjust_delta` returns `+2` without the x-intercept test The
    Building Coder applies to `+2` and `-2`; corrected, it gives Revit's area to
    0.1 sqft. **Fixed upstream 2026-09-13 and confirmed in Revit**: re-probed
    with the self-check passing, those two floors export one piece with holes at
    exactly Revit's area and no other floor's geometry changed. The surround had
    been attributed to POOL EX.03 at 99.8% of the pool; the pool is in its hole.
    Snapshots exported before the fix keep the filled holes until re-exported.
    The same code classifies CEILING loops: no House A ceiling changed, and on
    RHH's re-export (2026-09-14) **14 ceilings reclassified an island as a
    hole**, the rest identical to float noise. Owners barely moved (3 room
    lists changed), but coverage did: one ceiling's overlap with its only room
    fell from 554 to 94 sqft, because the old export had filled its void.
  - **Sloped and shape-edited floors lose area**: 20 floors export under 90% of
    Revit's area, 7 of them nothing -- mostly site lawn, road and driveway.
    `get_unique_horizontal_faces` skips non-planar faces and admits
    near-vertical ones (51 sliver rings), and `pair_faces_by_area` pairs any two
    faces of equal area wherever they are.
  - Edge orientation (arcs tessellated against the loop) is NOT a measured
    problem: one crossing ring, on one floor.
- **The plan layer is a DOTTED ink ring below the ceilings.** Room outline,
  ceiling and floor can share a line, so they are told apart by stroke pattern
  (solid, dash, dot) and dot duty cycle rather than colour. Off by default, poll
  gated, no model picker, `/floors`' ETag covers rooms -- the ceilings reasons.

## Blueprints: a room's layout as recipes against its edges

Built: the store, capture, the resolver (blueprint + another room -> placements and findings)
the blueprint viewer (`/blueprints/`), the assign view (the same page, "Assign to rooms"), the
assignment store and the HTTP/MCP reads and writes, a Blueprint tool in the main viewer (below) and per-item overrides
(below). **No mirrored copy, on purpose: handed families make a reflection wrong in ways that look right, so a person
authors the other hand as its own blueprint.**
The reasoning and the measurements behind every rule below are in
[`PLAN-blueprints.md`](docs/PLAN-blueprints.md) and its stage 0 note; what is expensive to
get wrong:

- **A recipe, never coordinates**, and both halves ride the record: the distance *as
  captured* (`along_mm`, `off_mm`) and the *mode* that says how a room of another size reads
  it (`fixed_from_start`, `fixed_from_end`, `percent`). Switching a dimension between fixed
  and percentage is a change of mode, so nothing is recomputed that could drift.
- **Two outlines, two jobs.** Placements index the **coarse** outline's edges (jogs under
  350 mm removed), because same-named RHH patient rooms differ in column nibs and an edge
  count that includes them matched half of them; "is it inside the room" is checked against
  the **fine** one (30 mm), because the items that sit in those nibs are outside the coarse.
  Mixing them up reports findings that are not there. **The coarse outline is the fine one with the jogs taken out of
  it** (`blueprint_geometry::coarse_outline`), never `simplify(raw, 350)`: that removes the first vertex within 350 mm
  of its neighbours' line, so a boundary point 60 mm from a corner cost the room the corner and tilted three edges
  of House A's `WC 00.03` by 1.2 degrees. Measured on RHH: it changes 37% of the rooms' coarse outlines, none of the
  fine, and the share of rooms with an edge off every fine direction falls from 47% to 21%. The price is sibling
  edge-count agreement: `1 BED B` pairs that differ rise from 16% to 20% (the old result agreed partly by cutting
  corners). A stored blueprint keeps the outline it was captured with until it is pushed again.
- **A blueprint's reference outline (the BOX) starts as a rectangle, and a person reshapes it.** Capture from Revit
  (`Reference::Box`) turns the room so its walls run along the axes (`dominant_angle_deg`: the length-weighted mean of
  the edge angles modulo a quarter turn, so a room on an angled part of the building is squared up) and writes the
  recipes against the rectangle round it; the real outline rides beside it as `fine`. Turning changes no recipe, because
  an item is a distance along an edge and its facing is measured from the edge. **Editing the box never moves an item**:
  `service::blueprint_reshape` reads each item's centre and facing out of the OLD outline and writes them again against
  the NEW one (`POST .../blueprints/reshape`, a read of a draft, stores nothing), and an item on a wall that did not
  change keeps its recipe whole, modes included. A box has 4 to 64 nodes, no side under 10 mm, and no crossing
  (a clockwise one is turned round). The page's gestures live in `blueprints/boxEdit.ts` (drag a node and it snaps its
  two lines horizontal or vertical, Alt off; double-click the line adds a node, double-click a node removes it; align a
  side with a room wall; Square up). **The price, accepted**: the best-fit path compares the blueprint's edges with a
  target room's, so a rectangle blueprint fits only 4-edge rooms and anything else is `edge_count_differs`; the box
  tool in the main viewer does not care, and "Room outline" in the viewer gives back the old shape in one click.
  Existing blueprints keep the outline they were made with. The box draws in `[appearance] blueprint_box` (a blue).
- **An item belongs to the edge it lies beside** (nearest edge whose span contains its
  centre), not the nearest segment: nearest-segment clamped six items in House A's notched
  rooms to an edge's end, where the position cannot be reproduced.
- **No mirroring, anywhere.** Outlines are counter-clockwise and a room fits a blueprint by
  a cyclic shift of edges only. A mirror image is a different blueprint (handed families).
- **Revisions are append-only** and an assignment will pin one: an edit is a new revision,
  and `current_revision` moving is the only way to change which one new assignments use.
  Writes name the `taken_at` they read (`base`); a mismatch is a 409.
- **The blueprint viewer is its own page, `/blueprints/` (`src-js/blueprints`), drawn in SVG.** One
  room and a few hundred rectangles do not need the GL renderer, and an editor needs the
  hit-testing and labels SVG gives for free. It edits a blueprint in its OWN room; placing one in
  ANOTHER room is the server's resolver and belongs to the assign view, because two copies of
  "where does the bed go" is how a preview and a result disagree (`geometry.ts` ports only `edges`,
  `point` and `depth_at`).
  - **Edit a DRAFT, save a NEW revision.** The items of the revision being viewed are copied into a
    draft; Save appends them as the next revision. Nothing edits a revision that exists.
  - **A dimension is a captured distance plus a mode.** Switching fixed/percentage moves nothing in
    its own room; it decides what the item does when the room is another size. The number typed is
    in the mode's unit (mm from the start, mm from the END, or a percentage).
  - **A pending revision is read-only** and sits on the timeline until accepted or rejected.
  - **An import never overwrites.** From another project on the same server or from a file: a name
    the project already has goes in as a pending revision of that blueprint; a new name is created.
    It uses the existing endpoints, so it added no server surface.
  - **Room rows name no model**, so `model_id` is optional on capture, resolve and suggest: it is
    looked up among the project's rooms and refused, naming the models, if more than one has the id.
- **Doors orient a blueprint; they never create one.** The doors are already in the model.
  Capture reads the doors either side of which names the room (`room_doors`, NOT the attributed
  ones: a door to the corridor is still a door in this room's wall) and stores each as an edge, a
  distance along it and a width (`BlueprintDoor`, `door_on_edges`: nearest edge by segment
  distance, because a door is IN the wall). The resolver compares them with the target room's
  doors (`door_gap`: mean mm to the nearest door on the edge the blueprint's lands on, 3,000 where
  that edge has none) to break a **tie of shape**: rotations whose cost is within `SHAPE_TIE` of
  the best, which is every rectangle's two half-turn fits. A rotation that fits clearly worse is
  never promoted because a door lines up. No door on either side is no evidence (`door_gap_mm`
  absent), not agreement. A pinned shift or a hand mapping is never overridden, only reported
  (`doors_disagree`). `candidates` and `mapped_doors` ride the resolve answer so the view needs no
  rule of its own.
- **An assignment is a request, never a result** (`assignments`, one JSON per assignment under
  `assignments/<project>/`, NOT under `blueprints/`, whose listing reads every `*.json`). It names the
  blueprint, the revision it pins, the room (with its model, qualified server-side) and the mapping
  (`shift` or `edge_map` or neither = best fit, worked out again on every read). Placements are
  never stored, so a room that moves is followed. One per room: assigning again replaces it and
  keeps its id. `newer_revision` and `blueprint_missing` are reported states on the list.
- **The assign view is a mode of the blueprints page, not a drawing over the main plan.** It draws
  the resolver's answer (`AssignCanvas`) and chooses what to ask; it has no placement rule of its
  own. Solid bar = a door the room has, dashed = where the blueprint's lands.
- **Assigning several rooms is the same resolver, asked once per room** (`service::blueprint_solve`,
  `POST .../blueprints/{id}/solve`, `POST .../assignments/many`). The rooms and doors are read once for the
  batch; each row is a verdict (`clear`, `check`, `cannot`) that is advice and never a gate. The batch assigns
  the best fit only: a mapping by hand is a decision about one room and stays in the single-room view.
- **A blueprint comes only from Revit** (`POST .../blueprints/from-revit`). The server captures the room from the
  rooms and FF&E already pushed; a new name creates, an existing name becomes a pending revision. **A blueprint holds
  no sheets and no tags** (plan decision 27): they live in the sheet library and the tag rules **bound to the blueprint** by
  id (`SheetRecord::blueprint`, `TagRule.key.blueprint`), so there is one copy of each. A bound sheet is layout only (no
  tags) and its number, name and id are never used on assignment. `BlueprintSummary.sheets` is the count of bound sheets,
  filled from the library (`list_with_sheet_counts`). A blueprint with none is flagged and a populate task with
  `create_rls` refuses it. The viewer has no "new from a room".
- **Sheets and tags travel with a push as a `Bundle` and are applied on ACCEPTANCE** (`blueprint_bundle`). The Revit push
  carries the sheets, the tags it read and the type-code parameter; a new blueprint applies them at once, a re-push stores them
  in the pending revision (`PendingRevision.bundle`, opaque JSON) and changes nothing until it is accepted, and a rejected
  revision drops them. **Applying reconciles**: the sheets pushed are bound; a sheet bound to the blueprint and not pushed
  is **unbound** (`sheets::unbind_missing`: generic now, with its `room_key`); the tags pushed are bound and a rule bound to the
  blueprint and not written by the push is **unlinked** (`tag_rules::unlink_blueprint`): its samples merge into the generic rule
  with the same key (boundary recipes dropped, they were against the blueprint's room) and the bound rule goes. Tags are only
  reconciled when the push says it read them (`observations` present), so a push that read none never turns a blueprint's
  tags generic. Accepting applies after the revision is accepted; if that fails the answer says the revision was accepted and
  the sheets and tags were not. **Export and Import carry the same bundle** (`GET .../blueprints/{id}/bundle`: the bound
  sheets and rules with their samples; an import re-binds the rules to the new blueprint, and into an existing name it
  waits as a pending revision like any other). A blueprint made before this keeps its old embedded `sheets` in the file,
  which are ignored: push it again to bind its sheets.
- **A blueprint is never deleted, only marked not in use** (`Blueprint::in_use`, `PUT` with `in_use`; the page's "Mark not in use"). Not in
  use: `assignments::assign` refuses it, `suggest` leaves out the sheets bound to it, and the page stops nagging about it. What is bound to it
  and every assignment already made stay as they are, and it can be marked in use again. There is no delete route; `blueprint_missing` on an
  assignment remains a reported state for a file removed by hand.
- **Per-item overrides belong to ONE assignment and are applied by the resolver, never by the page.** An
  `ItemOverride` (leave out, or nudge in whole mm and turn in whole degrees, in the plan frame) is applied to the poses
  the recipes gave, *before* the `outside` and `overlap` findings, so those are judged where the item now is. One per
  item, no-ops dropped, an unknown item is the `override_unused` finding and not an error. **It affects its own item
  only**: a follower keeps the place the resolver gave it. The assign view previews through the resolve read's
  `overrides` parameter (so the drawing is the server's answer), and assigning again WITHOUT `overrides` keeps the
  room's (a client written before them must not clear them); `[]` clears. Changing the mapping after nudging can leave a
  nudge that no longer fits, and that is the person's to look at.
- **The Blueprint tool in the main viewer lays a blueprint into a room two ways, and the BOX is the point.** Rooms are
  never perfectly the blueprint's shape, so a person moves, turns and stretches the blueprint's own outline over the room
  (`assignments::Frame`: `world = centre + R(turn) * S(sx, sy) * (p - pivot)`, stretch along the BLUEPRINT's axes so it
  follows the box round, `centre` = the room's vertex mean + `(dx, dy)` so a room that moves is followed, both scales
  positive and between 0.2 and 5 because a mirror is a different blueprint). **The box is the room reference the
  recipes resolve against (plan decision 6): items keep their size, a fixed distance keeps its mm, a percentage
  stretches with the box**, and the room's real outline only judges the findings (`outside`); the box has no doors, so
  none are compared. It is `EdgeMap::Frame` in the resolver (`FitKind::Framed`, identity edge mapping, no
  simplification so the box keeps the blueprint's edge count), `mapping.frame` on an assignment (one of shift, edge map
  or box), and the `frame` parameter of the resolve read. The client's `boxPoint` is the same arithmetic and is used
  ONLY to draw the box while it is dragged (the placements are always the server's answer, a moment behind); a test on
  each side pins the same numbers. The other way is the best fit (click a room, Rotate through the fitting rotations).
  A click on a room moves the tool there (the choke point is `store.select`, exclusive with route, reach and
  connections); the handles are dots, six, with the turn handle on a corner because one that stands off a side by a
  fixed WORLD distance lands on that side's handle when the plan is small. **The viewer's world is feet and the
  resolver's numbers are millimetres; `BlueprintOverlay` converts at its edge** (it once drew them unconverted and the
  ghost was 300 times too large). Not in the SVG export.
- **Resolving is partly manual, by design.** `edge_map` on resolve is the person's own mapping of
  blueprint edges to room edges (the way to place in a room with a different edge count); the fit
  kind is then `manual` and nothing about the mapping is judged.
- **A push is PENDING, never current** (`blueprints::push`, plan decision 10). A revision that
  arrives from elsewhere is stored beside the blueprint, changes nothing about the history,
  what is current or what an assignment pins, and waits for a person to accept it (it becomes
  the next revision and the current one) or reject it (kept, marked rejected, never deleted).
  `.backups/` is the safety copy made when a file is rewritten, a different thing from review.
  Only `add_revision` (a person's own edit in the editor) can make a revision current directly.
- **The resolver (`service::blueprint_resolve`) always returns what it could place** and says the
  rest as findings. Measured on RHH's real siblings (12 rooms each, every ordered pair): `ENS-IN`
  and `ENS-OUT` 90% of pairs within 150 mm (median 71 and 58 mm), `1 BED B` 68% (116 mm),
  `1 BED A` 58% (147 mm). Rules that are easy to break:
  - **Scaling is the room's.** A fixed distance keeps its mm, a percentage keeps its fraction of
    the edge it is anchored to (or of the room's depth in front of it), items keep their size.
  - **A follower is placed rigidly relative to the item it follows**, in that item's own frame,
    so a run is never squashed and a shelf stays in its wardrobe. A follower of an excluded item,
    or in a loop, is placed by its own recipe and reported (`validate_content` refuses a loop).
  - **A finding is about what the room CHANGES.** `outside` and `overlap` are reported only
    where the blueprint did not already have them (three of 66 `1 BED B` items are outside their
    own source room, and would otherwise be reported on every resolve). Wall devices are left out.
  - **Mirror image is said only when no rotation fits.** A rectangle's reflection is a rotation,
    so a badly pinned shift on a symmetric room is `no_fit`, not `mirror_image`. The resolver
    never mirrors; the answer is a mirrored copy of the blueprint.
  - **A rectangle fits in two mappings a half turn apart**, and shape cannot choose: pass `shift`
    (the blueprint's doors are what will, in the assign UI).
  - **A different coarse edge count places nothing** (`edge_count_differs`): 32% of `1 BED B` and
    62% of `1 BED A` pairs on RHH. Mapping across differing edge counts is not built.
  - **Results are in the project's plan frame**, so they draw straight onto the plan.
- **`family_name` and `host` on an FF&E item are read from Revit by `item_facts`**, not from
  duHast: the export carries neither (probed on 16,161 RHH items). Same footing as the
  category; absent means unread, never guessed. **Not yet run in Revit.**
- **Capture reads the whole project's FF&E** (`assemble_items`, unscoped by storey) and keeps
  the room's items, so on RHH it costs a few seconds. Narrowing it with a storey scope is the
  obvious fix and has not been needed yet.

## Room tasks: what a Revit session is told, kept as a record

Built (server side): the task store (`tasks`), making tasks (`service::task_build`), the HTTP and MCP
surface (`tasks_api`; MCP reads `list_tasks`, `get_task`) and the task list page at `/tasks/` (`src-js/tasks`:
filters, a task's items, sheets, findings and attempts, dismiss, resend, and a panel that makes populate and update
tasks; it reads the list again every few seconds to show what a session did). the main viewer's
"Room tasks" colouring (`viewer/taskStatus.ts`, a choice in the zone's colour picker: a room draws in the state
of its newest task that is not dismissed; a task's number must agree with the room's, because a viewer room names no
model; read only while a zone uses it, every 8 s; not in the SVG export). The Revit side has its checking
(`room_m/task_plan.py`: every task checked before any is written, aborts reported first, one question for the
warnings; pure, tested on CPython in `extractor/tests`) and its transport (`post_tasks.py`). The **Apply tasks** button
is written (`apply_tasks_entry`): `apply_run` is the sequence (claim, check, ask once, report aborts before any work,
write and report each task, report any stranded one failed; tested with fakes), `apply_facts` answers the planner from
Revit, `apply_write` writes one task per `TransactionGroup` so a failed task leaves no trace, `apply_geometry` is the
pure arithmetic. **Not run in Revit.** The `update_room` kind is built on the server and the tasks page (items or
locations only, a dry run unless turned off, made from an assignment, carrying the layout as it resolves now with no
sheets or tags); **the plug-in does not know it yet, so the planner aborts one as an unknown kind.** The server
leaves an empty list out of a task, so the page reads a task back with its lists filled in. Plan decisions 21 to 26.

- **A task is a record, unlike an assignment.** It holds the resolver's placements, the sheet layouts
  and the findings as they were when it was made, so what Revit writes cannot change underneath it. The
  plugin validates and writes; it never resolves.
- **Placements leave in the target MODEL's own frame, mm.** The resolver answers in the project's plan
  frame, so `task_build` puts each position back through the inverse of that model's placement
  (`ModelToShared::inverse`) and each direction through its linear part. A model with no placement (the
  anchor) is left as it is. Do not hand Revit project-frame coordinates.
- **States and who moves them.** `queued` on creation or resend; `POST .../roomtasks {model_id}` hands a
  session that model's queued tasks in full and marks them `in_progress` (a POST, because asking changes
  them); `POST .../roomtasks/report` records `completed`, `completed_with_warnings`, `failed` or
  `aborted`, and only for a task that is `in_progress`. A person can `dismiss` (any state) or `resend`
  (any but queued, which is also the way out of a session that died holding tasks). Attempts are kept.
- **One live task per kind per room.** A second queued or in-progress task of the same kind for a room is
  refused at creation and at resend: both would be handed out together.
- **Each room of a request stands alone**: a blueprint whose file has gone, a room that has gone, a layout
  that would place nothing, or `create_rls` on a blueprint with no bound sheet is a row saying why.
- **The room is found by its `Number` property** (a room has no number field of its own) and the task
  carries the id as well; absent means the room has none.
- **`create_rls` compares the sheet's stored room summary with the target's** and warns (never refuses)
  when they differ by more than `RLS_ROOM_WARN_DISTANCE`.

## Applying a task: what the button does and leaves out

- **A family not in the model is loaded from `ROOMMATE_FAMILY_LIBRARY`** (a folder of `<family>.rfa`), else it is a
  warning. Items are placed on the room's level with the facing's rotation; wall-hosted ones go on the nearest wall
  within 450 mm; face and work plane based ones are reported, not placed. Instance and type parameters are not applied.
- **Sheets, viewports and tags are not made by this button.** A `create_rls` task goes through the RLS create button
  (below), which numbers, names and sizes everything from the model's RLS settings and the task's `rls` plan. A populate task
  that still asks for sheets (`create_rls` on it) is warned about (`task_plan.SHEETS_ELSEWHERE`) and places its items only. The
  earlier sheet-making code in this module (number template, title block by size token, learned tag rules placed from the
  model) was removed with the settings window, because two sheet makers disagreed on every rule.

## Create-RLS: what the task says beyond "copy this sheet"

A `create_rls` task carries `rls` (`service::rls_plan`, built in `task_build::rls_from_sheet`): the `sheet_size` asked for
(`a0`..`a3`, absent = the template's) and, per elevation marker, where it and each of its views go **in this room**, in the
target model's frame, mm. The RLS create button reads it (`room_m/rls_task.py`, below).

- **Transfer is by the walls.** Template and target are read at the COARSE outline, the wall each elevation faces
  (`elevation_facing`) is mapped by the cyclic shift that fits the shapes (`best_shift`, no reflection), and then the marker
  keeps its fraction along the wall and its distance from it, each view's line ends and direction keep their offsets from the
  marker in the frame of the wall that view faces, and the **depth keeps its length** (a view shows a stretch of wall, not a
  share of the room). `depth_to_wall_mm` is how far the faced wall is, for a person who wants "to the room boundary".
- **A marker is placed once, from its lowest-numbered view**, so its views share one point; placing it from each view's own
  wall would give four different markers in a room that scaled unevenly.
- **Different coarse edge counts place nothing** (the blueprint rule) and say so as a finding; the sheet is still made.
- `POST /projects/{id}/tasks/rls-preview` is the same read for one room and stores nothing, so a preview is the task. A request
  may override a view's depth (`depths`: `depth_mm`, or `to_wall`).
- **Tags are a PROTOTYPE** (`service::rls_tags`, `rls.tags` in the task and in the preview). Every item attributed to the room
  (no nested components) is looked up in the library's tag rules for the plan viewport type and tagged in the plan at the rule's
  offset (paper mm x the view scale, turned with the item). What no rule tags is **offered, per view** (`candidates`) and overall
  (`items_without_tags`, with the views that show it and why no rule did); a person answers per item in `tags.choices`: `tag` it in a
  named view (a default head beside it, a rectangle round its footprint, the text of `tags.tag_property`: `$id`, `$type_name`,
  `$family_name` or a property name, a setting of its own and not the viewer's FF&E tag) or `dismiss` it. **Elevations tag nothing by
  rule, on purpose**: a rule's offset is measured in plan (`tag_rules::to_offset` drops z), which says nothing about where a tag sits
  in an elevation, so an elevation needs offsets along the view's right and up and whether the item faces the viewer or is seen from
  the side. A chosen elevation tag is placed along the line only. Computing it reads the room's FF&E (seconds on RHH), so a request
  without `with_tags` skips it. The tasks page does not show it yet; the plug-in does not read it.

## The RLS creation script: settings, task, duHast

**The RLS create button** (`rls_create_entry`, `room_m/rls_task.py` plans, `room_m/rls_write.py` writes) makes room layout sheets from
`create_rls` tasks, **not yet run in Revit**. It claims only that kind (`claim_tasks(..., kinds=["create_rls"])`, and the Apply button
now claims only populate and update tasks, so the two never take each other's work). Order: the model's RLS settings first (extensible
storage, `rls` key; missing or incomplete ones open the settings window, still incomplete = stop), then the claim, then the WHOLE list is
checked before anything is made, then one transaction group per task (`duHast...in_transaction_group`), reported as it finishes.

- **RoomMate decides, Revit applies.** The task carries the size and where each elevation marker and its views go (`rls`); the settings
  supply the title block per size, the view types, the base schedule, the tags and the number/name/view-name templates
  (`{Room Parameter}` tokens, filled from the room). Abort (nothing written, reported): sheet exists, a view name is taken, no title block
  for the size, a view type or the base schedule not in the model, a template the room cannot fill, room/phase missing, settings
  incomplete. Warn (written, "completed with warnings"): no room tag, no independent tag, elevations the server could not place.
- **The phase is the room's own, never typed.** A `create_rls` task carries `ROOM_OWN_PHASE` as its phase (a label); the planner aborts if the room has
  none (`PHASE_MISSING`) and `rls_write` makes every view in `get_room_phase(doc, room)`. The request's `phase` is accepted and ignored.
- **A template's 3D viewport gives a NEW 3D view** (`rls_task._three_ds`, `rls_write._three_ds`, duHast `view_3d_create`): named `<view name> 3D`, looking
  along the template view's `forward_direction`/`up_direction` at THIS room, cut to the room's bounding box plus `SECTION_MARGIN_MM` (600 mm) on every
  side, in the room's phase, and its viewport goes where the template's was. **The template's eye is not copied** (the export reads zero for a default
  3D view, and a height measured in another room is wrong in this one): the camera sits back along the direction, so the pitch above the room is the
  template's. A 3D viewport with no usable direction is a warning. **Not run in Revit.**
- **View names are the view-name template plus ` PLAN` / ` ELEV n` / ` SCHEDULE`**; the schedule's FIRST filter is set to the room number.
  The plan view type is assumed to carry its view template; none is applied. Free-text sheet content, a number of sheets other than one per
  task, FF&E placement, elevation tags and the rectangle round a chosen item are **not done**.
- **The Revit-side assumptions are checked at run time, not trusted**: after the markers' views are made the marker is turned by how far the
  first view's `ViewDirection` is from the plan, and every view whose direction differs from the plan afterwards is a note. Also unverified:
  the crop box's far extent sits at negative z, and an elevation line is placed by the crop box's width and the far clip, not by an offset from
  the marker.
- **The Revit calls live in duHast** (PR in the duHast repo): `Family/family_symbol_lookup`, `Views/views_names`, `viewports_create`,
  `schedules_create`, `elevation_marker`, `Annotation/independent_tags_create`, `Rooms/room_tags_create` and `Common/transaction.in_transaction_group`,
  plus fixes to `sheets_create` (unicode text, activate the title block), `plan_view_create` and `elevation_view_create` (optional phase, the crop went to
  the plan view). **`extractor/duhast.lock` is not bumped**: build with `-DuHastPath` until that PR is tested and merged.

## Tag rules: where a tag goes, learned from tags in Revit views

Built: the observations endpoint, the rules store (`tag_rules`, `tag_rules_api`), MCP reads (`list_tag_rules`,
`get_tag_rule`), tag rules carried in a task, and the pyRevit side (the sheet read takes tags out of the sheet,
`room_m/sheet_tags.py`, and `post_tags.py` sends them). The tag classes and their reading from Revit are in duHast
(`Data/Objects/Collectors/data_tag*.py`, `Revit/Annotation/Export/to_data_tag.py`). **Not run in Revit, and
the RLS create button places tags** from the server's tag plan (`rls.tags`), not from these rules directly. Plan decision 24.

- **An item's rule is keyed by the item's TYPE CODE** (plan decision 27): the value of the one type parameter the project
  names in its settings (`tag_type_code`, under "Tag type code" on the settings page), with the tag's category, family and
  type and the viewport type. Every instance of a type shares the code, so a tag learned beside one bed is found for that
  type in any room, whatever the family and type are called; **never Revit ids** (a blueprint places new elements with new
  ids) **and never family and type names, with no fall-back to them**. An item whose type has no value is skipped with a
  reason and left untagged (`items_without_tags` says "no value for the type-code parameter"); a project that names no
  parameter keeps no item tags at all, and the extractor says so. The parameter rides the key (`code_parameter`), so codes
  read from two parameters never meet and changing the setting leaves the old rules unused (`rules_for` carries only those
  of the current parameter). A rule file of the name-keyed schema (version 1) is not loaded, only logged. A linked model's
  element is not read (its type is in another document) and is skipped. **A room, space or area tag is a rule too**
  (`tag_rules::is_spatial`): the tagged element has no code or facing, so its key is the category alone
  (`Rooms`, code and parameter empty), its offsets are in the plan's own axes, and a task carries it for every viewport of its
  type whatever items it places. Revit 2026's `RoomTag` names its room by `Room`; it has no `TaggedLocalElementId`. The ids in an observation say where a sample was
  read and are how a re-push replaces its own samples.
- **One rule per key**: the median of its samples, recomputed on every push, so a blueprint always tags the same
  way. The samples are kept; the answer is `placement`.
- **A rule is generic or bound to a room blueprint** (`key.blueprint`, `key.instance`, plan decision 27). A push that names a
  blueprint (the blueprint push applies its tags with `observe_for`, naming the blueprint) binds the room tag to it, and an item's tag to the blueprint
  ITEM (`instance` = the item id, the Revit element id the blueprint kept) when the item is one of the blueprint's (its current
  revision: a revision waiting to be accepted brings its tags with it); a tag on any other item stays generic. A bound item rule holds exactly one sample. **A
  sample lives in exactly one rule**: identified by its source (sheet, tag, element), pushing it again under another key moves it
  and a rule left empty is deleted (`take_moved_samples_out`). `rules_for` carries the generic rules and those bound to the
  sheet's blueprint, never another's. For an item, `rls_tags::rule_for` takes the rule bound to the blueprint item the room's
  item stands for, else the generic one by type code. **Which blueprint item an item stands for** is matched from the room's
  assignment of the sheet's blueprint (`rls_tag_place::blueprint_placements`, `rls_tags::match_instances`: same type code, nearest
  placement within `INSTANCE_MATCH_MM` = 500 mm, one to one); a room with no assignment gets generic rules.
- **A room tag is written against the room's BOUNDARY, not its middle** (`service::room_tag_place`, `Boundary`). A bound rule uses
  the wall recipe against the COARSE outline of the room the sheet was made for (`record.room`): the nearest edge, the fraction
  along it, paper mm into the room, carried to another room by `best_shift` and refusing a different coarse edge count as the
  elevations do. A generic rule uses the smallest rectangle round the room (four sides in a fixed order, long axis towards +x),
  which works for any shape. The server turns the recipe into a place in THIS room when the task is made (`rls.room_tag.head`, model
  frame, mm) and the RLS create button uses it; a rule learned while its room was not on the server has no boundary and keeps the
  old offset in the plan's axes, applied by the plug-in. Distances are paper mm so the rule holds at any view scale.
- **A sample is paper millimetres in the element's own frame** (x along its facing, y to its left), the model
  offset divided by the view scale. A rule therefore holds at any scale and turns with the item; the rotation is
  relative to the facing. An element with no point location or no plan facing gives no sample, said in `skipped`.
- **Tags never ride in a sheet.** The extractor takes them out of the viewports before a sheet goes to the library
  or a blueprint, and a task carries the rules for its sheets' viewport types (for a populate task only those for
  the type codes it places) as they were when it was made.

## Room layout sheets: a library kept by identity, and two buttons that choose

Built: the sheet library on the server (`sheets`, `sheets_api`, `service::sheet_suggest`),
two pyRevit buttons, MCP reads, and a page for the library and the tag rules at `/library/` (`src-js/library`: each
sheet's paper size, preview, rooms covered and blueprint or generic key, accept or reject a renumbered push, suggest, browse
and delete rules). The server can still set a sheet's room by `PUT` (a bare `room_id` is qualified as `suggest` does), but no page
does. **Not built: view template in the export (tags and view
scale are read: see Tag rules).** Rules that are expensive to get wrong:

- **Their own endpoint (`/projects/{id}/sheets`), apart from blueprints, and not a snapshot
  push.** Rooms, doors and the rest replace a model's state; the library is authored data
  kept by sheet identity, one file per sheet. A sheet is **generic or bound to one room blueprint**
  (`blueprint`, plan decision 27): the blueprint holds no copy.
- **Identity is the Revit sheet id plus the document title.** A match with the same sheet
  number updates the sheet; a match whose number changed is **stashed pending** for a person
  to accept or reject; a match on neither is a new sheet *even if its number is taken*.
- **The sheet body is duHast's export, as exported, and opaque to the server.** Tags, view
  scale, view template and view names are not in it. Typing it server-side would pin the
  server to an export that is still changing.
- **A sheet needs no room.** The Sheets button opens a window (`room_m/sheet_export_window.py`, rules in `sheet_export_form.py`):
  the model's sheets in one column and, beside each, the rooms it covers (none, one or several). The rooms are **not linked**: the
  extractor works out one figure from them (`room_figures`: smallest rectangle round all their outlines, summed area and items)
  and sends it as `room_summary`, with the labels in `rooms` and the paper size measured from the title block's bounding box in
  `size_mm` (mm as measured, never snapped to A0..A3). A sheet from a blueprint may still carry a `room` link; the library page no
  longer sets one. **Not run in Revit.**
- **`suggest` offers and never applies, and only for sheet creation mode.** Area, proportions
  and item count are weighed equally, each as a log-ratio, worked out from today's rooms and
  FF&E and never stored. Proportions is the smallest enclosing rectangle's long over short
  side, so a rotated room reads the same.
- **A blueprint's sheets are in the library, bound to it** (`blueprints_api::bind_to_library`, made for the
  blueprint's room), so a room layout sheet can be made from one with no items: the main viewer's
  **RLS tool** (zone toolbar, `viewer/rlsTool.ts`, `app/RlsPanel.tsx`: choose a library sheet with its preview, click rooms on the plan,
  Make) queues a `create_rls` task per room, **with tags worked out only for a sheet bound to a blueprint** (`with_tags`). The library and tasks pages no longer make RLS tasks. Matched by sheet identity: a re-push updates the sheet and a renumbered one waits for a
  person. **A generic sheet carries `room_key`**, an identifier from its room's bounding rectangle and item count (`4000x3000-12`,
  sides to the nearest 10 mm; none for a bound sheet or one without room figures). **A bare re-push (the Sheets button) never unbinds a sheet**; only a push naming a blueprint sets it. A library that cannot take the sheets fails the blueprint push (the blueprint is stored and flagged as having none). The library page shows each sheet's blueprint.
- **A room outline sent by the extractor has its collinear vertices dropped** (`room_m/ring_tidy.py`, rooms only).
  Revit puts a vertex wherever the boundary changes wall: 77.5% of RHH's rooms (2,080 of 2,683) have one and 28% of all
  boundary vertices (7,659 of 27,558) are one, nearly all within 0.5 mm of the line. Lossless, so the tolerance is
  0.5 mm and a real jog is never cut (that question is the server's coarse and fine outlines). A spike that doubles back
  stays. Spaces, ceilings and floors are not tidied. Snapshots already stored keep theirs until re-pushed.
- **Which wall an elevation looks at is derived at read time, never stored** (`service::elevation_facing`,
  `GET /projects/{id}/sheets/{sheet}/elevations`, MCP `get_sheet_elevations`). The sheet carries, per elevation viewport,
  its marker (`marker_id`, `marker_location`), the view's `marker_index`, the elevation line (`line_start`/`line_end`),
  `view_direction`, `view_depth` and the far clip (duHast PR #96; all absent on a sheet exported before it, which is a
  finding). An edge is faced when its inward normal points back at the viewer (`square_on` >= 0.1) and part of it lies
  between the line's ends and within the depth; the sheet's model-frame numbers are compared with the room put back
  through the inverse placement. **Occlusion is not judged.** Front-on versus side-on is the same `square_on`
  (1 front, near 0 side); tag rules do not yet key on it. **Not run in Revit**: the depth is read as minus the crop
  box's `Min.Z` and the marker's `Location` is assumed to be a point.
- **The Blueprint button pushes a room and its sheets to `.../blueprints/from-revit`; the Sheets button
  pushes sheets alone to the library.** The Blueprint button sends NO room contents: a rooms or FF&E push
  replaces a model's whole snapshot, so the blueprint's room is captured on the server from what is
  already pushed, and the button only says which room, which sheets and what name. A room with no sheet
  is skipped, and a blueprint is not pushed unless every chosen sheet could be read.
- **A library sheet stores a summary of the room(s) it covers** (`RoomSummary`: the enclosing rectangle's long and
  short sides, area, item count), sent by the extractor (the server reads one itself only for a sheet that names a linked room and
  sent none). Suggest ranks on it, so a sheet still ranks when its room has
  been redrawn or is not on the server; a sheet without one is read from its room as before.
- **Neither sheet entry point is in `ENTITY_EXPORTERS`.** They are selection-driven, and the
  generic run driver has no per-entity choosing step. **Not yet run in Revit**: the pyRevit
  side was checked for syntax, ASCII and duHast-import closure only.

## RLS settings: kept in the model, edited in a window

The settings of how an RLS is made in one model (title block per sheet size, view types, schedule base, tags, number/name/view-name
templates) live in the **Revit model's extensible storage** under the key `rls` of `room_m/project_settings.py` (one JSON text field with a
`version`, so a new setting is a new key). They are edited in a **XAML window** (`rls_settings.xaml`, drawn by `rls_settings_window.py`
through pyRevit's `forms.WPFWindow`; every field and rule that is not drawing is `rls_settings_form.py`, tested on CPython). The **RLS
settings** button opens it; the **RLS create** button opens it when the settings are not complete. It needs no server and no browser, and
returns its answer straight to the calling function. A status line says what is still to be chosen and which template words are not room
parameters. `rls_settings.collect_options` reads what the model offers (title blocks, view types, schedules, tags, room parameters); a list
it cannot read is left out, and the window says so.

- **What crashed the first WPF attempt is kept out on purpose, and tests enforce it**: no `x:Class` and no `DataGrid` or `ListView` (only a
  ScrollViewer, TextBlocks, ComboBoxes and TextBoxes), handlers are plain methods named in the XAML (a test checks each exists), and a
  field that changes reports to `on_change`. Do not add a grid of rows to it; a long list belongs in a ComboBox.
- **A stored value the model no longer has stays visible, marked "(not in this model)"**, and saving keeps it; keys the form does not know are
  kept, so a setting a later version wrote survives this one saving. `doc.Title` is the model id, as for every push.
- **The window needs the `.xaml` file next to the module**: `build-extension.ps1` copies `room_m` whole, so it travels. An earlier web-page
  version of this (a server draft endpoint and a React page) was tried and removed in favour of this.

## Serving a closed network: how Revit on another computer reaches the server

- **Loopback is the default and leaving it is an explicit opt-in**: `--bind 0.0.0.0` (or `ROOMMATE_BIND`; an
  address, never a name; IPv6 works). The server has **no authentication** and `/api/settings` writes project files, so
  this is for a network the operator trusts, never the internet. A non-loopback start prints the addresses to type
  (this machine's primary IPv4 and `<name>.local`) and a one-line warning. `src/network.rs` is the module.
- **The `Host` guard stays on off loopback, and that is the part that is easy to break.** A request from the Revit
  machine says `Host: 192.168.x.x:5151`, which the loopback-only guard refused. A network bind allows IP literals
  (a DNS-rebinding page cannot present one), this machine's own names (`mac`, `mac.local`) and `--allow-host` names
  (repeatable or comma separated, `ROOMMATE_ALLOW_HOSTS`); a Host naming any other domain is still a 403, on reads and
  on the settings writes. Do not widen it to "any Host" because a LAN client was refused: list the name.
- **CORS is irrelevant to the pyRevit side** (it is not a browser); a browser on the other machine loads the viewer
  same-origin from the server's address.
- **The macOS or Windows firewall is the usual reason a correct setup does not connect**: allow incoming connections
  for the binary, and use a fixed address or the `.local` name. A tunnel (`ssh -N -L 5151:127.0.0.1:5151 user@host`)
  needs none of this and keeps the server on loopback.
- **The pyRevit side takes the server from `room_m/server_config.py`, resolved when a request is made**, first match
  wins: the `ROOMMATE_SERVER` environment variable, the **open model's own settings** (extensible storage in the project
  file, one JSON text field with a `version` and a `server` key: `room_m/project_settings.py`), `server.json` in
  `%APPDATA%\RoomMate`, and then **nothing**: no silent loopback default any more. A computer that has never been told
  gets `ServerNotSet`, whose message points at the **Server** button (`local` there chooses this computer on purpose;
  `reset` forgets). The button checks the address answers, then writes the model AND the file, so a model carries its
  server to whoever opens it and a model without one still works from the file. The model's copy is shared by everyone
  and travels with a copy of the file, which is why it is only the second source. A typed address gets the default port
  5151 when it has none. **Every `post_*` module holds a PATH** (`/rooms`), joined to the base URL only in `post_common`
  (the project fetch and `_post_content`, which report `ServerNotSet` as a failed push, never raise): a module constant
  would freeze the address for a pyRevit session that stays open for hours, and `scripts/check_extension.py` fails on a
  literal loopback address anywhere but `server_config.py`. Every run prints which server it used and why
  (`provenance`); an unreachable one names the URL and the usual causes. Not run in Revit (the extensible-storage
  read and write are the untested part)

## Traps in the door export

- **`±1e30` is not geometry.** duHast used to return Revit's *uninitialized*
  `BoundingBoxXYZ` for a door it could not measure, and its own guards passed,
  so it arrived looking plausible. The producer drops it and sends empty
  `loops`; the door is still pushed, because it has real room references.
  **Keep the guard, but do not trust the story that came with it.** It was read
  as "these two families have no 3D geometry" — and that was never true. With
  duHast's geometry walk fixed (2026-08-07) both `2040x620x40` doors measure
  5.10 × 0.13 ft like any other. The sentinel was a *symptom of the bug*, not a
  property of the families, and every current House A door has a footprint.
  Old snapshots on disk still carry empty `loops`, which is why the guard and
  the empty-`loops` handling stay.
- **Never read `from_room`/`to_room` from the export.** They are per-phase
  arrays tagged with a `phase_id` that resolves against nothing on the wire. The
  extractor reads `FamilyInstance.FromRoom[phase]` from the Revit API instead.
- **The door's footprint IS trustworthy now, and the extractor must not
  re-derive it** (fixed upstream in duHast, 2026-08-07). It used to arrive as a
  world **axis-aligned** box — right on an orthogonal wall, an upright rectangle
  lying across a slanted one otherwise. Two attempts to fix it *here* both
  failed, and both are worth knowing so nobody writes them a third time:
  - `GetOriginalGeometry` + `GeometryElement.GetBoundingBox()` gets the angle
    right and the size **badly** wrong — measured ×1.97 along the wall and
    **×9.87 through it**, every door of one type reporting an identical box.
    That is the family *symbol*: uncut by its host, and `GetBoundingBox()`
    counts curve objects, so the plan swing arc is in the measurement.
  - Reconstructing the rectangle from the axis-aligned box is impossible, not
    merely hard: two extents plus an unknown angle is three unknowns against
    two measurements, degenerate at exactly 45°.

  The real fix was in duHast (`get_oriented_bounding_box_from_family_instance`):
  measure the *instance's* solids in the *instance's own frame*, and carry the
  placement on the box `Transform`. **If a door footprint ever looks wrong
  again, check which duHast the extension is running before touching
  `room_m/`** — an extractor that computes its own footprint silently discards
  whatever duHast sends, which is exactly how a correct duHast fix produced a
  byte-identical bad export.

## Passages: a drawn line is the record, the crossings are derived

A passage joins two rooms on ONE storey by a straight line a person drew
(2026-10-12), for a way through that no door and no shared wall shows.

- **Stored: the two clicks and the two rooms. Never the crossing points.** The
  points where the line leaves the first room and enters the second come from the
  outlines at read time (`connectivity::passage_fact`), so a passage follows its
  rooms when they move, and a click that no longer lies inside its room is a
  reported `reason`, not a stale edge.
- **A save that leaves out `passages` KEEPS them**, as `routes` does and for the
  same reason: a client written before the list existed sends none, and reading
  that as "empty" would delete them all. Only an explicit `[]` clears. Every body
  builder in `viewer/connections.ts` carries the list for the same reason.
- **It is not deduplicated against a door.** A zone or link is skipped when a door
  already joins the pair; a passage is drawn on purpose, and the stated width
  beside a door's estimated one is exactly when both should stay. Width and height
  are the author's; absent is unknown, so a size check lets it through and counts it.
- **Two ports, not one.** An `Edge` has `point` (on room `a`) and `far_point` (on
  `b`); anything that walks an edge in either direction goes through
  `Edge::length_between` / `crossing_from`, never `e.point` alone.

## Reach: a distance field, not a route

`POST /projects/{id}/reach` / `get_reach` lays a grid over the plan and gives each
cell the walk from the nearest of several start points (a hose reel's coverage is one use, an exit's travel distance another).
The viewer draws it as a heat map (below).

- **It is the router's own graph and walk, never a second one.**
  `connectivity::build_graph` is shared with the route read and `routing::field` is
  the same port network run from every source at once, so a cell's distance and a
  route's length agree. Don't give reach its own adjacency or its own in-room walk.
- **Three cell states, and they are different facts.** `-1` no room, `-2` a room
  beyond the limit or one nothing reaches, else whole mm. "Beyond" is deliberately
  not a number: the search stops at the limit, so a longer distance was never
  measured. A room no door, zone or passage reaches is `reached: false`, a finding
  about what is connected (RHH's GROUND has 1,378 rooms in no multi-room component,
  so a source there covers little until zones and passages are authored).
- **`ignore_cuts` is a scope switch on the graph, not a filter on the answer.** It
  empties each open zone's `disconnects` and `hub` before the zone edges are made
  (`ConnectivityScope::ignore_cuts`). A cut says where a person routes; a coverage question is not
  bound by it. The routes read never sets it.
- **Two methods, `exact` (default) and `grid`.** `grid` is Dijkstra over each room's
  cells seeded from the same port distances: the lattice approximation (RHH GROUND: median +0.2%, worst +14%
  long; changes with spacing), offered to compare against. Only `exact` agrees with a route.
- **The heat map is a GL mesh between the element layers and the labels.** One
  translucent quad per COVERED cell (`renderer/reach.ts`; `-1` and `-2` are left
  bare so the plan shows), painted above doors/windows/FF&E and directly below the
  labels, with the reader's opacity in the vertex alpha. It is not pickable. The
  tool is per zone (`ZoneRow.reach`), exclusive with route and open-zone editing
  because all three claim a room pick; it is measured on a button, never on a
  keystroke, and any change to what is asked clears the answer so a stale map never
  shows.
- **The SVG export draws it too, from the answer already in hand.** `renderer/svg/reach.ts`
  paints it between the element layers and the labels, as the screen does, and the
  export reads `zone.reach.result` rather than asking again: the answer holds every
  storey the walk touched, so each exported level takes its own grid and a level the
  walk missed draws none. The walk is cut into 24 bands, one path each (a file with
  one element per cell would be tens of thousands), and a legend states the scale.
- **Tags: route hops and reach cells, each its own zone toggle, both default OFF**
  (`ZoneRow.showRouteTags` / `showReachTags`, in the Layers menu; the SVG export reads
  them like every other toggle). A route tag is a hop's length and, in brackets, the
  walk from the start to the END of that hop (`routeHops`): the lengths are the
  server's (`step.length`, and what is left of `distance_ft` for the last leg), never
  re-measured from the polyline, which only says where on the plan a hop lies. A
  reach tag is the cell's walk in metres (`reachTags`), thinned by whole rows and
  columns only past a budget (4,000 on screen, 20,000 in a file) so a fine grid cannot
  build an unbounded number of text objects. The GL tag container is NOT moved by the
  scene: like the labels it needs the view transform set in `#pushView`, and without
  it the tags exist, report in `debugState().order` and draw nowhere.
- **A saved reach is the request, never the grid** (`connections::SavedReach`, in the
  connections document beside routes). Start rooms with their clicked points, the
  walk and grid in mm, method, `ignore_cuts`; the map is measured again when shown,
  so it follows the doors, zones and passages. A save that leaves out `reaches`
  KEEPS them (the routes rule), so only the reach panel sends the list. The numbers
  are checked by the same `reach::params` and `Method::parse` the read uses, so a
  saved reach cannot hold a request the read would refuse. In the viewer, "show" in
  the left panel loads it into the reach tool of the zone a pick last went to and
  measures it (`reachMeasure.ts`, shared with the bar's Measure): the map is a zone's
  state, not a second overlay, which is why two zones can compare two saved reaches.
- **One storey at a time.** Vertical edges are skipped: a walk on one plate does not continue by lift.
- **A room too small for a cell is judged on its inside point**, so a cupboard
  still gets a verdict. The grid's origin is a whole multiple of the spacing, so
  two reads at one spacing share cell edges.

## Linked models: one frame, one storey rule

Both were reported as five separate viewer bugs on RHH and are one cause each.

- **Geometry is placed into a PROJECT-LOCAL frame at read time, never into
  shared space.** `model_to_shared` rode the envelope for a long time with
  nothing applying it, which is invisible while a project's models share an
  origin (RHH's four architectural models agree to six decimals) and glaring
  when they do not (its five services models sit ~250 ft away, so the spaces
  layer drew beside the plan). `service::placement` composes `anchor⁻¹ ∘ model`
  in f64; the survey translations cancel and what is applied is a small local
  delta. **Do not "simplify" this to placing everything into shared space.**
  Shared space is the survey grid — RHH's is MGA Sydney, 2.06e7 ft — and the GL
  renderer uploads world coordinates as `Float32Array`, where that magnitude has
  a ULP of 324 mm. Measured, not feared. The anchor is `[project]
  anchor_model`, else the lexicographically smallest model id per project, and
  it must stay request-independent: it comes from `model_index()` because that
  is the only source all five entity reads share, and five reads disagreeing
  about the frame means doors that do not sit on rooms. The anchor's own
  geometry is untransformed, which is why House A and the golden SVGs did not
  move.
- **Which model is the anchor cannot make the geometry wrong.** Every model is
  mapped through the same rigid `anchor⁻¹`, so the choice moves the whole plan
  as one piece and never changes how models sit relative to each other — even
  if the anchor's own `model_to_shared` is misregistered. `anchor_model` exists
  for the two things that *do* leak out: exported coordinates (SVG, `/areas`)
  come out in the anchor's frame, and the derived anchor shifts if a
  lower-sorting model id is ever pushed.
- **`ModelEntry::placement` is an index fact, filled at push time and
  backfilled once at startup.** `bootstrap::backfill_placements` exists because
  every snapshot already on disk predates the field; without it the fix would
  only reach data pushed after the upgrade. It skips a model that already has
  one, so it costs a manifest read per project after the first run.
- **An element joins a storey by NAME + ELEVATION, never by level id.** A
  `Level.id` is per document; the level picker is built from `/rooms`, which has
  already deduped ids across linked models. Comparing the two directly fails
  silently in two directions — a model that lost the dedup race has every
  element dropped, and a model with no rooms (a facade package) contributes no
  canonical id at all, which is why RHH's 78 external doors were invisible on
  every level. The rule is `src-js/renderer/storey.ts`, shared by all four
  element layers, and it deliberately mirrors `rooms::dedup_levels`. Elevation
  alone is not enough: RHH's car park stacks "C 00" at the hospital GROUND's
  elevation. Every element read carries `levels_by_model` for it.
- **Element reads fetch only the storeys on screen, and the server's filter
  must stay a SUPERSET of `storey.ts`** (2026-09-17). The viewer sends
  `storey_elevations` + `storey_level_ids` for every zone's storey;
  `entity_scope::StoreyScope` keeps an element within `LEVEL_EPS_MM` of any
  requested elevation, keeps by id for a model declaring no levels, and
  narrows nothing when no model declares any. It never re-implements
  `onStorey`'s name-and-elevation rule, because that rule's fallbacks are
  decided over the whole payload: **change the rule in `storey.ts` and
  re-check the superset**, or a "(by elevation)" layer silently loses
  elements. RHH `/ffe`: 133.2 MB for every storey, 23.6 MB for one.
- **An `EntityPoll` revision only vouches for the URL that produced it.** A
  storey switch or `?building=` changes the body and not the contributing
  snapshots, so the revision stays equal; the poll treats a changed URL as a
  new scope and drops an answer whose URL moved in flight. A null URL (storeys
  not knowable before the first rooms payload) skips, so nothing asks for
  every storey on first load. The room contents panel says "storey not on
  screen" for a room whose storey no zone shows, rather than "none".
- **A room's label is placed from its REAL outline, by `renderer/labelPlacement.ts`
  (`placeLabel`), in both painters.** It went at the area centroid, sized by the
  bounding box: for a T or L the centroid can fall in the notch and the box is far
  wider than the arm the text sits in. Now: the pole of inaccessibility (holes
  excluded), the text centred on the open stretch around it, the font shrunk until
  the block is inside the outline. On RHH's busiest level 96 of 478 labels spilled
  before and none do. Never drops a label (a circle of radius `d` always fits), and
  caches per room so a repaint does not search again. The SVG goldens moved with it;
  change the rule there and nowhere else, or the file and the screen disagree.
- **FF&E's model choice is a set of checkboxes, as exclusions** (`ZoneRow.ffeModelsOff`,
  under the FF&E entry of the Layers menu once there is more than one model). Spaces
  pick one model; FF&E comes from several fit-out packages at once, so a zone draws
  every model unless it is unticked, a model pushed later is drawn by default, and
  unticking all draws none. Like spaces it is presentation: the read stays unscoped
  and the filter is applied at paint and in the SVG export (which names what is left out).
- **Element tags are the room label's placement on the element's own outline**
  (`renderer/elementTags.ts`, `[tags]` in settings, one entry per FF&E / spaces / ceilings /
  floors; toggled per zone in the Layers menu, default OFF). Text is resolved by the page
  (`viewer/elementTags.ts`: `$id`, `$name`, `$type_name`, else a property, instance then type),
  and placed by `placeLabel` on the footprint: a space's rings, the LARGEST piece of a
  ceiling or floor, an item's footprint or else the marker it is drawn as (so such a tag is
  tiny, because the object is). GL and SVG share `placeElementTags`. RHH's ceilings and
  floors carry no `type_name`, so the default `$type_name` prints nothing there until a
  property is named.
- **Settings that name a property are checked against the data** (`settings_check`, logged
  once at startup and returned as `findings` on `GET /api/settings/projects/{id}`; the
  page puts a red triangle on the setting and its section). Room label, colour plans,
  hover and element tags are covered. A name counts as present if it is in the dictionary
  (last line) of the latest snapshot of that entity in any model; **"cannot tell" is never
  a finding**: nothing stored for the entity, a pre-dictionary snapshot of anything but
  rooms (rooms are parsed), `$` tokens. A finding never refuses a load.
- **A layer that could not resolve exactly says so on its own toggle.** The
  "(all levels)" / "(by elevation)" suffixes are the point: a fallback nobody
  can see is the failure mode this area keeps producing.
- **The building picker has an "All buildings" option and defaults to it.**
  `?building=` scopes an opening or item *through the room it is attributed to*,
  so a homeless element — every window and external door in a facade package —
  matches no building. With every option naming a building there was no way to
  ask the unscoped question, and half of "I can't see any external doors" was
  that.

## The FF&E level came from geometry, and that was the bug

Same shape as the door footprint above, same lesson, different field.
`duHast.Revit.Family.Export.to_data_item` derived an item's level by walking to
the nearest level at or below its bounding box, and **ignored the level the
modeller assigned** — which the same export was already sending in
`instance_properties`. That is right only for a model whose levels are exactly
its storeys. RHH's interior models carry reference levels mis-elevated onto
other storeys' heights ("LEVEL 6" at 74000, where LEVEL 4 sits), and 9,186 of
15,070 items in one model exported onto a floor they were not on.

Fixed upstream 2026-09-07 (`get_item_level_data`: authored first, the walk only
for an item that states no level — which is what it was written for, a
face-hosted family). **The extractor must not re-derive it**, for the reason it
must not re-derive the door footprint: computing a level in `room_m` would
silently discard a correct duHast and produce a byte-identical bad export. If an
item draws on the wrong level, check which duHast the extension is running
first. **Snapshots exported before that date still carry the geometric level**
and will keep drawing on the wrong storey until they are re-exported.

## `crates/drofus-mcp`: a standalone MCP server for dRofus

A workspace member **only so CI builds and tests it**; it depends on nothing else here (no path dependency, no `workspace = true`, its
own lint table, no import of `roommate` or `roommate-shared`), so the folder can be copied out and become its own repository.
`scripts/check_drofus_standalone.py` (a CI step) fails if that stops being true. It is **a scaffold**: one tool, `server_info`; the dRofus
API's shape and the first real tools are undecided. A session working on it follows `crates/drofus-mcp/CLAUDE.md` and touches nothing outside
the crate. It is separate from `src/bin/mcp.rs` (the roommate data server) and from the `reference` module (dRofus as uploaded CSV).

## Which document wins

[`docs/README.md`](docs/README.md) indexes everything. **The strategy docs hold
only what is *not* built; the code documents what is.** Shipping a feature
includes deleting its description from the strategy doc — and does *not* mean
growing the doc comment to compensate. The rule and its failure modes are in
[`CODING-CONVENTIONS.md`](docs/CODING-CONVENTIONS.md) under "Code documents what
is built"; read it before adding prose to a `STRATEGY-*.md`.

`docs/Superseded/` is an archive — nothing there is live, live docs do not link
into it, and it pins `file.rs:NNN` line numbers that have drifted. Trust the
*symbol* name, search for it, never jump to the line.

## Verify before claiming done

```
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

All three are CI gates; clippy runs with `-D warnings`, so a warning is a
failure. Frontend changes are verified by driving the page, not by reading the
diff — a bug shipped this week was only visible after expanding a panel.

**Stop every server you start, as soon as you no longer need it.** That covers
`preview_start`, a background `roommate.exe`, and `npm run dev`. Nobody else
starts them: a server still running on :5151 was started by *some* session,
and it keeps serving whatever it loaded at startup (settings are not
file-watched). So before you stop one, check it is yours; another session may
be driving it. Measure against a second instance on its own `--port` and a
copied store rather than restarting a shared one.

**Touched `src-js/`?** Then also:

```
npm run typecheck && npm test && npm run build
```

`npm run build` is not optional. `static/vendor/renderer.bundle.js` is a
**generated file that is committed** (so a fresh clone plus `cargo run` works
with no node installed), and `.github/workflows/frontend.yml` rebuilds it and
fails if the committed copy disagrees. Forgetting it means a red PR, or worse a
green one serving a stale renderer. The same build, and the same gate, cover
`static/settings/`, `static/reports/` and — since the viewer moved to React on
2026-09-20 — `static/index.html` with `static/viewer.js` and `static/viewer.css`.
**Every page is generated now**: `static/` holds no hand-written HTML, and the
only hand-written files left in it are `common.js` and `tokens.css`.
The viewer's own Vite config writes into `static/` itself, which is why its
`emptyOutDir` is false and must stay false — emptying that directory would
delete the other three pages and the shared scripts. The settings page was the
first of these, on 2026-09-12. Two things that page did are worth remembering, because they are what
the replacement is shaped to prevent — it rebuilt the settings JSON field by
field on save (so it silently emptied every milestone's door, window, FF&E,
space and ceiling pins), and it had no controls for seven settings, which is why
`merge_over_stored` exists.

**Changed a settings type in `crates/roommate-shared`?** `cargo test` rewrites
the TypeScript the settings page reads (`src-js/settings/generated/`,
ts-rs), those files are committed, and `rust.yml` fails if the committed copy is
stale. So a Rust field added without re-running the tests is a red PR — and a
field added without a control on the page is a type error there, which is the
point: the page could not silently stop exposing a setting.

**The settings page's defaults are `src-js/settings/defaults.ts`, once.** A new project
starts from every setting at its Rust default, and the sections import the same
constants as their fallback; a second copy in a section is how page and server drift.
A save WRITES every section, defaults included (`Settings::fill_defaults`, in
`save_project`), so a file lists what the project runs on; the cost is that a later
change to a default does not reach a project already written. Only a setting whose
default is *none* stays absent, since TOML has no null. Each section is a `<details>`
expander (`Block`), closed except Identity, and stays mounted while closed.

**The UI framework is React, and that was measured, not preferred.** Rust+WASM
(Leptos) was built against the same slice and lost, chiefly because a committed
wasm cannot be reproduced across Windows and Linux. The numbers and what would
reopen it are in STRATEGY-BROWSER.md's "UI growth" — read that before proposing
Leptos or Dioxus again.

## House rules the code won't tell you

The rules in short form, so nothing here is violated mid-task by not having read
another file. Each is stated in full, with its reasoning, in
[`CODING-CONVENTIONS.md`](docs/CODING-CONVENTIONS.md) — go there before arguing
with one, not here.

- **Tests are inline**, never a `tests/` tree; a shared helper is duplicated per
  module rather than hoisted.
- **Doc comments carry the *rationale***, not a restatement of the what. This is
  the single most visible house style; matching ordinary Rust terseness reads as
  foreign.
- **`service/` is transport-agnostic** — never imports `axum` or `rmcp`.
  `handlers.rs` and `bin/mcp.rs` are thin adapters over it, and `bin/mcp.rs`
  keeps one tool per HTTP *read* route (update its count when you add one).
- **"Signal, not error"** — an unresolved cross-reference is usually a reported
  state, not a failure.
- **Line endings are LF**; see Traps below for the one way that goes wrong here.
- **A module past ~500 real lines is a split candidate**, not a violation — but
  if it stays whole, say why in its header.

## Empty pushes, and the two guards that were wrong

- **A rooms push with no rooms is a 422**, on both ingest paths. A push exists
  because someone exported a document that has rooms in it; an empty one is a
  producer fault, never an empty model. Not the same as an empty *level*, which
  is ordinary. The message names the phase, because a filter matching nothing is
  what it nearly always is.
- **Doors get no equivalent rule *server-side*** — a model with rooms and no
  doors is legitimate, and the server cannot tell that from a broken export. The
  extractor does refuse one; see the last bullet, and note the two are answering
  different questions rather than disagreeing.
- **`has_room_snapshot` is gone with the gate it served.** Worth knowing why it
  existed: it used to ask whether a rooms snapshot *file* existed, which an empty
  one does — so it waved through 26 doors referencing 22 room ids, none
  resolvable. That regression is now guarded from both ends instead:
  `reject_empty_rooms` stops the empty snapshot being written, and `door_report`
  reports every unresolvable reference on every read rather than once, at the
  push.
- **The extractor refuses empty pushes too, and for doors it is deliberately
  stricter than the server** (`post_rooms.empty_push_refusal`, 2026-08-06). The
  server must accept zero doors — it cannot tell a shell from a broken export.
  The producer is answering a different question ("someone asked for a doors
  push and there are none"), and it knows what the server never sees: how many
  the export held and where each one went, so its message names the phase filter
  instead of reporting a bare zero. The refusal rides the normal
  `(ok, status, text)` tuple with `status = None`, so callers need no second
  failure channel.
- **That refusal is scoped to the RUN, not to one model**, and the difference
  mattered. Asked per model, a rooms-only document in a multiselect run made the
  doors push fail and reddened an otherwise clean run — routine, and wrong. The
  residual cost is unchanged: **a run whose documents genuinely hold no doors at
  all cannot record that fact through this producer.**

## Traps

- **Shipped binaries are x64, and they come from CI, not a dev machine.**
  `release.yml` builds on `windows-latest` (x64), and that is the only source of
  an installer anyone is given. A dev VM may be Windows on arm64 (this one has
  been since 2026-10-05), where `cargo build --release` yields arm64 exes that
  will not run on Intel. That is fine for `cargo test`/`clippy`/running the
  viewer locally, and the reason **not** to hand out a local
  `installer\build.ps1` output. Decided deliberately: do not add
  `--target`/arch switching to `build.ps1` or `roommate.iss`.

- **A fixed duHast file is not a fixed duHast run.** A script exec'd into a
  Revit console keeps `duHast` in `sys.modules` for the life of the session,
  so an upstream fix on disk does nothing until Revit restarts. Measured
  2026-09-13: a House A floors probe run an hour after the `adjust_delta` fix
  had landed in every copy pyRevit loads still exported both holes as islands.
  The probes (v2) now ask the LOADED module a question the old code gets wrong
  and record the answer; the analysers warn on a stale run. "Check which duHast
  the extension is running" means the one in memory, not the one on disk.
- **Line endings are LF**, enforced by `.gitattributes`. Writing files through a
  Python heredoc on Windows silently converts them to CRLF — check with
  `git diff --stat` (it warns) after any scripted file write.
- **Contract is rooms v7 / doors v2, `phase` is required, and one push carries
  MANY models.** The envelope has a `models` list, not a `model` block, and every
  room or door line names the model it belongs to. A hand-rolled push in the old
  single-model shape gets a 422 naming a stale extractor.
- **The bucket is a transport shape, never a storage one.** Ingest decomposes a
  push into one snapshot per model, so `RoomPayload`/`DoorPayload`, `ModelKey`,
  milestone pins and every per-model read are untouched by it. What the run
  gains is one shared `taken_at` across its models — which is what makes "these
  documents were read together" expressible at all. Storing the bucket whole
  would flatten exactly the identity the rest of the codebase is built on.

## Open, as of 2026-09-07

- **`/ffe` on RHH: the read is fixed, the response is not.** Storage and memory
  now hold each property key and type bag once (`property_codec`, 2026-09-17):
  release assembly 17.3 s → 3.4 s, `/validation` 14.5 s → 4.5 s, the whole
  local store 1,459 MB → 279 MB. **Only for snapshots written since** — a legacy
  snapshot still reads (9.1 s) but keeps its old layout until re-pushed, and
  nothing rewrites the store for you. The *body* sends each type bag once since
  `service::type_table`: 279.6 MB → 133.2 MB, and only the storeys on screen
  since `StoreyScope` (one RHH storey: 23.6 MB). What is left: every snapshot is
  still parsed whole before the storey filter, instance property keys repeat
  per item on the wire, and `assemble_items` still clones each item it serves.
- **RHH's windows are pushed (2026-09-07, 2026-09-12) and draw — but "no
  external windows" is still what a reader will report.** Checked end to end
  2026-09-21: 368 windows, 152 of them in `HOS-FACADE`, every one resolving to
  a picker storey (bar 3 at `level_id` `-1`) and drawn. No facade window names
  a room, so they are attributed only by `[windows] room_resolution =
  "project"` (set in RHH's local settings the same day): 132 of 152, once a
  room-less model's opening stepped a wall thickness rather than the 15 mm
  centreline floor (`entity_scope::Candidates::cross_model_gap`). The 20 left
  homeless are LEVEL 8's 14 (no room within 8 ft of that facade), 3
  ambiguous and the 3 unhosted. Still true: the facade exports only punched
  windows and louvres (none on LEVEL 2 or 4) — curtain-wall glazing is not
  `OST_Windows` and no entry point exports it.
- **RHH's FF&E is on the wrong storeys until it is re-exported** — see the FF&E
  level trap above. The fix is upstream and lands on the next export.
- **`/ceilings` was 9.25 MB and 33 s on RHH** (1,833 ceilings against 3,112
  rooms). Unlike `/ffe`, the cost is not payload size but the attribution
  itself: every read intersects every ceiling with every room on its storey,
  because nothing is stored. With the bounding-box reject added alongside
  floors, a DEBUG build answers `/ceilings` in 7.8 s (1,816 ceilings) and
  `/floors` in 8.5 s (1,749 floors, 10.5 MB), 2026-09-16. The 33 s figure did
  not record its build profile, so the two are not a like-for-like speed-up.
  An index over room bounding boxes per storey is the next shape if it matters.
  Both layers default off so neither is on the first-paint path.

Both older items stay closed: the extractor's phase filter is verified against
Revit, and R4 landed.

## The extractor's export entry points, and one of them is a trap

`rooms_export_entry` still pushes **rooms and doors**, despite the name, so
narrowing it to rooms would not fail — it would keep succeeding while quietly no
longer pushing doors. Its button is gone (the Export window replaced it); the function name still reads wrong. The split is in
the siblings instead: `rooms_only_export_entry`, `doors_export_entry`,
`windows_export_entry`, `ffe_export_entry`, `ffe_by_space_export_entry`,
`spaces_export_entry`, `ceilings_export_entry` and `floors_export_entry`. Each
is one line over `export_entry(..., entities)`; document selection, the
one project and the one phase never differ. The Export window
(`export_all_entry`) is the same driver with the entities chosen by check boxes.

**FF&E identifies an item against a Room or a Space, never both, and that is
two entry points rather than a flag that probes the document.** A fit-out
services model usually holds no Room elements at all, so `ffe_export_entry`'s
`get_Room(phase)` resolves nothing for every instance in it.
`room_m.utils.items.item_facts` takes `identify_by` and reads `get_Space(phase)`
instead when asked; `ffe_by_space_export_entry` is the button that asks. Reading
both references per instance was rejected for the same reason
`rooms_only_export_entry` exists — it is the slow half of a push, doubled, for a
value the run was not asked for. **The result rides the same "ffe" server bucket
and the same
schema either way** — `Item.owner_spaces` is a list alongside `room`, populated
by whichever reference the run read, following `owner_rooms`'s list shape on
the wire though the extractor only ever fills zero or one. It is
extractor-authored, not server-derived: unlike `owner_rooms`, nothing
server-side joins it to anything, there is no `RoomResolution` equivalent, and
no `?building=` filter can see it — it is exactly what `get_Space(phase)`
answered in the push that sent it, carried straight through.

**The buttons live in this repository now** (2026-09-23), in
`extractor/pyRevit/RoomMate.extension`, and the installer ships them. That ends
the copy-under-the-duHast-tab arrangement and the `diff -rq` that was the only
thing keeping the two in step — but **the NET8 copy is only gone once it is
deleted there**, which is the last step of the move and is verified in Revit
first. Until then a dev machine loads both, and the panel appears twice.

- **A checkout is not a loadable extension.** `lib/` is empty on purpose:
  `room_m` has one source and duHast has its own repository.
  `installer\build-extension.ps1 -Deploy` assembles one and writes it to
  `%APPDATA%\pyRevit\Extensions`, which is the development loop;
  `-DuHastPath` builds against a local duHast instead of the pinned one.
- **duHast is pinned in `extractor/duhast.lock` and shipped**, python only —
  nothing `room_m` imports loads one of its DLLs, and
  `scripts/check_duhast_closure.py` is what keeps that true. Shipped rather
  than found because duHast decides what an export MEANS, and a copy of
  unknown age answers differently while still looking successful.
- **The check that decides runs in Revit, not in the installer.**
  `room_m.utils.provenance` prints the RoomMate build, the duHast build and
  the path it loaded from at the top of every run, and warns when that is not
  the shipped copy. The installer's scan of other duHast copies is a warning
  about a snapshot; `sys.path` order and an already-imported module decide the
  real answer. The buttons declare a clean engine for the same reason.
- **The TAB is `duHast`, the EXTENSION is `RoomMate`, and neither name is
  cosmetic.** pyRevit merges ribbon tabs by tab name
  (`create_ribbon_tab(..., update_if_exists=True)`), so the panel joins the
  duHast tab where that extension is installed. It keys its parse cache on the
  EXTENSION name (`cache_<name>`) and builds command ids from the folder names
  inside it, so a second `duHast-2025.extension` would collide with the real
  one in both.

**A run exports every selected model first, then pushes one bucket per entity.**
So `entities` no longer carries a push *order* — the buckets are independent, and
`ENTITY_EXPORTERS` lost its `blocking` flag with the server gate that justified
it. A cancelled run pushes nothing at all rather than sending the models read so
far: a partial run stored under one snapshot id would read downstream as "these
are the documents that were exported together", which would be a lie.

A doors-only push does **not** check that rooms are on the server first, and now
neither does the server. Doors may be pushed before their rooms; an unresolvable
reference is reported by `door_report` as *pending* rather than refused.

## Spaces: what the fifth entity does differently

Three rules are this entity's alone, and each was measured on RHH rather than
reasoned into:

- **An empty spaces push is sent and stored**, where an empty rooms push is a
  422 on both ends. "This services model was audited and holds no spaces" is the
  finding the entity exists to report, and a different fact from "never pushed".
- **A disagreeing phase is quarantined, not refused.** The openings refusal
  exists because promoting would strand `from_room`/`to_room`; a space carries
  no room id. And a services model usually holds no rooms, so "re-phase it with
  a rooms push first" is advice it cannot take — refusing would fix its lineage
  on the first phase that reached it, permanently. `put_pending_raw` is keyed by
  `(model, kind)` for this.
- **Enclosure is stated by the extractor, from `Area` and the export's polygon —
  never from `GetBoundarySegments`.** 96.3% of RHH's spaces report zero boundary
  segments while exporting a perfectly good polygon, because their bounding
  elements are in the linked architectural model. `Perimeter` is `0.0` for the
  same reason and is no substitute. Unplaced spaces are dropped; unenclosed ones
  are pushed with empty `loops`, which is the difference `translate_room` cannot
  make and `translate_space` does.

**A spaces run is per phase, and the disciplines disagree on the name.** RHH's
mechanical model keeps 1,532 of its 1,533 spaces in `Future` while its siblings
use `New Construction`, and the push phase is one per *run*. A run under the
common name pushes three models in full and one space from the fourth, correctly
and silently — which is why `exporters/spaces.py` reports "N of M spaces are in
phase X" for every model on every run. Read that line.

## Phase filtering: rooms and doors are not alike

Verified against Revit 2026-08-03, and it cost five empty pushes to find.
**A room BELONGS to one phase** (`ROOM_PHASE`); a door is built in one and may
be demolished in a later one. So rooms use an equality test
(`room_mate.rooms_in_phase`) and doors use the range test
(`elements_in_phase` / `exists_in_phase`). Running rooms through the range test
returns *nothing* — silently. Both paths raise on an unknown phase name, and
that guard is the only thing standing between a typo and another five empty
snapshots.
