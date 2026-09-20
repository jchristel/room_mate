// The header's scope pickers.
//
// The ids and classes are the old page's (`picker`, `#projectSelect` …) because
// `viewer.css` was moved whole — see `App.tsx`. What is NOT the old page's is
// the hiding rule: it kept `populate*Select` functions that diffed option lists
// against signature strings so the 2-second tick would not rebuild `<option>`
// nodes under the user's cursor. React rebuilds from the list and keeps the
// selection because `value` is controlled, so those guards went.

import { buildingLabel, type Scope } from "../scope.js";
import { setScope } from "./poll.js";
import { Search } from "./Search.js";
import { addZone, MAX_ZONES, removeZone, setLinkViews } from "./store.js";
import { useViewer } from "./useViewer.js";

export function Header() {
  const { scope, projects, buildings, milestones, zones, linkViews } = useViewer();

  const change = (patch: Partial<Scope>) => {
    const next: Scope = { ...scope, ...patch };
    // A project change drops the other two: a building key and a milestone name
    // are only meaningful inside the project that declared them, and carrying
    // one across silently scopes the new project's reads to something it has
    // never heard of.
    if (patch.projectId !== undefined && patch.projectId !== scope.projectId) {
      next.building = null;
      next.milestone = null;
    }
    void setScope(next);
  };

  return (
    <header>
      <h1>Room Plan</h1>
      <select
        className={`picker${projects.length ? "" : " hidden"}`}
        id="projectSelect"
        title="Project — the scope of everything on screen"
        value={scope.projectId ?? ""}
        onChange={(e) => change({ projectId: e.target.value || null })}
      >
        {projects.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
      </select>
      <select
        className="picker"
        id="milestoneSelect"
        title="Milestone — Latest or a pinned issue"
        value={scope.milestone ?? ""}
        onChange={(e) => change({ milestone: e.target.value || null })}
      >
        <option value="">Latest</option>
        {milestones.map((m) => (
          <option key={m.name} value={m.name}>
            {m.date ? `${m.name} (${m.date})` : m.name}
          </option>
        ))}
      </select>
      <select
        className={`picker${buildings.length ? "" : " hidden"}`}
        id="buildingSelect"
        title="Building"
        value={scope.building ?? ""}
        onChange={(e) => change({ building: e.target.value || null })}
      >
        {/* First, and not cosmetic: a homeless element — every window and
            external door in a facade package — matches no building, so without
            this there is no way to ask the unscoped question. */}
        <option value="">All buildings</option>
        {buildings.map((b) => (
          <option key={b.key} value={b.key}>
            {buildingLabel(b)}
          </option>
        ))}
      </select>
      <button className="ctl" title="Add a zone" disabled={zones.length >= MAX_ZONES} onClick={addZone}>
        + zone
      </button>
      <button className="ctl" title="Remove the last zone" disabled={zones.length <= 1} onClick={removeZone}>
        &minus; zone
      </button>
      <button
        className={`ctl${linkViews ? " on" : ""}`}
        title="Sync zoom/pan across all zones"
        onClick={() => setLinkViews(!linkViews)}
      >
        Link views: {linkViews ? "on" : "off"}
      </button>
      <Search />
      <div className="links">
        <a href="/reports/">reports</a>
        <a href="/settings/">settings</a>
      </div>
    </header>
  );
}
