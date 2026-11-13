// The multi-room RLS composer's rules, with no store and no DOM (the RLS tool's shape).
//
// The server's set preview (`POST .../tasks/rls-set-preview`) says which views a set of adjacent rooms gets and how
// big each is on paper; it places none of them. Where they go is a person's decision, so the layout lives here: a
// list of sheets, each a list of views at a point in paper millimetres (y down, the box's top left corner), and the
// views left out. A first layout is made automatically, and a person moves views, adds and removes sheets, and leaves
// views out. Saving queues ONE `create_rls_set` task for the set (`setTaskBody`).
//
// **One view, one sheet**, schedules included for now: a schedule on several sheets is a split, which comes with
// schedule splitting. **Changing the paper size keeps every view where it is and at its size**; what no longer fits
// is a finding, never moved for the person.

/** The server's answer, as far as the composer reads it (`service::rls_set::SetPreview`). */
export interface SetPreview {
  valid: boolean;
  rooms: SetRoom[];
  lead_room?: string;
  suggestions: { sheet_id: string; sheet_number: string; sheet_name: string; fit: number }[];
  sheet_id?: string;
  views?: {
    plan: { scale: number; paper_mm: [number, number] };
    elevations: { room_id: string; marker_index: number; scale: number; paper_mm: [number, number]; height_mm: number }[];
    schedules: { room_id: string; paper_mm: [number, number]; estimate: boolean }[];
    three_d?: { paper_mm: [number, number]; estimate: boolean };
  };
  findings: { kind: string; message?: string; [k: string]: unknown }[];
}

export interface SetRoom {
  room_id: string;
  model_id: string;
  name: string;
  number?: string;
  area_m2: number;
  height_mm: number;
  height_from: string;
}

export type ViewKind = "plan" | "elevation" | "schedule" | "three_d";

/** A view to place: its size on paper and what to call it. `estimate`: Revit decides the real size. */
export interface Port {
  id: string;
  kind: ViewKind;
  roomId?: string;
  label: string;
  w: number;
  h: number;
  estimate: boolean;
}

export interface Place {
  port: string;
  x: number;
  y: number;
}

export interface Layout {
  sheets: Place[][];
  leftOut: string[];
}

export type PaperSize = "a0" | "a1" | "a2" | "a3";

/** ISO A sizes, landscape, mm. */
export const PAPER_MM: Record<PaperSize, [number, number]> = {
  a0: [1189, 841],
  a1: [841, 594],
  a2: [594, 420],
  a3: [420, 297],
};

/** The border kept clear on every side. The server does not know a title block's drawable area yet (the plan's
 *  "margins per size" question), so this is a stand-in, and said so on the page. */
export const MARGIN_MM = 15;
/** The space the first layout leaves between views. */
export const GAP_MM = 10;

/** The sheet size a template's measured paper is, or null when it is none of A0..A3 (within 5 mm, either way up). */
export function sizeOfPaper(mm: { width_mm: number; height_mm: number } | undefined): PaperSize | null {
  if (!mm) return null;
  const long = Math.max(mm.width_mm, mm.height_mm);
  const short = Math.min(mm.width_mm, mm.height_mm);
  for (const [k, [w, h]] of Object.entries(PAPER_MM) as [PaperSize, [number, number]][]) {
    if (Math.abs(long - w) <= 5 && Math.abs(short - h) <= 5) return k;
  }
  return null;
}

const MARKER_SIDE: Record<number, string> = { 0: "W", 2: "N", 4: "E", 6: "S" };

/** The views of a preview, in the order a first layout takes them: the plan, the 3D view, then room by room (the
 *  lead first) its four elevations and its schedule, so a room's views stay together. */
