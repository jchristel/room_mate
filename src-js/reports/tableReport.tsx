// A schedule, or one entity by room: the column pickers, the row shape, and
// the preview.
//
// **The rows come from the server** (`POST /projects/{id}/reports`), projected
// to the columns asked for. That is not an optimisation detail a reader can
// ignore: `/ffe` is 133 MB for one RHH storey, and a report of it is a few, so
// a page that fetched the entity and projected locally would be unusable on
// the project this was built for.
//
// CSV is the same request with `format=csv`, so the download is the server's
// rendering rather than a second one written here.

import { useCallback, useEffect, useMemo, useState } from "react";

import { apiGet, apiSend } from "./common.js";
import { downloadCsv } from "./csv.js";
import { entityById, ROOM_COLUMNS } from "./reportTypes.js";
import { fromWire, group, toWire, type Group } from "./filter.js";
import { idFor, toForm, toSaved } from "./savedReports.js";
import type { SavedReport } from "./generated/SavedReport.js";
import { FilterBuilder, type FieldOption } from "./filterBuilder.js";
import { summarise, toBody, type FormState } from "./reportRequest.js";
import type { ColumnCatalog, MilestonesResponse, ReportResponse } from "./types.js";

interface Props {
  projectId: string;
  entityId: string;
  byRoom: boolean;
  /** A saved report to open, when the reader picked one from the rail. */
  saved?: SavedReport | null;
  /** Told after a save or a delete, so the rail can re-read itself. */
  onSaved?: (id: string) => void;
}

const CAP = 500;

