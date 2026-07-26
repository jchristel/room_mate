# HANDOVER — Viewport culling disable switch

> **Superseded — built, and the question it asked is answered:** culling is
> worth keeping, by a wide margin (**16.5 ms/frame with it, 912 ms without**,
> `big-plate`, 2026-07-25). The switch itself is permanent and lives in
> `index.html` as `CULL_ENABLED`; [STRATEGY-BROWSER.md](../STRATEGY-BROWSER.md)
> carries the live pointer. This stays as the method and the result — re-run
> §5 and append here whenever the renderer changes.

**Status: implemented (flag + guard, console-only), and the test run below has
been done.** Verdict up front, since that is what this document exists to
produce: **culling is still worth it by a wide margin** — a zoomed-in pan on
`big-plate` costs **16.5 ms/frame with it and 912 ms/frame without**. Nothing
has made it redundant, so the §6 "delete the feature" branch does not apply.
The switch stays for the next time the renderer changes.
**Scope:** `index.html` (viewer frontend) only — no Rust, no server, no wire format
**Purpose:** add a single, reversible switch that turns viewport culling off, so
culling's value can be re-measured against later rendering changes (e.g. a
fingerprinting/diffing improvement) without deleting the feature.

---

## 1. What is actually being disabled

The thing people call "the culling" in this viewer is **viewport culling**: rooms
whose bounding box falls outside the current view are given `display: none` so
the browser stops paying per-frame cost for them. It is a *visibility* filter
keyed on the viewBox.

It is **not**:

- polygon simplification / decimation (the only ring simplification lives
  server-side in `areas.rs`, and is a separate concern),
- level-of-detail (dropping labels, merging rooms) — that is listed as **still
  open** in `STRATEGY-BROWSER.md` and has never been implemented,
- anything to do with room identity, fingerprinting, matching, or revisions.

**Consequence worth stating up front:** if the symptom being chased is slowness
on a *fitted* (zoomed-out) view, culling is not the cause and this switch will
not tell you anything. `STRATEGY-BROWSER.md` already records that a fitted view
of a ~5,000-room level paints everything at ~0.5 s+/frame *with culling active*,
because nothing is off-screen to cull. The switch is only informative for the
**zoomed-in pan** case.

## 2. Current implementation (what the switch has to intercept)

Four touch points in `index.html` (line numbers verified against the current
file, which is 3,333 lines — an earlier draft of this document cited anchors
from a version ~900 lines shorter):

| Location | Role |
|---|---|
| `roomBBox(room)` (line 954) | Computes a room's Y-flipped bbox from loop points, never `getBBox` (which would force sync layout). |
| `paintLevel(...)` (line 1014) | When passed a `cullUnits` array, collects one unit per room. Nodes = polygon + holes + label, gathered across both paint passes. |
| `cullZone(zone)` (line 972) | Walks the units, hides/shows against `zone.view` plus a 20%-of-view margin, toggling `display` only when a unit's state actually changes. |
| `scheduleCull(zone)` (line 990) | `requestAnimationFrame` throttle so a burst of wheel/drag events collapses to one cull per frame. |

Entry points: `setViewBox()` (line 948) calls `scheduleCull()`; `renderLevel()`
calls `cullZone()` directly after painting (line 1118). Those are the **only two
callers** — which is what makes guarding inside `cullZone` sufficient.

### The unit shape — flat, not nested

Line 1070 builds each unit as:

```js
units.set(room, { room, ...roomBBox(room), nodes, hidden: false });
```

The bbox is **spread flat onto the unit**, so it is `u.minX` / `u.maxY`, *not*
`u.bbox.minX`. Worth stating precisely because the shorthand `{bbox, nodes}`
appears both in this document's earlier draft and in a comment at line 2861, and
code written from that shorthand would not work.

### The trap — do not disable by skipping unit collection

`zone.cullUnits` is **load-bearing for more than culling**, and for more than
this document originally claimed. There are now **three** consumers:

| Line | Consumer | What breaks without units |
|---|---|---|
| 2612 | `roomAtNode(zone, node)` | **Click-to-select.** Resolves a clicked SVG node to a room via `u.nodes.includes(node)`. Break this and room selection stops working at all — taking the adjacency graph's plan→graph direction with it. |
| 2589 | `applySelection(zone)` | The `.selected` outline on `nodes[0]`. |
| 2566 | `applyHighlight(zone)` | `.match` / `.dim` for the global room search, via `u.room.id` and `nodes[0]`. |

