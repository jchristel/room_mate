# HANDOVER — Room Adjacency Graph

**Status:** built and verified end-to-end against a live server; **one item
open** — validation against a genuine finish-face Revit model, which no fixture
in the repo provides. Every code item is landed and tested, the naive algorithm
was measured, found wanting on 5,000-room levels, and given the spatial index
this brief prescribed (~22s → ~2.5s). See the checklist at the bottom. Written as
a brief and kept as one: the reasoning above the DoD is still the reasoning, not
a retrospective.

**Goal:** derive a room-to-room adjacency graph from stored room geometry,
expose it on its own endpoint, and render it in the viewer as a radial
(focal-node) graph — pick a room, see what it touches, and what those touch.

---

## Scope decisions (settled — do not re-litigate without saying so)

| Question | Decision | Why |
|---|---|---|
| What counts as adjacent? | **Shared wall only.** Two rooms are adjacent when their outer boundaries run parallel within a wall-thickness tolerance over a non-trivial length. | Pure geometry from data already in the contract. Door-connectivity needs door elements the Revit extractor does not currently collect. |
| Cross-level? | **No. Single level only.** Rooms are adjacent only to rooms sharing their `level_id`. | Keeps the graph planar and the first version honest. Vertical stacking (risers, cores) is a real future case, not this one. |
| Where does it render? | **A mode inside `index.html`, drawn on a `<canvas>`** — not a standalone `graph.html`. | Bidirectional plan↔graph selection is the payoff, and selection is *page* state (see Prerequisites). Two static documents share nothing but the URL and localStorage. Canvas because a force simulation is the first thing in this project that actually meets the animation trigger — see "The viewer". |

The first two are *first-version* limits, not permanent ones. Design the types
so lifting them later is additive — see "Designed-in extension points".

---

## Prerequisites (built first, in this order)

Neither of these is adjacency work. Both blocked it, and one of them blocked
something else too — so they were sequenced here rather than smuggled into the
definition of done, and each landed on its own.

### 1. Room click-selection — shared with HANDOVER-ui-layout Decision 3

*Landed.* Before this, the viewer had **no room selection at all**: rooms carried
a hover `<title>` ([index.html:1008](static/index.html:1008)) and nothing more.
That was the exact prerequisite blocking the right-hand inspector —
[HANDOVER-ui-layout.md](HANDOVER-ui-layout.md) Decision 3 records it as "not
buildable yet: the viewer has hover `<title>` tooltips, not click selection".
**That block is now lifted**; the inspector is buildable whenever someone wants
it.

Two features wanted the same missing thing, so it was built once, on its own,
under Decision 3's rules — which had already settled the design questions:

- **Selection is page state, not zone state.** One selected room at a time
  regardless of which zone the click landed in.
- **The selection names the zone it came from.** That is a label, not
  `activeZoneId` machinery — selection is explicit (the user clicked), never
  inferred from where they last interacted.
- Clicking a room toggles a `.selected` class through the existing cull-unit
  room→nodes map, the same render-free fast path room search uses; it must not
  trigger a re-render, and must not disturb pan/zoom.

Land that alone. The inspector and this graph then both consume it.

**This is also what settles `graph.html` vs an in-page mode.** A separate static
document can only receive a selection through the URL or localStorage
([common.js:77](static/common.js:77)) — which cannot express "click a node,
the plan highlights *now*". Bidirectional selection therefore forces the graph
into `index.html`. If bidirectional selection is ever dropped from scope, a
standalone page with a deep-linked `?room=` becomes viable again and this
decision should be revisited; while it is in scope, it is not.

To keep `index.html` from absorbing the whole feature, the graph renderer lives
in its own **`static/graph.js`**, loaded as a classic `<script>` alongside
`common.js` and served by the same `ServeDir`. One document, one selection, one
zero-build vanilla layer — and a file boundary where the concern boundary is.

### 2. Extract the shared palette into `common.js`

*Landed.* "Node colour reuses the existing `qualitative("Set2", i)` palette" was
never free reuse: `qualitative`, `SCHEMES` and the two hex helpers were inline in
`index.html`, where `graph.js` could not reach them. They now live in
[common.js:51](static/common.js:51); the plan-specific maths that reads them
(`sampleScheme`, `lighten`) stayed behind.

