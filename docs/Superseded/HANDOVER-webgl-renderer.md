# HANDOVER — Replace the SVG plan renderer with WebGL

> **Superseded — built, and the number it was commissioned to move has moved.**
> A fitted pan of `big-plate` went from **733 ms p95 to 1 ms**, against the
> ≤16 ms budget this document set. The live pointer is
> [STRATEGY-BROWSER.md](../STRATEGY-BROWSER.md) "Renderer"; how it was built is
> [PLAN-webgl-renderer.md](PLAN-webgl-renderer.md), superseded beside this.
>
> **Two of its decisions did not survive contact, and both are worth knowing:**
>
> - **Decision 1's criterion was wrong**, though its conclusion held. "Put the
>   things there are dozens of on the SVG overlay" is a performance test, and it
>   grouped selection with hover because both are one room. They are not alike:
>   a selection stroke has `fill: none` and composites harmlessly, an opaque
>   hover fill hides whatever is under it — which is what it did to the label of
>   the room being pointed at. The rule is **occlusion, not size**.
> - **DoD item 5 could not hold as written.** It wanted the renderer flag kept
>   as a re-measurement handle *and* P6 to delete the live SVG path; with the
>   path gone there is nothing to flip to. Resolved by keeping the capability
>   without the code — `measureSvgPaint()` times the export painter — and by
>   accepting that a browser without WebGL shows no plan.
>
> Its warnings were mostly right, and two were not: labels were indeed not the
> bottleneck, but *constructing* 5,046 of them costs ~1.1 s of a level build;
> and DPR turned out barely to matter, because the frame is four draw calls
> rather than anything fill-bound. What it did not anticipate at all was the
> hardest part — non-scaling strokes, which no scene graph provides.

**Status:** design settled, not built. Reviewed against `static/index.html`
(4,287 lines) on 2026-08-02.
**Audience:** the session that implements it.
**One-line scope:** move the *bulk* of the plan — room fills, holes, outlines and
labels — off SVG DOM onto a WebGL canvas, keeping the SVG export, the areas
overlay and every interaction behaviour that exists today.

**Third-party libraries are wanted here, not tolerated.** Polygon triangulation
with holes, a text atlas, and batched 2D draw calls are all solved problems and
none of them should be written again in this repo.

---

## Why, and the number that decides it

Measured on the `poc-canvas-vs-webgl` branch (`poc/canvas-vs-webgl/RESULTS.md`
there — the folder is deliberately not on `main`, so the numbers are reproduced
here rather than linked). Same dataset, same spatial index, same cull, same
label policy, same pick; only the draw layer differed. DPR 1, 1200×800.

| | Canvas2D | WebGL |
|---|---|---|
| cost per room, fitted view | 4.3 µs | ~40 ns |
| largest plate holding 16 ms p50, fitted | ~1,900 rooms | ~440,000 rooms |
| cost of ~199 two-line labels, per frame | +2.4 ms | +1.0 ms |

Three things follow, and each one changes what gets built:

1. **The pan case is already solved and is not the reason to move.** With
   `cullZone` live, panning is O(rooms on screen), and rooms on screen is
   roughly constant whatever the plate size. SVG pan measured **16.5 ms/frame
   with culling against 912 ms without** (STRATEGY-BROWSER.md). Canvas2D held
   ~9 ms flat from 1,000 rooms to 1,600,000. Nobody needs a new renderer to pan.
