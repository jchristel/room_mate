// The right-hand panel: whatever KIND is selected.
//
// One dispatcher over one shared header, with a component per kind — adding a
// kind is a line here, not another branch inside a room-shaped function. Every
// kind the plan can pick has a panel now: rooms, the three element layers, the
// two surfaces, spaces, and the areas overlay's footprints.
//
// With no selection the column costs no width, which is what keeps the plan
// full-width until a reader asks a question of it.

import { useViewer } from "../useViewer.js";
import { AreaInspector } from "./AreaInspector.js";
import { ElementInspector } from "./ElementInspector.js";
import { RoomInspector } from "./RoomInspector.js";
import { SpaceInspector } from "./SpaceInspector.js";
import { SurfaceInspector } from "./SurfaceInspector.js";

export function Inspector() {
  const { selection } = useViewer();
  if (!selection) return null;

  return (
    <aside id="inspector" aria-label="Selected element">
      {selection.kind === "area" ? (
        <AreaInspector selection={selection} />
      ) : selection.kind === "room" ? (
        <RoomInspector selection={selection} />
      ) : selection.kind === "door" || selection.kind === "window" || selection.kind === "item" ? (
        <ElementInspector selection={selection} />
      ) : selection.kind === "space" ? (
        <SpaceInspector selection={selection} />
      ) : (
        <SurfaceInspector selection={selection} />
      )}
    </aside>
  );
}