So the switch must **keep populating `cullUnits` exactly as today** and only
suppress the hide/show behaviour. Disabling by passing `cullUnits: null` into
`paintLevel` would silently break selection, click-to-select and search
highlighting at once — regressions that look nothing like a culling change and
would be painful to trace back.

## 3. Proposed change

### 3.1 The flag

Add near the other viewer module-level constants (by `MATCH_COLOUR`, line 747).

`index.html`'s scripts are **classic `<script>` tags, not `type="module"`**
(lines 553–558), so a top-level `let` lands in the global lexical environment
and the devtools console can read and assign it. That is what makes the
console-only recommendation in §3.3 viable at all; under a module it would need
`window.CULL_ENABLED` instead.

```js
// ---- Viewport culling kill switch -------------------------------------------
// `true` (default) = normal behaviour: off-screen rooms are hidden on pan/zoom.
// `false` = every room stays in the render tree regardless of the view.
//
// Kept as a switch rather than deleted code so the culling win can be
// re-measured whenever the renderer changes. Expect deep zoomed-in panning to
// collapse from ~4-15 ms/frame to ~595 ms/frame (~2 fps) on the 10k-room
// `big-plate` fixture when this is false — that number IS the feature's value.
//
// NOTE: this only suppresses hide/show. Cull *units* are still collected,
// because room-search highlighting (`applyHighlight`) walks `zone.cullUnits`.
let CULL_ENABLED = true;
```

`let`, not `const`, so it can be flipped from the devtools console mid-session.

### 3.2 The early exit

Replace the body of `cullZone` with a guarded version. Everything below the
guard is **verbatim the current body** — margin, `off !== u.hidden` test, node
loop — so the whole change is the inserted block:

```js
function cullZone(zone) {
  const units = zone.cullUnits;
  if (!units || units.length === 0) return;

  // Kill switch: restore anything a previous culling pass hid, then do nothing.
  // The restore loop matters — flipping the flag at runtime must not leave rooms
  // stranded with display:none from the last enabled pass.
  if (!CULL_ENABLED) {
    for (const u of units) {
      if (u.hidden) {
        for (const n of u.nodes) n.style.display = "";
        u.hidden = false;
      }
    }
    return;
  }

  const v = zone.view;
  const mx = v.w * 0.2, my = v.h * 0.2; // keep a 20%-of-view margin loaded
  const x0 = v.x - mx, y0 = v.y - my, x1 = v.x + v.w + mx, y1 = v.y + v.h + my;
  for (const u of units) {
    const off = u.maxX < x0 || u.minX > x1 || u.maxY < y0 || u.minY > y1;
    if (off !== u.hidden) {
      const d = off ? "none" : "";
      for (const n of u.nodes) n.style.display = d;
      u.hidden = off;
    }
  }
}
```

Guarding inside `cullZone` (rather than at each call site) means every current
and future caller is covered by one branch — `setViewBox`, `renderLevel`, and
anything added later.

`scheduleCull` needs no change: it still schedules, and the scheduled `cullZone`
becomes a cheap no-op after the first restoring pass.

### 3.3 Optional — a UI toggle

If the test needs to be run by someone who won't open a console, add a button
beside the existing `linkToggle` (line 431). Follow `labelsToggle` (line 432,
listener at line 2972), which is the established pattern for a header toggle —
it carries the `on` class as well as its label, so a button that only swaps text
would render permanently "off":

```html
<button class="ctl on" id="cullToggle" title="Disable viewport culling (debug)">Culling: on</button>
```

```js
// Debug-only control. Flipping the flag alone changes nothing until each zone
// re-culls, so force a pass over every zone immediately.
document.getElementById("cullToggle").addEventListener("click", (e) => {
  CULL_ENABLED = !CULL_ENABLED;
  e.target.textContent = `Culling: ${CULL_ENABLED ? "on" : "off"}`;
  e.target.classList.toggle("on", CULL_ENABLED);
  for (const z of zones) cullZone(z);
});
```

Note this needs no re-render, unlike `labelsToggle` — that one calls
`renderLevel` because label visibility is baked in at paint time, whereas
culling only toggles `display` on existing nodes.

Recommendation: **console-only for the first pass.** A visible button invites
someone to leave it off and file the resulting slowness as a new bug.

## 4. Reverting

Reversion is deliberately trivial, in ascending order of permanence:

