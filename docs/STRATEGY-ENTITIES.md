# RoomMate — Entities

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[Area calculation](STRATEGY-AREA-CALCULATION.md) ·
[Browser](STRATEGY-BROWSER.md) · [MCP](STRATEGY-MCP.md) ·
[Authored](STRATEGY-AUTHORED.md) · [Security](STRATEGY-SECURITY.md)

**Open work only.** Rooms, doors, windows, FF&E and spaces all ship — contract,
ingest, storage, read, QA, MCP, the plan and the pyRevit exporter — and phasing
ships under them. What each of those does, and the invariants that are expensive
to rediscover (tier precedence, opening ownership, item attribution, the
model-scoped element→room join, the phase rules, and the three rules that are
spaces' alone), live in the code and in `CLAUDE.md`.

What is left here is the **entity dimension**: the test that decides whether the
next candidate is an entity at all, what five entities proved comes for free,
and what is still unbuilt.

**The bet below has been tested three times, and the second and third tests are
the ones worth having.** Windows were the third entity and cost nothing structural — they reuse
the `Opening` record, so they reused the whole opening stack by construction.
FFE was the first candidate that is *not* an opening, and it is the reason three
things in this codebase are now named for entities rather than for openings:
`contract::SnapshotEnvelope` (the six facts every stored snapshot answers),
`service::entity_scope` (the scoping, pinning and geometry every read shares),
and the extractor's `post_entity` (the transport every push shares). Each was
opening-shaped only because no non-opening had ever asked.

**Spaces were the third test, and they broke the one rule the first two left
standing: the element→room join is model-scoped everywhere *except* here.** A
space lives in a services model and the room it matches lives in the
architectural one, so its join is keyed and **project-scoped** — the single
exception, and the reason it is safe is that the key is unique project-wide
rather than that the rule was wrong. What spaces cost structurally was a
per-`(model, kind)` pending slot (a quarantined spaces push must not displace a
quarantined rooms one) and nothing else; what they reused, by being the *rooms*
record rather than the openings one, was the whole rooms read path.

**What did NOT need widening is the more useful half of that result**: the phase
envelope, snapshot-id resolution, the bytes-at-the-boundary store, the manifest,
the property tiers, the filter grammar, the reference-join namespace and the
`ENTITY_EXPORTERS` table took a fourth *and* a fifth entity without changing
shape.

Two lines decided every split, and the second is the extension the fourth entity
forced:

- **Share it unless sharing would change a serde key.** That is why the
  `Opening` record is shared while the doors and windows *envelopes* are not — a
  stored snapshot names its element list after its own entity, and every file
  already on disk says so, making a merged
payload type a migration rather than a refactor.
- **Share it unless sharing would make a field mean nothing.** The first line
  cannot decide a case where the records genuinely differ, and FFE is that case.
  Modelling an item as a one-sided `Opening` would have compiled, kept every key,
  and reported every item in the model as an external opening, because
  `OpeningReport::external` counts "a room on exactly one side". A field that is
  right in shape and wrong in meaning is worse than a new type — and the same
  test decided the QA report, where only one finding type turned out to be
  genuinely shareable.

## What makes something a primary entity

A thing is a **primary entity** when it is extracted from the model, carries its
own geometry and identity, and other data joins *onto* it. A thing is a
**reference source** when it arrives from outside the model and joins onto
something else.

The practical test, and the one to apply to the next candidate: **does anything
join onto it?** If yes, it needs identity, storage, and an endpoint of its own.
If it only joins onto something else, it is a reference source and the existing
`[sources.reference.<name>]` machinery already covers it. Doors qualified on all
three counts; a door *schedule* does not — it is reference data for the door
entity.

**Geometric room association generalizes too, as of phase 2.**
`service::room_locator` takes a point, an optional plan direction and candidate
rooms — never a `Door` — so windows (same two-sided shape) and an FF&E instance
(one side, `Room` rather than `FromRoom`/`ToRoom`) need the glue, not the
geometry. The extractor already made the same split: `room_reference(instance,
phase, which)` takes the property name as an argument because that is the only
thing that varies per category. **A category whose Revit references go
unpopulated in a split-model setup needs exactly that glue and nothing else.**

**What generalizes to every primary entity:**

- the upload envelope, and snapshot id resolution through
  `contract::ensure_taken_at` / `validate_snapshot_id` — never a
  reimplementation;
- `SnapshotStore` and the `project.toml` manifest as the index;
- the source-native flat property map plus settings-driven canonical name
  resolution;
- the `<source>.<label>` reference-join namespace and the `?filter=` grammar;
- "signal, not error" for an unresolved cross-reference.

**What does not, and is per-entity work every time:**

- geometry semantics — a room's boundary regime means nothing to a door's swing
  footprint;
- connectivity and ownership;
- which canonical property names exist at all. `Mark` on a door and `Mark` on a
  room are different properties that happen to share a spelling, which is why a
  reference source and a `[[builtin_properties]]` entry each declare an `entity`.

**A dependent entity needs one more thing**, and it is a consequence of
depending on rooms rather than of being new:

- **model-scoped reference resolution** everywhere the join appears.

Every entity that joins on a room **id** has needed it, FFE included: an item's
`room` is a room id, so the join is model-scoped in the read, in QA and in the
extractor's per-model `facts` map.

**Spaces are the exception, and the test that tells them apart is what the join
is keyed on, not how new the entity is.** A space matches a room on a *property
value* rather than an id, across documents, so scoping it per model would match
nothing — the two live in different Revit files by construction. An id is only
unique within its model; the space key is unique project-wide, which is what
makes the wider scope safe rather than merely convenient. So the rule for the
next dependent entity is: **joining on an id means model-scoped, and nothing
else may be**; joining on a key means the key's own uniqueness decides the
scope, and that uniqueness has to be *checked* rather than assumed — which is
why `SpaceReport` reports ambiguous keys as an invariant violation.

**It used to need two, and losing the second is worth recording.** There was an
*ingest gate*: a doors push was refused unless the target `(project, model)`
lineage already had a live rooms snapshot. It is gone. The gate asked "can these
references resolve **now**?", which has a legitimate answer of "not yet" — rooms
may arrive in a later push — so refusing meant refusing data that becomes
resolvable the moment they do. It was also the only place in the server where an
unresolved cross-reference was an *error* rather than a reported state, against
the "signal, not error" rule listed above.

The check did not disappear; it moved to `service::validation::opening_report`,
which distinguishes **pending** (this model has no rooms yet — expected) from
**dangling** (the named room is not among the ones it has — a finding). That is
a distinction the gate could not make at all, and it is re-answered on every
read rather than once, at the push, on the least information anyone will ever
have. **Do not re-add an ingest-time gate for the next dependent entity.**

## Deferred

- **Door connectivity graph.** Door connectivity is a genuinely different graph
  from `/projects/{id}/adjacency`, not a refinement of it: two rooms can share a
  wall with no door in it, and a door can connect two rooms sharing almost no
  wall. It is a second edge set, so adjacency keeps its meaning and connectivity
  gets its own endpoint.

  **The simple question is already a read, not a computation** — every door on
  `/doors` names both of its rooms, so "which rooms are connected by a door"
  needs no endpoint. What a real connectivity endpoint adds is the *graph*:
  traversal, components, path length. Worth building when something asks for one.

- **Design options, as a second model-variant axis.** They cross phase the same
  way, and the same "one at a time, chosen at export" logic would apply. Still no
  varying sample data — all 26 House A doors sit in `{"option_name": "-",
  "set_name": "Main Model"}` — so there is nothing to design against yet.

- **Type-property deduplication.** `type_properties` rides per instance today; a
  shared type table is a payload-size optimization to take **when measured, not
  before**. The figure to start from: the House A doors snapshot is 414 KB for 26
  doors, and `type_id` is already on the wire ready to key a shared table.

  FF&E is what makes this worth measuring rather than deferring — hundreds of
  instances per model against tens of openings. It is also where the shape of
  the answer would show: an item's footprint is close to a *type* fact, so a
  type table could carry a local-frame box and let every instance keep only its
  placement. Close to, not exactly, because flexed instances of one type differ
  — which is why FF&E ships with the footprint flattened per instance instead.

- **Verifying the doors, windows and FF&E extractors against a live Revit
  document.** All three are verified by running their real translation over a
  captured export -- FF&E most thoroughly, since all 644 House A items were
  round-tripped and its millimetre-to-feet conversion checked against Revit's own
  reading on every one -- but the `get_FromRoom` / `get_Room` accessors and the
  category collectors need a live document. The item footprint joins them:
  duHast populates it now, and no capture taken from Revit has been seen
  carrying one. **This is the only unverified half left.** The room extractor's
  phase filter used to share this standing and was run against a real document
  on 2026-08-03 — which found the failure that had been named as the first
  thing to check. That is the argument for doing the same here, not the
  reassurance that someone else is in the same position.

- **Opening labels on the plan**, and the `door_label` setting they need. Both
  entities draw a glyph now; neither draws a label. Ordinary unbuilt viewer work,
  blocked on nothing — see [Browser](STRATEGY-BROWSER.md)
  for why it is a cost to take deliberately rather than a gap to close.

- **The FF&E level heuristic, unverified against intent.** duHast derives an
  item's level from the bottom of its solid geometry rather than reading the
  instance's own `Level`, so a ceiling-mounted or recessed item can be assigned
  to the storey below the one it serves — and unlike an unresolved level, that
  answer looks correct. Measured at 53 disagreements on House A, and seen in the
  wild: a dining chair whose `Level` parameter says LEVEL 01 draws on LEVEL 00.
  It matters little for attribution (the authored room carries that) and a lot
  for the viewer, which is where a reader sees it. The fix, if the rate turns
  out to matter, is to prefer the instance's parameter and keep the derived
  value as the fallback — the precedence authored data has over geometry
  everywhere else here.

- **FF&E at scale, unmeasured.** Everything known about this entity comes from
  one house: 647 instances across nine categories. The open questions are what
  `OST_GenericModel` pulls in on a real project, what the nested-component rule
  should be when the parent is in a *different* category (87 of 179 on House A
  were furniture inside casework), and whether the payload wants the
  type-property deduplication this document defers below. `scripts/probe_ffe_export.py`
  is the instrument and RHH is the model; the analyser refuses to interpret a run
  whose category list has drifted from duHast's, so the two must move together.

- **Windows in milestone comparison.** `Milestone::window_attachments` landed
  with storage, so a milestone can pin them; `ComparisonResponse.windows` was cut
  from the first pass and has no stated demand. Nothing is half-built — the pins
  simply have no consumer yet.

- **The probe's curtain-wall symbol test disagrees with its host-based
  sibling** — 0 against 51 doors on the same document — and should not be relied
  on until diagnosed. The host test (`Wall.CurtainGrid` is not None) is the
  trustworthy one. It matters because duHast discriminates curtain-wall doors
  and has no window equivalent, so this is the only instrument for that
  question.

- **Geometric verification of a space against its matched room** — the v2 the
  spaces plan deferred with a reason rather than a shrug. It wants a
  point-in-room or boundary-overlap check between the two, and it is blocked on
  the same thing `RoomResolution::Project` is: nothing has ever checked
  `model_to_shared` against a real survey, and two models that never had shared
  coordinates set up emit identity transforms and stack **silently**. Until
  then the viewer's spaces outline layer is the check — a human sees the drift
  in a second, which is what that layer was for. When it is built it takes
  `room_resolution`'s shape exactly: opt-in, reporting disagreement, never
  overriding the key match.

- **Composite match keys for spaces.** One property is the key today. A
  composite is deferred with a reason rather than a hope: the single key is
  unique project-wide on the models measured, so a composite would be machinery
  answering a question nothing has asked. `SpaceReport`'s ambiguous-key
  reporting is the instrument that would say otherwise — if duplicates ever
  appear, that guarantee broke upstream, and *that* is the signal to build
  this, not a preference for stronger keys.

- **Ceilings, as a sixth entity — gated on a probe.** duHast exports them
  (`Revit.Ceilings.Export.to_data_ceiling.get_all_ceiling_data`) in the shape
  `translate_room` already eats: polygon groups, an *authored* level via
  `CEILING_HEIGHTABOVELEVEL_PARAM`, phasing, both property tiers. It also ships
  the join itself, in `Data.process_ceilings_to_rooms` — which **cannot run in
  the extractor**, because it needs shapely and numpy and pyRevit is IronPython
  2.7. So the join is server-side, at read time, on `geo::BooleanOps`, which is
  where door ownership already lives and for the same reason: changing the
  policy changes every answer and rewrites nothing.

  **What makes this entity different is that geometry is not the fallback, it
  is the only answer.** Doors, windows and FF&E each carry an authored room
  reference, and `room_resolution` is opt-in *because* there is something
  authored to disagree with. A Revit ceiling has no room parameter and a room
  has no ceiling parameter, so there is nothing to prefer geometry over and
  nothing for it to overwrite. That inverts the precedence rule this codebase
  applies everywhere else, and it is a rule of ceilings' own, in the way spaces
  have three.

  Attribution is also **many-to-many and area-weighted**, not a single owner:
  one ceiling can span rooms, one room can hold several. `room_locator` does
  not serve — a ceiling is not at a point — so the geometry is genuinely new,
  which is what "geometry semantics are per-entity work every time" predicts.

  **Probed on House A, 2026-09-10** (`scripts/probe_ceilings_export.py`, read
  by `scripts/analyse_ceilings_probe.py`). Both kill conditions pass, and the
  geometry questions are settled enough to write a contract against:

  - **Q1 — 30 of 30 ceilings exported.** No drops. But House A holds **no
    in-place ceilings at all**, so the failure mode the question was written
    for is untested here rather than absent, and the duHast fix for it is
    unverified against a document. What the run *did* find is a different
    thing the question did not ask: **2 of 30 exported a footprint of 0.33 and
    0.18 sqft** against a population whose next smallest is two orders of
    magnitude larger. Exported is not the same as usable, and those two are
    where two of the four marginal room overlaps came from.
  - **Q2 — co-located.** Ceilings and rooms are both in the architectural
    model, so the model-scoped join survives and ceilings need no exception.
    **House A is one model, so this establishes very little**; RHH is the test
    that matters and has not been run.
  - **Q6 — take `polygon[0]`, and never union or sum.** 7 of 30 ceilings
    export more than one polygon, and the count is misleading:
    `convert_solid_to_flattened_2d_points` walks *horizontal faces*, and a slab
    has two of them. 4 of the 7 are the **same face twice** — IoU above 0.98
    between the two largest pieces, sum exactly twice the union — and the other
    3 are the largest face plus sub-1-sqft noise off the side faces. The
    largest polygon equalled the union of all of them on **every** ceiling
    measured. So reusing the room `loops` shape verbatim is correct, and
    aggregating the polygons would double-count area on 4 ceilings.
  - **Q4 — many-to-many is NOT demonstrated, and the threshold is why.** One
    ceiling spans three rooms at duHast's 0.1% and nothing spans anything at
    0.5%, because the two extra matches are 0.97 sqft slivers. duHast's
    threshold is too low for this geometry, and being a percentage of the
    *room* it scales with the wrong operand — a sliver against a large room
    passes more easily than a real overlap against a small one. Keep the
    list-shaped answer, but keep it because a stored single owner would need a
    migration to undo, **not** because this model proved it necessary.
  - **Q5 — no ceiling on House A is demolished**, so the range test and the
    equality test happen to agree. Use the **doors range test** on
    `PHASE_CREATED` / `PHASE_DEMOLISHED` anyway: a ceiling can be demolished,
    and "it worked on this model" is how the five empty rooms pushes were made.

  Two calibrations for the QA report, before it is written. **12 of 32 rooms
  have no ceiling and almost all are external** — POOL, DECK, DRIVEWAY, FRONT
  YARD, the `EX` suffix throughout — so "room without a ceiling" is not a
  finding on its own and a report that flags it will be mostly noise. And **6
  ceilings match no room**, all of one type (`CLFS-550`), 4 of them on LEVEL 02
  which carries no rooms at all. A type-level pattern, not six anomalies.

- **Multi-phase comparison — explicitly out of scope**, recorded so it is not
  re-proposed. It is a second axis crossing the snapshot axis, and milestones
  already answer "the model as it was on date X" without it. RoomMate supports
  exactly one phase per push.

## One rule from the door work that outlives it

**The extractor reads from Revit what the export does not contain, and does not
re-measure what it does.**

It cost two failed attempts to learn. Room references and facing direction are
genuinely absent from the duHast export, so the extractor reads them from the
Revit API — correct. A *footprint* is in the export; it was simply measured in
the wrong frame, and re-measuring it extractor-side produced a worse answer than
the one it replaced. Worse, an extractor that computes its own footprint silently
discards whatever the export sends, so fixing the upstream library changed
nothing until the extractor stopped competing with it.

The generalization for the next entity: "where the upstream answer is lossy, ask
Revit" is a good instinct that is wrong whenever the upstream answer is *present
but wrong*, because then you are not filling a gap — you are choosing between two
implementations, and the one with the whole document in scope usually wins.
