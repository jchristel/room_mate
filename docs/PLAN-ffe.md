# RoomMate — FFE implementation plan

> **Status: built — A, C, D, E, F and G are all in.** Revit → extractor →
> `/ffe/stream` → store → `/ffe` → the plan, with `items` on `/validation` and a
> `get_ffe` MCP tool. Verified end to end against the real House A export: 644
> items pushed, 465 read back (179 components excluded by policy), drawn,
> picked and inspected in the browser.
>
> What is left is outside this repository: the pyRevit button for
> `ffe_export_entry`, and duHast's U1 and U2. Until U1 ships, every item is
> drawn as a marker rather than a footprint — which is the state the viewer was
> designed around rather than a gap in it.
>
> This records the design agreed *before* any code, so the implementation did not
> re-derive it and the open questions were open before the work rather than after
> — the discipline the phasing and windows plans were both written under, and why
> both are still readable as records now that their work has landed.
>
> **PR A has run against House A (2026-09-05) and the kill condition is
> cleared.** The decisions below were written from a reading of duHast's source
> and are left standing rather than edited into agreement with the data;
> [As measured](#as-measured--house-a-2026-09-05) records what the probe found,
> including the two predictions it inverted and the one upstream change it
> demoted. Read that section before trusting any figure above it.

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Entities](STRATEGY-ENTITIES.md) · [Server](STRATEGY-SERVER.md) ·
[Browser](STRATEGY-BROWSER.md) · [MCP](STRATEGY-MCP.md) ·
[Conventions](CODING-CONVENTIONS.md)

## Why FFE exists as an entity

**FFE sits in the same Revit file as the rooms, and Revit cannot schedule FFE
against those rooms.** RoomMate performs the join Revit will not.

That single sentence is the demand, and it decides more than it looks like it
does. The model that pushes FFE is the model that pushes rooms, so
`FamilyInstance.get_Room(phase)` has a live room to answer with and the join is
same-model everywhere — which is what the model-scoped discipline already
requires of doors and windows. The facade-file pathology that made
`[windows] room_resolution` load-bearing does not arise: there, zero of 158
windows named a room because Revit cannot resolve one across a link, and
geometry was the *only* attribution mechanism. Here authored references are
expected to populate, and geometry is a fallback.

It also means the entity earns its place with or without a reference source.
dRofus sits on top rather than underneath: it verifies room data and FFE data as
two separate sets, and FFE may later be augmented by it the way rooms are
already. R4 scoped reference sources per entity for exactly that, so
`[sources.reference.<name>] entity = "ffe"` is a settings line rather than a
design. Plenty of projects run without dRofus at all, and those projects still
want the join.

**This is the harder test [Entities](STRATEGY-ENTITIES.md) named.** Windows were
a second *opening* and cost one `ENTITY_EXPORTERS` row plus one envelope. FFE is
the first candidate that is not an opening — it sits *in* one room rather than
between two — and the finding below is that three of the generalisations were
opening-shaped rather than entity-shaped. That is a pass for the bet, not a
failure: each one widens, none needs replacing.

## The finding: an item is not an opening

`DataDoor` and `DataWindow` both extend `DataFamilyBase`, which mixes in
`DataElementGeometryBase`. `DataItem` extends `DataBase` alone. Read from
duHast at `SampleCodeRevitBatchProcessor-NET8/src/duHast`, which is the copy
this plan is written against and the one the upstream changes land in — the
other two copies on disk predate `data_window.py` and are not live.

| Fact | Opening (`DataDoor` / `DataWindow`) | FFE (`DataItem`) |
|---|---|---|
| Footprint | `polygon` — oriented box flattened to a loop, decimal feet | **absent**; no geometry mixin at all (changed by U1) |
| Rotation | derived extractor-side from `FacingOrientation` | `location_point.rotation_coord`, a 3×3 matrix of world-space basis vectors |
| Position | read extractor-side from `LocationPoint`, feet | `location_point.translation_coord`, **metric mm** |
| Room reference | `from_room` / `to_room`, per-phase arrays of `DataRoomToPhase` | `rooms` — flat list of integer ids, **unioned across every phase**, untagged |
| Room fallback | none | third fallback is `doc.GetRoomAtPoint(...)`, producer-side geometry |
| Level | `get_level_data` — the element's own level | `get_level_data_by_bounding_box` — nearest level at or below the solid bbox's min Z |
| Z extents | `bounding_box_min_z` / `max_z`, `room_calculation_point` | absent |
| Silently dropped | a door duHast cannot measure (14 of 205 on the facade file) | any instance whose `Location` is not a `LocationPoint` |
| Category | one, fixed | nine by default, and the list is an argument |
| Properties, nesting | identical — same `get_instance_properties` / `get_type_properties` helpers, same `super_component_id` |

