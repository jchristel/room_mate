# RoomMate — Phasing implementation plan

**Status: built (P1–P7), unverified against Revit.** Records the phase design
agreed before any code, so the implementation doesn't re-derive it and the open
questions were open *before* the work rather than after. All seven phases have
landed; see [As built](#as-built) for the three places the result differs from
the plan below, and for what still needs a live Revit run to confirm.

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[Entities](STRATEGY-ENTITIES.md) · [MCP](STRATEGY-MCP.md) ·
[Conventions](CODING-CONVENTIONS.md)

Phases are being built **before** doors, which reverses the order
[Entities](STRATEGY-ENTITIES.md) assumes. That doc's Decision 2 describes phase
arriving *with* doors; several of its details are superseded here (see
[What this supersedes](#what-this-supersedes)). Doors and FFE are the reason
phasing exists, but rooms carry it first.

## Why phasing exists

A Revit document holds many phases. An element exists in a phase per the range
test

```
created <= selected  AND  (demolished is invalid  OR  demolished > selected)
```

and only the live document has the phase ordering that `<=` needs. So the
**extractor** filters, and the server never re-evaluates the predicate. The
phase name rides the push so the server knows what it was given.

RoomMate supports **one phase per push**, and — per the decisions below — one
phase per `(project, model)` lineage for its whole life. Multi-phase comparison
stays out of scope; milestones already answer "the model as it was on date X".

## Decisions

### D1 — The phase is a name, and nothing else

`phase: Option<String>` on the payload; wire shape `"phase": "New Construction"`.

No `{id, name}` struct. Every document numbers its own phases, so an id is not
comparable across models and was only ever going to be carried "for display" —
and the sample value (`3`) is low enough that it may be a `doc.Phases` *index*
rather than an `ElementId`, i.e. possibly meaningless even for display. An
unread field that might be wrong is one that drifts. If a phase ever needs a
second attribute, that is a schema bump, which is what schema versions are for.

### D2 — `Option` on the type is forced; strictness lives at ingest

The stored file `<project>/<model>/<taken_at>.json` **is** a serialized
`RoomPayload`, re-parsed on every read and every boot (`get_latest`,
`all_latest`, `get_snapshot`). A required `phase` field would stop every
snapshot already on disk from deserializing — the server could not hydrate,
list history, or serve `/rooms`. So the field is `Option` on the type
permanently.

That is independent of what a *new push* may omit, which is enforced in the
ingest handler (D4). Keeping the two separate is what lets ingest be strict
without breaking stored data: the type stays permissive so history reads, the
handler stays strict so new data is correct.

**`SUPPORTED_SCHEMA` bumps to 6.** Unlike `model_to_shared` and `room_boundary`,
this is not a pure relaxation — under D4 a payload that was valid v5 (no phase)
is now rejected, so its meaning has changed from "a legal unphased push" to "an
error". That is precisely the test `contract.rs` states for a bump. The bump is
also the more useful failure: a stale producer is told "schema 5 is not
supported" rather than "you forgot a field", which names the actual problem —
its extractor is old. No transition window, same as the v4 → v5 move: update the
extractor and the server together.

The bump does not touch stored data. The version is checked at ingest only;
snapshots already on disk are deserialized without a version check, and D2's
`Option` is what keeps them readable.

### D3 — Comparison is trimmed and case-insensitive; the first push's casing is stored

Trimming absorbs export whitespace. Case is absorbed too: `"NEW CONSTRUCTION"`
and `"New Construction"` are the same phase. The **first** push's casing is what
is stored and echoed back; later pushes' casing is discarded.

This reverses [Entities](STRATEGY-ENTITIES.md) Decision 2's case-sensitive rule.
That rule was argued for a world where a mismatch was a *rejection* worth making
loud; under D5 a mismatch quarantines the push instead, and quarantining a
correct push over letter-case would be a bad trade.

### D4 — Every push carries a phase, rooms included

The extractor always resolves and sends one, so a push arriving without a phase
is not a legitimate unphased export — it is a producer predating phase support,
whose content was never filtered by the range test at all. It is rejected
regardless of whether the lineage is already phased (D5).

This applies to rooms exactly as to doors and FFE. There is no "rooms may be
unphased" concession: allowing one would mean accepting unfiltered mixed-phase
content into the store, which is the very thing phasing exists to prevent.

**Unphased *models* still exist** — every model already on disk is one, because
its pushes happened before the field did. That is a property of stored history,
not of anything new, and D8 keeps it honest: those snapshots report themselves
as unphased forever. Their lineage becomes phased the next time anything is
pushed to it, since that push necessarily carries a phase.

### D5 — A lineage's phase is immutable once set, and phasing is a one-way door

Once `(project, model)` has a phase, every later push must state the same one. A
push naming a *different* phase is **quarantined**, not rejected (D6).

A push naming **no** phase is **rejected outright — a 422, never quarantined**,
against any lineage (D4). The two failures look similar and are not. A
differently-phased push is a correct export of a different phase, worth keeping
so the user can activate it. A push with no phase was never filtered by the
range test at all — it is unfiltered mixed-phase data. There is nothing to
activate, and offering to activate it would be offering to corrupt the model.

Under the D2 bump this is normally caught one step earlier, by the
`schema_version` check. The explicit no-phase rejection stays as the backstop
for a producer that claims v6 and sends no phase anyway, and its message names
that cause specifically, per "loud startup over silent no-op".

So phasing is one-way: unphased → phased, never back.

### D6 — A *differently-phased* push is stored, not refused

This covers only a push naming a different phase. An unphased push is a hard
reject (D5) and never reaches here.

Refusing a differently-phased push would make the user re-run the export to fix
a mistake, and would leave a mis-phased model permanently wrong given there is
no delete route. Instead:

- the payload is stored under `<model>/pending/`, and the response is
  **`202 Accepted`** with the stored id and the reason: semantically exact
  ("accepted, not yet acted upon") and distinguishable from a normal `200`
  ingest without inventing a convention;
- **one pending snapshot per model**, overwritten on re-push. Only the newest is
  ever the one anyone would activate, and with no delete route an accumulating
  pile is unclearable;
- nothing reads it. `get_latest` / `all_latest` / `list_snapshot_ids` and
  milestone pinning must all skip it.

`pending/` as a subdirectory rather than a manifest flag because both model-dir
scans filter on `extension == "json"` (`storage/fs.rs`), and a directory has no
extension — so the quarantine is invisible to every existing scan with no change
to them. Same additivity argument [Entities](STRATEGY-ENTITIES.md) makes for
`doors/`.

### D7 — Activation is an endpoint, not a settings field

`POST /projects/{p}/models/{m}/snapshots/pending/activate`.

Activation updates `ModelEntry.phase` and moves the pending file into the model
dir. It is a store mutation, and settings today never writes to the store —
routing it through the settings save pipeline would introduce that coupling for
nothing. The settings *UI* can still own the button; it calls this route.

### D8 — The manifest is the enforcement key; the snapshot file is the record

`ModelEntry.phase` in `project.toml` says what future pushes must match. Each
snapshot's own JSON says what *that push* actually was.

**Reads always report the phase of the snapshot they loaded, never the
manifest.** This is what stops an old unphased snapshot from being retroactively
relabelled: its file has no phase because it was never filtered to one, and that
stays true after a later push sets the lineage's phase. A milestone pinning an
old snapshot correctly reports it as unphased.

Same index-vs-record split `ProjectManifest` already documents for `name`.

### D9 — Mixed-phase reads merge, and are reported

`/rooms` merges every model's latest snapshot even when models disagree on
phase. Enforcing agreement across a project deadlocks — moving a project from
Phase A to Phase B would require pushing model 1 (rejected, it disagrees with
model 2) and neither could go first.

Disagreement is "signal, not error": `/rooms` carries each model's phase so a
client can tell what it is looking at, and the validation report names the
disagreement so a user hunting for problems finds it. Under D5 the disagreement
is *permanent* until someone activates a re-phase, which makes reporting it more
important than it would be for a transient state.

### D10 — Deliberately not doing

- **No `?phase=` filter.** One phase per model means there is nothing to filter
  within a model; filtering across models is multi-phase comparison.
- **No storage partition by phase.** A model has exactly one phase, so
  `<model>/<phase>/` would always hold exactly one subdirectory, and
  partitioning would fork a model's history into parallel timelines.
- **No viewer display** in this change.
- **No `contract/` split.** [Entities](STRATEGY-ENTITIES.md) called for splitting
  `contract.rs` into a directory; that was driven by the door types, which are
  not in this change. `contract.rs` stays one file.

## Implementation

### P1 — Contract

`Option<String>` phase on `RoomPayload` and `StreamEnvelope`, in lockstep — the
streamed and buffered paths must store identical envelope facts, and there is an
existing test asserting exactly that for `room_boundary` to copy.

`SUPPORTED_SCHEMA` 5 → 6 (D2), with the constant's doc comment extended to say
why this one *is* a bump when the last three additions were not: the field is
required at ingest, so a v5 payload that omitted it has changed meaning rather
than staying valid. The existing v5 test fixtures all need the new version and a
phase; that churn is the point — it is the same churn a real producer faces.

A `phases_agree(a: Option<&str>, b: Option<&str>) -> bool` free function beside
`ensure_taken_at`: trim, case-insensitive, either side absent ⇒ compatible.
Tests: agreement across casing and whitespace, disagreement on a real
difference, absence on either side, and that the streamed envelope carries the
field.

### P2 — Storage

- `ModelEntry.phase: Option<String>`, declared **after `name` and before
  `snapshots`** — the TOML ordering footgun in
  [Conventions](CODING-CONVENTIONS.md); a scalar after a collection field lands
  inside it. `#[serde(default)]` keeps existing manifests parseable.
- `PENDING_DIR` constant beside `REFERENCE_DIR`.
- Trait methods for putting, reading and promoting a pending snapshot. All
  concretely typed on `RoomPayload` — `AppState` holds `Box<dyn SnapshotStore>`,
  so nothing here may be generic.
- `MemStore` keeps one pending per key in memory; it is latest-only by design
  and that is unchanged.
- Tests: a pending snapshot is invisible to `get_latest`, `all_latest` and
  `list_snapshot_ids`; a second pending overwrites the first; promotion moves it
  and updates the manifest.

### P3 — Ingest

Normalize (trim; empty ⇒ `None`), then read `ModelEntry.phase`:

| pushed | stored | result |
| --- | --- | --- |
| none | *any* | **reject, `422`** — a producer predating phase support (D4, D5) |
| some | none | accept, set the lineage's phase |
| some | some, agree | accept |
| some | some, differ | quarantine, `202` (D6) |

The two failure rows are the ones to get right, and they are asymmetric on
purpose: a differently-phased push is kept because it is real filtered data the
user may want; an unphased one is refused because it is not.

The `schema_version` check runs first and catches a stale producer before this
table is reached; row 1 is the backstop for something claiming v6 without a
phase.

Doors/FFE will additionally reject an absent phase outright (D4) — not in this
change, but the check belongs where this one goes.

### P4 — Activate

The endpoint from D7, plus its handler and service function. Absent pending ⇒
`404`. Tests through `FsStore` so the file move is real.

### P5 — Reads

- `/rooms`: per-model phase on the response.
- `snapshots/latest`: the phase, from the manifest index — this read
  deliberately never opens a snapshot file, which is why D8 puts the phase in
  the manifest at all.
- `ValidationResponse` gains a top-level `phases: PhaseReport { by_model:
  BTreeMap<String, Option<String>>, disagree: bool }`. It cannot go under
  `sources`, which is keyed by reference-source name — a phase disagreement has
  no source.

### P6 — Docs and MCP

- Rewrite [Entities](STRATEGY-ENTITIES.md) Decision 2 against what was actually
  built (see below), and fix that doc's two `Superseded/` link paths.
- [Entities](STRATEGY-ENTITIES.md) is currently an orphan — nothing links to it.
  Add it to [Index](STRATEGY.md)'s doc list (and its "seven docs … plus one
  forward-looking" count), `docs/README.md`'s table, and the nav line in the
  seven sibling docs.
- [Server](STRATEGY-SERVER.md): the pending/activate route and the phase rules.
- `bin/mcp.rs`: `get_rooms`' description must state results are scoped to one
  phase per model, or an agent reads a partial model as a complete one. No new
  tools, so the header's "Fifteen in total" is unaffected.

### P7 — Extractor (not Rust)

Filter elements by the range test — **not** `created == selected`, which drops
every element built in an earlier phase and still standing. This will not show
up against the current sample, where all 26 doors carry
`demolished: "Invalid phase id."` and the two agree. Send the resolved phase
name on the envelope.

Prompt UX: `pick_document` is multiselect and phases are per-document, so prompt
**once** with the names common to the selected documents and resolve each
document's own phase by name, failing loudly for a document lacking it. Skip the
prompt when a document has one phase.

## As built

Three deliberate departures from the plan above, and one caveat.

**P4 gained a read route the plan didn't specify.** `GET
/projects/{p}/models/{m}/snapshots/pending` sits beside the activate route,
because without it the feature does not work: the `202` at push time is the only
other place a quarantine is announced, and the person who pushes from Revit is
generally not the person deciding to re-phase. That made it an HTTP *read*
route, and `bin/mcp.rs` asserts one MCP tool per read route — so
`get_pending_snapshot` was added and the tool count moved **fifteen → sixteen**.
Activating stays HTTP-only, the line ingest already draws.

**The validation report now reads storage even when no reference source is
configured.** `compute_project_validation` used to bail before `all_snapshots()`
when nothing was loaded. The phase report is a room-versus-room finding that
owes nothing to reference data, and a project reconciling against nothing can
still be serving two phases — that being exactly the project nobody is watching.
`total_rooms` and `discrepancies` still read zero in that case, unchanged.

**`phase_by_model` on `/rooms` is nested per project**, not flat by model id: an
unscoped read merges every stored project and model ids are only unique within
one. Same shape, same reason, as `reference_labels`.

**The extractor half (P7) is unverified against Revit.** `exists_in_phase` — the
range test itself — is a pure function and was exercised directly, including the
`created == selected` trap it exists to prevent. Everything around it
(`doc.Phases` ordering, `CreatedPhaseId` / `DemolishedPhaseId`, the
`OST_Rooms` collector, and whether the collector's element ids match the ids
duHast writes into the export) has never run against a document. That id match
is the one to check first: if it does not hold, the filter silently keeps
nothing. `element_id_str` handles both the `.Value` and `.IntegerValue` spellings
because this script must run on old and new Revit alike.

## What this supersedes

[Entities](STRATEGY-ENTITIES.md) Decision 2 has since been **rewritten in place**
to describe what shipped, so the two no longer disagree. The table below is the
record of what changed between design and build — useful for understanding *why*
the decisions look as they do, not a live warning about a stale doc:

| Entities said | Now |
| --- | --- |
| `Phase { id, name }` | a bare name (D1) |
| case-sensitive comparison | case-insensitive (D3) |
| a disagreeing push is a 422 | quarantined and activatable (D6, D7) |
| phase arrives with doors | rooms carry it first (D4) |
| no schema bump on the room contract | `SUPPORTED_SCHEMA` 5 → 6 (D2) |
| — | a lineage's phase is immutable (D5) |

The schema row is the one that most needs correcting in place: Entities argues
the no-bump case from the omittable-snapshot-id precedent, and that argument is
sound *for an optional field*. It stops holding the moment the field is required
at ingest, which is a decision taken after that doc was written.

Its Decision 2a (identity is the name), 2b (the range test, extractor-side, and
the ordered phase list staying off the wire), 2d (extend `snapshots/latest`
rather than adding a route) and 2e (prompt once against multiselect) stand
unchanged.
