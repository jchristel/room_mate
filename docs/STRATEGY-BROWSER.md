# Roommate — Browser

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[MCP](STRATEGY-MCP.md) · [Authored](STRATEGY-AUTHORED.md) ·
[Entities](STRATEGY-ENTITIES.md) · [Security](STRATEGY-SECURITY.md)

**Open work only.** The viewer is a WebGL plan with a thin SVG overlay, three
sibling static pages, and a `src-js/` TypeScript build emitting one committed
bundle. How each part works is documented where it is built — `src-js/renderer/`,
`static/index.html`, `static/graph.js` — and the invariants that are expensive to
rediscover are below rather than in the code, because they are properties of the
*seam* between two layers and no single file owns them.

## Deferred

- **Serve `model_to_shared`, then consume it.** A model may carry a placement
  affine on its *upload* envelope, and the server validates and stores it — but
  **no read endpoint serves it**, so the renderer does not merely ignore it, it
  cannot see it. The first step for every feature below is therefore a *server*
  change (surface the per-model transform on `/rooms`, following the
  `boundary_by_level` precedent), not a browser one:
  - north alignment,
  - a real-world scale bar,
  - the georeferencing map underlay.

  Composing it is then a browser job — the existing Y-flip *plus* the transform
  *plus*, for the underlay, a reprojection into the tile frame — and the server
  stays out of it. It emits the transform as data; the renderer composes the
  picture.

- **Surface `measurement_standard` and `wall_gap_by_level` in the band-1 areas
  block.** `/areas` returns both and the UI ignores both. An area figure without
  its definition is exactly what a measurement standard exists to prevent, and a
  reader has no other way to tell a centreline level (walls already inside the
  rooms) from a finish-face one (walls filled to a declared thickness). Left
  undone deliberately: it is a UI decision about where a *per-level* fact belongs
  in a *scope-level* table, not a mechanical follow-on from the server change.

- **Door labels on the plan.** The inspector answers "what is this door" as
  ordinary DOM, so on-plan door text is a cost to take deliberately rather than a
  gap to close by default — see the label build cost below. Needs `door_label` in
  project settings (mirroring `room_label`), which is also unbuilt.

- **Level-of-detail.** Not needed and not built; the labels toggle already puts
  the manual half in place, so an automatic mode would drive the same
  `paintLevel` flag from zoom level rather than from a button. The grid is also
  still not capped to the visible region. **Revisit only if measurement says so.**

- **Zoom-responsive area tier labels.** A group too small for legible text gets
  no label, and the threshold derives from the level's *fitted* bounds rather
  than the current view — so a group suppressed at floor scale stays unlabelled
  however far you zoom. The fix is driving the areas overlay from the pan/zoom
  path, throttled. Deferred pending need.

- **Label build cost, if level-switch latency ever needs attention.** On a
  5,046-room level roughly 1.1 s of the build is constructing that many
  `BitmapText` objects; the geometry alone is ~180 ms. The fix is folding glyphs
  into the same attribute mesh the fills use. Nothing needs it today.

- **A multi-project comparator gets its own page**, following the
  `comparison.html` precedent — never a mode flag on the viewer. Scope is global
  to the page by decision, and re-introducing per-zone scope to allow
  cross-project side-by-side would restore the entire focus model that was
  deliberately deleted.

- **Out of scope, recorded so it is not re-proposed:** all-levels-in-one-file
  export, raster (PNG/PDF) export, and a graph export. "Export SVGs" is a plan
  feature.

- **Considered and deliberately not built: a checkbox property picker in the
  inspector.** The hide-empty toggle and the name filter covered the cases it was
  for, and unused UI is worse than none. If users do start re-picking the same
  columns every session, the durable answer is extending `room_label` in project
  settings — server-side, per project, shareable — rather than adding
  `localStorage` here.

## The hybrid's one invariant

The plan is two layers — a WebGL canvas with a transparent `<svg>` over it — and
**they must agree about coordinates and about paint order.** Nothing enforces
that, and the DOM will actively mislead you about it, which is why it is written
here once rather than left in the three comments where it was each discovered.

