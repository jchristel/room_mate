// Everything that is not the draw layer, shared verbatim by both pages.
//
// This is the brief's "one thing that is not the renderer". Viewport culling,
// pick and the label policy are the same operation whichever API painted the
// pixels, so they live here and both pages get the identical implementation.
// The usual way to make Canvas2D lose for a bogus reason is to redraw all n
// polygons every frame with no cull; giving the cull to both is what makes the
// comparison mean anything. WebGL only earns the win if it still needs to
// *after* Canvas2D has one.
//
// Two measurement problems this file exists to solve honestly:
//
// FORCED SYNC. Both APIs defer rasterization. `performance.now()` bracketing a
// pile of draw calls measures the time to *issue* them, not to complete them,
// and it understates WebGL far more than Canvas2D. So each page hands back a
// `sync()` that forces its pipeline to drain -- `gl.finish()` for WebGL, a 1x1
// `getImageData` for Canvas2D -- and it runs inside the timed region. This is
// conservative against both, equally, and it is the difference between a number
// and a fiction.
//
// rAF IS CLAMPED. Frame-to-frame delta cannot go below the display interval, so
// it is blind to any headroom above 60fps -- exactly the reason the brief asks
// for a frame-time distribution rather than FPS. Work time (cull + draw + pick,
// with the sync) is the number that discriminates when a renderer is fast;
// rAF delta is the number that tells the truth when a renderer is slow. Both
// are captured, and the report shows both.