2. **The fitted view is the whole reason.** It is where the cull culls nothing,
   it is what the viewer shows on load because it auto-fits, and it is the item
   STRATEGY-BROWSER.md still lists as open ("a *fitted* view of a 5,000-room
   level still paints everything, ~0.5 s+/frame"). Canvas2D would be ~33 ms at
   the 5,046-rooms/level `big-plate` fixture — better than SVG and **still over
   budget today**. WebGL is ~0.6 ms there.
3. **Labels are not the risk they were assumed to be.** A glyph atlas measured
   *cheaper* than `fillText`, and this repo's own earlier SVG finding said the
   same thing from the other direction
   (`Superseded/HANDOVER-viewer-performance.md`: "Labels are NOT the
   bottleneck"). Do not spend the budget defending against label cost.

**Budget for this work:** ≤16 ms p95 per-frame work on a fitted view of
`big-plate` (5,046 rooms/level), labels and pick included, at the target DPR.
That is the number the POC was run against and the one to re-measure with.

---

## Decision 1 — a hybrid, not a rewrite. Only the bulk layer moves.

**Keep an SVG layer on top of the WebGL canvas, and put the small things on it.**

| Layer | What it draws | Count |
|---|---|---|
| WebGL canvas (new) | room fills, holes, outlines, room labels | thousands |
| SVG overlay (existing code, mostly unchanged) | areas footprints, the selected-room outline, hover outline | dozens |

This is the single highest-leverage decision in the document, because the
features that are *painful* in WebGL are all in the second row and all tiny:

- **The areas overlay** (`renderAreasOverlay`) draws even-odd `<path>` footprints
  with interior rings, per-group colours through a `--area-colour` custom
  property, and `.area-poly.selected` styling. There are dozens of these, never
  thousands. Moving them buys nothing and costs a triangulator, a custom-property
  emulation and a rewrite of `areaAtNode`. **Leave the whole function alone.**
- **Selection** is exactly one room (`.room.selected` — dashed accent, stroke 5).
  Drawing one outline into the overlay keeps the CSS rule, keeps
  `applySelection`'s shape, and avoids re-uploading a GPU buffer on every click.
- **Hover** (`.room.error:hover`) is one room, same argument.

What *must* move is only what there are thousands of. Resist the urge to make
this total.

## Decision 2 — PixiJS v8 for the draw layer

Rejected, with reasons, because the next reader will ask:

- **regl / raw WebGL** — what the POC used, deliberately, to measure a floor with
  no library overhead. Wrong for the product: it means hand-rolling
  triangulation, a glyph atlas and batching, which is precisely what this
  handover is told not to do.
- **Konva / Fabric** — Canvas2D, not WebGL. The measurement above rules Canvas2D
  out at the fitted view.
- **deck.gl** — genuinely strong here and the runner-up: `PolygonLayer` takes
  rings-with-holes directly, `TextLayer` ships SDF text, picking is GPU-side, and
  `updateTriggers` change one attribute without rebuilding geometry. Rejected as
  the default on bundle size (~1 MB plus luma.gl) and on conceptual distance —
  its viewState/layer model would have to be mapped onto `zone.view`, and the
  geospatial idioms cost a reader more than they save. **Reach for it if
  per-frame attribute updates or text crispness become the binding constraint;
  those are the two places it is clearly better.**
- **PixiJS v8 — chosen.** A 2D scene graph over WebGL (with a WebGPU path),
  batched geometry, polygon fill with holes, and bitmap text. Closest to the
  mental model already in `index.html`, and the smallest conceptual jump for
  whoever maintains this next.

Verify the exact v8 API against current docs at implementation time; this
document commits to the library, not to method names.

**Do not use Pixi's culler or its scene-graph hit-testing.** Both are CPU walks
over the display list. The POC measured a **Flatbush** R-tree over room bounding
boxes doing the same two jobs in ~0.1 ms at 1.6 M rooms, and the index is
reusable for snapping later. Build the index once per level, feed it both the
viewport cull and the pick.

## Decision 3 — the export stays SVG, and that is a seam, not a footnote

`paintLevel` is **shared verbatim** between the live renderer and
`buildLevelSvgFile`, and its doc comment says why: so the two "can't drift". A
WebGL renderer cannot serialize to `.svg`, so that sharing has to be replaced by
something, and the something must not be "two painters that happen to agree".

**Extract appearance resolution, keep two emitters.** One pure function per room
returning the resolved appearance — fill (colour plan, else error, else default),
outline, dim, match — with no DOM and no GL in it. `paintLevel` keeps its two
passes and stays the **export-only** painter; the WebGL renderer consumes the
same resolved appearance and uploads it. The thing that must not drift is the
*decision*, and after this only one copy of it exists.

Everything `buildLevelSvgFile` does today survives untouched: fitted framing via
`fittedBounds`, the `exportStyleBlock` resolved-CSS-variable block, the opaque
paper background, no selection stroke in the file, `showLabels` following the
header toggle.

---

## What must still be true when this lands

A checklist, because most of it is invisible until it is missing. Every item
below is a shipped behaviour with a reason recorded in the code or in
STRATEGY-BROWSER.md.

- **Rooms have holes.** `room.loops[0]` is the outer ring; `loops[1..]` are
  voids, drawn today as `.hole` polygons filled with `--paper` and dashed. In GL
  either triangulate with holes properly or keep the same paint-over trick — but
  the dashed hole stroke is a visible feature, not incidental.
- **The SVG export is byte-comparable in content** to what it produces today for
  the same level, plan and toggle state.
- **Two-line labels, server-driven.** `room.label` is an ordered field list; the
  first field is the primary line, the rest stack smaller in the accent colour.
  `label` present-but-empty means "the configured properties did not resolve" and
  must stay blank; `label` missing falls back to name/id. Font size is fitted to
  the room's own bbox (`addLabel`) — a bitmap font scales differently, so
  re-derive the sizing rather than porting the arithmetic blindly.
- **Labels toggle** (`showRoomLabels`) drives both the screen and the export.
- **Colour plans** — all three modes (property-compare, hierarchy, date-range)
  are client-side and per zone (`zone.activeColourPlan`). They resolve to a
  literal fill per room; that is exactly what the GL renderer wants as a
  per-vertex colour.
- **Search** paints `.match` (accent outline) on matches and `.dim` (opacity
  0.15) on everything else. Both are per-room and belong in the bulk layer as
  attributes, not as one-off overlay nodes — a search can match thousands.
  `applyHighlight` is a fast path today that toggles classes *without*
  re-rendering; preserve that property, or a keystroke re-uploads the level.
- **Validation errors** — `.room.error` fill, driven by the global `showErrors`.
- **Selection survives a re-render.** `renderLevel` re-applies it after painting
  because paint rebuilds nodes. Same obligation with buffers.
- **Areas mode ghosts the rooms beneath it** (`svg.plan.areas-active` sets rooms
  to low opacity, holes 0.16, labels 0.22, but a selected room back to 1). With
  the overlay staying SVG and the rooms moving to GL, this cross-layer rule is
  now a uniform on the GL side. Easy to miss; visibly wrong when missed.
- **Per-room tooltips.** Every room polygon carries an SVG `<title>` giving the
  browser a native tooltip. WebGL has no equivalent — this becomes a DOM tooltip
  driven by the hover pick. Small, but it is a feature that silently disappears.
- **Click-to-select and click-empty-to-clear**, including the `CLICK_SLOP_PX`
  drag/click distinction and `setPointerCapture`. Today `roomAtNode` scans cull
  units for the hit node; with GL there is no node, so this becomes an index
  query plus point-in-polygon. Note the ordering rule the pointerup handler
  encodes: rooms first, then footprints, because "clicking what you can see is
  the rule".
- **Pan, wheel-zoom about the cursor, and double-click-to-refit**, all expressed
  against `zone.view`.
- **Multiple zones.** `createZone()` clones a template and `+ zone` is unbounded.
  See Traps.

---

## Suggested order

Each step is separately verifiable, and the first two change no behaviour at all.

**P1 — Extract appearance resolution.** Pure refactor. `paintLevel` and the
export keep working and look identical. Verifiable by exporting a level before
and after and diffing.

**P2 — Put a renderer seam behind the zone.** `zone.renderer` with
`paint(rooms, fitted, opts)` / `setView(view)` / `applyState(...)` / `dispose()`,
implemented *first by the existing SVG code*. Still no behaviour change; this is
the step that makes the swap reviewable, and skipping it means P3 is a 1,000-line
diff nobody can read.

**P3 — Add the Pixi renderer behind a flag**, module-level like `CULL_ENABLED`
(`Superseded/HANDOVER-culling-disable-switch.md` is the precedent, and its
re-measurement method is the one to use). Both implementations live at once, so
they can be compared on the same data in the same session.

**P4 — Move areas, selection and hover onto the thin SVG overlay**, and add the
DOM tooltip. This is where the hybrid actually forms.

**P5 — Flip the default.** Keep the flag as the escape hatch and as the
re-measurement handle.

**P6 — Delete the SVG live path.** `paintLevel` stays, now export-only, and its
doc comment must be rewritten to say so — the current one claims a sharing that
will no longer exist, and a stale rationale comment is worse than none.

---

## Traps

- **WebGL contexts are a limited resource and `+ zone` is unbounded.** Browsers
  cap live contexts (commonly ~16) and silently kill the oldest past the limit —
  which would blank a zone with no error anyone can act on. Cap zones
  explicitly, dispose the context in `removeZone`, and if the cap ever bites, the
  escape hatch is one canvas across the zone strip with a `gl.viewport`/scissor
  per zone rather than a context each.
- **Keep the existing Y-flipped coordinate space.** World data is Y-up; the
  viewer flips Y when building geometry, and `zone.view`, `fittedBounds`,
  `roomBBox`, `cullZone` and the areas overlay all live in that flipped space.
  GL clip space is Y-up, so it is tempting to drop the flip — don't. Flipping in
  the projection matrix keeps every one of those untouched and confines the
  change to the draw layer.
- **Instrument devicePixelRatio.** The POC measured at **DPR 1**; a retina
  display roughly 4×'s fill cost. Size the backing store by DPR, the way
  `static/graph.js` already does (it is the shipped precedent for a canvas
  renderer in this codebase — read it before starting, including its note on
  losing accessibility and text selection).
- **`getImageData`/`gl.finish` belong in benchmarks, not the render loop.** The
  POC used a forced sync to make its timings honest. Do not carry it into the
  client; it stalls the pipeline every frame.
- **Text selection and accessibility go away** for room labels. SVG `<text>` is
  selectable and exposed; GL text is pixels. `graph.js` already accepted this
  trade for the adjacency graph and says so. It is a real regression for the plan
  and should be an explicit, recorded acceptance rather than a discovery.
- **The 2 s poll re-renders on revision change.** A GPU buffer rebuild per real
  push is fine; a rebuild per *tick* is not. The revision compare already
  prevents that — don't regress it by rebuilding on every poll response.
- **Do not add a server endpoint.** "Keep axum as a pure JSON API" is the
  load-bearing decision in STRATEGY-BROWSER.md. This is a browser change.

---

## Deliberately out of scope

- **The `/rooms` serving shape.** A bboxes-plus-labels overview response, split
  from the full-polygon payload, is the natural companion (cheap to *fetch* as
  well as cheap to draw) and is recorded as a follow-on in STRATEGY-BROWSER.md.
  It is not needed to make this land — the client already receives everything it
  needs.
- **Level-of-detail beyond the existing label cull.** The fitted view stops being
  the bottleneck the moment this lands; revisit only if measurement says so.
- **`static/graph.js`.** Already canvas, already fine, unrelated.
- **`static/comparison.html`.** Draws no plan geometry at all (no
  `createElementNS` anywhere in it); it is a table page and is not affected.

---

## Definition of done

1. The plan renders on WebGL with holes, outlines, two-line labels, colour plans,
   error highlighting, search match/dim, and the areas ghosting rule.
2. Pan, wheel-zoom about the cursor, double-click refit, click-to-select,
   click-empty-to-clear and hover tooltips all behave as they do today.
3. The SVG export produces the same document it produces today.
4. A fitted view of `big-plate` (5,046 rooms/level) holds the **≤16 ms p95**
   budget at the target DPR, measured with the `CULL_ENABLED` method, and the
   figure is recorded in STRATEGY-BROWSER.md beside the existing SVG numbers.
5. The renderer flag survives as the re-measurement handle; `paintLevel`'s doc
   comment is rewritten to say it is export-only.
6. Verified by **driving the page**, not by reading the diff — the house rule,
   and the one that caught a bug in this project's last frontend change.
