# RoomMate — Coding Conventions

The engineering rules this codebase already follows, in one place so future
work (and future handovers) can point at them instead of re-deriving them.
These are *conventions*, not architecture — the "why it's shaped this way" of
the pipeline lives in the [STRATEGY docs](STRATEGY.md); this is the "how we
write it" that sits underneath.

## Module structure & length
- Judge a module's size by its **non-test** lines (tests live inline — see
  below — and legitimately dominate a small module).
- A module past **~500 real lines**, or one mixing distinct concerns (types +
  logic + IO), is a split candidate. Split a `foo.rs` into a `foo/` directory
  with `mod.rs` plus per-concern files.
- **Re-export public items from `mod.rs`** (`pub use fs::FsStore;`) so the
  public path (`crate::foo::Bar`) never moves — a split then touches zero call
  sites outside the module. `storage/` and `settings/` are the worked examples.
- A type's inherent `impl { fn validate() }` stays *with the type*; only
  standalone free functions move to a sibling file (see `settings/`).

**Where the codebase actually stands (measured 2026-07-26).** Eight modules are
past the ~500 real-line trigger — `service/areas.rs` (1,072), `service/rooms.rs`
(1,021), `settings/mod.rs` (804), `service/adjacency.rs` (748), `handlers.rs`
(586), `bin/mcp.rs` (551), `settings_api.rs` (531), `contract.rs` (519). That is
not automatically a defect: the trigger is "worth a second look", not a limit.
But only `adjacency.rs` writes down *why* it declined to split, and a rule
nothing is measured against stops being a rule. Two of the eight are worth
naming specifically:

- **`settings/mod.rs` at 804 lines is the one that reads as unfinished.** The
  `settings/` split was done — and is cited above as the worked example — yet
  `mod.rs` kept the bulk rather than becoming the thin re-export the pattern
  describes. The types are already grouped by concern in the file (area policy,
  dRofus field config, hierarchy, milestones, colour plans), so the seams exist.
- **`areas.rs` and `rooms.rs` are cohesive by argument, not by accident** — one
  geometry pipeline and one assembly pipeline respectively, each with an
  invariant only checkable by reading the parts together. If they stay whole,
  say so in the header the way `adjacency.rs` does, so the next reader knows it
  was decided rather than deferred.

## Tests
- Unit tests live **inline** as `#[cfg(test)] mod tests` at the bottom of the
  file they exercise — not in a separate `tests/` tree. They move with their
  code when a module is split (a small shared helper like `payload(...)` is
  duplicated into each sub-module rather than hoisted).
- Test-drive behaviour end-to-end where it's cheap: `FsStore`-backed tests when
  history matters, `MemStore` when it doesn't.

## Long functions
- A long function is a bigger smell than a long module. The `too_many_lines`
  clippy lint (in `Cargo.toml`, `warn` — fires >100 lines) surfaces them.
- **Clippy is a CI gate**: `.github/workflows/rust.yml` runs
  `cargo clippy --all-targets -- -D warnings` alongside build and test, so the
  tree is warning-free and stays that way. `--all-targets` because most of this
  crate's lines are inline test modules, and they drifted just as far as the lib
  did while nothing was watching.
- **`-D warnings` does not make `too_many_lines` a hard limit.** The escape
  hatch stays exactly what it always was — `#[allow(clippy::too_many_lines)]`
  **with a reason** — and an explicit allow compiles clean under `-D warnings`.
  A long-but-cohesive function is still permitted; it just has to say so out
  loud. `settings_api::tests::test_settings_toml_round_trip` (118 lines) is the
  worked example: it asserts that the *whole* `Settings` shape round-trips
  through TOML at once, so splitting it per section would test each against a
  separately built document and lose the value-before-table ordering property
  (see the TOML footgun below) that only bites when sections are interleaved.
- Extract helpers only at **seams that improve clarity** — a natural phase
  boundary with a nameable input/output — never mechanically to hit the line
  count. `assemble_rooms` (scope → dedup levels → assemble) and
  `compute_validation` (resolve link values → compare fields → coverage) are
  the worked examples. A helper called once that just relocates a block without
  clarifying it is not worth it; a cohesive function that trips the lint carries
  `#[allow(clippy::too_many_lines)]` with a reason instead.
