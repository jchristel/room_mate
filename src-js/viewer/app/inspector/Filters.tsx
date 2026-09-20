// The property filter and the hide-empty box.
//
// ROOM-ONLY in the old page and kept that way: they exist for the 45 model
// properties a room carries, and an opening's two short tiers have nothing to
// filter. The state lives in the store, so it survives a selection change —
// the common use is comparing one field across rooms by clicking each in turn.

import { setInspectorFilter } from "../store.js";
import { useViewer } from "../useViewer.js";

export function Filters() {
  const { inspector } = useViewer();
  return (
    <div className="insp-controls">
      <input
        type="search"
        placeholder="Filter properties…"
        value={inspector.filter}
        onChange={(e) => setInspectorFilter({ filter: e.target.value })}
      />
      <label>
        <input
          type="checkbox"
          checked={inspector.hideEmpty}
          onChange={(e) => setInspectorFilter({ hideEmpty: e.target.checked })}
        />
        Hide empty
      </label>
    </div>
  );
}
