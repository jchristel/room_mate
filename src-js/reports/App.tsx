// The reports page, served at /reports/.
//
// **What it replaces, and what it does not yet do.** It took over milestone
// comparison from `static/comparison.html` (deleted with this landing) and the
// QA CSV export from the viewer's QA band (whose Download button went at the
// same time, for the reason STRATEGY-REPORTS.md gives: the same findings
// rendered by two pieces of code drift the first time either is fixed).
//
// Schedules and the by-room reports read `POST /projects/{id}/reports`, which
// projects to the columns asked for: `/ffe` is 133 MB for one RHH storey and a
// report of it is a few, so this page never fetches an entity payload to
// project it locally. The filter builder is the next slice; its design, and the
// mockup it was reviewed against, are in docs/STRATEGY-REPORTS.md. A report
// type not built yet sits in the dropdown disabled and says so, rather than
// being absent — an empty category is a question a reader cannot answer.
//
// The report *type* decides which form appears below it. That is the structure
// the design turns on: adding a type should cost a registry row and, at most,
// one new section, never a screen of its own.

import { useCallback, useEffect, useState } from "react";

import { CheckReport } from "./checks.js";
import { apiGet, persistSelection, seedProjectId } from "./common.js";
import { CHECKS } from "./checkRows.js";
import { ENTITIES } from "./reportTypes.js";
import { TableReport } from "./tableReport.js";
import { ComparisonReport } from "./comparison.js";
import type { ProjectRow } from "./types.js";

interface ReportType {
  id: string;
  group: string;
  label: string;
  desc: string;
  /** A type whose form is not built yet. Listed, disabled, and honest about it. */
  later?: boolean;
}

const TYPES: ReportType[] = [
  ...CHECKS.map((c) => ({ id: `check.${c.id}`, group: "Checks", label: c.label, desc: c.desc })),
  {
    id: "check.ceilings",
    group: "Checks",
    label: "Rooms without a ceiling",
    desc: "Needs the ceilings QA report, which is not built server-side yet.",
    later: true,
  },
  {
    id: "cmp.rooms",
    group: "Between milestones",
    label: "Room changes",
    desc: "Rooms added, removed or changed between a baseline milestone and the others.",
  },
  ...ENTITIES.map((e) => ({
    id: `sched.${e.id}`,
    group: "Schedules",
    label: `${e.label} schedule`,
    desc: `One row per ${e.one}, with the columns you pick. Only those columns are read.`,
  })),
  ...ENTITIES.map((e) => ({
    id: `by_room.${e.id}`,
    group: "By room",
    label: `${e.label} by room`,
    desc: e.byRoom
      ? `Each ${e.many.slice(0, -1) === e.one ? e.one : e.one} with the room it is attributed to — the attribution the ${e.label.toLowerCase()} read already made, never recomputed here.`
      : (e.byRoomNote as string),
    later: !e.byRoom,
  })),
];

const GROUPS = ["Checks", "Between milestones", "Schedules", "By room"];

export function App() {
  const [projects, setProjects] = useState<ProjectRow[]>([]);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [typeId, setTypeId] = useState("check.reference");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    apiGet<ProjectRow[]>("/projects")
      .then((rows) => {
        setProjects(rows);
        if (!rows.length) return;
        // A seed the server no longer lists is ignored, so a stale id never
        // drives the reads below.
        const seed = seedProjectId();
        const first = rows[0] as ProjectRow;
        const restored = seed && rows.some((p) => p.id === seed) ? seed : first.id;
        setProjectId(restored);
        persistSelection(restored);
      })
      .catch((e: unknown) => setError(String((e as Error).message ?? e)));
  }, []);

  const pick = useCallback((id: string) => {
    setProjectId(id);
    persistSelection(id);
  }, []);

  const type = TYPES.find((t) => t.id === typeId) as ReportType;

  return (
    <>
      <header>
        <h1>Reports</h1>
        <select aria-label="Project" value={projectId ?? ""} onChange={(e) => pick(e.target.value)}>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <div className="links">
          <a href="/">viewer</a>
          <a href="/settings/">settings</a>
        </div>
      </header>

      <main>
        <div className="content">
          <section className="group">
            <h2>
              <label htmlFor="type">Report type</label>
            </h2>
            <select
              className="kind"
              id="type"
              value={typeId}
              onChange={(e) => setTypeId(e.target.value)}
            >
              {GROUPS.map((g) => (
                <optgroup label={g} key={g}>
                  {TYPES.filter((t) => t.group === g).map((t) => (
                    <option key={t.id} value={t.id} disabled={t.later}>
                      {t.label}
                      {t.later ? "  (later)" : ""}
                    </option>
                  ))}
                </optgroup>
              ))}
            </select>
            {type.desc && <p className="kind-desc">{type.desc}</p>}
          </section>

          {error && <p className="msg err">{error}</p>}
          {!projectId && !error && <p className="foot">No project has any stored data yet.</p>}

          {projectId && typeId.startsWith("check.") && (
            <CheckReport projectId={projectId} checkId={typeId.slice("check.".length)} />
          )}
          {projectId && typeId === "cmp.rooms" && <ComparisonReport projectId={projectId} />}
          {projectId && (typeId.startsWith("sched.") || typeId.startsWith("by_room.")) && (
            <TableReport
              key={typeId}
              projectId={projectId}
              entityId={typeId.slice(typeId.indexOf(".") + 1)}
              byRoom={typeId.startsWith("by_room.")}
            />
          )}
        </div>
      </main>
    </>
  );
}