export function portsOf(p: SetPreview): Port[] {
  const v = p.views;
  if (!p.valid || !v) return [];
  const nameOf = (id: string) => {
    const r = p.rooms.find((x) => x.room_id === id);
    // A room's name often carries its number already (House A's "OFFICE 00.02"): say it once.
    if (!r) return id;
    return r.number && !r.name.includes(r.number) ? `${r.name} ${r.number}` : r.name;
  };
  const rooms = [...p.rooms.map((r) => r.room_id)].sort((a, b) => Number(b === p.lead_room) - Number(a === p.lead_room));
  const out: Port[] = [{ id: "plan", kind: "plan", label: `Plan 1:${v.plan.scale}`, w: v.plan.paper_mm[0], h: v.plan.paper_mm[1], estimate: false }];
  if (v.three_d) out.push({ id: "three_d", kind: "three_d", label: "3D", w: v.three_d.paper_mm[0], h: v.three_d.paper_mm[1], estimate: v.three_d.estimate });
  for (const room of rooms) {
    for (const e of v.elevations.filter((x) => x.room_id === room)) {
      out.push({
        id: `elevation:${room}:${e.marker_index}`,
        kind: "elevation",
        roomId: room,
        label: `${nameOf(room)} ${MARKER_SIDE[e.marker_index] ?? `#${e.marker_index}`} 1:${e.scale}`,
        w: e.paper_mm[0],
        h: e.paper_mm[1],
        estimate: false,
      });
    }
    const s = v.schedules.find((x) => x.room_id === room);
    if (s) out.push({ id: `schedule:${room}`, kind: "schedule", roomId: room, label: `${nameOf(room)} schedule`, w: s.paper_mm[0], h: s.paper_mm[1], estimate: s.estimate });
  }
  return out;
}

/** A first layout: views in `portsOf`'s order, left to right in rows, a new sheet when a row would run off the
 *  bottom. A view larger than the drawable area gets a sheet of its own and is left for the findings to report. */
export function autoLayout(ports: readonly Port[], size: PaperSize, leftOut: readonly string[] = []): Layout {
  const [W, H] = PAPER_MM[size];
  const right = W - MARGIN_MM;
  const bottom = H - MARGIN_MM;
  const sheets: Place[][] = [];
  let sheet: Place[] = [];
  let x = MARGIN_MM;
  let y = MARGIN_MM;
  let row = 0;
  const newSheet = () => {
    if (sheet.length > 0) sheets.push(sheet);
    sheet = [];
    x = MARGIN_MM;
    y = MARGIN_MM;
    row = 0;
  };
  for (const p of ports) {
    if (leftOut.includes(p.id)) continue;
    if (p.w > right - MARGIN_MM || p.h > bottom - MARGIN_MM) {
      newSheet();
      sheets.push([{ port: p.id, x: MARGIN_MM, y: MARGIN_MM }]);
      continue;
    }
    if (x + p.w > right && x > MARGIN_MM) {
      x = MARGIN_MM;
      y += row + GAP_MM;
      row = 0;
    }
    if (y + p.h > bottom && y > MARGIN_MM) newSheet();
    sheet.push({ port: p.id, x, y });
    x += p.w + GAP_MM;
    row = Math.max(row, p.h);
  }
  if (sheet.length > 0 || sheets.length === 0) sheets.push(sheet);
  return { sheets, leftOut: [...leftOut] };
}

/** The sheet a view is on, or -1. */
export const sheetOf = (l: Layout, port: string): number => l.sheets.findIndex((s) => s.some((p) => p.port === port));

/** Put a view on a sheet at a point (its top left corner), taking it off any other: one view, one sheet. A view that
 *  was left out is put back. Whole millimetres, so a drag does not store float noise. */
export function place(l: Layout, port: string, sheet: number, x: number, y: number): Layout {
  const at: Place = { port, x: Math.round(x), y: Math.round(y) };
  const sheets = l.sheets.map((s, i) => {
    const rest = s.filter((p) => p.port !== port);
    return i === sheet ? [...rest, at] : rest;
  });
  return { sheets, leftOut: l.leftOut.filter((p) => p !== port) };
}

/** Take a view off its sheet; it waits in the list to be placed. */
export const unplace = (l: Layout, port: string): Layout => ({ ...l, sheets: l.sheets.map((s) => s.filter((p) => p.port !== port)) });

