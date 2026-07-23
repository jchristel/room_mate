# Plan — actioning the three open handovers

Review of `HANDOVER-area-label-sizing.md`, `HANDOVER-comparison-sources.md` and
`HANDOVER-ui-layout.md` against the [STRATEGY docs](STRATEGY.md) and the code
they describe, a priority per item by **impact**, and an ordered plan working
highest to lowest.

Every step follows [CODING-CONVENTIONS.md](CODING-CONVENTIONS.md); the rules
that actually bind here are called out per item rather than restated.

---

## Review verdict

All three handovers were checked line-by-line against the source. **Every
factual claim about the code holds** — line references, function names, and the
described behaviours are accurate as of `53df6b3`. Three corrections and one
strategy conflict are recorded below; nothing else needs changing before the
work starts.

### `HANDOVER-area-label-sizing.md` — accurate, ready to paste

Verified: `renderAreasOverlay` sets a flat `baseFont` from `zone.fitted`
([index.html:1473](static/index.html:1473)) and applies it unconditionally
([:1493](static/index.html:1493)); `addLabel` already fits text to the room bbox
([:801-803](static/index.html:801)). The proposed code is correct and
self-contained.

**Conflict with [Browser](STRATEGY-BROWSER.md) — must be reconciled, not
ignored.** That doc records removing an identical `fontSize < baseFont * 0.25`
cutoff from room labels as a *bug fix*: "labels no longer silently disappear on
small rooms — the old cutoff (a floor-wide threshold that dropped a label
outright rather than just shrinking it) is gone … zoom can't recover a dropped
label anyway". The handover reintroduces exactly that threshold for tier labels,
and its own "Known consequence" section restates the same zoom limitation as
acceptable.

The distinction is real and defensible — **a suppressed tier label is still
named in the areas summary panel; a suppressed room label had no other
surface** — but it is not currently written down anywhere. Without it the next
reader will read the threshold as the bug that was already fixed and remove it.
The plan therefore treats the doc update as part of the change, not a follow-up.

Not a defect, worth knowing: `ringBox` duplicates `loopBox`'s job for the
`[[x,y]]` ring shape. That mirrors the existing `ringAreaAbs`/`ringCentroid`
pair, which already duplicate `roomNetArea`/`centroid` for the same reason —
so a second helper is house style here, not drift. Keep it.

### `HANDOVER-comparison-sources.md` — accurate; one open decision resolved, one spec defect

Verified: `index_by_key` calls `lookup_property` directly
([comparison.rs:156](src/service/comparison.rs:156)); `diff_room` calls
`property_presence` twice ([:209](src/service/comparison.rs:209),
[:215](src/service/comparison.rs:215)); `JOINED_SOURCES`
([rooms.rs:150](src/service/rooms.rs:150)), `Predicate::parse`'s split rule
([:252-263](src/service/rooms.rs:252)) and `resolve_field`
([:310](src/service/rooms.rs:310)) are exactly as described. **Confirmed: there
is zero validation of `comparison_key` / `comparison_properties` anywhere** —
`settings/validate.rs`, `settings/load.rs` and `settings_api.rs` have no
references to either field.

**Open decision resolved — put the resolver in `service/rooms.rs`, not
`contract.rs`.** The handover asks for this call to be made explicitly.
`contract.rs` imports exactly one thing, `settings::BuiltinPropertyDef`
([contract.rs:21](src/contract.rs:21)); `RoomResponse` lives in
`service::rooms`. A `&RoomResponse`-taking function in `contract.rs` would make
`contract` depend on `service`, inverting the direction every other module
follows (§43 — "dependency direction is the seam"). Rust would compile it; the
layering would be a lie. `resolve_presence` goes next to `resolve_field`, which
already owns the vocabulary, and `comparison.rs` imports it from there. The
non-negotiable part of the handover — **one** function owning the namespace
vocabulary — is satisfied either way.

