# HANDOVER — Door Direction Glyph (all-WebGL)

**Status:** design settled, not yet built. This is a brief for a self-contained
viewer feature: draw each door as a directional glyph inside the existing WebGL
scene and make it clickable like a room. No service work; the one data-layer ask
is a single new field from the exporter (see "The exporter change"). Written as a
brief and kept as one.

**Goal:** render a door as a rectangle (its bounding box) with an embedded arrow
running through the wall, pointing from-room → to-room. The user can click the
glyph and have it select like a room.

---

## Scope decisions (settled — do not re-litigate without saying so)

| Question | Decision | Why |
|---|---|---|
| WebGL or hybrid overlay? | **All WebGL.** The whole glyph — rectangle and arrow — lives in the same scene as the room polygons. | Performance, and it is genuinely simpler here: one context, one camera transform, no overlay-sync discipline, and picking stays uniform with rooms. The hybrid canvas overlay stays for what it is already doing; the door does not join it. |
| Arrow tail shape? | **Chevron, not a circle.** | A circle needs a triangle fan (or a template to translate/scale). A chevron is all triangles, reads as direction just as clearly, and costs almost nothing. Drop the circle. |
| How is the glyph oriented? | **From an exporter-supplied wall vector**, not by the viewer re-deriving which room boundary the door sits on. | The exporter already knows the host wall exactly. Re-deriving it is a nearest-segment search that breaks precisely where it is least affordable — curved walls, doors in shared walls between two rooms, thick walls where the insertion point sits off the polygon edge, openings on no room boundary. |
| How is the glyph picked? | **Point-in-rectangle**, the same hit-testing path rooms already use. | Exact for a rectangle, cheap, and no offscreen `readPixels`. Colour/ID picking stays in reserve for if rotated or overlapping doors ever make the CPU test fiddly. |

---

## The exporter change (the one prerequisite)

Ask the exporter to emit, per door, a **through-wall normal**: a unit vector
pointing from the from-room to the to-room, i.e. through the wall the door is
hosted in.

This is the whole reason the viewer needs no rotation logic. With the normal in
hand the arrow points *along that vector directly* — there is no ±90° question,
no `atan2`, no trig at all. The alternative (exporting the wall *tangent*, the
direction along the wall) would leave the viewer to rotate 90° and then decide
the sign of that rotation, which is exactly the ambiguity the exported normal
removes.

One thing to nail down and then keep consistent forever: the vector is the
**normal** (through the swing, room-to-room), not the tangent (along the wall).
The tangent is only worth exporting as well if the rectangle's long side needs to
be sized to the wall's run; the wall thickness gives the short side. Neither is
needed for orientation.

If the field is absent on a door (older exports), the glyph should degrade
gracefully — draw the rectangle, skip the arrow — rather than guess a direction.
A guessed arrow is worse than no arrow.

---

## How the glyph is built

Everything below is **baked once when the door set changes**, not per frame.
Pan and zoom are handled entirely by the shared camera transform, exactly as for
rooms.

- **Bake orientation at build time.** Rotate the glyph's vertices into place using
  the exported normal as you write them into the buffer. Do *not* orient per-door
  with a uniform or a matrix switch at draw time — that would fragment the batch.

- **Append into the room vertex stream.** The glyph is a handful of triangles:
  rectangle (two), arrow shaft (one quad), arrowhead (one triangle), chevron tail
  (two triangles). Roughly fifteen to twenty triangles per door — negligible — and
  because they go into the same buffer the rooms already fill, it stays one draw
  call.

- **Rebuild only on change.** Same lifecycle as the room geometry. There is no
  per-frame glyph work.

At any realistic door count this is not a measurable cost. The circle was the only
part with a real price attached, and dropping it for the chevron removes it.

### Degraded glyphs

The glyph has three inputs — a bounding box, and a through-wall normal — and each
can be missing or bad. The rule is the same throughout: draw what the data
supports, never invent the part that is missing.

