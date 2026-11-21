// Choosing one item or several in a list, shared by the sheets and the tag rules. The toggle switches the list
// between single choice (a click opens the item) and multiple choice (a click ticks it, the details are not shown,
// and the only action is deleting what is ticked).

import { useState, type ReactNode } from "react";

export interface Selection {
  multi: boolean;
  picked: ReadonlySet<string>;
  setMulti(on: boolean): void;
  toggle(id: string): void;
  /** Replace the ticked items. */
  set(ids: Iterable<string>): void;
}

export function useSelection(): Selection {
  const [multi, setMultiState] = useState(false);
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  return {
    multi,
    picked,
    setMulti: (on) => {
      setMultiState(on);
      setPicked(new Set());
    },
    toggle: (id) =>
      setPicked((p) => {
        const next = new Set(p);
        if (!next.delete(id)) next.add(id);
        return next;
      }),
    set: (ids) => setPicked(new Set(ids)),
  };
}

export function MultiToggle({ selection }: { selection: Selection }) {
  return (
    <button
      type="button"
      className={selection.multi ? "on" : ""}
      aria-pressed={selection.multi}
      onClick={() => selection.setMulti(!selection.multi)}
    >
      Select several
    </button>
  );
}

/**
 * What the right-hand side shows while several are being chosen: how many are ticked, select all / clear, and whatever
 * the page can do with them (`children`: its action buttons, which it disables itself). `ticked` counts only the
 * items still in the list.
 */
export function MultiPanel({
  noun,
  shown,
  ticked,
  busy,
  onAll,
  onNone,
  hint,
  children,
}: {
  noun: string;
  shown: number;
  ticked: number;
  busy: boolean;
  onAll(): void;
  onNone(): void;
  hint?: string;
  children?: ReactNode;
}) {
  return (
    <div className="detail">
      <div className="toolbar">
        <strong>
          {ticked} {noun}
          {ticked === 1 ? "" : "s"} selected
        </strong>
        <span className="spacer" />
        <button type="button" disabled={busy || ticked === shown} onClick={onAll}>
          Select all shown ({shown})
        </button>
        <button type="button" disabled={busy || ticked === 0} onClick={onNone}>
          Clear
        </button>
        {children}
      </div>
      <p className="hint">{hint ?? `Tick ${noun}s on the left. Details and previews are shown one at a time: switch "Select several" off to see one.`}</p>
    </div>
  );
}
