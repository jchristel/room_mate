// Screen-space lines: outlines, dashed hole strokes, and the grid.
//
// THIS IS THE HARD PART OF THE WHOLE EXERCISE, and it is not the part the
// handover warns about. Every stroke in the plan today carries
// `vector-effect: non-scaling-stroke` — outlines 1.5px, hole strokes 1px dashed
// `4 3`, grid 0.5px — so all of them hold a constant *screen* width at any zoom.
// That is a visible, load-bearing property: at a fitted view of a 5,000-room
// plate, strokes that scaled with the view would collapse into invisibility, and
// zoomed in they would become slabs.
//
// Nothing in a scene graph gives you that. Under a scaled container a stroke
// scales with everything else, and the obvious fix — rebuild the stroke geometry
// whenever the view changes — is a per-frame rebuild during pan, which is
// exactly the cost this renderer exists to remove.
//
// So each segment is expanded into a quad IN THE VERTEX SHADER, in pixel space,
// after projection. The geometry stores the segment's two endpoints; the shader
// projects both, measures the direction in device pixels, and offsets by the
// pixel normal. Width is per-vertex (a search match strokes at 3px where an
// ordinary room strokes at 1.5px), dashes are per-mesh uniforms.
//
// This is not "hand-rolling what a library solves". Triangulation is earcut's,
// batching and GL plumbing are Pixi's, the spatial index is Flatbush's.
// Non-scaling strokes are a property of THIS plan's appearance and no library
// ships them.

import { Buffer, BufferUsage, Geometry, GlProgram, Mesh, Shader, UniformGroup } from "pixi.js";
import { PROJECTION_GLSL } from "./projection.js";

const VERTEX = /* glsl */ `
in vec2 aP0;        // segment start, world
in vec2 aP1;        // segment end, world
in float aSide;     // -1 or +1: which side of the line this corner sits on
in float aEnd;      // 0 -> at aP0, 1 -> at aP1
in vec4 aColor;
in float aWidthPx;  // FULL stroke width in CSS pixels

${PROJECTION_GLSL}

uniform float uDpr;

out vec4 vColor;
out float vAlongPx;

void main() {
  vec2 p0 = ndcToPx(worldToNdc(aP0));
  vec2 p1 = ndcToPx(worldToNdc(aP1));

  vec2 d = p1 - p0;
  // A zero-length segment has no direction to normalise. Degenerate input is
  // real -- duplicate consecutive points arrive from Revit -- and normalising
  // it yields NaN, which drops the whole draw call rather than one quad.
  float len = length(d);
  vec2 dir = len > 1e-6 ? d / len : vec2(1.0, 0.0);
  vec2 nrm = vec2(-dir.y, dir.x);

  vec2 basePx = mix(p0, p1, aEnd);
  vec2 offsetPx = nrm * (aWidthPx * uDpr * 0.5) * aSide;

  gl_Position = vec4(pxToNdc(basePx + offsetPx), 0.0, 1.0);
  vColor = aColor;
  // Distance along the segment, in device pixels, for the dash pattern. Phase
  // resets per segment rather than accumulating along the ring: at the corner
  // of a room that is what a reader sees anyway, and carrying a running total
  // would make the dash depend on where the ring happened to start.
  vAlongPx = aEnd * len;
}
`;

const FRAGMENT = /* glsl */ `
in vec4 vColor;
in float vAlongPx;

// Both already in DEVICE pixels, scaled by DPR on the CPU at setView time.
//
// That is not a micro-optimisation. uDpr was originally declared in both
// stages, and Pixi gives the vertex and fragment shaders different default
// precisions -- so the program failed to LINK with "Precisions of uniform
// 'uDpr' differ between VERTEX and FRAGMENT shaders", and every attribute then
// reported as missing. Keeping each uniform in exactly one stage avoids the
// whole class of problem.
uniform float uDashPeriodPx; // 0 disables dashing
uniform float uDashOnPx;

out vec4 finalColor;

void main() {
  if (uDashPeriodPx > 0.0) {
    if (mod(vAlongPx, uDashPeriodPx) > uDashOnPx) discard;
  }
  // Premultiplied, which is what Pixi's default blend state expects.
  finalColor = vec4(vColor.rgb * vColor.a, vColor.a);
}
`;

/** Floats per vertex: p0(2) p1(2) side(1) end(1) colour(4) width(1). */
const STRIDE = 11;
/** Four corners per segment, wound as two triangles. */
const CORNERS: ReadonlyArray<readonly [side: number, end: number]> = [
  [-1, 0],
  [1, 0],
  [1, 1],
  [-1, 1],
];

export interface LineStyle {
  /** Dash length in CSS px, and gap. Omit for a solid stroke. */
  dash?: readonly [on: number, off: number] | undefined;
}

export interface Segment {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

/**
 * Accumulates segments, then bakes them into one Mesh.
 *
 * Deliberately one mesh per STYLE (grid, outlines, holes) rather than one per
 * room: a draw call per room at plate scale is the Canvas2D failure mode with
 * extra steps.
 */
export class LineBatch {
  readonly #verts: number[] = [];
  readonly #indices: number[] = [];
  #vertexCount = 0;

