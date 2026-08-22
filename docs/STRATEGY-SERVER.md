# Roommate — Server

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Browser](STRATEGY-BROWSER.md) ·
[MCP](STRATEGY-MCP.md) · [Authored](STRATEGY-AUTHORED.md) ·
[Entities](STRATEGY-ENTITIES.md) · [Security](STRATEGY-SECURITY.md)

**This document holds the Rust/axum server's *open* work — what is not built.**
What the server already does is documented where it is built: every module under
`src/` carries its rationale in a header doc comment, the house rules live in
[Coding Conventions](CODING-CONVENTIONS.md), and the invariants that are
expensive to rediscover live in `CLAUDE.md`. A description restated here would
be a second copy, free to drift from the first — which is what this document had
become.

## Deferred — design settled, not built

- **Per-model read endpoints** (`GET /projects/{p}/models/{m}` and siblings).
  `/rooms` merging every stored model into one flat payload is a stopgap: it
  flattens stored identity, and a raw Revit room id is unique only *within* its
  model. Doors made that load-bearing rather than cosmetic — a door's
  `from_room`/`to_room` are room ids, which is why `/doors` carries `model_id`,
  QA resolves references per model, and a doors read carries its own model
  identity. The endpoint is what removes the flattening rather than working around
  it. The store's flat `(project, model)` key is the same deferral seen from
  below: nesting only earns its place once endpoints address projects and models
  as separate resources. Both are additive to fix, not migrations.

- **`GET /hierarchy`.** The classification hierarchy is resolved per room and
  never exposed as a tree of its own. The one tier that grew a consumer got a
  purpose-built endpoint instead (`/projects/{id}/buildings`), which is the
  evidence that a general endpoint should wait for a second such consumer rather
  than be guessed at.

- **Snapshot delete UI.** History is kept forever, one file per push. Pruning is a
  UI concern (select-and-delete), deliberately not an ingest-time decision. The
  *query* half is built; nothing removes.

- **A database `SnapshotStore`.** A third impl behind the existing trait, which
  is the only reason the trait exists. Nothing about it is designed yet beyond
  that seam.

- **`SnapshotStore::put_streaming`.** `POST /rooms/stream` streams *parsing*
  only — rooms are still accumulated in memory, and `set_snapshot` then
  serializes the whole payload again, so peak is roughly the payload twice and
  streaming does not help when the room set itself is too large. Writing rooms to
  disk as they arrive is the next step. One snapshot file stays the unit; what
  changes is that it is written line by line and committed at the end, so it must
  write to a temp path and rename — the store re-parses every snapshot at boot,
  and a truncated file from an aborted push must never read as a complete one.

  **The multi-model push raised the ceiling this is measured against**: one
  request now holds a whole run's models rather than one at a time, and the
  writer will need to handle N open snapshots (or group and write them
  sequentially). (The limitation is also recorded at the handler.)

- **A milestone-aware validation report.** `GET /projects/{id}/validation` is
  latest-based regardless of `?milestone=`: it resolves each source's link
  values itself rather than through the rooms path, so the milestone
  substitution that reaches every other read never reaches it. The deliberate v1
  limit shipped with milestones, and the open question below is the same
  question from the other side.

- **A settings-relative `static/` path.** Every relative path in a settings file
  resolves against that file's own directory; `static/`, served by
  `ServeDir::new("static")`, is the one exception and is still resolved against
  the process's working directory. So the viewer page needs the exe launched
  from the crate root, or `static/` copied alongside it.

## Should validation reuse the rooms join?

An open question, not a plan — and a correction to what this document used to
say. There is no dRofus-specific link anywhere in the server: reference sources
are generic and configured N at a time, and `compute_validation` runs once per
source.

`service::validation::compute_validation` resolves each room's link value itself
(`resolve_link_values`) rather than going through
`service::rooms::assemble_room`'s join. That is deliberate, not an oversight:
validation's duplicate-link-value detection and its absent-vs-empty distinction
are structurally different from "assemble one room for display", so routing
through `assemble_room` today would mean either losing that distinction or
paying for the classification and label work validation never uses — a
speculative abstraction with no current payoff.

`assemble_room` therefore stays private to `rooms.rs` until a second consumer
actually exists and its real needs are known (F&E validation was the anticipated
one). Widening it to `pub(crate)` is the cheap first move when that happens, and
the decision of whether validation fits belongs at that point, not now.

## An owning level above project

The committed hierarchy is **project → model → snapshot → {levels, rooms}**.
Nothing sits above project, and cross-project operations are not the argument
for adding something.

Comparing or moving data *between* projects does **not** require a container
above project. Those are *operations across peers*; modelling the verb (compare,
move) as a noun (a new level) is the wrong instinct. A container is justified
only when things share a lifecycle or ownership, and "compare A to B" implies
neither. What such operations actually need is a stable addressable identity per
project — already provided by the project id, so comparison and move are
functions over two ids — plus a shared coordinate frame, which is a geometry
problem no amount of nesting solves (see below).

**When a top level *is* justified:** a real owning entity emerges — a portfolio,
organization, or client that groups many projects, controls access, or is the
unit queried at ("all rooms across the hospital network"). That is a genuine
container with its own identity and metadata, driven by *organizational* need
(multi-tenancy, access control, rollups), not by compare/move. Absent that need,
the level is dead weight. The committed structure blocks neither path:
cross-project operations can be added without a new level, and an owning level
can be added above project later without disturbing anything below it —
additive, like snapshot history.

## A shared coordinate datum across projects

The subtlety behind any cross-project geometry, and the reason "same structure ⇒
comparable" is false. Each project's rooms sit in their own Revit model space,
with their own origin and rotation. Comparing footprints or moving a room across
projects is meaningless until the two share a datum — a shared survey point, or
an explicit alignment transform between them.

**The first half exists.** A model may carry a `model_to_shared` transform on
its upload envelope (see [Index](STRATEGY.md), "The upload envelope") that maps
its room points into the project's *shared* coordinate system, so the rooms of
one project's linked models land in one frame. It was shipped deliberately ahead
of any consumer, and nothing numeric depends on it being present or correct.

What is **still missing** is a frame shared *across* projects. Two
survey-registered projects in the same CRS become directly comparable; the
general case still needs an explicit alignment, and neither the representation
of that alignment nor where it would be authored has been designed.
