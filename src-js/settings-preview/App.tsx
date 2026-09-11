// The settings page, in React — served at /settings-preview/, beside
// static/settings.html, which stays the real page until this one replaces it.
//
// It is the half of the 2026-09-11 framework comparison that won (the result and
// its measurements are in STRATEGY-BROWSER.md, "UI growth"), now grown from one
// slice to the whole page.
//
// **Two things it does that the JavaScript page does not.**
//
// 1. **It edits every setting, including the seven that never had a UI**: the
//    coordinate anchor, the area policy, the door/window/FF&E/space policies,
//    and the hierarchy exclusions. A setting only reachable by hand-editing TOML
//    is one nobody knows exists.
// 2. **It cannot silently drop what it does not show.** The whole settings
//    object is held as it was read and sent back (see `saveBody`), where the old
//    page rebuilt the JSON field by field — which is why saving from it emptied
//    every milestone's door, window, FF&E, space and ceiling pins.
//
// The types come from `./types.js`, which re-exports the TypeScript GENERATED
// from the Rust structs; a field added in Rust shows up here as a type error
// rather than as a silently missing control.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { apiGet, apiSend, persistSelection, seedProjectId } from "./common.js";
import { saveBody } from "./saveBody.js";
import { AreasSection } from "./sections/areas.js";
import { ColourPlansSection } from "./sections/colourPlans.js";
import { HierarchySection } from "./sections/hierarchy.js";
import { IdentitySection } from "./sections/identity.js";
import { MilestonesSection } from "./sections/milestones.js";
import { PoliciesSection } from "./sections/policies.js";
import { ReferenceSourcesSection } from "./sections/referenceSources.js";
import type { SourceStatus } from "./sections/referenceSources.js";
import { RoomLabelSection } from "./sections/roomLabel.js";
import type {
  ModelSnapshots,
  ProjectFileSummary,
  ProjectSettingsResponse,
  ProjectSnapshotsResponse,
  ReferenceSnapshotInfo,
  ReferenceSnapshotList,
  ReferenceUploadResult,
  SaveResponse,
  Settings,
} from "./types.js";

const PROPERTY_LIST_ID = "roomPropertyOptions";
const DATE_FORMAT_LIST_ID = "dateFormatOptions";

interface Editing {
  file: string | null;
  settings: Settings;
  isNew: boolean;
}

type Message = { ok: boolean; text: string } | null;

const errorText = (err: unknown): string => (err instanceof Error ? err.message : String(err));

/** A project that does not exist yet: the Rust defaults, spelled once. */
const blankSettings = (): Settings => ({
  project_id: "",
  is_default: false,
  comparison_properties: [],
  sources: {},
  hierarchy: [],
  builtin_properties: [],
  room_label: ["$name", "$id"],
  milestones: [],
  colour_plans: [],
});

