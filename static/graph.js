// Radial adjacency graph, drawn on a <canvas>.
//
// Loaded as a classic <script> after common.js (whose `qualitative` palette this
// shares with the plan renderer, so the two agree on what colour a department
// is, and whose `pathKey`/`tierLabel` it shares with the areas overlay, so the
// two agree on what a group *is*). Zero build step, same ServeDir as every other
// page asset.
//
// ---------------------------------------------------------------------------
// What a node is: a room, or a hierarchy area
// ---------------------------------------------------------------------------
// The `/adjacency` payload is always a ROOM graph — that is the only granularity
// the geometry supports, since a shared wall is a fact about two room boundaries.
// A node here is either one of those rooms, or an **area group**: every room
// sharing a classification-path prefix at one tier, collapsed to a single node,
// with the room edges between two groups summed into one weighted edge.
//
// The aggregation is done here rather than server-side because it is a pure
// relabelling of a payload the client already holds: the nodes carry their
// classification path, the edges carry their shared length, and grouping by
// `pathKey` is exactly what `/areas` does to produce a group. A second endpoint
// would re-derive the same geometry to answer a question this file can answer by
// summing. It also means switching granularity is a re-layout, not a refetch.
//
// Group identity is `level|pathKey(path, tier)` — byte-identical to
// `areaKey` in index.html — so a footprint clicked on the plan focuses the
// matching node, and a node clicked here selects the matching footprint. That
// shared key is the whole reason the vocabulary sits in common.js.
//
// ---------------------------------------------------------------------------
// Why canvas here when the floor plan stays SVG
// ---------------------------------------------------------------------------
// STRATEGY-BROWSER names two triggers for leaving SVG: element count on screen,
// and a need for continuous animation. Only the second one applies, and it is
// worth being precise about that rather than claiming both:
//
//   - Element count is NOT the trigger. A depth-2 graph is tens of nodes; SVG
//     degrades in the low tens of thousands. It is three orders of magnitude
//     away, and an SVG version of this would render perfectly well.
//   - Continuous animation IS. The layout settles over a run of frames, and SVG
//     is retained-mode with no render loop — this is the first thing in the
//     project that fights that model rather than merely drawing on top of it.
//
// So the plan keeps SVG (where hit-testing, hover, CSS styling and the cull-unit
// machinery are all load-bearing) and the graph takes canvas. Two renderers on
// one page is a deliberate outcome, not an accident, which is why this file
// exists separately instead of growing index.html by another few hundred lines.
//
// What canvas costs, and how each cost is paid here:
//   - no DOM, so hit-testing is by hand — a nearest-node scan (`pick`), which is
//     free at tens of nodes and needs no point-in-polygon;
//   - no CSS cascade, so colours are read once from the resolved :root custom
//     properties (`readPalette`), exactly as the SVG exporter already does, so
//     the graph can never drift from tokens.css;
//   - no automatic HiDPI, so the backing store is sized by devicePixelRatio;
//   - no accessibility and no text selection. Accepted for a graph canvas.
//   - no SVG export. "Export SVGs" is a plan-view feature and does not extend
//     here; raster export is explicitly out of scope in STRATEGY-BROWSER, so
//     there is deliberately no download button on this panel.
//
// ---------------------------------------------------------------------------
// Why the layout simulates ANGLE ONLY
// ---------------------------------------------------------------------------
// A general 2-D force-directed layout would let ring position drift, and ring
// position is the whole message: ring N means "N walls away from the room you
// selected". So each node is pinned to the radius of its hop count and only its
// ANGLE is simulated. That makes the layout a set of independent 1-D problems
// (one per ring), which is both far more stable than free 2-D — no tangling, no
// nodes flung off-screen, no need for a cooling schedule to look sane — and
// cheap enough that settling takes a fraction of a second.
//
// ---------------------------------------------------------------------------
// Why each ring uses the WHOLE circle
// ---------------------------------------------------------------------------
// The first version of this pulled every node toward the circular mean of its
// neighbours one ring in — including ring 1, whose only neighbour is the focus.
// The focus sits at the centre, where an angle is meaningless (it is stored as
// 0), so every ring-1 node was sprung toward angle 0 and the entire graph
// collapsed into a fan on the right-hand side, with only a capped repulsion
// holding the dots apart. The rings were four times longer than the picture
// drawn on them.
//
// Two changes fix it, and they are the reason the force model reads the way it
// does now:
//
//   - **Ring 1 is not attracted to anything.** There is nothing at the centre to
//     sit beside. Its target spacing is `2π/n`, i.e. even over the full circle,
//     which is also the seeded state — so ring 1 starts correct and stays there.
//   - **Deeper rings keep the parent spring**, because "sits next to its
//     neighbour" is real information there, and their spread force degrades to a
//     collision guard (`2π/n` capped at what a label needs). They still fill the
//     circle, but they get it from their parents being spread rather than from a
//     force of their own — so a room's children stay visibly its children.
//
// Nodes are also SEEDED in plan order (their real bearing from the focus), so a
// ring's cyclic order matches the way the rooms sit around it on the floor plan.
// The angles are even, not the bearings themselves: two rooms in the same
// direction would seed on top of each other, and two nodes at an identical angle
// cannot be separated by the spread force (it pushes both the same way).

