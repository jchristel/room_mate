# RoomMate — Entities

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[Area calculation](STRATEGY-AREA-CALCULATION.md) ·
[Browser](STRATEGY-BROWSER.md) · [MCP](STRATEGY-MCP.md) ·
[Authored](STRATEGY-AUTHORED.md) · [Security](STRATEGY-SECURITY.md)

**Open work only.** Rooms, doors, windows and FF&E all ship — contract, ingest,
storage, read, QA, MCP, the plan and the pyRevit exporter — and phasing ships
under them. What each of those does, and the invariants that are expensive to
rediscover (tier precedence, opening ownership, item attribution, the
model-scoped element→room join, the phase rules), live in the code and in
`CLAUDE.md`.

What is left here is the **entity dimension**: the test that decides whether the
next candidate is an entity at all, what four entities proved comes for free,
and what is still unbuilt.

**The bet below has been tested twice, and the second test was the one worth
having.** Windows were the third entity and cost nothing structural — they reuse
the `Opening` record, so they reused the whole opening stack by construction.
FFE was the first candidate that is *not* an opening, and it is the reason three
things in this codebase are now named for entities rather than for openings:
`contract::SnapshotEnvelope` (the six facts every stored snapshot answers),
`service::entity_scope` (the scoping, pinning and geometry every read shares),
and the extractor's `post_entity` (the transport every push shares). Each was
opening-shaped only because no non-opening had ever asked.

**What did NOT need widening is the more useful half of that result**: the phase
envelope, snapshot-id resolution, the bytes-at-the-boundary store, the manifest,
the property tiers, the filter grammar, the reference-join namespace and the
`ENTITY_EXPORTERS` table all took a fourth entity without changing shape.

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

Every entity so far has needed it, FFE included: an item's `room` is a room id,
so the join is model-scoped in the read, in QA and in the extractor's per-model
`facts` map.

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
  category collectors need a live document. **This is the only unverified half
  left.** The room extractor's phase filter used to share this
  standing and was run against a real document on 2026-08-03 — which found the
  failure that had been named as the first thing to check. That is the argument
  for doing the same here, not the reassurance that someone else is in the same
  position.

- **Opening labels on the plan**, and the `door_label` setting they need. Both
  entities draw a glyph now; neither draws a label. Ordinary unbuilt viewer work,
  blocked on nothing — see [Browser](STRATEGY-BROWSER.md)
  for why it is a cost to take deliberately rather than a gap to close.

- **The two duHast changes FF&E is waiting on**, neither of them this
  repository's to make. **U1**: `DataItem` has no geometry, so every item is
  drawn as a marker rather than a footprint. The viewer was built around that
  and draws real footprints the day they arrive with no change, so this is a
  fidelity gap rather than a blocker. **U2**: the rotation-preserving bounding
  box does not walk `GetSubComponentIds()` while its axis-aligned sibling does —
  measured as 4 of 184 box disagreements, so a correctness tidy-up rather than
  the main event it was predicted to be.

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

- **The pyRevit buttons for `windows_export_entry` and `ffe_export_entry`.**
  Both extractor entry points exist; their buttons live outside this repository
  and have to be wired there before anyone can push either from Revit. This is
  the one cost adding an entity does not absorb, and it is now owed twice.
  `rooms_export_entry` was deliberately not widened — it still pushes rooms AND
  doors despite its name, and adding another entity to it would keep succeeding
  while changing what every existing button does.

  A combined rooms-and-FF&E entry would be genuinely useful, since the two live
  in one document and one run could read it once. It is one line of
  `export_entry(..., (ROOMS, FFE))` and deliberately unwritten: adding it before
  anyone asks would make a third unwired button rather than a saving.

- **Windows in milestone comparison.** `MilestonePins.windows` landed with
  storage, so a milestone can pin them; `ComparisonResponse.windows` was cut
  from the first pass and has no stated demand. Nothing is half-built — the pins
  simply have no consumer yet.

- **The probe's curtain-wall symbol test disagrees with its host-based
  sibling** — 0 against 51 doors on the same document — and should not be relied
  on until diagnosed. The host test (`Wall.CurtainGrid` is not None) is the
  trustworthy one. It matters because duHast discriminates curtain-wall doors
  and has no window equivalent, so this is the only instrument for that
  question.

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
