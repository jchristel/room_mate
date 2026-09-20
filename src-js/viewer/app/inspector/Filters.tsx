// The property filter, the hide-empty box and the property chooser.
//
// **Three controls over one list, and they narrow it in different ways.** The
// filter matches a name a reader is hunting for, hide-empty drops the rows
// Revit left unset, and the chooser (G7) picks the handful of properties this
// KIND is being read for. The first two were room-only on the old page,
// because "an opening's two short tiers have nothing to filter" — that stopped
// being true when the element panels grew full instance and type property
// sections, and they have been applying the filters invisibly ever since. So
// every panel shows all three now: a control that is in effect and not on
// screen is how a panel comes to look broken.
//
// The filter state lives in the store and is deliberately kept ACROSS
// selection changes — the common use is comparing one field over several rooms
// by clicking each in turn — and so does the chooser's, per kind.

import { PropertyChooser, type ChoiceItem } from "../PropertyChooser.js";
import { setInspectorFilter, type PropertyScope } from "../store.js";
import { useViewer } from "../useViewer.js";

export function Filters({ scope, items }: { scope: PropertyScope; items: readonly ChoiceItem[] }) {
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
      <PropertyChooser scope={scope} items={items} />
    </div>
  );
}
