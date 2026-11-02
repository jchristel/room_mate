// Room fills: one triangulated mesh for the whole level.
//
// One mesh, not one per room. At `big-plate` scale that is the difference
// between 3 draw calls and 5,046 of them, and a draw call per object is the
// Canvas2D failure mode with extra steps.
//
// HOLES ARE REAL HOLES here, not the paint-over trick SVG uses. earcut takes
// hole indices directly, so a courtyard becomes an actual void in the
// triangulation rather than a `--paper` polygon drawn on top. That is a small
// visible improvement as well as less geometry: under `.dim` (opacity 0.15) the
// SVG version stacks 15%-opacity paper over a 15%-opacity room and reads muddy,
// where a true void just shows the page. The dashed hole STROKE is drawn
// separately and is unaffected — it is a feature, not an artefact of how the
// fill was faked.

import { Buffer, BufferUsage, Geometry, GlProgram, Mesh, Shader, UniformGroup } from "pixi.js";
import earcut from "earcut";
import type { Loop, Room } from "../types.js";
import { flip } from "../geometry.js";
import { PROJECTION_GLSL } from "./projection.js";

const VERTEX = /* glsl */ `
in vec2 aPosition;
in vec4 aColor;

${PROJECTION_GLSL}

out vec4 vColor;

void main() {
  gl_Position = vec4(worldToNdc(aPosition), 0.0, 1.0);
  vColor = aColor;
}
`;

const FRAGMENT = /* glsl */ `
in vec4 vColor;

// Areas mode ghosts every room beneath the footprint overlay. A CROSS-LAYER
// rule: the overlay is SVG, the rooms are GL, and with the two in different
// technologies the CSS that used to express it cannot reach here. Easy to
// miss; visibly wrong when missed.
uniform highp float uLayerAlpha;

out vec4 finalColor;

void main() {
  float a = vColor.a * uLayerAlpha;
  finalColor = vec4(vColor.rgb * a, a); // premultiplied, as Pixi's blend expects
}
`;

/** Floats per vertex: pos(2) colour(4). */
const STRIDE = 6;

/** Where one room's vertices live, so its colour can be rewritten in place. */
export interface VertexRange {
  start: number;
  count: number;
}

export type Rgba = readonly [number, number, number, number];

/** Triangulate one room, holes included, in the flipped space. */
function triangulate(loops: readonly Loop[]): { coords: number[]; indices: number[] } {
  const coords: number[] = [];
  const holeStarts: number[] = [];
  loops.forEach((loop, i) => {
    // earcut wants hole indices in VERTICES, not floats.
    if (i > 0) holeStarts.push(coords.length / 2);
    for (const raw of loop.points) {
      const p = flip(raw);
      coords.push(p.x, p.y);
    }
  });
  return { coords, indices: earcut(coords, holeStarts) };
}

export class FillBatch {
  readonly #verts: number[] = [];
  readonly #indices: number[] = [];
  #vertexCount = 0;

