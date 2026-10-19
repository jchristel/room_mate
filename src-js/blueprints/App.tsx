// The blueprints page, served at /blueprints/.
//
// Left: the project's blueprints, one selected at a time, with New, Import and Export. Centre: the
// room's outline, its FF&E and its edge numbers, with a toggle for item tags. Top: the revision
// timeline. Right: the selected item and its dimension constraints.
//
// **A blueprint is viewed one revision at a time and edited as a DRAFT.** Revisions are
// append-only (plan decision 9), so editing never changes one: the items of the revision being
// viewed are copied into a draft, changed freely, and Save sends them as the NEXT revision. The
// draft is the only mutable thing on the page, and the page says when it has unsaved changes.
//
// **A pending revision is shown read-only.** One that has arrived (an import) is not part of the
// history and nothing pins it; you look at it and accept or reject it on the timeline.
//
// Placing a blueprint in ANOTHER room is the server's resolver and belongs to the assign view in
// the main viewer; this page edits a blueprint in its own room.

import { useCallback, useEffect, useMemo, useState } from "react";

import * as api from "./api.js";
import { BlueprintList } from "./BlueprintList.js";
import { Canvas } from "./Canvas.js";
import { persistSelection, seedProjectId } from "./common.js";
import { sameItems } from "./draft.js";
import { fileNameFor, toFile, type Imported } from "./exchange.js";
import { pt } from "./geometry.js";
import { ImportBlueprint } from "./ImportBlueprint.js";
import { Inspector } from "./Inspector.js";
import { NewBlueprint } from "./NewBlueprint.js";
import { Timeline } from "./Timeline.js";
import type { Blueprint, BlueprintItem, BlueprintSummary, PendingRevision, ProjectRow } from "./types.js";

type Panel = "none" | "new" | "import";

const message = (e: unknown): string => String((e as Error).message ?? e);

