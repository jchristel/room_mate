---
name: weekly-review
description: Weekly drift review for RoomMate — runs the mechanical docs-vs-code checks, verifies the CI gates, then works a short human checklist of the judgement calls a script cannot make. Use when the user asks for the weekly review, a drift check, or a docs-vs-code audit.
---

# RoomMate weekly review

This repo's documentation is its main asset and its main liability. The docs
assert a great deal that is specific and falsifiable — symbol names, route
counts, measured line counts, "X takes no `?building=`" — so they carry a large
surface that a *later* change can silently falsify. A 2026-08-07 review found
~20 such items, and all but a handful were mechanically detectable.

So the review runs in three passes, cheapest first. **Do not skip to the human
pass** — the script's output is the input to it.

---

## Pass 1 — mechanical checks

```bash
python scripts/weekly_review.py
```

Four checks: symbol liveness, `.md` path liveness, HTTP-route/MCP-tool parity,
and drift in `CODING-CONVENTIONS.md`'s measured line counts. Stdlib only, runs
in about a second, no server needed.

**Its output is a list to judge, not a list to fix.** Every hit is one of three
things, and telling them apart is the reviewer's job:

| Verdict | What to do |
|---|---|
| **Rot** — the doc was true and a later change falsified it | Fix the doc |
| **Benign** — names an external API, a rename it is explaining, or a design deliberately not built | Add to `scripts/weekly_review_ignore.toml` **with a reason** |
| **Checker bug** — the script resolved the wrong thing | Fix the script, and say so in its comments |

The ignore file converging on zero *unexplained* hits is the goal. An entry with
no reason is worse than no entry: nobody can later tell one that is still true
from one that quietly stopped being true.

**Never make this a CI gate.** Live docs legitimately name symbols that do not
exist — when explaining a rename, recording a rejected design, or describing
something settled but unbuilt. A gate would punish good writing, and the
pressure would be to delete the history rather than fix the rot.

---

## Pass 2 — the CI gates

Only if the week's work touched the relevant tree. These are the same gates
`.github/workflows/` runs, so a failure here is a red PR either way.

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```

Touched `src-js/`? Then also — and `npm run build` is **not** optional, because
`static/vendor/renderer.bundle.js` is a generated file that is committed:

```bash
npm run typecheck && npm test && npm run build && git diff --exit-code -- static/vendor/
```

That last `git diff` is the whole point: a stale committed bundle fails silently,
serving an old renderer with no error anyone can act on.

---

## Pass 3 — the judgement calls

Five questions. None is answerable by grep; all five have bitten this repo.

### 1. Did a change make a *rationale* stale rather than a fact?

**The highest-value question here, and the one nothing automates.** The type
specimen: `STRATEGY-MCP.md` did not merely state that `get_doors` had no
`?building=` parameter — it *argued* the absence from "Decision 6's open
question". Decision 6 was then decided, the parameter shipped, and the argument
kept reading perfectly plausibly while being entirely wrong. A symbol check
cannot see this, because every symbol in the sentence still exists.

Ask it of anything that shipped this week: **which document explains why this
thing is absent, partial, or deferred?**

### 2. Did a frontend change add a mutable global to `static/index.html`?

`CODING-CONVENTIONS.md`'s standing rule is that each frontend change moves the
module it touches into `src-js/`. It is being half-honoured: the door glyph work
put real geometry into `src-js/renderer/gl/doorGlyph.ts` with tests, and *also*
added `doorsPayload`, `lastDoorsRevision` and `showDoors` to the ~44 mutable
globals in the page. The computation migrates; the state does not. Note it when
it happens rather than re-discovering the pile later.

### 3. Is a newly-long module missing its "decided, not deferred" header?

`CODING-CONVENTIONS.md:54-57` asks that a module which stays whole past the
~500-line trigger say so in its header, the way `service/adjacency.rs` does.
`rooms.rs` and `areas.rs` were named specifically and still have not.

### 4. Do two live docs now disagree with each other?

The script checks docs against *code*, never against each other.
`STRATEGY-ENTITIES.md` says R4 is unbuilt and cites `STRATEGY-SOURCES.md` as its
evidence; `STRATEGY-SOURCES.md` marks R4 **Closed**. Both look authoritative in
isolation. Worth one pass over any pair of docs the week's work touched.

### 5. Is anything in `docs/` finished enough to supersede?

The `Superseded/` discipline works well — check whether a live doc's
"Open items" section is down to nothing, or whether an in-flight scaffolding
block ("What survived contact") has outlived the build it was written for.

---

## Output

Write findings to `docs/CODE-REVIEW-<YYYY-MM-DD>.md` (untracked unless the user
asks to commit), and report in chat:

- what the script found, **already triaged** into rot / benign / checker bug —
  do not just paste its output;
- which gates ran and their result, stated plainly including failures;
- the answers to the five questions above, with "nothing this week" as a
  perfectly good answer;
- a suggested work order, most-consequential first.

Prefer "wrong" over "stale" when ranking: a doc that misstates a fact an agent
or a reader will act on beats a doc that merely points at a moved file.
