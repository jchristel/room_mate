// The settings page, in React — a PREVIEW of one slice, served at
// /settings-react/ beside static/settings.html, which stays the real page.
//
// It is the half of the 2026-09-11 framework comparison that won (the result and
// its measurements are in STRATEGY-BROWSER.md, "UI growth"), kept as the worked
// example of how a React page arrives here: its own Vite config
// (vite.settings-react.config.ts), built into static/settings-react/ and
// committed like the renderer bundle, under the same CI freshness gate.
//
// The slice: the project list, identity (id, display name, default flag), the
// room label, and save. Everything else the JS page edits is not ported, and is
// preserved — the whole settings object is held as it was read and sent back
// (see `saveBody`).

import { useCallback, useEffect, useRef, useState } from "react";
import type { FormEvent, KeyboardEvent } from "react";

import { apiGet, apiSend, persistSelection, seedProjectId } from "./common.js";
import { saveBody } from "./saveBody.js";
import type { ProjectFileSummary, ProjectSettingsResponse, SaveResponse, Settings } from "./types.js";

/** The project open in the editor, exactly as the server returned it. */
interface Editing {
  file: string;
  settings: Settings;
}

/** A save's outcome as shown beside the button. */
type Message = { ok: boolean; text: string } | null;

type Edit = (change: (settings: Settings) => Settings) => void;

const errorText = (err: unknown): string => (err instanceof Error ? err.message : String(err));

