// The right-hand panel: whatever KIND is selected.
//
// One dispatcher over one shared header, with a component per kind — adding a
// kind is a line here plus a hit test, not another branch inside a room-shaped
// function. An unknown kind is NAMED rather than silently rendering nothing: a
// blank panel reads as a bug in the selection, which is exactly what it would
// be hiding.
//
// With no selection the column costs no width, which is what keeps the plan
// full-width until a reader asks a question of it.

import { useViewer } from "../useViewer.js";
import { ElementInspector } from "./ElementInspector.js";
import { Note } from "./parts.js";
import { RoomInspector } from "./RoomInspector.js";

export function Inspector() {
  const { selection } = useViewer();
  if (!selection) return null;

  return (
    <aside id="inspector" aria-label="Selected element">
      {selection.kind === "room" ? (
        <RoomInspector selection={selection} />
      ) : selection.kind === "door" || selection.kind === "window" || selection.kind === "item" ? (
        <ElementInspector selection={selection} />
      ) : (
        // Spaces, ceilings and floors are selectable on the plan since phase A,
        // and their panels are the selection plan's C2. Saying so beats a blank
        // column that looks like a broken selection.
        <Note>No inspector for a &quot;{selection.kind}&quot; selection yet.</Note>
      )}
    </aside>
  );
}
