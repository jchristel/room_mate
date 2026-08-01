# RoomMate — working notes

Revit → Rust → browser room data pipeline. The **reasoning** lives in
[`docs/`](docs/README.md) — this file holds only what is expensive to get wrong
and impossible to infer from the code. Don't duplicate the docs here; link them.

## ⛔ Before starting doors

Doors have prerequisites, in
[PLAN-generalisation.md § The line in the sand](docs/PLAN-generalisation.md#the-line-in-the-sand):

- **R2 (lift the property lookup off `&Room`) lands *before* the `Door` contract
  is final.** Its open question — does a door's instance property *shadow* its
  type property, or is a name in both a finding? — is a **contract** decision.
  Decide it after `Door` is written and you rewrite the type.
- **R1 (generalise `SnapshotStore` off `RoomPayload`) lands *with* doors.** The
  moment `put_doors` appears beside `put`, the third parallel method set exists
  and FFE makes it a fourth. `AppState` holds `Box<dyn SnapshotStore>`, so the
  trait must stay **object-safe** — a generic `put<T>` is out.
- **R4 (entity-scope `[sources.reference.*]`) lands with doors' first reference
  source** — not before (needs R2), not later (back-compat obligation).

## Which document wins

[`docs/README.md`](docs/README.md) indexes everything. Two supersessions matter:

- **Phase:** [`PLAN-phasing.md`](docs/PLAN-phasing.md) is authoritative over
  `STRATEGY-ENTITIES.md` Decision 2.
- **Generalisation:** [`PLAN-generalisation.md`](docs/PLAN-generalisation.md)
  supplies the signatures `STRATEGY-ENTITIES.md` Decisions 3 and 5 only assert.

Docs pin `file.rs:NNN` line numbers that have drifted — trust the *symbol* name,
search for it, never jump to the line.

## Verify before claiming done

```
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

All three are CI gates; clippy runs with `-D warnings`, so a warning is a
failure. Frontend changes are verified by driving the page, not by reading the
diff — a bug shipped this week was only visible after expanding a panel.

## House rules the code won't tell you

- **Tests are inline** — `#[cfg(test)] mod tests` at the bottom of the file they
  exercise, never a `tests/` tree. A shared helper is duplicated per module
  rather than hoisted.
- **Doc comments carry the *rationale*** — why this seam, what breaks otherwise,
  what was rejected. Not a restatement of the what. This is the single most
  visible house style; matching ordinary Rust terseness reads as foreign.
- **`service/` is transport-agnostic** — never imports `axum` or `rmcp`.
  `handlers.rs` (HTTP) and `bin/mcp.rs` (MCP) are thin adapters over it, and
  `bin/mcp.rs` keeps one tool per HTTP *read* route (update its count when you
  add one).
- **"Signal, not error"** — an unresolved cross-reference is usually a reported
  state, not a failure.

## Traps

- **Line endings are LF**, enforced by `.gitattributes`. Writing files through a
  Python heredoc on Windows silently converts them to CRLF — check with
  `git diff --stat` (it warns) after any scripted file write.
- **Contract is v6 and `phase` is required.** A hand-rolled test push without it
  gets a 422 naming a stale extractor.

## Open, as of 2026-08-01

- **The pyRevit extractor's phase filter is unverified against Revit.** Check
  first that `FilteredElementCollector`'s element ids match the ids duHast
  writes into the export — if they don't, the filter silently keeps nothing.
  See [`PLAN-phasing.md`](docs/PLAN-phasing.md) "As built".