/** Leave a view out of the set: it is not made. */
export const leaveOut = (l: Layout, port: string): Layout => {
  const off = unplace(l, port);
  return { ...off, leftOut: off.leftOut.includes(port) ? off.leftOut : [...off.leftOut, port] };
};

/** Put a left-out view back in the list to place. */
export const putBack = (l: Layout, port: string): Layout => ({ ...l, leftOut: l.leftOut.filter((p) => p !== port) });

export const addSheet = (l: Layout): Layout => ({ ...l, sheets: [...l.sheets, []] });

/** Remove a sheet, only an empty one: a sheet with views on it is emptied by a person first, so nothing is dropped
 *  by a click on the wrong sheet. The last sheet stays. */
export function removeSheet(l: Layout, sheet: number): Layout {
  if (l.sheets.length <= 1 || (l.sheets[sheet]?.length ?? 0) > 0) return l;
  return { ...l, sheets: l.sheets.filter((_, i) => i !== sheet) };
}

/** The views neither on a sheet nor left out. */
export function unplaced(l: Layout, ports: readonly Port[]): Port[] {
  const on = new Set(l.sheets.flat().map((p) => p.port));
  return ports.filter((p) => !on.has(p.id) && !l.leftOut.includes(p.id));
}

export interface LayoutFinding {
  code: "too_big" | "not_placed" | "off_sheet" | "overlap" | "empty_sheet";
  sheet?: number;
  ports: string[];
  message: string;
  /** No moving of views fixes it: the paper, the scale or the set must change. Shown first and loudest. */
  serious?: boolean;
}

/** Whether a view fits inside the border of a paper size at all, wherever it is put. */
export const fitsOn = (p: Port, size: PaperSize): boolean => {
  const [W, H] = PAPER_MM[size];
  return p.w <= W - 2 * MARGIN_MM && p.h <= H - 2 * MARGIN_MM;
};

/** The smallest A size a view fits on, or null when not even A0 holds it. */
export function smallestFit(p: Port): PaperSize | null {
  return (["a3", "a2", "a1", "a0"] as const).find((s) => fitsOn(p, s)) ?? null;
}

/** "Plan 1:50 is too big for the sheet: it is 900 x 700 mm, but an A1 sheet holds 811 x 564 mm ..." The way out depends
 *  on whether a larger paper would do: a plan of a large set at 1:50 can need a smaller scale, which is a library
 *  sheet's, or (not built) a plan split into dependent views. */
function tooBigText(p: Port, size: PaperSize): string {
  const [W, H] = PAPER_MM[size];
  const mm = (n: number) => Math.round(n);
  const fit = smallestFit(p);
  const way = fit
    ? `It fits on ${fit.toUpperCase()}: choose that paper size, or a library sheet with a smaller scale.`
    : "No paper size up to A0 holds it: choose a library sheet with a smaller scale, fewer rooms, or leave it out.";
  return `${p.label} is too big for the sheet: it is ${mm(p.w)} x ${mm(p.h)} mm, but an ${size.toUpperCase()} sheet holds ${mm(W - 2 * MARGIN_MM)} x ${mm(H - 2 * MARGIN_MM)} mm inside its border. ${way}`;
}

/** What is wrong with a layout. Advice, never a gate: a person may save a layout with an overlap they mean. A view too
 *  big for the paper is said on its own, whether or not it is placed, and not again as running past an edge. */