The last row is why this is cheaper than the table suggests:
`post_common.properties_to_map` works on an item unchanged, and the two property
tiers mean the same thing they mean on a door.

## Decisions

### D1 — The record is a sibling of `Opening`, not a widening of it

`contract::items::Item`, carrying `id`, `level_id`, `category`, `room`
(one `Option<String>`), `insertion_point`, `facing`, `loops`, `type_id`,
`type_name`, `properties`, `type_properties`. Its `PropertyTiers` impl is the
same two lines `Opening`'s is.

The line that settled every windows question was **share it unless sharing would
change a serde key**. It does not reach here, because the records genuinely
differ rather than differing only in what their envelope calls them. The honest
extension is: **share it unless sharing would make a field mean nothing.** A
one-sided opening would carry a `through_wall_normal` describing nothing and a
`from_room` that is permanently `None` — and `OpeningReport::external` counts
"a room on exactly one side", so every item in the model would be reported as an
external opening. A field that is right in shape and wrong in meaning is worse
than a new type.

### D2 — The footprint arrives flattened, and RoomMate does not re-derive it

duHast will flatten the instance's oriented bounding box exactly as
`to_data_door` does — `get_oriented_bounding_box_from_family_instance` →
`convert_bounding_box_to_flattened_2d_points` → `convert_xyz_in_data_geometry_polygons`
— so an item's footprint reaches the wire as a rectangle in **decimal feet**,
placed and rotated, in the room convention verbatim. `loops` on `Item` is then
the same field it is on `Opening`, and one renderer and one `model_to_shared`
transform still serve every entity.

The alternative — local-frame extents composed against `rotation_coord` by the
consumer — is more compact and is what a symbol renderer wants. It was rejected
because it is a *second geometry convention*, and `Opening::loops`' own header
says a second convention forks every consumer that draws or transforms geometry.
The compactness argument is real but it is the deduplication problem, not a
geometry problem: a local-frame box is close to a **type** fact, and belongs on
the shared type table [Entities](STRATEGY-ENTITIES.md) defers rather than on
every instance now. Close to, not exactly — flexed instances of one type differ,
which is why per-instance flattening is the safe interim.

`rotation_coord` is carried beside it anyway, projected to plan as `facing`. A
rectangle does not say which end is the front, and a symbol renderer will want
that later.

**The extractor must not compute its own footprint.** That is the one rule from
the door work that outlives it, and it cost two failed attempts to learn: an
extractor that measures its own box silently discards whatever the export sends,
which is exactly how a correct duHast fix produced a byte-identical bad export.
Once U1 lands, the footprint is present and correct, so RoomMate reads it.

### D3 — The room reference is read from Revit, never from `DataItem.rooms`

`get_room_ids` tries `get_Room(phase)` across *every* phase and unions the
results, then `FamilyInstance.Room`, then
`doc.GetRoomAtPoint(point, last_phase)`. Two things are wrong with that field
for this pipeline and both are familiar:

- it is the `phase_id` trap again — a union across phases, untagged, and
  RoomMate pushes exactly one phase, so the export cannot say which entry
  belongs to it;
- the third fallback is a *geometric* answer computed producer-side, where the
  server can no longer report `SideOrigin` or a disagreement. That is what
  `windows_location_by_rooms.py` was rejected for, and it arrives
  indistinguishable from an authored reference.

So the extractor reads `room_reference(instance, phase, "Room")`, which
`utils/room_refs.py` was given a `which` argument for before this entity
existed. Geometry stays server-side under `[ffe] room_resolution`.

This is the third consecutive entity where the export's own room reference is
the thing to ignore, and it is worth stating as the general rule: **the export
answers "which rooms, ever"; RoomMate asks "which room, in this phase", and
only the live document can answer that.**

### D4 — One room, so there is no attribution policy

`[ffe]` carries **no `room_attribution`**. The five-way `RoomAttribution` policy
exists because a door lies between two rooms and something has to choose; an
item names one room or none, and there is nothing to choose.

