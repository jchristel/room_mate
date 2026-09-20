// What the page says while there is no plan on it yet.
//
// It reports what the data layer actually has — the scope, the status, the room
// and level counts, and when the last changed payload landed — rather than
// showing an empty frame. During the port that is the only way to see that a
// slice works before the slice that draws arrives, and it names the live page
// for anyone who reaches `/viewer/` by guessing the URL.
//
// Deleted by the cutover slice (B10), with `static/index.html`.

import { useViewer } from "./useViewer.js";

export function PortNotice() {
  const { scope, payload, status, updatedAt } = useViewer();
  const rooms = payload?.rooms?.length ?? 0;
  const levels = payload?.levels?.length ?? 0;

  return (
    <div className="port-notice">
      <h2>The viewer is being ported</h2>
      <p>
        This page is the React rebuild of the room plan, built one slice at a
        time. Nothing is drawn here yet — the working viewer is at{" "}
        <a href="/">/</a>.
      </p>
      <dl className="port-status">
        <dt>Project</dt>
        <dd>{scope.projectId ?? "(none)"}</dd>
        <dt>Building</dt>
        <dd>{scope.building ?? "All buildings"}</dd>
        <dt>Milestone</dt>
        <dd>{scope.milestone ?? "Latest"}</dd>
        <dt>Rooms</dt>
        <dd>
          {rooms} in {levels} {levels === 1 ? "level" : "levels"}
        </dd>
        <dt>Status</dt>
        <dd>
          {status}
          {updatedAt ? ` · ${updatedAt.toLocaleTimeString()}` : ""}
        </dd>
      </dl>
    </div>
  );
}
