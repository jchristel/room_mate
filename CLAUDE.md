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
- **`room_reference_property` reconciles the modeller against the geometry**,
  and finds real disagreements: 4 of the 26 House A doors, mostly where the
  geometry picks an exterior or circulation space over the served room. Absent
  means the check is **off**, not clean — the QA response says which.

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

**Touched `src-js/`?** Then also:

```
npm run typecheck && npm test && npm run build
```

`npm run build` is not optional. `static/vendor/renderer.bundle.js` is a
**generated file that is committed** (so a fresh clone plus `cargo run` works
with no node installed), and `.github/workflows/frontend.yml` rebuilds it and
fails if the committed copy disagrees. Forgetting it means a red PR, or worse a
green one serving a stale renderer.

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
- **`has_room_snapshot` reads the latest snapshot, not the index.** It used to
  ask whether a rooms snapshot *file* existed, which an empty one does — so it
  waved through 26 doors referencing 22 room ids, none resolvable. Reading costs
  one file per doors push and is the only honest answer while empty snapshots
  from before the fix are still on disk.
- **The extractor refuses empty pushes too, and for doors it is deliberately
  stricter than the server** (`post_rooms.empty_push_refusal`, 2026-08-06). The
  server must accept zero doors — it cannot tell a shell from a broken export.
  The producer is answering a different question ("someone asked for a doors
  push and there are none"), and it knows what the server never sees: how many
  the export held and where each one went, so its message names the phase filter
  instead of reporting a bare zero. The accepted cost: **a model with genuinely
  no doors can no longer record that fact through this producer.** The refusal
  rides the normal `(ok, status, text)` tuple with `status = None`, so callers
  need no second failure channel.

## Traps

- **Line endings are LF**, enforced by `.gitattributes`. Writing files through a
  Python heredoc on Windows silently converts them to CRLF — check with
  `git diff --stat` (it warns) after any scripted file write.
- **Contract is v6 and `phase` is required.** A hand-rolled test push without it
  gets a 422 naming a stale extractor.

## Open, as of 2026-08-05

**Nothing.** Both long-standing items closed: the extractor's phase filter is
verified against Revit, and R4 landed.

## The extractor has three entry points, and one of them is a trap

`rooms_export_entry` still pushes **rooms and then doors**, despite the name.
Its pyRevit button lives outside this repo, so narrowing it to rooms would not
fail — it would keep succeeding while quietly no longer pushing doors. The split
is in the two siblings instead: `rooms_only_export_entry` and
`doors_export_entry`. All three are one line over `export_entry(..., entities)`;
document selection, the one project and the one phase never differ.

A doors-only push does **not** check that rooms are on the server first. That is
the server's question and it already answers it; a client-side re-check would
mean this script deciding what counts as "has rooms", which is what
`has_room_snapshot` got wrong.

## Phase filtering: rooms and doors are not alike

Verified against Revit 2026-08-03, and it cost five empty pushes to find.
**A room BELONGS to one phase** (`ROOM_PHASE`); a door is built in one and may
be demolished in a later one. So rooms use an equality test
(`room_mate.rooms_in_phase`) and doors use the range test
(`elements_in_phase` / `exists_in_phase`). Running rooms through the range test
returns *nothing* — silently. Both paths raise on an unknown phase name, and
that guard is the only thing standing between a typo and another five empty
snapshots.