1. **Per session** — `CULL_ENABLED = true; zones.forEach(cullZone)` in the console.
2. **Per deployment** — flip the initialiser back to `true`. One character.
3. **Remove the switch entirely** — delete the flag declaration, delete the
   `if (!CULL_ENABLED)` block from `cullZone`, delete the button + listener if
   added. `cullZone` returns to its current body verbatim; nothing else in the
   file references the flag.

No state is persisted (deliberately — the flag is not in `localStorage` and not
in the URL, so a reload always returns to the default). No server, snapshot, or
settings change is involved, so there is no migration and no cross-version
concern.

## 5. Test plan

Use the 10k-room `big-plate` fixture (5,046 rooms/level) — the same one that
produced the numbers in `STRATEGY-BROWSER.md`, so results are comparable.

| # | Check | Expected with `CULL_ENABLED = false` |
|---|---|---|
| 1 | Deep zoomed-in pan, frame time | Regresses toward ~595 ms/frame. If it *doesn't*, that's the interesting result — something else now bounds the cost. |
| 2 | Fitted (zoomed-out) view | Roughly unchanged (~0.5 s+/frame). Culling never helped here. |
| 3 | Room search highlighting | Still works. Guards against the §2 trap. |
| 4 | Flip flag mid-session while zoomed in | Previously-hidden rooms reappear immediately, no stranded `display: none`. |
| 5 | Flip back to `true` | Culling resumes, no stuck-visible rooms. |
| 6 | SVG export | Unchanged, full room count. Export passes no cull-unit array, so it never went through this path either way. |
| 7 | Areas overlay + level switch | Unchanged. |

Measure with the Performance panel over a fixed drag, not by feel — the
zoomed-in delta is two orders of magnitude and should be unmistakable.

### Results — run against `big-plate` L0 (5,046 rooms), 2026-07-25

Measured by driving the viewer from the page context rather than the Performance
panel: a fixed 6-step pan at 20× zoom, timing rAF-to-rAF so each sample spans the
browser's real style/layout/paint work. Same drag, flag flipped between runs.

| # | Check | Result |
|---|---|---|
| 1 | Zoomed-in pan | **16.5 ms/frame on, 912 ms/frame off** — a 20–55× regression (a repeat run measured 43.7 ms on, hence the range). The predicted ~595 ms was if anything optimistic. **Culling is not redundant.** |
| 2 | Fitted view | Not re-measured; the flag cannot affect it (nothing is off-screen to cull) and no code path changed for it. |
| 3 | Room search | Identical on and off: 24 matches, 5,022 dimmed, clears to 0. Also verified `roomAtNode` still resolves a click to its room and `applySelection` still marks it — the two consumers §2 originally missed. |
| 4 | Flip to off while zoomed in | 5,043 hidden → **0 hidden and 0 stranded `display:none` in the DOM**. A further pan while off re-hid nothing. |
| 5 | Flip back to on | Culling resumed immediately (5,004 hidden at the new pan position). |
| 6 | SVG export | 5,764 `<polygon>` elements with culling off — unchanged, as expected: `buildLevelSvgFile` passes no `cullUnits`, so it never took this path. |
| 7 | Areas overlay + level switch | Unchanged; no console errors at any point. |

One measurement note worth keeping: **rAF is throttled in a background tab**, so
a scripted run must front the tab first or every frame time is meaningless.

## 6. Follow-ups this may surface

> **Resolved for this run: test 1 regressed hard (16.5 → 912 ms/frame), so
> culling is not redundant and the delete-the-feature branch below does not
> apply.** Re-run before concluding otherwise; that is what the switch is for.

If test 1 shows the zoomed-in case is now fast *without* culling, culling has
genuinely been made redundant by whatever changed, and the right follow-up is
deleting the feature (per §4.3) plus its `STRATEGY-BROWSER.md` entry — not
leaving a permanent flag in place.

If test 2 remains the real complaint, the open lever is the one already named in
`STRATEGY-BROWSER.md`: level-of-detail (drop labels / merge rooms when the whole
plate is on screen) and capping the grid to the visible region. Neither is
culling, and neither is affected by this switch.

## 7. Docs to update on merge

- `STRATEGY-BROWSER.md` — the "Viewport culling on pan/zoom — implemented"
  bullet should gain a sentence noting the debug switch and pointing here.
- This file — record the measured numbers once the test is run, so the next
  person asking "is culling still worth it?" reads a result rather than a plan.