export function layoutFindings(l: Layout, ports: readonly Port[], size: PaperSize): LayoutFinding[] {
  const [W, H] = PAPER_MM[size];
  const byId = new Map(ports.map((p) => [p.id, p]));
  const out: LayoutFinding[] = [];
  const tooBig = new Set<string>();
  for (const p of ports) {
    if (l.leftOut.includes(p.id) || fitsOn(p, size)) continue;
    tooBig.add(p.id);
    out.push({ code: "too_big", ports: [p.id], message: tooBigText(p, size), serious: true });
  }
  const waiting = unplaced(l, ports);
  if (waiting.length > 0)
    out.push({ code: "not_placed", ports: waiting.map((p) => p.id), message: `${waiting.length} view${waiting.length === 1 ? " is" : "s are"} not on a sheet` });
  l.sheets.forEach((sheet, i) => {
    if (sheet.length === 0) out.push({ code: "empty_sheet", sheet: i, ports: [], message: `Sheet ${i + 1} is empty` });
    const boxes = sheet.flatMap((p) => {
      const port = byId.get(p.port);
      return port ? [{ id: p.port, label: port.label, x: p.x, y: p.y, w: port.w, h: port.h }] : [];
    });
    for (const b of boxes) {
      if (tooBig.has(b.id)) continue;
      if (b.x < MARGIN_MM || b.y < MARGIN_MM || b.x + b.w > W - MARGIN_MM || b.y + b.h > H - MARGIN_MM)
        out.push({ code: "off_sheet", sheet: i, ports: [b.id], message: `${b.label} runs past the edge of sheet ${i + 1}` });
    }
    for (let a = 0; a < boxes.length; a++) {
      for (let b = a + 1; b < boxes.length; b++) {
        const p = boxes[a]!;
        const q = boxes[b]!;
        if (p.x < q.x + q.w && q.x < p.x + p.w && p.y < q.y + q.h && q.y < p.y + p.h)
          out.push({ code: "overlap", sheet: i, ports: [p.id, q.id], message: `${p.label} overlaps ${q.label} on sheet ${i + 1}` });
      }
    }
  });
  return out;
}

/** Keep a layout across a new preview when it names the same views (a new scale changes sizes, not which views
 *  there are); otherwise lay out again. */
export function carryLayout(old: Layout | null, ports: readonly Port[], size: PaperSize): Layout {
  if (old) {
    const ids = new Set(ports.map((p) => p.id));
    const had = new Set([...old.sheets.flat().map((p) => p.port), ...old.leftOut]);
    if (ids.size === had.size && [...ids].every((id) => had.has(id))) return old;
  }
  return autoLayout(ports, size);
}

/** The text of a server finding, for the bar. */
export function findingText(f: SetPreview["findings"][number]): string {
  return typeof f.message === "string" && f.message ? f.message : f.kind.replace(/_/g, " ");
}

/** The `create_rls_set` request (`service::rls_set_task::SetTaskRequest`): the rooms, the library sheet the views were
 *  worked out from, the paper, and per sheet each view's id at its top left corner. The server works the views out
 *  again from the rooms and sheet, so only WHERE goes over the wire, never a size or a crop. */
export interface SetTaskBody {
  kind: "create_rls_set";
  room_ids: string[];
  sheet_id: string;
  sheet_size: PaperSize;
  overwrite_rls: boolean;
  sheets: { views: { view: string; x_mm: number; y_mm: number }[] }[];
  left_out: string[];
}

export function setTaskBody(roomIds: readonly string[], sheetId: string, size: PaperSize, overwrite: boolean, l: Layout): SetTaskBody {
  return {
    kind: "create_rls_set",
    room_ids: [...roomIds],
    sheet_id: sheetId,
    sheet_size: size,
    overwrite_rls: overwrite,
    sheets: l.sheets.map((s) => ({ views: s.map((p) => ({ view: p.port, x_mm: p.x, y_mm: p.y })) })),
    left_out: [...l.leftOut],
  };
}

/** Why the layout cannot be saved yet, or null. The server refuses the same two (the layout must say exactly what is
 *  made); every other finding is advice and saves. */
export function saveBlocked(findings: readonly LayoutFinding[]): string | null {
  const waiting = findings.find((f) => f.code === "not_placed");
  if (waiting) return `${waiting.message}: place each view or leave it out`;
  const empty = findings.find((f) => f.code === "empty_sheet");
  if (empty) return `${empty.message}: remove it or put a view on it`;
  return null;
}
