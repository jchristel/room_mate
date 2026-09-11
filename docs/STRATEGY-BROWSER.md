# Roommate — Browser

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[MCP](STRATEGY-MCP.md) · [Authored](STRATEGY-AUTHORED.md) ·
[Entities](STRATEGY-ENTITIES.md) · [Security](STRATEGY-SECURITY.md)

**Open work only.** The viewer is a WebGL plan with a thin SVG overlay, three
sibling static pages, and a `src-js/` TypeScript build emitting the committed
renderer bundle and a React preview of the settings page. How each part works is documented where it is built — `src-js/renderer/`,
`static/index.html`, `static/graph.js` — and the invariants that are expensive to
rediscover are below rather than in the code, because they are properties of the
*seam* between two layers and no single file owns them.

## Deferred

- **The room inspector's "in this room" section cannot say which model the
  room is in.** The section ships for doors, windows and FF&E; what is unbuilt
  is the disambiguation. A room id is unique only *within* a model, and
  `/rooms` does not serve one — `RoomResponse::model_id` is skip-serialized so
  the rooms JSON stays byte-for-byte unchanged — so the panel matches on a bare
  id and can only *detect* an ambiguous one, never resolve it. It says so when
  it happens, which is the same call the layer toggles make with their "(by
  elevation)" suffix.

  **Not yet worth the wire change**, and the measurement is why: no room id on
  RHH is claimed by more than one model, across 3,013 rooms, 1,857 doors and
  38,913 items in five models. The signal to serve the field is a project where
  that stops being true — the note is the instrument that would say so — not a
  preference for stronger keys. Note that the viewer's whole selection model is
  bare-id (`selectRoom(roomId)`), so serving `model_id` on `/rooms` alone would
  not finish the job.

- **Serve `model_to_shared` itself.** *Aligning* linked models is done and is
  not a browser concern any more: the server places every read's geometry into
  one project-local frame (`service::placement`), so the renderer draws models
  that line up without knowing a transform exists. What is still deferred needs
  the raw, survey-absolute affine, which no read endpoint serves:
  - north alignment,
  - a real-world scale bar,
  - the georeferencing map underlay.

  So the first step for these is still a *server* change — surface the per-model
  transform on `/rooms`, following the `boundary_by_level` precedent. **It must
  be served as a transform for the renderer to compose with, never applied to
  the coordinates**: shared space is the survey grid, and survey-magnitude
  coordinates quantise to ~324 mm in the f32 vertex buffers the GL renderer
  uploads. That constraint is why the alignment above is project-local, and it
  is written up in `service::placement`'s module header with the measurements.

- **Surface `measurement_standard` and `wall_gap_by_level` in the band-1 areas
  block.** `/areas` returns both and the UI ignores both. An area figure without
  its definition is exactly what a measurement standard exists to prevent, and a
  reader has no other way to tell a centreline level (walls already inside the
  rooms) from a finish-face one (walls filled to a declared thickness). Left
  undone deliberately: it is a UI decision about where a *per-level* fact belongs
  in a *scope-level* table, not a mechanical follow-on from the server change.

  **No longer hypothetical as of 2026-08-25.** A centreline project now reports
  `0` on every level while House A reports `1.5`, so two projects one dropdown
  apart produce areas that mean measurably different things, and the UI
  presents them identically.

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

- **A build step is not a framework.** Vite + TypeScript over `src-js/` emits
  one committed IIFE the viewer calls, and a second Vite config
  (`vite.settings.config.ts`) builds the React settings page into
  `static/settings/`, served at `/settings/`. The hand-written
  `static/settings.html` it replaced was deleted on 2026-09-12, so the two are no
  longer comparable — what they cost relative to each other is the table below,
  measured while both existed. The viewer and the other two pages still have no
  component model, router or store.

- **Which signal actually fired is worth knowing, because it was not the
  predicted one.** The advice was "grow the vanilla JS until it hurts", and the
  predicted hurt was a feeling — the same state written into several DOM places,
  drifting. What broke the zero-build rule was instead a hard capability the page
  could not reach without dependencies: a WebGL plan layer needs polygon
  triangulation with holes, a glyph atlas and batched draw calls, all solved
  problems that must not be written again here. **The framework question and the
  toolchain question were separate**, and the framework one was answered later,
  by measurement — below.

- **The fork is answered: a JS framework (React), not Rust+WASM.** This doc used
  to tilt toward Leptos / Dioxus on one argument — reuse the Rust structs in the
  UI instead of re-describing a versioned contract in TypeScript. On 2026-09-11
  the same slice of the settings page (project list, identity, room label, save)
  was built both ways, against the running server, and put through the same
  browser tests. Both worked. What they cost:

  | | Leptos (Rust+WASM) | React (TypeScript) |
  |---|---|---|
  | Download, gzipped | ~164 KB | ~71 KB |
  | First build | ~6 min | ~5 s |
  | Rebuild after an edit | 5 s debug, 75 s release | ~5 s, production build |
  | New tooling | pinned rustc, wasm32 target, trunk, wasm-bindgen, wasm-opt | 5 npm packages |
  | `common.js` helpers | ported to Rust | reused |
  | Server types | used directly | 4-field hand-written subset |

  - **The deciding fact is reproducibility, not size.** A committed generated
    artifact is only safe behind a rebuild-and-compare gate, and a Rust wasm
    build embeds source paths for its panic locations: absolute registry paths,
    and on Windows, backslashes inside them. `--remap-path-prefix` removes the
    machine-specific prefix but not the separators, and `trim-paths` is unstable
    (cargo 1.95). A wasm built on the development machine therefore never matches
    a Linux CI rebuild. The Vite output carries no machine path and rebuilds
    byte-identically.
  - **Shared types bought less than argued, and brought a bug with them.** Using
    `Settings` directly meant echoing its TOML-shaped `Serialize`, which *skips* a
    `None` name; `merge_over_stored` then keeps the stored one, so the typed
    client could not clear a display name. The React page avoids it only by
    sending `null` on purpose. The hazard is the merge semantics, not the
    language, and on either side it is a test that guards it.
  - **What survived:** `crates/roommate-shared`, the settings types and the
    settings API's wire shapes in a crate with no server dependencies.

- **Open: generate the RENDERER's wire types too.** The settings tree is
  generated (ts-rs, `crates/roommate-shared` → `src-js/settings/generated/`,
  gated in `rust.yml`); `src-js/renderer/types.ts` is still a
  hand-written subset. It stays one while it is a handful of fields the renderer
  actually touches — the signal is the one the settings page hit: a page that
  needs *most* of a type.

- **What would reopen Rust+WASM:** stable path trimming in cargo, so a committed
  wasm can pass the rebuild gate; or a need to run shared *logic* in the browser
  — `service::areas` geometry, say — which generated types cannot carry. A
  preference for one language end to end is not on that list: it was weighed
  against the table above and lost.

- **The trigger for adopting React on a live page is still unmet**, and it is the
  one this doc has always named: the same state written into several DOM places
  and drifting. The one instance found so far — the four copy-pasted entity polls
  in `index.html` — was fixed by extracting a typed module, not by a framework
  (PR #120). Selection persistence remains a small URL + `localStorage` fix on
  purpose.

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
