// What a click found, when it found more than one thing.
//
// **Only when several things match**, which with the default filter means a
// reader has asked for two overlapping kinds at once — Rooms plus FF&E, say, or
// a ceiling against the room beneath it. One match still selects on the click,
// so the common case is unchanged and nothing new appears in the way.
//
// Hovering an entry marks that element on the plan. That is the whole reason
// the list is worth having over a cycle: you can see which one you are about to
// select before selecting it.

import { useEffect } from "react";

import { closePickList, select } from "./store.js";
import { useViewer } from "./useViewer.js";
import { handleOf } from "./zoneRegistry.js";

export function PickList() {
  const { pickList } = useViewer();

  // Escape closes it, like every other transient panel here. A click outside
  // is handled by the plan's own pointerdown, which closes before it picks.
  useEffect(() => {
    if (!pickList) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closePickList();
    };
    const zoneId = pickList.zoneId;
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      // Leaving no mark behind is part of closing: the hover mark belongs to
      // this list while it is open, and one that outlived it would read as a
      // stuck selection.
      handleOf(zoneId)?.renderer.setHover(null);
    };
  }, [pickList]);

  if (!pickList) return null;

  const hover = (kind: string, id: string | null) => {
    const handle = handleOf(pickList.zoneId);
    if (!handle) return;
    handle.renderer.setHover(id && kind !== "area" ? { kind: kind as never, id } : null);
  };

  return (
    <div
      className="pick-list"
      // Positioned at the click, in viewport coordinates, and clamped by the
      // stylesheet's `max-height` rather than by arithmetic: the list is short
      // — it names what is under one point — and a menu that measured itself
      // would be doing so to solve a problem it does not have.
      style={{ left: pickList.at.x, top: pickList.at.y }}
      onPointerDown={(e) => e.stopPropagation()}
    >
      {pickList.entries.map((entry) => (
        <button
          key={`${entry.kind}:${entry.id}`}
          className="pick-entry"
          onMouseEnter={() => hover(entry.kind, entry.id)}
          onMouseLeave={() => hover(entry.kind, null)}
          onClick={() => {
            hover(entry.kind, null);
            select(entry.kind, entry.id, pickList.zoneId);
            closePickList();
          }}
        >
          <span className="pick-kind">{entry.kind}</span> {entry.label}
          <span className="pick-id">{entry.id}</span>
        </button>
      ))}
    </div>
  );
}
