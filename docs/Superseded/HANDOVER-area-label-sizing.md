# Handover — area label sizing in hierarchical area views

**Status:** reviewed and specified, **not yet applied**. Two open decisions were
resolved by the user; the code below is ready to paste.

**File touched:** `index.html` (browser code only — no Rust change).

---

## Problem

Tier labels drawn on the hierarchical area overlay overflow the polygons they
name. Small tier footprints get the same text size as whole-floor ones.

## Cause

`renderAreasOverlay()` sets a flat font size for every group:

```js
const baseFont = Math.max(zone.fitted.w, zone.fitted.h) * 0.022;
// ...
label.setAttribute("font-size", baseFont);
```

`baseFont` derives from the level's fitted bounds, so it is constant across all
groups on that level regardless of each group's ring size.

By contrast `addLabel()` (room labels) already fits text to the polygon bbox:

```js
const widthLimited  = (box.w * 0.9) / longest / 0.6;   // 0.6 ≈ mono glyph aspect
const heightLimited = (box.h * 0.8) / (1 + 0.7 * (n - 1));
const fontSize = Math.min(baseFont, widthLimited, heightLimited);
```

The fix is to apply the same idea to tier labels, with two adjustments for the
ways dissolved footprints differ from room loops.

---

## Change 1 — new helper

Place next to `ringAreaAbs` / `ringCentroid` in `index.html`.

```js
// Axis-aligned bbox of a footprint ring ([[x,y],…], model coords). Used to size
// the tier label against the polygon it names. Note this OVERSTATES the usable
// interior for L-shaped or concave dissolved footprints — hence the deliberately
// conservative 0.7 width factor at the call site rather than addLabel()'s 0.9.
function ringBox(ring) {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const [x, y] of ring) {
    if (x < x0) x0 = x;
    if (x > x1) x1 = x;
    if (y < y0) y0 = y;
    if (y > y1) y1 = y;
  }
  return { w: x1 - x0, h: y1 - y0 };
}
```

## Change 2 — label block in `renderAreasOverlay`

Replaces the existing `if (big && big.length >= 3) { … }` block.

```js
    const big = grp.rings.reduce((a, b) => (b && ringAreaAbs(b) > ringAreaAbs(a || []) ? b : a), grp.rings[0]);
    if (big && big.length >= 3) {
      const text = tierLabel(grp.path[depth]);
      // Fit the label to its ring, mirroring addLabel()'s room-label sizing:
      // baseFont is the ceiling, the polygon's own bbox the constraint. 0.6 is
      // the mono glyph aspect ratio; 0.7 (vs 0.9 for rooms) leaves margin for
      // the concave footprints a bbox can't see.
      const bb = ringBox(big);
      const widthLimited = (bb.w * 0.7) / Math.max(text.length, 1) / 0.6;
      const heightLimited = bb.h * 0.8;
      const fontSize = Math.min(baseFont, widthLimited, heightLimited);
      // Groups too small to carry legible text get no label at all — a
      // sub-pixel speck is worse than nothing, and the summary panel already
      // names every group.
      if (fontSize >= baseFont * 0.25) {
        const c = ringCentroid(big);
        const label = document.createElementNS(SVG, "text");
        label.setAttribute("class", "area-label");
        label.setAttribute("x", c.x);
        label.setAttribute("y", -c.y);
        label.setAttribute("font-size", fontSize);
        label.textContent = text;
        g.appendChild(label);
      }
    }
```

---

## Decisions taken

**1. Width factor 0.7, not addLabel's 0.9.** A bbox overstates usable interior
for an L-shaped or concave dissolved footprint, so a label can fit the bbox and
still spill outside the polygon. Room outer loops are usually convex-ish and
rarely hit this; tier footprints are not. 0.7 buys margin cheaply. If overflow
still shows up on badly-shaped groups, the real fix is sizing against the
largest inscribed axis-aligned rectangle rather than shrinking the factor
further.

**2. Skip the label below `baseFont * 0.25`.** Very small groups would otherwise
get unreadable specks. The summary panel already lists every group by name, so
nothing is lost by omitting the label.

Note the guard is `>= baseFont * 0.25`, not `> 0` — that subsumes the
degenerate-ring case, so no separate zero check is needed.

---

## Known consequence — labels do not respond to zoom

`baseFont` comes from `zone.fitted` (the level's fitted bounds), not
`zone.view` (the current pan/zoom frame). The suppression threshold is therefore
fixed per level: **a group suppressed at floor scale stays unlabelled however far
you zoom in.**

This is acceptable as specified, but if labels should reappear on zoom:

- `renderAreasOverlay` would need calling from the pan/zoom path, with
  `baseFont` computed from `zone.view` instead of `zone.fitted`.
- That re-runs the overlay render on every wheel event, so it needs throttling —
  compare how `cullZone` is driven from the same path.
- Larger change than the above; deliberately out of scope here.

---

## Context for whoever picks this up

- `renderAreasOverlay` is called at the end of `renderLevel`, which is the single
  choke point every re-render passes through (poll, colour plan, validation,
  level switch). That is why the overlay survives the `replaceChildren()` those
  paths do, and why no caller needs to know about the footprints.
- It is also called directly from the areas toggle and tier `change` handlers,
  and from `loadAreas`.
- Footprint rings are `[[x, y], …]` in model coords with Y **up**; the SVG space
  is Y **down**. Hence `-c.y` on the label and the negation inside
  `ringPointsAttr`. `ringBox` returns width/height only, so the flip does not
  affect it.
- `tierLabel(t)` yields `undefined <tier>` for undefined tiers, else
  `name || code || tier`. Label length varies a lot between tiers, which is
  exactly why width-limiting matters here.

## Testing

No Rust tests are affected. Check manually:

- A level with mixed group sizes at several tiers — labels fit, small groups
  unlabelled.
- An L-shaped or courtyard-style dissolved footprint — label stays inside.
- Toggle areas off/on and switch tier and level — overlay rebuilds correctly.
- SVG export path (`exportZoneLevels`) is unaffected; the overlay is not
  included in exports. Confirm that is still the intent.
