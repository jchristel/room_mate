// Where a 3D view's camera looks, on the plan and from the side: the heading a person gave it, the view turned to it at
// its own pitch, and the extent of its section box as it then sees it. Pure, tested in sheetPreviews.test.ts.

import type { RlsPlan, ThreeDCamera } from "./types.js";

/** The headings a person has set and not saved, by 3D view index (from 1): unit vectors in the model's frame. */
export type Aims = ReadonlyMap<number, [number, number]>;

const unit2 = (d: readonly [number, number]): [number, number] | null => {
  const len = Math.hypot(d[0], d[1]);
  return len > 1e-9 && Number.isFinite(len) ? [d[0] / len, d[1] / len] : null;
};

/** A camera's heading in plan (a unit vector): the one a person has set, else the one the task holds, else from its eye
 * to what it looks at. Null for a camera standing over what it looks at. */
export function headingOf(cam: ThreeDCamera, aims?: Aims): [number, number] | null {
  const chosen = aims?.get(cam.index) ?? cam.direction;
  return unit2(chosen ?? [cam.looks_at[0] - cam.eye[0], cam.looks_at[1] - cam.eye[1]]);
}

/**
 * `forward` and `up` looking along `heading` in plan at their own pitch, the up vector turned with them
 * (`room_m.apply_geometry.aim_xy`, which the plug-in runs). A view looking straight down is given 45 degrees.
 */
export function aimed(forward: Vec, up: Vec, heading: [number, number]): [Vec, Vec] {
  const plan = Math.hypot(forward[0], forward[1]);
  const pitch = plan > 1e-6 ? plan : Math.abs(forward[2]) || 1;
  const fz = plan > 1e-6 || forward[2] ? forward[2] : -1;
  const now = plan > 1e-6 ? Math.atan2(forward[1], forward[0]) : Math.atan2(heading[1], heading[0]);
  const turn = Math.atan2(heading[1], heading[0]) - now;
  const [c, s] = [Math.cos(turn), Math.sin(turn)];
  return [
    [heading[0] * pitch, heading[1] * pitch, fz],
    [up[0] * c - up[1] * s, up[0] * s + up[1] * c, up[2]],
  ];
}

/** How far back from what it looks at a camera's arrow starts on the plan: inside the plan's crop, so both grips are reachable. */
export function arrowLength(rls: RlsPlan): number {
  const c = rls.plan_crop;
  return 0.45 * (c ? Math.hypot(c.max[0] - c.min[0], c.max[1] - c.min[1]) : 6000);
}

export type Vec = [number, number, number];

export const unit = (v: Vec): Vec => {
  const n = Math.hypot(...v) || 1;
  return [v[0] / n, v[1] / n, v[2] / n];
};

/**
 * How wide and tall (model mm) a box of `size` looks from a camera with these directions: the extent of its eight
 * corners along the view's right and up, which is what the plug-in crops the new 3D view to (duHast `_crop_to_box`).
 * Only the box's size matters, not where it stands, and a plan turned with its box and camera gives the same answer.
 */
export function boxExtent(forward: Vec, up: Vec, size: Vec): [number, number] {
  const f = unit(forward);
  const u = unit(up);
  const r: Vec = [f[1] * u[2] - f[2] * u[1], f[2] * u[0] - f[0] * u[2], f[0] * u[1] - f[1] * u[0]];
  const along = (axis: Vec) => Math.abs(axis[0]) * size[0] + Math.abs(axis[1]) * size[1] + Math.abs(axis[2]) * size[2];
  return [along(r), along(u)];
}
