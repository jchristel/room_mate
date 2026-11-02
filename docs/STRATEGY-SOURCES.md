# Roommate — Sources

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Server](STRATEGY-SERVER.md) · [Browser](STRATEGY-BROWSER.md) ·
[MCP](STRATEGY-MCP.md) · [Authored](STRATEGY-AUTHORED.md) ·
[Entities](STRATEGY-ENTITIES.md) · [Security](STRATEGY-SECURITY.md)

Everything that supplies raw data into the pipeline: the Revit/pyRevit producer,
and reference sources (external data joined onto an entity; dRofus is the one
most projects configure, but the pipeline is keyed on N of them).

**Open work only.** The producer, the loader, the join, the two-header CSV
format and the entity-scoped source model all ship, and each carries its
rationale where it is built — `src/reference.rs`, `src/service/rooms.rs`,
`src/settings/`, and `extractor/pyRevit/`. What follows is what is *not* built,
plus the two rules that govern the next source rather than the current ones.

## Deferred

- **An `Api` reference origin.** `ReferenceOrigin` is a tagged enum with one
  live variant, `Upload`. A polled API source — dRofus queried live rather than
  exported to CSV — slots in as a second variant with no other consumer touched.
  That possibility is why the loader is byte-source-agnostic and why the join
  sits at response assembly rather than at load: a live poll changes *when* the
  data arrives, and nothing else. It is also the case the separate `reference`
  sub-object was shaped for — fusing a source into `properties` would couple two
  things with different refresh lifecycles into one bag.

- **A second producer.** Everything about name resolution exists because sources
  vary — `model.source` on the envelope, the `[[builtin_properties]]`
  `by_source` map, the fallback to matching a name verbatim. None of it has a
  second producer yet, so none of it has been exercised against one. IFC is the
  motivating candidate, and the trap it is built for is in "What a second source
  breaks" below.

- **Incremental extraction.** See "Extraction is the dominant cost" below. Only
  worth attacking if near-live updates while modelling are wanted.

## Extraction is the dominant cost, and that decides the optimization axis

Measured: ~840 rooms exported in ~11 s, about 13 ms/room — normal-to-good for
Revit boundary extraction, and almost entirely Revit API time, single-threaded on
Revit's main thread because it must be. Serialization, POST and server storage
are milliseconds against it.

So **the real optimization axis for the slow side is extracting less, or
incrementally** — fewer parameters, skipping rooms that cannot matter, pulling
only what changed since the last snapshot (the snapshot hierarchy leaves that
door open). It is *not* server-side speed or language choice. The phase filter is
the shipped instance of this rule: it is strictly less extraction.

This is the source-side half of [Index](STRATEGY.md)'s "measure where the seconds
actually go", and it is the reason a "just compute one thing here, the data is
right there" change to the extractor is nearly always the wrong trade.

## What a second source breaks, and why the fix is settings rather than types

A typed struct of built-in properties made sense while Revit's Room schema was
the only schema: Revit guarantees a fixed set of built-in parameters on every
Room, so a non-`Option` typed field was a correct model, not merely a convenient
one. **That guarantee is not transferable.** IFC property sets are optional and
exporter-dependent — the same concept, say area, can live in
`Pset_SpaceCommon.NetFloorArea` from one tool, be named differently by another,
or be absent entirely. "Guaranteed present" stops being true even for what feels
like a core field.

Hence the shape the wire and the settings already have: one flat, source-native
property map, with reconciliation pushed to a per-source name table in settings
rather than into Rust types. **Adding a source is a settings-file change — a new
`by_source` entry per canonical property — not a new struct field and not a
recompile.** The tradeoff is real and worth restating so nobody tries to undo it:
a compile-time guarantee was traded for a runtime-resolved name, but that
guarantee was never something a second source could promise, so keeping it in the
type system was enforcing a fiction.

The same rule governs a new *reference* source: the extension point is one line
of settings. The recognized-namespace set is computed from the settings files at
load time rather than compiled in, and the namespace is reserved in the filter
grammar rather than inferred — an unknown prefix is a parse error naming every
known source, never a silent fallback to a room property, so a raw property
literally named `Newsource.Field` cannot quietly change meaning the day that
source is added.

## Two joins that report rather than fail

Both are built; they are here because they are *states a reader will meet* and
will otherwise mistake for bugs.

- **Room ↔ level.** Each room's `level_id` must match an `id` in the level
  export. A mismatch surfaces as rooms landing on a fallback level named by raw
  id — a signal that the two collectors saw different model state, not an error.
- **An unmatched reference key.** A room with no link value simply gets no
  joined data; a key present on the room but absent from the source's map is a
  useful mismatch of the same kind. Consistently with that, a room whose link
  value matched nothing fails **every** predicate on that source, negative
  operators included: "no value" is not evidence that the value differs.