(function (global) {
  "use strict";

  var Flatbush = global.Flatbush;

  // ---------------------------------------------------------------- constants

  // Fixed CSS viewport so a rung is reproducible and the two pages are
  // measuring the same number of pixels. The backing store is this times DPR.
  var VIEW_W = 1200;
  var VIEW_H = 800;

  // The pan phase's zoom, in screen pixels per world foot. Chosen so a working
  // view holds a couple of hundred rooms -- a plausible floorplan working zoom,
  // and (because gen.js grows the plate with the room count instead of
  // shrinking rooms) constant across every rung of the sweep.
  var WORKING_PPF = 4.0;

  // A label is drawn only when its room's on-screen bounding box is at least
  // this wide. Below it the two lines would be illegible anyway, so drawing
  // them is pure cost. Same policy, both renderers.
  var LABEL_MIN_PX = 42;

  // The budget, pre-stated in README.md before anything was measured: p95 of
  // per-frame work during the pan phase, labels and pick included, at this
  // machine's DPR.
  var BUDGET_MS = 16;

  // A renderer that is at four times budget has lost decisively; doubling the
  // rung again only measures how much worse it gets. Each page stops its own
  // ladder there. The cross-renderer crossover is read off the two tables
  // afterwards, which is the only place it can honestly be determined.
  var STOP_AT_MULTIPLE = 4;

  // The ladder starts well below the brief's 50k. Canvas2D turned out to be
  // over budget on the fitted view at 12 500 already -- the lowest rung the
  // ladder originally had -- so the rungs that actually locate its ceiling are
  // the small ones, and they bracket the real fixture (`big-plate`, 5 046
  // rooms/level) instead of sitting an order of magnitude above it.
  var RUNGS = [1000, 2000, 4000, 8000, 12500, 25000, 50000,
               100000, 200000, 400000, 800000, 1600000];

  var SEED = 0x5eed1234;

  // Adaptive sampling: a phase ends at whichever of frames-or-seconds comes
  // first, with a floor so a slow rung still yields a usable distribution. A
  // rung where a renderer is at three seconds a frame then costs a minute
  // instead of hanging the tab, and the frames actually sampled are reported so
  // a thin sample is visible rather than hidden.
  var PHASES = [
    { name: "warmup", frames: 30, maxMs: 2000, minFrames: 5, discard: true },
    { name: "fitted", frames: 120, maxMs: 6000, minFrames: 12 },
    { name: "pan", frames: 240, maxMs: 8000, minFrames: 20 },
    { name: "zoom", frames: 120, maxMs: 6000, minFrames: 12 }
  ];

  // Shared palette. The pages convert it to whatever form their API wants.
  var PALETTE = [
    [0x7c, 0x9c, 0xbf], [0x9f, 0xbf, 0x7c], [0xbf, 0xa8, 0x7c], [0xbf, 0x7c, 0x8f],
    [0x8f, 0x7c, 0xbf], [0x7c, 0xbf, 0xb4], [0xbf, 0x9a, 0xd0], [0xa8, 0xb0, 0xb8]
  ];

  // ------------------------------------------------------------ spatial index

  // Flatbush rather than RBush: the box set is static for the life of a rung
  // (it is a snapshot), which is the case Flatbush is built for -- one packed
  // build, no incremental insert, no per-node objects. At the top of the ladder
  // that difference is the difference between an index that builds in a moment
  // and one that dominates the run.
  function buildIndex(rooms) {
    var idx = new Flatbush(rooms.count);
    var bbox = rooms.bbox;
    for (var i = 0; i < rooms.count; i++) {
      idx.add(bbox[i * 4], bbox[i * 4 + 1], bbox[i * 4 + 2], bbox[i * 4 + 3]);
    }
    idx.finish();
    return idx;
  }

  // Ray-cast point-in-polygon over a room's vertex range. Used by pick only --
  // the index narrows to a handful of candidates first, so this never runs at
  // scale.
  function pointInRoom(rooms, i, px, py) {
    var xs = rooms.xs, ys = rooms.ys;
    var a = rooms.offsets[i], b = rooms.offsets[i + 1];
    var inside = false;
    for (var p = a, q = b - 1; p < b; q = p++) {
      var xi = xs[p], yi = ys[p], xj = xs[q], yj = ys[q];
      if (((yi > py) !== (yj > py)) && (px < ((xj - xi) * (py - yi)) / (yj - yi) + xi)) {
        inside = !inside;
      }
    }
    return inside;
  }

  // ------------------------------------------------------------------- camera

  // The camera is a pure function of the frame index, so both pages traverse
  // byte-identical viewports. Nothing about the path may depend on how fast the
  // renderer happens to be running, or a slow renderer would quietly be given a
  // shorter journey through the geometry.
  function tri(t) {
    t = t - Math.floor(t);
    return t < 0.5 ? 4 * t - 1 : 3 - 4 * t;
  }

  function cameraFor(phase, i, world) {
    var worldW = world.maxX - world.minX;
    var worldH = world.maxY - world.minY;
    var cx = world.minX + worldW / 2;
    var cy = world.minY + worldH / 2;
    var fit = Math.min(VIEW_W / worldW, VIEW_H / worldH) * 0.95;

    if (phase === "fitted") {
      // The whole plate on screen, drifting and breathing slightly. The cull
      // culls nothing here -- this is the case STRATEGY-BROWSER.md still lists
      // as open for SVG, and the one where mass unculled geometry actually
      // lands. A static redraw would not do: the brief is explicit that initial
      // draw is nearly irrelevant and what kills a renderer is continuous
      // transform.
      return {
        scale: fit * (1 + 0.03 * Math.sin(i * 0.05)),
        x: cx + 0.02 * worldW * Math.sin(i * 0.031),
        y: cy + 0.02 * worldH * Math.cos(i * 0.027)
      };
    }

    if (phase === "pan" || phase === "warmup") {
      // Constant-velocity sweep at working zoom: triangle waves, not sines, so
      // speed does not drop at the turns, and with co-prime-ish periods so the
      // path does not retrace itself within the phase and hand the renderer
      // warm caches.
      var mx = VIEW_W / (2 * WORKING_PPF);
      var my = VIEW_H / (2 * WORKING_PPF);
      var ampX = Math.max(0, worldW / 2 - mx) * 0.9;
      var ampY = Math.max(0, worldH / 2 - my) * 0.9;
      return {
        scale: WORKING_PPF,
        x: cx + ampX * tri(i / 220),
        y: cy + ampY * tri(i / 370 + 0.25)
      };
    }

    // zoom: exponential sweep between fitted and working and back, which is
    // what a real zoom gesture traverses.
    var u = 0.5 - 0.5 * Math.cos((i * Math.PI * 2) / 120);
    return {
      scale: fit * Math.pow(WORKING_PPF / fit, u),
      x: cx + 0.01 * worldW * Math.sin(i * 0.02),
      y: cy + 0.01 * worldH * Math.cos(i * 0.02)
    };
  }

  // World rect currently on screen. Y is flipped at draw time (world is Y-up,
  // screen is Y-down) exactly as the shipped viewer does it.
  function viewRect(cam) {
    var hw = VIEW_W / (2 * cam.scale);
    var hh = VIEW_H / (2 * cam.scale);
    return { minX: cam.x - hw, minY: cam.y - hh, maxX: cam.x + hw, maxY: cam.y + hh };
  }

  // The cursor pick runs every frame in both pages, on a path derived from the
  // same frame index. Including it in one page and not the other would rig the
  // comparison; the brief calls this out specifically.
  function cursorFor(i, cam) {
    var sx = VIEW_W * (0.5 + 0.35 * Math.cos(i * 0.07));
    var sy = VIEW_H * (0.5 + 0.35 * Math.sin(i * 0.053));
    return {
      x: cam.x + (sx - VIEW_W / 2) / cam.scale,
      y: cam.y - (sy - VIEW_H / 2) / cam.scale
    };
  }

  // ------------------------------------------------------------------- stats

  function percentile(sorted, p) {
    if (!sorted.length) return 0;
    var idx = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
    return sorted[idx];
  }

  function summarize(samples) {
    var s = samples.slice().sort(function (a, b) { return a - b; });
    var sum = 0;
    for (var i = 0; i < s.length; i++) sum += s[i];
    return {
      n: s.length,
      mean: s.length ? sum / s.length : 0,
      p50: percentile(s, 50),
      p95: percentile(s, 95),
      p99: percentile(s, 99),
      max: s.length ? s[s.length - 1] : 0
    };
  }

  // -------------------------------------------------------------- the runner

  function nextFrame() {
    return new Promise(function (resolve) { requestAnimationFrame(resolve); });
  }

  /**
   * Run every phase for one rung against one renderer.
   *
   * `renderer` is what the page supplies:
   *   draw(visible, cam, showLabels)  paint the culled set
   *   sync()                          force the pipeline to drain (see header)
   *   dispose()                       release GPU/CPU resources before the next rung
   *   gpuMs()                         optional: real GPU time if the timer-query
   *                                   extension is available, else undefined
   */
  async function runRung(rooms, index, renderer, showLabels) {
    var out = {};
    var lastT = performance.now();

    for (var pi = 0; pi < PHASES.length; pi++) {
      var phase = PHASES[pi];
      var work = [], drawT = [], cullT = [], pickT = [], rafT = [], gpuT = [];
      var roomsDrawn = [], labelsDrawn = [];
      var started = performance.now();
      var i = 0;

      while (true) {
        await nextFrame();
        var now = performance.now();
        var delta = now - lastT;
        lastT = now;

        var cam = cameraFor(phase.name, i, rooms.worldBBox);
        var rect = viewRect(cam);

        var t0 = performance.now();
        var visible = index.search(rect.minX, rect.minY, rect.maxX, rect.maxY);
        var t1 = performance.now();

        var nLabels = renderer.draw(visible, cam, showLabels);
        renderer.sync();
        var t2 = performance.now();

        // Pick: index query at the cursor, then point-in-polygon on whatever
        // few candidates came back.
        var cur = cursorFor(i, cam);
        var hits = index.search(cur.x - 0.01, cur.y - 0.01, cur.x + 0.01, cur.y + 0.01);
        var picked = -1;
        for (var h = 0; h < hits.length; h++) {
          if (pointInRoom(rooms, hits[h], cur.x, cur.y)) { picked = hits[h]; break; }
        }
        var t3 = performance.now();
        renderer.lastPick = picked;

        if (!phase.discard) {
          cullT.push(t1 - t0);
          drawT.push(t2 - t1);
          pickT.push(t3 - t2);
          work.push(t3 - t0);
          rafT.push(delta);
          roomsDrawn.push(visible.length);
          labelsDrawn.push(nLabels);
          if (renderer.gpuMs) {
            var g = renderer.gpuMs();
            if (typeof g === "number") gpuT.push(g);
          }
        }

        i++;
        var elapsed = performance.now() - started;
        if (i >= phase.frames) break;
        if (elapsed > phase.maxMs && i >= phase.minFrames) break;
      }

      if (!phase.discard) {
        out[phase.name] = {
          work: summarize(work),
          draw: summarize(drawT),
          cull: summarize(cullT),
          pick: summarize(pickT),
          raf: summarize(rafT),
          gpu: gpuT.length ? summarize(gpuT) : null,
          roomsDrawn: Math.round(median(roomsDrawn)),
          labelsDrawn: Math.round(median(labelsDrawn))
        };
      }
    }
    return out;
  }

  function median(a) {
    if (!a.length) return 0;
    var s = a.slice().sort(function (x, y) { return x - y; });
    return s[(s.length / 2) | 0];
  }

  /**
   * The full sweep: the rung ladder, run twice -- labels on, then labels off.
   *
   * The labels-off ladder is not padding. The brief asserts that labels are what
   * sank SVG and calls them the single most decision-relevant variable; this
   * repo's own measurement
   * (docs/Superseded/HANDOVER-viewer-performance.md, "Labels are NOT the
   * bottleneck") says the opposite for SVG. Running both ladders settles it for
   * these two renderers by subtraction instead of inheriting either claim.
   */
  async function runSweep(createRenderer, canvas, report) {
    var results = { meta: meta(canvas), ladders: {} };
    global.POC_RESULTS = results;

    for (var li = 0; li < 2; li++) {
      var showLabels = li === 0;
      var ladder = [];
      results.ladders[showLabels ? "labelsOn" : "labelsOff"] = ladder;

      for (var ri = 0; ri < RUNGS.length; ri++) {
        var n = RUNGS[ri];
        var row = { n: n, stopped: null };
        var rooms = null, index = null, renderer = null;

        try {
          var g0 = performance.now();
          rooms = POCGen.generateRooms(n, SEED);
          var g1 = performance.now();
          index = buildIndex(rooms);
          var g2 = performance.now();
          renderer = createRenderer(rooms, index, canvas);
          var g3 = performance.now();
          row.actualRooms = rooms.count;
          row.genMs = g1 - g0;
          row.indexMs = g2 - g1;
          row.uploadMs = g3 - g2;
        } catch (err) {
          // Out of memory building the rung is itself a finding, not a crash to
          // hide: it is the ceiling of what this renderer can be handed at all.
          row.stopped = "build failed: " + (err && err.message ? err.message : String(err));
          ladder.push(row);
          report(results);
          break;
        }

        row.phases = await runRung(rooms, index, renderer, showLabels);
        renderer.dispose();
        ladder.push(row);
        report(results);

        var panP95 = row.phases.pan.work.p95;
        if (panP95 > BUDGET_MS * STOP_AT_MULTIPLE) {
          row.stopped = "pan p95 " + panP95.toFixed(1) + " ms exceeds " +
            (BUDGET_MS * STOP_AT_MULTIPLE) + " ms (" + STOP_AT_MULTIPLE + "x budget)";
          report(results);
          break;
        }
        // Let the tab breathe so the table paints and the previous rung's
        // buffers are actually collected before the next one allocates.
        await new Promise(function (r) { setTimeout(r, 250); });
      }
    }

    results.done = true;
    report(results);
    return results;
  }

  function meta(canvas) {
    var dpr = global.devicePixelRatio || 1;
    return {
      renderer: null, // filled in by the page
      dpr: dpr,
      cssSize: VIEW_W + "x" + VIEW_H,
      backingStore: canvas.width + "x" + canvas.height,
      budgetMs: BUDGET_MS,
      workingPpf: WORKING_PPF,
      labelMinPx: LABEL_MIN_PX,
      seed: SEED,
      ua: navigator.userAgent,
      when: new Date().toISOString()
    };
  }

  // -------------------------------------------------------------- the report

  function fmt(v) { return v == null ? "—" : v.toFixed(1); }

  function renderReport(results, el) {
    var html = "<h2>" + (results.meta.renderer || "?") + "</h2>";
    html += "<p class=meta>DPR <b>" + results.meta.dpr + "</b> · CSS " +
      results.meta.cssSize + " · backing store " + results.meta.backingStore +
      " · budget " + results.meta.budgetMs + " ms p95 work on pan · " +
      results.meta.workingPpf + " px/ft · label ≥ " + results.meta.labelMinPx + " px</p>";

    ["labelsOn", "labelsOff"].forEach(function (key) {
      var ladder = results.ladders[key];
      if (!ladder || !ladder.length) return;
      html += "<h3>Labels " + (key === "labelsOn" ? "ON" : "OFF") + "</h3>";
      html += "<table><thead><tr>" +
        "<th>rooms</th><th>drawn<br>(pan)</th><th>labels<br>(pan)</th>" +
        "<th>pan<br>work p50</th><th>pan<br>work p95</th><th>pan<br>draw p95</th>" +
        "<th>pan<br>cull p95</th><th>pan<br>rAF p50</th><th>pan<br>frames</th>" +
        "<th>fitted<br>work p95</th><th>fitted<br>frames</th>" +
        "<th>zoom<br>work p95</th>" +
        "<th>gen<br>ms</th><th>index<br>ms</th><th>upload<br>ms</th>" +
        "</tr></thead><tbody>";
      ladder.forEach(function (row) {
        if (!row.phases) {
          html += "<tr class=stopped><td>" + row.n.toLocaleString() +
            "</td><td colspan=14>" + row.stopped + "</td></tr>";
          return;
        }
        var p = row.phases.pan, f = row.phases.fitted, z = row.phases.zoom;
        var over = p.work.p95 > results.meta.budgetMs;
        html += "<tr class=" + (over ? "over" : "under") + ">" +
          "<td>" + row.actualRooms.toLocaleString() + "</td>" +
          "<td>" + p.roomsDrawn.toLocaleString() + "</td>" +
          "<td>" + p.labelsDrawn.toLocaleString() + "</td>" +
          "<td>" + fmt(p.work.p50) + "</td>" +
          "<td><b>" + fmt(p.work.p95) + "</b></td>" +
          "<td>" + fmt(p.draw.p95) + "</td>" +
          "<td>" + fmt(p.cull.p95) + "</td>" +
          "<td>" + fmt(p.raf.p50) + "</td>" +
          "<td>" + p.work.n + "</td>" +
          "<td>" + fmt(f.work.p95) + "</td>" +
          "<td>" + f.work.n + "</td>" +
          "<td>" + fmt(z.work.p95) + "</td>" +
          "<td>" + fmt(row.genMs) + "</td>" +
          "<td>" + fmt(row.indexMs) + "</td>" +
          "<td>" + fmt(row.uploadMs) + "</td>" +
          "</tr>";
        if (row.stopped) {
          html += "<tr class=stopped><td></td><td colspan=14>stopped: " + row.stopped + "</td></tr>";
        }
      });
      html += "</tbody></table>";
    });

    if (results.done) html += "<p class=done>sweep complete</p>";
    el.innerHTML = html;
  }

  // `?demo` draws one static frame at pan zoom instead of sweeping. It exists
  // because the whole comparison assumes the two pages paint the same picture,
  // and nothing in the numbers would reveal it if they did not -- a page that
  // silently dropped its outlines, or its labels, would simply look fast.
  // Same rooms, same camera, both pages, eyeball them side by side.
  function runDemo(createRenderer, canvas, note) {
    var rooms = POCGen.generateRooms(12500, SEED);
    var index = buildIndex(rooms);
    var renderer = createRenderer(rooms, index, canvas);
    var cam = cameraFor("pan", 100, rooms.worldBBox);
    var rect = viewRect(cam);
    var visible = index.search(rect.minX, rect.minY, rect.maxX, rect.maxY);
    var labels = renderer.draw(visible, cam, true);
    renderer.sync();
    note.textContent = "demo frame · 12,500 rooms · " + visible.length +
      " culled to view · " + labels + " labels · scale " + cam.scale + " px/ft";
    return { visible: visible.length, labels: labels };
  }

  global.POCHarness = {
    VIEW_W: VIEW_W,
    VIEW_H: VIEW_H,
    LABEL_MIN_PX: LABEL_MIN_PX,
    BUDGET_MS: BUDGET_MS,
    PALETTE: PALETTE,
    buildIndex: buildIndex,
    pointInRoom: pointInRoom,
    runSweep: runSweep,
    runDemo: runDemo,
    isDemo: /(\?|&)demo\b/.test(global.location.search),
    renderReport: renderReport
  };
})(this);