Every bug this produced was the same shape: *a property SVG gave away for free
that has to be reconstructed once the plan is a canvas with an overlay on top.*

- **Aspect.** A `viewBox` defaults to `preserveAspectRatio: xMidYMid meet` —
  uniform scale, letterboxed. A GL projection has no such default and will
  stretch the view rect onto the whole canvas. `fitViewToAspect` reproduces the
  SVG rule for GL, and the projection, the label transform and the pick all read
  it. The overlay is given the **raw** view, because SVG applies the correction
  itself; correcting it twice is the other way to get this wrong.
- **Coordinate space.** The overlay draws in world coordinates, so it needs its
  `viewBox` kept in step by the renderer. An `<svg>` without one is in *pixel*
  space, and every footprint collapses into the top-left corner.
- **Paint order.** DOM order does not decide it. The canvas is
  `position: absolute`, and CSS paints positioned elements after non-positioned
  ones regardless of document order, so the layers carry **explicit z-indices**.

**Why tests could not catch these:** `pointer-events: none` on the canvas removes
it from hit-testing but not from painting, so `document.elementFromPoint` happily
reports the `<svg>` as topmost while the canvas draws over it. A DOM assertion
cannot see a paint-order fault. Check these with **pixel readback** — render and
read in one synchronous turn, because a WebGL drawing buffer is cleared once the
compositor presents it.

### What belongs on the overlay

The original rule was "things there are dozens of, not thousands" — a performance
test. It is the wrong one, and following it put the hover highlight on the
overlay where it covered the label of the room being pointed at.

**The rule is occlusion, not size: the overlay is for marks that add pixels
without removing any.** A selection stroke with `fill: none` composites
harmlessly over anything; an opaque fill cannot, and belongs in the GL layer.

On that rule the overlay currently earns its place carrying the areas footprints
(even-odd paths with interior rings, per-group colour through a custom property,
its own labels — dozens of shapes, and moving them to GL would buy a multi-ring
triangulator and a custom-property emulation for nothing) and the selection mark
(one stroke, which keeps the stylesheet as the single definition of how selection
looks).

**When to revisit:** if the areas overlay ever has to scale past dozens, or if a
fourth ordering bug appears. Either is evidence the overlay has stopped paying
for itself. An abstract preference for one technology is not.

## What the WebGL move cost, and what would undo it

Room labels are pixels in a canvas, not `<text>` nodes. They are therefore **not
selectable, not searchable with the browser's own find, and not exposed to a
screen reader**, and **a browser without WebGL shows no plan at all** — the SVG
live renderer was deleted rather than kept as an unexercised fallback, on the
grounds that two live renderers is a permanent tax on every frontend change.

Three things blunt the accessibility loss and none of them undo it: the **SVG
export** still emits real `<text>`, so the selectable, searchable artefact exists
on demand and is what leaves the browser; the **inspector** shows the selected
room's properties as ordinary DOM; and **search** matches server-side data rather
than rendered glyphs, so finding a room by name still works — it is Ctrl+F over
the plan that does not.

This is recorded as an accepted trade rather than a defect, but it is the thing to
revisit if accessibility becomes a requirement rather than a preference. Reviving
a second live renderer is not the answer; making the export path a first-class
view would be.

## UI growth: toward a richer browser tool

The goal is a richer browser tool run locally, not a desktop app.

- **Keep axum a pure JSON API. This is the load-bearing decision.** The server
  emits data over HTTP, never HTML, and never assumes what the UI looks like.
  Holding this line is what keeps every later choice reversible and local, and it
  is why CSV export, colour maths, QA rendering and area tabulation are all
  client-side: each is a presentation reshuffle of data the browser already
  holds, so none of them earned a server endpoint.

- **A build step is not a framework, and this is still not one.** Vite +
  TypeScript over `src-js/` emits one committed IIFE the page calls. There is no
  component model, no router, no virtual DOM, no reactive store. The fork below
  is therefore still open — the toolchain does not pre-commit it, though it does
  lower the cost of the JS-framework branch and raise the relative cost of the
  Rust+WASM one, which would now be a second toolchain rather than the first.