**Spec defect in step 5 — the error text cannot be reused verbatim.** The
handover shows the target message as `unknown data source "drofuss" — known
sources: drofus`. The real message
([rooms.rs:255-258](src/service/rooms.rs:255)) is prefixed with the predicate
context: `filter "drofus.NetAra=x": unknown data source …`. A settings-load
failure that says "filter" names the wrong thing. Fix: extract the split rule
into a `split_namespace` helper (which does not exist yet — the handover's
sketch assumes it) that returns a structured result, and let each caller supply
its own context prefix. The *vocabulary* and the *wording* are then shared; the
noun is not.

**Migration risk checked, and it is clear.** The repo's own fixtures use only
unqualified names — `showcase.toml` has `comparison_key = "RoomNumber"` and
`comparison_properties = ["Area", "Department", "SubDepartment"]`;
`sample-project.toml` has an empty list. Nothing in-repo fails the boot under
step 5. Deployed configs outside the repo are still the caller's risk and still
belong in the commit message.

**Sequencing constraint the handover does not state:** step 5 must land *with or
after* steps 1–3, never before. Alone, it would accept `drofus.NetArea` as a
valid namespace while the read path still silently ignored it — validation
saying "fine" over behaviour saying "nothing" is worse than today's single
silence.

### `HANDOVER-ui-layout.md` — accurate; one real correctness bug in step 4

Verified: `.validation-panel` and `.areas-panel` are both children of
`.zone-canvas` ([index.html:252-262](static/index.html:252)); scope pickers are
per-zone in the zone template ([:235-237](static/index.html:235)); `paintLevel`
is a pure painter with an options bag ([:684](static/index.html:684)); and
`buildLevelSvgFile` ([:1273](static/index.html:1273)) calls it directly with its
own options — so the handover's reasoning that a CSS-only labels toggle would be
silently ignored by export is correct, and the `showLabels` option is the right
answer.

**Step 4 is wrong as specified.** It states the dRofus label set is "one label
set per response only because the response is already project-scoped". It is
not. `RoomScope.project` is `Option<&str>`
([rooms.rs:406](src/service/rooms.rs:406)) and the default is "merge every
stored model" — an unscoped `GET /rooms` spans **every project**, with
`effective_drofus` resolved per project inside the merge loop
([:734](src/service/rooms.rs:734)). A flat `all_labels` on `RoomsResult` would
silently mean "some project's labels" in that case.

Fix before building: carry the label set **keyed by project id** — e.g.
`drofus_labels: BTreeMap<String, DrofusLabelSet>` with `all_labels` +
`reconciliation` per entry — so a multi-project merge stays honest and the
project-scoped grid reads one entry. Additive either way; the correct shape
costs nothing extra now and is a breaking reshape later.

**Two things the handover gets right that are worth reinforcing:** selection
persistence already special-cases `zones[0]`
([:1834](static/index.html:1834)), so global scope *removes* that special case
rather than complicating it — step 1's open question resolves in its favour.
And the labels toggle's stated cost ("placing a button in a header whose meaning
is about to change") is near zero: the toggle is header-resident in both the
current and target layout, so it can land early with no rework.

**Cross-handover dependency, not noted in either document:** ui-layout step 4
also serves comparison step 6. `comparison.html`'s `loadPropertyKeys`
([comparison.html:255](static/comparison.html:255)) already fetches `/rooms`;
once the label set is on that response it can build the `drofus.`-prefixed
datalist from the authoritative list instead of mirroring `settings.html`'s
`drofusLabels`. Doing step 4 once serves both handovers — which is why it is
sequenced early here rather than at its handover's position 4.

### Doc hygiene found while reviewing

- **Broken links.** [STRATEGY.md](STRATEGY.md) (twice) and
  [Browser](STRATEGY-BROWSER.md) point at `docs/HANDOVER-georeferencing.md`; the
  file is at `docs/Superseded/HANDOVER-georeferencing.md`.
- **Stale index.** [docs/README.md](README.md) lists
  `settings-infrastructure-handoff.md` under "Implementation notes"; it lives in
  `Superseded/`. The three open handovers are not indexed at all.
- **Undocumented shipped feature.** The header room search (query input, field
  picker, `.match`/`.dim` rendering) is built in `index.html` but has no bullet
  in [Browser](STRATEGY-BROWSER.md)'s Implemented list. The ui-layout target
  header includes search, so this gap must close when that work lands.
