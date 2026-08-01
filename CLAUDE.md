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
- **A room id is unique only within a model, and a door's `from_room`/`to_room`
  are room ids.** So the door→room join is model-scoped *everywhere*: ingest
  refuses doors to a model with no rooms, QA resolves references per model, and
  every `/doors` row carries `model_id`. A project-scoped shortcut anywhere here
  turns a dangling reference into a false clean bill.
- **Doors never re-phase a lineage.** A rooms push that disagrees on phase is
  quarantined and promotable; a doors push is **refused**. Promoting it would
  move the lineage while the rooms stayed behind.
- **R4 (entity-scope `[sources.reference.*]`) is still open**, and lands with
  doors' first reference source — not before, not later (back-compat
  obligation). Doors shipped with none, so it stays open.
- **Door ownership is decided but not built**: a door belongs to the room it
  opens *into*, else the room it opens *from*, else it is **homeless** — a
  precedence chain, not one of the four single picks the docs first offered.
  Nothing built assumes it yet, which is why `[doors]` has no
  `room_attribution` and `/doors` no `?building=`. Two things to keep in mind
  when building it: Revit's `to_room` follows the door's *orientation*, not the
  leaf swing (flipping swaps it), so this is project policy with an override,
  never a hard-coded rule; and the authored `Door Room Reference` disagrees with
  the chain on 4 of the 26 House A doors — mostly where the chain picks an
  exterior or circulation space over the served room.

## Traps in the door export

- **`±1e30` is not geometry.** duHast returns Revit's *uninitialized*
  `BoundingBoxXYZ` for a door family with no 3D geometry, and its own guards
  pass, so it arrives looking plausible. The producer drops it and sends empty
  `loops`; the door is still pushed, because it has real room references.
- **Never read `from_room`/`to_room` from the export.** They are per-phase
  arrays tagged with a `phase_id` that resolves against nothing on the wire. The
  extractor reads `FamilyInstance.FromRoom[phase]` from the Revit API instead.

## Which document wins

[`docs/README.md`](docs/README.md) indexes everything. Two supersessions matter:

- **Phase:** [`PLAN-phasing.md`](docs/PLAN-phasing.md) is authoritative over
  `STRATEGY-ENTITIES.md` Decision 2.
- **Generalisation:** [`PLAN-generalisation.md`](docs/PLAN-generalisation.md)
  supplies the signatures `STRATEGY-ENTITIES.md` Decisions 3 and 5 only assert.

Docs pin `file.rs:NNN` line numbers that have drifted — trust the *symbol* name,
search for it, never jump to the line.

## Verify before claiming done

```
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

All three are CI gates; clippy runs with `-D warnings`, so a warning is a
failure. Frontend changes are verified by driving the page, not by reading the
diff — a bug shipped this week was only visible after expanding a panel.

## House rules the code won't tell you

- **Tests are inline** — `#[cfg(test)] mod tests` at the bottom of the file they
  exercise, never a `tests/` tree. A shared helper is duplicated per module
  rather than hoisted.
- **Doc comments carry the *rationale*** — why this seam, what breaks otherwise,
  what was rejected. Not a restatement of the what. This is the single most
  visible house style; matching ordinary Rust terseness reads as foreign.
- **`service/` is transport-agnostic** — never imports `axum` or `rmcp`.
  `handlers.rs` (HTTP) and `bin/mcp.rs` (MCP) are thin adapters over it, and
  `bin/mcp.rs` keeps one tool per HTTP *read* route (update its count when you
  add one).
- **"Signal, not error"** — an unresolved cross-reference is usually a reported
  state, not a failure.

## Traps

- **Line endings are LF**, enforced by `.gitattributes`. Writing files through a
  Python heredoc on Windows silently converts them to CRLF — check with
  `git diff --stat` (it warns) after any scripted file write.
- **Contract is v6 and `phase` is required.** A hand-rolled test push without it
  gets a 422 naming a stale extractor.

## Open, as of 2026-08-01

- **The pyRevit extractor's phase filter is unverified against Revit.** Check
  first that `FilteredElementCollector`'s element ids match the ids duHast
  writes into the export — if they don't, the filter silently keeps nothing.
  See [`PLAN-phasing.md`](docs/PLAN-phasing.md) "As built".
