// Synthetic floor-plate geometry, shared byte-for-byte by both renderer pages.
//
// Why this file exists rather than loading a real snapshot: the POC compares
// draw layers, and a real fixture caps out at 5,046 rooms/level. The sweep needs
// to double past that repeatedly, so the geometry has to be generated. It is
// still shaped like the real thing -- `scripts/gen_big_plate.py` builds
// Building > Department > Room grids of 16x14 ft cells in feet, Y-up, and this
// mirrors those units and that structure so the world scale is comparable to
// the fixture the existing SVG frame-time numbers came from.
//
// Three properties are load-bearing for the comparison, and each is here for a
// reason the brief spells out:
//
//   1. DETERMINISTIC. Both pages call generateRooms(n, SEED) and must get
//      identical geometry, or the two curves are not measuring the same work.
//      Hence an explicit mulberry32 rather than Math.random.
//   2. VARIED. "50k identical squares" flatters both renderers and especially
//      flatters WebGL, which would then invite instancing that real rooms never
//      permit. So vertex count varies 4-12 and footprint varies 1x to 4x cells.
//   3. THE PLATE GROWS WITH THE COUNT, at fixed cell size. Doubling the room
//      count doubles the plate *area* instead of shrinking the rooms. That is
//      what keeps the pan phase's on-screen room count roughly constant from
//      rung to rung -- without it, a higher rung would quietly also be a
//      zoomed-out test and the ladder would confound two variables.
//
// Storage is struct-of-arrays. At the top of the sweep this holds millions of
// rooms; an object-with-its-own-Float32Array per room is millions of small
// allocations, and generation time starts competing with the measurement.

