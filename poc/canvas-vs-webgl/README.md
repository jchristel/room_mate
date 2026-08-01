# Renderer POC — Canvas2D vs WebGL

Throwaway measurement commissioned by `docs/HANDOVER-renderer-poc.md`. It exists
to produce a number and a verdict about which renderer the plan viewer's next
generation is built on, and then to be deleted.

**Nothing here is wired into the project.** No Rust, no `static/`, no server, no
build step, no dependency on the contract. It generates its own geometry and
opens straight from the filesystem. The findings live in
[`RESULTS.md`](RESULTS.md); the only thing that ever leaves this folder is the
threshold, recorded in `docs/STRATEGY-BROWSER.md` after review.

## Running it

Open either page directly — `file://` is fine, no server needed:

- [`index-2d.html`](index-2d.html) — Canvas2D
- [`index-gl.html`](index-gl.html) — WebGL via regl

Press **Run sweep**. Each page takes several minutes and the tab will be
unresponsive in places; that is the measurement, not a bug. Results render into
the page as they arrive and the full structure is left on `window.POC_RESULTS`
as JSON.

`?n=` is not wired up — the sweep is the only mode, because a single rung is the
thing this POC was explicitly widened away from.

## The budget — stated before anything was measured

> **p95 of per-frame work during the `pan` phase ≤ 16 ms**, at 50k rooms, with
> labels and pick included, at this machine's DPR.

"Per-frame work" is cull + draw + pick, with the renderer's pipeline forced to
drain inside the timed region (see *Honesty* below). 16 ms is one frame at 60 Hz.

Two crossovers get read off the two tables afterwards:

- **Budget crossover** — the smallest rung where Canvas2D's pan p95 breaks 16 ms
  while WebGL's still holds it.
- **Clear-winner rung** — the smallest rung where Canvas2D's pan p95 is at least
  2× WebGL's.

Each page also stops its own ladder once *its* pan p95 passes 64 ms (4× budget).
A renderer that far out has lost decisively and doubling again only measures how
much worse it gets.

**The verdict rule, from the brief:** WebGL wins only if the sustained-pan number
breaks budget *after* Canvas2D has been given viewport culling and label
culling. If Canvas2D holds, WebGL's text cost is not worth paying.

## What is measured

The ladder is **12 500 → 25 000 → 50 000 → 100 000 → 200 000 → 400 000 →
800 000 → 1 600 000** rooms, run twice: labels on, then labels off. Four phases
per rung, with the camera a pure function of the frame index so both pages
traverse identical viewports:

| Phase | What it is | Why it is here |
|---|---|---|
| `warmup` | 30 frames, discarded | JIT and first-draw costs are not the subject |
| `fitted` | whole plate on screen, drifting | the cull culls nothing — the case `STRATEGY-BROWSER.md` still lists as open for SVG, and where mass unculled geometry actually lands |
| `pan` | constant-velocity sweep at 4 px/ft | **the budget phase**; sustained transform is what killed SVG, not initial draw |
| `zoom` | exponential sweep between fitted and working | the transition, where the visible set churns hardest |

Phases end at whichever of frame-count or wall-clock comes first, with a floor,
so a rung at three seconds a frame costs a minute instead of hanging the tab.
The frames actually sampled are printed per row — a thin sample is visible
rather than hidden.

### Held constant across both pages

Everything except the draw layer, which is the point:

- **The dataset.** Deterministic from one seed, so both pages get byte-identical
  geometry at every rung. Vertex count varies 4–12, footprint varies 1× to 4×
  cells; units and structure mirror `scripts/gen_big_plate.py` (16×14 ft cells,
  feet, Y-up) so the world scale is comparable to the real fixture the existing
  SVG numbers came from.
- **The spatial index.** Flatbush over room bounding boxes, built once per rung.
- **The viewport cull.** Same index query, same rect, both pages.
- **The label policy.** A label is drawn only when its room's on-screen bbox is
  ≥ 42 px wide.
- **Pick.** Index query plus point-in-polygon at a moving cursor, every frame, in
  both. Giving one page free hit-testing and making the other build it is the
  brief's named way to rig this.
- **The plate grows with the room count** at fixed cell size, so doubling rooms
  doubles plate *area* rather than shrinking rooms. Without that, a higher rung
  would quietly also be a zoomed-out test and the ladder would confound two
  variables.

## Honesty

Things that would make the numbers a fiction if they were not done:

- **Forced sync inside the timed region.** Both APIs defer rasterization, and
  timing the calls that *queue* work understates WebGL far more than Canvas2D.
  So WebGL calls `gl.finish()` and Canvas2D does a 1×1 `getImageData`, both
  inside the measurement. This is conservative against both.
- **Work time, not FPS.** rAF delta cannot go below the display interval, so it
  is blind to headroom above 60 fps. Work time discriminates when a renderer is
  fast; rAF delta tells the truth when it is slow. Both are reported.
- **Canvas2D is not sabotaged.** Text is drawn in two passes (all primary lines,
  then all accent lines) so `ctx.font` is assigned twice per frame instead of
  twice per label — interleaving it is a pathological pattern that would sink
  Canvas2D on an avoidable mistake. Geometry is drawn under the canvas transform
  rather than by transforming vertices in JS.
- **WebGL is not flattered.** No instancing (varied real rooms would never permit
  it), no scene graph doing its own culling, and text goes through a real glyph
  atlas rebuilt every frame rather than being quietly omitted.

## Known limitations, stated up front

- **Polygons are star-shaped about their centroid.** This is what makes the
  WebGL page's centroid-anchored fan a correct triangulation without an earcut
  dependency. It is weaker than requiring convexity, but an L-shaped room still
  cannot appear here. Real plates have some.
- **Labels come from a pool of 4 096 pre-built pairs**, indexed by room. A few
  million distinct strings is a hundred-odd MB of overhead unrelated to
  rendering, and real plates repeat room names heavily. What matters for the
  text layer — that widths vary across the visible set — is preserved.
- **WebGL draws the whole static buffer when ≥ 98% of rooms are visible**
  instead of rebuilding an index list. Canvas2D has no equivalent and
  structurally cannot. That is not a thumb on the scale; it is the retained-GPU-
  buffer advantage the comparison exists to measure.
- **One glyph atlas at one size**, scaled per line. An SDF atlas is the usual
  next step and is deliberately not built — this measures the cheap end of
  WebGL text, not the best possible one.
- **Line width differs by a hair at DPR > 1**: Canvas2D strokes one CSS pixel,
  WebGL one device pixel. Visually minor, perf-irrelevant.
- **One machine, one GPU.** Recorded in the results header along with DPR.

## Files

```
gen.js         synthetic geometry — shared
harness.js     index, cull, pick, label policy, camera script, sweep, capture — shared
index-2d.html  Canvas2D draw layer
index-gl.html  WebGL draw layer
poc.css        chrome for both pages
vendor/        flatbush 4.6.2 (UMD), regl 2.1.0 (min) — vendored, no toolchain
RESULTS.md     written after the sweep
```
