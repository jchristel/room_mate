// Radial room-adjacency graph, drawn on a <canvas>.
//
// Loaded as a classic <script> after common.js (whose `qualitative` palette this
// shares with the plan renderer, so the two agree on what colour a department
// is). Zero build step, same ServeDir as every other page asset.
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

// Ring 0 sits at the centre; each further hop steps outward by an equal share of
// the available radius, so a depth-3 graph is not squeezed into the same rings a
// depth-1 graph uses.
const GRAPH_NODE_R = 7;          // node dot radius, CSS px
const GRAPH_FOCUS_R = 10;        // the selected room reads larger
const GRAPH_LABEL_FONT = 11;     // CSS px
// Simulation: angular spring toward the neighbours in the ring below, angular
// repulsion between same-ring nodes so labels do not stack, and enough damping
// that the whole thing is visibly still within a second.
const GRAPH_ATTRACT = 0.06;
const GRAPH_REPEL = 0.35;
const GRAPH_DAMPING = 0.82;
// Below this peak angular velocity (radians/frame) the picture is not visibly
// moving, so the loop stops. A force sim left running is a permanent 60fps
// repaint of a static image — on the same page as a 5,000-room plan, that is not
// a cost worth paying for nothing.
const GRAPH_SETTLE_EPS = 0.0006;

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
  let d = (b - a) % (Math.PI * 2);
  if (d > Math.PI) d -= Math.PI * 2;
  if (d <= -Math.PI) d += Math.PI * 2;
  return d;
}

