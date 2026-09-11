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
  is **one element per line** and the buffered path still writes `to_vec_pretty`;
  they differ in whitespace and key order only, and nothing re-reads a snapshot
  by shape.
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

## Ceilings: the entity where geometry is the only answer

Ceilings ship server-side and in the extractor (contract, ingest, storage,
`/ceilings`, MCP, exporter). No viewer layer and no QA report yet.

- **A ceiling has no room parameter and a room has no ceiling parameter.** So
  the join is geometric or it does not exist. That inverts the precedence rule
  every other entity follows — doors, windows and FF&E each carry an authored
  reference and `room_resolution` is opt-in *because* there is something
  authored to disagree with. `service::ceilings` therefore passes
  `RoomResolution::SameModel` unconditionally and never reads the project's
  setting: reading it would let a project switch the entity off and report every
  ceiling as homeless, which is a wrong answer dressed as a disabled feature.
- **Attribution is many-to-many, area-weighted, and derived at read time.**
  `rooms` on a `/ceilings` row is a list ordered largest-overlap-first, so
  `rooms[0]` is a usable single owner. **Empty means unattributed, which is a
  reported state** — 8 of House A's 30, being external soffits, a level with no
  rooms, and two degenerate exports. Nothing is stored, so changing the rule
  changes every answer and rewrites nothing.
- **Two thresholds, and one cannot do both jobs** (`MIN_OVERLAP_AREA` 1.0 sqft,
  `MIN_FRACTION_OF_CEILING` 0.005). Measured: 26 intersecting pairs on House A,
  22 genuine and 4 not, separated by an **18x gap**. The 4 are two different
  faults — slivers where a large ceiling grazes a neighbour (0.97 sqft, 0.414%
  of itself) need the fraction test; ceilings whose whole exported footprint is
  0.33 and 0.18 sqft overlap by 100% *of themselves* and only the absolute test
  rejects them. **The fraction is of the CEILING**, which is where duHast's own
  `_intersect_ceiling_vs_room` is wrong: it divides by the ROOM's area despite
  naming its variable for the ceiling, so a sliver against a large room passes
  more easily than a real overlap against a small one.
- **`loops[0]` is the whole ceiling; never union or sum the polygons.** duHast
  exports one polygon per *horizontal face*, and a slab has two — 4 of House A's
  7 multi-polygon ceilings are the same face twice (IoU above 0.98, areas
  summing to exactly twice their union) and the other 3 are the largest face
  plus sub-1-sqft edge noise. The largest polygon equalled the union of all of
  them on every ceiling measured. A polygon count above 1 therefore reads as
  evidence the `loops` field is too narrow and **is not**; the signal to widen
  it is `analyse_ceilings_probe.py` reporting "carries genuinely ADDITIONAL
  geometry" under Q6, which House A does not.
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
  the contract wants decimal feet, so `utils/ceilings.ceiling_offsets` reads the
  parameter itself. That is not the re-derivation this file forbids — there is
  no measurement to disagree about, only a rendering not to parse. The *level*
  is duHast's and must stay so; that one is the FF&E trap.
- **`/ceilings`' ETag cursor covers ROOMS as well as ceilings**, which no other
  entity read needs. Attribution derives from the rooms in scope, so a rooms
  push alone changes every answer and a ceilings-only cursor would serve a stale
  304.

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
green one serving a stale renderer. The same build, and the same gate, cover
`static/settings-react/` — the React preview of the settings page.

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

- **`/ffe` is 273 MB and 94 s on RHH** (38,913 items, each with its full
  instance *and* type property map). The viewer's poll loop is sequential, so
  this blocks first paint for ~2 minutes. Measured, not fixed — the shape of the
  fix (drop the per-item type-property repetition, or scope the read) is not
  decided.
- **RHH has no windows snapshot at all.** Not a code gap: `windows_export_entry`
  has simply never been run against it, so `/windows?project=RHH` answers 200
  with an empty list. The level-id fix above is what windows needed to be
  visible once pushed, since they live in the facade model that has no rooms.
- **RHH's FF&E is on the wrong storeys until it is re-exported** — see the FF&E
  level trap above. The fix is upstream and lands on the next export.

Both older items stay closed: the extractor's phase filter is verified against
Revit, and R4 landed.

## The extractor has seven entry points, one of them a trap and one unwired

`rooms_export_entry` still pushes **rooms and doors**, despite the name. Its
pyRevit button lives outside this repo, so narrowing it to rooms would not fail —
it would keep succeeding while quietly no longer pushing doors. The split is in
the siblings instead: `rooms_only_export_entry`, `doors_export_entry`,
`windows_export_entry`, `ffe_export_entry`, `spaces_export_entry` and
`ceilings_export_entry`. All seven are one line over
`export_entry(..., entities)`; document selection, the one project and the one
phase never differ.

**Their buttons live outside this repository**, in
`SampleCodeRevitBatchProcessor-NET8/.../duHast.tab/RoomMate.panel`, over a COPY
of `extractor/pyRevit/room_m` under that tab's `lib/`. Six are wired as of
2026-09-06; **`ceilings_export_entry` has no button yet** (2026-09-10), so it is
reachable from code and not from the ribbon. **The copy is the trap**: an
extractor change here is inert until it is copied there, and nothing checks the
two are in step — `diff -rq` between them is the only check there is.

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
