// One zone's selection filter: which kinds a click and a hover may reach.
//
// The same control as the visibility menu, over a different question — what is
// DRAWN against what is PICKABLE — and deliberately built from the same parts,
// because a reader who has learned one has learned the other.
//
// **A kind must be drawn to be pickable**, which the filter does not override:
// the gesture code checks the layer as well, since a click resolving to
// something nobody can see is the bug both rules exist to prevent.

import { useEffect, useRef, useState } from "react";

import { setZonePickable, type SelectionKind, type ZoneRow } from "./store.js";

/** The kinds, in the order the pick stack returns them, with `area` last: a
 *  footprint is page geometry over the plan rather than an element, and it is
 *  picked by its own overlay rather than through the stack. */
const KINDS: readonly { kind: SelectionKind; label: string }[] = [
  { kind: "door", label: "Doors" },
  { kind: "window", label: "Windows" },
  { kind: "item", label: "FF&E" },
  { kind: "room", label: "Rooms" },
  { kind: "space", label: "Spaces" },
  { kind: "ceiling", label: "Ceilings" },
  { kind: "floor", label: "Floors" },
];

export function SelectionFilter({ zone }: { zone: ZoneRow }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("click", close);
    return () => document.removeEventListener("click", close);
  }, [open]);

  const on = KINDS.filter((k) => zone.pickable[k.kind]).length;

  return (
    <div className="layer-menu" ref={root}>
      <button
        className="ctl"
        title="Which kinds a click in this zone can select"
        onClick={(e) => {
          e.stopPropagation();
          setOpen(!open);
        }}
      >
        Select: {on} ▾
      </button>
      <div className={`fields-panel${open ? "" : " hidden"}`}>
        {KINDS.map((k) => (
          <label key={k.kind}>
            <input
              type="checkbox"
              checked={zone.pickable[k.kind]}
              onChange={(e) => setZonePickable(zone.id, k.kind, e.target.checked)}
            />{" "}
            {k.label}
          </label>
        ))}
      </div>
    </div>
  );
}
