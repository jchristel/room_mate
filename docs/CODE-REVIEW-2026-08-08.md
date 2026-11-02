# Weekly review — 2026-08-08

First run of `/weekly-review`. The deeper one-off audit is
[CODE-REVIEW-2026-08-07.md](CODE-REVIEW-2026-08-07.md); this covers what the
skill's three passes found and is the format subsequent weeks should follow.

> **Outcome: all 16 rot items fixed** (the 15 below plus one that had been hidden
> by a checker bug — see *Fixed* at the end). The mechanical checks now run clean
> apart from the one open judgement call. Findings are left in place below rather
> than deleted, because the point of the record is what drifted and why.

## Pass 1 — mechanical checks

`python scripts/weekly_review.py` → **21 items**, triaged below. It reproduced
every finding in its four categories that yesterday's manual audit made, and
found **four more** that audit missed — which is the argument for the script.

### Rot (fix the doc) — 15

**Dead symbols in live docs (6).** The four marked ★ are new.

| Symbol | Cited at | Reality |
|---|---|---|
| `BadInput` | `STRATEGY-MCP.md:154` | `ServiceError` has only `Internal`/`Invalid` |
| `contract.rs` | `CODING-CONVENTIONS.md:25,38`, ★`STRATEGY-ENTITIES.md:307`, ★`STRATEGY-SOURCES.md:33` | Split into `contract/mod.rs` + `contract/doors.rs` |
| ★`settings.rs` | `STRATEGY-SOURCES.md:33` | Split into `settings/` |
| ★`service/drofus.rs` | `STRATEGY-SERVER.md:685` | Now `service/reference.rs` |
| ★`DrofusSource` | `STRATEGY-SERVER.md:472` | Renamed in the R4 generalisation |

**Stale `.md` paths (9).** Six build/config files still cite
`docs/PLAN-webgl-renderer.md`, which moved to `docs/Superseded/`:
`package.json`, `tsconfig.json`, `vite.config.ts`, `static/common.js`,
`static/index.html`, `.gitignore`. Plus `settings/server.toml`
(`../docs/HANDOVER-per-project-settings.md`), `src/contract/doors.rs`
(`../../docs/PLAN-phasing.md`), and `extractor/…/room_mate.py`
(`Superseded/HANDOVER-areas-boundary-location.md` — missing its `docs/` prefix).

**Measured counts in `CODING-CONVENTIONS.md` (5).** `contract.rs (615)` names a
file that no longer exists; `handlers.rs` +42%, `service/validation.rs` +47%,
`settings/mod.rs` +25%, `bin/mcp.rs` +10%. The doc's own closing paragraph
argues that a discipline nothing measures is one nobody can tell has stopped
being necessary — these numbers are that discipline, unmeasured since
2026-08-01.

### Judgement call — 1

`/api/settings/resolve/{id}` still has no MCP tool. Deliberate or not, it makes
`bin/mcp.rs:2`'s "one per existing HTTP read route" false. Either add the tool
or soften the sentence. Unchanged from yesterday; left open on purpose so the
check keeps asking.

### Benign — 15, now in `weekly_review_ignore.toml` with reasons

External APIs (`Int64`, `IfcRelSpaceBoundary`, `BitmapText`, `FormData`,
`HttpClient`, `StringContent`, `ActiveProjectLocation`), prose that matches an
identifier shape (`Comments`, `DELETE`), illustrative placeholders (`foo.rs`,
`foo.ts`, `crate::foo::Bar`), a design settled-but-unbuilt (`DocumentIndex`),
and two cases of docs correctly naming an old symbol *to explain a rename*
(`DrofusRecord`, `BuiltinProperties`).

That last category is why this is advisory and not a gate: `STRATEGY-SOURCES.md`
writing "`ReferenceRecord`, renamed from `DrofusRecord`" is good documentation
that a liveness check cannot distinguish from rot.

