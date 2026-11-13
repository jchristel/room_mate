// The RLS tool's set mode: the rooms clicked on the plan make ONE set of adjacent rooms, and the composer under the
// bar lays the set's views out on sheets. The server says which views there are and how big each is on paper
// (`previewSet`); every rule about where they go is `rlsSet.ts`, so this file draws and turns gestures into calls.
//
// Gestures: drag a view from the list onto a sheet; drag a view on a sheet to move it, onto another sheet to move it
// there, or back onto the list to take it off. The list says what is waiting and what is left out.

import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import {
  addSheet,
  autoLayout,
  carryLayout,
  findingText,
  layoutFindings,
  leaveOut,
  MARGIN_MM,
  PAPER_MM,
  place,
  portsOf,
  putBack,
  removeSheet,
  saveBlocked,
  setTaskBody,
  sizeOfPaper,
  unplace,
  unplaced,
  type Layout,
  type PaperSize,
  type Port,
  type SetPreview,
} from "../rlsSet.js";
import type { RlsMode, RlsToolState } from "../rlsTool.js";
import { makeRlsSetTask, previewSet, type LibrarySheet } from "./rlsData.js";
import { patchZoneRls, setZoneRlsMode } from "./store.js";

const SIZES: PaperSize[] = ["a0", "a1", "a2", "a3"];
/** A drag moves a view only past this many paper mm, so a click to select it does not nudge it. */
const DRAG_FROM_MM = 1;

interface Drag {
  port: string;
  sheet: number;
  /** Where in the view it was grabbed, from its top left corner, paper mm. */
  grabX: number;
  grabY: number;
  /** Where the pointer went down, and where it is now, paper mm of the sheet it started on. */
  startX: number;
  startY: number;
  x: number;
  y: number;
  moved: boolean;
}

