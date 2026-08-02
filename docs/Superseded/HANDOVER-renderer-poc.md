# HANDOVER — Renderer POC: Canvas2D vs WebGL at 50k polygons

> **Superseded — the POC ran, the verdict was WebGL, and the renderer built on
> it has shipped.** Its numbers are reproduced in
> [HANDOVER-webgl-renderer.md](HANDOVER-webgl-renderer.md) (the POC folder was
> deliberately never on `main`); the shipped result is in
> [STRATEGY-BROWSER.md](../STRATEGY-BROWSER.md) "Renderer".
>
> Its method held up where it mattered — insisting both candidates get the same
> spatial index, and refusing a geometry-only benchmark — and both instincts
> were vindicated: the shipped renderer uses a Flatbush index for the pick, and
> labels turned out to dominate, though as *construction* cost rather than draw
> cost, which no draw-time benchmark would have found.
>
> Two of its cautions did not survive the real build. **"Instrument DPR"**
> anticipated retina roughly 4×-ing fill cost; measured on the real renderer it
> barely registers, because a frame is four draw calls rather than fill-bound.
> And the POC's own framing — that the choice was Canvas2D versus WebGL — left
> out what actually cost the most effort: non-scaling stroke widths, which
> neither candidate provides and which a scene graph cannot express.

**Status:** not started. This is a brief, kept as one. It commissions a
throwaway measurement, not a feature — its only deliverable is a number and a
verdict about which renderer the viewer's next generation is built on.

**Goal:** decide whether Canvas2D is sufficient for a floorplate viewer at scale,
or whether WebGL is forced. Build two disposable proofs of concept drawing ~50k
2D polygons, measure sustained interaction performance, and pick one.

**Why now:** SVG is proven unusable at ~5k rooms *plus labels* on a real
floorplate — DOM node count under continuous pan/zoom is the wall. The adjacency
graph already moved *one* mode off SVG onto canvas (see
HANDOVER-adjacency.md), but that was justified by an animation trigger, not
element count. The plan renderer is still SVG and is the thing that actually has
to scale. This POC settles what replaces it before the rewrite is committed.

---

## Scope decisions (settled — do not re-litigate without saying so)

| Question | Decision | Why |
|---|---|---|
| 2D or 3D? | **2D, permanently.** Bounding boxes and simple polygons (4–12 vertices). | The data is 2D footprints. No mesh geometry, no depth. This is what rules out the whole BIM-geometry-format family (Fragments/FlatBuffers/WebGL-mesh pipelines) — they solve a problem we do not have. |
| Does this need a new storage format? | **No.** | 2D boxes/polygons are the primitives the contract already carries (`Loop`/`Point2D`). "50k of them" is *volume*, not a new *kind* of data. The pressure is on rendering and on read-time query, never on the on-disk format. |
| Where does it live? | **A disposable folder outside the crate graph** — e.g. `poc/canvas-vs-webgl/`. Static HTML+JS, opened in a browser or served by any trivial static server. | It is a browser benchmark of JS/DOM APIs; a Rust crate is the wrong unit. It generates its own synthetic geometry, needs neither the server nor real data, and is deleted once the decision lands. Keep it out of the workspace so it never couples to `roommate`'s compile. |
| Real client location? | **Out of scope here.** | Where the shipped browser client sits relative to the Rust server is a STRATEGY-BROWSER.md question, settled when the *real* rewrite starts — not by this throwaway. |

The 2D decision is permanent; the others are POC-scoped.

---

## The one thing that is not the renderer

**The load-bearing piece is a spatial index, and it is shared by both
candidates.** Viewport culling, pick, and (later) snapping are all the same
operation — "nearest / overlapping geometry to a screen region" — and none of
them care what painted the pixels. Build this once and feed it to both POCs, or
the comparison is measuring the wrong thing.

- Library: **RBush** (R-tree, small, battle-tested) over room bounding boxes.
  **Flatbush** is the faster/immutable alternative and is a better fit if the
  box set is static per snapshot (it is).
- This is also what makes the whole exercise honest: the common way to make
  Canvas2D lose for a bogus reason is to redraw all 50k every frame with no
  cull. **Give both renderers the same viewport cull.** WebGL only earns the win
  if it still needs to *after* Canvas2D has one.

Note for later, not for the POC: snapping does **not** rule out Canvas. It is an
index query plus a cheap overlay redraw, identical cost under either renderer.
It is called out here only so the POC's index is built knowing snap will reuse
it.

---

## What to build

Two static pages, same synthetic dataset, same interaction harness, differing
only in the draw layer.

