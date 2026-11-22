// The one rule every filter box follows: a row matches when it contains EVERY word typed, in any of its texts,
// whatever the case. Nothing typed matches everything.

/** Whether `hay` contains every word of `text` (split on spaces, case-insensitive). */
export function matchesAll(hay: string, text: string): boolean {
  const h = hay.toLowerCase();
  return text.toLowerCase().split(/\s+/).filter(Boolean).every((w) => h.includes(w));
}
