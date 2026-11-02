# RoomMate — code review, 2026-08-07

A read-only critique against seven questions. Nothing was changed. Line numbers
are from `main` at `db8161d`.

**Headline.** The code is in good shape and the conventions are unusually well
written down. The defect concentration is not in the code — it is in the
**documentation's factual claims about the code**, and specifically in claims
that were true when written and were falsified by a *later* change that did not
revisit them. Six of the eight highest-severity findings below are of that one
shape. That is a direct consequence of this project's greatest strength: the
docs assert a lot, so they have a large falsifiable surface. Section 7 argues
that most of it should be checked by a script, not by a reader.

---

## 1. Claude-specific settings / instruction files

`.claude/` contains **only** `launch.json`. There is no `settings.json`, no
hooks, no skills, and no directory-scoped `CLAUDE.md`. `CLAUDE.md`, `.mcp.json`
and `.codex/config.toml` at the root are good and doing real work.

Ranked by value:

### 1a. A hook for the committed-bundle trap — the highest-value addition

`CLAUDE.md` names forgetting `npm run build` as the mistake that produces "a red
PR, or worse a green one serving a stale renderer", and
`.github/workflows/frontend.yml` exists almost entirely to catch it. That gate
fires *after push*. A `PostToolUse` hook on `Edit`/`Write` matching `src-js/**`
can print the reminder at the moment of the edit, which is where it is cheap.

Proposed `.claude/settings.json`:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "git diff --quiet --exit-code -- src-js/ || echo 'src-js/ is dirty — static/vendor/renderer.bundle.js must be rebuilt (npm run build) and committed before this lands.'"
          }
        ]
      }
    ]
  }
}
```

### 1b. A permission allowlist for the four verification commands

`cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`npm run typecheck`, `npm test`, `npm run build` are mandatory per `CLAUDE.md`
and are all safe. Allowlisting them in `.claude/settings.json` removes a
prompt from every single session that touches code.

### 1c. A directory-scoped `CLAUDE.md` for `extractor/pyRevit/`

This is the one part of the tree that plays by different rules — IronPython 2.7,
no `cargo`/`npm` verification path, no CI coverage at all, LF-sensitivity
(`CLAUDE.md` explicitly warns that Python heredocs on Windows silently write
CRLF), and a hard dependency on which duHast the pyRevit extension is running.
Today all of that sits in the root `CLAUDE.md`, where it is loaded for every
session including the ones that never leave `src/`.

### 1d. What *not* to add

A `/weekly-review` skill is worth having (see section 7), but do not add skills
for the build/test commands — `CLAUDE.md`'s "Verify before claiming done" block
already covers them, and a skill would be a second place for that list to drift.

---

## 2. Common code conventions we are not following

The repo has its own `docs/CODING-CONVENTIONS.md`, so "conventions" is judged
against that first, and against language norms second.

### 2a. Two conventions the doc sets that the code then ignored

**`rooms.rs` and `areas.rs` never wrote down why they declined to split.**
`CODING-CONVENTIONS.md:54-57` asks, explicitly, that if these two stay whole
they "say so in the header the way `adjacency.rs` does, so the next reader knows
it was decided rather than deferred." Neither header mentions it.
[`service/rooms.rs`](../src/service/rooms.rs) is now the largest module in the
crate at ~1,165 non-test lines behind a six-line header. `adjacency.rs` still
carries its rationale, so the pattern exists and was simply not applied.

**`settings/mod.rs` was named as "the one that reads as unfinished" and then
grew 25%.** The doc measured it at 804/826 lines; it is now ~1,034 non-test.
The doc even notes the seams already exist. Not a defect — but a rule nothing is
measured against, which is the exact failure that document's last paragraph warns
about.

*(Counting method: lines before the first `#[cfg(test)]`. The doc does not state
its method, so treat the deltas as indicative, not exact.)*

### 2b. Rust: pre-generalisation naming survived on generalised types

In [`service/validation.rs`](../src/service/validation.rs), functions now
operating on the generic reference-source types still carry `drofus` identifiers
and doc comments:

- `field_config(drofus_fields: &[ReferenceFieldConfig], …)` — :585
- `compare_mode(drofus_fields: &[ReferenceFieldConfig], …)` — :593
- `compute_field_coverage(drofus: &ReferenceData, drofus_fields: …)` — :675
- doc comments reading "one dRofus field label" (:584, :589) and "narrowing the
  dRofus side" (:601)