(function (global) {
  "use strict";

  // Matches gen_big_plate.py's CELL_W / CELL_H. Feet.
  var CELL_W = 16.0;
  var CELL_H = 14.0;

  // Gap between a room's footprint and its cell, standing in for wall
  // thickness. Keeps rooms visibly separate so the fill/stroke work is
  // representative rather than one merged mass of colour.
  var WALL_GAP = 0.9;

  // Labels are drawn from a fixed pool rather than built per room. Two reasons:
  // a few million distinct strings is a hundred-odd MB of pure overhead that has
  // nothing to do with the thing being measured, and real plates genuinely do
  // repeat room names heavily. What matters for the text layer is that the
  // *widths* vary across the visible set, which pooling preserves -- room i uses
  // pool[i % POOL], and the visible set spreads across the pool.
  var LABEL_POOL = 4096;

  var DEPARTMENTS = [
    "Emergency", "Surgery", "Imaging", "Wards A", "Wards B", "Outpatient",
    "Laboratory", "Pharmacy", "Administration", "Sterilisation", "ICU", "Maternity"
  ];
  var ROOM_KINDS = [
    "Bedroom", "Office", "Store", "Consult", "Plant", "WC", "Corridor",
    "Treatment", "Lab", "Waiting", "Clean Utility", "Dirty Utility"
  ];

  // Palette indices only -- the pages own the actual colours, because canvas and
  // WebGL want them in different forms (CSS string vs. normalized float).
  var PALETTE_SIZE = 8;

  function makeRng(seed) {
    var a = seed >>> 0;
    return function () {
      a = (a + 0x6d2b79f5) >>> 0;
      var t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  function buildLabelPool(rng) {
    var pool = new Array(LABEL_POOL);
    for (var i = 0; i < LABEL_POOL; i++) {
      var dept = DEPARTMENTS[(rng() * DEPARTMENTS.length) | 0];
      var kind = ROOM_KINDS[(rng() * ROOM_KINDS.length) | 0];
      var level = 1 + ((rng() * 9) | 0);
      var num = 100 + ((rng() * 899) | 0);
      var area = 8 + ((rng() * 60) | 0);
      // Two lines, shaped like the viewer's server-resolved `room.label`: a
      // primary identifier line and a smaller accent line beneath it.
      //
      // ASCII only, deliberately. The WebGL page's glyph atlas covers 32..126,
      // so a "·" or a "²" would be silently dropped there and drawn by
      // Canvas2D -- two fewer glyphs per label for one renderer, which is a
      // small thumb on the scale and a confusing screenshot. Widening the atlas
      // would work equally well; keeping the text identical is simpler and
      // leaves nothing to argue about.
      pool[i] = [
        level + "." + num + " " + kind,
        dept + " - " + area + " sqm"
      ];
    }
    return pool;
  }

  // Footprints, as (cols, rows) of cells with their relative frequency. Most
  // rooms are one cell; a minority are doubles and a few are quads. Cumulative
  // weights so a single rng() draw picks one.
  var FOOTPRINTS = [
    { w: 1, h: 1, cum: 0.70 },
    { w: 2, h: 1, cum: 0.82 },
    { w: 1, h: 2, cum: 0.94 },
    { w: 2, h: 2, cum: 1.00 }
  ];

  /**
   * A polygon inscribed in `rect`, star-shaped about its own centroid.
   *
   * Star-shaped-about-centroid, not merely convex, is the exact property the
   * WebGL page depends on: it triangulates with a fan anchored at the centroid
   * rather than at vertex 0, and that fan is a correct, non-overlapping cover
   * for any polygon whose whole boundary is visible from the centroid. Sorted
   * distinct angles with positive radii guarantee it. This is what lets the POC
   * skip an earcut dependency without quietly restricting itself to convex
   * shapes.
   *
   * The honest limitation, recorded in README.md: an L-shaped room is not
   * star-shaped about its centroid and cannot appear here.
   */
  function emitPolygon(rng, x0, y0, x1, y1, xs, ys, at) {
    var cx = (x0 + x1) / 2;
    var cy = (y0 + y1) / 2;
    var hx = (x1 - x0) / 2;
    var hy = (y1 - y0) / 2;

    // Just under half of rooms are plain rectangles, which is what a real plate
    // looks like. Vertex count only varies for the rest.
    if (rng() < 0.45) {
      xs[at] = x0; ys[at] = y0;
      xs[at + 1] = x1; ys[at + 1] = y0;
      xs[at + 2] = x1; ys[at + 2] = y1;
      xs[at + 3] = x0; ys[at + 3] = y1;
      return 4;
    }

    var k = 5 + ((rng() * 8) | 0); // 5..12; the 4-vertex case is the rect above
    // Angles are evenly spaced then jittered by less than half a step, so they
    // stay strictly increasing and the ring never self-crosses.
    var step = (Math.PI * 2) / k;
    for (var j = 0; j < k; j++) {
      var theta = j * step + (rng() - 0.5) * step * 0.8;
      var r = 0.62 + rng() * 0.38; // fraction of the half-extent in that direction
      xs[at + j] = cx + Math.cos(theta) * hx * r;
      ys[at + j] = cy + Math.sin(theta) * hy * r;
    }
    return k;
  }

  /**
   * Generate `count` rooms.
   *
   * Returns a struct-of-arrays bundle:
   *   count                 number of rooms
   *   xs, ys                Float32Array, all vertices concatenated
   *   offsets               Uint32Array(count + 1), vertex range of room i is
   *                         [offsets[i], offsets[i + 1])
   *   bbox                  Float32Array(count * 4), minX minY maxX maxY
   *   cx, cy                Float32Array(count), centroid: label anchor and the
   *                         WebGL fan's centre vertex
   *   colour                Uint8Array(count), palette index
   *   labels                Array(LABEL_POOL) of [lineA, lineB]
   *   labelOf(i)            the pooled label pair for room i
   *   worldBBox             {minX, minY, maxX, maxY} of the whole plate
   */
  function generateRooms(count, seed) {
    var rng = makeRng(seed);

    // Rooms consume 1.42 cells on average under FOOTPRINTS; allocate with
    // margin and stop as soon as `count` rooms exist, so the grid is never the
    // thing that limits the rung.
    var slots = Math.ceil(count * 1.8) + 64;
    var cols = Math.ceil(Math.sqrt(slots));
    var rows = Math.ceil(slots / cols);
    var taken = new Uint8Array(cols * rows);

    // Worst case is every room a 12-gon. Allocated up front and subarray'd down
    // at the end -- growing these mid-generation is what makes naive generators
    // slower than the benchmark they feed.
    var maxVerts = count * 12;
    var xs = new Float32Array(maxVerts);
    var ys = new Float32Array(maxVerts);
    var offsets = new Uint32Array(count + 1);
    var bbox = new Float32Array(count * 4);
    var cxs = new Float32Array(count);
    var cys = new Float32Array(count);
    var colour = new Uint8Array(count);

    var n = 0;      // rooms emitted
    var at = 0;     // vertex cursor
    var slot = 0;   // grid cursor

    // The populated extent, accumulated as rooms are emitted. It is NOT the
    // allocated grid: slots are over-allocated so the grid never limits the
    // rung, and generation stops the moment `count` rooms exist, which leaves
    // the last fifth or so of the rows empty. Reporting the grid here would
    // send the pan camera sweeping across blank space and quietly average a
    // pile of zero-room frames into the result.
    var wMinX = Infinity, wMinY = Infinity, wMaxX = -Infinity, wMaxY = -Infinity;

    while (n < count && slot < taken.length) {
      if (taken[slot]) { slot++; continue; }

      var col = slot % cols;
      var row = (slot / cols) | 0;

      // Pick a footprint, shrinking to 1x1 if the cells it wants are not free.
      var pick = rng();
      var fw = 1, fh = 1;
      for (var f = 0; f < FOOTPRINTS.length; f++) {
        if (pick <= FOOTPRINTS[f].cum) { fw = FOOTPRINTS[f].w; fh = FOOTPRINTS[f].h; break; }
      }
      if (col + fw > cols || row + fh > rows) { fw = 1; fh = 1; }
      var free = true;
      for (var dy = 0; dy < fh && free; dy++) {
        for (var dx = 0; dx < fw; dx++) {
          if (taken[(row + dy) * cols + (col + dx)]) { free = false; break; }
        }
      }
      if (!free) { fw = 1; fh = 1; }
      for (var my = 0; my < fh; my++) {
        for (var mx = 0; mx < fw; mx++) taken[(row + my) * cols + (col + mx)] = 1;
      }

      var x0 = col * CELL_W + WALL_GAP;
      var y0 = row * CELL_H + WALL_GAP;
      var x1 = (col + fw) * CELL_W - WALL_GAP;
      var y1 = (row + fh) * CELL_H - WALL_GAP;

      offsets[n] = at;
      var k = emitPolygon(rng, x0, y0, x1, y1, xs, ys, at);

      // bbox and centroid from the emitted vertices, never from the cell: a
      // jittered polygon sits inside its cell, and culling against the cell
      // would over-report what is on screen.
      var minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
      var sx = 0, sy = 0;
      for (var v = 0; v < k; v++) {
        var px = xs[at + v], py = ys[at + v];
        if (px < minX) minX = px;
        if (px > maxX) maxX = px;
        if (py < minY) minY = py;
        if (py > maxY) maxY = py;
        sx += px; sy += py;
      }
      bbox[n * 4] = minX;
      bbox[n * 4 + 1] = minY;
      bbox[n * 4 + 2] = maxX;
      bbox[n * 4 + 3] = maxY;
      if (minX < wMinX) wMinX = minX;
      if (minY < wMinY) wMinY = minY;
      if (maxX > wMaxX) wMaxX = maxX;
      if (maxY > wMaxY) wMaxY = maxY;
      cxs[n] = sx / k;
      cys[n] = sy / k;
      colour[n] = (rng() * PALETTE_SIZE) | 0;

      at += k;
      n++;
      slot++;
    }
    offsets[n] = at;

    var labels = buildLabelPool(rng);

    return {
      count: n,
      xs: xs.subarray(0, at),
      ys: ys.subarray(0, at),
      offsets: offsets,
      bbox: bbox,
      cx: cxs,
      cy: cys,
      colour: colour,
      paletteSize: PALETTE_SIZE,
      labels: labels,
      labelOf: function (i) { return labels[i % LABEL_POOL]; },
      worldBBox: { minX: wMinX, minY: wMinY, maxX: wMaxX, maxY: wMaxY }
    };
  }

  global.POCGen = {
    generateRooms: generateRooms,
    makeRng: makeRng,
    CELL_W: CELL_W,
    CELL_H: CELL_H
  };
})(this);
