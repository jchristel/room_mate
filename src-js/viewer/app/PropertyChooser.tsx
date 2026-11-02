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
// test. The consequence here is that the two actions only ever remove ids from
// that set or add ones already on screen; neither can hide something the
// reader has not been shown.
//
// **A prefix box, and two actions scoped to what it leaves.** Ticking 162 door
// properties one at a time is not a control, it is a chore, and the names come
// in families — `Vis_`, `Frame`, `VisionPanel_`, `drofus_` — because that is
// how Revit and a reference source spell a group. So the box narrows by prefix
// and "check / uncheck all visible" act on exactly what it left, which turns a
// family into one click.
//
// **Names, not sections, in a panel.** A name carried by both the model and a
// joined reference source is one entry, so unticking "Area" hides it
// everywhere on that panel. That is deliberate: it is exactly the granularity
// of the name filter sitting beside it, and two controls over one list that
// disagreed about what a property IS would be worse than the collision. The
// grid is the exception and keys by COLUMN, because two sources' columns are
// two columns there, which is why an entry carries an id apart from its label.

import { useState } from "react";

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
  /** The menu's own name filter. Local, and deliberately NOT in the store: it
   *  is a way of reaching a row, not a choice about what the panel shows, and
   *  it dies with the menu. */
  const [query, setQuery] = useState("");
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

  // **Prefix, not substring**, and the difference from the name filter sitting
  // outside this menu is deliberate rather than an inconsistency to tidy away.
  // That one hunts for a property whose name you half-remember, so it matches
  // anywhere; this one is how you reach a FAMILY of them -- `Vis_`, `Frame`,
  // `drofus_`, `VisionPanel_` -- to tick or untick the lot, which is the job
  // the two actions above it exist for. A substring match would drag in
  // everything that merely mentions the word.
  const q = query.trim().toLowerCase();
  const shown = q ? items.filter((i) => i.label.toLowerCase().startsWith(q)) : items;

  // **Both actions are scoped to what is SHOWN**, so with an empty box they act
  // on the whole list and with `Vis_` typed they act on that family alone.
  //
  // Checking removes ids from the hidden set and never adds any, so a property
  // that arrives later is still on by default -- the rule `keepChosen` is built
  // around. One consequence worth knowing: "check all visible" no longer clears
  // the set outright, so a name hidden on a PREVIOUS element of this kind that
  // the current one does not carry stays hidden. It is not on screen and not in
  // the list, so acting on it would be acting on something the reader cannot
  // see.
  const checkShown = () => {
    const next = new Set(hidden ?? []);
    for (const i of shown) next.delete(i.id);
    set(next);
  };
  const uncheckShown = () => {
    const next = new Set(hidden ?? []);
    for (const i of shown) next.add(i.id);
    set(next);
  };

  let group: string | undefined;

  return (
    <DropMenu label={label} title="Which properties this panel shows">
      {/* One sticky block, not two sticky siblings: the actions act on what
          the box leaves, so they are one control and have to stay together
          over a list 162 names long. */}
      <div className="menu-head">
        <div className="fields-actions">
          <a onClick={checkShown}>Check all visible</a>
          <a onClick={uncheckShown}>Uncheck all visible</a>
        </div>
        <input
          type="search"
          placeholder="Name starts with…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
      {/* Three different empty states, and they are not the same answer: this
          kind offers nothing at all, or the filter matched nothing. */}
      {items.length ? null : <label className="menu-off">Nothing to choose from</label>}
      {items.length && !shown.length ? <label className="menu-off">No property starts with “{query.trim()}”</label> : null}
      {shown.map((item) => {
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