  push(room: Room, colour: Rgba): VertexRange | null {
    const loops = room.loops;
    if (!loops || !loops[0]) return null;

    const { coords, indices } = triangulate(loops);
    // A ring that cannot be triangulated (collinear points, or the ±1e30
    // uninitialised bounding box duHast emits for a door family with no 3D
    // geometry) yields no triangles. Skipping it is right: it is a reported
    // state, not an error, and drawing degenerate geometry is worse than
    // drawing none.
    if (indices.length === 0) return null;

    const start = this.#vertexCount;
    const [r, g, b, a] = colour;
    for (let i = 0; i < coords.length; i += 2)
      this.#verts.push(coords[i]!, coords[i + 1]!, r, g, b, a);
    for (const idx of indices) this.#indices.push(start + idx);
    this.#vertexCount += coords.length / 2;
    return { start, count: this.#vertexCount - start };
  }

  /**
   * Append pre-built triangles — a flat `[x0,y0, x1,y1, …]` run of independent
   * triangles, already in flipped space.
   *
   * This is what lets the door glyphs share the rooms' buffer instead of adding
   * a mesh (and so a draw call) per entity type. They arrive as triangles
   * rather than as loops because a glyph is not a polygon: an arrow with a
   * chevron tail is several disjoint pieces, and asking earcut to find them in
   * one ring would be inventing a shape to fit the tool.
   *
   * Returns a `VertexRange` on the same terms as `push`, so a glyph can be
   * recoloured in place by the same fast path a room uses.
   */
  pushTriangles(coords: readonly number[], colour: Rgba): VertexRange | null {
    if (coords.length < 6) return null;
    const start = this.#vertexCount;
    const [r, g, b, a] = colour;
    for (let i = 0; i < coords.length; i += 2) {
      this.#verts.push(coords[i]!, coords[i + 1]!, r, g, b, a);
      this.#indices.push(this.#vertexCount);
      this.#vertexCount++;
    }
    return { start, count: this.#vertexCount - start };
  }

  get isEmpty(): boolean {
    return this.#indices.length === 0;
  }

  build(): FillMesh {
    const data = new Float32Array(this.#verts);
    const buffer = new Buffer({ data, usage: BufferUsage.VERTEX | BufferUsage.COPY_DST });
    const geometry = new Geometry({
      attributes: {
        aPosition: { buffer, format: "float32x2", stride: STRIDE * 4, offset: 0 },
        aColor: { buffer, format: "float32x4", stride: STRIDE * 4, offset: 2 * 4 },
      },
      indexBuffer: new Uint32Array(this.#indices),
    });

    const uniforms = new UniformGroup({
      uView: { value: new Float32Array([0, 0, 1, 1]), type: "vec4<f32>" },
      uPxSize: { value: new Float32Array([1, 1]), type: "vec2<f32>" },
      uLayerAlpha: { value: 1, type: "f32" },
    });

    const shader = new Shader({
      // Both stages at highp for the same reason as lines.ts: Pixi declares the
      // whole uniform group in both, and mismatched default precisions fail the
      // link.
      glProgram: GlProgram.from({
        vertex: VERTEX,
        fragment: FRAGMENT,
        name: "plan-fills",
        preferredVertexPrecision: "highp",
        preferredFragmentPrecision: "highp",
      }),
      resources: { planFills: uniforms },
    });

    return new FillMesh(geometry, shader, data, buffer);
  }
}

export class FillMesh {
  readonly mesh: Mesh<Geometry, Shader>;
  readonly #data: Float32Array;
  readonly #buffer: Buffer;
  readonly #uniforms: Record<string, unknown>;

  constructor(geometry: Geometry, shader: Shader, data: Float32Array, buffer: Buffer) {
    this.mesh = new Mesh({ geometry, shader });
    this.#data = data;
    this.#buffer = buffer;
    this.#uniforms = (shader.resources["planFills"] as UniformGroup).uniforms as Record<
      string,
      unknown
    >;
  }

  setView(view: { x: number; y: number; w: number; h: number }, pxW: number, pxH: number): void {
    const v = this.#uniforms["uView"] as Float32Array;
    v[0] = view.x;
    v[1] = view.y;
    v[2] = view.w;
    v[3] = view.h;
    const p = this.#uniforms["uPxSize"] as Float32Array;
    p[0] = pxW;
    p[1] = pxH;
  }

  setLayerAlpha(a: number): void {
    this.#uniforms["uLayerAlpha"] = a;
  }

  /** THE search fast path. A keystroke rewrites some floats; it never rebuilds
   *  geometry and never re-uploads a level. */
  recolour(range: VertexRange, colour: Rgba): void {
    for (let i = 0; i < range.count; i++) {
      const o = (range.start + i) * STRIDE + 2;
      this.#data[o] = colour[0];
      this.#data[o + 1] = colour[1];
      this.#data[o + 2] = colour[2];
      this.#data[o + 3] = colour[3];
    }
  }

  commit(): void {
    this.#buffer.update();
  }

  destroy(): void {
    this.mesh.destroy({ children: true });
  }
}
