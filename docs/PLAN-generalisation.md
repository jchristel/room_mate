# RoomMate — Generalisation plan

**Status: R3 done. R1, R2 and R4 are prerequisites for doors — see
[The line in the sand](#the-line-in-the-sand).** Four structural items surfaced
by reviewing the codebase after phasing shipped. Each is a generalisation the
codebase deferred one entity too long; none is a defect today, and three of the
four become blocking the moment a second primary entity (doors, then FFE)
arrives.

## The line in the sand

Doors are next. These are the terms:

- **R2 lands before doors' contract is final.** Its open question — whether a
  door's instance property shadows its type property — is a *contract* decision,
  not a refactor detail. Deciding it after `Door` is written means rewriting the
  type or living with the wrong answer.
- **R1 lands with doors, not after.** The moment a `put_doors` appears beside
  `put`, the third parallel set exists and FFE makes it a fourth. That is the
  precise failure [Entities](STRATEGY-ENTITIES.md) Decision 3 was written to
  prevent, and it is much cheaper to avoid than to undo.
- **R4 lands with doors' first reference source**, not before (it needs R2) and
  not later (a shipped `[sources.reference.*]` that silently means "rooms"
  becomes a back-compat obligation the day someone relies on it).

If doors ship without R1 and R2, this document has failed and the debt is
permanent — every subsequent entity pays it again. That is the line.

R3 is done: [see below](#r3--the-toml-ordering-footgun-is-documented-not-designed-out)
— and it did not go as planned, which is recorded there.

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Server](STRATEGY-SERVER.md) · [Entities](STRATEGY-ENTITIES.md) ·
[Sources](STRATEGY-SOURCES.md) · [Conventions](CODING-CONVENTIONS.md)

## The shape of the problem

Rooms were the only primary entity for the whole life of the project, so three
load-bearing seams grew room-shaped: the store persists `RoomPayload`, the
property lookup takes `&Room`, and reference sources implicitly mean "for
rooms". [Entities](STRATEGY-ENTITIES.md) names all three — Decisions 3 and 5 —
but asserts the fixes in a clause each without a signature that survives contact
with the code. This document supplies those.

**Honest framing on urgency: if doors are not imminent, only R3 is worth doing
now.** R1, R2 and R4 are prerequisites for a second entity and buy nothing
before it. Doing them speculatively would be the same mistake in the other
direction.

## R1 — `SnapshotStore` is typed on `RoomPayload`

**The problem.** `put`, `get_latest`, `all_latest` and `get_snapshot` all name
`RoomPayload` concretely. A doors payload has nowhere to go. The obvious fix —
a parallel `put_doors` set — is what
[Entities](STRATEGY-ENTITIES.md) Decision 3 rightly rejects, since FFE would make
it a fourth.

**The constraint the doc misses.** `AppState` holds `Box<dyn SnapshotStore>`
([state.rs](../src/state.rs)), so the trait must stay **object-safe**. A generic
method (`fn put<T: Serialize>`) is therefore out, and adding a `kind` parameter
to the existing signatures accomplishes nothing while the payload type is still
fixed.

**Proposal: bytes at the trait boundary, types above it.**

```rust
pub enum SnapshotKind { Rooms, Doors }

fn put_raw(&self, kind: SnapshotKind, key: &ModelKey, taken_at: &str, json: &[u8]) -> Result<()>;
fn get_latest_raw(&self, kind: SnapshotKind, key: &ModelKey) -> Result<Option<Vec<u8>>>;
// … list/get/pending equivalents, each gaining `kind`
```

Serde moves to a thin typed layer on `AppState`, whose public methods keep their
current signatures — so `set_snapshot(payload)` and `all_snapshots()` are
unchanged for every caller outside `storage/` and `state.rs`.

**This is not a new pattern.** `put_reference` already does exactly this: bytes
in, a `source: &str` discriminator, typed parsing above. R1 is applying the
store's own existing shape to its other half.

**Decisions this forces, all worth writing down:**

- **`SnapshotKind` is a closed enum, not a string** — unlike `Model.source`,
  which is deliberately open. An entity needs a Rust assembler to exist, so the
  set genuinely is code-bound; it also becomes a storage path component, and an
  enum makes it path-safe by construction.
- **Layout stays asymmetric.** `Rooms` keeps `<model>/<ts>.json`; other kinds get
  `<model>/<kind>/<ts>.json`. Migrating existing data to a `rooms/`
  subdirectory buys nothing and risks a store nobody can read.
- **What does `list_models()` mean now?** Today: "models with room snapshots".
  Candidate: "models with a snapshot of any kind", which changes `/rooms` and
  `/projects` for a doors-only model. Needs an explicit answer.

**Effort:** medium. Blast radius is mostly inside `storage/` and `state.rs`.
**Unblocks:** doors and FFE storage. Nothing else.

## R2 — the property/join path is `&Room`-typed

**The problem.** `lookup_property`, `property_presence`
([contract.rs](../src/contract.rs)), and the join/filter machinery in
`service::rooms` and `service::validation` all take `&Room`. Doors need the same
resolution, and [Entities](STRATEGY-ENTITIES.md) Decision 5 covers this in one
clause ("generalize by entity") that is in fact the largest single chunk of
doors work.

**The complication that makes it interesting.** These functions need only the
property map from a `Room` — so the cheap fix is to take
`&BTreeMap<String, CustomValue>` and be done. But **a door has two property
tiers**: its own instance properties and its family *type* properties, shared
across every instance of that type. A door lookup is "instance tier, then type
tier", and a flat map cannot express that.

**Proposal: a tiered-properties trait.**

```rust
pub trait PropertyTiers {
    /// Highest precedence first.
    fn tiers(&self) -> Vec<&BTreeMap<String, CustomValue>>;
}
```

`Room` returns one tier, `Door` returns two. `lookup_property` walks them in
order and returns the first hit. Object-safe, no lifetimes beyond the borrow,
and it puts tier precedence in exactly one place.

**The decision it forces — and this is the valuable part, not the refactor:**
does an instance property **shadow** a type property of the same name, or is a
same-named property in both tiers a data-quality *finding*? Shadowing is the
conventional Revit reading and is what `tiers()` above implements. But
[Entities](STRATEGY-ENTITIES.md) Decision 4 keeps the tiers separate precisely
because "this leaf is 820 wide" and "every door of this type is 820 wide" are
different claims — and silently collapsing them is what that decision exists to
prevent. Resolve before doors' contract is final, not after.

**Also in scope:** `PropertyPresence`'s `Absent`/`Empty` distinction has to
survive tiering — "absent from both tiers" and "empty in the instance tier" are
different findings.

**Effort:** medium-low as a refactor (mechanical once the trait exists);
the precedence decision is the real work.
**Unblocks:** R4, door reference joins, door validation, door `?filter=`.

## R3 — the TOML ordering footgun is documented, not designed out — **DONE**

> **Outcome: the hazard does not currently exist.** Measuring it before building
> the guard turned out to be the whole value of this item.
>
> `toml 0.8`'s serializer emits **all** value-typed fields before any table or
> array-of-tables, *regardless of declaration order* — verified against both
> write paths and against `to_string` as well as `to_string_pretty`. A struct
> declared in deliberately the wrong order still round-trips correctly. So the
> discipline this codebase has been carefully following, and the two bug reports
> behind it, describe a hazard a dependency upgrade removed at some point
> without anyone noticing.
>
> Shipped anyway, in changed form:
> `test_toml_serializer_hoists_values_above_tables` declares a struct in the
> wrong order and asserts the values still land top level — so it fails the day
> a `toml` upgrade, a switch to `toml_edit`, or a hand-rolled writer restores
> source-order emission, *before* settings files start being written wrong.
> `test_project_manifest_scalars_stay_top_level` covers `project.toml`, the
> other TOML document this server writes.
> [Conventions](CODING-CONVENTIONS.md) now says the rule is belt-and-braces
> rather than a live hazard, and points at the guard.
>
> The rejected alternative below (a hand-built ordered `toml::Table`) is now
> *doubly* rejected: it would add a second representation to keep in step in
> order to solve a problem the dependency already solves.

**What follows is the original analysis, kept because the reasoning still
applies if the guard ever fires.**


**The problem.** serde emits struct fields in declaration order, and a scalar
declared *after* a map or sub-table lands inside that table rather than the
parent. [Conventions](CODING-CONVENTIONS.md) documents this and it has bitten
twice (`Milestone` has two map fields, so every scalar must precede both). A
correct settings file becomes a corrupt one on a field reorder, silently, and
only through the save path — a reviewer sees a harmless-looking diff.

**Proposal: assert the emitted shape, per settings struct.**
`settings_api::tests::test_settings_toml_round_trip` already round-trips the
whole `Settings`, which is why the existing bugs were caught at all — but it
asserts values, not *placement*, and a reorder that nests a scalar can still
round-trip through the same document. Add, per struct carrying both scalars and
collections: serialize a fully-populated instance, parse it back as a
`toml::Table`, and assert every scalar key is present **at the top level of its
own table** rather than swallowed by a sibling.

Rejected alternative: serializing through a hand-built ordered `toml::Table`.
It removes the class outright, but it is materially more code and a second
representation to keep in step with the structs — worse than a test that fails
loudly.

**Effort:** low. **Unblocks:** nothing — it is pure risk reduction.
**Do this one regardless of whether doors ever happen.** It is the only item
here whose value does not depend on a second entity, and it protects R4, which
adds a scalar (`entity`) to two structs that already carry collections.

## R4 — reference sources are implicitly "for rooms"

**The problem.** `[sources.reference.<name>]` and `[[builtin_properties]]` both
mean "for rooms" with nothing saying so. Adding `[sources.reference.doors]`
today parses, loads, and silently no-ops — configured and joined nowhere. The
test corpus already names a source `doors`
([bootstrap.rs](../src/bootstrap.rs), [service/rooms.rs](../src/service/rooms.rs)),
which will read as wrong the day a doors *entity* exists.

**Proposal**, per [Entities](STRATEGY-ENTITIES.md) Decision 5:

- both gain an optional `entity`, defaulting to `"rooms"`, so every existing
  settings file is unchanged and still means what it meant;
- an unknown entity is a **loud startup failure** naming it, per "loud startup
  over silent no-op" — this is what retires the silent no-op above;
- the join namespace stays **flat** (`hardware.FireRating`, not
  `doors.hardware.FireRating`): it exists to answer "what goes before the dot in
  a filter", and nesting forks that grammar for no gain. Source names are
  therefore unique across entities, and a duplicate is a startup error.

**Mind R3 when doing this:** `entity` is a scalar and must be declared *before*
`fields` on `ReferenceSourceConfig` and *before* `by_source` on
`BuiltinPropertyDef`, or the save path nests it.

**Rename the `doors` test fixtures to `hardware`** in the same change.

**Effort:** low-medium. **Blocked by R2** — entity-scoping a join that cannot
target a non-room entity is a config field with no behaviour behind it.

## Sequencing

| | Item | Effort | Do it when |
| --- | --- | --- | --- |
| 1 | ~~**R3** TOML shape assertions~~ | low | **done** — hazard measured away, guard shipped |
| 2 | **R2** tiered property trait | medium-low | **before doors' contract is final** |
| 3 | **R1** `SnapshotKind` store | medium | **with doors**; parallel to R2 |
| 4 | **R4** entity-scoped settings | low-medium | with doors' first reference source |

R1 and R2 are independent of each other and can run in parallel; both are
prerequisites for doors. R4 depends on R2. R3 depends on nothing.

## What is deliberately not here

- **Splitting `contract.rs` into `contract/`.** Real, but driven by the door
  types rather than by any of the above — see
  [Conventions](CODING-CONVENTIONS.md)' measured-module note.
- **`settings/mod.rs` at 826 lines**, which that same note calls the one that
  "reads as unfinished". A genuine cleanup, unrelated to entity generalisation.
- **Anything about doors' own shape.** [Entities](STRATEGY-ENTITIES.md)
  Decisions 4 and 6 own that, including the unresolved question of which room
  owns a door.