This matters more than it sounds, and it is the reason it is a prerequisite
rather than a cleanup: two views disagreeing about what colour a department is
is worse than either being arbitrary. Copying the function would guarantee that
drift eventually.

---

## Why this earns its own endpoint

[STRATEGY-BROWSER.md](STRATEGY-BROWSER.md) already names "an adjacency graph"
as the canonical example of derived data that should **not** ride inside
`/rooms`. It passes the test there on both counts:

- **Different trigger.** Recomputed when a *room selection* changes, not on the
  2s room poll.
- **Different consumer.** A graph canvas, not the SVG plan renderer.

So: `GET /projects/{id}/adjacency`, modelled on the shipped
`/projects/{id}/areas` route. That is the closest precedent in the codebase and
should be read first — it is the same shape of problem (derive geometry
server-side, scope it, serve it, render it). Read it as a *structural* model
only: `/areas` has no `revision` field
([areas.rs:270](src/service/areas.rs:270)) and this endpoint needs one, so
copy its scoping and its 204, not its response type. Note that
`STRATEGY-BROWSER.md` names the future endpoint `/adjacencies`; the singular,
project-scoped form here matches the `/areas` precedent and is the one to build
— the strategy doc gets corrected on landing, not the route.

This is also the second real geometry-processing service, after `areas`.
[STRATEGY.md](STRATEGY.md) flags adjacency graphs specifically as the kind of
work that finally makes the Rust server's performance advantage *real* rather
than potential. Worth noting in the module header.

---

## Where the code goes

```
src/service/adjacency.rs      new — the algorithm + result types
src/handlers.rs               new handler `get_project_adjacency`
src/main.rs                   new route registration
src/bin/mcp.rs                new tool `get_adjacency`
static/graph.js               new — the radial canvas renderer
static/index.html             the graph mode: toolbar, panel, selection wiring
```

`src/bin/mcp.rs` is **not optional**. Its module header declares one tool per
existing HTTP read route ([mcp.rs:2](src/bin/mcp.rs:2)), and
`get_hierarchy_areas` ([mcp.rs:296](src/bin/mcp.rs:296)) is the worked
example: a thin adapter over the same one service function the HTTP handler
calls. Skipping it is how the two front doors start to drift.

Follow the existing conventions ([CODING-CONVENTIONS.md](CODING-CONVENTIONS.md)):

- `service/` is **transport-agnostic**. `adjacency.rs` imports `geo` and
  `crate::contract`; never `axum` or `rmcp`. Each handler is a thin adapter that
  extracts query params, calls exactly one service function, and translates the
  result.
- Unit tests live **inline** at the bottom of `adjacency.rs` as
  `#[cfg(test)] mod tests`. `areas.rs` has good worked examples of geometry
  tests built from small `rect()` helpers — copy that style.
- If the module passes ~500 non-test lines, split it into `adjacency/` with a
  `mod.rs` that re-exports, so `crate::service::adjacency::X` never moves.
- **Annotate the why.** Every non-obvious constant (especially the tolerances
  below) carries a doc comment explaining the decision and what would break
  otherwise — not a restatement of the what.

---

## The algorithm

Input is a scoped, level-filtered set of rooms. Reuse the existing plumbing
rather than reimplementing it:

```rust
let scope = RoomScope { project: Some(project), building, milestone, ..Default::default() };
// assemble_rooms(...) — same entry point areas::assemble_areas uses
```

Then, per level:

1. **Explode each room's outer loop into edge segments.** `loops[0]` only —
   interior loops are the room's own holes (a column, a shaft) and cannot bound
   a neighbour. This mirrors `areas::room_outer_polygon`, which drops interior
   loops by construction for the same reason.
2. **Pair rooms and test their segments.** For each candidate segment pair:
   - **Parallel** within an angular tolerance;
   - **Separated** by a perpendicular distance within the wall tolerance —
     `0 ≤ d ≤ wall_max`, *inclusive of zero*. See "Boundary location" below:
     zero is not a degenerate case, it is one of the two normal ones;
   - **Overlapping** when projected onto the shared axis, by more than a minimum
     length.