- **No conventions coverage for `static/`.**
  [CODING-CONVENTIONS.md](CODING-CONVENTIONS.md) is Rust-only; `index.html` is
  2,020 lines and the restructure will grow it. Not a blocker — flagged so the
  question is asked deliberately rather than answered by accretion.

---

## Priority

Impact means: does a user get a **wrong answer**, a **missing capability**, or a
**worse-looking one** — in that order. Cost is recorded separately because it
changes sequencing within a band, never the band itself.

| # | Item | Source | Impact | Cost | Why this band |
|---|---|---|---|---|---|
| **P1** | Comparison reads joined sources; malformed fields fail loudly | H2 §1–3, §5 | **Critical** | M | The only item where the tool reports something *false*: a `drofus.` property or a typo yields an empty diff indistinguishable from "no changes", in a feature whose whole job is saying what changed |
| **P2** | dRofus label set on `RoomsResult` (per project) | H3 §4 (corrected) | High | S | Small, additive, server-only; closes a real wire gap and unblocks both P4 and H3 §5 |
| **P3** | Labels toggle (`showLabels` through `paintLevel`) | H3 §6 | High | S | The request that triggered the whole restructure; independent, export-correct, no rework cost |
| **P4** | `drofus.`-qualified names discoverable in the comparison UI | H2 §6 | High | S | Without it P1 works but is invisible — a feature nobody can find is close to one that doesn't exist |
| **P5** | Tier label sizing on the areas overlay | H1 | Medium | S | Cosmetic, but complete, zero-risk and ready to paste |
| **P6** | Scope migration — global project/milestone/building | H3 §1 | Medium-High | **L** | Largest structural payoff (deletes the focus model, collapses polling/validation/colour-plan duplication) at the largest risk. Lands alone |
| **P7** | Bottom region shell + migrate validation/areas panels | H3 §2–3 | Medium | M | Frees the right side and gives tabular data a shape that suits it |
| **P8** | Source-data grid (band 2) | H3 §5 | Medium | **L** | Genuine new capability, but the only item with a performance constraint of its own (row windowing) |
| **P9** | Source-aware `values_agree` | H2 §4 | Low-Medium | M | The handover itself rates deferral "a legitimate ship"; affects a narrow class of dRofus-vs-dRofus artefacts |
| **P10** | Doc hygiene: broken links, README index, search bullet | this review | Low | XS | Ten minutes; do it whenever a doc is open anyway |

The inspector (H3 Decision 3) is **not** in this plan: it cannot be built until
room click-selection exists, which nothing here provides. It stays recorded as
reserved space with a known purpose.

---

## The plan

### P1 — Comparison reads joined sources, and malformed fields fail loudly

> **Status: LANDED 2026-07-23.** All eight steps below done as specified, with
> one deviation: the settings-load validation call sits in
> `bootstrap::load_project_bundle` (not `settings/load.rs`) so the vocabulary
> stays owned by `service::rooms` without `settings` importing `service`.
> 179 lib tests pass, including the nine added here.

Handover steps 1, 2, 3 and 5. **Land as one change**: step 3 exists to stop step
2 producing noise, and step 5 alone would validate a namespace the read path
still ignores.

1. **Extract `split_namespace` in `service/rooms.rs`.** Lift the split rule out
   of `Predicate::parse` ([:252-263](src/service/rooms.rs:252)) verbatim,
   including the subtlety that **a dot inside a name containing spaces stays
   part of the property name**. Return a structured result (recognised
   namespace / unknown namespace / unqualified) so each caller writes its own
   context prefix — the filter path keeps `filter {expr:?}: `, the settings path
   gets its own. Do not re-derive the rule anywhere.

2. **Add `resolve_presence` next to `resolve_field`** (placement decision
   resolved above), signature as the handover specifies:
   `(Option<&'static str>, PropertyPresence)`. The namespace comes back
   alongside the presence so P9 never has to re-split. A missing joined record
   returns `Absent` with `Some(ns)` — deliberately, so the caller can collapse
   it (step 3). Keep the catch-all `Some(_) => Absent` arm rather than
   `unreachable!()`, matching `resolve_field`'s existing discipline.

