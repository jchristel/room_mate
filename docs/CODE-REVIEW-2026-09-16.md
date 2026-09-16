# Weekly review — 2026-09-16

Run of `/weekly-review`. Previous: [CODE-REVIEW-2026-08-13.md](CODE-REVIEW-2026-08-13.md).
Window: `8208d4c`'s week and the one before it, roughly PRs #119–#134 —
ceilings end to end, floors on a shared surfaces stack, the React settings page
replacing `settings.html`, per-project appearance, the entity-poll extraction and
the reports plan.

> **Outcome: every CI gate is green and the committed bundle is not stale. The
> drift is concentrated in two renames that were only half propagated** — the
> settings page stopped being a "preview" on 2026-09-12 and five files still call
> it one, and floors renamed `service::ceilings` to `service::surfaces` on the
> same day `STRATEGY-REPORTS.md` was written against the old name. The second
> was invisible to the script because of a **checker bug**, now fixed.
>
> Fixed items are uncommitted, in the same working tree as the in-progress
> `CLAUDE.md` / `STRATEGY-ENTITIES.md` edits that were already there.

## Pass 1 — mechanical checks

`python scripts/weekly_review.py` → 9 items at first run, all in the module-length
check; symbols, `.md` paths, route/tool parity (21 routes, 23 tools) and section
references clean.

### Triage

| Hit | Verdict | Action |
|---|---|---|
| `service/adjacency.rs` ~772 | **Benign** — the header already carries the judgement ("On length", names `SpatialGrid` as the seam) | Added to `[modules]` with that reason |
| `service/entity_scope.rs` ~529 | **Unjudged, new-ish** — see Q3 | Left open |
| `service/areas.rs` ~1087, `service/rooms.rs` ~1261 | **Unjudged, decided** — `CODING-CONVENTIONS.md` records the answer ("cohesive by argument") and that only the header text is missing | Left open |
| `bin/mcp.rs` ~1012, `service/comparison.rs` ~698, `settings_api.rs` ~617, `state.rs` ~839, `roommate-shared/src/settings/mod.rs` ~1685 | **Unjudged backlog** — none crossed this week (`mcp.rs` grew 962→1012 with two tools) | Left open |
| *(not reported)* `service::ceilings` at `STRATEGY-REPORTS.md:62, 254` | **Checker bug** — see below | Script fixed; doc fixed |
| *(surfaced by the fix)* `service::reports` at `STRATEGY-REPORTS.md:249` | **Benign** — the proposed module of the unbuilt `/reports/` page | Scoped ignore entry |

### The checker bug

The symbol check resolved `a::b::c` by asking whether the **leaf** appeared as a
word anywhere in the tree. A module name almost always does, so
`service::ceilings` passed after floors renamed the module — "ceilings" is in
hundreds of lines, and `contract::ceilings` even exists as a module. The check
now requires a lowercase leaf under a lowercase local parent to be a module or a
`fn`/`mod`/`macro_rules!` **of that parent** (`rust_names`, `path_is_dead`).
Leaves under a type (`ProjectSettings::comparison_key`) may be fields and keep
the word test; external crates (`std::`, `geo::`) keep it too. A first cut
flagged `settings_api::tests::test_settings_toml_round_trip` — inline `mod tests`
was not a parent — and that is handled. After the fix: 0 unexplained hits.

One scope note, not fixed: **`CLAUDE.md` is not scanned by any check**, though it
is the file an agent acts on first. Run against it today, the symbol check gives
two hits, both benign (`GetOriginalGeometry`, a Revit API; `service::floors`, the
module the file says must *not* be written). Cheap to include.

## Pass 2 — the CI gates

The week touched both trees, so all ran.

| Gate | Result |
|---|---|
| `cargo test` | pass |
| `cargo fmt --check` | pass |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `npm run typecheck` | pass |
| `npm test` | pass — 12 files, 170 tests |
| `npm run build` + `git diff --exit-code -- static/vendor/` | pass — committed bundle matches |

Re-run after this review's `src-js/` comment edits: typecheck and build pass,
and `static/vendor/` and `static/settings/` are byte-identical (comments do not
survive minification).

## Pass 3 — the judgement calls

### 1. A rationale made stale rather than a fact

- **`src-js/settings/style.css`'s header argued from a future that arrived**: "If
  the preview becomes the page, the two become one stylesheet." The preview
  became the page and the other stylesheet was deleted. *Fixed.*
- **`STRATEGY-ENTITIES.md`'s split rule is stated more strongly than floors
  honoured it.** "Share it unless sharing would change a serde key" — floors
  renamed `fraction_of_ceiling` to `fraction_of_element` on the wire, which is a
  serde key. It was right to, because the rule's *reason* is stored snapshots and
  that field was never stored. The rule should say **stored** key, or the next
  reader will either refuse a harmless rename or conclude the rule is not
  followed. *Left open — rewording a rule.*
- **`STRATEGY-REPORTS.md` was planned the day before floors shipped and does not
  know about them.** Its association table and the index line list ceilings,
  spaces and FF&E; floors join rooms the same way but with a different sliver
  rule, and the many-to-many counting warning bites harder (a House A room lies
  on up to 7 floors). *Left open — new writing.*

### 2. Mutable globals in `static/index.html`

Net improvement: 86 → 80 top-level `let`/`var`. The entity-poll extraction
removed twelve (`*Payload`, `last*Revision`, `last*Etag` for four entities).
This week added `appearance`, `showRooms`, `showCeilings`, `showFloors`.

