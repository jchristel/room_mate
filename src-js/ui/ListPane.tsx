// The left-hand pane of a list page, so the tasks, the blueprints, the sheets and the tag rules all look and behave
// alike: an action row, a filter box, the preset filters (chips), then the list, each section ruled off from the next.
// A page supplies what goes in each; this is only where they stand.

import type { ReactNode } from "react";

export function ListPane({
  label,
  actions,
  search,
  filters,
  footer,
  children,
}: {
  /** The pane's accessible name. */
  label: string;
  /** Buttons for the list as a whole (make, import, select several). */
  actions?: ReactNode;
  /** A `SearchBox`. */
  search?: ReactNode;
  /** Preset filters: chips and any select that narrows the list with them. */
  filters?: ReactNode;
  /** What acts on the chosen row; ruled off below the list. */
  footer?: ReactNode;
  /** The list (`RowList`) and its empty-state hints. */
  children: ReactNode;
}) {
  return (
    <aside className="left" aria-label={label}>
      {actions ? (
        <>
          <div className="actions">{actions}</div>
          <hr className="sep" />
        </>
      ) : null}
      {search ? (
        <>
          {search}
          <hr className="sep" />
        </>
      ) : null}
      {filters ? (
        <>
          {filters}
          <hr className="sep" />
        </>
      ) : null}
      {children}
      {footer ? (
        <>
          <hr className="sep" />
          <div className="actions">{footer}</div>
        </>
      ) : null}
    </aside>
  );
}

/** The filter box. Every word typed must be found (`ui/match.ts`). */
export function SearchBox({ value, onChange, placeholder, label }: { value: string; onChange(text: string): void; placeholder: string; label: string }) {
  return <input type="search" placeholder={placeholder} value={value} onChange={(e) => onChange(e.target.value)} aria-label={label} />;
}

/** A preset filter that is either on or off. `kind` colours it (a task state, `warning`, `attention`). */
export function Chip({ on, onClick, kind, title, children }: { on: boolean; onClick(): void; kind?: string; title?: string; children: ReactNode }) {
  return (
    <button type="button" className={`chip ${kind ?? ""} ${on ? "on" : ""}`} aria-pressed={on} title={title} onClick={onClick}>
      {children}
    </button>
  );
}

/** The chips of one filter, side by side. */
export function Chips({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="chips" role="group" aria-label={label}>
      {children}
    </div>
  );
}

export function RowList({ children }: { children: ReactNode }) {
  return <ul>{children}</ul>;
}

/** One row: a button that says which row is chosen (`on`) and what clicking it does. Its children are the row's
 * lines, a `<b>` title then `<span>`s. */
export function Row({ on, onClick, children }: { on: boolean; onClick(): void; children: ReactNode }) {
  return (
    <li>
      <button type="button" className={on ? "on" : ""} aria-pressed={on} onClick={onClick}>
        {children}
      </button>
    </li>
  );
}
