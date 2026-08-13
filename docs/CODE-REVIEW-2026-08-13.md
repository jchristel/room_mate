# Weekly review — 2026-08-13

Second run of `/weekly-review`. Previous: [CODE-REVIEW-2026-08-08.md](CODE-REVIEW-2026-08-08.md).

> **Outcome: the mechanical checks and all six CI gates are clean. Everything
> found this week is in the class no script sees** — twelve places where a live
> doc still describes work that has since shipped, or explains an absence with a
> reason that stopped being true. The 2026-08-08 review fixed 16 items of
> *name* rot; this one is entirely *status* rot, which is the more dangerous
> kind: every symbol in these sentences still exists, so they read as current.
>
> **Findings 1–10 are fixed** (2026-08-13, uncommitted) across `README.md`,
> `STRATEGY.md`, `STRATEGY-BROWSER.md` and `STRATEGY-ENTITIES.md`. **Findings 11
> and 12 are left open** by decision — both are new writing rather than
> correction, and what belongs in the docs about the DPR trap is a call to make
> deliberately. Findings are kept below as written, because the record of what
> drifted and why is the point.

Requested emphasis this week: **flag all documentation of implemented steps that
is no longer relevant.** That is Findings 1–8 below.

## Pass 1 — mechanical checks

`python scripts/weekly_review.py` → **0 items to judge.**

| Check | Result |
|---|---|
| Symbols named in live docs that exist nowhere | clean (19 known-benign ignored) |
| Stale `.md` paths cited from code or live docs | clean |
| HTTP read routes without an MCP tool | clean — 17 routes, 18 tools |
| Measured line counts in `CODING-CONVENTIONS.md` | within ±10% |

No triage needed: nothing to sort into rot / benign / checker bug. The
`resolve_project_settings` tool (2026-08-08, `92c4872`) closed the one open
judgement call from last week, and `bin/mcp.rs:9-12` now says "one per existing
HTTP read route is now literally true" and records why it had not been.

**The script found nothing this week and the review still found twelve items.**
Worth stating plainly: the four checks cover names, paths, parity and counts.
They do not cover *tense*, and tense is where this week's drift is.

## Pass 2 — the CI gates

The week touched both `src/` and `src-js/`, so all six ran.

| Gate | Result |
|---|---|
| `cargo test` | pass |
| `cargo fmt --check` | pass |
| `cargo clippy --all-targets -- -D warnings` | pass, no warnings |
| `npm run typecheck` | pass |
| `npm test` | pass — 92 tests, 7 files |
| `npm run build` + `git diff --exit-code -- static/vendor/` | pass, committed bundle matches |

## Pass 3 — the judgement calls

### Findings 1–8: shipped work still documented as unbuilt

Ranked wrong-before-stale. Three separate pieces of shipped work are involved:
**R4** (landed 2026-08-05, `c41c1fd`), **door ownership** (2026-08-06,
`90223e1`) and **the door glyph viewer** (2026-08-06/07, `087758e`, `8d4d3b1`,
`e24e070`).

---

**1. `STRATEGY-ENTITIES.md:404-416` — Decision 5 declares R4 unbuilt.** The
worst of the twelve, because it is the section that *is* R4:

> **Not built, on purpose — this is R4, and it is the one prerequisite doors
> shipped without.** […] the day R4 lands, that predicate starts *matching*

R4 landed 2026-08-05. `ReferenceSourceConfig::entity` is at
`src/settings/mod.rs:794`, documented there as "Entity-scoped since R4". An
agent reading Decision 5 to implement entity scoping would set out to build
something that exists.

Three-way self-contradiction inside one document: the header (lines 20-27)
retired a "what survived contact" summary *specifically* because it "went on
describing R4 as unbuilt for three days after it landed" — and the Deferred list
(line 694) correctly strikes R4 through. The summary was retired on the argument
that per-decision notes are where a reader actually looks. That argument only
holds if the per-decision note gets updated, and this one never was.

**Fix:** replace the blockquote with an *as built* note in the shape the other
five decisions use. The prediction it made is worth keeping — R4 was a settings
and wiring change rather than a grammar fork, and it held.

---

**2. `STRATEGY.md:41-43` — the index says door ownership is undecided.**

> **Decision 6's open question — which of a door's two rooms owns it — is still
> open**, and doors deliberately shipped without answering it.

Decided and built 2026-08-06: `[doors] room_attribution`
(`src/settings/mod.rs:349`), `owner_rooms` on `/doors`, `?building=` answered
through the owning room. `CLAUDE.md` carries it as a settled rule with its
default and its override.

Ranked second only because `STRATEGY.md` is the index — the first thing a reader
or agent opens. It also directly contradicts `STRATEGY-ENTITIES.md:700`
("**Decided and built** (Decision 6)"), both looking authoritative in isolation:
Pass 3 question 4 exactly.

---

**3. `STRATEGY-BROWSER.md:75-96` — "Doors are served but not drawn".**

> That is scope, not an oversight […] **When a viewer is built, doors get their
> own fetch, not a ride on the 2s room poll**