const TAU = Math.PI * 2;
const GRAPH_NODE_R = 7;          // node dot radius, CSS px
const GRAPH_FOCUS_R = 10;        // the selected room reads larger
const GRAPH_LABEL_FONT = 11;     // CSS px
// Simulation: angular spring toward the neighbours in the ring below, an
// even-spacing force between same-ring nodes, and enough damping that the whole
// thing is visibly still within a second.
const GRAPH_ATTRACT = 0.06;
const GRAPH_SPREAD = 0.35;
const GRAPH_DAMPING = 0.82;
// Below this peak angular velocity (radians/frame) the picture is not visibly
// moving, so the loop stops. A force sim left running is a permanent 60fps
// repaint of a static image — on the same page as a 5,000-room plan, that is not
// a cost worth paying for nothing.
const GRAPH_SETTLE_EPS = 0.0006;
// Above this many nodes on screen, only the focus, ring 1 and whatever is
// hovered are labelled: past it the text is the noise rather than the data.
const GRAPH_LABEL_BUDGET = 24;

// Read the resolved palette from :root once per construction. Same approach as
// the SVG exporter's `exportStyleBlock`: canvas has no cascade, so the colours
// have to be pulled out of CSS rather than referenced from it, and pulling them
// live is what stops this file from quietly forking tokens.css.
function readPalette() {
  const cs = getComputedStyle(document.documentElement);
  const v = (name, fallback) => (cs.getPropertyValue(name) || "").trim() || fallback;
  return {
    ink: v("--ink", "#1a1a1a"),
    paper: v("--paper", "#faf8f3"),
    accent: v("--accent", "#b4532a"),
    rule: v("--rule", "#d8d2c4"),
    fill: v("--fill", "#efe9dc"),
  };
}

// Shortest angular difference b - a, wrapped to (-π, π]. Everything angular in
// here goes through this: without it a node at 359° and one at 1° look 358°
// apart and shove each other the wrong way round the ring.
function angleDelta(a, b) {
  let d = (b - a) % TAU;
  if (d > Math.PI) d -= TAU;
  if (d <= -Math.PI) d += TAU;
  return d;
}

// The same difference measured one way round only: the counter-clockwise gap
// from a to b, in [0, 2π). This is what "the space between me and the next node"
// means, and it is not `|angleDelta|` — two nodes 350° apart the short way are
// 10° apart, but the gap on one side of each of them is still 350°.
function gapCCW(a, b) {
  return ((b - a) % TAU + TAU) % TAU;
}

// Keep an accumulating angle inside (-π, π]. The spread force sorts each ring by
// angle to find every node's two neighbours, and a ring whose angles have drifted
// past 2π sorts into the wrong order.
function normAngle(a) {
  return angleDelta(0, a);
}

