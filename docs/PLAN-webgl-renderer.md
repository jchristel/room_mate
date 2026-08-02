# PLAN — Implementing the WebGL plan renderer

**Status: DONE. P0–P6 built; the WebGL renderer is the only one.** The measured
outcome lives in
[STRATEGY-BROWSER.md](STRATEGY-BROWSER.md) "Renderer" — a fitted pan of
`big-plate` went from 733 ms p95 to 1 ms, against a ≤16 ms budget — along with
the accessibility acceptance and the re-measurement handle. Written 2026-08-02
against
[`HANDOVER-webgl-renderer.md`](HANDOVER-webgl-renderer.md), which commissions
this work and remains authoritative on *what must not break*.

**Relationship to the handover:** that document settles the destination (a
hybrid WebGL/SVG viewer on PixiJS v8, with a Flatbush index and an SVG export
that survives). This one settles the questions it deliberately left to
implementation time — how third-party code reaches the browser, what the seam
looks like, what is tested and how — and it records **two places where it
departs from the handover**, both flagged below rather than left for a reader to
discover as a discrepancy.

---

## The premise that changed, and what it moves

The handover was written while the frontend was deliberately buildless: no
`package.json`, no bundler, `static/` served raw by axum's `ServeDir`, and
`index.html` / `common.js` / `graph.js` loaded as classic `<script>` tags sharing
plain globals. That was a **proof-of-concept-stage decision** and it has done its
job — it got a working viewer up without a toolchain to argue about.

It is now retired. The project is past the stage where avoiding a build step is
worth what it costs, and **long-term structure is preferred over short-term
containment**. Four decisions in this plan are downstream of that and would have
gone the other way a month ago:

| Decision | Under the old premise | Here |
|---|---|---|
| Dependencies | vendor a minified blob, no toolchain | npm + Vite, `src-js/` as source |
| Language | JS, because there is no compile step | **TypeScript**, for new code *and* the plan-rendering subsystem |
| Export parity | diff an export by hand, once | **golden-file test**, permanently |
| Two live renderers | keep SVG as a cheap fallback | **delete it** (see Departure 2) |

What does *not* move: the handover's Decision 1 (hybrid — only the bulk layer
goes to GL) and its "resist the urge to make this total". Those get *stronger*
under a long-term lens, not weaker. The long-term move is to make `src-js/` **the
place new frontend code lands** and migrate the modules this work already forces
us to touch. The rest follows later, incrementally, on its own merits.

## The migration boundary, and why it is where it is

The subsystem that moves to typed modules in `src-js/` is the **plan-rendering**
one, roughly 500–700 lines: `setViewBox`, `roomBBox`, `cullZone`,
`scheduleCull`, `fittedBounds`, `paintLevel`, `addLabel`, `renderLevel`,
`commitView`, `applyHighlight`, `applySelection`, `roomAtNode`,
`wireInteractions`, and the geometry/colour helpers they call (`flip`,
`loopBox`, `centroid`, `bounds`, `pointsAttr`, `colourForRoom`,
`buildColourContext`).

**That boundary and P2's seam are the same boundary**, which is the whole reason
to draw it there. Those functions read module-level mutable globals today —
`errorRoomIds`, `showErrors`, `searchActive`, `searchMatchIds`, `showRoomLabels`,
`CULL_ENABLED` — and threading that state in explicitly is precisely what
`paint(rooms, fitted, opts)` and `applyHighlight(state)` already do. So the
conversion is not extra work bolted onto the seam; it *is* the seam, done once in
one place, rather than half in inline JS and half in TypeScript across an
unchecked boundary. Leaving `paintLevel` behind would put
`resolveRoomAppearance`'s single most important consumer outside the type system,
which defeats most of the point of typing it at all.

**What deliberately stays inline JS:** search, the inspector, the validation
panel, the areas band, scope pickers, colour-plan configuration, polling and the
export plumbing — about 3,600 lines. Converting them buys this work nothing, and
the conversion would not be mechanical: `currentPayload`, `zones`, `selection`
and `areasData` are shared mutable globals, and that *is* the page's
architecture. Reshaping it is a refactor, not a translation, on code with **no
test coverage today** — the combination that produces silent behaviour changes
with nothing to catch them. It becomes safe once there is an end-to-end net under
it (see "Out of scope"), and the standing convention from here on is that each
future frontend change moves the module it touches.

**`common.js` stays put too**, despite looking like the cheap win. It is 8 KB of
pure helpers, but it is a classic `<script>` shared by `index.html`, `graph.js`,
`settings.html` and `comparison.html` — converting it drags all four pages into
needing the bundle, for no benefit here.

---

## Departures from the handover

