// The viewer's SCOPE: which project, building and milestone everything on
// screen is showing, and the rules that keep a selection honest when the
// server's lists change under it.
//
// Pure, and outside `app/` on purpose. These four rules are the ones the old
// page got wrong in ways that were invisible — a building default that hid
// every homeless element, a stale milestone that kept scoping reads after the
// server stopped offering it — so they are the part of the port that earns
// tests rather than a comparison by eye.

/** What every read is scoped by. `null` means unscoped, which is a real
 *  answer for each of the three and not "not chosen yet". */
export interface Scope {
  projectId: string | null;
  /** `null` is **All buildings**, and it is the default. `?building=` scopes an
   *  opening or item THROUGH the room it is attributed to, so a homeless
   *  element — every window and external door in a facade package — matches no
   *  building. Defaulting into one silently hid them. */
  building: string | null;
  /** `null` is Latest. */
  milestone: string | null;
}

export const UNSCOPED: Scope = { projectId: null, building: null, milestone: null };

/** The `/rooms` URL for a scope. `milestone` rides only with a project, since
 *  a milestone name means nothing without one to resolve it against. */
export function roomsUrl(scope: Scope): string {
  const params = new URLSearchParams();
  if (scope.projectId) params.set("project", scope.projectId);
  if (scope.building) params.set("building", scope.building);
  if (scope.projectId && scope.milestone) params.set("milestone", scope.milestone);
  const qs = params.toString();
  return qs ? `/rooms?${qs}` : "/rooms";
}

/**
 * Which project to show, given the server's list and what is selected.
 *
 * The seed (URL `?project=`, else `localStorage`) applies ONCE, on the first
 * resolve, and only if the server still lists it: a deep link to a deleted
 * project falls through to the first one rather than showing an empty page.
 * After that a project that disappears re-defaults plainly, which is why
 * `seeded` is an argument rather than state in here.
 */
export function resolveProject(
  projects: readonly { id: string }[],
  current: string | null,
  seed: string | null,
  seeded: boolean,
): string | null {
  if (current && projects.some((p) => p.id === current)) return current;
  if (!projects.length) return null;
  if (!seeded && seed && projects.some((p) => p.id === seed)) return seed;
  return projects[0]!.id;
}

/** A building selection the freshly-loaded list still offers, else All
 *  buildings. Never picks one: see `Scope.building`. */
export function keepBuilding(buildings: readonly { key: string }[], current: string | null): string | null {
  return current && buildings.some((b) => b.key === current) ? current : null;
}

/** A milestone selection the freshly-loaded list still offers, else Latest.
 *  A pin the server stopped serving must not keep scoping reads — that shows
 *  as data quietly frozen at an old issue. */
export function keepMilestone(milestones: readonly { name: string }[], current: string | null): string | null {
  return current && milestones.some((m) => m.name === current) ? current : null;
}

/** How a building reads in the picker. `unclassified` is the server's bucket
 *  for rooms that name no building; an `ambiguous` key is disambiguated by its
 *  code, because two buildings sharing a name is exactly when the label alone
 *  stops being an answer. */
export function buildingLabel(b: {
  key: string;
  name?: string | null;
  code?: string | null;
  unclassified?: boolean;
  ambiguous?: boolean;
}): string {
  if (b.unclassified) return "Unclassified";
  if (b.ambiguous && b.name && b.code) return `${b.name} (${b.code})`;
  return b.name || b.code || b.key;
}
