# Handover — Milestone comparison: support joined sources

> **Status (2026-07-23): steps 1–3 and 5 have LANDED** (see
> PLAN-handover-actioning.md P1); STRATEGY-SERVER.md and STRATEGY-SOURCES.md
> absorbed the outcome. Two placement decisions differ from the text below,
> both deliberate: the resolver lives in `service/rooms.rs` (not
> `contract.rs` — it takes `RoomResponse`, and `contract` must not depend on
> `service`), and the settings-side namespace check runs in
> `bootstrap::load_project_bundle` via `rooms::validate_comparison_field`
> (not `load_settings` — same dependency-direction reason), which the
> save path re-runs (verified by test, message shared via
> `rooms::unknown_source_message` rather than reused verbatim — the `filter
> …:` prefix belongs to the filter surface only).
> **Step 6 also LANDED** (plan item P4): `comparison.html`'s datalist reads
> the `drofus_labels` set P2 put on `/rooms` — the authoritative column list,
> not a mirror of `settings.html`'s `drofusLabels`.
> **Still open:** step 4 only (source-aware `values_agree` — TODO recorded at
> the function, plan item P9).

**Goal.** Let `comparison_key` and `comparison_properties` name fields from
joined sources (`drofus.NetArea`) as well as Revit room properties (`Area`),
using the namespace vocabulary that already exists for filter predicates.

**Shape of the work.** This is a *routing* change, not new comparison logic.
Steps 1–2 are mechanical; step 3 is the one real design decision; steps 4–6
close the loop on correctness and discoverability. Steps 1–3 land together or
not at all (step 3 exists to stop step 2 producing noise). Steps 4–6 are
independently landable follow-ups.

Read `CODING-CONVENTIONS.md` first. Two rules bind almost every decision here:
**"Signal, not error"** (§54) and **"Loud startup over silent no-op"** (§65) —
this change moves one specific failure from the wrong side of that line to the
right one. Annotate the *why*, not the what (§91).

---

## Background: the current gap

`service/comparison.rs` reads only Revit room properties. Both
`index_by_key` (~line 149) and `diff_room` (~line 197) call
`lookup_property` / `property_presence` directly against `rr.room`:

```rust
// index_by_key
if let Some(value) = lookup_property(&rr.room, key_prop, &rr.source, builtin) {

// diff_room
let PropertyPresence::Present(baseline_value) =
    property_presence(&baseline.room, property, &baseline.source, builtin)
else {
    continue;   // <-- a `drofus.`-qualified name lands here, silently
};
```

So a `comparison_properties` entry of `drofus.NetArea` resolves to nothing and
is skipped as "not `Present` on the baseline". **Silent no-op, not an error** —
the user sees an empty diff that is indistinguishable from "no changes".

Meanwhile `service/rooms.rs` already solved exactly this. `resolve_field`
(~line 310) owns the `source.property` vocabulary, `JOINED_SOURCES`
(~line 150) is the single list of legal namespaces, and `Predicate::parse`
(~line 252) owns the split rule. Comparison is the **second consumer** that
needs this and currently duplicates half of it without the joined arm.

Note the data is already present and already correct per-milestone: a milestone
can pin a `drofus_snapshot`, and `assemble_rooms` hands back the right joined
record on `RoomResponse.drofus`. We are not fetching anything new — only
reading what is already on the struct.

**Why it's worth doing:** dRofus drift between milestones (a room's programmed
NetArea changed between issues) is invisible today, and is arguably the more
interesting diff than Revit-vs-Revit.

---

## Step 1 — Extract the namespace resolver into `contract.rs`

Add a shared, presence-returning resolver so the namespace vocabulary has one
home. Signature:

```rust
pub fn resolve_presence(
    room: &RoomResponse,
    field: &str,
    builtin: &[BuiltinPropertyDef],
) -> (Option<&'static str>, PropertyPresence)
```

Returning the resolved namespace alongside the presence is what makes step 4
possible; a bare `PropertyPresence` would force the caller to re-split the
string.