3. **Make `resolve_field` a thin wrapper** collapsing the presence to
   `Option<String>`. **Absent *and* empty must both map to `None`** — that is
   what makes "a room missing the field never matches" fall out of
   `RoomFilter::matches` for every operator without per-operator special-casing
   ([rooms.rs:306](src/service/rooms.rs:306)). A regression here is silent and
   changes filter semantics.

4. **Route `comparison.rs` through it.** `index_by_key`: replace
   `lookup_property`, taking only `Present` as a usable key value (collapsing
   `Absent`/`Empty` preserves today's "no key value → dropped"). `diff_room`:
   replace both `property_presence` calls; the "only properties `Present` on the
   baseline are comparable" rule is unchanged.

5. **Add `unjoined_sources: Vec<String>` to `ChangedRoom`.** Populate when
   `resolve_presence` reports `Absent` under a `Some(ns)` namespace *and* the
   room has no joined record for that source; suppress the per-property
   `MissingProperty` rows for that source. Adjust the
   `differences.is_empty() && missing_properties.is_empty()` early return
   ([comparison.rs:237](src/service/comparison.rs:237)) — **a room that lost its
   dRofus join must still appear in `changed_rooms`**, losing the join is a
   reportable change. Prefer this over widening `PropertyPresence`, which would
   force a new arm on every existing match in `contract.rs`/`validation.rs` for
   a state they cannot produce.

6. **Validate at settings load** (§65). Reject a namespace not in
   `JOINED_SOURCES` and an empty property after the dot, for both
   `comparison_key` and `comparison_properties`. Goes beside the other
   settings-only validators in `settings/validate.rs`, called from
   `load_settings` as `validate_colour_plans` already is
   ([load.rs:109](src/settings/load.rs:109)). **An unqualified name stays
   unvalidated** — it is free-text and may legitimately match no currently
   loaded room; an empty store still boots. Only the namespace is checkable at
   load time.

7. **Verify the save path gets 422 for free**, don't assume it. The settings
   save re-runs `bootstrap::load_project_bundle` verbatim (§69), so it should —
   but the handover explicitly asks for a save-path test asserting the bad
   namespace is rejected with the right message.

8. **Rewrite `comparison.rs`'s module header** (lines 12–13, 20–24). Its claim
   that dRofus is "irrelevant here" is now wrong for *properties* but **still
   right for the fallback rule** — comparison must never silently fall back to
   the dRofus `link_property` when no `comparison_key` is set. Do not delete the
   paragraph wholesale.

**Tests** — inline `#[cfg(test)] mod tests` (§21), `FsStore`-backed as the
existing ones are. `make_bundle` hardcodes `drofus: None` deliberately; add a
*variant* that attaches dRofus + a `drofus_snapshot` pin and **keep at least one
existing no-dRofus test passing untouched** — standing alone without dRofus is
the original design property and must not regress.

- `drofus.`-qualified property differs between milestones → reported.
- dRofus pinned per-milestone → the diff reflects the *pinned* snapshots
  ([rooms.rs:1610](src/service/rooms.rs:1610) has a worked setup).
- Room unjoined on one side → one `unjoined_sources` entry, **not** N
  `MissingProperty` rows; room still appears in `changed_rooms`.
- `drofus.`-qualified `comparison_key` matches rooms across milestones.
- Unqualified properties behave exactly as before (regression guard).
- Bad namespace → rejected at load **and** on save, with the right message.

**Docs:** [Server](STRATEGY-SERVER.md) gains the comparison-reads-joined-sources
fact and the new load-time validation; [Sources](STRATEGY-SOURCES.md)'s "a
joined source is queryable under its `[sources.<name>]` key" bullet gains
comparison as its second consumer. Commit message calls out the migration
direction for out-of-repo configs.

**Done when:** `drofus.NetArea` works in both settings; a malformed namespace
fails at boot and at save, never silently at read; the vocabulary has exactly
one definition, so a future source is one `JOINED_SOURCES` entry plus one match
arm; no regression for dRofus-less projects.

### P2 — dRofus label set on `RoomsResult`, keyed by project

> **Status: LANDED 2026-07-23.** `RoomsResult.drofus_labels` (project id →
> `{all_labels, reconciliation}`), collected inside `assemble_scoped_rooms`
> from the same `effective_drofus` the rows join against. 182 lib tests pass
> (three added: unmatched-column visibility, per-project keying, pinned-
> milestone labels).

Handover H3 step 4, with the correction above. Small, additive, server-side —
the only server change in the whole UI restructure, and a prerequisite for both
P4 and P8.

1. Add `drofus_labels` to `RoomsResult` as a **per-project map**, each entry
   carrying `all_labels` (every row-1 CSV label, mapped or not) and
   `reconciliation` (the mapped subset, so a consumer can mark which columns
   have a Revit counterpart). Not a flat list — an unscoped `/rooms` spans
   several projects.

2. Take the labels from the **same resolved `DrofusData` the rows were joined
   against** — `effective_drofus` ([rooms.rs:734](src/service/rooms.rs:734)),
   not `bundle.drofus`. Otherwise a milestone view shows current column headers
   over pinned data. The value is already computed; it only needs carrying out
   of the loop.

3. Additive, no schema bump — the viewer ignores unknown fields, exactly as
   `model_to_shared` and the omittable snapshot id were added.

**Tests:** a project-scoped read carries its labels; an unmatched-by-any-room
label still appears (the case that motivates the whole change); a multi-project
unscoped read carries one entry per project, not a merged list; a milestone with
a pinned dRofus snapshot reports the *pinned* label set.

**Docs:** [Server](STRATEGY-SERVER.md)'s `/rooms` description;
[Sources](STRATEGY-SOURCES.md)'s `all_labels` note gains its second consumer.

### P3 — Labels toggle

> **Status: LANDED 2026-07-23.** Verified in the running app: toggle hides and
> restores labels across zones with pan/zoom preserved (`refit: false`), and a
> labels-off export contains 0 `<text>` nodes with all polygons intact.
> Unpersisted, matching `linkViews`.

Handover H3 step 6 — the original request, and independent of everything else.

- Global presentation state, header `button.ctl` reusing `.ctl` / `.ctl.on`. No
  new CSS class.
- **`paintLevel` gains `showLabels = true`** in its options bag, label pass
  conditional. `renderLevel` and `buildLevelSvgFile` each pass it through.
  A CSS-only toggle is cheaper but `buildLevelSvgFile` never sees a zone-level
  class, so **export would silently ignore it** — the same class of silent
  no-op P1 exists to remove.
- `exportStyleBlock()` needs no change; a leftover `.label` rule in the exported
  `<style>` is harmless, and omitting elements beats styling them away.
- **Re-render every zone with `refit: false`** — `refit: true` snaps every zone
  back to fitted bounds and users lose their pan/zoom. Guard zones with no
  `currentPayload`.
- Confirm nothing indexes a cull unit's `nodes` array positionally; `cullZone`
  iterates ([:642](static/index.html:642)), so a shorter array is fine, but the
  check is cheap.
- **Persistence: decide once for all view state, not for this flag.**
  `linkViews` does not persist today. Either persist global view prefs
  consistently (`common.js` has `persistSelection`) or not at all. A single
  persisted flag beside an unpersisted one is the inconsistency that confuses
  the next reader.

**Docs:** [Browser](STRATEGY-BROWSER.md) Implemented. Note the level-of-detail
tie-in while it is fresh: once `paintLevel` takes `showLabels`, the label half
of the open fitted-view LOD item has its mechanism in place — an automatic mode
would drive the same flag from zoom rather than a button.

### P4 — `drofus.` names discoverable in the comparison UI

> **Status: LANDED 2026-07-23.** Verified live: sample-project's datalist now
> carries `drofus.Department` / `drofus.NetArea` from `drofus_labels`, beside
> the room-property union; the empty-list fallback is untouched.

Handover H2 step 6, simplified by P2.

`comparison.html`'s `loadPropertyKeys` ([:255](static/comparison.html:255))
already fetches `/rooms`; with P2 landed it reads `drofus_labels` from that same
response and emits `drofus.`-prefixed entries alongside the union of
`room.properties` keys. No second call, no mirroring of `settings.html`'s
`drofusLabels`.

Both `#keyInput` and `#propInput` share the one `propertyOptions` datalist, so
one change covers both. **Preserve the existing `catch { keys = [] }`
fallback** — free-text entry must keep working when the list is empty.

### P5 — Tier label sizing on the areas overlay

> **Status: LANDED 2026-07-23.** Applied as specified and verified in the app
> (showcase project, SubDepartment tier): labels scale per group, small groups
> suppressed but still named in the summary panel, overlay rebuilds on tier
> switch. STRATEGY-BROWSER.md carries the threshold rationale; the handover
> moved to `Superseded/`.

Handover H1, applied as written. Two edits to `index.html`:

1. `ringBox(ring)` beside `ringAreaAbs` / `ringCentroid`, with the comment
   explaining *why* the bbox overstates usable interior for concave dissolved
   footprints — that is what justifies the conservative factor at the call site.
2. The label block in `renderAreasOverlay`
   ([:1486-1496](static/index.html:1486)), fitting the label to its ring:
   `baseFont` as ceiling, `0.6` mono glyph aspect, **`0.7` width factor rather
   than `addLabel`'s `0.9`** (room outer loops are usually convex-ish; tier
   footprints are not), and skip the label below `baseFont * 0.25`. The `>=`
   guard subsumes the degenerate-ring case — no separate zero check.