**Departure 1 — the grid moves to WebGL too.** The handover's layer table lists
the GL layer as "room fills, holes, outlines, room labels" and never mentions the
grid, but `paintLevel` draws one: a `<line>` every 5 world units across the
fitted bounds. On a `big-plate` level that is thousands of DOM nodes, which would
leave a large share of the per-frame cost on the SVG side and blunt the measured
win. It is high-count, so by the handover's own rule ("what must move is only
what there are thousands of") it belongs in the bulk layer. The export keeps
drawing it as SVG lines, unchanged.

**Departure 2 — P6 lands, and DoD item 5 is reinterpreted.** The handover's
Definition of Done says "the renderer flag survives as the re-measurement
handle", while its P6 deletes the live SVG path. Those cannot both hold: with the
SVG path gone there is nothing for the flag to flip *to*. Resolved in favour of
deleting, because two live renderers is a permanent tax — every future frontend
change is made twice or silently rots one path, and the rotten one is always the
fallback nobody exercises.

The *capability* the DoD actually wants is preserved without the live path:
`paintLevel` survives as the export painter regardless, so re-measurement is a
dev-only harness that paints a level into a **detached** SVG with `paintLevel`
and times it — the same comparison, without a second shipped renderer. A machine
with no WebGL loses the viewer; that is an accepted cost, recorded here so it is
a decision and not a surprise.

`CULL_ENABLED` is untouched by any of this. It is a flag about the *cull*, not
about the renderer, and the GL path uses the same index it guards.

---

## Settled decisions

Each with the reason, because the next reader will ask.

- **Vite + vitest, not esbuild.** esbuild is the right call for bolting one
  bundle onto a page; it is the wrong one once `src-js/` is where frontend code
  lives. Vite gives a dev server that proxies `/rooms` to axum (edit-refresh
  without restarting `cargo run`), a library-mode build straight into `static/`,
  and vitest with no separate configuration. esbuild is still underneath it.
- **The built bundle is committed** to `static/vendor/renderer.bundle.js`, so a
  fresh clone plus `cargo run` serves a working viewer with no node installed —
  the property that made the buildless frontend worth having, kept.
- **…and gated in CI.** A committed generated artifact with no freshness check
  drifts from its source, and the failure is silent: a stale renderer with no
  error anyone can act on. A node job rebuilds and fails on any diff. This is the
  first non-cargo job in `rust.yml`.