- **Which signal actually fired is worth knowing, because it was not the
  predicted one.** The advice was "grow the vanilla JS until it hurts", and the
  predicted hurt was a feeling — the same state written into several DOM places,
  drifting. That never arrived, and it still has not. What broke it was a hard
  capability the page could not reach without dependencies: a WebGL plan layer
  needs polygon triangulation with holes, a glyph atlas and batched draw calls,
  all solved problems that must not be written again here. **The framework
  question and the toolchain question are separate, and only the second has been
  answered.**

- **When it does hurt, the fork is JS framework vs. Rust+WASM.** Behind axum,
  either a JS framework (Svelte gentlest, React most-supported) or a Rust+WASM
  one (Leptos / Dioxus). The project tilts toward **Leptos / Dioxus**: the Rust
  `Room` / `Level` / processed-geometry structs can be reused directly in the UI,
  eliminating the recurring friction of re-describing a carefully versioned
  contract in TypeScript. The trade is a smaller ecosystem and fewer ready-made
  components — a fair deal for a single-developer tool valuing one language and
  shared types end to end.

- **The trigger for a router or a state library is unmet.** Selection
  persistence is a small URL + `localStorage` fix on purpose. The stated trigger
  — writing the same state into several DOM places and watching them drift — has
  not been hit.

## Endpoints follow fetch lifecycle, not data type

As capabilities are added, give each its own **purpose-shaped endpoint** rather
than overloading `/rooms`. `/rooms` stays raw geometry and new endpoints carry
derived data. Small endpoints mean any future frontend composes them freely, and
no presentation assumption gets baked into the data layer.

The principle is **not** "one endpoint per data type" — it is "one endpoint per
thing fetched independently, on its own schedule, by its own consumer." The test:
*would this ever be fetched on a different trigger, or be expensive enough that it
shouldn't sit in the default payload?*

- **No → keep it in the snapshot.** Levels are the worked example: the viewer
  needs levels and rooms *together*, in the same render pass, from the same push.
  They share a lifecycle. Splitting them would mean two requests that always
  travel together, recombined client-side, with a race between them — cost, no
  benefit.
- **Yes → its own endpoint.** Derived data recomputed on a different trigger,
  sized differently, or consumed by a different part of the UI. Adjacency is the
  worked example: fetched when the *selection* changes rather than on the poll,
  and feeding a canvas rather than the plan.

Two distinctions the shipped endpoints have already drawn, worth reusing rather
than re-deriving:

- **Expense and independent versioning are different reasons, and the second is
  stronger.** `/adjacency` is on-demand because it is *expensive*. `/doors` is a
  separate poll because doors are *independently versioned and independently
  pushed* — their own `taken_at`, their own milestone pins, their own `revision`
  — so folding them into the room payload would make one revision stand for two
  lineages that move apart.
- **A join is not an endpoint.** Reference sources and classification are
  resolved at `/rooms` response assembly rather than given endpoints of their
  own, because today they still share the viewer's render pass. Each becomes a
  candidate for its own endpoint the moment it refreshes on a different trigger
  (a live source poll) or serves a different consumer (a hierarchy browser) — see
  the deferred `/hierarchy` in [Server](STRATEGY-SERVER.md).

## When the renderer moves again

WebGL is the current choice and is likely right for a long time; the plan is four
draw calls whatever the room count. Recording the escalation rule anyway, because
it is what decided the *last* move and will decide the next:

**The trigger is element count on screen, or a need for continuous animation —
never "draw shapes on top."** Drawing on top is well inside SVG's comfort zone,
which is why the overlay still exists. Continuous animation is what took the
adjacency graph to canvas at *tens* of nodes: SVG is retained-mode with no render
loop, so a layout that settles over a run of frames fights the model regardless
of how few elements it has. Element count is what took the plan to WebGL, and the
number that justified it was a fitted 5,000-room level at ~0.5 s/frame.

Because the server emits geometry as data, the renderer stays swappable without
touching the server or the extractor — so this decision can always be deferred
until measurement demands it.