**Do not skip the doc update.** Add a bullet to [Browser](STRATEGY-BROWSER.md)
recording that tier labels *do* carry a suppression threshold and *why* that is
not the room-label bug being reinstated: **the areas summary panel names every
group, so a suppressed tier label loses no information; a suppressed room label
had no other surface.** Also record the accepted limitation — the threshold
derives from `zone.fitted`, not `zone.view`, so a group suppressed at floor
scale stays unlabelled however far you zoom. Making labels zoom-responsive means
driving `renderAreasOverlay` from the pan/zoom path with `baseFont` from
`zone.view`, throttled the way `cullZone` already is; deliberately out of scope.

**Verification is manual** (no Rust tests affected): mixed group sizes at
several tiers; an L-shaped or courtyard footprint; toggle areas off/on and
switch tier and level; and confirm the export path (`exportZoneLevels`) still
excludes the overlay — the handover asks for that intent to be re-confirmed, not
assumed.

### P6 — Scope migration

> **Status: LANDED 2026-07-23.** Verified in the running app: header scope
> pickers (one set); add-zone catches up from globals with zero refetch;
> per-zone level independence; global project/milestone switches fan to all
> zones from one poll; URL/localStorage persistence without the `zones[0]`
> special case; search shares one match set; areas and validation follow the
> global scope; picker auto-hide intact; no console errors. The open
> question resolved cleanly — nothing else depended on per-zone scope.