- Factor a repeated or gnarly type into a `type` alias with a doc comment
  (`ScopedPayload`, `LinkValueIndex`) — it reads better *and* silences clippy's
  `type_complexity`.

## `static/` has no conventions yet — and that is now the open question
These rules are Rust-only. `PLAN-handover-actioning.md`'s P10 flagged that gap
and deliberately left it a question rather than answering it by accretion, when
`index.html` was 2,020 lines. It is now **3,941** (~3,260 of them one inline
`<script>`), and two extractions have happened since without a rule prompting
them — `common.js` (the palette and the classification-path vocabulary, moved
because two views disagreeing about a group's identity is worse than either
being arbitrary) and `graph.js` (moved because the concern boundary is a
different renderer). Both were pulled by a *specific* argument, which is the
honest pattern so far: **extract when two consumers must agree, or when the
boundary is a genuinely different concern — not to hit a line count.** The
zero-build vanilla constraint (STRATEGY-BROWSER) is what makes anything more
aggressive expensive. If a rule is ever written down, that is the one the code
already follows.

## Dependency direction is the seam
- `service/` is transport-agnostic: it never imports `axum`, `rmcp`, or
  `reqwest`. `handlers` (HTTP) and `bin/mcp.rs` (MCP) are thin adapters — each
  extracts its own input form, calls exactly one `service` function, and
  translates the result to its wire format.
- Neither binary's transport crate leaks into the other: `main.rs` never
  imports `rmcp`/`reqwest`, `mcp.rs` never imports `axum`. (An HTTP *client* in
  `mcp.rs` is fine — the rule is about server frameworks.)
- `bootstrap` (settings files → a running `AppState`) is shared verbatim by
  both binaries so they can't drift on how the store is picked or data is seeded.

## "Signal, not error"
A reference that *should* resolve but doesn't is usually a represented state to
surface, not a hard failure — pick the shape that fits:
- **soft-empty** (unknown/unregistered project → `[]`/`204`, not `404`);
- **skip + warn** (a milestone pin to a since-deleted snapshot → drop that
  model, log it);
- **fall back + warn** (a milestone's dRofus pin that's missing/unparseable →
  join the current dRofus instead).
An unmatched dRofus key, a room↔level mismatch, and a partially-classified room
are all diagnostic signals, not errors.

## Loud startup over silent no-op
- Config mistakes fail the boot with a **specific** message (an unkeyable
  hierarchy tier, a `drofus_fields` label the CSV never declared, a malformed
  strftime pattern) rather than silently misbehaving on the first request.
- The settings-save path re-runs the **exact** startup validation
  (`bootstrap::load_project_bundle`) before installing a file, so a file the API
  accepts can never fail the next boot; a rejected save leaves the old file
  untouched.

## Identity & the upload envelope
- Ids are immutable keys; names are display-only. Tie storage to the id (the
  Revit model GUID, the project id), never the name, so a rename doesn't fork
  the record.
- Every snapshot id resolves **only** through `contract::ensure_taken_at` /
  `validate_snapshot_id` (RFC3339 UTC, omittable → server-minted, echoed back),
  never a reimplementation. Any new upload type rides the same envelope.
- **Validate uploaded data before storing it** — a stored artifact is hydrated
  at every boot, so accepting a bad one turns a bad upload into a failed
  startup. (See the dRofus upload path.)

## Line endings
- `*.rs`, `*.py`, and config files are **LF** in the working tree, enforced by
  the repo-root `.gitattributes` (`* text=auto` plus per-extension `eol=lf`).
  Windows-only scripts (`*.bat`/`*.cmd`/`*.ps1`) stay CRLF. A file that drifted
  to CRLF is renormalized with `git add --renormalize`.

## Annotate the "why"
- Module headers and non-obvious code carry the **rationale**, not a restatement
  of the what. The house style is a doc comment that explains the decision (why
  this seam, why this fallback, what would break otherwise) — the thing a future
  reader can't recover from the code alone.

## TOML footgun (serialize side)
- A scalar struct field serialized *after* a map/sub-table field lands inside
  that `[table]`, not the parent. Declare scalar fields **before** any
  map/sub-table field in the struct (e.g. `Milestone.drofus_snapshot` before
  `attachments`) so the round-trip through the settings API stays correct.
