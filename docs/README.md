# RoomMate — Documentation

Design and strategy documentation for the RoomMate application, capturing the
design decisions behind the Revit → Rust → browser room data pipeline.

## Strategy

| Document | Description |
|---|---|
| [Architecture & Strategy](STRATEGY.md) | Index and overview of the design decisions |
| [Browser](STRATEGY-BROWSER.md) | Browser front-end strategy |
| [Server](STRATEGY-SERVER.md) | Rust server strategy |
| [Area calculation](STRATEGY-AREA-CALCULATION.md) | How rooms become per-tier areas: boundary regimes, the wall zone, what a wall belongs to, and the relationship to IPMS 3 / DIN 277 |
| [Sources](STRATEGY-SOURCES.md) | Data sources strategy |
| [MCP](STRATEGY-MCP.md) | Model Context Protocol integration strategy |
| [Authored](STRATEGY-AUTHORED.md) | User-authored data & documents — connections, PDFs, hierarchy scopes _(design settled, not built)_ |
| [Entities](STRATEGY-ENTITIES.md) | What makes something a primary entity, what generalizes from rooms to doors and FFE, and how a Revit phase scopes a push _(**built** — each decision now records what shipped and where it departed from the sketch; read its "Deferred" list for what is still open, chiefly R4 and which room owns a door)_ |
| [Security](STRATEGY-SECURITY.md) | Threat model for the LAN-reachable deployment: trust boundary, invariants, settings backups, rate limiting |

## Implementation notes

| Document | Description |
|---|---|
| [Coding Conventions](CODING-CONVENTIONS.md) | The engineering rules this codebase follows (module structure, testing, dependency direction, error stance) |
| [Plan: phasing](Superseded/PLAN-phasing.md) | **Built (P1–P7), extractor half unverified against Revit.** The ten decisions behind Revit phase support and the phases that implemented them, plus an "As built" section recording where the result departs from the plan |
| [Plan: generalisation](Superseded/PLAN-generalisation.md) | **R1 through R4 all landed** (R4 on 2026-08-05). Four seams that grew room-shaped because rooms were the only primary entity. The two doors prerequisites landed ahead of any door code — read R2's outcome note for the tier-precedence rule the `Door` contract is written against (a tier wins only when it is `Present`), and R1's for why `list_models()` still means "models with rooms". R4's own trigger turned out to be circular, which its header records |

> **Reading either plan above: do not trust their `file.rs:NNN` deep
> links.** Both pin line numbers, and the files have moved underneath them —
> spot-checked 2026-07-26, most now land on unrelated code (e.g.
> `areas.rs:270` was cited as "`/areas` has no `revision` field" and is now
> `wall_zone`). The *file* and the *symbol name* in the surrounding prose are
> still right; search for the symbol rather than jumping to the line. New
> cross-references should name the symbol, not the line.

## Open handovers

**None.** The last one — the [door direction
glyph](Superseded/HANDOVER-door-glyph.md) — landed on 2026-08-07. Read its
header before writing another brief of that shape: every *scope decision* in it
held, and most of its *estimates* did not. It said "no service work, one new
field" for something whose real cost was that the viewer had no door plumbing at
all, and two of its instructions ("append into the room vertex stream",
"point-in-rectangle") each produced a bug that only showed up on screen.

That is a statement about *handovers*, not about outstanding work — open items
otherwise live in the strategy doc that owns them rather than in a brief that
outlived its build. `HANDOVER-adjacency.md` is the example:
its two remaining false-positive checks went into
[Area calculation](STRATEGY-AREA-CALCULATION.md)'s "Open" section, beside the
`max_wall_thickness` value whose choice is what creates the risk they test.
Keeping them in a superseded brief would have buried a live item in an archive.

Handoff documents whose work has fully landed live in
[Superseded](Superseded/), and so do **all four plans**: `PLAN-handover-actioning.md`,
closed with P1 through P10 all landed and every handover it reviewed superseded
alongside it; `PLAN-webgl-renderer.md`, closed with P0 through P6 landed and both
renderer handovers superseded beside it; and `PLAN-phasing.md`, closed once the
extractor half was finally run against a real Revit document (2026-08-03) — which
found the failure the plan had named as the first thing to check; and
`PLAN-generalisation.md`, closed when R4 landed. Each is kept as the record of
how the work was sequenced and what was deliberately *not* built.

**No plan is live any more.** `PLAN-generalisation.md` joined them on
2026-08-05 when R4 landed — its "wait for doors' first reference source" trigger
turned out to be circular, since nothing could declare such a source until the
config could express one.

Four of those are worth knowing about even though they are superseded,
because each leaves something recorded rather than pending:

- **[Room inspector](Superseded/HANDOVER-room-inspector.md)** — step 6, a
  checkbox property picker, was deliberately not built. Hide-empty plus the name
  filter covered the cases it was for. Act on it only if the need shows up.
- **[Viewport culling kill switch](Superseded/HANDOVER-culling-disable-switch.md)**
  — **the switch took its own advice.** It said to delete culling if anything
  ever made it redundant rather than leave it switched on; WebGL did, and both
  the cull and `CULL_ENABLED` are gone (2026-08-02). The document is kept for its
  method and its number — **16.5 ms/frame with culling, 912 ms without** — which
  is the honest reason the feature was worth having right up until it wasn't.
- **[WebGL plan renderer](Superseded/PLAN-webgl-renderer.md)** — records what the
  plan got *wrong*, which is the useful half: the shader it feared was not what
  broke the frame budget (5,046 label transforms were, fixed by one line), and
  the label build cost it expected to be a regression is within 2% of SVG end to
  end. What held: extract the appearance decision first, put a seam in before the
  swap, freeze golden files for the export — those survived all six phases
  byte-identical.
- **[QA coverage of the secondary source](Superseded/HANDOVER-qa-cardinality-and-coverage.md)**
  — closed, but it records a mis-diagnosis worth not repeating: two drafts read
  `duplicate_link_values` as "the reference source has duplicates" when it means
  "the rooms do", and so proposed a cardinality *setting* while the loader was
  silently discarding duplicate and blank-id rows. The two sides of a join are
  different questions; a check named after the join does not say which side it
  inspects.

Area calculation no longer has a handover: its design moved into
[Area calculation](STRATEGY-AREA-CALCULATION.md), which is the live document and
carries the remaining open items in its "Open" section. The handover stays in
`Superseded/` as the record of how the decisions were reached.