Handover H3 step 1. **Land it alone** — debugging a scope migration alongside
anything else means debugging two unfamiliar things at once.

Project / milestone / building become module-level global state beside
`linkViews`. Zones differ only in **level and colour** — presentation, never
data. Zone state reduces to `activeLevelId`, `activeColourPlan`, `areasMode`,
`areasTier`, view/pan state and cull units.

What this deletes, and is the main prize: any pin/unpin concept, and **the
entire focus model** — no `activeZoneId`, no active-zone border, no
focus-transfer rules on zone add/remove. Both bottom bands then have an
unambiguous subject.

What consolidates, all real: **poll once and fan out to zones** (removing the
bug class where two zones on one project drift a tick apart); one colour-plan
settings read per project change; one `currentValidation` / `errorRoomIds`
instead of a copy per zone.

The capability given up is cross-*project* and cross-*milestone* side-by-side
display — confirmed unused, and emergent rather than designed. Cross-level
viewing, which is what zones are actually used for, is untouched. A future
multi-project comparator gets **its own page**, following the established
`comparison.html` / `settings.html` pattern; it can borrow `paintLevel` and the
zone machinery, and its real blocker is the cross-project alignment transform
[Server](STRATEGY-SERVER.md) describes, not UI.

**The one genuinely open question** — does anything else depend on per-zone
scope? — surfaces here. `persistSelection`'s `zones[0]` special case
([:1834](static/index.html:1834)) resolves in the migration's favour; the poll
layer is where to look next.