**Placement decision (make this call explicitly, don't default):**
`contract.rs` already owns `property_presence` / `lookup_property` /
`numeric_match`, so it is the natural home. But it currently knows nothing of
`RoomResponse`, which lives in `service::rooms` — check the dependency
direction before committing (§43: `service/` is transport-agnostic; this is a
*within-service* dependency, so it's likely fine, but confirm there's no
import cycle). If `contract.rs` can't see `RoomResponse` cleanly, put the
resolver in `service/rooms.rs` next to `resolve_field` and have
`comparison.rs` import it from there. Either way **one** function owns the
vocabulary — that is the non-negotiable part.

`rooms::resolve_field` then becomes a thin wrapper that collapses the presence
to `Option<String>`, exactly as it does today (absent *and* empty → `None` —
preserve that, it's what makes "a room missing the field never matches" fall
out of `RoomFilter::matches` for every operator without per-operator
special-casing; see the note at `rooms.rs:306`).

Reuse `Predicate::parse`'s split rule verbatim, including the subtlety at
`rooms.rs:262`: **a dot inside a name containing spaces stays part of the
property name**, because a raw Revit property name is far likelier to contain
a dot than to be an attempted namespace. Do not re-derive this rule — factor
it out or call it.

Sketch (annotate to this density — it is the house style):

```rust
/// Resolve one comparable/filterable field against an assembled room, in the
/// `source.property` vocabulary shared with `rooms::resolve_field`. "What can I
/// write before the dot" must have one answer across filtering, comparison, and
/// settings validation, or a name that filters correctly will silently diff as
/// nothing — which is the bug this function exists to close.
///
/// Returns the resolved namespace alongside the presence so callers can vary
/// comparison semantics per source (see `values_agree`) without re-splitting.
pub fn resolve_presence(
    room: &RoomResponse,
    field: &str,
    builtin: &[BuiltinPropertyDef],
) -> (Option<&'static str>, PropertyPresence) {
    match split_namespace(field) {
        // Unqualified: a Revit room property, resolved canonically against the
        // room's own source — unchanged from the pre-joined-source behaviour.
        None => (None, property_presence(&room.room, field, &room.source, builtin)),

        Some(("drofus", label)) => match room.drofus.as_ref() {
            // No joined record at all. Deliberately `Absent`, not `Empty`: the
            // caller collapses this into one per-room "unjoined source" note
            // instead of N identical missing-property rows (see step 3).
            None => (Some("drofus"), PropertyPresence::Absent),
            // Joined: the source's own field labels, verbatim — no canonical
            // mapping, since those labels are dRofus's vocabulary, not Revit's.
            // Matches how `resolve_field` reads them.
            Some(d) => (Some("drofus"), match d.fields.get(label) {
                None => PropertyPresence::Absent,
                Some(v) if v.trim().is_empty() => PropertyPresence::Empty,
                Some(v) => PropertyPresence::Present(v.clone()),
            }),
        },

        // Rejected at settings load (step 5), so unreachable at read time. Kept
        // total so a source added to JOINED_SOURCES but not here degrades to
        // "nothing to compare" rather than panicking mid-request — the same
        // discipline as `resolve_field`'s catch-all arm.
        Some(_) => (None, PropertyPresence::Absent),
    }
}
```

---

## Step 2 — Route `comparison.rs` through it

- `index_by_key`: replace the `lookup_property` call with `resolve_presence`,
  taking only `Present` as a usable key value. Collapsing `Absent`/`Empty`
  together here preserves today's behaviour (a room with no key value is
  dropped — there is nothing to diff it against).
- `diff_room`: replace both `property_presence` calls. The
  "only properties `Present` on the baseline are comparable" rule is unchanged.

Both then accept `drofus.NetArea` for free. `MissingProperty` gains a real
meaning for joined sources: the room had no dRofus match on the other side.

A dRofus-valued `comparison_key` now also works — worth a test, since matching
rooms across milestones by their dRofus id is a plausible thing to want.

---

## Step 3 — Distinguish "source unmatched" from "property absent"

**This is the design decision; the rest is mechanical.**

A room whose dRofus join failed entirely would otherwise report *every*
configured dRofus property as individually missing — one fact reported N
times, which buries the actual signal.

Recommended: add `unjoined_sources: Vec<String>` to `ChangedRoom`, populate it
when `resolve_presence` reports `Absent` with a `Some(ns)` namespace *and* the
room's joined record for that source is `None`, and suppress the per-property
`MissingProperty` rows for that source. A room that is otherwise unchanged but
has lost its dRofus join **should still appear** in `changed_rooms` — losing
the join is a reportable change. Adjust the
`differences.is_empty() && missing_properties.is_empty()` early-return at
`comparison.rs:237` accordingly, or it will be filtered out.

Alternative if that proves awkward: a third `PropertyPresence`-like variant
local to comparison (`SourceUnmatched`). Prefer the `ChangedRoom` field —
widening the shared `PropertyPresence` enum forces every existing match arm in
`contract.rs`/`validation.rs` to grow a case for a state they can't produce.

Whichever you pick, update `comparison.html`'s renderer to show it — an
unjoined source rendered as nothing is the same silent no-op in a new place.

---

## Step 4 — Make `values_agree` source-aware

`comparison.rs:184` deliberately drops the date and ASCII-narrowing rungs, on
the stated grounds that "both sides came through the same export, so any such
artefact is symmetric and cancels."

That reasoning is **true for Revit fields and false for dRofus ones**, where
CSV export artefacts reappear on both sides of the comparison. Take the
resolved namespace from step 1:

- `None` → today's strict two-rung comparator (`numeric_match`, then trimmed
  string). Unchanged.
- `Some("drofus")` → the fuller ladder, including ASCII-narrowing.