`ItemResponse.owner_rooms` stays a **list** of zero or one all the same. Keeping
the shape identical is what lets "empty means homeless" mean the same thing on
every entity, and lets `owner_rooms_qualified` and `?building=` work with no
special case — a homeless item drops out of a building-scoped view exactly as a
homeless door does.

`room_resolution` defaults **`Off`**, matching doors rather than windows, and
the argument is stated rather than inherited: windows had it argued up because a
facade model's openings are unattributable from authored data alone, and per
"Why FFE exists" above that model shape does not arise here.

### D5 — `items` beside `openings` in the QA response

`ValidationResponse` grows `items: BTreeMap<String, ItemReport>`, keyed `"ffe"`,
sitting beside `openings` and `sources` and keyed the same way, so a reader
navigates all three alike.

Not *inside* `openings`: that key is named for what it holds, and an item is not
an opening. It drops the two findings that are two-sided by construction —
`external` ("a room on exactly one side") and `room_attribution` — and nothing
an existing consumer reads is renamed.

**As built, the sharing this section promised was mostly not there, and the
correction is worth more than the prediction.** This said `ItemReport` would
reuse `UnresolvedRoomReference`, `PendingRoomReference`, `OpeningPhaseDrift`,
`RoomGeometryMismatch` and `RoomResolutionCounts` *verbatim*. Reading them, one
does: `RoomResolutionCounts`, which counts what the geometry answered and has
nothing entity-shaped in it. Two carry a `side` an item has no equivalent for,
and three name their element `opening_id` or `openings_phase`.

So the rule that made `Item` a sibling of `Opening` applies one level up —
**share it unless sharing would make a field mean nothing** — and the report got
six small structs of its own. That is still far below the ~250 lines the
`DoorReport` → `OpeningReport` rename was made to avoid, but for a different
reason than this section assumed: an item simply has fewer findings, not because
the types were reused.

**One finding is deliberately not a discrepancy, and that is a departure from
the doors report rather than an inheritance.** An item naming no room is listed
in `without_room` and left out of `discrepancies`. A door with neither side
connects nothing and is nearly always a data problem; an item outside every room
is very often correct — a bollard, external furniture, plant on a roof — and on
House A that is 75 of 647. Folding 11.6% of a model into a discrepancy count
would train a reader to ignore the count. `unresolved_room`, which names a room
the model demonstrably does not have, stays a real finding.

### D6 — Category is a record field, not yet a setting

All of duHast's `DEFAULT_ITEM_CATEGORIES` are pushed: `OST_Furniture`,
`OST_FurnitureSystems`, `OST_Casework`, `OST_MechanicalEquipment`,
`OST_ElectricalEquipment`, `OST_ElectricalFixtures`, `OST_PlumbingFixtures`,
`OST_SpecialityEquipment`, `OST_GenericModel`. Narrowing that list is the
exporter's problem later.