### P7 — Bottom region shell, then migrate the panels

Handover H3 steps 2–3. These can land together if band 1 turns out to be a
straight move.

- `body` becomes `grid-template-rows: auto 1fr auto`; the middle row becomes
  `grid-template-columns: 1fr auto` (zones, inspector). **The CSS must tolerate
  a zero-width or `display: none` inspector column** without the zones
  reflowing oddly — the inspector is future work and absent for now.
- **Sibling of `main`, one per page, never one per zone.** A region that
  multiplies with zone count stops being a place users can point at.
- Two bands, not one collapsible panel, because their lifecycles differ. One
  user-draggable *total* height: expanding band 1 takes space from band 2, never
  from the plans — **the plans must not resize because a QA run finished**. Both
  bands scroll internally; band 1 gets a max height, which is what stops a long
  mismatch list squeezing band 2 to two rows.
- **Try side-by-side with real data before committing to stacked.** It is a
  container-level CSS change, not a restructure, and the handover is explicit
  that this is a proportions judgement far easier to settle by looking than by
  reasoning. Stacked is the default because band 2 needs the width — model *and*
  dRofus columns together is easily fifteen-plus, and hiding columns behind a
  horizontal scroll breaks exactly the side-by-side comparison the source toggle
  exists to enable.
- Then move `.validation-panel` and `.areas-panel` into band 1. Both are
  project-scoped, so under P6 that is **one instance each, not one per zone**.
  **Watch for hidden coupling:** both are currently positioned relative to
  `.zone-canvas` ([index.html:252-262](static/index.html:252)), so migration may
  surface assumptions about that containing block.
- Build band 1 so a mismatch entry can *scroll band 2 to its row* rather than
  duplicating detail. Not required for a first cut; cheap to allow for and
  expensive to retrofit.

### P8 — Source-data grid (band 2)

Handover H3 step 5. Largest piece after P6, and deliberately late: it benefits
from scope already being global and from band 1 having established the region's
layout. P2 is its prerequisite.

- **Follows scope only, never the visible plans.** Every level in the current
  global scope. This will feel wrong during implementation — a level filter will
  seem like it "should" follow the on-screen levels. It should not: with several
  zones on different levels there is no single answer, and it would make the
  grid's contents depend on presentation state, reintroducing exactly the
  blurring P6 removes. Level filtering stays available as an ordinary
  user-driven column filter.
- **Row windowing from the start.** `big-plate` is 5,046 rooms on a *single*
  level and the grid spans every level in scope, so rendering every row as DOM
  will not hold. Follow `paintLevel`'s cull-unit shape — row windowing is the
  one-dimensional case of the same idea — rather than inventing a second
  pattern.
- **Grouped column headers, not an interleaved flat table.**
  [Sources](STRATEGY-SOURCES.md) keeps dRofus as a separate sub-object precisely
  because the two have different lifecycles and provenance; the grid preserves
  that separation visibly. An unmatched room simply has no `drofus` key —
  render empty cells, not an error.
- **Unmapped dRofus columns still appear**, visibly distinguished, not hidden —
  that is what P2's `all_labels` is for, and what the coverage report already
  goes out of its way to show as "not currently checked".
- **Read-only.** Sorting and column filtering are fine; editing is a much larger
  commitment (dirty state, per-cell validation, conflict against the 2 s poll)
  and is out of scope.
- Reuse `revision` as the refresh trigger — it changes only when a contributing
  snapshot actually changes, so no row diffing is needed.
- CSV export follows the existing client-side precedent — the visible columns,
  honouring the source toggle, no server endpoint.

### P9 — Source-aware `values_agree`

Handover H2 step 4. Sequenced last of the functional work because the handover
itself rates deferral a legitimate ship, and P1 leaves it a clean follow-up: the
resolved namespace is already threaded through.

