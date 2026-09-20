// The drop-down every checkbox menu on this page is built from: a button that
// carries its own summary, a panel of whatever the caller puts in it, and a
// click anywhere else that closes it.
//
// **Three menus, one shell.** C1's visibility menu, C2's selection filter and
// C4's room-contents chooser ask three different questions with the same
// control — a list of checkboxes behind a button saying how many are on. The
// third copy of the outside-click effect is where that stopped being
// repetition and started being the "same state in several places, drifting"
// signal, so the effect, the anchoring and the `fields-panel` class live here
// once.
//
// **The children are arbitrary, not a list of `{label, checked}`.** The
// visibility menu nests a `<select>` under its Spaces entry and the contents
// chooser names a disabled one, so a shell that owned the rows would have
// grown a variant per caller — which is the thing it was extracted to stop.

import { useEffect, useRef, useState, type ReactNode } from "react";

/**
 * Every open menu's close function, so opening one shuts the rest.
 *
 * **The outside-click listener cannot do this**, and that is worth stating
 * rather than rediscovering: the button stops the event so a click on it is not
 * also a click on the plan, and a React handler's `stopPropagation` stops the
 * NATIVE event at the root container — so it never reaches the `document`
 * listener the other menu is waiting on. Two panels then sit open over each
 * other, which the zone toolbar shows plainly: Layers and Select are adjacent
 * and their panels overlap.
 */
const openMenus = new Set<() => void>();

export function DropMenu({ label, title, children }: { label: ReactNode; title: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement | null>(null);

  // A click anywhere else closes it, the way the search field picker does.
  useEffect(() => {
    if (!open) return;
    const shut = () => setOpen(false);
    openMenus.add(shut);
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("click", close);
    return () => {
      openMenus.delete(shut);
      document.removeEventListener("click", close);
    };
  }, [open]);

  return (
    <div className="layer-menu" ref={root}>
      <button
        className="ctl"
        title={title}
        onClick={(e) => {
          // The zone under this button selects on a click; the menu's own
          // clicks are not plan clicks.
          e.stopPropagation();
          // Copied, because closing a menu mutates the set while we walk it.
          if (!open) for (const shut of [...openMenus]) shut();
          setOpen(!open);
        }}
      >
        {label} ▾
      </button>
      <div className={`fields-panel${open ? "" : " hidden"}`}>{children}</div>
    </div>
  );
}
