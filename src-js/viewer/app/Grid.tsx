// The rooms grid: every room in scope, every column the payload offers.
//
// **Row windowing, not a full table.** Only the visible slice is ever in the
// DOM, with two spacer rows standing in for the rest of the scroll height —
// RHH is 3,013 rooms against 40-odd columns, and a full table is over a hundred
// thousand cells. It is the one-dimensional case of the plan's own cull.
//
// The head is NOT rebuilt on a sort or a filter keystroke, only when the column
// set changes. That is more than a cost: re-rendering it would detach the
// `<input>` being typed into, losing focus and caret mid-word — which React
// does for free here only because the inputs are keyed by column.

import { useEffect, useMemo, useRef, useState } from "react";

import { buildColumns, buildCsv, columnGroups, computeRows, visibleColumns, type SortState } from "../grid.js";
import { detectReferenceSources, sourceDisplayName } from "../properties.js";
import { gridErrors } from "../validation.js";
import { select } from "./store.js";
import { useViewer } from "./useViewer.js";
import { panToRoom } from "./zoneRegistry.js";

/** Row height in CSS pixels, and the old page's own number. The window
 *  arithmetic is wrong the moment these disagree, which is why it is stated
 *  here rather than measured per render. */
const ROW_H = 20;
/** Rows rendered beyond the viewport at each end, so a fast scroll does not
 *  show a band of empty table before the next frame lands. */
const OVERSCAN = 8;
/** How far the pointer may travel between press and release and still count as
 *  a click rather than a drag, in CSS pixels.
 *
 *  **This is how copying out of the grid keeps working.** A drag across a cell
 *  selects its text and ends in a `click`, which would otherwise select the
 *  room and yank the plan out from under the reader. Asking
 *  `window.getSelection()` at click time looks like the direct test and is
 *  not: the press that starts the NEXT click has already collapsed the
 *  selection in the DOM but not always by the time the handler runs, so the
 *  first click after a copy was swallowed — measured, on House A. Where the
 *  pointer went is a fact about this gesture alone. */
const DRAG_SLOP = 4;

