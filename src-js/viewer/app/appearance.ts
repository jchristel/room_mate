// The project's `[appearance]`: what colour each layer draws in, and the
// selection colours the stylesheet reads.
//
// Fetched from the settings API once per project change — the viewer's one
// read-only use of it. The RESOLVING route, not the strict one the editors use:
// the viewer holds the payload's project id, which is not necessarily a
// settings `project_id`, so it needs the server's default-fallback to find the
// file that carries this project's appearance.

import { fetchJson } from "./api.js";
import { getState, setState } from "./store.js";
import type { PlanAppearance } from "../../renderer/seam.js";
import type { ColourPlan } from "../colour.js";

/**
 * `[appearance]` as the PAGE reads it: the renderer's paint colours plus the
 * `selection` each entity may name.
 *
 * The renderer's own type has no `selection`, and correctly — it paints fills
 * and lines, while a selection ring is an SVG element in the marks overlay
 * that the stylesheet colours. The two live in one settings block because they
 * are one project decision, so the page widens the type rather than fetching
 * the same block twice.
 */
type WithSelection<T> = T & { selection?: string | null };
export interface ViewerAppearance extends PlanAppearance {
  rooms?: WithSelection<NonNullable<PlanAppearance["rooms"]>> | undefined;
  doors?: WithSelection<NonNullable<PlanAppearance["doors"]>> | undefined;
  windows?: WithSelection<NonNullable<PlanAppearance["windows"]>> | undefined;
  ffe?: WithSelection<NonNullable<PlanAppearance["ffe"]>> | undefined;
  spaces?: WithSelection<NonNullable<PlanAppearance["spaces"]>> | undefined;
  ceilings?: WithSelection<NonNullable<PlanAppearance["ceilings"]>> | undefined;
  floors?: WithSelection<NonNullable<PlanAppearance["floors"]>> | undefined;
}

interface ResolvedSettings {
  settings?: {
    appearance?: ViewerAppearance;
    colour_plans?: ColourPlan[];
  };
}

/**
 * Push the SELECTION colours into custom properties on the document root.
 *
 * Selection rings are SVG in the marks overlay, styled by the stylesheet — so
 * unlike every other colour here they are not something GL paints and cannot
 * ride the paint request. Each property is REMOVED rather than set to a default
 * when the project does not name it, so the stylesheet's own
 * `var(--sel-x, var(--accent))` fallback answers; writing the accent in
 * explicitly would pin it and stop it following a theme change.
 */
export function applySelectionColours(appearance: ViewerAppearance): void {
  const root = document.documentElement;
  const set = (name: string, value: string | null | undefined) => {
    if (typeof value === "string" && value) root.style.setProperty(name, value);
    else root.style.removeProperty(name);
  };
  set("--sel-rooms", appearance.rooms?.selection);
  set("--sel-doors", appearance.doors?.selection);
  set("--sel-windows", appearance.windows?.selection);
  set("--sel-ffe", appearance.ffe?.selection);
  // The three outline layers, since C6. Named by ENTITY like the rest, not by
  // the one `surface-selected-mark` class the three share: they are three
  // settings and a reader comparing a ceiling against the floor under it needs
  // to be able to tell the two marks apart.
  set("--sel-spaces", appearance.spaces?.selection);
  set("--sel-ceilings", appearance.ceilings?.selection);
  set("--sel-floors", appearance.floors?.selection);
}

/**
 * Load one project's appearance.
 *
 * A failed fetch RESETS it rather than leaving what was there: after a project
 * that did set colours, keeping them would paint the next project's plan in the
 * previous one's palette, which reads as the wrong data rather than a failed
 * request.
 */
export async function loadAppearance(projectId: string | null): Promise<void> {
  if (!projectId) {
    setState({ appearance: {}, colourPlans: [] });
    applySelectionColours({});
    return;
  }
  let appearance: ViewerAppearance = {};
  let colourPlans: ColourPlan[] = [];
  try {
    // Both fields from ONE read, deliberately: they are two fields of one
    // settings file fetched on one trigger, and a second request would be a
    // second thing to keep in step with the project picker.
    const data = await fetchJson<ResolvedSettings>(`/api/settings/resolve/${encodeURIComponent(projectId)}`);
    appearance = data.settings?.appearance ?? {};
    colourPlans = data.settings?.colour_plans ?? [];
  } catch {
    appearance = {};
    colourPlans = [];
  }
  // Only if the project is still the one asked for: two project changes in
  // quick succession would otherwise let the slower response win.
  if (getState().scope.projectId !== projectId) return;
  // Every zone takes the project's DEFAULT plan (the one marked active), which
  // is what "this project is normally read by department" means.
  const active = colourPlans.find((p) => p.active)?.name ?? null;
  setState({
    appearance,
    colourPlans,
    zones: getState().zones.map((z) => ({ ...z, colourPlan: active })),
  });
  applySelectionColours(appearance);
}