export function RlsSetPanel({
  zoneId,
  tool,
  project,
  sheets,
  name,
}: {
  zoneId: string;
  tool: RlsToolState;
  project: string | null;
  sheets: LibrarySheet[] | null;
  name: (roomId: string) => string;
}) {
  const [preview, setPreview] = useState<SetPreview | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** The library sheet asked for; null: the server's best match. */
  const [sheetId, setSheetId] = useState<string | null>(null);
  /** The paper size a person chose; null: the template's. */
  const [chosenSize, setChosenSize] = useState<PaperSize | null>(null);
  const [layout, setLayout] = useState<Layout | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  /** The composer folded away, so the plan has the room to pick rooms on. */
  const [folded, setFolded] = useState(false);

  const roomKey = tool.roomIds.join("\u0000");
  useEffect(() => {
    // an empty set keeps the notice: it is what a save that just cleared the rooms said
    if (!project || tool.roomIds.length === 0) return setPreview(null);
    setNotice(null);
    const ac = new AbortController();
    setBusy(true);
    void previewSet(project, tool.roomIds, sheetId, ac.signal).then((answer) => {
      if (ac.signal.aborted) return;
      setBusy(false);
      if (typeof answer === "string") {
        if (answer) setNotice(answer);
        return;
      }
      setPreview(answer);
    });
    return () => ac.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project, roomKey, sheetId]);

  const template = sheets?.find((s) => s.id === preview?.sheet_id);
  const size: PaperSize = chosenSize ?? sizeOfPaper(template?.size_mm) ?? "a1";
  const ports = preview ? portsOf(preview) : [];
  const portKey = ports.map((p) => `${p.id}:${p.w}x${p.h}`).join(",");
  useEffect(() => {
    setLayout((old) => (ports.length === 0 ? null : carryLayout(old, ports, size)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [portKey]);

  const mode = (m: RlsMode) => patchZoneRls(zoneId, { ...tool, mode: m });
  const byId = new Map(ports.map((p) => [p.id, p]));
  const findings = layout ? layoutFindings(layout, ports, size) : [];
  const waiting = layout ? unplaced(layout, ports) : [];
  const bigIds = new Set(findings.filter((f) => f.code === "too_big").flatMap((f) => f.ports));
  const leftOut = layout ? layout.leftOut.flatMap((id) => byId.get(id) ?? []) : [];
  const lead = preview?.lead_room ? name(preview.lead_room) : null;
  const blocked = !layout || !preview?.sheet_id ? "lay the set out first" : saveBlocked(findings);
  const save = async () => {
    if (!project || !layout || !preview?.sheet_id || blocked) return;
    setBusy(true);
    setNotice(null);
    const answer = await makeRlsSetTask(project, setTaskBody(tool.roomIds, preview.sheet_id, size, tool.overwrite, layout));
    setBusy(false);
    if (typeof answer === "string") return setNotice(answer);
    const row = answer[0];
    if (!row || row.refused) return setNotice(`Not queued: ${row?.refused ?? "the server said nothing"}`);
    setNotice(`Queued one task for the set of ${tool.roomIds.length} rooms (${layout.sheets.length} sheet${layout.sheets.length === 1 ? "" : "s"}).`);
    // the set is done with: pick the next one
    patchZoneRls(zoneId, { ...tool, roomIds: [] });
  };
  const others = (sheets ?? []).filter((s) => !preview?.suggestions.some((x) => x.sheet_id === s.id));

  /** Drop at a client point: onto a sheet (the view's top left corner is the point less where it was grabbed), or
   *  onto the list (taken off). Anywhere else, nothing changes. */
  const dropAt = (port: string, clientX: number, clientY: number, grabX: number, grabY: number) => {
    if (!layout) return;
    const el = document.elementFromPoint(clientX, clientY);
    if (el?.closest(".rlsPorts")) return setLayout(unplace(layout, port));
    const svg = el?.closest("svg[data-sheet]") as SVGSVGElement | null;
    if (!svg) return;
    const at = toPaper(svg, clientX, clientY);
    if (at) setLayout(place(layout, port, Number(svg.dataset.sheet), at.x - grabX, at.y - grabY));
  };

  return (
    <>
      <div className="routeBar rlsBar">
        <strong>RLS</strong>
        <select className="picker" value={tool.mode} onChange={(e) => mode(e.target.value as RlsMode)} aria-label="RLS mode">
          <option value="manual">RLS manual</option>
          <option value="auto">RLS auto</option>
          <option value="set">RLS set</option>
        </select>
        <select className="picker" value={sheetId ?? ""} onChange={(e) => setSheetId(e.target.value || null)} aria-label="Library sheet the set starts from">
          <option value="">Best match{template && !sheetId ? `: ${template.sheet_number} ${template.sheet_name}` : ""}</option>
          {preview?.suggestions.map((s) => (
            <option key={s.sheet_id} value={s.sheet_id}>
              {Math.round(s.fit)}% {s.sheet_number} {s.sheet_name}
            </option>
          ))}
          {others.map((s) => (
            <option key={s.id} value={s.id}>
              {s.sheet_number} {s.sheet_name}
            </option>
          ))}
        </select>
        <select className="picker" value={chosenSize ?? ""} onChange={(e) => setChosenSize((e.target.value || null) as PaperSize | null)} aria-label="Paper size of every sheet of the set">
          <option value="">{`${size.toUpperCase()} (the template's)`}</option>
          {SIZES.map((s) => (
            <option key={s} value={s}>
              {s.toUpperCase()}
            </option>
          ))}
        </select>
        <span className="routeResult">
          {tool.roomIds.length === 0 ? "click adjacent rooms on the plan" : busy ? "Reading…" : lead ? `lead room ${lead}` : ""}
        </span>
        <span className="rlsRooms">
          {tool.roomIds.map((id) => (
            <button key={id} className="chip" title="Take this room off" onClick={() => patchZoneRls(zoneId, { ...tool, roomIds: tool.roomIds.filter((r) => r !== id) })}>
              {name(id)} ×
            </button>
          ))}
        </span>
        <button className={`chip${folded ? "" : " on"}`} disabled={!layout} title="Show or fold away the sheets, to give the plan room" onClick={() => setFolded(!folded)}>
          Sheets {folded ? "▸" : "▾"}
        </button>
        <button className="chip" disabled={!layout} title="Throw this layout away and lay the views out again" onClick={() => setLayout(autoLayout(ports, size))}>
          Lay out again
        </button>
        <button className="chip" disabled={!layout} onClick={() => layout && setLayout(addSheet(layout))}>
          Add sheet
        </button>
        <label title="Replace sheets that already exist (off, they are skipped with a warning)">
          <input type="checkbox" checked={tool.overwrite} onChange={(e) => patchZoneRls(zoneId, { ...tool, overwrite: e.target.checked })} /> overwrite
        </label>
        <button
          className="chip on"
          disabled={busy || blocked !== null}
          title={blocked ?? "Queue one task for the set, for the Revit session to make (the Tasks button)"}
          onClick={() => void save()}
        >
          {busy ? "Working…" : "Save task"}
        </button>
        {notice ? <span className="meta">{notice}</span> : null}
        <a className="chip" href={`/tasks/?project=${encodeURIComponent(project ?? "")}`} title="See the tasks and what Revit did with them">
          Tasks page
        </a>
        <button className="chip" title="Leave the RLS tool (Esc)" onClick={() => setZoneRlsMode(zoneId, false)}>
          Close
        </button>
      </div>
      {preview && (!preview.valid || preview.findings.length > 0 || findings.length > 0) ? (
        <ul className="rlsFindings">
          {findings
            .filter((f) => f.serious)
            .map((f, i) => (
              <li key={`b${i}`} className="refused">
                {f.message}
              </li>
            ))}
          {preview.findings.map((f, i) => (
            <li key={`s${i}`} className={preview.valid ? "" : "refused"}>
              {findingText(f)}
            </li>
          ))}
          {findings
            .filter((f) => !f.serious)
            .map((f, i) => (
              <li key={`l${i}`}>{f.message}</li>
            ))}
        </ul>
      ) : null}
      {layout && !folded ? (
        <div className="rlsComposer">
          <div className="rlsPorts" aria-label="Views to place">
            <strong>To place</strong>
            {waiting.length === 0 ? <span className="meta">every view is on a sheet or left out</span> : null}
            {waiting.map((p) => (
              <div
                key={p.id}
                className={`rlsPort ${p.kind}${bigIds.has(p.id) ? " tooBig" : ""}`}
                draggable
                onDragStart={(e) => e.dataTransfer.setData("text/x-rls-port", p.id)}
                title="Drag onto a sheet"
              >
                <span>{portText(p)}</span>
                <button className="chip" title="Leave this view out of the set: it is not made" onClick={() => setLayout(leaveOut(layout, p.id))}>
                  Leave out
                </button>
              </div>
            ))}
            {leftOut.length > 0 ? <strong>Left out</strong> : null}
            {leftOut.map((p) => (
              <div key={p.id} className={`rlsPort ${p.kind} out`}>
                <span>{portText(p)}</span>
                <button className="chip" onClick={() => setLayout(putBack(layout, p.id))}>
                  Put back
                </button>
              </div>
            ))}
            <span className="meta">
              Sheets are {size.toUpperCase()} with a {MARGIN_MM} mm border, until the title block's drawable area is known. Sizes marked ~ are estimates: Revit decides them.
            </span>
          </div>
          <div className="rlsSheets">
            {layout.sheets.map((sheet, i) => (
              <SheetBox
                key={i}
                index={i}
                places={sheet}
                byId={byId}
                bigIds={bigIds}
                size={size}
                drag={drag}
                onGrab={(d) => setDrag(d)}
                onMove={(x, y) => setDrag((d) => (d ? { ...d, x, y, moved: d.moved || Math.hypot(x - d.startX, y - d.startY) > DRAG_FROM_MM } : d))}
                onDrop={(clientX, clientY) => {
                  if (drag?.moved) dropAt(drag.port, clientX, clientY, drag.grabX, drag.grabY);
                  setDrag(null);
                }}
                onListDrop={(port, x, y) => {
                  const p = byId.get(port);
                  if (p) setLayout(place(layout, port, i, x - p.w / 2, y - p.h / 2));
                }}
                onRemove={layout.sheets.length > 1 && sheet.length === 0 ? () => setLayout(removeSheet(layout, i)) : null}
              />
            ))}
          </div>
        </div>
      ) : null}
    </>
  );
}

const portText = (p: Port) => `${p.label} · ${Math.round(p.w)} x ${Math.round(p.h)} mm${p.estimate ? " ~" : ""}`;

/** A client point in a sheet's paper mm, or null when the browser cannot say. */
function toPaper(svg: SVGSVGElement, clientX: number, clientY: number): { x: number; y: number } | null {
  const ctm = svg.getScreenCTM();
  if (!ctm) return null;
  const p = new DOMPoint(clientX, clientY).matrixTransform(ctm.inverse());
  return { x: p.x, y: p.y };
}

function SheetBox({
  index,
  places,
  byId,
  bigIds,
  size,
  drag,
  onGrab,
  onMove,
  onDrop,
  onListDrop,
  onRemove,
}: {
  index: number;
  places: Layout["sheets"][number];
  byId: Map<string, Port>;
  bigIds: ReadonlySet<string>;
  size: PaperSize;
  drag: Drag | null;
  onGrab: (d: Drag) => void;
  onMove: (x: number, y: number) => void;
  onDrop: (clientX: number, clientY: number) => void;
  onListDrop: (port: string, x: number, y: number) => void;
  onRemove: (() => void) | null;
}) {
  const ref = useRef<SVGSVGElement>(null);
  const [W, H] = PAPER_MM[size];
  // A label shrinks to fit its view: a 1:25 elevation can be a few centimetres wide on an A1 sheet.
  const fontSize = (p: Port) => Math.min(Math.max(W, H) / 70, p.h * 0.3, p.w / (p.label.length * 0.62));
  const at = (e: { clientX: number; clientY: number }) => (ref.current ? toPaper(ref.current, e.clientX, e.clientY) : null);

  const grab = (e: ReactPointerEvent<SVGGElement>, port: string, x: number, y: number) => {
    const p = at(e);
    if (!p) return;
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    onGrab({ port, sheet: index, grabX: p.x - x, grabY: p.y - y, startX: p.x, startY: p.y, x: p.x, y: p.y, moved: false });
  };

  return (
    <figure className="preview rlsSheet">
      <figcaption className="hint">
        Sheet {index + 1}
        {onRemove ? (
          <button className="chip" onClick={onRemove}>
            Remove
          </button>
        ) : null}
      </figcaption>
      <svg
        ref={ref}
        data-sheet={index}
        viewBox={`-5 -5 ${W + 10} ${H + 10}`}
        role="img"
        aria-label={`Sheet ${index + 1}`}
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => {
          e.preventDefault();
          const port = e.dataTransfer.getData("text/x-rls-port");
          const p = at(e);
          if (port && p) onListDrop(port, p.x, p.y);
        }}
      >
        <rect className="paper" x={0} y={0} width={W} height={H} />
        <rect className="margin" x={MARGIN_MM} y={MARGIN_MM} width={W - 2 * MARGIN_MM} height={H - 2 * MARGIN_MM} />
        {places.map((pl) => {
          const port = byId.get(pl.port);
          if (!port) return null;
          const dragging = drag?.port === pl.port && drag.sheet === index && drag.moved;
          const x = dragging ? drag.x - drag.grabX : pl.x;
          const y = dragging ? drag.y - drag.grabY : pl.y;
          return (
            <g
              key={pl.port}
              className={`port ${port.kind}${port.estimate ? " estimate" : ""}${dragging ? " dragging" : ""}${bigIds.has(pl.port) ? " tooBig" : ""}`}
              onPointerDown={(e) => grab(e, pl.port, pl.x, pl.y)}
              onPointerMove={(e) => {
                const p = drag?.port === pl.port ? at(e) : null;
                if (p) onMove(p.x, p.y);
              }}
              onPointerUp={(e) => drag?.port === pl.port && onDrop(e.clientX, e.clientY)}
            >
              <title>{portText(port)}</title>
              <rect x={x} y={y} width={port.w} height={port.h} />
              <text x={x + port.w / 2} y={y + port.h / 2} fontSize={fontSize(port)} textAnchor="middle" dominantBaseline="central">
                {port.label}
              </text>
            </g>
          );
        })}
      </svg>
    </figure>
  );
}