3. **Accumulate** total overlap length per room pair. Two rooms can share
   several disjoint segments (an L-shaped junction); sum them.
4. **Emit** one edge per pair whose accumulated shared length clears the
   minimum.

### Boundary location — two regimes, one parameter

Revit's `SpatialElementBoundaryLocation` decides where a room's boundary sits,
and **both settings are in play**. The algorithm must handle both, because the
server cannot tell which it is looking at: the setting lives in the Revit
document, and the extractor does not put it on the contract.

- **Centreline** — neighbouring rooms tile edge-to-edge and their shared
  boundaries are **coincident**. Perpendicular distance is `0`, up to float
  noise. This is what `areas` assumed
  ([HANDOVER-hierarchy-areas.md](Superseded/HANDOVER-hierarchy-areas.md)), what
  every fixture generator in `scripts/` produces
  ([gen_big_plate.py:80](scripts/gen_big_plate.py:80) steps by `CELL_W` with
  no gap), and what `areas`' dissolve tests assert
  ([areas.rs:673](src/service/areas.rs:673)).
- **Finish face** — rooms float inside their walls, so neighbours across a wall
  are offset by roughly the wall thickness. Perpendicular distance is a real,
  positive number.

**The tolerance is the thing that spans the two.** That is the honest reason
`wall_max` is a request parameter with a slider rather than a compiled constant
— not merely that the right value is unknown, but that the right value is a
property of the model in front of you. On a centreline model the correct
setting is at or near zero; on a finish-face model it is just over the thickest
partition. One control, two regimes.

Three consequences the implementation must respect:

- **Zero is a valid `wall_max`,** and the acceptance band is closed at both
  ends. Validation rejects negative and non-finite values; it must *not* reject
  zero as "non-positive".
- **Coincident edges still need an epsilon.** Two rooms whose shared edge
  disagrees at the 1e-9 level are the same edge — `areas` already banked that
  Revit exports are not bit-identical across rooms
  ([areas.rs:732](src/service/areas.rs:732)). Fold a
  `COINCIDENT_EPS_FT` into the test (`d ≤ wall_max + COINCIDENT_EPS_FT`) so a
  slider at zero still matches a centreline model. Name it as a sibling of
  `areas.rs`'s `COLLINEAR_EPS_FT` and give it the same kind of comment.
- **The failure modes differ by regime.** At centreline, corner touches are
  exact point contacts and `MIN_SHARED_FT` is what rejects them. In the
  finish-face regime with a generous tolerance, the danger is bridging — see
  the next note.

### Bridging through a thin room

Nothing in the four steps above checks that *no third room lies between* the two
segments. With `wall_max` at 1.5 ft (~450mm), any room narrower than that lets
its two neighbours match straight through it — and duct shafts, risers and
service voids are routinely modelled as rooms in a hospital model. That is a
likelier first-contact failure than bridging a corridor, and it gets worse the
higher the slider goes.

Cheapest correct fix: after a pair clears the distance test, reject it if the
midpoint of the gap falls inside any third room's outer polygon on that level.
If that proves too slow, it is the same problem the spatial index below solves.
Either way it needs a test.

### Duplicate rooms from linked models

`assemble_rooms` dedups **levels** across linked models
([rooms.rs:789](src/service/rooms.rs:789)) but **concatenates rooms**. Two
models covering the same floor therefore contribute two copies of the same room
— coincident outlines, distance 0, full-length overlap, i.e. the
strongest possible edge between what is really one room. Decide and test what
happens here; a near-100%-overlap pair with near-identical polygons is a
detectable signal, not something to leave to the layout engine to make look odd.

### Complexity — measured, and it bit

Naive pairwise over rooms × segments is O(n²·s²). It was left naive on purpose
until measured (STRATEGY.md: optimising before measuring is the wrong end).
**Measured, it bit:** the 5,046-room `big-plate` level took ~22s and the
5,375-room `sample-project` ~8.7s per fetch — unusable for a fetch-on-selection
interaction. The dominant cost was the O(n²) pair loop, made worse by rebuilding
a full-length candidate list per pair.