- **Bounding box smaller than a threshold (invalid/degenerate box):** draw **just
  the arrow**, no rectangle. A box below the threshold is not a real footprint, and
  a sub-pixel or zero-area rectangle reads as a rendering fault rather than a door.
  The arrow still carries the useful information — direction — so keep it and drop
  the frame. The threshold is a named constant with a doc comment saying what it is
  and why (world units, roughly the smallest box that reads as a rectangle
  on-screen at normal zoom).

- **No from/to room direction (no normal):** if the door has a valid bounding
  box, draw **just the rectangle** — the door's footprint is real, only its
  direction is unknown, so show the frame and omit the arrow. Draw the **cross only
  when neither input is usable** (no valid box *and* no normal): that is the true
  "door exists, nothing else known" marker — deliberately not an arrow (nothing to
  point) and deliberately a fixed arbitrary size (a placeholder, not a
  measurement).

The cases combine cleanly:

| Bounding box | Normal | Glyph |
|---|---|---|
| valid | present | full glyph — rectangle + arrow |
| valid | missing | rectangle only |
| below threshold | present | arrow only |
| below threshold | missing | cross at the insertion point |

---

## How the glyph is picked

The door's **bounding rectangle is the hit target**. Transform the click into
world space and run the same point-in-rectangle test the room selection already
uses. A door then selects exactly like a room, through the same path, with no new
picking machinery.

Reach for colour/ID picking (render each pickable object in a unique flat colour
to an offscreen framebuffer, read back the pixel under the cursor, map colour →
id) only if rotated or overlapping doors ever make the geometric test unreliable.
For an axis-aligned-in-its-own-frame rectangle it is not needed.

For the degraded glyphs there is no bounding box to test against, so the hit
target is a **fixed-size square around the insertion point** — sized to the cross,
or to the arrow's extent, whichever the door drew. The door stays clickable
whichever glyph it received; the pick target is defined by what was drawn, not by
the (missing or invalid) box.

---

## Why this is simpler than the hybrid, not harder

The instinct is that staying in WebGL means more work than dropping the arrow onto
the 2D canvas overlay. Here it is the reverse, because the infrastructure the glyph
needs already exists: the vertex buffer, the shared camera transform, and the
point-in-rectangle hit-testing are all in place for rooms. The glyph reuses all
three. The hybrid path would instead add a second coordinate system to keep in
sync every frame and a second surface to reason about for pointer events. The only
discipline all-WebGL asks in return is baking orientation at build time so the
whole scene stays in one batch — which is the same discipline the room geometry
already follows.

---

## Definition of done

1. Exporter emits a per-door through-wall normal (from-room → to-room), documented
   as a normal, not a tangent.
2. Each door renders in the WebGL scene as a rectangle with an embedded arrow —
   pointed head, chevron tail — pointing along that normal.
3. The glyph is baked into the shared vertex buffer on door-set change and draws in
   the existing batch; no per-frame glyph work, no second draw call added per door.
4. Clicking a door selects it through the same point-in-rectangle path rooms use;
   pan/zoom and existing room selection are undisturbed. Degraded glyphs that have
   no box are clickable via a fixed-size square around the insertion point.
5. A door whose bounding box is below the size threshold draws the arrow only, no
   rectangle. If it also has no normal, it draws a fixed-size cross at the insertion
   point.
6. A door with a valid box but no from/to room direction draws the rectangle only —
   not an arrow, not a cross.

---

## Docs to update on landing

A change touching the renderer and the export contract updates the docs it
touches:

- The browser strategy doc — record the door glyph as an all-WebGL feature and the
  decision against the canvas overlay for it, with the reason (batch + shared
  transform + uniform picking).
- The sources/exporter doc — record the new per-door through-wall normal field and
  the normal-not-tangent convention.
- The docs index — move this handover to the superseded folder once it lands.