function createRoomGraph(canvas, { onSelect } = {}) {
  const ctx = canvas.getContext("2d");
  let palette = readPalette();

  let payload = null;        // the /adjacency response
  let nodesById = new Map(); // room_id -> node
  let neighbours = new Map();// room_id -> [{ id, weight }]
  let focusId = null;
  let depth = 2;
  let tierDepth = 0;         // which classification tier drives node colour
  let colourKeys = [];       // stable, sorted key list -> palette index

  let placed = [];           // the laid-out nodes: { node, hop, angle, vel, x, y, r }
  let hoverId = null;
  let raf = 0;
  let cssW = 0, cssH = 0;

  // ---- data -------------------------------------------------------------

  function setData(next) {
    payload = next;
    nodesById = new Map();
    neighbours = new Map();
    if (payload) {
      for (const n of payload.nodes) {
        nodesById.set(n.room_id, n);
        neighbours.set(n.room_id, []);
      }
      for (const e of payload.edges) {
        // An edge naming a room that is not in `nodes` cannot happen from this
        // server, but a defensive skip beats a thrown TypeError inside a rAF.
        if (!neighbours.has(e.a) || !neighbours.has(e.b)) continue;
        neighbours.get(e.a).push({ id: e.b, weight: e.shared_length });
        neighbours.get(e.b).push({ id: e.a, weight: e.shared_length });
      }
    }
    rebuildColourKeys();
    layout();
  }

  // The colour key for a node at the active tier: its resolved code, else its
  // name, else null for an undefined/absent tier (drawn in the neutral fill,
  // never an error colour — same stance as the plan's "no data" grey).
  function colourKey(node) {
    const tv = (node.classification || [])[tierDepth];
    if (!tv || tv.undefined) return null;
    return tv.code != null ? tv.code : (tv.name != null ? tv.name : null);
  }

  // Sorted so the same department gets the same palette index on every fetch —
  // a colour that changed as rooms came and went would be worse than useless.
  function rebuildColourKeys() {
    const keys = new Set();
    for (const n of nodesById.values()) {
      const k = colourKey(n);
      if (k != null) keys.add(k);
    }
    colourKeys = [...keys].sort();
  }

  function colourFor(node) {
    const k = colourKey(node);
    if (k == null) return palette.fill;
    return qualitative("Set2", colourKeys.indexOf(k));
  }

  // The tier labels available for the colour picker, taken from the nodes'
  // own classification paths (depth i carries that tier's label).
  function tierNames() {
    const names = [];
    for (const n of nodesById.values())
      (n.classification || []).forEach((t, i) => { if (names[i] === undefined) names[i] = t.tier; });
    return names;
  }

  // ---- layout -----------------------------------------------------------

  // Breadth-first from the focus, capped at `depth`. The cap is not a
  // performance guard — it is the readability guarantee. The full graph of a
  // hospital level is an unreadable hairball and must never be the default view.
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
      if (!frontier.length) break;
    }
    return [...hops].map(([id, hop]) => ({ node: nodesById.get(id), hop }));
  }

  function layout() {
    const found = reachable();
    // Seed each ring's angles evenly, then let the simulation sort them out.
    // Rooms are seeded in a stable order (by id) so the same graph opens the
    // same way twice rather than reshuffling on every fetch.
    const byHop = new Map();
    for (const f of found) {
      if (!byHop.has(f.hop)) byHop.set(f.hop, []);
      byHop.get(f.hop).push(f);
    }
    placed = [];
    for (const [hop, group] of byHop) {
      group.sort((a, b) => (a.node.room_id < b.node.room_id ? -1 : 1));
      group.forEach((f, i) => {
        placed.push({
          node: f.node,
          hop,
          angle: hop === 0 ? 0 : (i / group.length) * Math.PI * 2,
          vel: 0,
          x: 0, y: 0,
          r: hop === 0 ? GRAPH_FOCUS_R : GRAPH_NODE_R,
        });
      });
    }
    start();
  }

  // One simulation step. Returns the peak angular velocity so the caller can
  // decide whether the picture is still moving.
  function step() {
    const byHop = new Map();
    for (const p of placed) {
      if (!byHop.has(p.hop)) byHop.set(p.hop, []);
      byHop.get(p.hop).push(p);
    }
    const index = new Map(placed.map(p => [p.node.room_id, p]));
    let peak = 0;

    for (const [hop, ring] of byHop) {
      if (hop === 0) continue; // the focus is anchored at the centre by definition
      const ringR = ringRadius(hop);
      // Minimum angular separation that keeps two dots (and their labels) from
      // colliding. Derived from the ring's radius, so an inner ring spreads its
      // nodes further apart in angle than an outer one — which is correct.
      const minSep = Math.min(Math.PI / 2, (GRAPH_NODE_R * 5) / Math.max(ringR, 1));

      for (const p of ring) {
        let force = 0;

        // Spring toward the circular mean of this node's neighbours one hop in,
        // weighted by shared wall length: a room joined by a 4m wall is pulled
        // to sit beside its neighbour harder than one joined by a 200mm reveal.
        let sx = 0, sy = 0;
        for (const nb of neighbours.get(p.node.room_id) || []) {
          const other = index.get(nb.id);
          if (!other || other.hop !== hop - 1) continue;
          const w = Math.max(nb.weight, 0.1);
          sx += Math.cos(other.angle) * w;
          sy += Math.sin(other.angle) * w;
        }
        if (sx !== 0 || sy !== 0) force += GRAPH_ATTRACT * angleDelta(p.angle, Math.atan2(sy, sx));

        // Angular repulsion from same-ring neighbours, so a ring with many
        // members spreads instead of piling up under one parent.
        for (const q of ring) {
          if (q === p) continue;
          const d = angleDelta(q.angle, p.angle);
          const ad = Math.abs(d);
          if (ad >= minSep) continue;
          force += GRAPH_REPEL * (minSep - ad) * (d >= 0 ? 1 : -1);
        }

        p.vel = (p.vel + force) * GRAPH_DAMPING;
        peak = Math.max(peak, Math.abs(p.vel));
      }
    }

    for (const p of placed) p.angle += p.vel;
    return peak;
  }

  function ringRadius(hop) {
    const usable = Math.min(cssW, cssH) / 2 - 28; // margin for labels
    return (Math.max(usable, 40) / Math.max(depth, 1)) * hop;
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
        focusId ? "selected room is not in this scope" : "click a room on the plan",
        cssW / 2, cssH / 2,
      );
      ctx.globalAlpha = 1;
      return;
    }

    // Faint ring guides, so "one ring out" is readable as a distance rather
    // than inferred from the node positions.
    ctx.strokeStyle = palette.rule;
    ctx.lineWidth = 1;
    for (let h = 1; h <= depth; h++) {
      ctx.beginPath();
      ctx.arc(cssW / 2, cssH / 2, ringRadius(h), 0, Math.PI * 2);
      ctx.stroke();
    }

    const index = new Map(placed.map(p => [p.node.room_id, p]));

    // Edges first, so nodes sit on top. Only edges between two laid-out nodes
    // are drawn — an edge to a room beyond the depth cap is not half-drawn.
    const maxWeight = Math.max(...(payload ? payload.edges.map(e => e.shared_length) : [1]), 1);
    for (const e of (payload ? payload.edges : [])) {
      const a = index.get(e.a), b = index.get(e.b);
      if (!a || !b) continue;
      // Thickness encodes shared wall length: a 4m wall is a stronger
      // relationship than a 200mm corner touch and should look like one.
      ctx.strokeStyle = palette.ink;
      ctx.globalAlpha = 0.18 + 0.42 * (e.shared_length / maxWeight);
      ctx.lineWidth = 1 + 3.5 * (e.shared_length / maxWeight);
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    ctx.font = `${GRAPH_LABEL_FONT}px ui-monospace, monospace`;
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    for (const p of placed) {
      const isFocus = p.hop === 0;
      const isHover = p.node.room_id === hoverId;
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.r + (isHover ? 2 : 0), 0, Math.PI * 2);
      ctx.fillStyle = colourFor(p.node);
      ctx.fill();
      ctx.lineWidth = isFocus ? 3 : 1.5;
      ctx.strokeStyle = isFocus || isHover ? palette.accent : palette.ink;
      ctx.stroke();

      // Label the focus and ring 1 always; label further rings only when the
      // graph is sparse enough for the text not to become the noise.
      if (isFocus || p.hop === 1 || placed.length <= 24 || isHover) {
        const text = p.node.name || p.node.room_id;
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
    const id = hit ? hit.node.room_id : null;
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
    if (hit && hit.node.room_id !== focusId && onSelect) onSelect(hit.node.room_id);
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
    setFocus(id) {
      if (id === focusId) return;
      focusId = id;
      layout();
    },
    setDepth(d) {
      depth = Math.max(1, d | 0);
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
  };
}