export function TableReport({ projectId, entityId, byRoom, saved, onSaved }: Props) {
  const entity = entityById(entityId);
  const [form, setForm] = useState<FormState>(() => (saved ? toForm(saved) : initialForm(entityId, byRoom)));
  const [name, setName] = useState(saved?.name ?? "");
  const [savedId, setSavedId] = useState<string | null>(saved?.id ?? null);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);
  const [catalog, setCatalog] = useState<ColumnCatalog | null>(null);
  const [milestones, setMilestones] = useState<string[]>([]);
  const [milestone, setMilestone] = useState("");
  const [filter, setFilter] = useState<Group>(() => group("all"));
  const [report, setReport] = useState<ReportResponse | null>(null);
  const [state, setState] = useState<"idle" | "loading" | "empty">("idle");
  const [error, setError] = useState<string | null>(null);

  // The form is per (entity, shape): switching report type starts from that
  // entity's defaults rather than carrying the last one's columns, which would
  // ask the server for properties this entity has never heard of.
  useEffect(() => {
    // Opening a saved report replaces the whole form; switching report type
    // starts from that entity's defaults. Either way the filter goes, because
    // it names fields of one entity and the server would otherwise be asked for
    // properties this entity has never heard of.
    setForm(saved ? toForm(saved) : initialForm(entityId, byRoom));
    setFilter(saved?.filter ? fromWire(saved.filter) : group("all"));
    setName(saved?.name ?? "");
    setSavedId(saved?.id ?? null);
    setSaveMsg(null);
    setReport(null);
    setError(null);
    setState("idle");
  }, [entityId, byRoom, saved]);

  // What this project's snapshots actually carry, read from their property
  // dictionaries rather than guessed from names. A project that has pushed
  // nothing of this entity answers with the intrinsics alone, which is the
  // honest list rather than an empty one.
  useEffect(() => {
    let live = true;
    setCatalog(null);
    apiGet<ColumnCatalog>(
      `/projects/${encodeURIComponent(projectId)}/reports/columns?entity=${encodeURIComponent(entityId)}`,
    )
      .then((c) => live && setCatalog(c))
      .catch(() => live && setCatalog(null));
    return () => {
      live = false;
    };
  }, [projectId, entityId]);

  useEffect(() => {
    apiGet<MilestonesResponse>(`/projects/${encodeURIComponent(projectId)}/milestones`)
      .then((d) => setMilestones((d.milestones ?? []).map((m) => m.name)))
      .catch(() => setMilestones([]));
    setMilestone("");
    setReport(null);
  }, [projectId]);

  const body = useMemo(
    () => ({ ...toBody(form), milestone: milestone || undefined, filter: toWire(filter) }),
    [form, milestone, filter],
  );

  // What a condition may name. The served catalog decides both the list and
  // each field's type, which is what the filter needs to offer the right
  // operators — a property called "Area Check" is not a number because of its
  // name, and Revit already said which it is.
  const fields = useMemo<FieldOption[]>(() => {
    if (!catalog) return [];
    const out: FieldOption[] = [];
    const push = (side: "room" | "element" | "join", sideLabel: string, columns: typeof catalog.entity) => {
      for (const c of columns) {
        out.push({ side, name: c.name, type: c.value_type === "number" ? "number" : "text", sideLabel });
      }
    };
    if (byRoom) push("room", "Room", catalog.rooms);
    push("element", entity.label, catalog.entity);
    if (byRoom) push("join", "Join", catalog.measures);
    return out;
  }, [catalog, byRoom, entity.label]);

  // The pickers offer the same vocabulary, and still take free text: a name the
  // dictionary does not carry may exist on an older snapshot, and refusing it
  // would be a guess in the other direction.
  const suggestions = useMemo(
    () => ({
      rooms: (catalog?.rooms ?? []).map((c) => c.name),
      entity: (catalog?.entity ?? []).map((c) => c.name),
      measures: (catalog?.measures ?? []).map((c) => c.name),
    }),
    [catalog],
  );

  const run = useCallback(async () => {
    setState("loading");
    setError(null);
    const response = await fetch(`/projects/${encodeURIComponent(projectId)}/reports`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
      cache: "no-store",
    });
    // 204 is "nothing of this entity has ever been pushed", which is a
    // different answer from a report with no rows — and the one a table cannot
    // tell on its own.
    if (response.status === 204) {
      setReport(null);
      setState("empty");
      return;
    }
    if (!response.ok) {
      setError(await response.text());
      setState("idle");
      return;
    }
    setReport((await response.json()) as ReportResponse);
    setState("idle");
  }, [projectId, body]);

  const save = useCallback(async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      setSaveMsg("a saved report needs a name");
      return;
    }
    // The id is derived once and then kept: renaming a report must not orphan
    // the document, or a link to it breaks.
    const id = savedId ?? idFor(trimmed);
    const document = toSaved(id, trimmed, form, milestone, toWire(filter));
    const result = await apiSend(
      "PUT",
      `/api/reports/projects/${encodeURIComponent(projectId)}/${encodeURIComponent(id)}`,
      document,
    );
    if (!result.ok) {
      setSaveMsg(result.text);
      return;
    }
    setSavedId(id);
    setSaveMsg("saved");
    onSaved?.(id);
  }, [name, savedId, form, milestone, filter, projectId, onSaved]);

  const remove = useCallback(async () => {
    if (!savedId) return;
    const result = await apiSend(
      "DELETE",
      `/api/reports/projects/${encodeURIComponent(projectId)}/${encodeURIComponent(savedId)}`,
      null,
    );
    if (!result.ok) {
      setSaveMsg(result.text);
      return;
    }
    setSavedId(null);
    setSaveMsg("deleted");
    onSaved?.("");
  }, [savedId, projectId, onSaved]);

  const exportCsv = useCallback(async () => {
    const response = await fetch(`/projects/${encodeURIComponent(projectId)}/reports?format=csv`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      // The export is the whole answer, never the preview's cap.
      body: JSON.stringify({ ...body, limit: undefined }),
      cache: "no-store",
    });
    if (!response.ok) {
      setError(await response.text());
      return;
    }
    downloadCsv(`${entityId}${byRoom ? "-by-room" : ""}-${projectId}.csv`, await response.text());
  }, [projectId, body, entityId, byRoom]);

  const summary = report ? summarise(report.rows.length, report.total_rows, report.unmatched_rows, entity) : null;

  return (
    <>
      <section className="group">
        <div className="add-row">
          <input
            type="text"
            value={name}
            placeholder="name this report"
            aria-label="Report name"
            onChange={(e) => setName(e.target.value)}
          />
          <button className="action" type="button" onClick={() => void save()}>
            {savedId ? "Save" : "Save report"}
          </button>
          {savedId && (
            <button className="action" type="button" onClick={() => void remove()}>
              Delete
            </button>
          )}
          {saveMsg && <span className={`msg ${saveMsg === "saved" || saveMsg === "deleted" ? "ok" : "err"}`}>{saveMsg}</span>}
        </div>
      </section>

      <section className="pickers">
        {byRoom && (
          <ColumnPicker
            title="Room columns"
            hint="Any room property, or a $intrinsic."
            chosen={form.roomColumns}
            suggestions={suggestions.rooms}
            onChange={(roomColumns) => setForm({ ...form, roomColumns })}
          />
        )}
        <ColumnPicker
          title={`${entity.label} columns`}
          hint={`Properties of each ${entity.one}, instance tier then type.`}
          chosen={form.columns}
          suggestions={suggestions.entity}
          onChange={(columns) => setForm({ ...form, columns })}
        />
        {byRoom && suggestions.measures.length > 0 && (
          <ColumnPicker
            title="Measures from the join"
            hint="What linking them produced. Belongs to neither side."
            chosen={form.measures}
            suggestions={suggestions.measures}
            onChange={(measures) => setForm({ ...form, measures })}
            accent
          />
        )}
      </section>

      <FilterBuilder root={filter} fields={fields} entityOne={entity.one} onChange={setFilter} />

      <section className="controls">
        <div className="group">
          <h2>Scope</h2>
          <div className="field">
            <label htmlFor="ms">Read from</label>
            <select id="ms" value={milestone} onChange={(e) => setMilestone(e.target.value)}>
              <option value="">Latest snapshot</option>
              {milestones.map((m) => (
                <option key={m} value={m}>
                  Milestone: {m}
                </option>
              ))}
            </select>
          </div>
        </div>

        {byRoom && (
          <>
            <div className="group">
              <h2>Rows</h2>
              <label className="check">
                <input
                  type="radio"
                  name="shape"
                  checked={form.shape === "per_match"}
                  onChange={() => setForm({ ...form, shape: "per_match" })}
                />
                One row per match
              </label>
              <label className="check">
                <input
                  type="radio"
                  name="shape"
                  checked={form.shape === "per_room"}
                  onChange={() => setForm({ ...form, shape: "per_room" })}
                />
                One row per room <span className="n">(counts and value lists)</span>
              </label>
            </div>
            <div className="group">
              <h2>Unmatched</h2>
              <label className="check">
                <input
                  type="checkbox"
                  checked={form.includeRoomsWithout}
                  onChange={(e) => setForm({ ...form, includeRoomsWithout: e.target.checked })}
                />
                Include rooms with no {entity.many}
              </label>
              <label className="check">
                <input
                  type="checkbox"
                  checked={form.includeUnattributed}
                  onChange={(e) => setForm({ ...form, includeUnattributed: e.target.checked })}
                />
                Include {entity.many} with no room
              </label>
            </div>
          </>
        )}
      </section>

      <section className="group">
        <div className="preview-head">
          <h2>Rows</h2>
          <span className="count">{summary?.shown ?? ""}</span>
          <button className="action primary" type="button" onClick={() => void run()}>
            {state === "loading" ? "Reading…" : "Run report"}
          </button>
          <button className="action" type="button" disabled={!report?.rows.length} onClick={() => void exportCsv()}>
            Export CSV
          </button>
        </div>

        {error && <p className="msg err">{error}</p>}
        {state === "empty" && (
          <div className="note">
            No {entity.many} have ever been pushed for this project. That is not an empty result — nobody has run the
            exporter.
          </div>
        )}
        {/* The server explaining its own answer — "no space key is configured",
            "this project has no rooms" — which the rows cannot say. */}
        {report?.note && <div className="note">{report.note}</div>}
        {summary?.unmatched && <div className="note">{summary.unmatched}</div>}
        {report && report.rows.length === 0 && state === "idle" && <p className="foot">No rows.</p>}

        {report && report.rows.length > 0 && (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  {report.columns.map((c, i) => (
                    <th key={`${c.name}-${i}`} className={c.side === "room" ? "" : "edge"}>
                      {c.name}
                      <span className="agg">{c.side}</span>
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {report.rows.map((row, i) => (
                  <tr key={i}>
                    {row.map((cell, j) => (
                      <td key={j}>{cell}</td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </>
  );
}

function initialForm(entityId: string, byRoom: boolean): FormState {
  const entity = entityById(entityId);
  return {
    entityId,
    byRoom,
    columns: [...entity.columns],
    roomColumns: [...ROOM_COLUMNS],
    measures: [...entity.defaultMeasures],
    shape: "per_match",
    includeRoomsWithout: false,
    // On by default: an element no room matched is part of the answer.
    includeUnattributed: true,
    limit: CAP,
  };
}

/**
 * Chips plus a free-text box.
 *
 * **Free text, not a closed list**, and that is a gap rather than a choice: a
 * project's property vocabulary is whatever its models carry, and nothing
 * serves that list yet. The suggestions are the intrinsics and the names every
 * model measured so far has. See the column-discovery open question in
 * docs/STRATEGY-REPORTS.md.
 */
function ColumnPicker({
  title,
  hint,
  chosen,
  suggestions,
  onChange,
  accent,
}: {
  title: string;
  hint: string;
  chosen: string[];
  suggestions: string[];
  onChange: (next: string[]) => void;
  accent?: boolean;
}) {
  const [draft, setDraft] = useState("");
  const listId = `suggest-${title.replace(/\W+/g, "-").toLowerCase()}`;
  const add = () => {
    const value = draft.trim();
    if (value && !chosen.includes(value)) onChange([...chosen, value]);
    setDraft("");
  };

  return (
    <div className="picker">
      <h2>{title}</h2>
      <p className="hint">{hint}</p>
      <div className="chips">
        {chosen.length === 0 && <span className="hint">none</span>}
        {chosen.map((name, i) => (
          <span className={`chip${accent ? " measure" : ""}`} key={`${name}-${i}`}>
            {name}
            <button type="button" aria-label={`Remove ${name}`} onClick={() => onChange(chosen.filter((_, j) => j !== i))}>
              ×
            </button>
          </span>
        ))}
      </div>
      <div className="add-row">
        <input
          type="text"
          value={draft}
          list={listId}
          placeholder="property name"
          aria-label={`Add to ${title}`}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== "Enter") return;
            e.preventDefault();
            add();
          }}
        />
        <datalist id={listId}>
          {suggestions.filter((s) => !chosen.includes(s)).map((s) => (
            <option key={s} value={s} />
          ))}
        </datalist>
        <button className="action" type="button" onClick={add}>
          Add
        </button>
      </div>
    </div>
  );
}
