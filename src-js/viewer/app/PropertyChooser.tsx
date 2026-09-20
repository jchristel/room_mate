// "Customize": which of the properties a kind offers actually get a row.
//
// **One component for six panels and the grid**, because it is one question —
// a room carries 45 properties and a reader wants six of them, and the grid
// carries the same 45 as columns. The panels' name filter and hide-empty box
// narrow a list nobody chose; the grid's source toggles switch a whole source
// on and off. This is the one control that picks WITHIN.
//
// **It records what is turned OFF**, so a property arriving later is on rather
// than silently missing — see `keepChosen`, which holds the reasoning and the
// test. The consequence here is that "All" writes an EMPTY set rather than
// every name, and "None" writes the names this kind is offering right now.
//
// **Names, not sections, in a panel.** A name carried by both the model and a
// joined reference source is one entry, so unticking "Area" hides it
// everywhere on that panel. That is deliberate: it is exactly the granularity
// of the name filter sitting beside it, and two controls over one list that
// disagreed about what a property IS would be worse than the collision. The
// grid is the exception and keys by COLUMN, because two sources' columns are
// two columns there, which is why an entry carries an id apart from its label.

import { DropMenu } from "./DropMenu.js";
import { setHiddenProperties, type PropertyScope } from "./store.js";
import { useViewer } from "./useViewer.js";

/** One line of the menu. `id` is what is hidden; `label` is what is read; a
 *  `group` heads a run of entries, for the grid's per-source columns. */
export interface ChoiceItem {
  id: string;
  label: string;
  group?: string;
}

export function PropertyChooser({ scope, items }: { scope: PropertyScope; items: readonly ChoiceItem[] }) {
  const { hiddenProperties } = useViewer();
  const hidden = hiddenProperties[scope];

  // Counted against what this kind is OFFERING, not against the stored set: a
  // hidden name left over from a previous element is not something the reader
  // can see or act on here, so counting it would make the button disagree with
  // its own list.
  const off = items.filter((i) => hidden?.has(i.id)).length;
  const label = off ? `Customize: ${items.length - off} of ${items.length}` : "Customize";

  const set = (next: ReadonlySet<string>) => setHiddenProperties(scope, next);
  const toggle = (id: string, on: boolean) => {
    const next = new Set(hidden ?? []);
    if (on) next.delete(id);
    else next.add(id);
    set(next);
  };

  let group: string | undefined;

  return (
    <DropMenu label={label} title="Which properties this panel shows">
      <div className="fields-actions">
        {/* "All" clears the set rather than ticking every name, which is what
            keeps a later arrival on. */}
        <a onClick={() => set(new Set())}>All</a>
        <a onClick={() => set(new Set(items.map((i) => i.id)))}>None</a>
      </div>
      {items.length ? null : <label className="menu-off">Nothing to choose from</label>}
      {items.map((item) => {
        const head = item.group && item.group !== group ? item.group : null;
        group = item.group;
        return (
          <div key={item.id}>
            {head ? <div className="menu-group">{head}</div> : null}
            <label>
              <input type="checkbox" checked={!hidden?.has(item.id)} onChange={(e) => toggle(item.id, e.target.checked)} />{" "}
              {item.label}
            </label>
          </div>
        );
      })}
    </DropMenu>
  );
}

/** Property names as chooser entries, deduplicated and in the order given.
 *  Every panel builds its list this way: the rows are already sorted by name,
 *  so the menu reads in the same order the panel does. */
export function nameItems(...lists: readonly (readonly (readonly [string, string])[])[]): ChoiceItem[] {
  const seen = new Set<string>();
  const items: ChoiceItem[] = [];
  for (const rows of lists) {
    for (const [name] of rows) {
      if (seen.has(name)) continue;
      seen.add(name);
      items.push({ id: name, label: name });
    }
  }
  return items;
}