Contradicted by lines 21-36 of the *same document*, which describe the shipped
glyphs, their layer position and the bug that established it. And the prediction
came true: `pollDoors()` (`static/index.html:3732`) is its own fetch on its own
`revision`, in its own `try` — so the doc is forecasting, in future tense,
something it got right and shipped.

**Fix:** collapse into the door-glyph bullet at line 21. The `model_id` warning
at lines 92-96 must survive the merge — it is a live rule for anyone touching
the door→room join, not a note about an unbuilt viewer.

---

**4. `STRATEGY-ENTITIES.md:707` — Deferred: "Any door viewer".**

> **Any door viewer.** `/doors` is served and nothing draws it

`src-js/renderer/gl/doorGlyph.ts` is 397 lines with its own test file; doors are
a `FillBatch` in the layer order `grid → fills → hover → outlines → doors →
labels`; they are selectable; there is a `doorsToggle` button. The two entries
above it in the same list were correctly struck through when they closed. This
one was missed.

---

**5. `STRATEGY-ENTITIES.md:686` — the "Where the rest of this lands" table.**

> | [Browser](STRATEGY-BROWSER.md) | ✅ doors are served and not drawn […] |

The ✅ means "this doc was updated", but reads as "this is verified current".
Same fact as Finding 4, in the more confident presentation.

---

**6. `STRATEGY-ENTITIES.md:546` — R4 examples marked "still design".**

> The `[sources.reference.hardware]` and `[[builtin_properties]] entity`
> examples below are R4, and are still design — see Decision 5.

Both are now real config the loader parses. Note this line *points at* Finding 1,
so fixing Decision 5 without fixing this leaves a live pointer to a corrected
section still asserting the uncorrected claim.

---

**7. `STRATEGY-ENTITIES.md:536` — `door_label`'s reason is dead.** The purest
specimen of Pass 3 question 1, and worth reading closely:

> `door_label` is still absent: it has no viewer to feed.

The **fact is still true** — `door_label` appears nowhere in `src/`,
`settings/`, `src-js/` or `static/`. The **reason is false**: there is a viewer
now. Every symbol in the sentence resolves, so no symbol check can ever see
this. The effect is that `door_label` has quietly become undocumented deferred
work: the one recorded argument for its absence has evaporated, and nothing
replaced it.

**Fix:** either state a current reason, or move it to Deferred as a real item.
It is now the smallest remaining gap between doors and rooms in the viewer.

---

**8. `README.md:17` — the Entities index row.**

> read its "Deferred" list for what is still open, chiefly R4 and which room
> owns a door

Both named items are closed, and the Deferred list it points at strikes both
through. The index sends a reader looking for open work at the two things most
firmly settled.

---

### Finding 9: `README.md` contradicts itself about the phasing plan

`README.md:25` heads the phasing plan row:

> **Built (P1–P7), extractor half unverified against Revit.**

`README.md:58-60`, in the same file, thirty lines down:

> `PLAN-phasing.md`, closed once the extractor half was finally run against a
> real Revit document (2026-08-03) — which found the failure the plan had named
> as the first thing to check

`CLAUDE.md` agrees with the second: verified 2026-08-03, and it cost five empty
pushes. Line 25 is the stale one. This is drift the 2026-08-08 review did not
catch because it was hunting names, not claims.

### Finding 10: a deferred item's analogy inverted

`STRATEGY-ENTITIES.md:719-722`, "Verifying the extractor against Revit":

> the `get_FromRoom` accessor and the `OST_Doors` collector need a live
> document. **Same standing as the room extractor's phase filter.**

The item itself is **correct and should stay** — the doors exporter genuinely is
unverified against a live Revit document. The comparison is what broke: the room
extractor's phase filter was verified on 2026-08-03, so "same standing" now
points at something settled. Rewrite the last sentence, keep the item.

### Finding 11: the DPR fix landed with no doc change

`635f9a2` (2026-08-11) fixed three bugs from one root cause — Pixi v8
`renderer.width` is the **logical** size, and the drawing buffer is that times
the resolution — and touched only `src-js/`. Nothing in `docs/` records it.

This is the class of lesson `STRATEGY-BROWSER.md` exists to hold: the commit's
own reasoning is that the mistake "is invisible in code review and obvious on
screen", which is the argument that file already makes about `fitViewToAspect`.
It is now the **second** bug of exactly that shape in exactly that area, and the
fix's real guard — the new label transform takes no DPR parameter at all, so the
bug is no longer expressible in the signature — is a design move worth recording
rather than rediscovering.

Adjacent but not contradicted: `STRATEGY-BROWSER.md:723-726` says "DPR barely
matters here". That is about fill *cost* and is still true; it is not the same
claim, but it is the only mention of DPR in the doc, so a reader searching for
DPR guidance currently finds only the performance note.

### Finding 12: an open bug lives outside the docs

The door glyphs are reported invisible on large plans (noted 2026-08-11, scale
measured, cause not yet explained). No "Open items" section in `docs/` mentions
it. `README.md:46-52` states the rule this violates: open items belong in the
strategy doc that owns them — here `STRATEGY-BROWSER.md`'s "Open items /
things to watch".

