# RoomMate — FFE implementation plan

> **Superseded — built and merged (PRs #97–#102, 2026-09-05).** The live
> pointers are the code: `contract::items` for the record, `contract::ffe` for
> the envelope, `contract::SnapshotEnvelope` for the seam that let one pipeline
> serve four entities, `service::entity_scope` for what every read shares,
> `service::items` for what an item does differently, and `room_m.post_entity`
> plus `room_m.post_ffe` on the producer side. Kept as the record of *why* the
> design looks as it does — and, in [As measured](#as-measured), of the four
> things the data changed and the two bugs that only a screenshot found.

**Status: built.** FF&E goes Revit → extractor → server → storage → QA → MCP →
plan. This records the design agreed before any code, so the implementation did
not re-derive it and the open questions were open *before* the work rather than
after.

Part of the Roommate strategy docs: [Index](../STRATEGY.md) ·
[Entities](../STRATEGY-ENTITIES.md)

---

## The question this plan existed to answer

**What does an entity that is NOT an opening cost?**
[Entities](../STRATEGY-ENTITIES.md) had claimed the envelope, the store, the
property machinery, the filter grammar and the geometric room locator all
generalize. Windows tested that claim and passed it cheaply — but windows reuse
the `Opening` record, so they reused the whole opening stack by construction.
FF&E was the first candidate that could not.

## The demand, which decided more than it looked like it would

**FF&E sits in the same Revit file as the rooms, and Revit cannot schedule FF&E
against those rooms.** RoomMate performs the join Revit will not.

That single sentence settled the shape of several answers. The model that pushes
FF&E is the model that pushes rooms, so `get_Room(phase)` has a live room to
answer with and the join is same-model everywhere; the facade-file pathology
that made `[windows] room_resolution` load-bearing does not arise, so FF&E
defaults it `Off`; and the entity earns its place with or without dRofus, which
verifies room data and FF&E data as two separate sets and is absent on plenty of
projects entirely.

## The line that settled every subsequent question

Windows were decided by **share it unless sharing would change a serde key.**
That line cannot reach a case where the records genuinely differ, and the honest
extension is:

**Share it unless sharing would make a field mean nothing.**

Modelling an item as a one-sided `Opening` was the tempting move. It would have
compiled, kept every serde key, and reported *every item in the model* as an
external opening, because `OpeningReport::external` counts "a room on exactly
one side". A field that is right in shape and wrong in meaning is worse than a
new type.

The same test decided the QA report one level up, and there it overruled the
plan — see [As measured](#as-measured).

## Rejected, recorded so they are not re-proposed

- **FF&E as a one-sided `Opening`.** See above.
- **FF&E as a reference source on rooms.** Fails
  [Entities](../STRATEGY-ENTITIES.md)' own three-part test: it is extracted from
  the model, carries its own identity and placement, and an FF&E schedule joins
  *onto* it.
- **One `SnapshotKind` per Revit category.** Eight lineages, eight pin maps and
  eight endpoints for one bucket the producer reads in one pass. Category is a
  property of an item, not an identity — the mirror of the argument that
  rejected `/openings?category=window`, reaching the opposite conclusion because
  the facts are opposite: there, one lineage would have merged two independently
  pushed entities; here, eight would have split one.
- **Widening `rooms_export_entry`.** Its pyRevit button lives outside this
  repository, so it would keep succeeding while silently changing what every
  existing button pushes.
- **An ingest-time gate requiring rooms before FF&E.** "Not yet" is a legitimate
  answer, and `item_report`'s pending/dangling split re-answers it on every read.
- **Local-frame extents plus a rotation, instead of a flattened footprint.** More
  compact, and what a symbol renderer wants — but a second geometry convention,
  which `Opening::loops`' own header says forks every consumer that draws or
  transforms geometry. The compactness argument is real and belongs to the
  deferred type-property table, not to the record.
- **Re-deriving a producer's rule to check the producer.** See the last section.

## What was measured before any code moved

The premise — "an item is not a door, in these eight ways" — came from reading
duHast's source, which is how the `±1e30` sentinel, the axis-aligned footprint
and the unresolvable `phase_id` all got believed before. The exposure was worse
than windows': that plan claimed *identity*, which a diff confirms outright,
while this one asserted what is **absent**, which a single export cannot
demonstrate.

So the first PR was an instrument, not a feature: `scripts/probe_ffe_export.py`
(collects, in Revit) and `scripts/analyse_ffe_probe.py` (decides, offline), with
a doors control captured in the same pass from the same duHast — the committed
fixture being months old and therefore unable to prove anything about today.

**The verdict line distinguished four outcomes, and naming the middle two before
the data arrived was the point.** "No FF&E in this model" and "FF&E present but
unresolvable" are different answers: the first means find another model and costs
nothing, the second was the kill condition. A verdict reporting "0 items name a
room" for a model containing no items would have been true and would have ended
the work for the wrong reason.

---

## As measured

Document `Building_BF_Framing_jan.r.christel`, 2026-09-05. 647 instances
collected across the categories duHast walks, 644 exported.

**The kill condition was cleared: 572 of 647 items (88.4%) named a room in the
pushed phase, and 0 did in the phase beside it** — which is exactly what a
phase-agnostic union would have blurred, and the vindication of reading the room
from Revit rather than from the export's own `rooms` field.

### What held

- **Deferring line-based instances is nearly free.** 3 dropped of 647, every one
  a `LocationCurve`, every one an `OST_GenericModel`, and **zero** dropped that
  had a `LocationPoint`.
- **The unit mix is real and the conversion is right.** 643 instances measured
  twice give a median ratio of **304.8 exactly** between the export's position
  and Revit's. Confirmed a second time, independently, when the shipping
  translation was run over the whole export: 0 of 644 disagreed with Revit.
- **An item is not a door, in both directions.** A full-depth key-path diff is
  full both ways. For windows the same diff was empty, which is the whole reason
  `Item` is a sibling and `WindowOpening` is an alias.

### What the data inverted

- **The nested-component filter could not be the doors one, and the margin was
  not close.** 179 of 647 instances have a super-component and the parent is the
  *same* category for **10** — so `nested_opening_ids`' test would catch 5.6% of
  the population it exists to catch. An item's parent is usually in a different
  category: 87 furniture components of casework (one family of joinery handles),
  70 generic models inside electrical fixtures, 3 generic models inside doors.

  Worse, **the doors discriminator does not transfer at all.** A nested door
  component carries neither a room reference nor a `Mark`; a nested item sits
  physically in a room and Revit says so — 97.8% of components name a room
  against 84.8% of top-level items. Only `Mark` still separates them (5.6%
  against 52.4%), and suggestively rather than safely. That left
  `super_component_id` as the one reliable discriminator, which is why it is on
  the wire and why the exclusion is a **read-time policy** rather than a
  producer-side filter. The count of what it removed is on every response and
  every QA report, which is what would have made 2236 hardware "doors" visible
  on the day.

- **`OST_Casework` was missing from the category list**, so the joinery handles
  shipped as first-class items while the runs they belong to were absent
  entirely. Fixed upstream; a list that admits the components of a thing but not
  the thing is harder to justify than either including or excluding both.

- **The sub-component bounding-box merge was a tidy-up, not a fix.** It had been
  the plan's largest upstream ask. Measured: the two duHast boxes disagree on
  height for 184 instances and only **4** of those have sub-components; the rest
  split 135 solids-taller against 49 oriented-taller, which is at least two
  causes and neither is nesting.

- **The QA report shared one type, not five.** The plan said `ItemReport` would
  reuse five finding types verbatim. Only `RoomResolutionCounts` does — two carry
  a `side` an item has no equivalent for, and three name their element
  `opening_id`. So *share it unless sharing would make a field mean nothing*
  applied one level up. Six small structs was still far below the ~250 lines the
  `DoorReport` → `OpeningReport` rename avoided, but for a different reason than
  predicted: an item simply has fewer findings.

### What was better than feared

The level heuristic disagrees with the level an instance names on **53** items,
not the third an early reading implied — and that early reading is the finding
below. 71 items carry no exported level at all because they hold no solids,
which is a real state to carry rather than a defect to fix.

### The two bugs a screenshot found and the tests did not

- **The FF&E mesh never got `setView`**, so it rendered against stale projection
  uniforms and painted two large black rectangles over the plan. This is the
  *same* bug the windows layer shipped. Both times the geometry was provably
  correct — measured here over the real export: no NaN, no coordinate above
  105 ft on a 100 ft plan — and both times every unit test passed. Twice is a
  pattern, so `GlPlanRenderer.#worldMeshes()` now exists and both `#pushView`
  and `setAreasActive` iterate it. **Drive the page after a renderer change.**

- **Re-deriving a producer's rule to check the producer measures the
  re-derivation.** The analyser first reported 103 level disagreements by
  re-running duHast's own rule and comparing; the export disagrees on 53. The gap
  was entirely method — the re-derivation was fed Revit's element box while
  duHast feeds it the solids box. It is the error `contract::items`' header
  forbids at full scale, committed in miniature inside the instrument built to
  catch it.

### What a fourth entity actually cost

| Shared, one implementation | Genuinely per entity |
|---|---|
| the upload envelope, snapshot-id resolution, the phase rules | the record — an item is not an opening |
| the store, the manifest, `SnapshotKind` | the envelope and its schema version |
| scoping, milestone pinning, revision, phase report (`entity_scope`) | attribution — one room, so no policy |
| candidate rooms and the probe (`room_locator`) | which locator entry point: sides, or containment |
| the filter grammar, `PropertyTiers`, the reference join | the intrinsics — `$category` and `$room` |
| the push transport and envelope (`post_entity`) | the translation, and the one field the export lacks |
| `ENTITY_EXPORTERS` — one row, again | the glyph, and what it means |

Three names changed to say what they had always meant: `OpeningEnvelope`'s five
shared methods became `SnapshotEnvelope`, `service::openings`' spine became
`service::entity_scope`, and `post_openings` became `post_entity`. None of them
was ever about openings; no non-opening had asked.

## Where the surviving open items went

Per [the archive rule](../README.md#archive), nothing live is left here. The
items this work did not close moved into
[Entities](../STRATEGY-ENTITIES.md)' **Deferred** section, which owns them:

- duHast's U1 (the instance footprint, without which every item draws as a
  marker) and U2 (merging sub-component solids);
- the FF&E level heuristic, and whether the instance's own `Level` should win;
- FF&E at scale — everything known comes from one house, and RHH is the model
  the probe is waiting for;
- the pyRevit buttons for `ffe_export_entry` and `windows_export_entry`, both
  owed outside this repository;
- type-property deduplication, which FF&E is what finally makes worth measuring.