export function App() {
  const [projects, setProjects] = useState<ProjectFileSummary[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [message, setMessage] = useState<Message>(null);
  const [saving, setSaving] = useState(false);

  const loadProjects = useCallback(async () => {
    try {
      setProjects(await apiGet<ProjectFileSummary[]>("/api/settings/projects"));
      setListError(null);
    } catch (err) {
      setListError(errorText(err));
    }
  }, []);

  const selectProject = useCallback(async (projectId: string) => {
    try {
      const data = await apiGet<ProjectSettingsResponse>(`/api/settings/projects/${encodeURIComponent(projectId)}`);
      persistSelection(projectId);
      setActiveId(projectId);
      setEditing({ file: data.file, settings: data.settings });
      setMessage(null);
    } catch (err) {
      setMessage({ ok: false, text: errorText(err) });
    }
  }, []);

  useEffect(() => {
    void loadProjects();
  }, [loadProjects]);

  // Open the seeded project once, after the list that must contain it has
  // arrived — the JS page's `didRestore` one-shot, for the same reason: the
  // refetch after a save must not re-trigger a restore and fight whatever the
  // user has clicked since.
  const restored = useRef(false);
  useEffect(() => {
    if (!projects || restored.current) return;
    restored.current = true;
    const seed = seedProjectId();
    if (seed && projects.some((p) => p.project_id === seed)) void selectProject(seed);
  }, [projects, selectProject]);

  const edit: Edit = useCallback(
    (change) => setEditing((current) => (current ? { ...current, settings: change(current.settings) } : current)),
    [],
  );

  const save = async () => {
    if (!editing || activeId === null) return;
    setSaving(true);
    try {
      const url = `/api/settings/projects/${encodeURIComponent(activeId)}`;
      const result = await apiSend("PUT", url, saveBody(editing.settings));
      if (result.ok) {
        const saved = JSON.parse(result.text) as SaveResponse;
        setEditing((current) => (current ? { ...current, settings: saved.settings } : current));
        setMessage({ ok: true, text: "✓ saved & applied live" });
        void loadProjects();
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
        </nav>
        <div>
          {editing ? (
            <Editor editing={editing} message={message} saving={saving} edit={edit} onSave={save} />
          ) : (
            <div className="placeholder">Select a project on the left.</div>
          )}
        </div>
      </main>
    </>
  );
}

interface ProjectRowProps {
  item: ProjectFileSummary;
  active: boolean;
  onSelect: (projectId: string) => void;
}

function ProjectRow({ item, active, onSelect }: ProjectRowProps) {
  if (item.error !== undefined) {
    return (
      <li>
        <span className="file-error">{`${item.file}: ${item.error}`}</span>
      </li>
    );
  }
  const projectId = item.project_id;
  if (projectId === undefined) {
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

interface EditorProps {
  editing: Editing;
  message: Message;
  saving: boolean;
  edit: Edit;
  onSave: () => void;
}

function Editor({ editing, message, saving, edit, onSave }: EditorProps) {
  const [newLabel, setNewLabel] = useState("");
  const { settings } = editing;

  const addLabel = () => {
    const value = newLabel.trim();
    if (!value) return;
    edit((s) => ({ ...s, room_label: [...s.room_label, value] }));
    setNewLabel("");
  };

  const moveLabel = (from: number, to: number) =>
    edit((s) => {
      if (to < 0 || to >= s.room_label.length) return s;
      const labels = [...s.room_label];
      const [moved] = labels.splice(from, 1);
      if (moved === undefined) return s;
      labels.splice(to, 0, moved);
      return { ...s, room_label: labels };
    });

  const removeLabel = (index: number) => edit((s) => ({ ...s, room_label: s.room_label.filter((_, i) => i !== index) }));

  const onSubmit = (ev: FormEvent) => {
    ev.preventDefault();
    onSave();
  };

  const onLabelKey = (ev: KeyboardEvent<HTMLInputElement>) => {
    if (ev.key === "Enter") {
      ev.preventDefault();
      addLabel();
    }
  };

  return (
    <form className="editor" autoComplete="off" onSubmit={onSubmit}>
      <p className="not-ported">
        This preview edits identity and the room label only. Every other section of the project is kept exactly as
        stored and saved back unchanged — edit it on the <a href="/settings.html">current settings page</a>.
      </p>

      <section className="block">
        <h2>Identity</h2>
        <p className="hint">
          Stored in {editing.file}. The id is the project&apos;s identity. The display name is a label only, and can be
          changed at any time.
        </p>
        <label className="field">
          project id
          <input type="text" size={28} readOnly value={settings.project_id} />
        </label>
        <label className="field">
          display name
          <input
            type="text"
            size={28}
            placeholder="(defaults to the project id)"
            value={settings.name ?? ""}
            // Stored raw, normalised on save: trimming per keystroke would fight
            // the cursor while someone types a space.
            onChange={(ev) => {
              const value = ev.target.value;
              edit((s) => ({ ...s, name: value }));
            }}
          />
        </label>
        <label className="field">
          <input
            type="checkbox"
            checked={settings.is_default}
            onChange={(ev) => {
              const checked = ev.target.checked;
              edit((s) => ({ ...s, is_default: checked }));
            }}
          />
          serve as default for unregistered projects
        </label>
      </section>

      <section className="block">
        <h2>Room label</h2>
        <p className="hint">
          Ordered fields shown on each room in the viewer. $name and $id are the room&apos;s own name/id;{" "}
          <code>source.Column</code> reads a joined reference source; anything else is a property name.
        </p>
        <div className="chips">
          {settings.room_label.map((name, index) => (
            <span className="chip" key={`${index}:${name}`}>
              <span>{name}</span>
              <button type="button" title="move left" onClick={() => moveLabel(index, index - 1)}>
                ‹
              </button>
              <button type="button" title="move right" onClick={() => moveLabel(index, index + 1)}>
                ›
              </button>
              <button type="button" title="remove" onClick={() => removeLabel(index)}>
                ✕
              </button>
            </span>
          ))}
        </div>
        <label className="field">
          <input
            type="text"
            size={16}
            placeholder="Area"
            value={newLabel}
            onChange={(ev) => setNewLabel(ev.target.value)}
            onKeyDown={onLabelKey}
          />
        </label>
        <button type="button" className="action" onClick={addLabel}>
          + field
        </button>
      </section>

      <div className="save-bar">
        <button type="submit" className="action primary" disabled={saving}>
          Save
        </button>
        <span className={`save-message${message ? (message.ok ? " ok" : " bad") : ""}`}>{message?.text}</span>
      </div>
    </form>
  );
}
