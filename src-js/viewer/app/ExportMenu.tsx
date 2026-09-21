// "Export SVGs": which levels to export, behind the zone's export button.
//
// **The zone's visibility, not a second set of toggles.** What gets drawn is
// what this zone's Layers menu says -- rooms, labels, each element layer, the
// spaces model -- so the only new question here is WHICH LEVELS. A second set of
// layer checkboxes in this menu would be two answers to one question, and the
// first time they disagreed the file would not match the screen.
//
// **It records the levels turned OFF**, as the property chooser records hidden
// properties (`keepChosen`): a level that arrives with a later push is ticked,
// so "Export" keeps meaning every level until the reader says otherwise, which
// is what the button did before it had a menu. Counted against the levels
// OFFERED, so a level that has since left the payload neither counts nor
// lingers in the label.
//
// **Local state, not the store.** `DropMenu` hides its panel rather than
// unmounting it, so the choice lasts as long as the zone does; it is a way of
// asking for files, not a view preference anything else reads.

import { useState } from "react";

import { levelLabel, pickerOrder, roomsOnLevel } from "../levels.js";
import type { ColourPlan } from "../colour.js";
import { DropMenu } from "./DropMenu.js";
import { exportLevels, type ExportResult } from "./svgExport.js";
import type { ZoneRow } from "./store.js";
import { useViewer } from "./useViewer.js";
import type { Level } from "../../renderer/types.js";

export function ExportMenu({
  zone,
  levels,
  plan,
  errorRooms,
}: {
  zone: ZoneRow;
  levels: readonly Level[];
  plan: ColourPlan | null;
  errorRooms: ReadonlySet<string>;
}) {
  const { payload, scope, showErrors, appearance } = useViewer();
  const [excluded, setExcluded] = useState<ReadonlySet<string>>(new Set());
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [report, setReport] = useState<string | null>(null);

  const ordered = pickerOrder(levels);
  // A level with no rooms has nothing to frame and can never produce a file, so
  // it is listed but not offered. RHH declares a second, empty "LEVEL 7" beside
  // the real one; ticked, it came back as "LEVEL 7: no rooms, skipped", which
  // reads as the export losing a 185-room floor.
  const offered = payload ? ordered.filter((l) => roomsOnLevel(payload, l.id).length > 0) : [];
  const chosen = offered.filter((l) => !excluded.has(l.id));
  const busy = progress !== null;

  const toggle = (id: string, on: boolean) => {
    const next = new Set(excluded);
    if (on) next.delete(id);
    else next.add(id);
    setExcluded(next);
  };

  const run = async () => {
    if (!payload || !chosen.length) return;
    setReport(null);
    setProgress({ done: 0, total: chosen.length });
    try {
      const result = await exportLevels(
        payload,
        scope,
        new Set(chosen.map((l) => l.id)),
        {
          plan,
          errorRoomIds: errorRooms,
          // Error highlighting follows the QA band, exactly as the plans on
          // screen do.
          showErrors,
          showLabels: zone.showLabels,
          showRooms: zone.showRooms,
          layers: zone.layers,
          spacesModel: zone.spacesModel,
          appearance,
        },
        (done, total) => setProgress({ done, total }),
      );
      setReport(summary(result));
    } finally {
      setProgress(null);
    }
  };

  const label = progress
    ? `Exporting ${Math.min(progress.done + 1, progress.total)} of ${progress.total}…`
    : chosen.length === offered.length
      ? "Export SVGs"
      : `Export SVGs: ${chosen.length} of ${offered.length}`;

  return (
    <DropMenu label={label} title="Export levels as SVG files, drawn with this zone's layers">
      <div className="menu-head">
        <div className="fields-actions">
          <a onClick={() => setExcluded(new Set())}>Select all</a>
          <a onClick={() => setExcluded(new Set(offered.map((l) => l.id)))}>Select none</a>
        </div>
        <button className="ctl export-run" disabled={!payload || !chosen.length || busy} onClick={() => void run()}>
          {busy ? label : `Export ${chosen.length} level${chosen.length === 1 ? "" : "s"}`}
        </button>
        {report ? <div className="export-report">{report}</div> : null}
      </div>
      {offered.length ? null : <label className="menu-off">No levels to export</label>}
      {payload
        ? ordered.map((level) =>
            offered.includes(level) ? (
              <label key={level.id}>
                <input type="checkbox" checked={!excluded.has(level.id)} onChange={(e) => toggle(level.id, e.target.checked)} />{" "}
                {levelLabel(payload, level)}
              </label>
            ) : (
              <label key={level.id} className="menu-off" title="No rooms on this level, so nothing to frame">
                <input type="checkbox" checked={false} disabled /> {levelLabel(payload, level)}
              </label>
            ),
          )
        : null}
    </DropMenu>
  );
}

/** One line on what happened. Every shortfall is named: a level quietly
 *  missing from the downloads reads as a picker that did not work. */
function summary(r: ExportResult): string {
  const parts = [`Exported ${r.exported} file${r.exported === 1 ? "" : "s"}.`];
  if (r.empty.length) parts.push(`No rooms, skipped: ${r.empty.join(", ")}.`);
  if (r.partial.length) parts.push(`A layer read failed on: ${r.partial.join(", ")} (named in each file).`);
  return parts.join(" ");
}
