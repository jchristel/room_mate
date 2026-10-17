// The saved reaches, as a table on the LEFT of the plan while a reach tool is open.
//
// Beside the connections and routes tables, stacked with them when more than one is
// open; with none open the column costs no width. A saved reach is the REQUEST the
// project keeps (start rooms, walk, grid, method), never a map: "show" puts it into
// the reach tool of the zone a pick last went to and measures it, so the map is the
// answer as the building stands now. Each zone has its own tool, so two saved reaches
// can be shown side by side to compare them.

import { useState } from "react";

import { bodyAfterDeleteReach, connectionsUrl, reachesOf } from "../connections.js";
import { describeSaved, stateFromSaved } from "../reach.js";
import { saveConnections } from "./connectionsApi.js";
import { abortReach, measureReach } from "./reachMeasure.js";
import { patchZoneReach } from "./store.js";
import { useViewer } from "./useViewer.js";

export function ReachesPanel() {
  const { scope, payload, connections, zones, toolFocus } = useViewer();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // The zone that takes a "show": the one a pick last went to, else the first.
  const tool = zones.find((z) => z.id === toolFocus && z.reach) ?? zones.find((z) => z.reach);
  if (!tool?.reach) return null;

  const doc = connections.projectId === scope.projectId ? connections.doc : null;
  const url = connectionsUrl(scope);
  const rows = [...reachesOf(doc)].sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));
  const roomNames = new Map((payload?.rooms ?? []).map((r) => [r.id, r.name || r.id]));
  const opacity = tool.reach.opacity;

  const show = (id: string) => {
    const saved = rows.find((r) => r.id === id);
    if (!saved) return;
    abortReach(tool.id);
    patchZoneReach(tool.id, stateFromSaved(saved, opacity));
    void measureReach(tool.id, scope);
  };

  const remove = async (id: string, name: string) => {
    if (!doc || !url || !window.confirm(`Delete the saved reach "${name}"?`)) return;
    setBusy(true);
    setError(null);
    const refused = await saveConnections(url, scope.projectId, doc, bodyAfterDeleteReach(doc, id));
    setBusy(false);
    if (refused) setError(refused);
  };

  return (
    <aside id="reachesPanel" className="leftPanel" aria-label="Saved reaches">
      <div className="insp-head">
        <div className="insp-title">Saved reaches</div>
        <div className="insp-sub">{rows.length} saved</div>
      </div>
      {connections.error ? <div className="insp-note">Could not read the saved reaches: {connections.error}</div> : null}
      {error ? <div className="insp-note routeNotice">{error}</div> : null}
      {!doc ? (
        <div className="insp-note">Reading the saved reaches…</div>
      ) : rows.length === 0 ? (
        <div className="insp-note">Nothing saved yet. Pick start rooms, measure, name it and press Save reach.</div>
      ) : (
        <table className="connTable">
          <thead>
            <tr>
              <th>Name</th>
              <th>Asks</th>
              <th className="fit" aria-label="Show and delete" colSpan={2} />
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.id}>
                <td title={r.sources.map((s) => roomNames.get(s.room.room_id) ?? s.room.room_id).join(", ")}>{r.name}</td>
                <td>{describeSaved(r)}</td>
                <td className="fit">
                  <button className="link" onClick={() => show(r.id)} title="Measure this reach in the zone the reach tool is in, replacing what it holds">
                    show
                  </button>
                </td>
                <td className="fit">
                  <button className="link" disabled={busy} onClick={() => void remove(r.id, r.name)} title="Delete this saved reach">
                    delete
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </aside>
  );
}