The fix was the one this section always named: a **uniform bounding-box grid**
(`SpatialGrid` in `adjacency.rs`). Each room registers into the cells its bbox
covers; pairing queries a room's reach-expanded bbox for only the rooms that
could touch it, and the occlusion test queries the single cell at the gap
midpoint instead of scanning every room. **After: `big-plate` ~2.5s,
`sample-project` ~1.8s — and `/rooms` alone (the shared `assemble_rooms` both
share) is ~2.3s and ~1.3s of that, so the adjacency compute itself is now
~0.2–0.5s.** Rayon was deliberately still not reached for: the grid removed the
quadratic term, so threading a near-linear pass would trade determinism for
little. A brute-force-vs-grid equivalence test (`test_grid_matches_brute_force_on_a_dense_layout`)
locks that the index changed only the speed, not the answer.

### Tolerances

Four constants, all in **model units (decimal feet)**, all requiring a doc
comment giving the reasoning:

- `WALL_MAX_FT` — the *default* largest gap still counted as a shared wall, used
  when the request names none. Must clear a thick partition without bridging a
  corridor. **Start ~1.5 ft (~450mm)** and validate against real models; a
  corridor is one failure mode to watch, a sub-tolerance shaft the other. On a
  centreline model the honest setting is near zero, which is why this is a
  default and not a floor.
- `COINCIDENT_EPS_FT` — float-noise allowance added to `wall_max` so a
  centreline model matches at a slider value of zero. Tight, in the spirit of
  `areas.rs`'s `COLLINEAR_EPS_FT`: it absorbs export noise and nothing else.
- `PARALLEL_EPS_RAD` — angular tolerance for "parallel". Needs to absorb float
  noise and slight non-orthogonality without matching genuinely skewed walls.
- `MIN_SHARED_FT` — minimum accumulated overlap for a real relationship.
  Suppresses corner-touch artefacts, where two rooms meet at a point and are
  not meaningfully adjacent. **Start ~1.0 ft.**

`areas.rs` already sets the house style here with `COLLINEAR_EPS_FT` — a named
constant with a comment explaining precisely why the value is tight and what it
does and does not remove. Match that.

### Tunable wall tolerance (`?wall_max=`)

`WALL_MAX_FT` is the value that depends on the model in front of you, so make it
a **query parameter with the constant as its default**, and give the viewer a
slider bound to it. Tuning then costs a drag, not a recompile.

- Endpoint takes `?wall_max=<feet>`, optional. Absent → `WALL_MAX_FT`.
- **Validate the input**: reject non-finite, negative, or absurd values (say,
  anything over 5 ft) rather than silently clamping. **Zero is valid** — it is
  the centreline setting, and rejecting it as "non-positive" would break the
  more common of the two regimes.
- **The status is 400, not 422.** In this codebase 422 is the *ingest* status
  (a schema mismatch, a malformed `taken_at` — [handlers.rs:46](src/handlers.rs:46)),
  while a caller-fault *read* parameter travels as `ServiceError::Invalid`,
  which `map_service_error` maps to 400
  ([handlers.rs:286](src/handlers.rs:286)). `/rooms` already answers a
  malformed `?filter=` that way. The discipline the earlier draft meant —
  loud over a silent no-op, message in the body, never a clamp — is fully
  preserved by `Invalid`; only the number changes. Do not invent a bespoke
  status in the handler, and do not teach `service/` about HTTP codes: the
  transport-agnostic seam is the point of `ServiceError`.
  Note also that a non-numeric `?wall_max=abc` is rejected by axum's own
  `Query` deserialization *before* the handler runs, with its own 400 and its
  own wording. Either accept that split, or take the field as `Option<String>`
  and parse it yourself to own the message. Decide deliberately; do not
  discover it in review.
- Thread it as a **parameter through the service function**, not a global or a
  thread-local. `compute_adjacency(rooms, wall_max)` — the constant becomes the
  caller's default, and the algorithm has no opinion about where the number
  came from. This also keeps the inline tests able to exercise tolerance
  behaviour directly by passing values in, which is how both boundary regimes
  get covered.
- **It must participate in the `revision`.** Two responses at different
  tolerances are different content; if the revision ignores `wall_max`, the
  viewer will skip a re-render after a slider change and appear frozen. See the
  response shape below for how to build it.

