// The RLS settings of one Revit model, edited in a browser tab.
//
// Opened by the Revit plug-in with ?project=&model=; the plug-in has already posted the model's
// current settings and the choices only Revit knows (title blocks, view types ...) as a draft
// (src/model_settings.rs). Save writes the edited settings back to that draft and the plug-in
// reads it and stores it in the model. The page keeps nothing of its own: close it unsaved and
// nothing has happened.

import { useEffect, useMemo, useState } from "react";

import { getDraft, saveDraft } from "./api.js";
import { missing, normalise, SIZES, unknownTokens, withToken, type Draft, type RlsSettings } from "./model.js";

const params = new URLSearchParams(window.location.search);
const project = params.get("project") ?? "";
const model = params.get("model") ?? "";

export function App() {
  const [draft, setDraft] = useState<Draft | null>(null);
  const [settings, setSettings] = useState<RlsSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!project || !model) {
      setError("This page is opened from Revit: the address needs ?project= and ?model=.");
      return;
    }
    getDraft(project, model)
      .then((d) => {
        setDraft(d);
        setSettings(normalise(d.settings));
        setSaved(d.saved);
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const options = draft?.options ?? {};
  const todo = useMemo(() => (settings ? missing(settings) : []), [settings]);

  if (error) return <Shell><p className="error">{error}</p></Shell>;
  if (!settings || !draft) return <Shell><p className="hint">Loading ...</p></Shell>;

  const change = (patch: Partial<RlsSettings>) => {
    setSettings({ ...settings, ...patch });
    setSaved(false);
  };
  const save = async () => {
    setBusy(true);
    const r = await saveDraft(project, model, settings);
    setBusy(false);
    if (r.ok) setSaved(true);
    else setError(r.error);
  };

  return (
    <Shell>
      <p className="sub">Model: <b>{draft.model_id}</b></p>

      <section>
        <h2>Title blocks by sheet size</h2>
        {SIZES.map((size) => (
          <Pick key={size} label={size.toUpperCase()} value={settings.sheet_sizes[size]} choices={options.title_blocks}
            onChange={(v) => change({ sheet_sizes: { ...settings.sheet_sizes, [size]: v } })} />
        ))}
      </section>

      <section>
        <h2>Views</h2>
        <Pick label="Plan view type" value={settings.plan_view_type} choices={options.plan_view_types} onChange={(v) => change({ plan_view_type: v })} />
        <Pick label="Elevation view type" value={settings.elevation_view_type} choices={options.elevation_view_types} onChange={(v) => change({ elevation_view_type: v })} />
        <Pick label="Schedule base" value={settings.schedule_base} choices={options.schedules} onChange={(v) => change({ schedule_base: v })}
          hint="Its first filter must be the room number." />
      </section>

      <section>
        <h2>Tags</h2>
        <Pick label="Room tag" value={settings.room_tag} choices={options.room_tags} onChange={(v) => change({ room_tag: v })}
          hint="Empty: continue with a warning." />
        <Pick label="Independent tag" value={settings.independent_tag} choices={options.independent_tags} onChange={(v) => change({ independent_tag: v })}
          hint="A multi-category tag. Empty: continue with a warning." />
      </section>

      <section>
        <h2>Naming</h2>
        <Template label="Sheet number" value={settings.sheet_number} known={options.room_parameters} onChange={(v) => change({ sheet_number: v })} />
        <Template label="Sheet name" value={settings.sheet_name} known={options.room_parameters} onChange={(v) => change({ sheet_name: v })} />
        <Template label="View name" value={settings.view_name} known={options.room_parameters} onChange={(v) => change({ view_name: v })} />
      </section>

      <div className="bar">
        <button onClick={save} disabled={busy}>Save</button>
        {saved ? <span className="ok">Saved. Go back to Revit and press OK.</span> : <span className="hint">Not saved yet.</span>}
      </div>
      {todo.length > 0 && <p className="hint">Still to choose before an RLS can be made: {todo.join(", ")}.</p>}
    </Shell>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="page">
      <header><h1>RLS settings</h1></header>
      <main>{children}</main>
    </div>
  );
}

/** A choice from what Revit offered. A value the model does not have any more stays visible, marked, never silently dropped. */
function Pick(props: { label: string; value: string; choices: string[] | undefined; onChange: (v: string) => void; hint?: string }) {
  const { label, value, choices, onChange, hint } = props;
  const list = choices ?? [];
  const absent = value !== "" && !list.includes(value);
  return (
    <label className="row">
      <span>{label}</span>
      <select value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">(not set)</option>
        {absent && <option value={value}>{value} (not in this model)</option>}
        {list.map((c) => <option key={c} value={c}>{c}</option>)}
      </select>
      {choices === undefined && <em className="hint">Revit did not offer a list; nothing to choose from.</em>}
      {hint && <em className="hint">{hint}</em>}
    </label>
  );
}

/** A name template: fixed text and `{Room Parameter}` tokens. */
function Template(props: { label: string; value: string; known: string[] | undefined; onChange: (v: string) => void }) {
  const { label, value, known, onChange } = props;
  const bad = unknownTokens(value, known);
  return (
    <div className="row">
      <label>
        <span>{label}</span>
        <input value={value} onChange={(e) => onChange(e.target.value)} placeholder="RLS-{Number}" />
      </label>
      {bad.length > 0 && <em className="error">Not a room parameter: {bad.map((t) => `{${t}}`).join(" ")}</em>}
      {known && known.length > 0 && (
        <span className="chips">
          {known.slice(0, 24).map((k) => (
            <button key={k} type="button" className="chip" onClick={() => onChange(withToken(value, k))}>{k}</button>
          ))}
        </span>
      )}
    </div>
  );
}