export function Grid() {
  const { payload, scope, validation, selection } = useViewer();
  const [showModel, setShowModel] = useState(true);
  /** Whether a row click also brings the room into view. Checked by default —
   *  "where is this room" is what a reader clicking a row is asking — and
   *  deliberately NOT remembered across reloads, like every other view
   *  preference here. */
  const [panTo, setPanTo] = useState(true);
  const [enabled, setEnabled] = useState<ReadonlySet<string>>(new Set());
  const [filters, setFilters] = useState<ReadonlyMap<string, string>>(new Map());
  const [sort, setSort] = useState<SortState>({ key: null, asc: true });
  const [scrollTop, setScrollTop] = useState(0);
  /** Where the press that may become a row click started. See `DRAG_SLOP`. */
  const downAt = useRef<{ x: number; y: number } | null>(null);
  const [viewH, setViewH] = useState(200);
  /** Folded to its bar, so the plans reclaim the height. */
  const [collapsed, setCollapsed] = useState(false);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  const sources = useMemo(() => detectReferenceSources(payload), [payload]);
  // QA marks at the finest address the report supports: a field-level finding
  // is a (row, column) here, so a mismatch is marked ON the disagreeing cell
  // rather than only listed in band 1. Room-level findings have no column and
  // mark the Id cell instead.
  const errors = useMemo(() => gridErrors(validation), [validation]);
  // A newly discovered source defaults to SHOWN, the same "new field → on" rule
  // the search field picker follows: a source that arrives and is invisible
  // looks like a join that failed.
  useEffect(() => {
    setEnabled((prev) => {
      const next = new Set(prev);
      for (const s of sources) if (!next.has(s)) next.add(s);
      return next;
    });
  }, [sources]);

  const columns = useMemo(() => buildColumns(payload, scope.projectId, sources), [payload, scope.projectId, sources]);
  const cols = useMemo(() => visibleColumns(columns, showModel, enabled), [columns, showModel, enabled]);
  const rows = useMemo(() => computeRows(payload?.rooms ?? [], cols, filters, sort), [payload, cols, filters, sort]);

  // Measured, and RE-measured on resize: the window arithmetic needs the real
  // height of the scroller, and it changes when the region is dragged, when a
  // band opens, and when the window itself resizes. Measuring once on the
  // first payload left the estimate stale and rendered far too many rows.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const measure = () => setViewH(el.clientHeight || 200);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [collapsed, cols.length]);

  const first = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const last = Math.min(rows.length, first + Math.ceil(viewH / ROW_H) + OVERSCAN * 2);
  const before = first * ROW_H;
  const after = Math.max(0, (rows.length - last) * ROW_H);
  const total = payload?.rooms?.length ?? 0;

  return (
    <div id="gridBand" className={collapsed ? "collapsed" : ""}>
      <div id="gridBar">
        {/* The Rooms label doubles as the collapse toggle, the same affordance
            the band-1 blocks carry — folding the table away when the plans
            matter more than the numbers. */}
        <button
          className="grid-collapse"
          id="gridHead"
          title={collapsed ? "Show the table" : "Collapse the table"}
          onClick={() => setCollapsed(!collapsed)}
        >
          {collapsed ? "▸" : "▾"} Rooms
        </button>
        <label>
          <input type="checkbox" checked={showModel} onChange={(e) => setShowModel(e.target.checked)} /> Model
        </label>
        <label title="A row click also centres the room in every zone showing its storey">
          <input type="checkbox" checked={panTo} onChange={(e) => setPanTo(e.target.checked)} /> Pan to room
        </label>
        <span id="srcToggles">
          {sources.map((name) => (
            <label key={name}>
              <input
                type="checkbox"
                checked={enabled.has(name)}
                onChange={(e) => {
                  const next = new Set(enabled);
                  if (e.target.checked) next.add(name);
                  else next.delete(name);
                  setEnabled(next);
                }}
              />{" "}
              {sourceDisplayName(name)}
            </label>
          ))}
        </span>
        <button title="Clear all column filters" onClick={() => setFilters(new Map())}>
          Clear filters
        </button>
        <button onClick={() => downloadCsv(rows, cols, scope.projectId)} disabled={!rows.length}>
          Download CSV
        </button>
        <span className="count" id="gridCount">{rows.length === total ? `${total} rooms` : `${rows.length} of ${total} rooms`}</span>
      </div>
      {cols.length === 0 ? (
        <div id="gridEmpty">No columns shown — enable Model or a reference source above.</div>
      ) : (
        <div id="gridScroll" ref={scrollRef} onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}>
          <table id="gridTable">
            <thead>
              <tr className="group">
                {columnGroups(cols).map((g) => (
                  <th
                    key={g.source}
                    className={`group-cell src-${cssIdent(g.source)}${g.source === "model" ? "" : " src-reference"}`}
                    colSpan={g.span}
                  >
                    <span>{g.label}</span>
                  </th>
                ))}
              </tr>
              <tr className="names">
                {cols.map((c) => (
                  <th
                    key={c.key}
                    className={`src-${cssIdent(c.source)}${c.source === "model" ? "" : " src-reference"}${
                      sort.key === c.key ? ` sorted${sort.asc ? " asc" : ""}` : ""
                    }${c.unmapped ? " unmapped" : ""}`}
                    {...(c.unmapped ? { title: `${c.label} — no Revit property mapped (not QA-checked)` } : {})}
                    onClick={() => setSort(sort.key === c.key ? { key: c.key, asc: !sort.asc } : { key: c.key, asc: true })}
                  >
                    {c.label}
                  </th>
                ))}
              </tr>
              <tr id="gridFilters">
                {cols.map((c) => (
                  <th key={c.key} className={`src-${cssIdent(c.source)}${c.source === "model" ? "" : " src-reference"}`}>
                    <input
                      value={filters.get(c.key) ?? ""}
                      placeholder="filter"
                      onChange={(e) => {
                        const next = new Map(filters);
                        const v = e.target.value.trim().toLowerCase();
                        if (v) next.set(c.key, v);
                        else next.delete(c.key);
                        setFilters(next);
                        // Back to the top: a filter that leaves fewer rows than
                        // the current scroll offset would otherwise show an
                        // empty table and read as "no matches".
                        setScrollTop(0);
                        if (scrollRef.current) scrollRef.current.scrollTop = 0;
                      }}
                    />
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {before > 0 ? (
                <tr className="spacer">
                  <td colSpan={cols.length} style={{ height: before }} />
                </tr>
              ) : null}
              {/* The selected class is DERIVED as the rows render, never
                  written onto a node: the table is windowed, so a row that
                  scrolls out and back is a different element and a mark
                  applied by hand would not survive the trip. It also means a
                  room selected on the plan marks its row for free. */}
              {rows.slice(first, last).map((room) => (
                <tr
                  key={room.id}
                  data-room={room.id}
                  className={selection?.kind === "room" && selection.id === room.id ? "selected" : ""}
                  style={{ height: ROW_H }}
                  onMouseDown={(e) => {
                    downAt.current = { x: e.clientX, y: e.clientY };
                  }}
                  // Selected with NO zone: this did not come from a plan, and
                  // the panel's "from zone-N" line would be a lie. A room whose
                  // storey no zone shows is still selected and the panel says
                  // so, which is why the pan is a separate step rather than a
                  // condition on the selection.
                  onClick={(e) => {
                    const from = downAt.current;
                    downAt.current = null;
                    if (from && Math.hypot(e.clientX - from.x, e.clientY - from.y) > DRAG_SLOP) return;
                    select("room", room.id);
                    if (panTo) panToRoom(room);
                  }}
                >
                  {cols.map((c) => {
                    const v = c.get(room);
                    const bad = errors.cells.has(`${room.id} ${c.key}`) || (c.key === "$id" && errors.rooms.has(room.id));
                    return (
                      <td
                        key={c.key}
                        className={`src-${cssIdent(c.source)}${c.source === "model" ? "" : " src-reference"}${bad ? " err" : ""}`}
                        title={v}
                      >
                        {v}
                      </td>
                    );
                  })}
                </tr>
              ))}
              {after > 0 ? (
                <tr className="spacer">
                  <td colSpan={cols.length} style={{ height: after }} />
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

/** A source name as a CSS class fragment. */
function cssIdent(name: string): string {
  return name.replace(/[^a-zA-Z0-9_-]/g, "-");
}

/** Client-side, like every other export here: a CSV is a presentation
 *  reshuffle of data the browser already holds. */
function downloadCsv(rows: Parameters<typeof buildCsv>[0], cols: Parameters<typeof buildCsv>[1], projectId: string | null): void {
  const csv = buildCsv(rows, cols);
  if (!csv) return;
  const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `rooms-${projectId || "project"}.csv`;
  a.click();
  URL.revokeObjectURL(a.href);
}
