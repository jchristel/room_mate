// The blueprint tool's bar: which blueprint, which room it is laid in, which rotation, and the way
// to assign it. Mounted inside a zone beside the route and reach bars, and renders nothing while the
// tool is off.
//
// Automatic like the route (and unlike reach): the answer is one room's placements, cheap, and the
// ghost is the thing being asked about, so it is read as soon as there is a blueprint and a room.
// Clicking a room moves the ghost there through the same choke point as the other tools
// (`store.select`), so a plan click, a grid row and a search chip all work.

import { useEffect, useState } from "react";

import { chooseBlueprint, describeTool, stepRotation } from "../blueprintTool.js";
import { abortResolve, assignTool, currentTool, listAssignments, listBlueprints, resolveTool, type AssignmentLite, type BlueprintRow } from "./blueprintData.js";
import { patchZoneBlueprint, setZoneBlueprintMode, type ZoneRow } from "./store.js";
import { useViewer } from "./useViewer.js";

export function BlueprintPanel({ zone }: { zone: ZoneRow }) {
  const { scope, payload, toolFocus } = useViewer();
  const tool = zone.blueprint;
  const zoneId = zone.id;
  const active = tool !== null;
  const project = scope.projectId;
  const [blueprints, setBlueprints] = useState<BlueprintRow[] | null>(null);
  const [assignments, setAssignments] = useState<AssignmentLite[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // The project's blueprints and assignments, read while the tool is on.
  useEffect(() => {
    if (!active || !project) return;
    const ac = new AbortController();
    setBlueprints(null);
    listBlueprints(project, ac.signal).then(setBlueprints).catch(() => !ac.signal.aborted && setBlueprints([]));
    listAssignments(project, ac.signal).then(setAssignments).catch(() => undefined);
    return () => ac.abort();
  }, [active, project]);

  // Ask again whenever what is asked changes: another blueprint, room, revision or rotation.
  const key = tool ? `${tool.blueprintId}|${tool.revision}|${tool.roomId}|${tool.shift}` : "";
  useEffect(() => {
    if (!active || !project) return;
    setNotice(null);
    void resolveTool(zoneId, project);
    return () => abortResolve(zoneId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, project, key]);

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && toolFocus === zoneId) setZoneBlueprintMode(zoneId, false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, toolFocus, zoneId]);

  if (!tool) return null;
  const room = payload?.rooms?.find((r) => r.id === tool.roomId);
  const done = tool.result.state === "done" ? tool.result.body : null;
  const rotate = (direction: 1 | -1) => {
    const now = currentTool(zoneId);
    if (!now || !done) return;
    const next = stepRotation(now, done.candidates, done.fit.shift, direction);
    if (next) patchZoneBlueprint(zoneId, next);
  };
  const here = assignments.find((a) => a.room.room_id === tool.roomId);
  const assign = async () => {
    const now = currentTool(zoneId);
    if (!now || !project) return;
    setSaving(true);
    const refused = await assignTool(project, now);
    setSaving(false);
    setNotice(refused ?? `Assigned ${blueprints?.find((b) => b.id === now.blueprintId)?.name ?? "the blueprint"} to ${room?.name ?? "the room"}.`);
    if (!refused) listAssignments(project).then(setAssignments).catch(() => undefined);
  };
  const canRotate = !!done && done.candidates.length > 1;

  return (
    <div className="routeBar blueprintBar">
      <strong>Blueprint</strong>
      <select
        className="picker"
        value={tool.blueprintId ?? ""}
        onChange={(e) => patchZoneBlueprint(zoneId, chooseBlueprint(tool, e.target.value || null))}
        aria-label="Blueprint to lay into a room"
      >
        <option value="">{blueprints === null ? "Reading…" : blueprints.length === 0 ? "No blueprints" : "Choose a blueprint…"}</option>
        {blueprints?.map((b) => (
          <option key={b.id} value={b.id}>
            {b.name} (r{b.current_revision}, {b.items} items)
          </option>
        ))}
      </select>
      <span className="routeResult">{describeTool(tool, room?.name ?? null)}</span>
      <button className="chip" disabled={!canRotate} title="The previous rotation that fits this room" onClick={() => rotate(-1)}>
        ‹ Rotate
      </button>
      <button className="chip" disabled={!canRotate} title="The next rotation that fits this room, best by doors first" onClick={() => rotate(1)}>
        Rotate ›
      </button>
      <button className="chip on" disabled={!done || saving || done.placements.length === 0} title="Keep this blueprint, at this rotation, as the room's assignment" onClick={() => void assign()}>
        {saving ? "Saving…" : here && here.blueprint_id === tool.blueprintId ? "Re-assign" : "Assign to this room"}
      </button>
      {here ? <span className="meta">now assigned: {here.blueprint_name ?? "a deleted blueprint"} r{here.revision}</span> : null}
      {notice ? <span className="meta">{notice}</span> : null}
      <a className="chip" href={`/blueprints/?project=${encodeURIComponent(project ?? "")}`} title="Edit blueprints, map edges by hand, or change an item in this room">
        Blueprints page
      </a>
      <button className="chip" title="Leave the blueprint tool (Esc)" onClick={() => setZoneBlueprintMode(zoneId, false)}>
        Close
      </button>
    </div>
  );
}
