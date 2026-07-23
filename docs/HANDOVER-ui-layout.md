# RoomMate — Handover: UI layout restructure

> **Status (2026-07-23): sequencing steps 1, 4 and 6 have LANDED** (see
> PLAN-handover-actioning.md P6, P2, P3).
> **Step 1 (scope migration / Decision 1)**: project/milestone/building are
> global header state; zones carry level + colour + areas only; one poll,
> one colour-plan read, one validation state; `persistSelection` lost its
> `zones[0]` special case — the open question ("does anything else depend on
> per-zone scope?") resolved cleanly, nothing did.
> **Step 4** landed with one correction: the label set rides `RoomsResult`
> **keyed by project id** (`drofus_labels`), not flat — the claim below that
> the response "is already project-scoped" is wrong for the unscoped merge.
> **Step 6** is the labels toggle, threaded through `paintLevel` so export
> honours it.
> **Still open: steps 2, 3, 5** — the bottom region, the panel migration,
> and the source-data grid (Decisions 2–3 below).

Design settled; steps 1, 4 and 6 built (see status above), the bottom
region and grid not yet. This document exists so the remaining work can be
picked up cold. Companion to [Browser](STRATEGY-BROWSER.md), which describes
the viewer as it stands today; when this lands, that doc absorbs the outcome
and this one moves to `Superseded/`.

The trigger was small — "add a show-labels on/off toggle" — but answering
*where the button goes* had no principled answer, which is the actual problem
this addresses.


## The problem

The viewer's UI has grown by accretion and its regions no longer carry
consistent meaning:

- **Header** holds genuinely global controls (add/remove zone, link views,
  room search) — this part is coherent.
- **Zone toolbar** holds a mix: `projectSelect` / `milestoneSelect` /
  `buildingSelect` (scope — *what data am I looking at*) alongside
  `levelSelect` / `colourSelect` / `areasTier` (presentation — *how is it
  drawn*). Two different kinds of control in one strip.
- **Right side** holds `.validation-panel` and `.areas-panel`, both
  absolutely positioned *inside* `.zone-canvas` — overlays that cover the plan
  rather than layout participants that reflow it. Both move to the bottom
  (Decision 2); the right side is repurposed for a room inspector
  (Decision 3).

The consequence is that a new control has no obvious home. "Should the labels
toggle be global or per-zone?" was genuinely ambiguous, and that ambiguity is
a symptom, not a one-off.

A further constraint from the discussion: the **bottom of the screen is
reserved** for future data interaction, and should not be spent casually.


## The target model

Three regions, each with one meaning:

| Region | Meaning | Holds |
|---|---|---|
| Top | *What am I looking at* — global chrome | Scope pickers, view prefs, search |
| Right | *What is this thing* — inspector | Per-room properties (future) |
| Bottom | *What am I doing to it* — workbench | Tabular/temporal data |

This is the IDE/CAD layout (Revit, Rhino, every DCC tool converges on it),
minus its left navigator — chosen because users already hold this mental
model, so region position becomes self-documenting.

**There is deliberately no left region.** See "Why no navigator" below; a
region reserved without a purpose invites something to be put there for
symmetry.

**The zone toolbar splits along the scope/presentation line.** Scope moves to
the header and becomes global; presentation stays on the zone. The zone
toolbar shrinks to level + colour + areas + meta — arguably no longer a
toolbar but a caption strip.

Concretely, with two zones open:

```
┌─ header ─────────────────────────────────────────────────────┐
│ [Project ▾] [Milestone ▾] [Building ▾]  │ Labels │ Link │ 🔍 │ + − │
├─ main ────────────────────────────┬──────────────┬───────────┤
│ [Level ▾] [Colour ▾] [Areas]      │ [Level ▾] ...│           │
│ meta line                         │ meta line    │ inspector │
│                                   │              │           │
│              plan                 │     plan     │  (room    │
│                                   │              │   props)  │
├───────────────────────────────────┴──────────────┴───────────┤
│ ▸ QA · 3 mismatches                    results (sometimes)   │
├──────────────────────────────────────────────────────────────┤
│ Rooms  [Model ☑] [dRofus ☑]        source data (always)      │
└──────────────────────────────────────────────────────────────┘
```

`body` becomes `grid-template-rows: auto 1fr auto`; the middle row becomes
`grid-template-columns: 1fr auto` — zones, inspector.

**Inspector and bottom region are both siblings of `main` — one each, never one
per zone.** A region that multiplies with zone count stops being a stable place
users can point at. Only the caption strip is per-zone.

Every zone carries its own level and colour pickers; there is one set of
scope pickers for the whole page. The caption strips must read as *belonging
to* their zone rather than as a second global toolbar — left-align each tight
against its zone's own left edge and keep it visually lighter than the header.

The inspector is **future work and absent for now** — it cannot be built until
room click-selection exists (the viewer has hover titles today, not selection).
The CSS layout must tolerate a zero-width or `display: none` inspector column
without the zones reflowing oddly.

Throughout this document, "grid" means the band-2 data table unless it is
explicitly a CSS `grid-template-*` declaration.

### Why no navigator

The left region is dropped rather than reserved, because every candidate for it
is already served:

- **Level tree** — the zone's own level picker does this, and better: levels
  are per-zone by design.
- **Project / building tree** — that is scope, which Decision 1 just moved to
  the header. Putting it left as well reintroduces exactly the duplication this
  restructure removes.
- **Room list** — tabular, so it belongs in the bottom region; it is precisely
  what band 2 is (Decision 2). Its access pattern is search and filter, not
  tree-browsing.
- **Zone manager** — the `+` / `−` buttons are sufficient for two or three
  zones. A panel to manage three things is overhead.

The general rule: a navigator earns its place when there is a **large,
hierarchical set the user browses to reach one thing**. This app's hierarchy is
project → building → level — three shallow tiers, fully covered by two header
pickers and one zone picker. There is nothing to browse.

**The one plausible future trigger** is browsing by *classification* rather
than by level: the server resolves an N-tier path per room and `areas.rs`
already groups footprints by tier, so "click Surgery, see its rooms across all
floors" is genuinely tree-shaped and genuinely unserved. It is also speculative
— the areas mode covers most of that need today. Revisit only if that
navigation is actually wanted; do not build the region ahead of it.

### What multiple zones are now *for*

Worth stating plainly, because Decision 1 changes the answer. Zones were
incidentally a comparison tool. Under this design they become deliberately a
**multi-level view**: several levels of one building on screen at once, each
independently coloured, panned and zoomed — with `linkViews` to move them
together when stacking floors, or unlinked to inspect different wings.

That is a clearer feature than the one it replaces, and it is what the
per-zone level picker exists to serve.


## Decision 1 — `index.html` is a single-project viewer

Scope (project / milestone / building) becomes **global state**, module-level
alongside `linkViews`. There is exactly one scope on screen. Zones differ only
in **level and colour** — presentation, never data.

### What this gives up, and why that's correct

Today's zones each carry their own scope, so two zones *can* show different
projects or milestones side by side. **Confirmed: nobody uses this.** It is an
emergent capability — it exists because zones happened to be built
independent, not because it was designed or asked for.

What zones *are* used for is viewing several levels at once, and that is
untouched: `levelSelect` stays per-zone (see "What multiple zones are now
for" above). The capability being dropped is cross-*project* and
cross-*milestone* display, not cross-level.

Multi-project comparison is a good future feature. It should get **its own
page**, not a mode flag on this one. The reasoning:

- **The codebase already answers this question.** `comparison.html` and
  `settings.html` are separate pages. The established pattern for "a distinct
  task with distinct UI" here is a new page.
- **A comparator diverges from a viewer over time.** Real cross-project
  comparison wants synced views, difference highlighting, and a shared
  coordinate frame — and [Server](STRATEGY-SERVER.md) is explicit that
  cross-project geometry comparison needs an alignment transform that does not
  exist yet. That is substantial future work a single-project viewer will
  never share. Binding them into one page means every comparison feature has
  to justify itself against "does this break single-project mode?"
- **A mode flag would make the simple case a mode.** Single-project is
  currently just "what happens." Under a mode switch it becomes one of two
  things a user must know exists — a concept added to the common path to serve
  the rare one.

A future comparator can borrow `paintLevel` and the zone machinery. Nothing
here forecloses it.

### What this deletes

Relative to a design that preserved per-zone scope, this removes:

- any pin / unpin concept and its affordance,
- the **entire focus model** — no `activeZoneId`, no active-zone border, no
  focus-transfer rules on zone add/remove. Both bottom bands have an
  unambiguous subject because there is only one scope on screen.

Both were real user-visible concepts. Not needing them is the main prize here.

### What consolidates

Real simplifications, not incidental:

- **Polling.** Each zone currently carries its own poll cursor and revision
  tracking. With one scope there is one payload — **poll once, fan out to
  zones.** Removes the bug class where two zones on the same project drift a
  tick apart.
- **Colour-plan settings read.** The viewer's one read of
  `/api/settings/projects/{id}` happens once per project change, not per zone.
- **Validation state.** `currentValidation` / `errorRoomIds` are
  project-scoped and currently duplicated per zone. Collapses to one.

Zone state reduces to: `activeLevelId`, `activeColourPlan`, `areasMode`,
`areasTier`, view/pan state, and cull units. All presentation.


## Decision 2 — the bottom region

**Sibling of `main`, not a child of `.zone`** (see the layout diagram above). A
per-zone bottom region would multiply with zone count and stop being a stable
place users can point at.

The region is **two bands, not one collapsible panel**, because its contents
have two different lifecycles:

```
├──────────────────────────────────────────────────────────────┤
│ ▸ QA · 3 mismatches          ← results band (sometimes shown)│
├──────────────────────────────────────────────────────────────┤
│ Rooms  [Model ☑] [dRofus ☑]                    ← source tabs │
│ ┌────────┬──────────┬─────────┬──────────┬─────────┐         │
│ │ Number │ Name     │ Area    │ NetArea  │ Dept    │         │
│ │ 1.01   │ Office   │ 25.5    │ 25.5     │ Admin   │         │
│ │ 1.02   │ Store    │ 12.0    │ 12.4  ⚠  │ Admin   │         │
│ └────────┴──────────┴─────────┴──────────┴─────────┘         │
└──────────────────────────────────────────────────────────────┘
      └── model-derived ──┘ └──── dRofus-derived ────┘
```

The `⚠` marks a QA mismatch shown *in place* on the disagreeing cell pair — see
"The two bands inform each other" below.

### How the two bands share height

The bottom region has **one user-draggable total height**. Expanding band 1
takes space from band 2, never from the plans — the plans must not resize
because a QA run finished.

Neither band pushes the other below usability, because **both scroll
internally**:

- **Band 2** scrolls by necessity — thousands of rows in a fixed-height band.
  It absorbs band 1 expanding by showing fewer rows, which is invisible: it is
  already a window over far more rows than fit.
- **Band 1** gets a **max height, then scrolls**. This is what removes the
  edge case: without it, a long mismatch list in a short bottom region would
  squeeze band 2 to a row or two.

So nothing moves except the divider between the bands, and the region's total
height stays exactly where the user put it.

Whether that height survives a reload is the same question the labels toggle
raises (Decision 4, "Persistence") — decide it once for all view state rather
than per control.

#### The live alternative: side by side

Worth trying during step 2, because it may be better and it is a CSS change on
one container, not a restructure. Band 1 becomes a **left column** that
collapses to nothing when there is no data; band 2 takes the rest and scrolls
horizontally if its columns overflow.

**For it:** results are typically short and few — a handful of mismatches, a
dozen area figures — which is a narrow-and-tall shape that suits a column
better than a wide strip. An absent column costs no space at all, whereas an
empty full-width band always does.

**Against it, and why stacked is the default:** band 2 loses width, and band 2
is the one that needs it. Model *and* dRofus columns shown together is easily
fifteen-plus columns; horizontal scrolling hides columns behind a gesture and
breaks exactly the side-by-side value comparison the source toggle exists to
enable. The moment `Area` and `NetArea` cannot be on screen together, the
grid's main purpose is damaged. Stacked protects the scarcer resource.

There is also a reading-order argument: sometimes-visible results *above*
always-visible data has a natural sequence, whereas left-right implies two
parallel things rather than a result and its subject.

**This is a judgement about proportions that is far easier to settle by looking
at it than by reasoning about it.** Build stacked; try side-by-side with real
data before committing.

### Band 1 — results (sometimes visible)

Output of a computation the user triggered: QA/validation mismatches,
hierarchy area figures. **Transient, belongs to an action, dismissable.**

Collapsed to a one-line summary strip by default (`▸ QA · 3 mismatches`),
expanding on click. A fully hidden band gets forgotten; an always-open one eats
plan area for output that is only sometimes relevant.

This is where `.validation-panel` and `.areas-panel` land. Both are already
tabular and are currently squeezed into a 300px right-hand overlay; moving them
here gives tabular data a shape that suits it and frees the right side for the
inspector (Decision 3). Both are project-scoped, which under Decision 1 means
**one instance each, not one per zone** — they read global state and need no
zone association at all.

**Watch for hidden coupling:** both are positioned relative to `.zone-canvas`.
Migration may surface assumptions about that containing block.

### Band 2 — source data (always visible)

A grid over the rooms in the current scope. **Ambient, belongs to the data, not
dismissable** — it is the tabular view of what the plans show graphically.

Its defining control is a **per-source column toggle**: show model-derived
columns, dRofus-derived columns, or both.

This is not a UI convenience layered on top of the data — it is the existing
data model surfacing honestly. [Sources](STRATEGY-SOURCES.md) records that
dRofus data is deliberately kept as a **separate sub-object, never merged into
`properties`** ("store raw, join late"), precisely because the two have
different lifecycles and provenance. The grid should preserve that separation
visibly: grouped column headers, not an interleaved flat table that implies a
single source of truth.

Notes:

- **Scope, not zone — settled.** The grid lists the rooms of the current
  global scope, every level. It does not follow any zone's level picker, and
  does not change when a zone switches level or when zones are added or
  removed.

  The alternative considered was filtering rows to the levels currently on
  screen. Rejected: with several zones showing different levels it has no
  single answer, and it would make the grid's contents depend on presentation
  state — reintroducing exactly the scope/presentation blurring Decision 1
  removes. Filtering by level stays available as an ordinary column filter,
  driven by the user rather than inherited from the plans.
- **Row count is the design constraint**, and the scope-only rule above makes
  it larger: the grid spans every level in scope, not one. The `big-plate`
  fixture is 5,046 rooms on a *single* level, so a whole building is the real
  target. Rendering every row as DOM will not hold, so the grid needs row
  windowing from the start. The codebase has already solved the analogous
  problem: `paintLevel`'s cull units hide off-screen *rooms* by precomputed
  bbox. Row windowing is the simpler one-dimensional case of the same idea,
  and should follow its shape rather than inventing a second pattern.
- **Read-only to begin with.** Sorting and column filtering are fine; editing
  is a much larger commitment (dirty state, per-cell validation, conflict
  against the 2 s poll) and is out of scope here.
- **Unmapped dRofus columns still appear.** `DrofusData.all_labels` retains
  every dRofus field regardless of whether row 2 of the CSV mapped it to a
  Revit property, specifically so coverage can show an unmapped column as
  "not checked" rather than omitting it silently. The grid should honour that —
  an unmapped column is shown, visibly distinguished, not hidden. **This needs
  a small server change**; see "Where band 2's data comes from" below.
- **CSV export follows the existing precedent.** The validation panel already
  builds its CSV client-side from data the browser holds, deliberately with no
  server endpoint ("a presentation reshuffle of data the browser already has").
  A grid export is the same case and should reuse that approach — exporting the
  visible columns, honouring the source toggle.

### Where band 2's data comes from (verified against the Rust source)

**`GET /rooms` already serves this, with one small addition.** Checked against
`service/rooms.rs`, `drofus.rs` and `handlers.rs`:

- **Scope-shaped, not level-shaped.** `RoomsQuery` is
  `{ project, building, milestone, filter }` — there is **no level
  parameter**. The endpoint already returns every room in scope plus the
  `levels` list, and the browser does the per-level slicing itself
  (`roomsOnLevel`). The scope-only rule above is therefore the *natural* shape
  of the existing read, not a constraint imposed on it.
- **Both sources arrive already separated.** `RoomResponse` is the stored
  `Room` flattened, plus `drofus: Option<DrofusRecord>` as a distinct
  sub-object, plus `classification` and `label`. `DrofusRecord.fields` is a
  `BTreeMap<dRofus label → value>`. The model/dRofus column split the grid
  needs is exactly this boundary — no client-side unpicking of a merged blob.
- **An unmatched room simply has no `drofus` key** (`skip_serializing_if`),
  which the grid should render as empty dRofus cells, not as an error. The
  code comment is explicit: an unmatched key is a signal, not an error.
- **`revision` gives the grid its refresh trigger for free.** It changes only
  when a contributing snapshot actually changes, so the grid can reuse the
  poll comparison the viewer already does rather than diffing rows.

**The one gap: `all_labels` is not on the wire.** `DrofusData` carries
`all_labels` (every row-1 CSV label, mapped or not) and `reconciliation`
(the mapped subset), but neither reaches `RoomsResult` — only per-room
`fields` for rooms that matched. Consequences:

- The grid can only discover dRofus columns by unioning `fields` across
  returned rooms. A dRofus column that exists in the CSV but matched **no**
  room in scope would be invisible.
- That is precisely the case the coverage report goes out of its way to show
  as "not currently checked" rather than omit.

**Fix: add the dRofus label set to `RoomsResult`** — the full `all_labels`
list, ideally alongside `reconciliation` so the grid can mark which columns
have a Revit counterpart. Additive, no schema bump (the viewer ignores unknown
fields), and it lets the grid render a complete, honest column set with
empty-but-present columns visibly distinguished from absent ones.

**One subtlety when implementing it.** `assemble_room` already takes its
`DrofusData` as an explicit parameter rather than reading `bundle.drofus`,
specifically so a milestone view can join a *pinned* dRofus snapshot. The
labels must come from the **same** resolved `DrofusData` the rows were joined
against — the per-project `effective_drofus`, not `bundle.drofus` — or a
milestone view would show current column headers over pinned data. The
existing code already computes the right value; the label set just needs to be
carried out alongside it. Note also that dRofus is resolved *per project*,
while a `RoomsResult` can span several models, so this is one label set per
response only because the response is already project-scoped.

Worth doing as a small server change *before* the grid is built (sequencing
step 4, ahead of step 5) rather than working around it client-side.

### The two bands inform each other

Once model and dRofus columns sit side by side, a QA mismatch becomes a
**cell-level** fact rather than a list entry: `compute_validation` already
produces `property_mismatches` keyed by room and field, which is exactly a
(row, column-pair) address in this grid.

Highlighting the disagreeing pair in place is likely a better presentation than
the band-1 list, with the list reduced to a summary and a jump target. Worth
building band 1 so that a mismatch entry can scroll band 2 to its row rather
than duplicating the detail. Not required for a first cut, but cheap to allow
for and expensive to retrofit.


## Decision 3 — the right inspector (future)

One inspector for the page, not one per zone. **Selection is page state.**

The inspector shows *the selected room*. There is one selection at a time
regardless of which zone the click landed in, so a per-zone inspector would sit
empty in every zone but one.

This quietly reintroduces a small piece of the focus problem Decision 1
deleted: the same room can appear in two zones showing the same level, so the
inspector should name the zone its selection came from. **That is a label, not
`activeZoneId` machinery** — selection is explicit (the user clicked a room)
rather than inferred from where they last interacted, which is what made the
full focus model expensive.

Not buildable yet: the viewer has hover `<title>` tooltips, not click
selection. Recorded here so the region is reserved with a known purpose rather
than left vague.


## Decision 4 — the labels toggle (the original request)

Global presentation state. Lives in the header as a `button.ctl`, reusing the
existing `.ctl` / `.ctl.on` styling — no new CSS class.

**It must be honoured by SVG export**, and that requirement is what determines
the implementation:

- A CSS-only toggle (a class on `svg.plan`, following the precedent of
  `svg.plan.areas-active .label { opacity: 0.22 }`) would be cheaper and needs
  no re-render — **but `buildLevelSvgFile` calls `paintLevel` directly and
  never sees a zone-level class**, so export would silently ignore it.
- Therefore: `paintLevel` gains a `showLabels = true` option alongside
  `colourPlan` / `showErrors`, and its label pass becomes conditional.
  `renderLevel` and `buildLevelSvgFile` each pass it through their existing
  options bag.

Notes:

- `exportStyleBlock()` needs no change. Omitting the elements is cleaner than
  styling them away; a leftover `.label` rule in the exported `<style>` is
  harmless.
- **Re-render every zone with `refit: false`.** With `refit: true` every zone
  snaps back to fitted bounds and users lose their pan/zoom. Guard zones with
  no `currentPayload`.
- **Cull units:** `paintLevel` pushes labels into each room's `nodes` array,
  which is shorter when labels are off. `cullZone` iterates rather than
  indexing, so this is fine — but confirm nothing indexes `nodes`
  positionally.
- Label-off exports get *faster* (skips per-room font-fitting arithmetic).
- **Persistence:** `linkViews` does not persist today. Either persist global
  view prefs consistently or not at all — a single persisted flag is the kind
  of inconsistency that confuses the next reader. `common.js` has
  `persistSelection` if wanted.

This is also why the labels toggle was worth pausing on: in the restructured
UI it is trivially and obviously correct, because the header finally has an
unambiguous meaning.

### A note on level-of-detail

[Browser](STRATEGY-BROWSER.md) records an open performance item: a *fitted*
view of a 5,000-room level still paints everything (~0.5 s+/frame), and the
remaining lever is level-of-detail — **"drop labels / merge rooms when the
whole plate is on screen."** Once `paintLevel` takes a `showLabels` flag, the
label half of that LOD idea has its mechanism already in place; an automatic
mode would just drive the same flag from zoom level rather than from a button.
Not in scope here, but worth not designing against.


## Sequencing

1. **Scope migration** — global scope; zone toolbar reduced to level +
   colour + areas. **The wide one.** Touches polling, validation, colour-plan
   loading, selection persistence. Land it alone.
2. **Bottom region shell** — two bands, collapse, internal scroll, draggable
   total height. Small. **Try stacked vs side-by-side here**, with real data,
   before the grid makes the choice expensive to revisit.
3. **Migrate validation + areas into band 1.** Where hidden coupling to
   `.zone-canvas` positioning would surface.
4. **Expose the dRofus label set on `RoomsResult`** — `all_labels` +
   `reconciliation`. Small, additive, server-side; the only server change in
   this whole restructure. Prerequisite for step 5.
5. **Source-data grid (band 2)** — the largest single piece after step 1, and
   the only one with a performance constraint of its own (row windowing).
   Deliberately late: it benefits from scope already being global, and from
   band 1 having established the region's layout.
6. **Labels toggle.** Independent of 1–5; can be done first if a quick win is
   wanted, at the cost of placing a button in a header whose meaning is about
   to change.

Steps 1 and 5 are the two substantial ones; everything else is small. Do not
combine step 1 with anything — landing scope migration alongside another change
means debugging two unfamiliar things simultaneously. Steps 2 and 3 can
reasonably land together if band 1 turns out to be a straight move.


## Open questions for whoever picks this up

Two items below are **settled but listed anyway**, because they are the
decisions most likely to be quietly reversed mid-implementation. One is
**settled with a live alternative** that should be tried before committing.

- **Does anything else depend on per-zone scope?** *Genuinely open.* The
  capability is unused *by users*, but selection persistence
  (`persistSelection`) and the poll layer may assume it structurally. Step 1 is
  where that surfaces.
- **Stacked bands, both scrolling internally; band 1 has a max height.**
  *Settled* (Decision 2, "How the two bands share height"). Side-by-side is
  recorded there as a live alternative worth trying with real data during
  step 2 — it is a container-level CSS change, not a restructure.
- **Grid follows scope only, not the visible plans.** *Settled* (Decision 2,
  band 2). Flagged because a level filter will feel like it "should" follow the
  on-screen levels during implementation. It should not.
- **`GET /rooms` feeds the grid, unchanged apart from the label-set
  addition.** *Settled and verified against the source* (Decision 2, "Where
  band 2's data comes from"). No new endpoint.
- **The future multi-project comparator** is out of scope here. When it comes,
  the reusable pieces are `paintLevel` (already a pure painter, reads no zone
  state) and the zone/pan/cull machinery. Its blocker is the cross-project
  alignment transform described in [Server](STRATEGY-SERVER.md), not UI.