---

### The five standing questions

**1. Did a change make a *rationale* stale rather than a fact?** **Yes — three,
and one is a textbook case.** Finding 7 (`door_label` "has no viewer to feed")
is the type specimen the skill describes: the fact holds, the argument is dead,
every symbol resolves. Findings 3 and 6 are the same shape with the fact broken
too. The question to ask of this week's work — *which document explains why this
thing is absent?* — has an answer for the door viewer, door ownership and R4,
and all three answers are now wrong.

**2. Did a frontend change add a mutable global to `static/index.html`?**
**No — and the opposite happened.** `static/index.html` is untouched since
2026-08-08 (45 top-level `let`/`var`, against ~44 last week — no change from
this week's work). The DPR fix moved the label transform *into*
`src-js/renderer/gl/viewport.ts` with a test pinning the property that matters,
which is `CODING-CONVENTIONS.md`'s standing rule working as written. The
`doorsPayload` / `lastDoorsRevision` / `showDoors` globals noted last week are
still there; nothing was added to the pile.

**3. Is a newly-long module missing its "decided, not deferred" header?**
**Unchanged and still open.** `service/rooms.rs` (2,506 lines) and
`service/areas.rs` (2,051) were named specifically in
`CODING-CONVENTIONS.md:66-69` and still have no header saying they stay whole
on purpose. `rooms.rs`'s header documents its *extraction* from `handlers.rs`,
not a decision to remain one file. `adjacency.rs` (1,264) remains the only one
that does it. No new module crossed the trigger this week.

**4. Do two live docs now disagree with each other?** **Yes, three pairs** —
Findings 2 (`STRATEGY.md` vs `STRATEGY-ENTITIES.md`, door ownership), 9
(`README.md` against itself, phasing verification) and 3
(`STRATEGY-BROWSER.md` against itself, doors drawn). Two of the three are a
document contradicting *itself*, which the script cannot see and a reader will
not, because nobody reads a 854-line doc end to end.

**5. Is anything in `docs/` finished enough to supersede?** **No whole document
— but two blocks have outlived their build**: Decision 5's "Not built, on
purpose" blockquote (Finding 1) and the "Any door viewer" deferred entry
(Finding 4). Both are in-flight scaffolding of exactly the kind the question
asks about. No live doc's "Open items" section is down to nothing:
`STRATEGY-MCP.md` is at one, `STRATEGY-AREA-CALCULATION.md` and
`STRATEGY-BROWSER.md` still carry real items. `STRATEGY-AUTHORED.md` remains
correctly marked design-not-built.

## What was fixed

Findings 1–10, in four passes. Docs only — no code changed, so the Rust and
frontend gates did not need re-running; the drift script was re-run and stays
clean.

1. **`STRATEGY-ENTITIES.md` — Findings 1, 4, 5, 6, 7, 10.** Decision 5's
   blockquote became an *as built* note in the shape the other decisions use,
   keeping the two things worth keeping: the scope prediction that held, and the
   circular trigger as a lesson about triggers that name downstream events. The
   R3 declaration-order constraint on `entity` is recorded there too, since it is
   the one thing the sketch did not anticipate. "Any door viewer" is struck
   through in Deferred with what shipped; the cross-doc table row now records the
   fetch decision instead of the retired one; and the extractor-verification item
   keeps its content with the inverted analogy rewritten into the argument it
   should have been.
2. **`STRATEGY.md` — Finding 2.** The index now states `room_attribution` and
   its default chain rather than an open question.
3. **`STRATEGY-BROWSER.md` — Finding 3.** The served-but-not-drawn bullet is now
   a doors-poll-separately bullet describing `pollDoors()`. Both things worth
   preserving survived: the `model_id` join rule verbatim, and the fact that the
   fetch decision was written before there was a viewer and held unchanged.
4. **`README.md` — Findings 8 and 9.** The Entities row points at the two items
   actually open; the phasing row no longer contradicts the same file thirty
   lines down.

## Left open, by decision

- **Finding 11** — the DPR lesson is unrecorded. Deliberate: how much of it
  belongs in `STRATEGY-BROWSER.md` is a judgement about what that doc is for.
- **Finding 12** — the glyph-visibility issue is still not in any Open items
  section, and its cause is not yet explained. Worth writing up once it is,
  rather than logging a symptom.

## One note on the checker

Every finding above is a **status claim in prose**, and the script's four checks
are structurally incapable of seeing any of them. Not a defect — the header of
`weekly_review.py` is explicit that live docs legitimately name unbuilt designs,
which is why this can never be a gate. But it means a clean script run says
nothing about tense drift, and tense drift is what a week of shipping produces.

The tractable sliver, if it is ever worth automating: a doc asserting **"not
built"** or **"still open"** about a named thing, where that thing has an
entry in a *Deferred* list that is struck through, is a mechanical
contradiction. That would have caught Findings 1, 4 and 8. It would not have
caught Finding 7, and nothing ever will.