function createRoomGraph(canvas, { onSelect } = {}) {
  const ctx = canvas.getContext("2d");
  let palette = readPalette();

  let payload = null;        // the /adjacency response — always a ROOM graph
  // The aggregation tier: null for room granularity, else the classification
  // depth whose groups are the nodes. Set from the focus, because the focus is
  // what states the granularity — selecting a footprint on the plan is a
  // question about that tier.
  let groupDepth = null;
  let nodesById = new Map();  // view node id -> node
  let neighbours = new Map(); // view node id -> [{ id, weight }]
  let viewEdges = [];         // view edges: { a, b, weight }
  let focusId = null;
  let depth = 2;              // Infinity is legal: "as far as the graph goes"
  let tierDepth = 0;          // which classification tier drives node colour
  let colourKeys = [];        // stable, sorted key list -> palette index

  let placed = [];            // the laid-out nodes: { node, hop, angle, vel, x, y, r }
  let drawEdges = [];         // view edges with both ends placed, resolved once per layout
  let drawMaxWeight = 1;      // heaviest of those, the thickness scale's top end
  let maxHop = 0;             // deepest ring actually present, which is what scales the rings
  let hoverId = null;
  let raf = 0;
  let cssW = 0, cssH = 0;

  // ---- data -------------------------------------------------------------

  function setData(next) {
    payload = next;
    rebuildColourKeys();
    rebuildView();
    layout();
  }

  // Rebuild `nodesById` / `viewEdges` / `neighbours` from the payload at the
  // current granularity. Everything downstream (BFS, layout, drawing, hit
  // testing) reads the view and never the payload, so room and area granularity
  // are one code path with two builders.
  function rebuildView() {
    nodesById = new Map();
    viewEdges = [];
    if (payload) {
      if (groupDepth == null) buildRoomView();
      else buildAreaView();
    }
    neighbours = new Map([...nodesById.keys()].map(id => [id, []]));
    for (const e of viewEdges) {
      // An edge naming a node that is not in the view cannot happen from either
      // builder, but a defensive skip beats a thrown TypeError inside a rAF.
      if (!neighbours.has(e.a) || !neighbours.has(e.b)) continue;
      neighbours.get(e.a).push({ id: e.b, weight: e.weight });
      neighbours.get(e.b).push({ id: e.a, weight: e.weight });
    }
  }

  // Room granularity: the payload verbatim, renamed onto the view's field names.
  function buildRoomView() {
    for (const n of payload.nodes) {
      nodesById.set(n.room_id, {
        id: n.room_id,
        name: n.name,
        levelId: n.level_id,
        classification: n.classification || [],
        centroid: n.centroid,
        rooms: 1,
      });
    }
    viewEdges = payload.edges.map(e => ({ a: e.a, b: e.b, weight: e.shared_length }));
  }

  // Area granularity: one node per (level, path prefix at `groupDepth`), and one
  // edge per pair of groups carrying the SUM of the room edges between them.
  // Summing is the honest aggregate — two departments are adjacent by however
  // much wall they actually share, and the thickness scale then reads the same
  // way it does for rooms.
  function buildAreaView() {
    const groupOf = new Map(); // room id -> group id
    for (const n of payload.nodes) {
      const path = n.classification || [];
      // A room classified shallower than this tier belongs to no group here —
      // the same rooms `/areas` reports at no group. Dropping it beats inventing
      // a group for it, and it takes its edges with it rather than wiring them
      // to something arbitrary.
      if (path.length <= groupDepth) continue;
      const id = `${n.level_id}|${pathKey(path, groupDepth)}`;
      groupOf.set(n.room_id, id);
      const existing = nodesById.get(id);
      if (!existing) {
        nodesById.set(id, {
          id,
          // The group's own tier value. Two groups under different parents can
          // read alike; the label is the leaf because that is what the areas
          // overlay and band both show, and the full path would not fit.
          name: tierLabel(path[groupDepth]),
          levelId: n.level_id,
          classification: path.slice(0, groupDepth + 1),
          // Mean of the member rooms' centroids. Only ever used to seed a
          // bearing, so an unweighted mean is enough — this is not an area
          // centroid and must not be read as one.
          centroid: { x: n.centroid.x, y: n.centroid.y },
          rooms: 1,
        });
      } else {
        existing.centroid.x += n.centroid.x;
        existing.centroid.y += n.centroid.y;
        existing.rooms += 1;
      }
    }
    for (const g of nodesById.values()) {
      g.centroid.x /= g.rooms;
      g.centroid.y /= g.rooms;
    }

    // Nested by the ordered pair rather than keyed on a joined string: a group id
    // already contains a level id and a "/"-joined path, so there is no separator
    // left that two ids cannot fake between them, and a collision here would
    // silently merge two different pairs of departments into one edge.
    const summed = new Map(); // group a -> group b -> summed shared length
    for (const e of payload.edges) {
      const ga = groupOf.get(e.a), gb = groupOf.get(e.b);
      // A wall inside one group is not a relationship BETWEEN groups: it is what
      // the group's own footprint already dissolved.
      if (!ga || !gb || ga === gb) continue;
      const [a, b] = ga < gb ? [ga, gb] : [gb, ga];
      if (!summed.has(a)) summed.set(a, new Map());
      const row = summed.get(a);
      row.set(b, (row.get(b) || 0) + e.shared_length);
    }
    for (const [a, row] of summed)
      for (const [b, weight] of row) viewEdges.push({ a, b, weight });
    // Deterministic order, like the server's: the drawing order of overlapping
    // edges should not depend on Map iteration.
    viewEdges.sort((x, y) => (x.a === y.a ? (x.b < y.b ? -1 : 1) : x.a < y.a ? -1 : 1));
  }

  // The colour key for a node at the active tier: its resolved code, else its
  // name, else null for an undefined/absent tier (drawn in the neutral fill,
  // never an error colour — same stance as the plan's "no data" grey). An area
  // node aggregated ABOVE the colour tier has no value there, so it falls to the
  // neutral fill rather than borrowing one member room's colour.
  function colourKey(node) {
    const tv = (node.classification || [])[tierDepth];
    if (!tv || tv.undefined) return null;
    return tv.code != null ? tv.code : (tv.name != null ? tv.name : null);
  }

  // Built from the PAYLOAD's rooms, not the view's nodes, and sorted: the same
  // department then gets the same palette index on every fetch, at every depth,
  // and in both granularities. A colour that changed as rooms came and went — or
  // as the user switched from rooms to areas — would be worse than useless.
  function rebuildColourKeys() {
    const keys = new Set();
    for (const n of (payload ? payload.nodes : [])) {
      const tv = (n.classification || [])[tierDepth];
      if (!tv || tv.undefined) continue;
      const k = tv.code != null ? tv.code : (tv.name != null ? tv.name : null);
      if (k != null) keys.add(k);
    }
    colourKeys = [...keys].sort();
  }

  function colourFor(node) {
    const k = colourKey(node);
    if (k == null) return palette.fill;
    return qualitative("Set2", colourKeys.indexOf(k));
  }

  // The tier labels available for the colour picker, taken from the ROOMS' own
  // classification paths (depth i carries that tier's label), so the picker
  // offers every tier even when the view is aggregated to a shallow one.
  function tierNames() {
    const names = [];
    for (const n of (payload ? payload.nodes : []))
      (n.classification || []).forEach((t, i) => { if (names[i] === undefined) names[i] = t.tier; });
    return names;
  }

  // ---- layout -----------------------------------------------------------

  // Breadth-first from the focus, capped at `depth` (which may be Infinity, for
  // every ring the graph has). The cap is not a performance guard — it is the
  // readability default. The full graph of a hospital level is an unreadable
  // hairball and must never be what opens, but asking for it is a legitimate
  // question and the control answers it.
  function reachable() {
    if (!focusId || !nodesById.has(focusId)) return [];
    const hops = new Map([[focusId, 0]]);
    let frontier = [focusId];
    for (let h = 1; h <= depth; h++) {
      const next = [];
      for (const id of frontier) {
        for (const nb of neighbours.get(id) || []) {
          if (hops.has(nb.id)) continue;
          hops.set(nb.id, h);
          next.push(nb.id);
        }
      }
      frontier = next;
      // Also what terminates an unbounded `depth`: the graph is finite, so the
      // frontier always empties.
      if (!frontier.length) break;
    }
    return [...hops].map(([id, hop]) => ({ node: nodesById.get(id), hop }));
  }

  function layout() {
    const found = reachable();
    const byHop = new Map();
    for (const f of found) {
      if (!byHop.has(f.hop)) byHop.set(f.hop, []);
      byHop.get(f.hop).push(f);
    }

    const focus = nodesById.get(focusId);
    // Bearing from the focus on the actual floor plan. Precomputed rather than
    // called from the comparator, so the sort cost stays n log n and a node's key
    // cannot change mid-sort.
    const bearing = new Map();
    for (const f of found) {
      const c = f.node.centroid, o = focus && focus.centroid;
      bearing.set(f.node.id, c && o ? Math.atan2(c.y - o.y, c.x - o.x) : 0);
    }

    placed = [];
    maxHop = 0;
    for (const [hop, group] of byHop) {
      maxHop = Math.max(maxHop, hop);
      // Plan order, with the id as a tie-break so a ring of co-located rooms (or
      // one with no geometry at all) still opens the same way twice.
      group.sort((a, b) =>
        bearing.get(a.node.id) - bearing.get(b.node.id) || (a.node.id < b.node.id ? -1 : 1));
      group.forEach((f, i) => {
        placed.push({
          node: f.node,
          hop,
          // Even angles, in that plan order. Ring 1 is already at its
          // equilibrium here; deeper rings relax toward their parents from it.
          angle: hop === 0 ? 0 : normAngle((i / group.length) * TAU),
          vel: 0,
          x: 0, y: 0,
          r: hop === 0 ? GRAPH_FOCUS_R : GRAPH_NODE_R,
        });
      });
    }

    // Resolve the drawable edges once per layout instead of filtering every view
    // edge on every frame: at depth 1 on a big level that is a handful of edges
    // out of thousands, and the frame loop should not keep rediscovering it.
    const index = new Map(placed.map(p => [p.node.id, p]));
    drawEdges = [];
    drawMaxWeight = 1;
    for (const e of viewEdges) {
      const a = index.get(e.a), b = index.get(e.b);
      if (!a || !b) continue;
      drawEdges.push({ a, b, weight: e.weight });
      drawMaxWeight = Math.max(drawMaxWeight, e.weight);
    }

    start();
  }

  // Minimum angular separation that keeps two dots (and their labels) from
  // colliding at this radius. An inner ring therefore spreads its nodes further
  // apart in angle than an outer one, which is correct.
  function labelSep(ringR) {
    return (GRAPH_NODE_R * 5) / Math.max(ringR, 1);
  }

  // One simulation step. Returns the peak angular velocity so the caller can
  // decide whether the picture is still moving.
  function step() {
    const byHop = new Map();
    for (const p of placed) {
      if (!byHop.has(p.hop)) byHop.set(p.hop, []);
      byHop.get(p.hop).push(p);
    }
    const index = new Map(placed.map(p => [p.node.id, p]));
    let peak = 0;

    for (const [hop, ring] of byHop) {
      if (hop === 0) continue; // the focus is anchored at the centre by definition
      const n = ring.length;
      const even = Math.min(TAU / n, Math.PI);
      // Ring 1 spreads over the whole circle (`even`); deeper rings only need
      // enough room not to collide, so their spacing target is whichever of the
      // two is smaller — a ring of 40 nodes cannot give each one a label's worth
      // of angle, and demanding it would just fight the parent spring forever.
      const ideal = hop === 1 ? even : Math.min(even, labelSep(ringRadius(hop)));

      // Sorted by angle, so every node's two angular neighbours are its
      // neighbours in this array. That makes the spread force O(n log n) rather
      // than the all-pairs O(n²) it used to be — which is what lets the depth
      // control offer "All" on a level with thousands of rooms.
      const sorted = ring.slice().sort((a, b) => a.angle - b.angle);

      for (let i = 0; i < n; i++) {
        const p = sorted[i];
        let force = 0;

        // Spring toward the circular mean of this node's neighbours one hop in,
        // weighted by shared wall length: a room joined by a 4m wall is pulled
        // to sit beside its neighbour harder than one joined by a 200mm reveal.
        // Skipped for ring 1: its only inward neighbour is the focus, which sits
        // at the centre and has no angle to be pulled toward — attracting to its
        // stored 0 is exactly what used to fold the graph into one quarter.
        if (hop > 1) {
          let sx = 0, sy = 0;
          for (const nb of neighbours.get(p.node.id) || []) {
            const other = index.get(nb.id);
            if (!other || other.hop !== hop - 1) continue;
            const w = Math.max(nb.weight, 0.1);
            sx += Math.cos(other.angle) * w;
            sy += Math.sin(other.angle) * w;
          }
          if (sx !== 0 || sy !== 0) force += GRAPH_ATTRACT * angleDelta(p.angle, Math.atan2(sy, sx));
        }

        // Even-spacing force: pushed away from whichever angular neighbour is
        // closer than `ideal`, in the direction of the roomier side. At rest
        // every gap in the ring is `ideal`, so a ring left to itself ends up
        // evenly distributed rather than piled up under one parent.
        if (n > 1) {
          const prev = sorted[(i - 1 + n) % n], next = sorted[(i + 1) % n];
          const gapIn = gapCCW(prev.angle, p.angle);
          const gapOut = gapCCW(p.angle, next.angle);
          force += GRAPH_SPREAD * (Math.max(0, ideal - gapIn) - Math.max(0, ideal - gapOut));
        }

        p.vel = (p.vel + force) * GRAPH_DAMPING;
        peak = Math.max(peak, Math.abs(p.vel));
      }
    }

    for (const p of placed) p.angle = normAngle(p.angle + p.vel);
    return peak;
  }

  // Rings are scaled to the deepest ring PRESENT, not to the depth requested: a
  // room whose graph runs out after two hops should not be drawn inside the
  // innermost third of the panel because the control happens to say 6.
  function ringRadius(hop) {
    const usable = Math.min(cssW, cssH) / 2 - 28; // margin for labels
    return (Math.max(usable, 40) / Math.max(maxHop, 1)) * hop;
  }

  function positions() {
    const cx = cssW / 2, cy = cssH / 2;
    for (const p of placed) {
      const r = ringRadius(p.hop);
      p.x = cx + Math.cos(p.angle) * r;
      p.y = cy + Math.sin(p.angle) * r;
    }
  }

  // ---- drawing ----------------------------------------------------------

  function draw() {
    positions();
    ctx.clearRect(0, 0, cssW, cssH);

    if (!placed.length) {
      ctx.fillStyle = palette.ink;
      ctx.globalAlpha = 0.45;
      ctx.font = `${GRAPH_LABEL_FONT + 1}px ui-monospace, monospace`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(
        focusId
          ? `selected ${groupDepth == null ? "room" : "area"} is not in this scope`
          : "click a room or an area on the plan",
        cssW / 2, cssH / 2,
      );
      ctx.globalAlpha = 1;
      return;
    }

    // Faint ring guides, so "one ring out" is readable as a distance rather
    // than inferred from the node positions.
    ctx.strokeStyle = palette.rule;
    ctx.lineWidth = 1;
    for (let h = 1; h <= maxHop; h++) {
      ctx.beginPath();
      ctx.arc(cssW / 2, cssH / 2, ringRadius(h), 0, TAU);
      ctx.stroke();
    }

    // Edges first, so nodes sit on top. Only edges between two laid-out nodes
    // are drawn — an edge to a room beyond the depth cap is not half-drawn.
    // Weights are scaled against the heaviest edge ON SCREEN (`drawMaxWeight`,
    // resolved with the edge set rather than per frame), so the thickness scale
    // means something at every depth and in both granularities — an aggregated
    // area edge is the sum of its rooms' and would otherwise flatten every
    // room-level edge against it.
    for (const e of drawEdges) {
      // Thickness encodes shared wall length: a 4m wall is a stronger
      // relationship than a 200mm corner touch and should look like one.
      const t = e.weight / drawMaxWeight;
      ctx.strokeStyle = palette.ink;
      ctx.globalAlpha = 0.18 + 0.42 * t;
      ctx.lineWidth = 1 + 3.5 * t;
      ctx.beginPath();
      ctx.moveTo(e.a.x, e.a.y);
      ctx.lineTo(e.b.x, e.b.y);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    ctx.font = `${GRAPH_LABEL_FONT}px ui-monospace, monospace`;
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    for (const p of placed) {
      const isFocus = p.hop === 0;
      const isHover = p.node.id === hoverId;
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.r + (isHover ? 2 : 0), 0, TAU);
      ctx.fillStyle = colourFor(p.node);
      ctx.fill();
      ctx.lineWidth = isFocus ? 3 : 1.5;
      ctx.strokeStyle = isFocus || isHover ? palette.accent : palette.ink;
      ctx.stroke();

      // Label the focus and ring 1 always; label further rings only when the
      // graph is sparse enough for the text not to become the noise.
      if (isFocus || p.hop === 1 || placed.length <= GRAPH_LABEL_BUDGET || isHover) {
        // An area node says how many rooms it stands for — without it a group of
        // one and a group of ninety are the same dot.
        const text = p.node.rooms > 1 ? `${p.node.name || p.node.id} (${p.node.rooms})` : (p.node.name || p.node.id);
        ctx.fillStyle = palette.ink;
        ctx.globalAlpha = isFocus || isHover ? 1 : 0.75;
        ctx.fillText(text.length > 22 ? `${text.slice(0, 21)}…` : text, p.x, p.y + p.r + 3);
        ctx.globalAlpha = 1;
      }
    }
  }

  // ---- loop -------------------------------------------------------------

  function start() {
    if (raf) return;
    const frame = () => {
      const peak = step();
      draw();
      if (peak < GRAPH_SETTLE_EPS) {
        raf = 0;      // settled: stop repainting a static picture
        return;
      }
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
  }

  // ---- interaction ------------------------------------------------------

  // Nearest node within its own radius (plus a little slack for fingers and
  // trackpads). A linear scan: at tens of nodes this is nothing, and it needs no
  // spatial structure or point-in-polygon — the cheap half of the canvas trade.
  function pick(x, y) {
    let best = null, bestD = Infinity;
    for (const p of placed) {
      const d = Math.hypot(p.x - x, p.y - y);
      if (d <= p.r + 6 && d < bestD) { best = p; bestD = d; }
    }
    return best;
  }

  function localPoint(e) {
    const rect = canvas.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  }

  canvas.addEventListener("mousemove", e => {
    const { x, y } = localPoint(e);
    const hit = pick(x, y);
    const id = hit ? hit.node.id : null;
    if (id === hoverId) return;
    hoverId = id;
    canvas.style.cursor = id ? "pointer" : "default";
    if (!raf) draw(); // settled: a hover needs a repaint, not a re-simulation
  });

  canvas.addEventListener("mouseleave", () => {
    if (hoverId === null) return;
    hoverId = null;
    if (!raf) draw();
  });

  canvas.addEventListener("click", e => {
    const { x, y } = localPoint(e);
    const hit = pick(x, y);
    // Clicking the focus itself is a no-op rather than a re-centre on itself.
    // The kind travels with the id: the page selection is kinded, and a group id
    // handed to `selectRoom` would name nothing.
    if (hit && hit.node.id !== focusId && onSelect) {
      onSelect(hit.node.id, groupDepth == null ? "room" : "area");
    }
  });

  // ---- sizing -----------------------------------------------------------

  // Size the backing store by devicePixelRatio and scale the context, or every
  // label is soft on a retina display. Called on construction, on container
  // resize, and whenever the panel is revealed (a hidden canvas measures 0).
  function resize() {
    const rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    cssW = Math.max(rect.width, 1);
    cssH = Math.max(rect.height, 1);
    canvas.width = Math.round(cssW * dpr);
    canvas.height = Math.round(cssH * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    draw();
  }

  return {
    setData,
    // The focus states both what to centre on and at what granularity:
    // `{ kind: "room", id }`, or `{ kind: "area", id, depth }` for a hierarchy
    // group at tier `depth`, or null for nothing selected.
    setFocus(focus) {
      const id = focus ? focus.id : null;
      const nextGroup = focus && focus.kind === "area" ? Math.max(0, focus.depth | 0) : null;
      if (id === focusId && nextGroup === groupDepth) return;
      focusId = id;
      if (nextGroup !== groupDepth) {
        groupDepth = nextGroup;
        rebuildView();
      }
      layout();
    },
    setDepth(d) {
      depth = d === Infinity ? Infinity : Math.max(1, d | 0);
      layout();
    },
    setTierDepth(d) {
      tierDepth = Math.max(0, d | 0);
      rebuildColourKeys();
      if (!raf) draw();
    },
    tierNames,
    resize,
    // Re-read :root after a theme change would go here; today tokens.css is
    // static, so this exists only for the resize path to reuse.
    refreshPalette() { palette = readPalette(); if (!raf) draw(); },
    // Node count actually on screen — the panel's meta line reports it, since
    // "how much of the level is this showing?" is the first question the depth
    // cap raises.
    shownCount() { return placed.length; },
    // How many nodes the focus touches directly, counted in the CURRENT
    // granularity: the panel cannot compute this from the payload once the view
    // is aggregated, because the answer is a count of groups, not of rooms.
    focusDegree() {
      return focusId && neighbours.has(focusId) ? neighbours.get(focusId).length : 0;
    },
    // The tier the view is aggregated to, or null in room granularity. Read by
    // the panel to keep an area focus resolvable when the click came from a graph
    // node rather than from a plan footprint.
    groupDepth() { return groupDepth; },
  };
}
