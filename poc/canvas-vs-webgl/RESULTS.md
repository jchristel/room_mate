# RESULTS — Canvas2D vs WebGL

Measured 2026-08-01. Budget and method were fixed in [README.md](README.md)
before any number existed.

## Verdict

**Go WebGL. Do not build the Canvas2D client.**

Canvas2D holds the budget comfortably while the viewport cull is doing its job,
and fails it at any zoom level that puts a whole plate on screen — including
plates smaller than the one already in this repo. Since the viewer auto-fits on
load, the zoomed-out view is not an edge case, it is the *first thing a user
sees*. Canvas2D is already over budget there on today's largest real fixture,
before any growth at all.

The crossover is not a room count. **It is a zoom level**, and both renderers
sit on the same side of it at every room count tested.

## Conditions

DPR **1**, CSS viewport 1200×800, backing store 1200×800, Chromium 148
(Electron 42) on Windows 10. Both sweeps run with **no other tab open** — an
earlier run with a second heavy tab resident measured Canvas2D's pan at
15–25 ms against the same code's 10.6 ms clean, so single-tab is load-bearing,
not hygiene.

## The curve — p50 / p95 of per-frame work, ms, labels on

| rooms | Canvas2D pan | WebGL pan | Canvas2D **fitted** | WebGL **fitted** | fitted ratio |
|---|---|---|---|---|---|
| 1,000 | 8.8 / 10.8 | 1.5 / 2.8 | 12.3 / 19.7 | 1.0 / 1.8 | 12× |
| 2,000 | 8.2 / 9.7 | 1.2 / 2.6 | 16.7 / 21.3 | 0.4 / 1.4 | 42× |
| 4,000 | 8.9 / 10.7 | 1.7 / 2.8 | 27.3 / 29.5 | 0.5 / 1.0 | 55× |
| 8,000 | 9.5 / 10.6 | 1.7 / 2.7 | 48.6 / 85.1 | 0.9 / 2.9 | 54× |
| 12,500 | 9.3 / 10.8 | 1.2 / 1.7 | 71.9 / 77.1 | 0.7 / 1.1 | 103× |
| 25,000 | 9.4 / 10.7 | 1.7 / 2.8 | 132.7 / 233.8 | 2.8 / 3.3 | 47× |
| 50,000 | 9.3 / 10.7 | 1.6 / 2.7 | 249.3 / 265.2 | 3.8 / 4.6 | 66× |
| 100,000 | 9.0 / 11.0 | 1.6 / 2.9 | 477.5 / 592.7 | 5.8 / 6.8 | 82× |
| 200,000 | 9.0 / 10.6 | 1.6 / 2.7 | 921.5 / 1685 | 7.6 / 9.1 | 121× |
| 400,000 | 8.9 / 10.6 | 1.4 / 2.4 | 1804 / 3127 | 14.5 / 17.1 | 124× |
| 800,000 | 8.6 / 10.5 | 1.7 / 2.9 | 3483 / 3695 | 29.7 / 37.1 | 117× |
| 1,600,000 | 9.2 / 10.1 | 1.9 / 3.4 | 6896 / 7237 | 70.5 / 100.5 | 98× |

Rooms drawn during pan held at 213–219 at every rung on both pages, and labels
at 194–202. The cull was live and the rungs are comparable.

## The two crossovers

**Pan — neither renderer ever breaks budget.** Canvas2D sits at ~9 ms p50 from
1,000 rooms to 1,600,000, WebGL at ~1.6 ms. The cull makes this phase O(rooms on
screen), and rooms on screen is constant by construction, so the curve is flat by
1,000× of scaling. There is no crossover here and there never will be. **The
brief's budget question — "does Canvas2D hold 16 ms on sustained pan at 50k" —
has the answer "yes, and also at 1.6 million", and it is the wrong question.**

**Fitted — the crossover is off the bottom of the ladder.**

| | Canvas2D | WebGL | ratio |
|---|---|---|---|
| largest plate holding 16 ms p50 | **~1,900 rooms** | **~440,000 rooms** | ~230× |
| largest plate holding 16 ms p95 | **under 1,000 rooms** | **~370,000 rooms** | >370× |