export function App() {
  const [projects, setProjects] = useState<ProjectFileSummary[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [message, setMessage] = useState<Message>(null);
  const [saving, setSaving] = useState(false);

  // What the STORE holds, which is what a milestone can pin. `null` = loading.
  const [models, setModels] = useState<ModelSnapshots[] | null>(null);
  const [snapshotsBySource, setSnapshotsBySource] = useState<Record<string, string[] | null>>({});
  const [labelsBySource, setLabelsBySource] = useState<Record<string, string[]>>({});
  const [statusBySource, setStatusBySource] = useState<Record<string, SourceStatus>>({});
  const [roomPropertyKeys, setRoomPropertyKeys] = useState<string[]>([]);

  const loadProjects = useCallback(async () => {
    try {
      setProjects(await apiGet<ProjectFileSummary[]>("/api/settings/projects"));
      setListError(null);
    } catch (err) {
      setListError(errorText(err));
    }
  }, []);

  /** One source's stored uploads, and the column vocabulary of the latest one.
   *  A 404 on `/latest` means "no upload yet", which is a neutral state. */
  const loadSource = useCallback(async (projectId: string, source: string) => {
    setSnapshotsBySource((current) => ({ ...current, [source]: null }));
    try {
      const list = await apiGet<ReferenceSnapshotList>(
        `/projects/${encodeURIComponent(projectId)}/reference/${encodeURIComponent(source)}/snapshots`,
      );
      setSnapshotsBySource((current) => ({ ...current, [source]: list.snapshots }));
    } catch {
      setSnapshotsBySource((current) => ({ ...current, [source]: [] }));
    }
    try {
      const latest = await apiGet<ReferenceSnapshotInfo>(
        `/projects/${encodeURIComponent(projectId)}/reference/${encodeURIComponent(source)}/latest`,
      );
      setLabelsBySource((current) => ({ ...current, [source]: latest.labels }));
      setStatusBySource((current) => ({
        ...current,
        [source]: {
          kind: "ok",
          text: `✓ latest upload ${latest.taken_at} · ${latest.record_count} records · link property "${latest.link_property}"`,
        },
      }));
    } catch {
      setLabelsBySource((current) => ({ ...current, [source]: [] }));
      setStatusBySource((current) => ({
        ...current,
        [source]: { kind: "", text: "no upload yet — QA labels load from the first uploaded CSV" },
      }));
    }
  }, []);

  const loadProjectData = useCallback(
    async (projectId: string, settings: Settings) => {
      setModels(null);
      try {
        const data = await apiGet<ProjectSnapshotsResponse>(`/projects/${encodeURIComponent(projectId)}/snapshots`);
        setModels(data.models);
      } catch {
        setModels([]);
      }
      // The union of property names actually present on this project's rooms —
      // half of the datalist every property input draws from. An unreachable or
      // empty result just means an empty list; the inputs stay free text.
      try {
        const rooms = await apiGet<{ rooms?: { properties?: Record<string, unknown> }[] }>(
          `/rooms?project=${encodeURIComponent(projectId)}`,
        );
        const keys = new Set<string>();
        for (const room of rooms.rooms ?? []) for (const key of Object.keys(room.properties ?? {})) keys.add(key);
        setRoomPropertyKeys([...keys].sort());
      } catch {
        setRoomPropertyKeys([]);
      }
      for (const source of Object.keys(settings.sources.reference ?? {})) void loadSource(projectId, source);
    },
    [loadSource],
  );

  const selectProject = useCallback(
    async (projectId: string) => {
      try {
        const data = await apiGet<ProjectSettingsResponse>(`/api/settings/projects/${encodeURIComponent(projectId)}`);
        persistSelection(projectId);
        setActiveId(projectId);
        setEditing({ file: data.file, settings: data.settings, isNew: false });
        setMessage(null);
        setLabelsBySource({});
        setStatusBySource({});
        setSnapshotsBySource({});
        void loadProjectData(projectId, data.settings);
      } catch (err) {
        setMessage({ ok: false, text: errorText(err) });
      }
    },
    [loadProjectData],
  );

  useEffect(() => {
    void loadProjects();
  }, [loadProjects]);

  // Open the seeded project once, after the list that must contain it arrives —
  // the JS page's `didRestore` one-shot, for the same reason: the refetch after a
  // save must not re-trigger a restore and fight whatever was clicked since.
  const restored = useRef(false);
  useEffect(() => {
    if (!projects || restored.current) return;
    restored.current = true;
    const seed = seedProjectId();
    if (seed !== null && projects.some((p) => p.project_id === seed)) void selectProject(seed);
  }, [projects, selectProject]);

  const edit = useCallback(
    (change: (s: Settings) => Settings) =>
      setEditing((current) => (current ? { ...current, settings: change(current.settings) } : current)),
    [],
  );

  const newProject = () => {
    persistSelection(null);
    setActiveId(null);
    setEditing({ file: null, settings: blankSettings(), isNew: true });
    setMessage(null);
    // A project that does not exist yet has nothing stored against it.
    setModels([]);
    setSnapshotsBySource({});
    setLabelsBySource({});
    setStatusBySource({});
    setRoomPropertyKeys([]);
  };

  const save = async () => {
    if (!editing) return;
    setSaving(true);
    try {
      const body = saveBody(editing.settings);
      const result = editing.isNew
        ? await apiSend("POST", "/api/settings/projects", body)
        : await apiSend("PUT", `/api/settings/projects/${encodeURIComponent(activeId ?? "")}`, body);
      if (result.ok) {
        const saved = (JSON.parse(result.text) as SaveResponse).settings;
        const projectId = saved.project_id;
        persistSelection(projectId);
        setActiveId(projectId);
        setEditing({ file: `${projectId}.toml`, settings: saved, isNew: false });
        setMessage({ ok: true, text: "✓ saved & applied live" });
        void loadProjects();
        void loadProjectData(projectId, saved);
      } else {
        // The server's 422 is the real validator — shown verbatim.
        setMessage({ ok: false, text: `⚠ ${result.text}` });
      }
    } catch (err) {
      setMessage({ ok: false, text: `⚠ ${errorText(err)}` });
    } finally {
      setSaving(false);
    }
  };

  // Property names the inputs offer: the rooms' own, then each source's columns
  // as `<source>.<label>` — the namespaced vocabulary a /rooms filter and a
  // comparison key already speak.
  const propertyOptions = useMemo(() => {
    const options = [...roomPropertyKeys];
    for (const [source, labels] of Object.entries(labelsBySource).sort()) {
      for (const label of labels) options.push(`${source}.${label}`);
    }
    return options;
  }, [roomPropertyKeys, labelsBySource]);

  // Date formats the date-range plans offer: the strftime patterns this
  // project's date-typed reference fields already declare.
  const dateFormats = useMemo(() => {
    const formats = new Set<string>();
    for (const source of Object.values(editing?.settings.sources.reference ?? {})) {
      for (const field of source.fields) {
        if (field.type === "date" && field.format !== null && field.format !== "") formats.add(field.format);
      }
    }
    return [...formats];
  }, [editing]);

  return (
    <>
      <header>
        <h1>Room Plan · Settings</h1>
        <span className="preview">React preview</span>
        <a href="/settings.html">current settings page</a>
        <a href="/">← viewer</a>
      </header>
      <main>
        <nav>
          <h2>Projects</h2>
          <ul>
            {listError !== null && (
              <li>
                <span className="file-error">{listError}</span>
              </li>
            )}
            {projects === null && listError === null && <li className="placeholder">loading…</li>}
            {projects?.map((item) => (
              <ProjectRow key={item.file} item={item} active={item.project_id === activeId} onSelect={selectProject} />
            ))}
          </ul>
          <button type="button" className="action" onClick={newProject}>
            + new project
          </button>
        </nav>
        <div>
          {editing === null ? (
            <div className="placeholder">Select a project on the left, or create a new one.</div>
          ) : (
            <form
              className="editor"
              autoComplete="off"
              onSubmit={(ev) => {
                ev.preventDefault();
                void save();
              }}
            >
              <datalist id={PROPERTY_LIST_ID}>
                {propertyOptions.map((option) => (
                  <option key={option} value={option} />
                ))}
              </datalist>
              <datalist id={DATE_FORMAT_LIST_ID}>
                {dateFormats.map((format) => (
                  <option key={format} value={format} />
                ))}
              </datalist>

              {/* Keyed by project, so a section's local text state (a
                  half-typed number, an unadded chip) cannot survive a switch
                  to a different project. */}
              <Sections
                key={activeId ?? "new"}
                editing={editing}
                edit={edit}
                activeId={activeId}
                models={models}
                snapshotsBySource={snapshotsBySource}
                labelsBySource={labelsBySource}
                statusBySource={statusBySource}
                onSourceStatus={(source, status) => setStatusBySource((current) => ({ ...current, [source]: status }))}
                onUploaded={(source, result) => {
                  setLabelsBySource((current) => ({ ...current, [source]: result.labels }));
                  if (activeId !== null) void loadSource(activeId, source);
                }}
              />

              <div className="save-bar">
                <button type="submit" className="action primary" disabled={saving}>
                  Save
                </button>
                <span className={`save-message${message ? (message.ok ? " ok" : " bad") : ""}`}>{message?.text}</span>
              </div>
            </form>
          )}
        </div>
      </main>
    </>
  );
}

function Sections({
  editing,
  edit,
  activeId,
  models,
  snapshotsBySource,
  labelsBySource,
  statusBySource,
  onSourceStatus,
  onUploaded,
}: {
  editing: Editing;
  edit: (change: (s: Settings) => Settings) => void;
  activeId: string | null;
  models: ModelSnapshots[] | null;
  snapshotsBySource: Readonly<Record<string, string[] | null>>;
  labelsBySource: Readonly<Record<string, string[]>>;
  statusBySource: Readonly<Record<string, SourceStatus>>;
  onSourceStatus: (source: string, status: SourceStatus) => void;
  onUploaded: (source: string, result: ReferenceUploadResult) => void;
}) {
  const { settings, file, isNew } = editing;
  return (
    <>
      <IdentitySection
        settings={settings}
        edit={edit}
        file={file}
        isNew={isNew}
        modelIds={(models ?? []).map((m) => m.id)}
        propertyListId={PROPERTY_LIST_ID}
      />
      <AreasSection settings={settings} edit={edit} />
      <PoliciesSection settings={settings} edit={edit} propertyListId={PROPERTY_LIST_ID} />
      <ReferenceSourcesSection
        settings={settings}
        edit={edit}
        projectId={activeId}
        isNew={isNew}
        snapshotsBySource={snapshotsBySource}
        labelsBySource={labelsBySource}
        statusBySource={statusBySource}
        onStatus={onSourceStatus}
        onUploaded={onUploaded}
      />
      <HierarchySection settings={settings} edit={edit} propertyListId={PROPERTY_LIST_ID} />
      <RoomLabelSection settings={settings} edit={edit} propertyListId={PROPERTY_LIST_ID} />
      <MilestonesSection settings={settings} edit={edit} models={models} snapshotsBySource={snapshotsBySource} />
      <ColourPlansSection
        settings={settings}
        edit={edit}
        propertyListId={PROPERTY_LIST_ID}
        dateFormatListId={DATE_FORMAT_LIST_ID}
      />
    </>
  );
}

function ProjectRow({
  item,
  active,
  onSelect,
}: {
  item: ProjectFileSummary;
  active: boolean;
  onSelect: (projectId: string) => void;
}) {
  if (item.error !== undefined && item.error !== null) {
    return (
      <li>
        <span className="file-error">{`${item.file}: ${item.error}`}</span>
      </li>
    );
  }
  const projectId = item.project_id;
  if (projectId === undefined || projectId === null) {
    return (
      <li>
        <span className="file-error">{`${item.file}: no project_id`}</span>
      </li>
    );
  }
  return (
    <li>
      <button type="button" className={active ? "active" : ""} onClick={() => onSelect(projectId)}>
        {projectId}{" "}
        {item.is_default && (
          <span className="default-mark" title="default bundle">
            ●
          </span>
        )}
      </button>
    </li>
  );
}