```
poc/canvas-vs-webgl/
  gen.js         synthetic geometry — shared by both
  index-2d.html  Canvas2D renderer
  index-gl.html  WebGL renderer (PixiJS or regl)
  harness.js     scripted pan+zoom, frame-time capture — shared
```

**Renderer libraries** (reach for these only if raw is painful; a bare first
pass is better for an honest ceiling — see below):

- Canvas2D: raw `CanvasRenderingContext2D` for the benchmark. Konva / Fabric.js
  exist for the *real* thing (scene graph, hit-testing, layers) but bring their
  own culling that muddies a raw measurement.
- WebGL: **PixiJS** (2D scene-graph API over WebGL — the natural fit) or **regl**
  (lower-level, less overhead for flat 2D). Raw WebGL is not worth it here.
- Spatial index (both): **RBush** or **Flatbush**, as above.

Do the first pass **bare** (plain Canvas2D loop vs. minimal Pixi/regl) plus the
index. Only reach for Konva/Fabric once a renderer is *chosen* and the real
client is being built — their built-in culling and hit-testing quietly answer
the exact questions the POC is trying to measure.

---

## How to measure (this is most of the value)

Get any of these wrong and the number is misleading.

- **Realistic geometry, not 50k identical squares.** Vary vertex count (4–12)
  and size. Identical geometry flatters both renderers — WebGL especially,
  inviting instancing that real varied rooms won't give you.
- **Measure sustained interaction, not a single draw.** Initial/static draw is
  nearly irrelevant — SVG could draw 50k once too. What killed SVG was per-frame
  cost under *continuous transform*. Script a pan+zoom and capture **frame time
  (ms) distribution**, not FPS. FPS clamps at 60 and hides headroom.
- **Include labels, or you answer the wrong question.** 5k *rooms* was
  survivable; rooms **plus labels** was not. A geometry-only POC makes Canvas2D
  look great, then labels sink it. Add a **zoom-culled** text layer (draw a
  label only when its on-screen bbox exceeds a pixel threshold). This is also
  where the two renderers diverge hardest — Canvas2D text is cheap-ish but
  redrawn every frame; WebGL text needs a glyph atlas / SDF — so omitting it
  removes the single most decision-relevant variable.
- **Hold interaction constant.** Include pick in both or neither, fed by the
  shared index. If one POC gets free hit-testing and the other builds it, the
  comparison is rigged.
- **Instrument device-pixel-ratio.** Retina/hidpi quietly 4×'s fill cost and can
  flip a Canvas2D "pass" into a "fail." Report it.
- **Test on target hardware.** Dev-machine GPU is not the target. If low-power
  laptops/tablets are in scope, measure there — the Canvas2D/WebGL gap widens on
  weak hardware, and that is where the decision is actually made.

**Set the budget before measuring**, so the result is pass/fail not vibes — e.g.
*"≤16 ms sustained frame time during pan at 50k, labels and pick included, at
target DPR on target hardware."*

---

## Definition of done

1. Both POCs draw the same ~50k varied-polygon synthetic dataset, both with
   viewport culling via the shared index, both with zoom-culled labels and pick.
2. A scripted pan+zoom captures a frame-time distribution for each, at target
   DPR, on at least one representative low-power device.
3. A one-paragraph verdict against the pre-stated budget: does Canvas2D hold it
   with labels + pick included? If yes, WebGL's text cost is not worth paying and
   the decision is Canvas2D. WebGL wins **only** if the sustained-pan number
   breaks budget *after* Canvas2D has been given viewport + label culling.
4. The folder is deleted. What survives is the verdict and the frame-time
   numbers, recorded in STRATEGY-BROWSER.md.

---

## What this reaches into (the only server-side consequence)

The renderer choice is client-only, with one exception worth flagging for the
*real* work that follows (not the POC): whichever renderer wins, the client wants
two things from `/rooms` it may not get cleanly today —

- each room's **bounding box** (cheap viewport culling without shipping full
  polygons), and
- ideally **geometry at the detail the zoom needs** (bbox when zoomed out, full
  polygon when zoomed in).

That is a serving-shape question — a possible lightweight "bboxes + labels only"
overview response split out from the full-polygon payload. It is out of scope for
the POC (which generates its own data) but is the first thing the real rewrite
should weigh. Record it in STRATEGY-BROWSER.md as a follow-on, don't build it
here.

---

## Docs to update on landing

- **STRATEGY-BROWSER.md** — record the verdict, the frame-time numbers, and
  which renderer the plan viewer moves to and why (measured budget, not vibes).
  Note the bbox/level-of-detail serving follow-on as an open item.