The shape to watch is back in a milder form: **seven parallel `show*` flags**
(doors → floors), and the spaces/ceilings/floors click handlers
(`index.html` ~5096–5130) are near copies differing only in the entity name. The
poll gating already reads them through `EntityPoll`'s `enabled`, so a layer
table beside it would be the natural home. Not urgent; noted so the eighth layer
does not add an eighth copy.

### 3. Newly long modules without a header judgement

- **`service/entity_scope.rs`** crossed the trigger on 2026-09-08 (478 → 505,
  linked-model placement) and reached 529 with ceilings. No header judgement, no
  ignore entry — and it has **no inline tests at all**, which is unusual here for
  a module every entity read goes through. The `[modules]` entry for
  `openings.rs` also rests on having moved code into it, so its argument is now
  leaning on a module that is itself unjudged.
- **`service/surface_attribution.rs` is the counter-example worth pointing at**:
  split out of `surfaces` *before* the trigger, with the reason in its header.
- `areas.rs` and `rooms.rs` are still the two named in `CODING-CONVENTIONS.md`
  as decided-but-unwritten.

### 4. Live docs disagreeing with each other

- **`STRATEGY-ENTITIES.md`'s opening said floors were "probed on House A only"**,
  while its own floors entry (in-progress edit), and `CLAUDE.md`, record RHH
  measurements. *Fixed.* Same paragraph: "six entities" → seven. *Fixed.*
- **`docs/README.md` said Entities records what "five" entities proved**, and its
  open list lacked the ceilings/floors QA reports. *Fixed.*
- **`CODING-CONVENTIONS.md` said "Three modules carry a recorded judgement"**;
  the ignore file had six (seven now). Replaced with a pointer to the ignore
  file — a hand-kept count is exactly what that section warns against. *Fixed.*
- **`STRATEGY-ENTITIES.md` contradicts itself on the test count**: "The bet below
  has been tested three times … the second and third tests are the ones worth
  having", then "Ceilings were the fourth test". Floors, which cost nothing
  structural in the way windows did, are not recorded as a test at all. *Left
  open — which tests are "worth having" is a judgement.*
- `CLAUDE.md`'s "Open, as of 2026-09-07" heading now holds 2026-09-16
  measurements. Part of the in-progress edit; flagged, not touched.

### 5. Shipped work still described in a strategy doc

- **The settings page is not a preview.** `ff2d656` said "Nothing is left calling
  the only settings page a preview"; five places did — `STRATEGY-BROWSER.md:10`,
  `vite.settings.config.ts`, `src-js/settings/main.tsx`, `style.css`, and
  `App.tsx` ("the JavaScript page does not", present tense for a deleted page).
  `static/common.js` still said "settings.html and comparison.html". *All fixed.*
- **`STRATEGY-ENTITIES.md`'s ceilings entry keeps a closed-questions paragraph**
  ("Both questions this entry used to carry are answered … a ceiling is a LIST
  of polygons and the consumer unions them"). That is built behaviour, recorded
  in `CLAUDE.md` and `service::surface_attribution`. The entry's open item is the
  QA report; the paragraph should go. *Left open — deletion in a section with
  in-progress edits.*
- **The floors entry is titled "what House A settled, and what it could not"**
  and opens with what was settled (sliver escape, host Level). RHH has since
  answered part of it, and "settled" is scaffolding. Retitle to what is left and
  drop the settled sentence. *Left open, same reason.*

## What was fixed

| File | Change |
|---|---|
| `scripts/weekly_review.py` | Parent-scoped resolution of `module::item` paths (checker bug) |
| `scripts/weekly_review_ignore.toml` | `service/adjacency.rs` judged; `service::reports@STRATEGY-REPORTS.md` benign |
| `docs/STRATEGY-REPORTS.md` | `service::ceilings` → `service::surface_attribution` / `service::surfaces`; `fraction_of_ceiling` → `fraction_of_element` |
| `docs/STRATEGY-BROWSER.md`, `vite.settings.config.ts`, `src-js/settings/{main.tsx,App.tsx,style.css}`, `static/common.js` | Settings page no longer called a preview |
| `docs/STRATEGY-ENTITIES.md` | "House A only" → "and measured on RHH"; six → seven entities |
| `docs/README.md` | five → seven; ceilings/floors QA added to Entities' open list |
| `docs/CODING-CONVENTIONS.md` | Hand-kept module-judgement count replaced with a pointer |

## Suggested work order

1. **Commit or stash the in-progress `CLAUDE.md` / `STRATEGY-ENTITIES.md` edits
   separately from this review's fixes** — they share a file now.
2. **Tighten the Entities split rule to "stored serde key"** — it is the one
   finding that would lead a reader to a wrong decision.
3. **Trim the ceilings entry's closed paragraph and retitle the floors entry**
   (Q5).
4. **Add floors to `STRATEGY-REPORTS.md`'s association table**, before anyone
   builds the page from it.
5. **Judge `entity_scope.rs`** — header plus ignore entry, and consider whether
   a shared module with no tests should get some.
6. **Include `CLAUDE.md` in the symbol check** (two benign ignore entries).
7. Write the `areas.rs` / `rooms.rs` header judgements; then the rest of the
   module backlog.
8. When the eighth plan layer arrives, fold the `show*` flags into a layer table.
