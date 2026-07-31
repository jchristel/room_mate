# Handover: link cardinality

> **Item 2 (the unmatched direction) is done.** `SourceValidation` now carries
> `reference_unmatched` — the source's link values no room resolves to —
> alongside `rooms_unmatched`, with its own `DiscrepancyCounts` entry, a QA
> band section, and a CSV row shape that fills only the link-value column
> (there is no room). A duplicated link value counts as *matched* and stays
> reported once as a duplicate, so the two directions never double-count one
> problem. **Only item 1, link cardinality, remains open.**
>
> Item 2's write-up is kept below as the reasoning behind what was built.

One open item, about **what the QA report asks**, not how it is shaped.

Supersedes `Superseded/HANDOVER-qa-generalization.md`, which proposed three
changes. Two of them are done or were never needed:

- *"Everything dRofus-named becomes per-source and iterated"* — **done**.
  `ValidationResponse` is `sources: BTreeMap<String, SourceValidation>`, each
  entry carrying its own `link_property`, discrepancy lists, `field_coverage`
  and `error_rooms`, plus a cross-source `discrepancies` total. `mcp.rs`,
  `comparison.rs` and the browser QA band all read that shape.
- *"Field-value QA becomes opt-out-able per field (`qa = "none"`)"* — **already
  existed** as `CompareMode::Ignore` (`qa = "ignore"`), which skips the value
  check and drops the field from `field_coverage` so it is not reported as a
  coverage gap. Same semantics, different name.

What follows is what actually remains.

## 1. Duplicate link values assume 1:1, and that is not configurable

`service/validation.rs`, in `compute_validation`:

```rust
for (value, rooms) in &by_value {
    if rooms.len() > 1 {
        duplicate_link_values.push(/* … */);
        continue; // ambiguous -- can't uniquely match, so no further checks
    }
```

A link value shared by several rooms is unconditionally treated as ambiguous:
reported, then **excluded** from the unmatched and mismatch checks. That is
right for dRofus, where one room has one record. It is wrong for a source that
is legitimately many-to-one — a hardware schedule where many rows describe one
door type — where every row would be reported as a defect and then skipped,
producing a report that is both noisy and silently incomplete.

Make it a per-source declaration on `ReferenceSourceConfig`
(`settings/mod.rs`), beside `fields`:

```toml
[sources.reference.drofus]
type = "upload"
link_cardinality = "one_to_one"   # default; today's behaviour

[sources.reference.hardware]
type = "upload"
link_cardinality = "bucket"
```

- `one_to_one` — current behaviour exactly. Must be the serde default so every
  existing project file keeps its present meaning with no edit.
- `bucket` — duplicates are expected: skip the duplicate check entirely, and do
  not let the mismatch check assume `rooms[0]` is *the* partner. Decide
  deliberately what a value comparison even means here — comparing a room
  against N records is a different question, and "compare against each, report
  per record" is one answer but not the only one. Whoever implements should
  write the chosen rule down rather than let it fall out of the loop shape.

## 2. Unmatched is only checked in one direction — **DONE**

`compute_validation` walks the *rooms*' resolved link values and looks each one
up in the source:

```rust
let Some(record) = reference.by_id.get(value) else {
    rooms_unmatched.push(room.id.clone());
    continue;
};
```

So `rooms_unmatched` answers **"which rooms have no record?"**. Nothing anywhere
iterates `reference.by_id` to ask the reverse: **"which records have no room?"**
`by_id` is only ever read through `.get(...)`.

That is a real reporting gap, and it fails quietly in the worst direction: a CSV
of 200 rows joined against 50 rooms reports **zero** unmatched and reads as
clean. The whole point of the QA pass is noticing that the two sides disagree,
and half of the disagreement is currently invisible.

Add a second list to `SourceValidation` — `reference_unmatched` or similar —
holding the link values present in the source with no room resolving to them.
Cheap to compute: the room-side pass already builds `by_value`, so it is
`reference.by_id.keys()` minus those keys.

Points to settle while implementing:

- **It counts toward `DiscrepancyCounts`**, which means a new field there and in
  the `add()` accumulator, and a new section in the browser band and the CSV
  export. Follow the existing per-category shape — counts are list lengths.
- **`error_rooms` does not apply.** Its entries are keyed by room id and there
  is no room here. The natural detail for an unmatched record is a few of its
  own field values; decide whether that is worth carrying or whether the bare
  key is enough. Bare key is the smaller change and probably sufficient.
- **Interacts with item 1.** Under `bucket`, "records with no room" is still a
  meaningful question; under either cardinality the direction is worth
  reporting.

**As built**, all three points above were settled the way this note suggested:
`reference_unmatched: Vec<String>` on `SourceValidation`, a matching
`DiscrepancyCounts` field summed by `add()`, bare link values with no
per-record detail, and no `error_rooms` involvement. The band section carries
no `data-room`, so these are not jump targets — there is nothing to jump to.

## Still true: the entity boundary

Unchanged from the superseded doc, and worth restating because it bounds the
cardinality item above. This generalizes QA across reference sources joined
onto **rooms**.
A door schedule joins onto *doors*, which are not an entity in this pipeline —
the extractor collects no door elements. So `[sources.reference.doors]` must not
be made to *look* QA-ready when nothing joins door-keyed data; door QA is
downstream of the doors-entity work, not of this change. `bucket` cardinality is
motivated by a door-schedule-shaped source, so it is easy to conflate the two —
they are separate pieces of work.

## Conventions for whoever implements

- Annotate the *why*, per `CODING-CONVENTIONS.md` — especially the cardinality
  rule, which is a judgement call a future reader cannot recover from the code.
- Update `STRATEGY-SOURCES.md` in the same change: the QA checks it documents
  and the per-source settings list both move.
- Tests live inline. `service/validation.rs` already has multi-source cases to
  extend (`test_every_configured_source_gets_its_own_report`).