The types are `ReferenceFieldConfig` / `ReferenceData`. dRofus is one configurable
source among several — `bootstrap.rs` tests already exercise a two-source project
(`drofus`, `ffe`). This reads as an unfinished rename, and it makes a genuinely
generic function look source-specific.

Same vestige in [`service/rooms.rs:1`](../src/service/rooms.rs) ("dRofus join")
and [`lib.rs:25`](../src/lib.rs).

Distinguish this from the *legitimate* uses of the string `drofus` as a sample
source name in test fixtures and tool descriptions (`bin/mcp.rs`,
`bootstrap.rs`) — those are correct and should stay.

### 2c. Frontend: `el()` is duplicated across two pages and has already drifted

- [`static/settings.html:247`](../static/settings.html) — `function el(tag, attrs, children)`
- [`static/comparison.html:182`](../static/comparison.html) — `function el(tag, attrs = {}, children = [])`

Same name, same job, different signatures — one tolerates omitted arguments and
one does not. `common.js` exists precisely for "two views must agree" and does
not carry it.

I want to be careful here, because `CODING-CONVENTIONS.md:108-110` deliberately
rejects extracting on a line count and requires a *specific* argument. The
specific argument exists in this case and it is the drift itself: the two copies
already disagree about a defaultable parameter, so a snippet moved between the
two pages behaves differently. That is the condition the rule names.

### 2d. Frontend: what is genuinely fine

Worth recording so a future review does not re-litigate it:

- **No `var` anywhere** in `static/` (0 occurrences across all five files).
- **`innerHTML` is used 7 times, all in `index.html`, all through
  `escapeHtml()`/`cssIdent()`.** Checked; no unescaped interpolation found.
- `lang="en"`, `charset`, and a viewport meta are all present on all three pages.
- The 15 loose-equality comparisons in `index.html` are `== null` / `!= null`
  idiom, which is the correct form for a null-or-undefined test.
- `tsconfig.json` is stricter than most projects manage (`strict`,
  `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`,
  `verbatimModuleSyntax`), and the comments explain why each is load-bearing.

### 2e. Rust: the lint stance is working

`Cargo.toml` sets `too_many_lines = "warn"`; CI runs `-D warnings`. Exactly
**one** `#[allow(clippy::too_many_lines)]` exists in the entire tree
(`settings_api.rs`), with a reason. The tree is clean. No action.

---

## 3. Stale inline comments

Ordered by consequence.

| Location | Claim | Reality |
|---|---|---|
| [`Cargo.toml:29`](../Cargo.toml) | "the MCP binary's `upload_drofus` tool" | The tool is `upload_reference` |
| [`contract/mod.rs:1159`](../src/contract/mod.rs) | "A stand-in for the two-tier entity doors **will be** … rather than waiting for `Door`" | `Door` exists and implements `PropertyTiers` (`contract/doors.rs:228`) |
| [`post_rooms.py:2`](../extractor/pyRevit/room_m/post_rooms.py) | "**POC** bridge" | Production; contract v6 |
| [`post_doors.py:2`](../extractor/pyRevit/room_m/post_doors.py) | "**POC** bridge for DOORS" | `CLAUDE.md`: "Doors ship end to end" |
| [`service/rooms.rs:1`](../src/service/rooms.rs), [`lib.rs:25`](../src/lib.rs) | "dRofus join" | Generic reference-source join since R4 |
| [`service/validation.rs:584,589,601`](../src/service/validation.rs) | "one dRofus field label" | `ReferenceFieldConfig`, any source |

**Six stale path references** to documents that have since moved into
`docs/Superseded/`. None break a build; all break a reader's navigation:

- [`package.json:5`](../package.json), [`tsconfig.json:3`](../tsconfig.json),
  [`vite.config.ts:11`](../vite.config.ts),
  [`static/common.js:7`](../static/common.js),
  [`static/index.html:809`](../static/index.html),
  [`.gitignore:6`](../.gitignore) → all cite `docs/PLAN-webgl-renderer.md`,
  now `docs/Superseded/PLAN-webgl-renderer.md`
- [`settings/server.toml:3`](../settings/server.toml) → `../docs/HANDOVER-per-project-settings.md`,
  now under `Superseded/`

**One live stale flag, not just a comment.**
[`package.json:8-9`](../package.json):

```json
"// test": "--passWithNoTests until P1 lands the first suite; drop the flag then.",
"test": "vitest run --passWithNoTests",
```

P1 landed; there are **seven** `.test.ts` suites. The note says to drop the flag
and nobody did. This one has teeth: `--passWithNoTests` means that if a config
change ever caused vitest to match zero files, CI would go **green** on a
frontend that ran no tests at all. The comment correctly predicted its own
removal condition, which is the best possible argument for acting on it.