**Constraint to be aware of before you start:** `validation::field_values_agree`
(~line 273) is **private and asymmetric** — it takes `(drofus_value,
room_value, field_cfg)` and narrows only the dRofus side, because it compares
dRofus *against* Revit. Milestone comparison is dRofus-vs-dRofus, so it is
**not** directly reusable. Options, in preference order:

1. Extract the symmetric rungs (`ascii_narrowed`, `date_match`, `numeric_match`)
   into a shared comparator in `contract.rs` that both callers configure — best
   long-term, most work.
2. Add a symmetric sibling in `contract.rs` that narrows *both* sides, leaving
   `field_values_agree` alone — smaller, some duplication.
3. Defer step 4 entirely and ship 1–3 + 5–6, with a `TODO` and a test asserting
   current behaviour.

Any of these is defensible; **(3) is a legitimate ship** if the extraction
turns out to be more invasive than it looks. Do not silently reuse the
asymmetric function.

Note also that the date rung needs a `DrofusFieldConfig` to know a field is a
date. That config is per-project, reachable from the bundle — thread it or
skip the date rung and say so in a comment.

---

## Step 5 — Validate at settings load, not silently at read

There is **no validation of `comparison_key` / `comparison_properties`
anywhere today** (confirmed: `validate.rs`, `load.rs`, `settings_api.rs` all
have zero references). A typo'd `drofus.NetAra` currently yields an empty diff
that looks exactly like "no changes".

Per §65 ("Loud startup over silent no-op"), reject at load: a namespace not in
`JOINED_SOURCES`, or an empty property after the dot. Reuse `Predicate::parse`'s
error text verbatim so the two surfaces can't drift:

```
unknown data source "drofuss" — known sources: drofus
```

An **unqualified** name must stay unvalidated — it's a free-text room property
that may legitimately not exist on any currently-loaded room (an empty store
still boots). Only the namespace is checkable at load time.

Because the settings-save path re-runs `bootstrap::load_project_bundle`
verbatim (§69), adding this to load-time validation gets the API `422` for
free — no separate check in `settings_api.rs`. **Verify that**, don't assume
it; add a save-path test asserting the bad namespace is rejected with the
right message.

⚠️ **Migration risk:** any existing `*.toml` in the wild with a malformed
comparison field will now **fail the boot** rather than silently no-op. That is
the intended direction, but check the repo's own settings fixtures
(`server.toml`, `showcase.toml`) and any deployed configs before landing, and
call it out in the commit message.

---

## Step 6 — Echo the vocabulary to the client

`comparison.html:254` (`loadPropertyKeys`) builds the `propertyOptions`
datalist from the union of `room.properties` keys only — so joined fields are
undiscoverable even once they work.

Add `drofus.`-prefixed entries. `settings.html` already keeps a `drofusLabels`
array from the last drofus-check/upload; mirror that. Both the key input
(`#keyInput`) and the property input (`#propInput`) share this datalist, so one
change covers both. Free-text entry must keep working when the list is empty —
preserve the existing `catch { keys = [] }` fallback.

---

## Testing

Inline `#[cfg(test)] mod tests` at the bottom of `comparison.rs` (§21). The
existing tests use `FsStore` because milestone pins address snapshot history —
keep that. `make_bundle` currently hardcodes `drofus: None` (deliberately,
to prove comparison stands alone without dRofus); you'll need a variant that
attaches dRofus + a `drofus_snapshot` pin. **Keep at least one existing
no-dRofus test passing untouched** — that property is the point of the
original design and must not regress.

Cover:
- `drofus.`-qualified property differs between milestones → reported.
- dRofus pinned per-milestone → diff reflects the *pinned* snapshots, not
  current. (`rooms.rs:1610` has a worked example of this setup.)
- Room unjoined on one side → one `unjoined_sources` entry, **not** N
  `MissingProperty` rows; room still appears in `changed_rooms`.
- `drofus.`-qualified `comparison_key` matches rooms across milestones.
- Unqualified properties behave exactly as before (regression guard).
- Bad namespace in settings → rejected at load *and* on save, with the
  `Predicate::parse` message.
- If step 4 lands: an ASCII-narrowing artefact on a dRofus field is not a
  difference, while a genuine mismatch still is. (`validation.rs:700` and
  `:720` are the paired precedents — mirror both, since the second is what
  stops narrowing from masking real changes.)

---

## Definition of done

- `drofus.NetArea` works in both `comparison_key` and `comparison_properties`.
- A malformed namespace fails loudly at boot and at save, not silently at read.
- The namespace vocabulary has exactly **one** definition; adding a future
  source is one `JOINED_SOURCES` entry + one match arm, as `rooms.rs:148`
  already promises.
- No regression for projects with no dRofus configured.
- Module headers updated: `comparison.rs`'s header currently asserts dRofus is
  "irrelevant here" (lines 12–13, 20–24). That claim is now wrong for
  properties but **still right for the fallback rule** — comparison must never
  silently fall back to the dRofus `link_property` when no `comparison_key` is
  set. Rewrite carefully; don't delete the paragraph wholesale.
