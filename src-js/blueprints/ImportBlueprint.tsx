// Bring a blueprint in from another project on this server, or from a file.
//
// The rule is in `exchange.ts`: an import never overwrites. A name the project already has gets
// the incoming revision as a PENDING one, to accept or reject on the timeline; a new name is
// created. The result says which happened, because the two look the same until you open the list.

import { useEffect, useState } from "react";

import * as api from "./api.js";
import { importInto, parseFile, toFile, type ExportFile, type Imported } from "./exchange.js";
import type { BlueprintSummary, ProjectRow } from "./types.js";

export interface ImportProps {
  project: string;
  projects: readonly ProjectRow[];
  existing: readonly BlueprintSummary[];
  onImported(result: Imported): void;
  onCancel(): void;
}

export function ImportBlueprint({ project, projects, existing, onImported, onCancel }: ImportProps) {
  const others = projects.filter((p) => p.id !== project);
  const [source, setSource] = useState(others[0]?.id ?? "");
  const [list, setList] = useState<BlueprintSummary[]>([]);
  const [pick, setPick] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setList([]);
    setPick("");
    if (!source) return;
    api
      .listBlueprints(source)
      .then(setList)
      .catch((e: unknown) => setError(String((e as Error).message ?? e)));
  }, [source]);

  const run = async (file: ExportFile, origin: string) => {
    setBusy(true);
    setError(null);
    const r = await importInto(project, existing, file, origin);
    setBusy(false);
    if (r.ok) onImported(r.value);
    else setError(r.error);
  };

  const fromProject = async () => {
    setBusy(true);
    try {
      const b = await api.getBlueprint(source, pick);
      const file = toFile(b, b.current_revision, source);
      if (!file) throw new Error("that blueprint has no current revision");
      await run(file, `project ${projects.find((p) => p.id === source)?.name ?? source}`);
    } catch (e) {
      setError(String((e as Error).message ?? e));
      setBusy(false);
    }
  };

  const fromFile = async (f: File | undefined) => {
    if (!f) return;
    const parsed = parseFile(await f.text());
    if (!parsed.ok) return setError(parsed.error);
    await run(parsed.file, `file ${f.name}`);
  };

  return (
    <section className="panel">
      <h3>Import a blueprint</h3>
      <p className="hint">
        Never overwrites. If this project has a blueprint of the same name, the import waits on its timeline as a pending revision for you
        to accept or reject.
      </p>
      {error ? <p className="error">{error}</p> : null}
      {others.length > 0 ? (
        <fieldset>
          <legend>From another project on this server</legend>
          <select value={source} onChange={(e) => setSource(e.target.value)} aria-label="Source project">
            {others.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
          <select value={pick} onChange={(e) => setPick(e.target.value)} aria-label="Blueprint to import">
            <option value="">{list.length ? "Choose a blueprint" : "No blueprints there"}</option>
            {list.map((b) => (
              <option key={b.id} value={b.id}>
                {b.name} (r{b.current_revision}, {b.items} items)
              </option>
            ))}
          </select>
          <button type="button" disabled={busy || !pick} onClick={() => void fromProject()}>
            Import its current revision
          </button>
        </fieldset>
      ) : null}
      <fieldset>
        <legend>From a file</legend>
        <input type="file" accept=".json,application/json" disabled={busy} onChange={(e) => void fromFile(e.target.files?.[0])} />
      </fieldset>
      <div className="actions">
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </section>
  );
}
