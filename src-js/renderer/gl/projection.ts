// The world -> clip projection, shared by every shader in the GL renderer.
//
// This is the whole reason pan and zoom cost nothing: the view is a UNIFORM, so
// changing it rewrites four floats rather than touching a vertex buffer. It is
// also why Pixi's container transform is deliberately NOT used — the moment the
// scene graph applies a scale, screen-space stroke widths stop being expressible
// (see lines.ts).
//
// The Y flip stays exactly where it has always been, in the geometry: payload
// data is Y-up, `flip()` makes it Y-down, and `zone.view`, `fittedBounds`,
// `roomBBox`, the cull and the areas overlay all live in that Y-down space. GL
// clip space is Y-up, so it is tempting to drop the flip and let the projection
// undo it. Don't — that would silently move five subsystems. The projection
// inverts Y here instead, which confines the whole difference to one line.

/** GLSL, injected into every vertex shader. `uView` is (x, y, w, h) in the
 *  flipped world space — the same four numbers as the SVG viewBox. */
export const PROJECTION_GLSL = /* glsl */ `
uniform vec4 uView;    // x, y, w, h  -- flipped world space
uniform vec2 uPxSize;  // drawing-buffer size in DEVICE pixels

vec2 worldToNdc(vec2 world) {
  float nx = (world.x - uView.x) / uView.z * 2.0 - 1.0;
  // 1.0 - t, not t - 1.0: world Y grows downward, clip Y grows upward.
  float ny = 1.0 - (world.y - uView.y) / uView.w * 2.0;
  return vec2(nx, ny);
}

vec2 ndcToPx(vec2 ndc) { return ndc * 0.5 * uPxSize; }
vec2 pxToNdc(vec2 px)  { return px / (0.5 * uPxSize); }
`;