- **TypeScript in `src-js/`, and the plan-rendering subsystem moves there with
  it.** The case is the *seam* and the *domain shapes*, in that order. The seam
  has eight methods and (during P2–P5) two implementations that must stay
  interchangeable — an interface makes "did the Pixi side cover all eight"
  a compile error rather than an `undefined is not a function` found while
  driving the page, and after P6 it is what documents the surviving contract.
  The domain shapes carry distinctions this code already depends on and only a
  comment currently enforces: `label?: string[]` is *absent* (fall back to
  name/id) versus *present-but-empty* (properties didn't resolve, stay blank),
  and `fill: string | null` is "use the CSS default", not `""`.

  Interleaved buffers are the *weakest* argument, not the strongest, and it is
  worth being exact about why: TypeScript does not catch a bad offset —
  `buf[i * 9 + 2] = r` is `number` either way, and a stride off by one gives
  garbage geometry with no error. What it buys is that the vertex layout can be
  declared once as a typed constant and both the Pixi geometry descriptor and
  the write accessors derived from it, so changing the layout breaks every write
  site at compile time. That is a pattern TS makes safe to rely on, not something
  the checker does unaided.

  `checkJs` stays off.
- **Tests are co-located** — `renderer/appearance.ts` beside
  `renderer/appearance.test.ts`. CLAUDE.md's "tests are inline, never a `tests/`
  tree" is a Rust rule, but its *spirit* is tests beside what they exercise, and
  co-location honours it. Recorded in CODING-CONVENTIONS.md rather than left to
  inference.
- **Zone cap of 8**, with `dispose()` in `removeZone`. Browsers cap live WebGL
  contexts (commonly ~16) and silently kill the oldest past the limit; 8 leaves
  headroom for `graph.js`'s canvas and for other tabs. The add-zone button
  disables at the cap.
- **Geometry buffers are keyed by `(level_id, revision)` and shared across
  zones.** Two zones on the same level currently pay for two uploads of identical
  geometry. This is much cheaper to design in than to retrofit once the seam has
  hardened, which is the only reason it is here rather than deferred.
- **DPR: measured at the development machine's real `devicePixelRatio`**, with a
  DPR-1 figure reported alongside so the number stays comparable to the POC's
  table. The backing store is sized by DPR the way `static/graph.js` already does.

---

## The part that is actually hard

Stated up front because it is the item most likely to consume the schedule, and
it is not the one the handover warns about.

Every stroke in the plan today uses `vector-effect: non-scaling-stroke`: room
outlines (1.5px), dashed hole strokes (1px, `4 3`), grid lines (0.5px). All are
constant **screen** width at any zoom. Under a Pixi container scaled from
`zone.view`, they would scale with the view instead — visibly wrong, and
progressively so.

The obvious fix — rebuild stroke geometry whenever the view changes — is a
per-frame rebuild during pan, i.e. precisely the cost this whole exercise exists
to remove. So strokes get a **screen-space line shader**: world position plus an
edge normal per vertex, expanded by a pixel-width uniform, with dashes driven by
a distance-along-edge attribute and `fract()`. This is built **first** in P3,
before any fill work, because if it does not work the shape of the rest changes.

This is not a violation of "don't hand-roll what libraries solve". Triangulation
is earcut's, batching and GL plumbing are Pixi's, the index is Flatbush's.
Non-scaling strokes are a property of *this* plan's appearance and no library
ships them.

## Why a Mesh and not Pixi's Graphics

One constraint decides it. `applyHighlight` is a documented fast path: a search
keystroke toggles `.match`/`.dim` on already-rendered nodes and never
re-renders, and a search can match thousands of rooms. `Graphics`'s only update
path is a rebuild, so building rooms as `Graphics` would make every keystroke
re-upload the level — the exact regression the handover names.

So each level is triangulated once (earcut, holes included) into one interleaved
buffer behind a Pixi `Mesh` + `Geometry` + a small shader, with per-vertex
attributes for colour, alpha and state flags. Search, error and colour-plan
changes become **attribute sub-updates**. Pan and zoom are a container transform
and touch no geometry at all.

---

## Phases

Each is separately verifiable, and the first three change no behaviour.

### P0 — Toolchain
`package.json`, `package-lock.json`, `node_modules/` in `.gitignore`, Vite
config, `src-js/` with a TypeScript project, and a library-mode build emitting
`static/vendor/renderer.bundle.js` publishing `window.PlanRenderer`.
Dependencies: `pixi.js` v8, `flatbush`, `earcut`.

`.gitattributes` already declares `*.js text eol=lf`, so the committed bundle is
covered — but check `git diff --stat` after the first build regardless (the CRLF
trap in CLAUDE.md).

**Verify:** `cargo run` serves the viewer identically. `npm run build` is
reproducible. CI's new node job passes.

### P1 — Move the plan-rendering subsystem to `src-js/`, and extract appearance resolution
Two things, in one phase, because the second is only meaningful once the first
has happened: the subsystem named above moves into typed modules, and the
appearance decision is pulled out of `paintLevel` as a pure function.

One pure function, no DOM and no GL:

```
resolveRoomAppearance(room, ctx) -> { fill: string|null, error, match, dim }
```

`ctx` carries `colourPlan`, `colourCtx`, `errorRoomIds`, `showErrors`,
`matchRoomIds`, `searchActive`. `paintLevel` consumes it and builds its class
string in the **same order** (`"room"` → `" error"` → `" match"` → `" dim"`), so
serialization is unchanged byte-for-byte.

`index.html` keeps its inline script and calls into the bundle's single global.
That call site is **unchecked in one direction** — TypeScript cannot see it — so
the surface is kept deliberately small (the seam, nothing else) and the bundle
ships a `.d.ts` so the boundary is at least described.

**Verify:** unit tests over the fill-precedence rules, plus the golden-file
export test — captured from `main` *before* this phase lands, so it is a genuine
baseline rather than a snapshot of the refactor's own output. This is the phase
the golden file exists for: a 600-line move of untested rendering code is exactly
where a silent behaviour change would otherwise hide.

### P2 — Renderer seam behind the zone
A TypeScript interface, implemented **first by the SVG code P1 has just moved**
— which is why P1 moves it: the seam is the typed contract over that code, not a
wrapper bolted onto inline JS from outside.

```
paint(rooms, fitted, opts)   setView(view)      applyHighlight(state)
setSelection(roomId)         setHover(roomId)   setAreasActive(on)
roomAt(clientX, clientY)     dispose()
```

`setSelection`/`setHover` are inside the seam deliberately: `applySelection`
toggles a class on the cull unit's room polygon today, and after P4 the GL
renderer draws the same thing into the SVG overlay instead. Same call site, two
implementations. `roomAt` replaces `roomAtNode`'s node scan with a coordinate
query. `renderAreasOverlay` and `areaAtNode` stay **outside** the seam and stay
inline JS, untouched, per the handover's Decision 1.

**Verify:** no behaviour change; the full drive-the-page checklist below passes
identically.

### P3 — The Pixi renderer, behind a flag
Module-level flag, the way `CULL_ENABLED` is, so both renderers can be compared
on the same data in one session. The canvas goes into `.zone-canvas` **beneath**
the existing `svg.plan`, which becomes transparent and keeps pointer handling.

Order within the phase: screen-space line shader → grid → outlines and dashed
holes → fills → labels → Flatbush index feeding cull and pick.

- **Y-flip stays in the geometry**, exactly as `flip()` does today. Pixi's stage
  is already Y-down, so `zone.view`, `fittedBounds`, `roomBBox`, `cullZone` and
  the areas overlay all keep working untouched — the trap the handover names.
- **Flatbush** over `roomBBox` values, built once per level, feeding both the
  viewport cull and the pick. Not Pixi's culler, not its hit-testing.
- **Labels** are Pixi `BitmapText`, with sizing **re-derived** from `addLabel`'s
  constraints rather than ported — a bitmap font scales differently. The
  two-line rule, the present-but-empty case and the name/id fallback all hold.
- No `gl.finish` / `getImageData` in the render loop.

**Verify:** A/B against the SVG renderer in the same session; unit tests on
triangulation, label sizing and the pick.

### P4 — Form the hybrid
- `renderAreasOverlay` **untouched**; it simply no longer has GL rooms beneath it
  in the DOM.
- Selection and hover become overlay polygons drawn by the GL renderer, keeping
  the existing `.room.selected` and `.room:hover` CSS rules verbatim.
- **Areas ghosting** becomes a GL-side uniform (rooms/holes α 0.16, labels α
  0.22). The selected room is in the overlay, so its α-1 exception is free.
  Per-room `.dim` composes as vertex-alpha × container-alpha.
- **DOM tooltip** replacing the per-room `<title>`, driven by an rAF-throttled
  hover pick.
- **Pick ordering**, carefully: the pointerup handler gets rooms-first for free
  today because with areas on, the pressed node *is* the `.area-poly`. With rooms
  off the DOM that accident is gone, so the *behaviour* is preserved explicitly —
  if the pressed node is an `.area-poly` the footprint wins, otherwise the GL
  pick resolves a room. `CLICK_SLOP_PX` and `setPointerCapture` are unchanged.
- Zone cap enforced; context disposed in `removeZone`.
- The revision compare stays the gate: a GPU rebuild per real push is fine, per
  poll tick is not.

### P5 — Flip the default and record the number
Push `big-plate` via `scripts/gen_big_plate.py`; measure p95 frame time on a
fitted 5,046-room level, labels and pick included, using
`Superseded/HANDOVER-culling-disable-switch.md`'s method. Record in
STRATEGY-BROWSER.md beside the existing SVG figures, at both the real DPR and
DPR 1.

Record there too, as an **explicit acceptance** rather than a discovery: room
labels lose text selection and accessibility exposure. `graph.js` already made
and stated this trade for the adjacency graph; the plan is a bigger surface and
deserves the same sentence.

### P6 — Delete the live SVG path
Per Departure 2. `paintLevel` stays as the export painter, and its doc comment is
rewritten — it currently claims a sharing with the live renderer that will no
longer exist, and a stale rationale comment is worse than none. The
re-measurement harness described in Departure 2 lands here, not as a shipped
renderer.

---

## Verified by driving the page

The house rule, and the one that caught a bug in this project's last frontend
change. The full pass, run at P2, P3 and P5:

holes and their dashed stroke · outlines at three zoom levels · two-line labels ·
the labels toggle · all three colour-plan modes · error highlighting · search
match/dim on a multi-thousand-match query · areas ghosting · selection surviving
a poll re-render · click-empty-to-clear · wheel-zoom about the cursor ·
double-click refit · tooltips · eight zones open · an SVG export diffed against
the pre-P1 baseline.

## Out of scope

As the handover directs: `/rooms` serving shape (bboxes-plus-labels overview),
level-of-detail beyond the existing label cull, `static/graph.js`,
`static/comparison.html`. No server endpoint is added — "keep axum as a pure JSON
API" is load-bearing in STRATEGY-BROWSER.md.

And one deferral this plan adds, recorded because something later depends on it:
**end-to-end browser tests (Playwright) are not built here.** The unit and
golden-file layer covers what is falsifiable without a GPU, and WebGL in headless
CI is its own fight, not one to pick in the same change that introduces WebGL.
But that net is the precondition for converting the remaining ~3,600 lines of
inline JS — see "The migration boundary" — so it is the natural next piece of
frontend infrastructure, not an optional extra.

## Docs to update on landing

- **STRATEGY-BROWSER.md** — the measured figures, the fitted-view open item
  closed, the accessibility acceptance.
- **CODING-CONVENTIONS.md** — the frontend toolchain, the co-located `*.test.ts`
  convention, and the standing rule that each future frontend change moves the
  module it touches into `src-js/`.
- **`static/common.js`'s header comment** — it currently states "vanilla JS, no
  build step" as a live constraint. That is now history, and a comment asserting
  a retired decision misleads the next reader more than no comment would.
- **CLAUDE.md** — the verify-before-done block gains the frontend commands.
- **docs/README.md** — index this plan.