`PARALLEL_EPS_RAD`, `COINCIDENT_EPS_FT` and `MIN_SHARED_FT` stay constants for
now — none is uncertain in the way the wall gap is, and exposing all four
invites fiddling with values that have a right answer.

**Viewer side:** a slider with a live numeric readout, in whatever units the
user actually thinks in (mm is probably the honest choice for a UK/EU hospital
job even though the wire value is feet — convert at the edge, and label it).
**Range 0–900mm**, starting at zero rather than 100mm precisely because zero is
the centreline setting, defaulting to the constant. **Debounce it** — a
continuous drag firing a graph recompute per frame will hammer the server;
~150ms after the drag settles is enough.

The slider is a **tuning instrument, not a permanent feature**. Once real models
establish which regime is normal and what value works, the honest move is to
bake that default in and decide whether the control still earns its place in the
UI. Note that intent in the code so a future reader does not mistake it for a
settled preference knob.

---

## Response shape

Node/edge, so the client does no geometry:

```json
{
  "schema_version": 5,
  "revision": "…",
  "levels": [{ "id": "311", "name": "Level 0" }],
  "nodes": [
    {
      "room_id": "324772",
      "name": "Room 1",
      "level_id": "311",
      "centroid": { "x": 12.5, "y": 8.0 },
      "classification": [ … ],
      "drofus": { … }
    }
  ],
  "edges": [
    { "a": "324772", "b": "324773", "level_id": "311", "shared_length": 14.2 }
  ]
}
```

Notes:

- `schema_version` is the `SUPPORTED_SCHEMA` constant, never a literal — see
  [areas.rs:352](src/service/areas.rs:352). The `5` above is illustrative.
- **`revision` — derive it, don't recompute it.** The earlier draft said "the
  same way `RoomsResult` does it (`scoped_revision`)"; `scoped_revision` is
  private to `rooms.rs` ([rooms.rs:601](src/service/rooms.rs:601)) and does
  not need to become public. `assemble_rooms` already hands back
  `RoomsResult.revision` ([rooms.rs:561](src/service/rooms.rs:561)) — hash
  that string together with the effective `wall_max` and use the result. That
  gives both properties for free: stable while the model is idle, and moving
  when the slider moves. The viewer compares that one field instead of
  re-stringifying the payload, which is what keeps a quiet system from
  re-rendering — see `Superseded/HANDOVER-viewer-performance.md`.
- Carry `classification` and `drofus` **through** onto nodes, resolved at
  response assembly exactly as `/rooms` does — never stored. The graph wants to
  colour by department, and the client should not have to cross-reference a
  second fetch to do it.
- Edges are **undirected**; emit each pair once with a stable ordering (`a < b`
  by room id) so the payload is deterministic and the revision is meaningful.
- Return `Option<…>` → `None` → **204**, not 404, when nothing has ever been
  posted for the project. That is the "signal, not error" / soft-empty rule, and
  it is what `assemble_areas` already does
  ([areas.rs:329](src/service/areas.rs:329)).
- Scope with `?building=` / `?milestone=` query params, matching `/rooms` and
  `/areas`. `?filter=` is deliberately **not** offered — `/areas` does not carry
  it either, and a filtered room set would silently drop neighbours, making the
  graph lie about what a room touches.
- An unplaced room (fewer than three points in its outer loop) is a node with no
  edges, not an error — a diagnostic signal.

---

## The viewer

The reference is a force-directed radial graph: a focal node anchored centre,
neighbours fanning outward, ring position encoding hop distance. It renders on
a **`<canvas>`**, inside `index.html`, from `static/graph.js`.

**Where in the page: band 1 of the bottom region**, as a third `result-band`
alongside QA and the hierarchy-area figures. That is the established home for a
page-level result derived from the scope, it already divides its height budget
between expanded blocks, and it collapses to a one-line strip so the graph costs
nothing until asked for. The right-hand column is *not* an option: it is reserved
for the room inspector (HANDOVER-ui-layout Decision 3).

