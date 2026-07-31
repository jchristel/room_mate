# Handover: generalizing QA across reference sources

> **Superseded by [`HANDOVER-qa-cardinality-and-coverage.md`](../HANDOVER-qa-cardinality-and-coverage.md).**
> Section 3 (per-source response shape) and section 4 (sweep the consumers) are
> **built**. Section 2 (`qa = "none"`) needed nothing — `CompareMode::Ignore`
> already did exactly that under a different name. Only section 1 (link
> cardinality) survives, together with the parenthetical in check 1 about the
> unmatched *direction*, which turned out to be the sharpest observation here
> and is still unimplemented. Both moved to the successor doc.
>
> Kept for the reasoning, not the plan. **The line below saying nothing is
> implemented is no longer true** — treat everything past this banner as a
> record of how the design was arrived at.

Companion to `HANDOVER-reference-sources.md`. That doc covers moving from a single
hardcoded dRofus reference source to a keyed collection of reference sources. **This
doc covers the QA consequence of that change.** Nothing implemented yet — design only.

Project: "Roommate", Rust (axum), Revit → Rust → browser room viewer. QA currently
lives in `validation.rs` and is hardwired to dRofus.

## The good news: the checks already exist

The three QA behaviours the user wants are **already implemented** in `validation.rs`
— they're just written against `drofus` specifically instead of "a reference source".
Generalizing is mostly renaming + looping, not new logic.

Mapping the requested checks to what's already there:

1. **"Do all link keys in secondary data have a match in primary?"**
   → today: unmatched detection via the `LinkValueIndex` (`BTreeMap<String, Vec<(&Room, &str)>>`)
   built by joining on `drofus.link_property`. Keys with no room = unmatched.
   (Note: the existing `rooms_unmatched_in_drofus` is the *reverse* direction — rooms
   with no dRofus record. The requested check is secondary-key → primary, so confirm
   both directions are reported per source.)

2. **"For one-to-one linkage only (vs bucket), any duplicate keys in secondary data?"**
   → today: `duplicate_link_values` — a link value mapping to more than one entry is
   flagged as ambiguous and **excluded** from unmatched/mismatch checks. This is
   exactly the "1:1 vs bucket" distinction. See below — it must become a per-source
   *setting*.

3. **"If secondary specifies fields linked to primary fields, do values match? (with
   scope to exclude linked fields)"**
   → today: `PropertyMismatch` + `field_values_agree` + per-field `CompareMode`
   (`Exact` / date / numeric fallback). The per-field `qa` override already exists.
   "Exclude a linked field" = a field-level opt-out; partly expressible today via the
   field config, but needs an explicit "don't check" mode (see below).

So: the engine is right. The **shape** (singular dRofus, hardcoded) is the problem.

## What has to change

### 1. Cardinality becomes a per-source setting (1:1 vs bucket)

Today duplicate link values are *always* treated as an error-ish exclusion. That
assumes 1:1 linkage. Some sources will legitimately be many-to-one ("bucket") — e.g.
a hardware schedule where many rows map to one door type. Add to each reference
source's settings something like:

```toml
[sources.reference.drofus]
link_cardinality = "one_to_one"   # one_to_one | bucket
[sources.reference.hardware]
link_cardinality = "bucket"
```

- `one_to_one` → run the duplicate-key check (current behaviour).
- `bucket` → duplicates are expected; **skip** the duplicate check, and the
  unmatched/mismatch logic must not assume a unique partner.

### 2. Field-value QA becomes opt-out-able per field

The user wants scope to **exclude** specific linked fields from value matching. The
per-field `CompareMode` already exists; extend it (or add a flag) so a field can be
declared "linked/displayed but not QA-checked":

```toml
[[sources.reference.drofus.fields]]
label = "NetArea"
qa    = "numeric"     # existing modes: exact | numeric | date | ...
[[sources.reference.drofus.fields]]
label = "Comments"
qa    = "none"        # NEW: carried through, never value-checked
```

Default when unset stays as today (lossy string match).

### 3. Everything dRofus-named becomes per-source and iterated

`validation.rs` and its response types are dRofus-shaped throughout. These need to
become "one result per reference source", keyed by the source key from settings:

- `drofus_configured: bool` → per-source configured/status map.
- `rooms_unmatched_in_drofus` → per-source, and clarify direction (see check 1).
- `duplicate_link_values`, `property_mismatches` → computed per source, gated by that
  source's `link_cardinality` and per-field `qa` settings.
- `PropertyMismatch.drofus_id` / `drofus_value` → source-neutral names
  (`reference_id` / `reference_value`) or nested under a source key.
- `compute_validation` / `compute_project_validation` → loop over configured
  reference sources; today they take a single `ReferenceData`.
- Doc comment at top of file (`//! dRofus reconciliation QA…`) → generalize wording.

### 4. Response contract / consumers

The MCP layer (`mcp.rs`) and comparison (`comparison.rs`) also touch QA and will read
the reshaped response. Whoever implements must sweep those + the browser panel that
renders category counts (the counts are **list lengths** — duplicate counts as
*groups*, not rooms; preserve that semantics per source).

## The entity boundary still applies

Same caveat as the reference-sources handover: this generalizes QA across **multiple
reference sources joined onto rooms**. A door schedule joins onto *doors*, which don't
exist as an entity yet — so door QA is downstream of the doors-entity work, not of
this change. Don't let `[sources.reference.doors]` look QA-ready when nothing joins
door-keyed data. Note it in `STRATEGY-SOURCES.md` alongside the sources boundary.

## Constraints for whoever implements

- Rust project — **add plenty of annotation/comments** to the code.
- Keep answers short and concise.
- Update the matching STRATEGY doc for any layer touched (validation/QA is documented
  where sources are; keep the "checks list" and the per-source settings in sync).

## Suggested implementation order (not started)

1. Settings: add `link_cardinality` per reference source + `qa = "none"` field mode.
2. `validation.rs`: parameterize the existing checks by a single reference source
   (rename dRofus-specific identifiers to source-neutral), driven by settings.
3. Wrap in a loop over all configured reference sources; response becomes per-source.
4. Sweep `mcp.rs`, `comparison.rs`, and the browser panel for the reshaped contract.
5. Update `STRATEGY-SOURCES.md`: the three-check QA definition + cardinality/opt-out
   settings + the doors-entity boundary.