### Checker bugs found and fixed during this run — 2

Recorded because the skill says to say so out loud:

- Line-count resolution matched by basename, so `settings/mod.rs` silently
  compared against `contract/mod.rs` and reported a −16% drift that did not
  exist. Now resolves by exact path first.
- Path liveness originally checked bare prose mentions ("see `PLAN-phasing.md`
  D6") and reported 60+ hits, nearly all of them fine. Narrowed to markdown
  links and citations that actually spell out a path — from 60+ to 9, all real.

## Pass 2 — CI gates

All green.

| Gate | Result |
|---|---|
| `cargo test` | **369 passed**, 0 failed |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `npm run typecheck` | clean |
| `npm test` | **87 passed** (7 files) |
| `npm run build` + `git diff -- static/vendor/` | **bundle current** |

## Pass 3 — judgement calls

1. **Stale rationale, not stale fact?** Yes — the standing one from yesterday:
   `STRATEGY-MCP.md:80-83` and `STRATEGY-ENTITIES.md:276-279` both *argue* that
   `GET /doors` takes no `?building=` from "Decision 6's open question".
   Decision 6 was decided and the parameter shipped. `STRATEGY-ENTITIES.md` then
   contradicts itself ten lines later at `:286`. Nothing new arose this week.
2. **Mutable global added to `index.html`?** Nothing new this week; the door
   work landed before this window. The pile stands at ~44.
3. **Long module missing its "decided, not deferred" header?** Unchanged —
   `rooms.rs` (~1,165 non-test) and `areas.rs` still lack it.
4. **Live docs disagreeing?** Yes, unresolved: `STRATEGY-ENTITIES.md:28-31`
   says R4 is unbuilt and cites `STRATEGY-SOURCES.md`, which marks it Closed.
5. **Anything ready to supersede?** `STRATEGY-ENTITIES.md`'s "What survived
   contact" block (`:10-45`) has outlived its build, and `STRATEGY-MCP.md`'s
   "Open items" (`:208-228`) is down to one live item.

## Fixed

All rot items, 2026-08-08. Gates re-run green afterwards (369 Rust tests, 87
frontend tests, fmt, clippy, typecheck, bundle current).

- **Dead symbols** — `BadInput` → the real `Invalid`/`Internal` split, with a
  note that the absence of a `NotFound` variant is the design (`STRATEGY-MCP.md`);
  `settings.rs` → `settings/` and `contract.rs` → `contract/mod.rs`
  (`STRATEGY-SOURCES.md`); `DrofusSource` → `ReferenceOrigin`, naming the old
  type to explain the rename (`STRATEGY-SERVER.md`); "the former `contract.rs`"
  (`STRATEGY-ENTITIES.md`).
- **`service/drofus.rs` → `service/reference.rs`** (`STRATEGY-SERVER.md`), and
  with it the two stale routes in the same sentence (`/projects/{id}/drofus/…`
  → `/projects/{id}/reference/{source}/…`) and the bullet's own heading. Fixing
  the symbol alone would have left the sentence incoherent.
- **Nine stale `.md` paths** — six build/config files pointing at
  `docs/PLAN-webgl-renderer.md`, plus `settings/server.toml`,
  `src/contract/doors.rs` and `extractor/…/room_mate.py`.
- **Measured counts re-measured**, now twelve modules past the trigger, with a
  note that `scripts/weekly_review.py` checks them so they cannot drift silently
  again. Also corrected the two prose counts ("eleven" → "twelve") that the
  re-measure invalidated.
- **`contract.rs` in `STRATEGY.md:206`** — a *sixteenth* item, and the
  interesting one: it was hidden all along because the checker truncated its
  location list at four sites, and three benign ones came first. Found only
  because fixing the others made it surface.

### Two checker changes this forced

- **No truncation of location lists.** Hiding a site behind a display cap is the
  one failure mode a drift checker must not have.
- **Ignores can now be scoped `SYMBOL@DOC.md`.** A bare `contract.rs` ignore
  would have silenced the `STRATEGY.md` hit permanently. Docs that name an old
  symbol *to explain a rename* are benign in that document only.

## The two retirements

Both done 2026-08-08. Neither was a deletion, and in one case it could not have
been.

**`STRATEGY-ENTITIES.md`'s "What survived contact" block.** The plan was to cut
it and let the per-decision *as built* notes carry the content. Checking first
showed that was wrong: **R2's tier-precedence rule — "a tier wins only when it is
`Present`", which `CLAUDE.md` leads with — existed nowhere else in the
document**, and Decision 4 pointed *at this block* for it ("stated in the summary
at the top"). Deleting it would have destroyed the only statement of a
load-bearing rule and broken the reference to it.

So the rule moved into Decision 4, which is the decision it constrains and where
the pointer already wanted it; then the scaffolding went. What replaced it says
why it was retired: a summary duplicating six *as built* notes had to be kept in
step with all six, and was not — it described R4 as unbuilt for three days after
it landed.

Two adjacent bits of the same R4 rot went with it: the "Deferred" list still
called R4 "the one prerequisite doors shipped without", and the intro still said
"axis 1 has not [shipped]" — a flat contradiction of the doc's own **Status:
built** header. Neither was on the list; both were the same falsehood.

**`STRATEGY-MCP.md`'s "Open items / things to watch".** Down from three to one.
The F&E validation reuse gap was never an MCP item — the tool calls the same
service function the HTTP route does, so a second copy of the note meant two
places to update for a gap that only ever existed in one. "A new `service/`
capability is a tool away" stopped being a thing to *watch*: doors proved it at
the scale of an entire second entity, where the only MCP-specific work was
correcting three tool descriptions — which is both the claim holding and the
reminder that descriptions are the part that does not come free.

## Still open — yours to decide

1. **`/api/settings/resolve/{id}` has no MCP tool.** Either add it (a four-line
   adapter over the same `settings_api` core) or soften `bin/mcp.rs:2` and
   `STRATEGY-MCP.md:41`, which both claim one tool per HTTP read route.
*(The `?building=` rationale rot from the 2026-08-07 audit was rewritten on
2026-08-08 — see below.)*

## The `?building=` rewrite

The one finding no script could have made, and the reason pass 3 exists. Neither
document merely *stated* that `GET /doors` took no `?building=` — both **argued**
it, from "Decision 6's open question". Decision 6 was then decided, the parameter
shipped, and every symbol in those sentences stayed valid, so the argument went
on reading persuasively while being wrong. `STRATEGY-ENTITIES.md` then
contradicted itself ten lines later at `:286`.

Rewritten at three sites rather than deleted, because *why the parameter arrived
late* is worth keeping:

- **`STRATEGY-ENTITIES.md:276`** — recast as a departure that **closed**, naming
  what closed it, plus the general trade it is a cheap instance of: a scope
  filter is a question, and a question whose answer depends on an undecided
  policy is better refused than guessed. A `?building=` that had quietly meant
  `to_room` would have been a wrong answer nobody could see.
- **`STRATEGY-MCP.md:80`** — the *principle* survived the fact. It used to read
  "an absent parameter an agent can see explained is better than one it
  retries"; it now applies the same principle to the parameter that exists, which
  needs explaining just as much: a homeless door (empty `owner_rooms`) matches
  **no** building, so a building-scoped query silently omits them, and an agent
  that does not know that reads the omission as a data gap. Verified against the
  live tool description in `bin/mcp.rs`, which does say this.
- **`STRATEGY-MCP.md:44`** — the tool-list line, corrected to
  "project/building/milestone/property filters".

**The lesson for future weeks:** when something ships, the question is not only
"which facts changed" but "which document explains why this was absent". The
second kind survives every mechanical check.