**Explicitly not stale** — checked and correct, do not "fix" these:
`main.rs:91` and `main.rs:414` reference a deleted `POST
/api/settings/drofus-check` endpoint, but both say so in the same breath ("That
endpoint has since been deleted…"). That is history carried on purpose, and it
is the load-bearing example in a security rationale.

---

## 4. Stale strategy-document sections

This is where the real drift is. Everything below was verified against code.

### 4a. Wrong, in a way a reader will act on

**`STRATEGY-MCP.md:80-83` and `:44-45` — "`get_doors` has no `building`
parameter".** It does.
[`bin/mcp.rs:98-102`](../src/bin/mcp.rs) declares `building: Option<String>` on
`GetDoorsParams` with a full doc comment about homeless doors. The doc goes
further and *justifies* the absence ("a door's building would depend on which of
its two rooms owns it, which is Decision 6's open question") — but Decision 6 was
decided and built, which the entities doc itself records. Two places, one of them
an argued rationale for a state of affairs that no longer exists.

**`STRATEGY-ENTITIES.md:276-279` — the same claim, and it contradicts itself ten
lines later.** :276 says "**`GET /doors` takes no `?building=`**"; :286 says
"`GET /doors` — scoped by `?project=` / `?building=` / `?milestone=` /
`?filter=` exactly as `/rooms`"; :505 confirms `?building=` shipped. The
"departure" note at :276 should simply be deleted — the departure was undone.

**`STRATEGY-MCP.md:131-140` — the `upload_drofus` bullet is stale in its name,
its route, and its heading.** The tool is `upload_reference`; the route is
`POST /projects/{id}/reference/{source}`, not `/projects/{id}/drofus`. The body
of the bullet is still correct and valuable (the forwarding rationale, the
single-writer argument, the staleness asymmetry) — only the identifiers are wrong.

**`STRATEGY-MCP.md:154-156` — names `ServiceError` variants that do not exist.**
Doc says "`NotFound`/`BadInput` both become `McpError::invalid_params`".
[`service/mod.rs:30-41`](../src/service/mod.rs) defines exactly two variants:
`Internal` and `Invalid`. The described *behaviour* is right; the variant names
are from a version of the enum that no longer exists.

**`STRATEGY-ENTITIES.md:28-31` — says R4 is unbuilt.** "R4 is the one
prerequisite doors deliberately shipped without … Until then
`[sources.reference.*]` still means 'for rooms' — [Sources] records the gap."
R4 landed 2026-08-05, and `STRATEGY-SOURCES.md:376` correctly marks it **Closed**.
So two strategy docs now disagree with each other about a shipped feature, and
the stale one points at the current one as its evidence. Same problem at
`STRATEGY-ENTITIES.md:67`.

**`STRATEGY-MCP.md:21` — lists a `drofus` module.** [`lib.rs:57-66`](../src/lib.rs)
has no such module; it is `reference`.

### 4b. Stale but low-consequence

- **`STRATEGY-MCP.md:189-196`** documents a `.gitignore` fix as
  `!VS/duHastApplications/roommate/src/bin/` — a path implying this crate was
  nested inside another repo. [`.gitignore:35`](../.gitignore) is now plain
  `!src/bin/`. The *lesson* (gitignore is last-match-wins) is worth keeping; the
  path is not.
- **`STRATEGY-SERVER.md:34`** cites `service::validation::compute_validation`;
  the public entry point is `compute_project_validation`.
- **`CODING-CONVENTIONS.md:21-28`** — the "measured 2026-08-01" table. It cites
  `contract.rs (615)`, a file that **no longer exists** (split into
  `contract/mod.rs` ~690 and `contract/doors.rs` ~321 — the split the doc itself
  predicted at :38-41). It also contradicts itself on `settings/mod.rs`: 826 in
  the table, 804 in the prose at :48. Several other counts have drifted
  (`handlers.rs` 744→~1,057, `validation.rs` 646→~947).
- **`CODING-CONVENTIONS.md:98-100`** — "It is now **4,369**" for `index.html`
  (actually 4,548) and "2,150 lines of TypeScript" (actually ~2,896 non-test).

### 4c. What can be retired

The `docs/Superseded/` discipline is working well — 38 of 55 docs are archived,
`docs/README.md` explains *why* the interesting ones were kept, and "Open
handovers: **None**" is accurate. I would retire nothing wholesale. Two
narrower retirements:

1. **`STRATEGY-ENTITIES.md`'s "What survived contact" header block (:10-45)** has
   done its job. It was scaffolding for a build in flight; the build landed, and
   each decision section below now carries its own "As built" note. Its R4
   paragraph is now actively wrong. Cut it to the two supersession pointers that
   still matter and let the per-decision notes carry the rest.
2. **`STRATEGY-MCP.md`'s "Open items / things to watch" (:208-228)** is down to
   one live item (no resources/prompts exposed). The other two are closed —
   `get_adjacency` is described as "the most recent worked example" when doors
   have shipped since, and the F&E reuse note just forwards to
   `STRATEGY-SERVER.md`. Collapse to the one item.

---

## 5. Bolted-on / organically grown

### 5a. `static/index.html` — the one genuine instance

4,548 lines, of which ~3,737 are a single inline `<script>` holding ~226
top-level declarations and **~44 module-level mutable globals**: `zoneSeq`,
`currentPayload`, `doorsPayload`, `lastDoorsRevision`, `showDoors`, `areasData`,
`adjData`, `adjGraph`, `gridColumns`, `gridRows`, `gridSort`, `gridScrollTop`,
`gridCellErrors`, `inspHideEmpty`, `searchQuery`, `selection`, … Each feature —
areas, adjacency, the grid, the inspector, search, doors — deposited its own
tranche of globals into one shared scope.

The important nuance: `CODING-CONVENTIONS.md:131-135` already has the rule ("each
frontend change moves the module it touches into `src-js/`"), and it is being
**partly** honoured. The door glyph work put 397 lines of real geometry into
`src-js/renderer/gl/doorGlyph.ts` with 348 lines of tests — genuinely good. But
the same change also added `doorsPayload`, `lastDoorsRevision` and `showDoors`
to the pile in `index.html`. So the *computation* migrates and the *state* does
not, and the page keeps growing (4,369 → 4,548 since the last measurement) even
as the rule works.

That is worth naming precisely, because "the rule is working" and "the page is
still growing" are both true, and only one of them is currently written down.

### 5b. Where the architecture resisted bolt-on

Worth recording, since the review is otherwise about drift. Doors were the big
test — a whole second entity — and they did **not** grow a parallel stack:
`SnapshotStore` took bytes rather than sprouting `put_doors` (R1 held);
`PropertyTiers` was generalised *before* the `Door` type needed it (R2 held);
`RoomFilter::parse` is reused verbatim for doors in both adapters;
`room_attribution` is derived at read time and stores nothing. The one asymmetry
(`/doors` refused for an unphased model where `/rooms` quarantines) is argued in
three separate places. This is the opposite of organic growth.

### 5c. Smaller

`service/validation.rs`'s `doors` section is reached through a report struct that
grew field-by-field alongside the reference-source sections it sits beside, and
the naming vestige in 2b is the visible residue. Not urgent.

---

## 6. Is the MCP at the same feature level as the server?

**Yes, essentially — with one gap and one doc overclaim.**

17 tools against 14 HTTP read routes plus 3 settings reads. Verified
parameter-by-parameter, not just route-by-route:

| HTTP | MCP | Params match |
|---|---|---|
| `GET /rooms` | `get_rooms` | ✅ project, building, milestone, filter |
| `GET /doors` | `get_doors` | ✅ project, building, milestone, filter |
| `GET /projects` | `list_projects` | ✅ |
| `…/buildings` | `list_buildings` | ✅ |
| `…/validation` | `get_validation` | ✅ |
| `…/snapshots` | `list_snapshots` | ✅ |
| `…/milestones` | `list_milestones` | ✅ |
| `…/areas` | `get_hierarchy_areas` | ✅ building, milestone |
| `…/adjacency` | `get_adjacency` | ✅ building, milestone, wall_max |
| `POST …/comparison` | `compare_milestones` | ✅ baseline, others |
| `…/snapshots/latest` | `get_latest_snapshot` | ✅ |
| `…/snapshots/pending` | `get_pending_snapshot` | ✅ |
| `…/reference/{source}/snapshots` | `list_reference_snapshots` | ✅ (+ optional `source`) |
| `…/reference/{source}/latest` | `get_reference_snapshot` | ✅ (+ optional `source`, `taken_at`) |
| `GET /api/settings/projects` | `list_project_settings` | ✅ |
| `GET /api/settings/projects/{id}` | `get_project_settings` | ✅ |
| **`GET /api/settings/resolve/{id}`** | **— none —** | ❌ |

**The gap:** `/api/settings/resolve/{id}` has no MCP tool. It is the
viewer-tolerant read that falls back to the `is_default` file when the payload id
is not a settings project id. An agent that calls `get_project_settings` with a
*payload* project id gets an error where the viewer would have resolved it.
Probably worth adding — it is a four-line adapter over the same
`settings_api` core — but it is a judgement call, not a defect.

**The overclaim:** `bin/mcp.rs:2` says "one per existing HTTP read route" and
`STRATEGY-MCP.md:41` says the same. With `resolve` unmapped that is now false.
Either add the tool or soften both sentences.

**Correctly and deliberately absent** (all three argued in the module doc, no
action): ingest `POST /rooms|/doors` and their `/stream` pairs;
`POST …/pending/activate`; settings *writes*. The one mutation that exists,
`upload_reference`, forwards over HTTP rather than writing, which preserves the
single-writer rule.

---

## 7. Should this become a weekly-review document?

**Yes — but most of it should be a script, not a checklist.**

The evidence for that is this review itself. Of the ~20 findings above, the
overwhelming majority are **mechanically detectable**: a doc names a symbol that
no longer exists, a comment cites a path that has moved, a measured number has
drifted, a doc asserts a parameter is absent that is present. A human reading
`STRATEGY-MCP.md` cannot be expected to notice that `ServiceError::NotFound` was
renamed eighteen months of commits ago. `grep` notices instantly.

This matters more here than in most repos precisely *because* the documentation
is so good. Dense, specific, falsifiable prose is this project's main asset — and
its maintenance cost scales with how much it asserts. A weekly human re-read does
not scale. A weekly `cargo test`-shaped check does.

### Recommended shape

**A `.claude/skills/weekly-review/SKILL.md`**, invocable as `/weekly-review`, in
two parts:

**Part 1 — automated, and the part that pays for itself.** A script the skill
runs first:

1. **Symbol liveness.** Extract every `` `identifier` `` from `docs/*.md` that
   looks like a Rust path (`service::foo::bar`, `FooStruct`, `snake_case_fn`) and
   assert it still exists in `src/`. This alone catches `compute_validation`,
   `ServiceError::NotFound`, `contract.rs`, `mod drofus`, `upload_drofus`.
2. **Path liveness.** Assert every `docs/…md` reference in code and docs resolves
   to a file that exists. Catches all seven stale-path findings.
3. **Route/tool parity.** Count `.route(` in `main.rs` against `#[tool(` in
   `bin/mcp.rs` and diff the names; fail when a read route has no tool. Catches
   the `resolve` gap and keeps `bin/mcp.rs:9`'s count honest.
4. **Measured-number staleness.** Re-run the non-test line count and diff against
   the table in `CODING-CONVENTIONS.md`; flag any figure off by >10%.
5. **Committed-bundle freshness.** `npm run build && git diff --exit-code --
   static/vendor/` — the same assertion CI makes, run before the PR.

**Part 2 — the judgement calls, which stay human.** A short checklist:

- Did any change this week add a mutable global to `static/index.html` rather
  than a module to `src-js/`? (Section 5a's question.)
- Did any change make a **rationale** stale rather than a fact? Section 4a's
  `get_doors`-`?building=` finding is the type specimen: the sentence explained
  *why* something was absent, and it kept reading plausibly long after the thing
  arrived. No script finds these.
- Is any newly-long module missing the "decided, not deferred" header the
  conventions require?
- Did a doc-vs-doc contradiction appear? (`STRATEGY-ENTITIES` vs
  `STRATEGY-SOURCES` on R4 is the worked example.)

### One caveat on scope

Do **not** put the build/test commands in this skill. `CLAUDE.md` already owns
that list and a second copy would be one more thing to drift — which is,
precisely, the finding this whole document keeps making.

---

## Suggested order of work

1. `package.json` — drop `--passWithNoTests` and its note. *(Only finding with a
   live CI-correctness consequence.)*
2. `STRATEGY-MCP.md` + `STRATEGY-ENTITIES.md` — the six wrong claims in 4a.
   *(Wrong beats stale; an agent reads tool descriptions and strategy docs as
   fact.)*
3. The seven stale doc paths + `Cargo.toml:29` + the two "POC bridge" headers.
   *(Mechanical, ~20 minutes.)*
4. The `weekly-review` skill and `.claude/settings.json` hook — so 1–3 do not
   silently recur.
5. `validation.rs` drofus→reference rename; `rooms.rs`/`areas.rs` split-rationale
   headers; `CODING-CONVENTIONS.md` re-measure.
6. Decide on the `resolve` MCP tool and the `el()` extraction. *(Judgement calls,
   no urgency.)*
