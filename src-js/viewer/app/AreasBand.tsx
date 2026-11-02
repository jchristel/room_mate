// Band 1's hierarchy-areas block: every level's footprint against its summed
// net room area, at one tier.
//
// **All levels, where a zone's overlay is one level.** The band belongs to the
// scope; the overlay belongs to a zone's level pick. They also have separate
// tier pickers for that reason — the question "what does this floor look like
// by department" and "what do the departments total across the job" are asked
// at different depths.

import { bandRows, buildAreasCsv, tierNames, type AreasData } from "../areas.js";
import { qualitative } from "../palette.js";
import { setAreasBandTier } from "./store.js";
import { useViewer } from "./useViewer.js";

export function AreasBand({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  const { areas, areasBandTier, payload, zones, scope } = useViewer();
  // The band appears when some zone is showing footprints: the data is fetched
  // on that trigger, and a block of figures with no plan to read them against
  // is not what the reader asked for.
  const anyOverlay = zones.some((z) => z.areasMode);
  if (!anyOverlay || !areas) return null;

  const tiers = tierNames(areas);
  const rows = bandRows(areas, payload?.rooms ?? [], areasBandTier);
  const fmt = (n: number) => n.toLocaleString(undefined, { maximumFractionDigits: 1 });
  let grandF = 0;
  let grandN = 0;

  return (
    <div className={`result-band${open ? " open" : ""}`} id="areasBand">
      <button className="band-head" id="areasHead" onClick={onToggle}>
        {open ? "▾" : "▸"} Hierarchy areas
      </button>
      {open ? (
        <div className="band-body" id="areasBody">
          <select
            className="picker"
            title="Tier for the figures below"
            value={areasBandTier}
            onChange={(e) => setAreasBandTier(Number(e.target.value))}
          >
            {tiers.map((name, depth) => (
              <option key={name} value={depth}>
                {name}
              </option>
            ))}
          </select>
          <button className="refresh" onClick={() => downloadAreasCsv(areas, payload?.rooms ?? [], scope.projectId)}>
            Download CSV
          </button>
          <div id="areasContent">
            {rows.size === 0 ? (
              <p className="note">No footprints at this tier.</p>
            ) : (
              <>
                <p className="note">
                  Dissolved footprint vs summed net room area (model units²), every level in scope. Δ = enclosed wall
                  bands + filled room voids (columns); open courtyards are excluded from the footprint.
                </p>
                <table>
                  <thead>
                    <tr>
                      <th className="lvl">Level</th>
                      <th>Group</th>
                      <th>Footprint</th>
                      <th>Net</th>
                      <th>Δ</th>
                    </tr>
                  </thead>
                  <tbody>
                    {[...rows.entries()].map(([levelId, levelRows]) => {
                      const tF = levelRows.reduce((s, r) => s + r.footprint, 0);
                      const tN = levelRows.reduce((s, r) => s + r.net, 0);
                      grandF += tF;
                      grandN += tN;
                      return (
                        <Fragmentish key={levelId}>
                          {levelRows.map((row, i) => (
                            <tr key={row.label + i}>
                              <td className="lvl">{i === 0 ? row.levelName : ""}</td>
                              <td>
                                {/* The swatch is the overlay's own colour, by
                                    the same per-level server order — so a
                                    figure here names the polygon on the plan. */}
                                <span className="swatch" style={{ background: qualitative("Set2", i) }} />
                                {row.label}
                                {row.countedUp ? null : <span className="uncounted"> (not counted up)</span>}
                              </td>
                              <td>{fmt(row.footprint)}</td>
                              <td>{fmt(row.net)}</td>
                              <td>{fmt(row.delta)}</td>
                            </tr>
                          ))}
                          <tr className="total">
                            <td className="lvl" />
                            <td>{levelRows[0]?.levelName} total</td>
                            <td>{fmt(tF)}</td>
                            <td>{fmt(tN)}</td>
                            <td>{fmt(tF - tN)}</td>
                          </tr>
                        </Fragmentish>
                      );
                    })}
                    <tr className="total">
                      <td className="lvl" />
                      <td>All levels</td>
                      <td>{fmt(grandF)}</td>
                      <td>{fmt(grandN)}</td>
                      <td>{fmt(grandF - grandN)}</td>
                    </tr>
                  </tbody>
                </table>
              </>
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
}

/** A keyed fragment, so a level's rows and its subtotal stay one unit inside
 *  `<tbody>` without an element the table layout would have to account for. */
function Fragmentish({ children }: { children: React.ReactNode }) {
  return <>{children}</>;
}

function downloadAreasCsv(areas: AreasData, rooms: Parameters<typeof buildAreasCsv>[1], projectId: string | null): void {
  const csv = buildAreasCsv(areas, rooms);
  if (!csv) return;
  const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `areas-${projectId || "project"}.csv`;
  a.click();
  URL.revokeObjectURL(a.href);
}