**The layout simulates angle only.** Each node is pinned to the radius of its hop
count and only its angle is solved, which makes the layout a set of independent
1-D problems — one per ring — rather than a general 2-D force graph. Two reasons,
both load-bearing: ring position *is* the message (ring N means "N walls from the
room you selected") and free 2-D would let it drift; and a 1-D problem per ring
is stable without a cooling schedule, so it settles in well under a second and
cannot fling a node off-screen.

### Why canvas here, when the plan stays SVG

[STRATEGY-BROWSER.md](STRATEGY-BROWSER.md) names two triggers for leaving SVG:
element count on screen, and a need for continuous animation. Be clear about
which one applies, because only one does.

- **Element count is not the trigger.** A depth-2 graph is on the order of tens
  of nodes. SVG degrades in the low tens of thousands. It is three orders of
  magnitude away.
- **Continuous animation is.** A force simulation ticks every frame while it
  settles, and dragging a node is continuous feedback. SVG is retained-mode and
  has no render loop; this is the first thing in the project that actually
  fights that model rather than merely drawing on top of it.

So the plan view stays SVG — hit-testing, hover, CSS styling and the whole cull
unit machinery are load-bearing there — and the graph is canvas. **Two
renderers on one page is a deliberate outcome, not an accident**, and the file
split (`graph.js` vs the inline plan renderer) keeps the boundary visible.

### What canvas costs, and how each cost is paid

Nothing here is hard at this scale, but none of it is free the way it is in SVG:

- **Hit-testing** — no DOM nodes, so clicks and hover need doing by hand. At
  tens of nodes this is a nearest-node-within-radius scan over the node array,
  not point-in-polygon. Trivial; just remember it exists.
- **Colour** — no CSS cascade. Read the resolved `:root` custom properties from
  `tokens.css` once via `getComputedStyle` and cache them, exactly as
  `exportStyleBlock` already does for the SVG export
  ([index.html:1607](static/index.html:1607)), so the graph can never
  diverge from the page palette. Re-read on nothing; the tokens are static.
- **HiDPI** — size the backing store by `devicePixelRatio` and scale the
  context, or every label is soft on a retina display.
- **No accessibility, no text selection.** Accepted for a graph canvas.
- **No SVG export.** The "Export SVGs" button is a plan-view feature and does
  not extend here; canvas gives raster only, and raster export is explicitly
  out of scope in STRATEGY-BROWSER. Do not quietly add a PNG button.

### The graph itself

- **Centre** — the selected room (page selection, from the prerequisite above).
- **Ring 1** — directly adjacent. **Ring 2** — two hops. Cap the default depth
  (2 is a sensible start) with a control to go deeper; the full graph on a
  hospital level is unreadable and should never be the default view.
- **Edge weight** → line thickness or spring length. A 4m shared wall is a
  stronger relationship than a 200mm corner touch, and the layout should say so.
- **Node colour** → classification tier or department, via the `qualitative`
  helper moved to `common.js` in the prerequisite step, so the graph and the
  plan overlay agree on what colour a department is.
- **Selection is bidirectional** with the plan view — click a room in the SVG,
  the graph re-centres; click a node, the plan highlights. Both directions read
  and write the one page selection; neither owns it.
- **Stop the simulation when it settles.** A force sim left running is a
  permanent 60fps repaint of a static picture, on the same page as a 5,000-room
  plan. Freeze on low kinetic energy, restart on interaction.

Fetch on **selection change**, not on the 2s poll. Re-fetch when the room
`revision` changes, matching how `loadAreas` handles the areas overlay
on-demand ([index.html:1717](static/index.html:1717)).

---

## Designed-in extension points

Neither of these should be built now. Both should be *cheap* to add later,
which constrains the types today:

- **Door connectivity.** Would need door elements added to the Revit extractor
  and the contract (a schema bump). Shape the edge type so a `kind` or
  `connection` discriminator can be added without breaking the existing wire
  format — an edge is already a pair plus metadata, so adding a field is
  additive.
- **Cross-level adjacency.** `level_id` sits on the edge, not just the node,
  precisely so a vertical edge (differing level ids) is representable without a
  type change. Keep it there even though every edge is same-level today.

---

## Definition of done

**Prerequisites**

- [x] Room click-selection as page state, per HANDOVER-ui-layout Decision 3 —
      `selectRoom` / `applySelection` / `roomAtNode` in `index.html`, with a
      4px click-vs-pan slop so a pan never selects. **The inspector unblocks
      with it**; `selectionListeners` is where it registers.
- [x] `qualitative` and its palette constants moved from `index.html` to
      `common.js`, plan view rendering identically.

**Service**

- [x] `service/adjacency.rs` with inline tests: shared wall detected in the
      **finish-face** regime (offset edges) **and** the **centreline** regime
      (coincident edges, `wall_max = 0`, including float noise); corner-touch
      rejected; corridor **not** bridged; **a room narrower than `wall_max` not
      bridged through**; L-shaped junction sums its disjoint runs; a wall split
      into several boundary segments counted once, not per segment pair;
      unplaced room yields an isolated node; two coincident rooms from linked
      models suppressed.
- [x] Timing measured on a realistic level, **and** the spatial index this
      section prescribed added once it bit: `big-plate` 22s → 2.5s,
      `sample-project` 8.7s → 1.8s, adjacency compute now ~0.2–0.5s over the
      shared `/rooms` cost. Brute-force-vs-grid equivalence test locks
      correctness.

**Endpoint**

- [x] `GET /projects/{id}/adjacency` with `?building=` / `?milestone=` /
      `?wall_max=`, 204 on empty, `revision` present and stable across idle
      requests **and changing when `wall_max` changes**.
- [x] `wall_max = 0` is accepted and returns the centreline result; negative,
      non-finite, unparseable and out-of-range values return **400** with a
      message, not a clamped result.
- [x] MCP tool `get_adjacency` over the same service function, per
      `src/bin/mcp.rs`'s one-tool-per-read-route rule.

**Viewer**

- [x] Radial canvas graph in `static/graph.js` with depth control, edge
      weighting, department colouring, and a simulation that stops when settled
      (verified: zero animation frames and byte-identical pixels once at rest).
- [x] Debounced wall-tolerance slider, 0–900mm, with a mm readout.
- [x] Bidirectional selection between plan and graph over the one page
      selection.

**Validation against reality**

- [x] End-to-end browser run against a live server. Verified against the
      `showcase` and `sample-project` fixtures: the band fetches the real
      endpoint on open; clicking a plan room centres the graph and highlights the
      room (plan→graph); clicking a graph node re-centres and highlights a
      different plan room (graph→plan), with `selectedZoneId` correctly null on a
      graph-originated click; the debounced slider refetches with a new
      `wall_max` and a new revision; the graph paints with real classification
      tiers and the shared Set2 palette, and the simulation stops when settled.
- [ ] **Still open — needs a genuine Revit export.** Every fixture in the repo
      is generated **centreline** (rooms tile edge-to-edge, gap 0), so the
      centreline regime is exercised on real-ish data but the **finish-face**
      regime is only covered by unit tests. The remaining validation is one real
      model: record **which boundary regime it uses**, confirm no
      corridor-bridging and no bridging through a thin service room at a realistic
      tolerance, and record the value that worked so the default can be baked in
      and the slider reconsidered. (Observed so far: on the centreline fixtures a
      `wall_max` sweep 0 → 5 ft moves the edge count only slightly — 10,497 →
      10,518 on `sample-project` — as expected when neighbours already touch.)

---

## Docs to update on landing

Per [STRATEGY.md](STRATEGY.md), a change touching more than one layer updates
every doc it touches. This one touches the service layer, the HTTP and MCP
adapters, and the browser:

- **[STRATEGY-BROWSER.md](STRATEGY-BROWSER.md)** — add the graph mode to
  "Implemented"; correct the `/adjacencies` reference to the shipped
  `/projects/{id}/adjacency`; record canvas as the first renderer to leave SVG
  and *why* (the animation trigger, not element count).
- **[STRATEGY-SERVER.md](STRATEGY-SERVER.md)** — the new endpoint alongside
  `/areas`.
- **[STRATEGY-MCP.md](STRATEGY-MCP.md)** — the new tool.
- **[docs/README.md](README.md)** — move this handover from "Open handovers" to
  `Superseded/`, and update the ui-layout row once click-selection lands.
