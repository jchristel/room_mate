// A task's sheets as the preview draws them, and what a person can change on them while the task is queued: where a
// viewport goes (drag it) and which way a 3D view looks (drag the ends of its arrow). Nothing is changed until it is
// saved; the page then shows the task the server answers with.

import { useEffect, useState } from "react";

import { SheetPreview } from "../ui/sheet/SheetPreview.js";
import { aimsBody, movesBody, type Aims, type Centres } from "./saveBodies.js";
import { sheetStatus, taskSheetPreviews } from "./sheetPreviews.js";
import type { CameraAimsBody, Task, ViewportMovesBody } from "./types.js";

export interface SheetsProps {
  task: Task;
  busy: boolean;
  /** Save where viewports were moved (`POST .../viewports`). */
  onMoveViewports(body: ViewportMovesBody): void;
  /** Save the headings the 3D views were aimed along (`POST .../cameras`); the server works out what each hides again. */
  onAimCameras(body: CameraAimsBody): void;
}

export function TaskSheets({ task, busy, onMoveViewports, onAimCameras }: SheetsProps) {
  const t = task;
  // centres a person has dragged and not saved, by viewport key; gone when the task is read again
  const [draft, setDraft] = useState<Centres>(new Map());
  // headings a person has set for the 3D views and not saved, by view index
  const [aims, setAims] = useState<Aims>(new Map());
  useEffect(() => {
    setDraft(new Map());
    setAims(new Map());
  }, [t.id, t.updated_at]);
  const previews = taskSheetPreviews(t, draft, aims);
  const fit = sheetStatus(t, draft, aims);
  // only a task still waiting in the queue: a session holds the others and a finished one is history
  const editable = t.state === "queued" && (t.kind === "create_rls" || t.kind === "create_rls_set");
  const body = movesBody(t, draft);
  const moved = (t.rls?.viewport_moves?.length ?? 0) > 0;
  // the 3D views can be aimed on a task that kept its room's outline, which the server turns them over again against
  const aimable = editable && t.kind === "create_rls" && (t.rls?.room_outline?.length ?? 0) >= 3 && (t.rls?.three_ds?.length ?? 0) > 0;
  const aimBody = aimsBody(t, aims);
  const aimed = (t.rls?.three_ds ?? []).some((c) => c.direction);
  if (previews.length === 0) return null;
  return (
    <>
      <h3>{previews.length === 1 ? "Sheet preview" : `Sheet previews (${previews.length})`}</h3>
      <p className="hint">
        {t.rls_set
          ? "The sheets as the set lays them out. Schedule and 3D sizes are estimates: Revit decides them."
          : "The template's viewports on the paper the task asks for, where the template puts them. The plan, the elevations and the 3D views are sized for this room; schedules keep the template's size, which Revit decides."}
      </p>
      {fit.state === "clipping" ? (
        <p className="banner error">
          ⚠ {fit.clipped} viewport{fit.clipped === 1 ? "" : "s"} past the sheet's edge (outlined in red). Revit will place {fit.clipped === 1 ? "it" : "them"} there.
        </p>
      ) : null}
      <div className="sheet-previews">
        {previews.map((p, i) => (
          <section key={i}>
            <div className="sub">{p.title}</div>
            <SheetPreview
              record={p.record}
              {...(p.room && { room: p.room })}
              {...(aimable && {
                onAim: (index: number, heading: [number, number]) => setAims((cur) => new Map(cur).set(index, heading)),
              })}
              {...(editable && {
                onMove: (port: number, centre: [number, number]) => {
                  const key = p.keys[port];
                  if (key) setDraft((cur) => new Map(cur).set(key, centre));
                },
              })}
            />
          </section>
        ))}
      </div>
      {editable ? (
        <div className="toolbar">
          <span className="hint">Drag a viewport to move it on the sheet; its size stays.</span>
          <span className="spacer" />
          <button type="button" disabled={busy || draft.size === 0} onClick={() => setDraft(new Map())}>
            Discard moves
          </button>
          {t.kind === "create_rls" && moved ? (
            <button type="button" disabled={busy} onClick={() => onMoveViewports({ moves: [] })}>
              Back to the template's places
            </button>
          ) : null}
          <button type="button" disabled={busy || !body} onClick={() => body && onMoveViewports(body)}>
            Save positions
          </button>
        </div>
      ) : null}
      {aimable ? (
        <div className="toolbar">
          <span className="hint">Drag either end of a 3D view's green arrow on the plan to turn it. The walls it hides are worked out again when you save.</span>
          <span className="spacer" />
          <button type="button" disabled={busy || aims.size === 0} onClick={() => setAims(new Map())}>
            Discard directions
          </button>
          {aimed ? (
            <button type="button" disabled={busy} onClick={() => onAimCameras({ cameras: [] })}>
              Back to the template's direction
            </button>
          ) : null}
          <button type="button" disabled={busy || !aimBody} onClick={() => aimBody && onAimCameras(aimBody)}>
            Save directions
          </button>
        </div>
      ) : null}
    </>
  );
}