Canvas2D's fitted cost is linear at **4.3 µs per room** above ~2,000 (r² is
essentially 1 across three orders of magnitude). WebGL's is linear at
**~40 ns per room**.

By the README's "clear-winner" test — Canvas2D at ≥2× WebGL — WebGL wins at
**every rung on the ladder**, starting at 1,000 rooms. There was never a range
where they were close.

## Against real data

`big-plate`, the largest fixture in this repo, is **5,046 rooms per level**.

- **Canvas2D, fitted:** ~33 ms/frame p50 — **2× over budget, ~30 fps**, today,
  with no growth.
- **WebGL, fitted:** ~0.6 ms/frame — **25× inside budget**, with room to grow
  the plate ~90× before it reaches Canvas2D's *current* position.

For reference, `STRATEGY-BROWSER.md` records the SVG renderer at ~0.5 s+/frame on
a fitted 5,000-room level. Canvas2D is ~15× faster than SVG there and still
misses the budget; WebGL is ~800× faster than SVG and clears it by a wide margin.

## Two things the brief got wrong

Both were checked because they were checkable, not to score points — each one
would have changed what got built.

**1. "Labels are the single most decision-relevant variable" — they are not, and
they favour WebGL.** Measured at pan, ~199 two-line labels:

| | labels off | labels on | cost of text |
|---|---|---|---|
| Canvas2D | 6.6 ms | 9.0 ms | **+2.4 ms** |
| WebGL | 0.57 ms | 1.6 ms | **+1.0 ms** |

The brief expected the glyph atlas to be WebGL's weak point and `fillText` to be
"cheap-ish". In absolute terms the atlas is **2.4× cheaper** than `fillText`.
Text is not what decides this, and where it does lean, it leans the other way.
This is consistent with the earlier SVG finding in
`docs/Superseded/HANDOVER-viewer-performance.md` ("Labels are NOT the
bottleneck") and inconsistent with the brief's account of it.

**2. "SVG is unusable at 5k rooms *under continuous pan*" — pan is the case that
was already fixed.** Viewport culling shipped after the brief was written
(`paintLevel`/`cullZone`), and pan is now O(on-screen). The failure that remains,
in SVG and Canvas2D alike, is the fitted view. Had this POC measured only the
pan phase as the brief specifies, **both renderers would have passed at every
rung and the exercise would have concluded that Canvas2D is fine.**

## Caveats

- **DPR 1.** A retina display roughly 4×'s fill cost. That erodes Canvas2D's
  fitted numbers further and eats into WebGL's margin; it does not plausibly
  reverse a 100× gap. Not measured.
- **One machine, one GPU.** Integrated graphics, Windows 10. A weaker GPU
  narrows WebGL's advantage; a weaker CPU widens it.
- **High-rung p95 is thin.** Adaptive sampling drops the fitted phase to its
  12-frame floor above 100,000 rooms, so those p95 values are close to "worst of
  12". The p50 column is the trustworthy one up there, and it is the one the
  crossovers are computed from.
- **WebGL's `zoom` phase is worse than it needs to be** (379 ms p95 at 1.6M).
  Mid-zoom falls outside this POC's ≥98%-visible fast path, so it rebuilds a
  per-frame index subset of hundreds of thousands of rooms. A real client would
  use persistent tiled buffers or an LOD switch and would not pay this. It is a
  property of the throwaway, not of WebGL — do not carry this number forward.
- **Geometry is star-shaped about its centroid**, so no L-shaped rooms. See
  README.md's limitations.

## What this implies for the real work

Not built here, recorded for whoever picks up the rewrite:

- The renderer decision is settled: **WebGL**. The cost that has to be budgeted
  is the glyph atlas and the buffer-management strategy, not the fill rate.
- The **fitted view is the load-bearing case**, so level-of-detail belongs in the
  first design, not a later optimisation. `STRATEGY-BROWSER.md` already carries
  "fitted-view cost at very high room counts — still open" and this is the
  measurement behind it.
- The serving-shape question the brief flags is real and unblocked by any of
  this: an overview response of **bounding boxes + labels only**, split from the
  full-polygon payload, is what makes a fitted view cheap to *fetch* as well as
  cheap to draw. The POC generated its own data and did not test it.
