# RoomMate — Entities & Phasing Strategy

**Status: doors are design-only; phasing is built.** It records the shape
agreed for the pipeline's *second* primary entity — doors — and for the phase
selection every primary entity after rooms will need, so the first
implementation doesn't re-derive it.

> ## ⛔ Before writing any door code, read this
>
> Doors have **prerequisites**, and they are not optional. They live in
> **[PLAN-generalisation.md § The line in the sand](PLAN-generalisation.md#the-line-in-the-sand)**.
> In short:
>
> - **R2 — lift the property lookup off `&Room` — lands *before* the `Door`
>   contract is final.** Its open question (does a door's instance property
>   *shadow* its type property, or is a name in both a finding?) is a **contract**
>   decision, not a refactor detail. Decide it after `Door` is written and you
>   either rewrite the type or live with the wrong answer. Decision 4 below keeps
>   the two tiers separate precisely because they are different claims.
> - **R1 — generalise `SnapshotStore` off `RoomPayload` — lands *with* doors, not
>   after.** The moment a `put_doors` appears beside `put`, the third parallel
>   method set exists and FFE makes it a fourth. That is the exact failure
>   [Decision 3](#decision-3-doors-are-their-own-upload-type-modelled-on-rooms)
>   below was written to prevent, and far cheaper to avoid than to undo. Note the
>   constraint Decision 3 does not mention: `AppState` holds
>   `Box<dyn SnapshotStore>`, so the trait must stay **object-safe** — a generic
>   `put<T>` is out.
> - **R4 — entity-scope `[sources.reference.*]` — lands with doors' first
>   reference source**, not before (it needs R2) and not later (a shipped table
>   that silently means "rooms" becomes a back-compat obligation).
>
> Its words, not a paraphrase: *if doors ship without R1 and R2, this document
> has failed and the debt is permanent — every subsequent entity pays it again.*

> **Decision 2 (phasing) is built, and has been rewritten here to match.** It
> shipped ahead of doors and several details changed on contact with the code;
> rather than leave the original sketch standing behind a warning, 2a and 2c now
> describe what exists and say what they replaced. **[PLAN-phasing.md](PLAN-phasing.md)
> carries the full rationale** and is authoritative if the two ever drift. The
> rest of this document — Decisions 1 and 3 through 6 — is untouched and still
> design-only.

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
> [PLAN-phasing.md](PLAN-phasing.md) carries the full rationale and the ten
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
`doors.json`'s `from_room[].phase_id` is `3`, low enough to be an *index* into
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
up in testing against the current sample: all 26 doors in `doors.json` carry
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

Full annotated types in `contract-doors.rs`. The decisions behind them:

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

### Open question: which room owns a door

Unresolved, and flagged here so it does not get decided twice by accident. For
area rollups, hierarchy scoping, and door schedules, "the room this door belongs
to" has at least four defensible answers: the `to_room`, the `from_room`, both
(the door counts twice), or neither (a door belongs to the *boundary*, not a
room). The sample data offers a fifth: a `Door Room Reference` instance property
(`"00.09"`) that looks like an authored room number and may disagree with the
geometric `to_room`.

Whatever is chosen, the choice belongs in `[doors]` settings, not in code — it is
project policy in exactly the way `measurement_standard` is
([Area calculation](STRATEGY-AREA-CALCULATION.md)).

## Settings changes

```toml
# Which phase a push is scoped to is NOT configured -- it is chosen at export
# time and rides the envelope (Decision 2). Nothing here declares it.

[doors]
# Which of a door's two room references is authoritative for rollups and
# schedules (Decision 6's open question). No default -- an unset value means
# "do not attribute doors to rooms", which is honest rather than arbitrary.
room_attribution = "to_room"   # to_room | from_room | both | none
# Whether an authored room reference that disagrees with the geometric one is a
# validation finding or ignored.
reconcile_room_reference = true

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

Three, all extractor-side, all blocking the contract:

1. **No door id.** The record's keys are `phasing`, `data_type`,
   `super_component_id`, `instance_properties`, `associated_elements`,
   `revit_model`, `from_room`, `level`, `to_room`, `type_properties`,
   `design_set_and_option`, `polygon`. Nothing identifies the element. `Mark` is
   not unique (one door in the sample is `"None"`); `IfcGUID` is an instance
   property, not identity. Fix: `str(el.Id.Value)` at extraction, per the
   ElementId discipline. Storage keys, connectivity edges, and every reference
   join are blocked on this.
2. **`phase_id` is unresolvable as shipped.** There is no phase table in the
   file (top level is `date processed`, `file name`, `door`), and the names live
   in a different shape — `phasing.created` is `"New Construction"` while room
   refs carry the integer `3`. Decision 2 fixes this by carrying `{ id, name }`
   for the *selected* phase on the envelope.
3. **Unit mismatch within one record.** `polygon.outer_loop` is decimal feet
   (~22–26, matching `Room.loops`) while instance and type properties are
   millimetres (`FrameDepth: 100.0`). Geometry stays feet; properties stay raw
   strings and are the settings layer's problem. Stated here because otherwise
   someone will "normalize" one of them.

**To verify before building:** that `from_room[].room_id` (e.g. `2626842`) is
the same id space as `Room.id`, which `translate_room` sources from
`instance_properties["id"]`. If those diverge, the door→room join has no key.

Also worth a look while the export is being changed: `associated_elements` is
`[]` and `design_set_and_option` is `{"option_name": "-", "set_name": "Main
Model", "is_primary": true}` on all 26 doors. Design options are a second
model-variant axis crossing phase, and the same "one at a time, chosen at
export" logic would apply — but with no varying sample data there is nothing to
design against yet.

## Where the rest of this lands

Per [Index](STRATEGY.md)'s rule that a change touching more than one layer
updates every doc it touches:

| Doc | What it gains |
| --- | --- |
| [Index](STRATEGY.md) | `phase` on the upload envelope; the door contract alongside v5; doors as the third upload type |
| [Sources](STRATEGY-SOURCES.md) | the phase prompt and the extractor-side filter; the door export shape; replace the "no doors entity" boundary note with a pointer here |
| [Server](STRATEGY-SERVER.md) | the `/doors` routes, the storage kind, the `SnapshotStore` generalization, the phase check, `[doors]` settings |
| [Browser](STRATEGY-BROWSER.md) | whether doors ride the 2s room poll or their own fetch — by the `/adjacency` precedent, their own |
| [MCP](STRATEGY-MCP.md) | the two new tools and the three corrected descriptions |
| [Conventions](CODING-CONVENTIONS.md) | nothing — every rule it states already covers this work |

## Deferred

- **Multi-phase comparison.** Explicitly out of scope (Decision 2).
- **Door connectivity graph.** The endpoint and algorithm (Decision 6).
- **Design options** as a second variant axis.
- **Type-property deduplication.** `type_properties` rides per instance today;
  a shared type table is a payload-size optimization to take when measured, not
  before.
- **FFE**, the next axis-1 entity. If it needs anything beyond the phase
  envelope and a `kind`-parameterized store, that is a sign one of the
  decisions above was too narrow.
