// The bundle's single public surface — the one thing `static/index.html` may
// reach for. Everything else in `src-js/` is internal to the build.
//
// P0 deliberately ships this almost empty. The phase's job is to prove the
// pipeline end to end (TypeScript compiles, Vite emits an IIFE, ServeDir serves
// it, the page can see the global) with no behaviour attached, so that when P1
// moves ~600 lines of real rendering code through it, a failure is unambiguously
// in the code and not in the toolchain. See docs/PLAN-webgl-renderer.md.

/** Build stamp, so a stale committed bundle is visible from the console rather
 *  than inferred from behaviour. CI gates on a rebuild-and-diff, but a human
 *  debugging a checkout wants to ask the page directly. */
export const version = "0.0.0-p0";

/** Proof the pipeline is live, called by nothing. Removed in P1, when this
 *  module starts exporting the renderer seam it exists for. */
export function selfTest(): string {
  return `PlanRenderer ${version}`;
}
