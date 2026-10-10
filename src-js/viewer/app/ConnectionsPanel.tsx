// The connections table: every saved open area and vertical link in one list, on
// the LEFT of the plan while a connections editor is open.
//
// Three columns: the name, the type (open area or vertical) and a delete. A row
// loads that record into the open editor for changing, as the chips in the editor
// bar used to, and the one being edited is marked. It is its own column and not the
// properties panel, so the two can be open together: picking a room to see its
// properties while deciding what to connect is the ordinary thing to do, and a
// panel shared between them would force a choice. With no editor open the column
// costs no width.

import { useState } from "react";

import {
  bodyAfterDeleteLink,
  bodyAfterDeleteZone,
  connectionRows,
  connectionsUrl,
  editOf,
  editOfLink,
  type ConnectionRow,
} from "../connections.js";
import { saveConnections } from "./connectionsApi.js";
import { patchZoneEdit } from "./store.js";
import { useViewer } from "./useViewer.js";

export function ConnectionsPanel() {
  const { scope, payload, connections, zones, toolFocus } = useViewer();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // The zone whose editor takes a row: the one a pick last went to, else the first.
  const editing = zones.find((z) => z.id === toolFocus && z.edit) ?? zones.find((z) => z.edit);
  if (!editing) return null;
  const edit = editing.edit!;

  const doc = connections.projectId === scope.projectId ? connections.doc : null;
  const url = connectionsUrl(scope);
  const names = new Map((payload?.rooms ?? []).map((r) => [r.id, r.name || r.id]));
  const levels = new Map((payload?.levels ?? []).map((l) => [l.id, l]));
  const roomLevel = new Map(
    (payload?.rooms ?? []).map((r) => {
      const l = r.level_id ? levels.get(r.level_id) : undefined;
      return [r.id, l ? { name: l.name, elevation: l.elevation } : null] as const;
    }),
  );
  const rows = connectionRows(
    doc,
    (id) => names.get(id) ?? id,
    (id) => roomLevel.get(id) ?? null,
  );

  const load = (row: ConnectionRow) => {
    if (!doc) return;
    if (row.kind === "open") {
      const z = doc.zones.find((x) => x.id === row.id);
      if (z) patchZoneEdit(editing.id, editOf(z));
      return;
    }
    const l = doc.links.find((x) => x.id === row.id);
    if (!l) return;
    // A stack is read from one room, so a link opens it at the link's first room.
    patchZoneEdit(
      editing.id,
      edit.kind === "stack"
        ? { ...edit, members: [{ room_id: l.a.room_id, model_id: l.a.model_id }], error: null }
        : editOfLink(l),
    );
  };

  const remove = async (row: ConnectionRow) => {
    if (!doc || !url) return;
    const what = row.kind === "open" ? `the open area "${row.name}"` : `the vertical link ${row.name}`;
    if (!window.confirm(`Delete ${what}? Routes that used it will stop using it.`)) return;
    setBusy(true);
    setError(null);
    const body = row.kind === "open" ? bodyAfterDeleteZone(doc, row.id) : bodyAfterDeleteLink(doc, row.id);
    const refused = await saveConnections(url, scope.projectId, doc, body);
    setBusy(false);
    if (refused) setError(refused);
    // Deleting the record being edited leaves the editor with nothing to change.
    else if (edit.id === row.id && edit.kind === row.kind) {
      patchZoneEdit(editing.id, { ...edit, id: null, members: [], name: "", error: null });
    }
  };

  return (
    <aside id="connectionsPanel" className="leftPanel" aria-label="Saved connections">
      <div className="insp-head">
        <div className="insp-title">Connections</div>
        <div className="insp-sub">
          {doc?.zones.length ?? 0} open · {doc?.links.length ?? 0} vertical
        </div>
      </div>
      {connections.error ? <div className="insp-note">Could not read the saved connections: {connections.error}</div> : null}
      {error ? <div className="insp-note routeNotice">{error}</div> : null}
      {!doc ? (
        <div className="insp-note">Reading the saved connections…</div>
      ) : rows.length === 0 ? (
        <div className="insp-note">Nothing saved yet. Draw an open area or a vertical link and save it.</div>
      ) : (
        <table className="connTable">
          <thead>
            <tr>
              <th>Name</th>
              <th className="fit">Type</th>
              <th className="fit">Level</th>
              <th className="fit" aria-label="Delete" />
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr
                key={`${row.kind}:${row.id}`}
                className={edit.id === row.id && edit.kind === row.kind ? "editing" : undefined}
              >
                <td>
                  <button className="link" onClick={() => load(row)} title="Load this to change it">
                    {row.name}
                  </button>
                </td>
                <td className="fit">{row.type}</td>
                <td className="fit">{row.level || "—"}</td>
                <td className="fit">
                  <button className="link" disabled={busy} onClick={() => void remove(row)} title="Delete this">
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