export function App() {
  const [projects, setProjects] = useState<ProjectRow[]>([]);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [rows, setRows] = useState<BlueprintSummary[] | null>(null);
  const [blueprint, setBlueprint] = useState<Blueprint | null>(null);
  const [viewing, setViewing] = useState(1);
  const [pendingView, setPendingView] = useState<PendingRevision | null>(null);
  const [draft, setDraft] = useState<BlueprintItem[]>([]);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [showTags, setShowTags] = useState(false);
  const [showExcluded, setShowExcluded] = useState(true);
  const [panel, setPanel] = useState<Panel>("none");
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const revision = blueprint?.revisions.find((r) => r.number === viewing) ?? null;
  const dirty = !!revision && !pendingView && !sameItems(draft, revision.items);
  const content = pendingView ?? revision;
  const items = pendingView ? pendingView.items : draft;
  const ring = useMemo(() => (content ? content.outline.coarse.map(pt) : []), [content]);

  // Projects, once.
  useEffect(() => {
    api
      .listProjects()
      .then((rows) => {
        setProjects(rows);
        const first = rows[0];
        if (!first) return;
        const seed = seedProjectId();
        const id = seed && rows.some((p) => p.id === seed) ? seed : first.id;
        setProjectId(id);
        persistSelection(id);
      })
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const refreshRows = useCallback(
    (project: string) => api.listBlueprints(project).then(setRows).catch((e: unknown) => setError(message(e))),
    [],
  );

  const show = useCallback((b: Blueprint, number?: number) => {
    const n = number ?? b.current_revision;
    setBlueprint(b);
    setViewing(n);
    setPendingView(null);
    setDraft(b.revisions.find((r) => r.number === n)?.items ?? []);
    setSelected(new Set());
  }, []);

  useEffect(() => {
    setBlueprint(null);
    setRows(null);
    setPanel("none");
    if (projectId) void refreshRows(projectId);
  }, [projectId, refreshRows]);

  /** Ask before throwing an unsaved draft away. */
  const discardOk = (): boolean => !dirty || window.confirm("Discard the unsaved changes to this revision?");

  const open = async (id: string) => {
    if (!projectId || !discardOk()) return;
    setError(null);
    setNotice(null);
    setPanel("none");
    try {
      show(await api.getBlueprint(projectId, id));
    } catch (e) {
      setError(message(e));
    }
  };

  /** Apply a write's answer: show the blueprint it returns, or say why it was refused. */
  const settle = async (r: api.Sent<Blueprint>, done: string, number?: number): Promise<boolean> => {
    setBusy(false);
    if (!r.ok) {
      setError(r.error);
      return false;
    }
    setError(null);
    setNotice(done);
    show(r.value, number);
    if (projectId) await refreshRows(projectId);
    return true;
  };

  const save = async () => {
    if (!projectId || !blueprint || !revision || !dirty) return;
    const note = window.prompt("A note for the new revision (optional):", "");
    if (note === null) return;
    setBusy(true);
    const r = await api.addRevision(projectId, blueprint.id, blueprint.taken_at, { outline: revision.outline, items: draft }, note.trim() || null);
    const number = r.ok ? Math.max(...r.value.revisions.map((x) => x.number)) : undefined;
    await settle(r, "Saved as a new revision. The earlier one is untouched.", number);
  };

  const setCurrent = async (n: number) => {
    if (!projectId || !blueprint) return;
    setBusy(true);
    await settle(await api.updateBlueprint(projectId, blueprint.id, blueprint.taken_at, { current_revision: n }), `r${n} is now the current revision.`, n);
  };

  const decide = async (pendingId: number, accept: boolean) => {
    if (!projectId || !blueprint) return;
    setBusy(true);
    const r = await api.decidePending(projectId, blueprint.id, pendingId, blueprint.taken_at, accept);
    await settle(r, accept ? "Accepted: it is the new current revision." : "Rejected. It is kept, marked rejected.");
  };

  const rename = async () => {
    if (!projectId || !blueprint) return;
    const name = window.prompt("Rename the blueprint:", blueprint.name);
    if (!name?.trim() || name.trim() === blueprint.name) return;
    setBusy(true);
    await settle(await api.updateBlueprint(projectId, blueprint.id, blueprint.taken_at, { name: name.trim() }), "Renamed.", viewing);
  };

  const remove = async () => {
    if (!projectId || !blueprint) return;
    if (!window.confirm(`Delete "${blueprint.name}" and all its revisions? A copy is kept in the server's backups.`)) return;
    setBusy(true);
    const r = await api.deleteBlueprint(projectId, blueprint.id, blueprint.taken_at);
    setBusy(false);
    if (!r.ok) return setError(r.error);
    setBlueprint(null);
    setNotice("Deleted.");
    await refreshRows(projectId);
  };

  const exportFile = () => {
    if (!projectId || !blueprint) return;
    const file = toFile(blueprint, viewing, projectId);
    if (!file) return;
    const url = URL.createObjectURL(new Blob([JSON.stringify(file, null, 2)], { type: "application/json" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = fileNameFor(blueprint.name, viewing);
    a.click();
    URL.revokeObjectURL(url);
    setNotice(`Exported r${viewing}.`);
  };

  const imported = async (r: Imported) => {
    setPanel("none");
    if (!projectId) return;
    await refreshRows(projectId);
    show(r.blueprint);
    setNotice(
      r.kind === "created"
        ? `Imported as a new blueprint, "${r.blueprint.name}".`
        : `"${r.into}" already exists, so the import is waiting on its timeline as a pending revision.`,
    );
  };

  const select = (id: string | null, additive: boolean) => {
    if (id === null) return setSelected(new Set());
    setSelected((cur) => {
      const next = new Set(additive ? cur : []);
      if (additive && cur.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <>
      <header>
        <h1>Blueprints</h1>
        <select
          aria-label="Project"
          value={projectId ?? ""}
          onChange={(e) => {
            if (!discardOk()) return;
            setProjectId(e.target.value);
            persistSelection(e.target.value);
          }}
        >
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <div className="links">
          <a href="/">viewer</a>
          <a href="/reports/">reports</a>
          <a href="/settings/">settings</a>
        </div>
      </header>

      <div className="page">
        <div className="left">
          <BlueprintList
            rows={rows}
            selectedId={blueprint?.id ?? null}
            busy={busy}
            onSelect={(id) => void open(id)}
            onNew={() => setPanel("new")}
            onImport={() => setPanel("import")}
            onExport={exportFile}
            onRename={() => void rename()}
            onDelete={() => void remove()}
          />
          {projectId && panel === "new" ? (
            <NewBlueprint
              project={projectId}
              onCancel={() => setPanel("none")}
              onCreated={(b) => {
                setPanel("none");
                show(b);
                setNotice(`Created "${b.name}".`);
                void refreshRows(projectId);
              }}
            />
          ) : null}
          {projectId && panel === "import" ? (
            <ImportBlueprint project={projectId} projects={projects} existing={rows ?? []} onImported={(r) => void imported(r)} onCancel={() => setPanel("none")} />
          ) : null}
        </div>

        <main>
          {error ? <p className="banner error">{error}</p> : null}
          {notice && !error ? <p className="banner">{notice}</p> : null}
          {blueprint && content ? (
            <>
              <Timeline
                blueprint={blueprint}
                viewing={viewing}
                viewingPending={pendingView?.pending_id ?? null}
                busy={busy}
                onView={(n) => {
                  if (!discardOk()) return;
                  show(blueprint, n);
                }}
                onViewPending={(p) => {
                  if (!discardOk()) return;
                  setPendingView(p);
                  setSelected(new Set());
                }}
                onSetCurrent={(n) => void setCurrent(n)}
                onDecide={(id, accept) => void decide(id, accept)}
              />
              <div className="toolbar">
                <strong>{blueprint.name}</strong>
                <span className="sub">
                  {pendingView ? "pending revision (read-only)" : `r${viewing}${viewing === blueprint.current_revision ? " · current" : ""}`} ·{" "}
                  {items.filter((i) => !i.excluded).length} items
                  {items.some((i) => i.excluded) ? `, ${items.filter((i) => i.excluded).length} removed` : ""}
                </span>
                <label>
                  <input type="checkbox" checked={showTags} onChange={(e) => setShowTags(e.target.checked)} /> FF&amp;E tags
                </label>
                <label>
                  <input type="checkbox" checked={showExcluded} onChange={(e) => setShowExcluded(e.target.checked)} /> removed items
                </label>
                {dirty ? (
                  <span className="dirty">
                    Unsaved changes
                    <button type="button" disabled={busy} onClick={() => void save()}>
                      Save as new revision
                    </button>
                    <button type="button" disabled={busy} onClick={() => setDraft(revision?.items ?? [])}>
                      Discard
                    </button>
                  </span>
                ) : null}
              </div>
              <Canvas outline={content.outline} items={items} selected={selected} showTags={showTags} showExcluded={showExcluded} onSelect={select} />
            </>
          ) : (
            <p className="hint empty">{rows?.length ? "Choose a blueprint on the left." : "Make a blueprint from a room, or import one."}</p>
          )}
        </main>

        {blueprint && content ? (
          <Inspector
            items={items}
            ring={ring}
            selected={selected}
            revisions={blueprint.revisions}
            viewing={viewing}
            readOnly={!!pendingView}
            onItems={(next, msg) => {
              setDraft(next);
              if (msg) setNotice(msg);
            }}
          />
        ) : (
          <aside className="inspector" />
        )}
      </div>
    </>
  );
}
