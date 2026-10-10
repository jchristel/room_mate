// The saved routes, as a table on the LEFT of the plan while a route tool is open.
//
// Columns: whether it is drawn, the name, the colour (changed here), the levels it
// runs between, and open and delete. Beside the connections table and stacked with
// it when both tools are open; with neither open the column costs no width. Which
// routes are drawn is the reader's own (the checkbox), the routes themselves are the
// project's, so a delete or a colour change is what everyone sees.

import { useState } from "react";

import {
  bodyAfterDeleteRoute,
  bodyAfterRecolourRoute,
  connectionsUrl,
  routeTableRows,
  type SavedRoute,
} from "../connections.js";
import { ColourInput } from "./ColourInput.js";
import { saveConnections } from "./connectionsApi.js";
import { patchZoneRoute, setSavedRouteShown } from "./store.js";
import { useViewer } from "./useViewer.js";

export function RoutesPanel() {
  const { scope, payload, connections, zones, toolFocus, savedRoutes } = useViewer();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // The zone whose route tool takes an `open`: the one a pick last went to, else the first.
  const tool = zones.find((z) => z.id === toolFocus && z.route) ?? zones.find((z) => z.route);
  if (!tool) return null;

  const doc = connections.projectId === scope.projectId ? connections.doc : null;
  const url = connectionsUrl(scope);
  const levels = new Map((payload?.levels ?? []).map((l) => [l.id, l]));
  const roomLevel = new Map(
    (payload?.rooms ?? []).map((r) => {
      const l = r.level_id ? levels.get(r.level_id) : undefined;
      return [r.id, l ? { name: l.name, elevation: l.elevation } : null] as const;
    }),
  );
  const rows = routeTableRows(doc, (id) => roomLevel.get(id) ?? null);
  const saved = new Map((doc?.routes ?? []).map((r) => [r.id, r]));

  const open = (r: SavedRoute) =>
    patchZoneRoute(tool.id, {
      start: r.from.room_id,
      end: r.to.room_id,
      startAt: r.from_at ?? null,
      endAt: r.to_at ?? null,
      method: r.method ?? null,
      width: r.width_mm ? String(r.width_mm) : "",
      height: r.height_mm ? String(r.height_mm) : "",
      notice: null,
      result: { state: "idle" },
    });

  const write = async (body: Parameters<typeof saveConnections>[3]) => {
    if (!doc || !url) return;
    setBusy(true);
    setError(null);
    const refused = await saveConnections(url, scope.projectId, doc, body);
    setBusy(false);
    if (refused) setError(refused);
  };

  const remove = (id: string, name: string) => {
    if (!doc || !window.confirm(`Delete the saved route "${name}"?`)) return;
    void write(bodyAfterDeleteRoute(doc, id));
  };

  return (
    <aside id="routesPanel" className="leftPanel" aria-label="Saved routes">
      <div className="insp-head">
        <div className="insp-title">Saved routes</div>
        <div className="insp-sub">{rows.length} saved · {savedRoutes.shown.size} shown</div>
      </div>
      {connections.error ? <div className="insp-note">Could not read the saved routes: {connections.error}</div> : null}
      {error ? <div className="insp-note routeNotice">{error}</div> : null}
      {!doc ? (
        <div className="insp-note">Reading the saved routes…</div>
      ) : rows.length === 0 ? (
        <div className="insp-note">Nothing saved yet. Find a route, name it and press Save route.</div>
      ) : (
        <table className="connTable">
          <thead>
            <tr>
              <th className="fit" aria-label="Shown" />
              <th>Name</th>
              <th className="fit">Colour</th>
              <th className="fit">Levels</th>
              <th className="fit">Size</th>
              <th className="fit" aria-label="Open and delete" colSpan={2} />
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => {
              const on = savedRoutes.shown.has(row.id);
              const result = savedRoutes.results[row.id];
              const problem = on && result?.state === "error" ? result.message : null;
              const route = saved.get(row.id);
              return (
                <tr key={row.id}>
                  <td className="fit">
                    <input
                      type="checkbox"
                      checked={on}
                      onChange={() => setSavedRouteShown(row.id, !on)}
                      aria-label={`Show ${row.name} on the plan`}
                      title={on ? "Hide this route on the plan" : "Show this route on the plan"}
                    />
                  </td>
                  <td title={problem ?? undefined}>
                    {row.name}
                    {problem ? <span className="routeNotice"> ⚠ {problem}</span> : null}
                  </td>
                  <td className="fit">
                    <ColourInput
                      value={row.colour}
                      label={`Colour of ${row.name}`}
                      onCommit={(c) => doc && void write(bodyAfterRecolourRoute(doc, row.id, c))}
                    />
                  </td>
                  <td className="fit">{row.levels || "—"}</td>
                  <td className="fit" title="Width × height in mm of the object this route was saved for">
                    {sizeText(route?.width_mm, route?.height_mm)}
                  </td>
                  <td className="fit">
                    <button className="link" disabled={!route} onClick={() => route && open(route)} title="Put this route in the tool, to see its steps">
                      open
                    </button>
                  </td>
                  <td className="fit">
                    <button className="link" disabled={busy} onClick={() => remove(row.id, row.name)} title="Delete this saved route">
                      delete
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </aside>
  );
}

/** The object a route was saved for, as "1200 × 2000 mm" with whichever of the two it has. */
function sizeText(width: number | null | undefined, height: number | null | undefined): string {
  if (width && height) return `${width} × ${height} mm`;
  if (width) return `${width} wide`;
  if (height) return `${height} tall`;
  return "—";
}
