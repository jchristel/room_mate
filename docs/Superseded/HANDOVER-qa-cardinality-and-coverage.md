# Handover: QA coverage of the secondary source — closed

> **Closed. Nothing here is outstanding.** Kept as the record of a
> mis-diagnosis worth not repeating, and of what the QA pass now checks.

Successor to `HANDOVER-qa-generalization.md`, which is also superseded. Between
them these two documents proposed five things. Three were already built or
never needed, one was real, and **one was framed against the wrong side of the
join** — which is the part worth reading.

## What the QA pass checks, as built

Two sides, and the questions are not symmetric.

**Primary (the rooms):**

| Check | Field |
|---|---|
| rooms sharing one link id | `duplicate_link_values` |
| rooms with no link value set | `rooms_missing_link_value` |
| rooms whose id finds no record | `rooms_unmatched` |

**Secondary (the reference CSV):**

| Check | Field |
|---|---|
| records no room reaches | `reference_unmatched` |
| ids used by more than one row | `reference_duplicate_ids` |
| rows with no id at all | `reference_blank_id_rows` |

Plus per-field value comparison (`property_mismatches`, `fields_absent_in_revit`,
`fields_empty_in_revit`) and `field_coverage`. All of it per source, keyed by
source name, with a cross-source `discrepancies` total.

## The mis-diagnosis

Both earlier drafts framed the open work as **link cardinality**: that
`compute_validation` treats a duplicated link value as ambiguous, hardcoding a
1:1 assumption, and that a many-to-one "bucket" source therefore needed a
per-source `link_cardinality` setting. The design question that followed —
*"under `bucket`, what does comparing one room against N records even mean?"* —
was posed as needing a decision before anything could be built.

That was wrong, and the reason is worth keeping:

```rust
// resolve_link_values: iterates ROOMS, groups them by resolved link value
for room in &payload.rooms { by_value.entry(value).or_default().push(...) }
// compute_validation:
if rooms.len() > 1 { /* duplicate_link_values */ }
```

`rooms.len() > 1` counts **rooms** sharing an id — a duplicate in the *primary*
data, which is exactly right and not a cardinality assumption about the source
at all. Meanwhile the secondary side could never present N records for one key,
because the loader collapsed them first:

```rust
by_id.insert(id, ReferenceRecord { fields });   // repeated id: silently overwrites
_ => continue,                                  // blank id: silently skipped
```

So there was no policy question to answer. There was **silent data loss**: rows
discarded at load, with `record_count` already reporting the deduplicated
number, so nothing downstream could see it. Demonstrated on a running server —
a 4-row CSV with three rows sharing id `1` and one blank-id row loaded as *one*
record, the survivor being whichever row happened to sit last in the file, and
the QA report said nothing.

The lesson: **the two sides of a join are different questions, and a check
named after the join tells you nothing about which side it inspects.** Both
drafts read `duplicate_link_values` as "the source has duplicates" when it
means "the rooms do".

## What was built instead

`ReferenceData` now carries `duplicate_ids` and `blank_id_rows`, populated by
the loader — the only place that can see the loss, since it is the thing
causing it. They surface in three places, because the mistake is worth catching
at different moments:

- **The upload response**, so the operator sees "1 record, 1 repeated id" while
  still looking at the file they picked.
- **`tracing::warn!`**, because the loader also runs at boot, where no HTTP
  response exists.
- **The QA report** (`reference_duplicate_ids`, `reference_blank_id_rows`,
  counted into `discrepancies`), for the same reason every other check is
  there.

Last-write-wins is deliberately kept. There is no better arbitrary winner, and
the fix is to *report* the arbitrariness, not to pick differently.

## If bucket sources ever arrive

The cardinality question is now well-posed rather than open: a repeated id in a
secondary source is currently **reported as a defect**, and a genuinely
many-to-one source would want it treated as normal. That is a real future
setting (`link_cardinality = "bucket"`), but it needs a motivating source
first, and it would build on the reporting above rather than replace it.

Note the boundary that makes this less urgent than it looks: the motivating
example is always a door or hardware schedule, and those join onto **doors**,
which are not an entity in this pipeline — the extractor collects no door
elements. A source keyed on something rooms do not have cannot be QA'd against
rooms at all, whatever its cardinality. Door QA is downstream of the
doors-entity work, not of this.
