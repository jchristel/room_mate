# RoomMate — Entities & Phasing Strategy

**Status: built.** Doors ship — contract, ingest, storage, read, QA, milestone
comparison and the pyRevit exporter. Phasing shipped ahead of them. This
document records the shape agreed for the pipeline's *second* primary entity and
for the phase selection every primary entity after rooms needs; the sections
below now say, per decision, what was built and where reality departed from the
sketch.

> ## What survived contact, in one place
>
> **The prerequisites were met.** R1 and R2 landed as their own change *before*
> any door code, so the `Door` contract was written against a settled precedence
> rule rather than racing it — see
> [PLAN-generalisation.md](Superseded/PLAN-generalisation.md), which carries an outcome
> note per item. No `put_doors` was ever written beside `put`.
>
> **R2's open question did not resolve either way this document expected**, and
> the answer came from the sample export rather than from first principles. The
> rule is: **a tier wins only when it is `Present`** — walk instance then type,
> return the first *non-empty* value. Plain shadowing hides `Door Leaf Thickness
> = 40.0` behind a blank instance parameter on 22 of the 26 sample doors;
> treating a name in both tiers as a finding fires on 26 of 26, because Revit
> carries `Workset` and `Edited by` on instances and types alike. Decision 4's
> separation is preserved by the two maps staying two maps on the wire, not by
> making an overlap an error.
>
> **R4 is the one prerequisite doors deliberately shipped without**, and that was
> the plan: it lands with doors' *first reference source*, and doors shipped with
> none. Until then `[sources.reference.*]` still means "for rooms" —
> [Sources](STRATEGY-SOURCES.md) records the gap.
>
> **Three things this document asserts about the export are now stale** and are
> corrected in place below: the door id exists, the room id space is verified,
> and the phase ids are moot. See
> [Blockers in the current export](#blockers-in-the-current-export).

> **Decision 2 (phasing) is built, and has been rewritten here to match.** It
> shipped ahead of doors and several details changed on contact with the code;
> rather than leave the original sketch standing behind a warning, 2a and 2c now
> describe what exists and say what they replaced. **[PLAN-phasing.md](Superseded/PLAN-phasing.md)
> carries the full rationale** and is authoritative if the two ever drift. The
> The same treatment has since been applied to Decisions 3 through 6, each of
> which now carries what shipped and where it departed from the sketch.

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[Area calculation](STRATEGY-AREA-CALCULATION.md) ·
[Browser](STRATEGY-BROWSER.md) · [MCP](STRATEGY-MCP.md) ·
[Authored](STRATEGY-AUTHORED.md) · [Security](STRATEGY-SECURITY.md)

This doc exists for the same reason [Area
calculation](STRATEGY-AREA-CALCULATION.md) does: it is the one place where the
*definition of the thing* is contested rather than read off the model. The
other docs split along pipeline boundaries (source → server → browser) and a
per-entity retelling of each would fight that split. What genuinely needs its
own home is the **entity dimension** — what makes something a primary entity,
what the rooms/doors pair proves generalizes and what does not, and how phasing
scopes a push. The pipeline-shaped consequences belong in the pipeline-shaped
docs, and are listed under [Where the rest of this
lands](#where-the-rest-of-this-lands).

## The problem this answers

[Sources](STRATEGY-SOURCES.md) already names the boundary this doc crosses:

> `[sources.reference.*]` currently means "reference sources *for rooms*." A
> door schedule needs a *doors entity* first, not just another entry under this
> table.

`docs/Superseded/HANDOVER-reference-sources.md` calls that **axis 1** (the model isn't only
rooms) and distinguishes it from **axis 2** (multiple reference sources for one
entity). Axis 2 has shipped — `[sources.reference.<name>]`, the
`/projects/{id}/reference/{source}` routes, `ProjectReferenceSource`. Axis 1 has
not: `/rooms` assembles rooms and nothing else, so a door schedule has nothing
to attach to.

Doors are the first axis-1 entity. FFE is the next. Getting the generalization
right once is cheaper than getting it wrong twice.

## Decision 1: what makes something a primary entity

A thing is a **primary entity** when it is extracted from the model, carries its
own geometry and identity, and other data joins *onto* it. A thing is a
**reference source** when it arrives from outside the model and joins onto
something else.

Doors qualify on all three counts: they come out of Revit, they have a swing
footprint and an ElementId, and a door hardware schedule joins onto them. A door
*schedule* does not qualify — it is reference data for the door entity.

The practical test, and the one to apply to the next candidate: **does anything
join onto it?** If yes it needs identity, storage, and an endpoint of its own. If
it only joins onto something else, it is a reference source and axis 2 already
covers it.

What generalizes from rooms to every primary entity:

- the upload envelope (`schema_version` / `project` / `model` / `snapshot`), and
  snapshot id resolution through `contract::ensure_taken_at` /
  `validate_snapshot_id` — never a reimplementation
  ([Index](STRATEGY.md), [Conventions](CODING-CONVENTIONS.md));
- `SnapshotStore` and the `project.toml` manifest as the index;
- the source-native flat property map plus settings-driven canonical name
  resolution ([Sources](STRATEGY-SOURCES.md));
- the `<source>.<label>` reference-join namespace and the `?filter=` grammar;
- "signal, not error" for an unresolved cross-reference.

What does **not** generalize, and is per-entity work every time:

- geometry semantics (a room's boundary regime means nothing to a door's swing
  footprint);
- connectivity and ownership (Decision 6);
- which canonical property names exist at all — `Mark` on a door and `Mark` on a
  room are different properties that happen to share a spelling (Decision 5).

## Decision 2: one phase per push, chosen by the user, carried on the envelope

> **Built, ahead of doors.** This section has been rewritten to describe what
> actually shipped rather than what was first sketched;
> [PLAN-phasing.md](Superseded/PLAN-phasing.md) carries the full rationale and the ten
> decisions behind it, and is authoritative where the two ever drift.

Phasing landed on **rooms** first, which reverses the order this document
assumed — it was designed as something doors would introduce. That turned out to
be the right way round: phasing is an envelope concern, and building it against
the entity that already existed meant doors inherit it rather than define it.

RoomMate supports **exactly one phase per push**. Multi-phase comparison is
deliberately out of scope: it is a second axis crossing the snapshot axis, and
milestones already answer "the model as it was on date X" without it.

The phase is a **user choice at export time**, not a document fact. That makes it
the first authored field on the envelope — `model_to_shared` and `room_boundary`
are both read off the document, and someone will eventually try to "fix" this one
by reading it from the doc too. It cannot be: a document has many phases and only
the user knows which one is being pushed.

### 2a. The phase is a name, and only a name

Each document has its own phases with its own ElementIds, so "New Construction"
in the architectural model and in the structural model are *different ids*. This
is the `Level.id` problem the codebase already solved — a level id is unique only
within one model, which is why `service::rooms` phase 2 dedups levels across
linked models. So all cross-model reasoning keys on the **name**.

This document originally proposed carrying `{ id, name }`, with the id kept for
display and debugging. **The id was dropped.** Once the name is the identity, the
id is a field nothing reads — and this doc's own 2b argument applies to it: an
unread field drifts. It was also the least trustworthy value available.
the raw export's (`scripts/fixtures/doors-raw.json`) `from_room[].phase_id` is `3`,
low enough to be an *index* into
`doc.Phases` rather than an ElementId, in which case it is not stable across
models and is useless even for display. Carrying a field that is unread *and*
possibly wrong is the worst of both, so `phase` is a bare string on the wire:
`"phase": "New Construction"`.

Comparison folds **case and surrounding whitespace**. The original sketch made it
case-sensitive, arguing that two phases differing only in case is a modelling
error worth surfacing loudly. That argument was built for a world where a
mismatch *rejected* the push; under 2c a mismatch quarantines it instead, and
quarantining a correct export over letter-case is a bad trade.

### 2b. "Exists in the selected phase" is a range test, not equality

The Revit predicate is:

```
created <= selected  AND  (demolished is invalid  OR  demolished > selected)
```

Filtering on `created == selected` would drop every element built in an earlier
phase and still standing — on a phased model, most of them. This will not show
up in testing against the current sample: all 26 doors in the raw export carry
`demolished: "Invalid phase id."`, so equality and the range test agree there.

The `<=` needs a **phase ordering**, and only the document has one. Two
consequences follow:

- **The extractor does the filtering**, because it is the only side with
  `doc.Phases`. This also aligns with [Sources](STRATEGY-SOURCES.md)' measured
  conclusion that the real optimization axis is *extracting less*, not
  processing faster — a phase filter is strictly less extraction.
- **The ordered phase list is deliberately not carried on the wire.** The server
  never re-evaluates the predicate, so it would be unused data, and an unused
  field is one that drifts. If a future feature needs the ordering it can be
  added then, `Option`-defaulted like everything else.

Per-element provenance comes for free: `Phase Created` and `Phase Demolished`
are already in the generic instance-property dump, so they land in the flat
`properties` map with no contract change and are queryable via `?filter=`.

### 2c. The handshake is advisory; the server enforces

The export flow — pick model(s), resolve the phase, show the user which phase
will be pushed, push only matching elements — is a *client* guarantee, and one
`curl` breaks it. So the server enforces, and its rules turned out sharper than
this section first proposed.

**A lineage's phase is immutable.** It is fixed by the first phased push to a
`(project, model)` and never moves again by ordinary means. This document did not
originally say so; it follows from the deadlock argument below applied
consistently.

**The two failure modes are asymmetric**, which is the part worth not
re-deriving:

- a push naming a **different** phase is *not* refused. It is stored inert under
  `<model>/pending/` and answered **202**, then made live by `POST
  .../snapshots/pending/activate`. That pair is the only way a model re-phases.
  Refusing instead — the original 422 — would leave a model pushed under the
  wrong phase permanently wrong, there being no delete route;
- a push naming **no** phase *is* refused, 422, against any lineage. It means a
  producer predating phase support, whose elements were never filtered by 2b's
  range test at all, so it is unfiltered mixed-phase content. There is nothing
  worth activating, and offering to activate it would be offering to corrupt the
  model.

**Scope the check to the `(project, model)` lineage, not the project.**
Cross-model enforcement deadlocks: to move a project from Phase A to Phase B you
would have to push model 1 (rejected — it disagrees with model 2) and you can
never push either first. Cross-model phase disagreement *within* a project is a
read-time diagnostic — `phases.disagree` on the validation report, with the raw
per-model fact as `phase_by_model` on `/rooms` — not an ingest rejection.
"Signal, not error" ([Conventions](CODING-CONVENTIONS.md)).

**This bumped the contract to v6**, where this section originally predicted no
bump. The no-bump argument was sound *for an optional field*: a payload that was
valid stays valid and means the same. It stops holding the moment the field is
required at ingest, because a previously-valid payload now errors — which is
exactly the test a bump exists for. The field is nonetheless still `Option` on
the Rust type: stored snapshots are re-parsed at every boot, and a hard
requirement there would stop the server hydrating its own store. Strictness lives
in the handler, permissiveness in the type.

### 2d. Where the client asks

`GET /projects/{p}/models/{m}/snapshots/latest` is already described in
[Server](STRATEGY-SERVER.md) as "the *what do I attach this follow-up upload to*
call". Extend its response with the resolved phase rather than adding a sibling
route — one call answers both halves of what a follow-up push needs to know.

### 2e. Prompt UX against multiselect

`pick_document` is multiselect and phases are per-document, so prompting per
model means five dialogs for five models. Instead: prompt **once** with the phase
names common to the selected documents, then resolve each document's own phase id
by name, failing loudly for a document that lacks the chosen name. Skip the
prompt entirely when a document has exactly one phase — which covers most models.

## Decision 3: doors are their own upload type, modelled on rooms

> **Built, with two additions this section did not foresee — both ingest rules,
> both consequences of doors being a *dependent* entity rather than a parallel
> one.**
>
> - **A doors push is refused unless the target `(project, model)` lineage
>   already has a live rooms snapshot.** Scoped to the model, not the project:
>   room ids are unique only within a model, so rooms under a sibling model are
>   the wrong id space and would satisfy a project-wide gate while resolving
>   nothing.
> - **A doors push whose phase disagrees with the lineage is refused, not
>   quarantined.** This section says doors get "the Decision 2c phase check",
>   which is true of the check and wrong about the remedy. Quarantine exists so a
>   model can be re-phased by promotion; promoting a doors push would move the
>   lineage while every rooms snapshot stayed on the old phase, stranding the
>   rooms its references point at. So there is no doors quarantine, no second
>   pending slot, and no 202 on `/doors`.
>
> Also as predicted: the storage layout is `<model>/doors/<ts>.json`, invisible
> to both `.json`-extension model-dir scans, with a `doors` index on
> `ModelEntry`. The `SnapshotStore` pressure point resolved as **bytes at the
> trait boundary** rather than a `kind` parameter over a fixed payload type —
> see [Server](STRATEGY-SERVER.md)'s storage section for why that was forced.
>
> One departure on the read side: **`GET /doors` takes no `?building=`**, unlike
> the "exactly as `/rooms`" this section promises. A door's building depends on
> which of its rooms owns it — Decision 6's open question — and a scope that
> silently picked an answer would settle it by accident.

Endpoints mirror rooms, not reference sources:

- `POST /doors` and `POST /doors/stream` — same body limits, same
  `ensure_taken_at` / `validate_snapshot_id`, same registered-project ingest
  check, plus the Decision 2c phase check.
- `GET /doors` — scoped by `?project=` / `?building=` / `?milestone=` /
  `?filter=` exactly as `/rooms`.

Storage: `<project>/<model>/doors/<taken_at>.json`, mirroring
`reference/<source>/`. This is safe as an additive change because both model-dir
scans in `storage/fs.rs` filter on `extension == "json"`, so a `doors/`
subdirectory is invisible to them. It needs a reserved `DOORS_DIR` constant next
to `REFERENCE_DIR` and a `doors` snapshot index on `ModelEntry`, for the same
reason `ModelEntry::snapshots` exists: listing history must never open the
snapshot files.

**The `SnapshotStore` trait is the pressure point.** It currently carries
`put`/`latest`/`list_snapshot_ids` for rooms plus a parallel `put_drofus` set for
reference sources. A third near-identical set is the signal to generalize on a
`kind` parameter instead of adding `put_doors` — and to do it *while* adding
doors, not after FFE makes it a fourth.

## Decision 4: the door contract shape

> **Built, and every bullet below held.** The types live in
> [`src/contract/doors.rs`](../src/contract/doors.rs) — the split of
> `contract.rs` into `contract/` that CODING-CONVENTIONS' measured-module note
> named the door types as the trigger for. (Rooms deliberately did *not* move out
> alongside them: nothing motivated it, and moving code for symmetry is how a
> split stops being reviewable.)
>
> Three things the shipped type carries that this section does not mention:
> `type_id` and `type_name` — the identity of the shared tier, which a future
> type-property table would key on and a hardware schedule would join per type —
> and **no `levels` array**, because the target model's rooms snapshot already
> carries the level set a door's `level_id` points into, and a second copy could
> only disagree.
>
> The instance-vs-type *lookup* rule, which this section rightly refuses to
> settle here, is R2's and is stated in the summary at the top of this document.

Full annotated types in [`src/contract/doors.rs`](../src/contract/doors.rs). The
decisions behind them:

- **A door is not a `Room` with different fields.** It carries a *second*
  property tier — its family type, shared across every instance of that type.
  Flattening the two would lose the distinction between "this leaf is 820 wide"
  and "every door of this type is 820 wide", which is exactly what a hardware
  schedule joins against. So `properties` **and** `type_properties`, both the
  same `BTreeMap<String, CustomValue>` shape.
- **`loops` uses the room convention verbatim** — `loops[0]` outer,
  `loops[1..]` holes, decimal feet, model space, Y-up — so one renderer and one
  `model_to_shared` transform serve both entities.
- **Single phase collapses the room references.** `from_room`/`to_room` are
  arrays in the raw export only because they carry one entry per phase (all 46
  refs in the sample are `phase_id: 3`). Under Decision 2 they become
  `Option<String>` each: `None` for an external door, which is a normal state,
  not a missing value.
- **Its own `schema_version`, starting at 1.** Versioning doors against the room
  contract's v5 would couple two things that will move independently.
- **Drop the per-door placement.** Each polygon carries `rotation_coord` and
  `translation_coord` (~`[1094986, 20547804]`, survey millimetres) — a
  model-level fact duplicated on every door. `translate_room` already drops the
  equivalent per-polygon copy for rooms; `model_to_shared` on the envelope is the
  one true placement. Same reasoning as
  `docs/Superseded/HANDOVER-georeferencing.md` "Fact 1".

## Decision 5: reference sources become entity-scoped

> **Not built, on purpose — this is R4, and it is the one prerequisite doors
> shipped without.** It must land *with* doors' first reference source: earlier
> is a config field with no behaviour behind it, later is a back-compat
> obligation the day someone relies on a table that silently means "rooms".
> Doors shipped with no reference source, so nothing has started the clock.
>
> What the door work did settle is everything below the settings layer: the join
> namespace stayed **flat**, and one `split_namespace` / `PropertyTiers` /
> filter grammar now serves both entities — so R4 is a settings and wiring
> change rather than a grammar fork. A source-qualified predicate on a door
> today resolves `Absent` (matching nothing) rather than erroring, which is the
> same answer a room gets for a source it did not join; the day R4 lands, that
> predicate starts *matching* instead of changing status.

`[sources.reference.<name>]` gains an optional `entity` field defaulting to
`"rooms"`, so every existing settings file is unchanged and still means what it
meant. `[[builtin_properties]]` gains the same field for the same reason: doors
have their own `Mark`, `Level` and `Comments`, canonical names that collide with
room ones under a different meaning.

This is what retires the trap `docs/Superseded/HANDOVER-reference-sources.md` flagged and
[Sources](STRATEGY-SOURCES.md) documented — that adding `[sources.reference.doors]`
today parses, loads, and then silently no-ops, configured but joined nowhere. With
`entity` present, an unjoinable configuration is a **loud startup failure**
naming the unknown entity, per "loud startup over silent no-op".

The join namespace stays **flat** (`hardware.FireRating`, not
`doors.hardware.FireRating`): the namespace exists to answer "what goes before
the dot in a filter", and nesting it would fork that grammar for no gain. Source
names are therefore unique across entities, and a duplicate is a startup error.
`split_namespace` / `resolve_presence` / `source_joined` generalize by entity
rather than being duplicated per entity.

## Decision 6: connectivity is a different graph from adjacency

> **The correction half is done; the graph itself is not built and the open
> question below is still open.**
>
> Both false claims were corrected in the same change that made them false, as
> this section demands: `service::adjacency`'s module header and the
> `get_adjacency` MCP description both said the extractor collects no doors, and
> both now state what the distinction *is* and point at `get_doors`.
>
> `/projects/{id}/adjacency` keeps its meaning unchanged. A connectivity
> endpoint was **not** built, and does not need to be for the obvious question:
> every door on `/doors` names both of its rooms, so "which rooms are connected
> by a door" is a read, not a graph computation. What a real connectivity
> endpoint would add is the *graph* — traversal, components, path length — and
> that is worth building when something asks for it.

`service::adjacency` computes *shared wall* adjacency and says so explicitly in
its module header and its MCP tool description — "not door connectivity (the
extractor collects no doors)". Both claims become false the day doors land and
**both must be corrected in the same change**, or the next reader concludes the
existing endpoint already covers it.

Door connectivity is a genuinely different graph over the same rooms: two rooms
can share a wall with no door in it, and a door can connect two rooms that share
almost no wall. It is a second edge set, not a refinement of the first —
`/projects/{id}/adjacency` keeps its meaning and connectivity gets its own
endpoint when it is built.

### Which room owns a door — **decided and built**

**The rule: a door belongs to the room it opens into. If there is no `to_room`,
it belongs to the room it opens *from*. If there is neither, it is homeless.**

This is a **precedence chain**, which is not one of the four answers this section
originally offered (`to_room`, `from_room`, both, neither). That matters: each of
those four is a single pick that leaves external doors either mis-attributed or
unattributed, whereas the chain attributes every door that has any room at all
and names the remainder rather than hiding it. "Homeless" is a **reported
state**, not an error and not a zero — the same "signal, not error" stance an
unresolved reference gets, and it is already what QA reports as
`doors_without_room_reference`.

Measured on the House A sample before writing it down:

| | |
| --- | --- |
| Attributed via `to_room` | 23 of 26 |
| Attributed via `from_room` (external, no `to_room`) | 3 |
| Homeless | 0 |
| Rooms owning at least one door | 21 of 26 |

**The authored fifth candidate is a reconciliation input, not the rule.** The
`Door Room Reference` instance property agrees with the chain on **22 of 26**
doors and disagrees on 4 — and the disagreements have a pattern worth knowing
before anyone trusts either side blindly: three of the four are doors where the
chain picks an **exterior or circulation space** (`BACKYARD AND SIDE SOUTH`,
`HALL`) while the modeller authored the *served* room. That is exactly what
`reconcile_room_reference` is for, and it now has a demonstrated job rather than
a hypothetical one.

**The caveat that decides how far to trust the default.** Revit's
`FromRoom`/`ToRoom` follow the door instance's *orientation*, not the leaf swing
— flipping a door in the model swaps them. So "the room it opens into" is a
**modelling convention**, reliable exactly as far as doors were inserted
consistently, which is why this stays project policy with an override rather
than becoming a hard-coded rule. Same reasoning as `measurement_standard`
([Area calculation](STRATEGY-AREA-CALCULATION.md)).

**As built.** `[doors] room_attribution` (default `to_room_then_from_room`, with
the four single picks as alternatives); `owner_rooms` on every `/doors` row — a
**list**, because `both` attributes a door twice, and empty means homeless;
`?building=` on `/doors`, which a door answers through its owning room and which
only became askable once this was decided; and two QA findings —
`doors_unattributed` (the door names a room the policy declines to use, empty
under the default chain) and `room_reference_mismatches`.

Two shapes departed from the sketch above, both for the same reason — the sketch
assumed a single owner:

- `owner_rooms` is a list, not the `owner_room` first proposed.
- `reconcile_room_reference = true` became
  **`room_reference_property = "Door Room Reference"`**. A bool would have needed
  the property name hard-coded, and which parameter carries an authored room
  reference is a family-and-office convention, not a fact about doors. One field
  says both that the check is wanted and what it reads — and absent means the
  check is **off**, which the QA response states rather than leaving to look like
  "clean".

Attribution is derived at read time and never stored, so changing the policy
changes every answer immediately and rewrites nothing.

## Settings changes

> **`[doors]` now carries four keys: `comparison_key`,
> `comparison_properties`, `room_attribution` and `room_reference_property`.**
> The first two arrived with milestone comparison; the last two with the
> ownership decision (Decision 6), which is why this block's
> `reconcile_room_reference` is superseded — see there for why a property name
> beat a bool. `door_label` is still absent: it has no viewer to feed.
>
> A third pin map arrived that this block does not mention:
> `[[milestones]] door_attachments`, model id → doors snapshot id. A *separate*
> map from `attachments` rather than a reuse of it, because rooms and doors are
> pushed independently and their snapshot ids do not correspond — pinning "the
> nearest doors snapshot" to a rooms pin would silently pair data that never
> coexisted.
>
> The `[sources.reference.hardware]` and `[[builtin_properties]] entity`
> examples below are R4, and are still design — see Decision 5.
>
> ```toml
> # As shipped, on the House A sample project:
> [doors]
> comparison_key = "$id"
> comparison_properties = ["$to_room", "$from_room", "Mark"]
>
> [[milestones]]
> name = "Rooms and doors"
> date = "2026-07-29"
> attachments      = { "…model…" = "2026-07-24T23:19:10.227000Z" }
> door_attachments = { "…model…" = "2026-07-29T20:03:41Z" }
> ```

```toml
# Which phase a push is scoped to is NOT configured -- it is chosen at export
# time and rides the envelope (Decision 2). Nothing here declares it.

[doors]
# Which room a door belongs to. The DEFAULT is a precedence chain, not one of
# the single picks: the room it opens into, else the one it opens from, else
# homeless.
room_attribution = "to_room_then_from_room"
# to_room_then_from_room | to_room | from_room | both | none

# The door property carrying an AUTHORED room reference, reconciled against the
# room the policy attributed the door to. Absent = check off. Not hypothetical:
# on the House A sample this disagrees with the attributed room on 4 of 26.
room_reference_property = "Door Room Reference"

# Ordered properties shown on a door in the viewer -- mirrors room_label.
door_label = ["$mark", "Door Type"]

# Entity-scoped reference sources (Decision 5). Absent `entity` means "rooms",
# so every pre-existing file is unchanged.
[sources.reference.hardware]
type = "upload"
entity = "doors"
join_property = "Mark"

[[builtin_properties]]
canonical = "Mark"
entity = "doors"
by_source = { revit = "Mark" }
```

## MCP surface

> **Built, with one tool instead of two.** `get_doors` shipped; a
> `list_door_snapshots` did **not**, because doors' snapshot ids were folded into
> the existing `/projects/{id}/snapshots` response rather than given a route of
> their own — and this file's own rule is one tool per HTTP *read route*, so no
> new route means no new tool. A documented departure, not an omission.
>
> The three descriptions this section says must be edited in the same change all
> were, and `get_validation`/`compare_milestones` each gained a doors section
> beyond what is listed here. The header count went 16 → 17.

`bin/mcp.rs`' module header declares one tool per HTTP read route, and skipping
that is how the two front doors drift ([MCP](STRATEGY-MCP.md)). Doors add
`get_doors` and `list_door_snapshots`, both thin adapters over the same
`service` functions the Axum handlers call. Ingest stays HTTP-only, consistent
with rooms.

Three descriptions need editing in the same change, not later:

- `get_adjacency` — drop "the extractor collects no doors" and point at the
  connectivity endpoint instead (Decision 6);
- the header's tool count ("Fifteen in total") and its inline list;
- `get_rooms` / `get_doors` — both must state that results are scoped to one
  phase, or an agent will read a partial model as a complete one.

## Blockers in the current export

> **All three are resolved, and two of them were never as bad as this section
> claimed.** Kept with corrections rather than deleted: two were assertions about
> the data that turned out to be wrong, and "we checked, and here is what was
> actually there" is worth more than a section that quietly disappears. A fourth
> problem this section did not anticipate is added at the end — it was the only
> one that changed the contract.

1. ~~**No door id.**~~ **Stale — there is one.** A later duHast carries it at
   `instance_properties.id`, the same place `translate_room` reads a room's id.
   26 unique values across the sample, colliding with no room id. The rest of the
   original point stands and is why the id was needed: `Mark` is not unique (one
   door in the sample is `"None"`) and `IfcGUID` is an instance property, not
   identity.
2. ~~**`phase_id` is unresolvable as shipped.**~~ **True, and now moot.** There
   is still no phase table in the file, and `phasing.created` is
   `"New Construction"` while room refs carry the integer `3`. The fix is not
   the one this section proposed — Decision 2 dropped the `{ id, name }` pair and
   carries a bare name. The extractor instead reads the room references straight
   from the Revit API (`FamilyInstance.FromRoom[phase]`, which takes the phase
   and answers exactly one room), so nothing ever needs to resolve `phase_id`.
   That is also what makes the reference genuinely one-to-one, as `Door`'s
   `Option<String>` claims.
3. **Unit mismatch within one record — unchanged, and still a trap.**
   `polygon.outer_loop` is decimal feet (~22–26, matching `Room.loops`) while
   instance and type properties are millimetres (`FrameDepth: 100.0`). Geometry
   stays feet; properties stay raw strings and are the settings layer's problem.
   Stated here because otherwise someone will "normalize" one of them.
4. **The one this section missed: a degenerate footprint that looks real.** Two
   of the 26 doors (both family type `2040x620x40`, no 3D geometry) carry
   `outer_loop` coordinates of **±1e30** — Revit's *uninitialized*
   `BoundingBoxXYZ`, whose min is `+1e30` and max `-1e30`. duHast's own "did we
   get a box" and "is the loop non-empty" guards both pass, so the bad value
   arrives looking like a plausible footprint rather than an absent one. Pushed
   as-is it hands every consumer a polygon 1e30 feet across — the
   million-foot-spike class from [Area calculation](STRATEGY-AREA-CALCULATION.md).
   The producer recognises the sentinel and sends **no loops**; the door is still
   pushed, because both of these carry valid room references and are therefore
   real doors QA must see. `Door.loops` is optional for exactly this reason.

**Verified, and it holds:** `from_room[].room_id` *is* the same id space as
`Room.id`. All 22 room ids referenced by the sample doors resolve against the
House A rooms snapshot from the same model. The door→room join has its key —
and, because room ids are unique only *within* a model, that join is
model-scoped everywhere it appears (ingest gate, QA resolution, `model_id` on
every `/doors` row).

Also worth a look while the export is being changed: `associated_elements` is
`[]` and `design_set_and_option` is `{"option_name": "-", "set_name": "Main
Model", "is_primary": true}` on all 26 doors. Design options are a second
model-variant axis crossing phase, and the same "one at a time, chosen at
export" logic would apply — but with no varying sample data there is nothing to
design against yet.

## Where the rest of this lands

Per [Index](STRATEGY.md)'s rule that a change touching more than one layer
updates every doc it touches:

All done, in the same pass that marked this document built:

| Doc | What it gained |
| --- | --- |
| [Index](STRATEGY.md) | ✅ doors as the third upload type, its independent `schema_version`, and the two ingest preconditions a dependent entity needs |
| [Sources](STRATEGY-SOURCES.md) | ✅ the "no doors entity" boundary note replaced — it is now a *gap* R4 closes, not a boundary, and the note says what the door work already settled |
| [Server](STRATEGY-SERVER.md) | ✅ the storage kind and the bytes-at-the-boundary trait, plus why the flat-merge stopgap stopped being harmless once room ids became a door's foreign key |
| [Browser](STRATEGY-BROWSER.md) | ✅ doors are served and not drawn, and the fetch decision is recorded: their own poll, for a stronger reason than `/adjacency`'s |
| [MCP](STRATEGY-MCP.md) | ✅ `get_doors`, the corrected `get_adjacency`, and why a stale tool description is a wrong answer rather than a doc debt |
| [Conventions](CODING-CONVENTIONS.md) | nothing — every rule it states already covered this work, and the one thing it predicted (door types trigger the `contract/` split) happened as written |

## Deferred

Still open after doors shipped:

- **R4 — entity-scoped reference sources.** The one prerequisite doors shipped
  without, on purpose; lands with doors' first reference source (Decision 5).
- ~~**Door ownership.**~~ **Decided and built** (Decision 6) — including
  `?building=` on `/doors`, which a door answers through its owning room. What
  remains open underneath it is a *data* question rather than a design one: the
  authored `Door Room Reference` disagrees with the attributed room on 4 of the
  26 House A doors, and QA now reports them. Three are doors where the geometry
  picks an exterior or circulation space over the served room, which is worth
  someone deciding about — but it is a modelling call, not a pipeline one.
- **Any door viewer.** `/doors` is served and nothing draws it
  ([Browser](STRATEGY-BROWSER.md)).
- **Multi-phase comparison.** Explicitly out of scope (Decision 2).
- **Door connectivity graph.** The endpoint and algorithm (Decision 6) — noting
  that the simple question ("which rooms does a door connect") is already a read
  off `/doors`.
- **Design options** as a second variant axis. Still no varying sample data:
  all 26 doors remain `{"option_name": "-", "set_name": "Main Model"}`.
- **Type-property deduplication.** `type_properties` rides per instance today;
  a shared type table is a payload-size optimization to take when measured, not
  before. Measured figure to start from: the House A doors snapshot is 414 KB
  for 26 doors, and `type_id` is on the wire ready to key a shared table.
- **Verifying the extractor against Revit.** The doors exporter is verified by
  running its real translation over a captured export; the `get_FromRoom`
  accessor and the `OST_Doors` collector need a live document. Same standing as
  the room extractor's phase filter.
- **FFE**, the next axis-1 entity. **The bet this document made is now
  testable**: if FFE needs anything beyond the phase envelope, a
  `kind`-parameterized store and a `PropertyTiers` impl, one of the decisions
  above was too narrow. Doors needed exactly two things past that list — the
  rooms-first ingest gate and the model-scoped reference resolution — and both
  are consequences of being a *dependent* entity. FFE that hangs off rooms will
  need them too; FFE that stands alone will not.
