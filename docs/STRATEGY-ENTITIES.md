# RoomMate — Entities

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[Area calculation](STRATEGY-AREA-CALCULATION.md) ·
[Browser](STRATEGY-BROWSER.md) · [MCP](STRATEGY-MCP.md) ·
[Authored](STRATEGY-AUTHORED.md) · [Security](STRATEGY-SECURITY.md)

**Open work only.** Rooms and doors both ship — contract, ingest, storage, read,
QA, milestone comparison and the pyRevit exporter — and phasing ships under them.
What each of those does, and the invariants that are expensive to rediscover
(tier precedence, door ownership, the model-scoped door→room join, the phase
rules), live in the code and in `CLAUDE.md`.

What is left here is the **entity dimension**: the test that decides whether the
next candidate is an entity at all, what a second entity proved comes for free,
and what is still unbuilt. FFE is the next candidate, which is what this document
exists to serve.

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

FFE that hangs off rooms will need it. FFE that stands alone will not.

**It used to need two, and losing the second is worth recording.** There was an
*ingest gate*: a doors push was refused unless the target `(project, model)`
lineage already had a live rooms snapshot. It is gone. The gate asked "can these
references resolve **now**?", which has a legitimate answer of "not yet" — rooms
may arrive in a later push — so refusing meant refusing data that becomes
resolvable the moment they do. It was also the only place in the server where an
unresolved cross-reference was an *error* rather than a reported state, against
the "signal, not error" rule listed above.

The check did not disappear; it moved to `service::validation::door_report`,
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

- **Verifying the doors extractor against a live Revit document.** It is verified
  by running its real translation over a captured export, but the `get_FromRoom`
  accessor and the `OST_Doors` collector need a live document. **This is the only
  unverified half left.** The room extractor's phase filter used to share this
  standing and was run against a real document on 2026-08-03 — which found the
  failure that had been named as the first thing to check. That is the argument
  for doing the same here, not the reassurance that someone else is in the same
  position.

- **Door labels on the plan**, and the `door_label` setting they need. Ordinary
  unbuilt viewer work, blocked on nothing — see [Browser](STRATEGY-BROWSER.md)
  for why it is a cost to take deliberately rather than a gap to close.

- **FFE, the next primary entity.** **The bet this document made is now
  testable:** if FFE needs anything beyond the phase envelope, the
  bytes-at-the-boundary store and a `PropertyTiers` impl, one of the
  generalizations above was too narrow. Doors needed exactly the two dependent-
  entity additions listed above and nothing else.

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