**`OST_Casework` is the ninth, added 2026-09-05 after the probe found the list
exported the wrong half of the model** (see [As measured](#as-measured--house-a-2026-09-05)):
87 of 179 nested instances were `OST_Furniture` components of *casework*, all
one family, so the joinery handles shipped as first-class items while the runs
they belong to were absent entirely. A list that admits the components of a
thing but not the thing is harder to justify than either including or excluding
both. The change is upstream, in `to_data_item.DEFAULT_ITEM_CATEGORIES` (U4),
and `scripts/probe_ffe_export.py` transcribes it — **the two must move
together**, because the probe walks its own list while the export walks
duHast's, and a drift makes every count a comparison of two populations. The
analyser refuses to interpret a run where they differ, which is the guard that
makes that safe rather than a hazard.

The distinction that survives: `category` is a **field on the record** even
though it is not a **setting**. So QA can break findings down by category and
`?category=` filters through the existing grammar on day one, and the day an
allow-list is wanted it changes what is pushed rather than what the wire can
say.

**And the extractor must supply that field itself, because the export has no
key for it.** `DataItem` carries `super_component_id`, `rooms`,
`location_point`, the two property maps, `level`, `revit_model`, `phasing` and
`design_set_and_option` — and no category, despite `get_all_item_data` walking
eight of them and discarding which one each instance came from. So `category`
is the one field on `Item` that is read from the collector pass rather than from
the export, which is a *third* rule alongside D2 (read the geometry from the
export) and D3 (read the room from Revit). It is not an exception to either: the
export does not carry it at all, so this is filling a gap rather than choosing
between two answers.

It also means the probe cannot attribute a dropped instance to a category by
reading the export, and has to join the export's ids against its own per-category
collection to do it. That is why `probe_ffe_export.py` keys its records by
element id and counts per category separately, rather than counting the export.

`OST_GenericModel` being in the set is what makes the probe's category histogram
and super-component matrix the two numbers that matter most — see the two
open risks below.

### D7 — The read side splits at the join, not at the top

`assemble_openings` is a scoping-then-deriving pipeline. Nine of its steps are
entity-agnostic — scope resolution, milestone pinning, the snapshot sweep,
candidate-room construction, the reference join, the filter grammar, revision
hashing, phase reporting. Two are not: how many room references an element has,
and how geometry resolves them.

Those two are precisely the "glue, not geometry" `service::room_locator`'s own
header predicted. So the shared spine is lifted into a module both call,
`OpeningResponse` stays, `ItemResponse` is added, and **`OpeningKind` gains no
`Ffe` variant** — it is the per-*opening* lookup table and would answer wrongly
for an entity that has no `to_room`, no opening policy section shaped like one,
and no two-sided pins.

`room_locator` gains exactly one function: its existing private `probe()`
exposed as `locate_within(point, elevation, candidates)`. `Ambiguous`,
`NoCandidate`, `NoPosition` and `UnknownLevel` keep their exact meanings;
`NoDirection` is unreachable for FFE, which is the shape of "an element with one
side" the module doc already described.

### D8 — Point-based only, and the skip must be countable

`populate_data_item_object` returns `None` for any instance whose `Location` is
not a `LocationPoint`, so every line-based family — continuous casework runs,
benching systems — is absent from the export. Supporting them is deferred
deliberately.

What is **not** deferred is the silence. The collector does not append, so what
is missing leaves no hole, which is the exact shape of duHast dropping 14
curtain-wall doors invisibly: `populate_data_door_object` returns `None` for
anything it cannot measure and the caller does not append it, so the loss is
undetectable from the export alone. One counter in `get_all_item_data` (U3)
turns a deliberate limitation into a stated number.

### D9 — Two unit systems, named rather than merged

After U1, one `DataItem` carries a footprint in feet and a position in mm.
`convert_XYZ_to_point3` converts to mm; the polygon path goes through
`get_point_as_doubles`, which does not convert — House A's stored doors are at
`26.62`, plainly feet.

`location_point` is **not** changed upstream. Other duHast consumers read it and
mm is its documented contract. Instead the mix is named at both ends: a sentence
in the `DataItem` docstring, and a named `MM_PER_FOOT` conversion at RoomMate's
producer boundary rather than a bare `/ 304.8`.

**A silent unit mix is how `room_locator::LEVEL_EPS_FT` came to mean half a
millimetre.** Level elevations are already on the wire in mm (House A's LEVEL 00
is `110250.0`) while every polygon is feet. It is currently harmless, because
elevations are only ever compared to other elevations — but the constant
documented as "half a foot … far tighter than any storey height and far looser
than the float noise a transform introduces" is doing neither job, and two
models naming one floor 0.4 mm apart already fail to match, silently, as
`UnknownLevel`. Fixed in PR C as its own commit, with the finding written into
the header rather than the constant quietly changed. It is not FFE's bug; it is
on the axis FFE leans on, and FFE is what found it.

### D10 — Nested components are excluded by a read-time policy, not by the extractor

`super_component_id` rides the `Item` record, every instance is pushed, and
`[ffe] nested_components` decides at read time whether a component is an item.
Default `exclude`; `include` is the opt-out. `ItemReport` states how many were
excluded, so the exclusion is a reported number rather than a silence.

**Three things this decides, and the evidence for each is in
[As measured](#as-measured--house-a-2026-09-05).**

*Which test.* Not `nested_opening_ids`' "is the parent the same category" — that
catches 10 of 179 nested instances on House A, 5.6% of the population it exists
to catch, because an item's parent is usually in a *different* category
(furniture in casework, generic models in electrical fixtures, generic models in
doors). The test is **having a super-component at all**.

*Why not the doors discriminator.* A nested door component carried neither a
room reference nor a Mark. A nested item sits physically in a room and Revit
says so: 97.8% of nested items name a room against 84.8% of top-level ones, so
"no room reference" is useless here. Only the Mark signal survives — 5.6%
against 52.4% — and it is suggestive rather than safe. Inheriting the doors rule
would have looked reasonable and been wrong twice over.

*Why at read time rather than in the extractor.* This is the one place FFE
should **not** follow doors, and the reason is what the two questions are. "Is
this door leaf a door" has one answer: no, always, everywhere — so
`nested_opening_ids` filters at the producer and nothing downstream ever sees
one. "Is this component an item" is a project convention: a handle is not, and a
chair nested in a workstation group might be. A convention belongs where every
other convention in this codebase lives — in settings, derived at read time,
changing every answer and rewriting nothing, exactly as `room_attribution` does.

It is also what would have made the door incident visible on the day. 2236 of
4134 exported "doors" were hardware, and they were invisible because the
producer had already dropped them by the time anyone could count them. Here the
count is on the QA report from the first push.

The cost is a payload 28% larger than it needs to be on House A, which is real
and is the right trade: it buys a policy that can be changed after the fact and a
number where there was a silence. It also **removes** work — the extractor needs
no nesting pass at all, so `utils/items.py` reads exactly one thing from Revit
(`get_Room(phase)`), and `nested_opening_ids` stays where it is, doing the job it
does correctly for openings. Two entities answering one question differently,
because it is not the same question.

## Rejected, recorded so they are not re-proposed

- **FFE as a one-sided `Opening`.** See D1.
- **FFE as a reference source on rooms.** Fails
  [Entities](STRATEGY-ENTITIES.md)' three-part test: it is extracted from the
  model, carries its own identity and placement, and an FFE schedule joins
  *onto* it.
- **One `SnapshotKind` per Revit category.** Eight lineages, eight pin maps and
  eight endpoints for one bucket the producer reads in one pass. Category is a
  property of an item, not an identity — the same argument that rejected
  `/openings?category=window`, reaching the opposite conclusion because the
  facts are opposite: there, one lineage would have merged two independently
  pushed entities; here, eight lineages would split one.
- **Widening `rooms_export_entry`.** Its pyRevit button lives outside this
  repository, so it would keep succeeding while silently changing what every
  existing button pushes. Note that "same file as the rooms" makes a combined
  rooms-and-FFE run genuinely useful — but that is a *new* entry point and a new
  button, one line of `export_entry(..., (ROOMS, FFE))`, not a change to that
  one.
- **An ingest-time gate requiring rooms before FFE.** [Entities](STRATEGY-ENTITIES.md)
  says not to re-add it for the next dependent entity, and the reasoning holds
  unchanged: "not yet" is a legitimate answer, and `opening_report`'s
  pending/dangling split re-answers it on every read.

## The upstream duHast changes

All three land in `SampleCodeRevitBatchProcessor-NET8/src/duHast`. They are
sequenced first not because RoomMate blocks on them — it does not, empty `loops`
is a state the contract already carries — but because U1 is what makes the
viewer layer nearly free.

**U1 — `DataItem` carries the instance footprint.** Give it the geometry mixin
and populate `polygon` through the three calls `to_data_door` already makes.
`convert_bounding_box_to_flattened_2d_points` applies the box `Transform`, so
the four corners come out world-placed and rotated rather than at the origin;
that is the fix that made the door footprint trustworthy and it is reused whole.
The rotation needs nothing — it is already on `location_point`.

**U2 — the oriented box merges sub-component solids.** A precise difference
between the two bbox helpers, and it lands where FFE is worst.
`get_solids_based_bounding_box_from_family_instance` merges
`GetSubComponentIds()` into its result; `get_oriented_bounding_box_from_family_instance`
— the one that keeps the rotation, and therefore the one U1 needs — measures
only `family_instance.get_Geometry()`. A desk whose drawers or monitor arm are
nested shared families would measure to the desk top alone. The merge happens in
the instance's own frame, where the inverse transform is already in hand.

Note U2 interacts with the nested-component filter: **the components excluded as
leaves are the ones the bounding box should be including as geometry.** They are
the same elements answering two different questions, and getting one right does
not get the other right.

**U3 — `get_all_item_data` reports what it skipped**, per category. See D8.

**U4 — `OST_Casework` joins `DEFAULT_ITEM_CATEGORIES`.** Landed
2026-09-05, and the only upstream change so far that the probe demanded
rather than the plan predicted; see D6 and
[As measured](#as-measured--house-a-2026-09-05). It is a behaviour change for
any duHast caller relying on the defaults, which is the argument for it being
in the default list rather than in RoomMate's own: a caller who wants the old
eight can pass them, and a caller who did not know casework was missing was
getting the wrong answer quietly.

Two more, worth doing on their own merits and not blocking anything here:

- **Drop the `GetRoomAtPoint` fallback from `get_room_ids`.** It mixes a
  geometric answer into a field every other consumer reads as authored. Fixing
  it does not change RoomMate's behaviour — per D3 the extractor reads Revit
  regardless, because the export cannot carry the run's phase choice.
- **Line-based instances**, deferred per D8.

## Sequence

**A — Probe the item export.** `scripts/probe_ffe_export.py` (collects, in
Revit) and `scripts/analyse_ffe_probe.py` (decides, offline), on the split the
windows probe used: the expensive half needs Revit and must be right first time,
the cheap half is where the thinking goes and can be re-run and argued with.
Six questions, none answerable from source:

- how often `get_Room(phase)` populates on a real same-file model;
- how many collected instances `populate_data_item_object` skipped, per
  category;
- the category histogram across all eight;
- the super-component category matrix;
- the bbox-derived level against the instance's own `Level` parameter;
- the unit of every numeric field.

Run against **House A first**, and a doors control from the same pass with the
same duHast — the committed `scripts/fixtures/doors-raw.json` cannot serve,
because it was captured from an older duHast and a field missing from it proves
nothing about today. The windows probe made this mistake impossible by writing
`doors-*-control.json` and never over the committed fixture; do the same.

**A2 — the second probe, against RHH, after implementation.** Deliberately not
part of the gate. House A answers "is the record what we think it is" on a model
small enough to check by hand; RHH answers "does it hold at scale", which is a
different question and one the implementation has to exist to ask. This is the
same two-document logic the windows probe used in one pass, split across time
instead — and it is the honest place for it, because the scale risks (the
category histogram under `OST_GenericModel`, the nesting matrix, payload size)
are exactly the ones a small sample cannot settle.

**B — U1 and U2 upstream.** Parallel with A, not behind it.

**C — Contract, storage, settings.** `SnapshotKind::Ffe` — the `ALL` guard makes
an omission a compile error, which is the guard that exists because
`list_models` once iterated a hand-written array and would have left every
windows snapshot out of the model index. Then `contract::items::Item`,
`contract::ffe` envelope and stream types, `SUPPORTED_FFE_SCHEMA = 1` (its own
version line, for the reason windows started at 1 rather than matching doors: a
version records *a contract's own* history and this one has none), manifest
index, `[ffe]` settings, `ReferenceEntity::Ffe`, a fourth milestone pin map.
Every test a round-trip. Carries the `LEVEL_EPS_FT` fix from D9 as its own
commit.

**D — Ingest and read.** `POST /ffe`, `POST /ffe/stream`, `GET /ffe`. The D7
spine extraction lands here and wants splitting: do it as its own commit with
`/doors` and `/windows` byte-identical *before* `/ffe` exists, so a regression
has one commit to be in. The pinned wire-shape test that guarded the doors
generalisation is the precedent.

**E — Extractor.** Ahead of QA, because nothing downstream is testable against
real data until a push exists. `room_m/exporters/ffe.py`, `post_ffe.py`,
`utils/items.py`, one `ENTITY_EXPORTERS` row, `ffe_export_entry`. The Revit pass
reads exactly one thing — `get_Room(phase)` — because D2 and D3 between them put
everything else in the export or out of scope; it is much smaller than
`opening_placements`. Phase filtering uses the **range** test
(`elements_in_phase`), not the equality test rooms use: an item is built in one
phase and may be demolished in a later one. Everything ASCII, because
IronPython 2.7 will not parse a file with an em-dash in it, not even in a
docstring.

**F — QA.** (The MCP tool moved forward into PR D: `bin/mcp.rs` keeps one
tool per HTTP *read* route and `scripts/weekly_review.py` checks the claim, so a
route landing without its tool would have left a known-red check standing across
two PRs. Parity is a route concern, not a QA one.)

**F — QA, continued.** `ItemReport` under the new `items` key (D5). One new tool,
`get_ffe`, which needs three things kept in step that are hand-maintained by
design: `bin/mcp.rs`'s spelled-out tool count, `STRATEGY-MCP.md`'s list, and
`scripts/weekly_review.py`'s route-to-tool mapping. Its description is written
fresh — an agent applying the doors reading to a furniture payload reports a
correct model as broken, which is the lesson [MCP](STRATEGY-MCP.md) already
carries from windows.

**G — Viewer.** A fourth layer and a fourth toggle, polling on its own revision
like the windows layer does. If U1 lands as D2 describes, this is close to free:
the footprint arrives in the room convention and the existing renderer draws it
with no new geometry code, leaving the toggle, the pick order and the inspector
panel. Depends on B; runs parallel with F.

## As measured — House A, 2026-09-05

**PR A has run. The kill condition is cleared and the plan proceeds**, with two
predictions inverted and one upstream change demoted. The predictions above are
left standing rather than edited into agreement with the data: what a plan got
wrong is worth more than a plan that reads as though it never guessed.

Document `Building_BF_Framing_jan.r.christel`. 647 instances collected across
the eight categories, 644 exported. Report in `temp/ffe-probe-report.md`.

### The kill condition is cleared

| phase | items naming a room | % |
|---|---|---|
| Existing | 0 | 0.0 |
| New Construction | **572** | **88.4** |

Zero lookup errors. Revit does know which room an item is in, so there is a join
to perform and the entity carries what rooms do not. D3 is vindicated
specifically: the reference is read per phase, and the Existing row shows what a
phase-agnostic union would have blurred.

### What held

- **D8's deferral is nearly free.** 3 instances dropped of 647 — every one a
  `LocationCurve`, every one an `OST_GenericModel`, and **zero** dropped that
  had a `LocationPoint`. The "softer stop" below does not fire: line-based
  support can wait.
- **D9 is confirmed empirically rather than by reading.** 643 instances measured
  twice; the median ratio between the export's X and Revit's is **304.8**
  exactly. The export converts to millimetres and RoomMate divides.
- **D1 holds in both directions.** An item carries `location_point`
  (`translation_coord`, `rotation_coord`) and `rooms`; a door carries `polygon`,
  `from_room`/`to_room` with their `phase_id`/`room_id`/`revit_model_name`,
  `room_calculation_point`, the Z extents and `associated_elements`. Neither is
  a subset of the other. For windows this diff was empty; here it is full, which
  is the whole reason `Item` is a sibling and not a widening.
- **`loops` is 0% populated**, exactly as predicted pre-U1, and 0% is why U1 is
  scheduled rather than assumed.

### What inverted

**The nested-component filter cannot be the doors one, and the margin is not
close.** 179 of 647 instances (27.7%) have a super-component, and the parent is
the *same* category for **10** of them — so `nested_opening_ids`' test would
catch 5.6% of the population it exists to catch.

| child | parent | count |
|---|---|---|
| Furniture | **Casework** | 87 |
| Generic Models | Electrical Fixtures | 70 |
| Plumbing Fixtures | Generic Models | 6 |
| Generic Models | **Doors** | 3 |

All 87 are one family, `Handle_Joinery_FIJO_900` — joinery handles, which is the
`PS Aluminium` pull-handle finding again at a quarter of the scale. Two things
follow that the doors experience does not give you:

- **`OST_Casework` is not one of the eight**, so the casework runs are absent
  from the export while their handles are present. That is backwards, and it is
  a question about D6's category list rather than about the nesting filter.
- **The doors-era discriminator fails here.** A nested door component carried
  neither a room reference nor a Mark; a nested *item* sits physically in a room
  and Revit says so.

| | names a room | carries a Mark |
|---|---|---|
| nested (179) | 175 (97.8%) | 10 (5.6%) |
| top-level (468) | 397 (84.8%) | 245 (52.4%) |

So "no room reference" is useless as a filter and "no Mark" is suggestive but
not safe. The only reliable discriminator is **having a super-component at
all** — which is on the wire as `super_component_id`, populated on exactly those
179.

Both findings are now decisions rather than open questions: the category list
gains `OST_Casework` (D6, upstream as U4) and the filter becomes a read-time
policy over `super_component_id` rather than an extractor pass (D10). The second
is the one place FFE deliberately does **not** follow doors, and this
measurement is the whole argument for it.

**U2 is demoted from a fix to a tidy-up.** The two duHast boxes disagree on
height for 184 instances, and only **4** of those have sub-components. The rest
split 135 where the solids box is taller and 49 where the oriented box is —
which is not one cause but at least two, neither of them nesting. Merging
sub-components into the oriented box is still right, and it is worth knowing it
addresses 2% of the disagreement rather than the bulk of it. None of this
touches the footprint: `loops` is 2D and a height difference cannot reach it.

### What was better than feared

**The level heuristic is wrong on 9%, not on the third C6 implied.**

| outcome | items |
|---|---|
| exported level agrees with the level the instance names | 516 |
| exported level disagrees | **53** |
| export carries no level (no solid geometry) | 71 |
| instance names no level — the heuristic is the only answer | 4 |
| fell back: below every level in the document | 26 |

The 71 with no exported level are why the verdict reads `PROCEED WITH CARE`:
`level_id` is 89% populated, not 100%, and an item with no solids has no
storey. That is a real state to carry, not a defect to fix.

### One finding about the instrument, not the model

The analyser first reported **103** level disagreements by re-running duHast's
rule here and comparing that. The export itself disagrees on 53. The gap is
entirely method: the re-derivation was fed Revit's own element bounding box
while duHast feeds it the *solids* box, and per U2 above those differ on 184
instances for reasons that have nothing to do with levels.

**Re-deriving a producer's rule to check the producer measures the
re-derivation.** The analyser now compares the level the export actually
carries, and keeps the re-derived figure only as a labelled diagnostic — the one
thing that explains *how* a disagreement arises once the export has said there
is one. It is the same error in miniature that D2 forbids at full scale: an
extractor that computes its own footprint silently discards what duHast sent.

## What would stop this

**The single kill condition: `get_Room(phase)` does not populate on a model that
holds both the rooms and the FFE.** That is the premise of the entity as stated
at the top — RoomMate performs the join Revit will not schedule. If Revit does
not know which room an item is in, there is no join to perform, containment is
the only mechanism left, and the entity carries little rooms do not already
have. It is one number out of PR A and it is the number to read first.

**A distinct outcome that is not a stop: House A may hold no FFE at all.** "No
items in this model" and "items present but unresolvable" are different answers
and the analyser must not collapse them — the first means find another model,
the second is the kill condition. Naming the difference before the data arrives
is the point of writing the verdict conditions down now.

**A softer stop:** if the skipped-instance count from D8 is a large fraction of
a real model rather than a handful, the ordering changes — line-based support
comes before RoomMate builds a QA report on a set it knows to be incomplete.
That is a scheduling question, not an abandonment, and PR A measures it either
way.

## Open risks, carried rather than resolved

- **Volume.** 26 doors is 414 KB, mostly repeated `type_properties`. Eight
  categories including `OST_GenericModel` plausibly puts one storey into four
  figures of instances. [Entities](STRATEGY-ENTITIES.md) defers type-property
  deduplication until measured, with `type_id` already on the wire ready to key
  a shared table — FFE is what measures it, and D2's rejected local-frame box
  is the same table. Streaming ingest already handles the push; what it does not
  handle is the viewer holding a whole payload to draw one level, which is the
  one place the doors design assumed "far fewer than rooms" and said so in
  `post_entity.post_stream`. PR A reports the instance count so PR G is
  planned against a number, and A2 is where that number becomes real.

- **Nesting.** `nested_opening_ids` exists because 2236 of 4134 exported
  "doors" on one job were hardware and leaves, none carrying a room reference,
  all landing in the homeless pile and making a data artifact look like a
  modelling gap. Its test is "is the parent the same category", which is well
  defined for one category and ambiguous across eight: a nested generic model
  inside a furniture item is a different statement from a chair inside a chair.
  The likely answer is a per-category rule in `[ffe]` rather than in code —
  which category counts as a component is an office convention, the argument
  `room_reference_property` already won. See also U2, which wants the opposite
  answer about the same elements.

- **The level heuristic.** An item's level is the last level at or below its
  solid bounding box's minimum Z, with the lowest level as a fallback for
  anything below all of them and an empty `DataLevel` when there are no solids.
  A ceiling-mounted projector, a wall-hung basin, a pendant light: all assigned
  to the storey *below* the one they serve if their geometry starts high enough
  — and unlike `UnknownLevel`, this answer looks correct.

  It matters less for attribution than it first appears, because with authored
  `Room` populating, containment is a fallback that rarely runs. It matters
  *more* for the viewer, because a wrong level draws the item on the wrong floor
  plan, where a human sees it. The probe reports the disagreement rate against
  the instance's own `Level` parameter; if it is material, the parameter wins
  and the derived value becomes the fallback — the precedence authored data has
  over geometry everywhere else here.

- **The fourth unwired button.** `windows_export_entry` exists and its pyRevit
  button still does not; `ffe_export_entry` will be the second in that state.
  Both live outside this repository. Worth counting rather than discovering.