  /** Vertex range for a room, so alpha/colour can be rewritten in place when
   *  search state changes — without rebuilding geometry. */
  push(
    segments: readonly Segment[],
    colour: readonly [number, number, number, number],
    widthPx: number,
  ): { start: number; count: number } {
    const start = this.#vertexCount;
    const [r, g, b, a] = colour;
    for (const s of segments) {
      const base = this.#vertexCount;
      for (const [side, end] of CORNERS)
        this.#verts.push(s.x0, s.y0, s.x1, s.y1, side, end, r, g, b, a, widthPx);
      this.#indices.push(base, base + 1, base + 2, base, base + 2, base + 3);
      this.#vertexCount += 4;
    }
    return { start, count: this.#vertexCount - start };
  }

  get isEmpty(): boolean {
    return this.#vertexCount === 0;
  }

  build(style: LineStyle = {}): LineMesh {
    const data = new Float32Array(this.#verts);
    const buffer = new Buffer({ data, usage: BufferUsage.VERTEX | BufferUsage.COPY_DST });
    const geometry = new Geometry({
      attributes: {
        aP0: { buffer, format: "float32x2", stride: STRIDE * 4, offset: 0 },
        aP1: { buffer, format: "float32x2", stride: STRIDE * 4, offset: 2 * 4 },
        aSide: { buffer, format: "float32", stride: STRIDE * 4, offset: 4 * 4 },
        aEnd: { buffer, format: "float32", stride: STRIDE * 4, offset: 5 * 4 },
        aColor: { buffer, format: "float32x4", stride: STRIDE * 4, offset: 6 * 4 },
        aWidthPx: { buffer, format: "float32", stride: STRIDE * 4, offset: 10 * 4 },
      },
      indexBuffer: new Uint32Array(this.#indices),
    });

    const [on, off] = style.dash ?? [0, 0];
    const uniforms = new UniformGroup({
      uView: { value: new Float32Array([0, 0, 1, 1]), type: "vec4<f32>" },
      uPxSize: { value: new Float32Array([1, 1]), type: "vec2<f32>" },
      uDpr: { value: 1, type: "f32" },
      uDashPeriodPx: { value: on > 0 ? on + off : 0, type: "f32" },
      uDashOnPx: { value: on, type: "f32" },
    });

    const shader = new Shader({
      glProgram: GlProgram.from({ vertex: VERTEX, fragment: FRAGMENT, name: "plan-lines" }),
      resources: { planLines: uniforms },
    });

    return new LineMesh(geometry, shader, data, buffer, on, on + off);
  }
}

/** A built line batch: the Pixi mesh plus the handles needed to rewrite vertex
 *  colours in place. */
export class LineMesh {
  // `Mesh<Geometry, Shader>`, not the bare `Mesh`: the default type parameters
  // are MeshGeometry/TextureShader, which assume a textured quad. This mesh has
  // neither -- its geometry is custom attributes and its shader samples nothing.
  readonly mesh: Mesh<Geometry, Shader>;
  readonly #data: Float32Array;
  readonly #buffer: Buffer;
  readonly #uniforms: Record<string, unknown>;

  readonly #dashOnCss: number;
  readonly #dashPeriodCss: number;

  constructor(
    geometry: Geometry,
    shader: Shader,
    data: Float32Array,
    buffer: Buffer,
    dashOnCss: number,
    dashPeriodCss: number,
  ) {
    this.mesh = new Mesh({ geometry, shader });
    this.#data = data;
    this.#buffer = buffer;
    this.#dashOnCss = dashOnCss;
    this.#dashPeriodCss = dashPeriodCss;
    this.#uniforms = (shader.resources["planLines"] as UniformGroup).uniforms as Record<
      string,
      unknown
    >;
  }

  setView(view: { x: number; y: number; w: number; h: number }, pxW: number, pxH: number, dpr: number): void {
    const v = this.#uniforms["uView"] as Float32Array;
    v[0] = view.x;
    v[1] = view.y;
    v[2] = view.w;
    v[3] = view.h;
    const p = this.#uniforms["uPxSize"] as Float32Array;
    p[0] = pxW;
    p[1] = pxH;
    this.#uniforms["uDpr"] = dpr;
    // Dash lengths are resolved to device pixels here rather than in the
    // fragment shader, so `uDpr` stays a vertex-only uniform. See FRAGMENT.
    this.#uniforms["uDashOnPx"] = this.#dashOnCss * dpr;
    this.#uniforms["uDashPeriodPx"] = this.#dashPeriodCss > 0 ? this.#dashPeriodCss * dpr : 0;
  }

  /** Rewrite the colour and width of a vertex range, then mark the buffer
   *  dirty. This is the search-highlight fast path: a keystroke touches some
   *  floats, never the geometry. */
  restyle(
    range: { start: number; count: number },
    colour: readonly [number, number, number, number],
    widthPx: number,
  ): void {
    for (let i = 0; i < range.count; i++) {
      const o = (range.start + i) * STRIDE;
      this.#data[o + 6] = colour[0];
      this.#data[o + 7] = colour[1];
      this.#data[o + 8] = colour[2];
      this.#data[o + 9] = colour[3];
      this.#data[o + 10] = widthPx;
    }
  }

  commit(): void {
    this.#buffer.update();
  }

  destroy(): void {
    this.mesh.destroy({ children: true });
  }
}

/** A closed ring's segments, in the flipped space the renderer draws in. */
export function ringSegments(points: readonly { x: number; y: number }[]): Segment[] {
  const out: Segment[] = [];
  for (let i = 0; i < points.length; i++) {
    const a = points[i]!;
    const b = points[(i + 1) % points.length]!;
    out.push({ x0: a.x, y0: a.y, x1: b.x, y1: b.y });
  }
  return out;
}
