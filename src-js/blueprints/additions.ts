// A room's additions: items this room has that the blueprint does not, each a copy of a blueprint item on the
// ROOM's own coarse edge (src/assignments.rs `ItemAddition`). This file only edits the list; the drawing is the
// resolver's answer to a request that carries it, as for overrides.

import type { ItemAddition } from "./types.js";

export const ADDITION_PREFIX = "add-";
export const MAX_ADDITIONS = 200;
export const MAX_ADDITION_MM = 100_000;

export const isAddition = (id: string): boolean => id.startsWith(ADDITION_PREFIX);

const clamp = (n: number, lim: number): number => Math.max(-lim, Math.min(lim, Math.round(Number.isFinite(n) ? n : 0)));

/** The next free id: `add-1`, `add-2` ... never one in use, so a removed addition's id may come back but a live one's
 *  never does. */
export function nextId(list: readonly ItemAddition[]): string {
  const used = new Set(list.map((a) => a.id));
  let n = 1;
  while (used.has(`${ADDITION_PREFIX}${n}`)) n += 1;
  return `${ADDITION_PREFIX}${n}`;
}

/** A copy of `cloneOf` against the room's edge 0, a little way in: somewhere visible that the person then moves. */
export function add(list: readonly ItemAddition[], cloneOf: string): ItemAddition[] {
  if (list.length >= MAX_ADDITIONS) return [...list];
  return [...list, { id: nextId(list), clone_of: cloneOf, edge: 0, along_mm: 600, off_mm: 400 }];
}

/** Change one addition's place. Whole numbers, kept in the range the server accepts; a turn wraps to a turn. */
export function edit(
  list: readonly ItemAddition[],
  id: string,
  patch: { edge?: number; along_mm?: number; off_mm?: number; facing_deg?: number },
): ItemAddition[] {
  return list.map((a) => {
    if (a.id !== id) return a;
    const next: ItemAddition = {
      ...a,
      edge: Math.max(0, Math.round(patch.edge ?? a.edge)),
      along_mm: clamp(patch.along_mm ?? a.along_mm, MAX_ADDITION_MM),
      off_mm: clamp(patch.off_mm ?? a.off_mm, MAX_ADDITION_MM),
    };
    const facing = clamp(patch.facing_deg ?? a.facing_deg ?? 0, 360);
    if (facing) next.facing_deg = facing;
    else delete next.facing_deg;
    return next;
  });
}

export const remove = (list: readonly ItemAddition[], id: string): ItemAddition[] => list.filter((a) => a.id !== id);

/** Whether two lists mean the same thing, whatever their order. */
export function sameAdditions(a: readonly ItemAddition[], b: readonly ItemAddition[]): boolean {
  const key = (l: readonly ItemAddition[]) =>
    JSON.stringify(
      [...l]
        .map((x) => ({ ...x, facing_deg: x.facing_deg ?? 0 }))
        .sort((x, y) => x.id.localeCompare(y.id)),
    );
  return key(a) === key(b);
}

/** The query value the resolve read takes; absent when there is nothing to preview. */
export const encode = (list: readonly ItemAddition[]): string | undefined => (list.length === 0 ? undefined : JSON.stringify(list));
