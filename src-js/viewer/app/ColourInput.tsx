// A colour picker that commits once, when the picker closes. Shared by the route bar
// (the colour a route is saved with) and the saved-routes table (changing one).

import { useEffect, useRef } from "react";

/** A colour picker that reports once, when the picker closes. A colour input fires
 *  `input` continuously while dragging, and each report of this one is a save. */
export function ColourInput({ value, onCommit, label }: { value: string; onCommit: (colour: string) => void; label: string }) {
  const ref = useRef<HTMLInputElement>(null);
  const latest = useRef(onCommit);
  latest.current = onCommit;
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const done = () => latest.current(el.value);
    el.addEventListener("change", done);
    return () => el.removeEventListener("change", done);
  }, []);
  // Uncontrolled between commits, re-keyed when the saved value moves.
  return <input key={value} ref={ref} className="colourInput" type="color" defaultValue={value} aria-label={label} title={label} />;
}