`comparison.rs:184` drops the date and ASCII-narrowing rungs on the grounds that
"both sides came through the same export, so any such artefact is symmetric and
cancels" — **true for Revit fields, false for dRofus ones**, where CSV export
artefacts reappear on both sides. Take the namespace from `resolve_presence`:
`None` → today's strict two-rung comparator, unchanged; `Some("drofus")` → the
fuller ladder.

**Do not silently reuse `validation::field_values_agree`** — it is private and
**asymmetric**, taking `(drofus_value, room_value, field_cfg)` and narrowing
only the dRofus side because it compares dRofus *against* Revit. Milestone
comparison is dRofus-vs-dRofus. In preference order: extract the symmetric rungs
into a shared comparator in `contract.rs` that both callers configure (best,
most work); add a symmetric sibling that narrows both sides (smaller, some
duplication); or leave a `TODO` plus a test asserting current behaviour.

The date rung needs a `DrofusFieldConfig` to know a field is a date — thread it
from the bundle or skip that rung and say so in a comment.

**Tests:** an ASCII-narrowing artefact on a dRofus field is not a difference,
**and** a genuine mismatch still is. `validation.rs:700` and `:720` are the
paired precedents — mirror both; the second is what stops narrowing from masking
real changes.

### P10 — Doc hygiene

> **Status: LANDED 2026-07-23** (except the `static/` conventions question,
> deliberately left as a question). Georeferencing links repointed to
> `Superseded/`; README re-indexed with open-handover statuses; room-search
> bullet added to STRATEGY-BROWSER. Found along the way: the conventions'
> `.gitattributes` does not actually exist (`core.autocrlf=true` active) —
> spun off as its own task rather than fixed mid-stream.

Ten minutes, any time a doc is already open.

- Repoint `docs/HANDOVER-georeferencing.md` to `Superseded/` in
  [STRATEGY.md](STRATEGY.md) (two places) and [Browser](STRATEGY-BROWSER.md).
- [docs/README.md](README.md): move `settings-infrastructure-handoff.md` to the
  Superseded note, and index the three open handovers plus this plan.
- Add the room-search bullet to [Browser](STRATEGY-BROWSER.md)'s Implemented
  list — it ships today and the P6/P7 header restructure assumes it.
- Decide whether [CODING-CONVENTIONS.md](CODING-CONVENTIONS.md) should say
  anything about the `static/` layer before P8 grows `index.html` further.

---

## Rules that apply throughout

- **Handovers move to `Superseded/` when their work lands**, matching the 23
  files already there. `HANDOVER-ui-layout.md` says so explicitly — and
  [Browser](STRATEGY-BROWSER.md) absorbs its outcome at that point.
- **A change touching more than one layer updates every doc it touches** —
  [STRATEGY.md](STRATEGY.md) names that as the cost of the doc split. P1 touches
  Server and Sources; P2 touches Server, Sources and Browser; P3/P5 touch
  Browser.
- **Annotate the *why*, not the what** (§91). Every non-obvious decision in this
  plan — the `0.7` width factor, the `Absent`-not-`Empty` choice for an unjoined
  source, per-project label keying — needs its rationale in the code, because
  none of it is recoverable from the code alone.
- **Tests inline** as `#[cfg(test)] mod tests` (§21), `FsStore` when snapshot
  history matters, `MemStore` when it does not.
- **LF line endings** for `*.rs` and config (§85); `.gitattributes` enforces it.

## Deliberately not in this plan

- **The right-hand inspector** (H3 Decision 3) — blocked on room click-selection,
  which nothing here builds. The region stays reserved with a known purpose.
- **A left navigator** — dropped rather than reserved, because every candidate
  for it is already served. Its one plausible future trigger is browsing by
  *classification* rather than level; revisit only if that navigation is
  actually wanted.
- **The multi-project comparator** — its own page when it comes, not a mode flag
  on the viewer.
- **`SnapshotStore::put_streaming`** and the other deferred server items — named
  in [Server](STRATEGY-SERVER.md), untouched by any of these three handovers.
