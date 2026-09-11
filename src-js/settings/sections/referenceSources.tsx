// Reference sources: where joined data comes from, what it joins ONTO, and the
// per-column declarations the QA report reads.
//
// **`entity` has never had a UI**, and it is the field that decides whether a
// source joins rooms, doors, windows, FF&E or spaces at all. A source scoped to
// one entity never joins another even when the key would match, so a source
// configured with the wrong entity looks configured and reports nothing.
//
// The CSV upload is here rather than on its own page because an upload IS the
// source's data: each one is stored as a dated snapshot a milestone can pin, and
// the response is what fills the column vocabulary the QA fields pick from.

import { useState } from "react";
import type { DragEvent } from "react";

import { Block, ListControls, OptionalSelect, OptionalText, Row, RowHead, Select, TextInput } from "../fields.js";
import type {
  CompareMode,
  FieldType,
  ReferenceEntity,
  ReferenceFieldConfig,
  ReferenceSourceConfig,
  ReferenceUploadResult,
  Settings,
} from "../types.js";

const ENTITIES: readonly (readonly [ReferenceEntity, string])[] = [
  ["rooms", "rooms"],
  ["doors", "doors"],
  ["windows", "windows"],
  ["ffe", "FF&E"],
  ["spaces", "spaces"],
];

const FIELD_TYPES: readonly (readonly [FieldType, string])[] = [
  ["string", "string"],
  ["numeric", "numeric"],
  ["date", "date"],
];

const QA_MODES: readonly (readonly [CompareMode, string])[] = [
  ["exact", "exact"],
  ["ignore", "ignore"],
];

/** What the last upload or read said about one source, shown on its card. */
export interface SourceStatus {
  kind: "ok" | "warn" | "bad" | "";
  text: string;
}

export function ReferenceSourcesSection({
  settings,
  edit,
  projectId,
  isNew,
  snapshotsBySource,
  labelsBySource,
  statusBySource,
  onUploaded,
  onStatus,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
  projectId: string | null;
  isNew: boolean;
  snapshotsBySource: Readonly<Record<string, string[] | null>>;
  labelsBySource: Readonly<Record<string, string[]>>;
  statusBySource: Readonly<Record<string, SourceStatus>>;
  onUploaded: (source: string, result: ReferenceUploadResult) => void;
  onStatus: (source: string, status: SourceStatus) => void;
}) {
  const sources = settings.sources.reference ?? {};
  const names = Object.keys(sources);

  const setSources = (next: Record<string, ReferenceSourceConfig>) =>
    edit((s) => ({ ...s, sources: { ...s.sources, reference: next } }));

  const rename = (from: string, to: string) => {
    // Rebuilt in order rather than deleted and re-added: the map IS the join
    // namespace, and a reordered one rewrites the TOML for no reason.
    const next: Record<string, ReferenceSourceConfig> = {};
    for (const [name, config] of Object.entries(sources)) next[name === from ? to : name] = config;
    setSources(next);
  };

  return (
    <Block
      title="Reference sources"
      hint={
        <>
          External data joined onto an entity (e.g. dRofus), keyed by name — the name is the join namespace a filter or
          comparison key writes as <code>&lt;name&gt;.&lt;label&gt;</code>. A project with none configured is a normal
          state. Each upload is stored server-side as a dated snapshot, so it is a version a milestone can pin.
        </>
      }
    >
      {names.length === 0 && <p className="row-head">none configured</p>}
      {names.map((name) => {
        const config = sources[name];
        if (config === undefined) return null;
        return (
          <SourceCard
            key={name}
            name={name}
            config={config}
            projectId={projectId}
            isNew={isNew}
            snapshots={snapshotsBySource[name] ?? null}
            labels={labelsBySource[name] ?? []}
            status={statusBySource[name] ?? { kind: "", text: "" }}
            onRename={(to) => rename(name, to)}
            onChange={(next) => setSources({ ...sources, [name]: next })}
            onRemove={() => {
              const next = { ...sources };
              delete next[name];
              setSources(next);
            }}
            onUploaded={(result) => onUploaded(name, result)}
            onStatus={(status) => onStatus(name, status)}
          />
        );
      })}
      <button
        type="button"
        className="action"
        onClick={() => {
          let name = "drofus";
          for (let n = 2; names.includes(name); n += 1) name = `source-${n}`;
          setSources({ ...sources, [name]: { entity: "rooms", fields: [], type: "upload" } });
        }}
      >
        + reference source
      </button>
    </Block>
  );
}

function SourceCard({
  name,
  config,
  projectId,
  isNew,
  snapshots,
  labels,
  status,
  onRename,
  onChange,
  onRemove,
  onUploaded,
  onStatus,
}: {
  name: string;
  config: ReferenceSourceConfig;
  projectId: string | null;
  isNew: boolean;
  snapshots: string[] | null;
  labels: readonly string[];
  status: SourceStatus;
  onRename: (to: string) => void;
  onChange: (next: ReferenceSourceConfig) => void;
  onRemove: () => void;
  onUploaded: (result: ReferenceUploadResult) => void;
  onStatus: (status: SourceStatus) => void;
}) {
  const [dragging, setDragging] = useState(false);
  const labelListId = `refLabels-${name}`;
  const locked = isNew || projectId === null;

  const upload = async (file: File | undefined) => {
    if (!file || projectId === null) return;
    onStatus({ kind: "", text: `uploading ${file.name}…` });
    try {
      const res = await fetch(
        `/projects/${encodeURIComponent(projectId)}/reference/${encodeURIComponent(name)}`,
        { method: "POST", headers: { "Content-Type": "text/csv" }, body: file },
      );
      const text = await res.text();
      if (!res.ok) {
        // The server's 422 is the real validator — shown verbatim.
        onStatus({ kind: "bad", text: `⚠ ${text}` });
        return;
      }
      const result = JSON.parse(text) as ReferenceUploadResult;
      // `record_count` is the DEDUPLICATED count, so a file with repeated or
      // blank ids stores fewer records than it has rows. Said here because this
      // is the one moment the operator is looking at the file they just picked.
      const lost: string[] = [];
      if (result.duplicate_ids.length > 0) {
        lost.push(`${result.duplicate_ids.length} repeated id(s), only the last row of each kept`);
      }
      if (result.blank_id_rows > 0) lost.push(`${result.blank_id_rows} row(s) with no id, skipped`);
      onStatus({
        kind: lost.length > 0 ? "warn" : "ok",
        text:
          `${lost.length > 0 ? "⚠" : "✓"} ${result.record_count} records · link property "${result.link_property}" · snapshot ${result.snapshot_taken_at}` +
          (result.stored ? " — applied live" : " — duplicate snapshot id, skipped") +
          (lost.length > 0 ? ` · ${lost.join(" · ")}` : ""),
      });
      onUploaded(result);
    } catch (err) {
      onStatus({ kind: "bad", text: `⚠ upload failed: ${err instanceof Error ? err.message : String(err)}` });
    }
  };

  const onDrop = (ev: DragEvent<HTMLDivElement>) => {
    ev.preventDefault();
    setDragging(false);
    if (!locked) void upload(ev.dataTransfer.files[0]);
  };

  return (
    <div className="ref-card">
      <div className="ref-card-head">
        <span>name</span>
        <TextInput value={name} size={14} onChange={onRename} title="the join namespace: <name>.<column>" />
        <span>joins</span>
        <Select
          value={config.entity}
          options={ENTITIES}
          title="Which primary entity this source joins onto. A source scoped to one entity never joins another, even when the key would match."
          onChange={(entity) => onChange({ ...config, entity })}
        />
        <span className="row-head">{config.type === "file" ? `legacy file origin: ${config.path}` : "upload"}</span>
        <button type="button" className="action icon" title="remove source" onClick={onRemove}>
          ✕
        </button>
      </div>

      {config.type === "file" && (
        <p className="ref-status bad">
          ⚠ <code>type = &quot;file&quot;</code> is removed: the server refuses to load it and says how to migrate.
          Upload the CSV here instead — every upload is a dated snapshot, which a path on disk never was.
        </p>
      )}

      <div
        className={`drop-zone${locked ? " disabled" : ""}${dragging ? " drag" : ""}`}
        onDragOver={(ev) => {
          ev.preventDefault();
          if (!locked) setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={onDrop}
      >
        <span>{locked ? "save the project first, then upload CSVs here —" : `drop a ${name} CSV here, or`}</span>
        <label className="action" style={{ cursor: locked ? "default" : "pointer" }}>
          choose file…
          <input
            type="file"
            accept=".csv,text/csv"
            hidden
            disabled={locked}
            onChange={(ev) => {
              void upload(ev.target.files?.[0]);
              ev.target.value = ""; // allow re-uploading the same file
            }}
          />
        </label>
      </div>

      {status.text !== "" && <div className={`ref-status ${status.kind}`}>{status.text}</div>}

      <ul className="ref-snapshot-list">
        {snapshots === null && <li className="row-head">loading stored uploads…</li>}
        {snapshots !== null && snapshots.length === 0 && <li className="row-head">no uploads stored yet</li>}
        {snapshots !== null &&
          [...snapshots].reverse().map((taken_at, index) => (
            <li key={taken_at}>
              <span>{taken_at} </span>
              {index === 0 && <span className="latest-mark">← latest (live)</span>}
            </li>
          ))}
      </ul>

      <div className="ref-fields">
        <p className="hint">
          Per-column type/comparison declarations for the validation report. Labels come from row 1 of the latest
          uploaded CSV.
        </p>
        <datalist id={labelListId}>
          {labels.map((label) => (
            <option key={label} value={label} />
          ))}
        </datalist>
        <div className="rows">
          {config.fields.map((field, index) => (
            <FieldRow
              key={`${index}:${field.label}`}
              field={field}
              index={index}
              labelListId={labelListId}
              fields={config.fields}
              onChange={(next) => onChange({ ...config, fields: config.fields.map((f, i) => (i === index ? next : f)) })}
              onList={(next) => onChange({ ...config, fields: next })}
            />
          ))}
        </div>
        <button
          type="button"
          className="action"
          onClick={() =>
            onChange({
              ...config,
              fields: [...config.fields, { label: "", type: "string", format: null, revit_format: null, qa: null }],
            })
          }
        >
          + field
        </button>
      </div>
    </div>
  );
}

function FieldRow({
  field,
  index,
  fields,
  labelListId,
  onChange,
  onList,
}: {
  field: ReferenceFieldConfig;
  index: number;
  fields: readonly ReferenceFieldConfig[];
  labelListId: string;
  onChange: (next: ReferenceFieldConfig) => void;
  onList: (next: ReferenceFieldConfig[]) => void;
}) {
  return (
    <Row>
      <RowHead>{index + 1}.</RowHead>
      <span>label</span>
      <TextInput value={field.label} size={14} list={labelListId} onChange={(label) => onChange({ ...field, label })} />
      <span>type</span>
      <Select value={field.type} options={FIELD_TYPES} onChange={(type) => onChange({ ...field, type })} />
      {field.type === "date" && (
        <>
          <span>format</span>
          <OptionalText
            value={field.format}
            size={20}
            placeholder="%-m/%-d/%Y %-I:%M:%S %p %z"
            onChange={(format) => onChange({ ...field, format })}
          />
          <span>revit format</span>
          <OptionalText
            value={field.revit_format}
            size={16}
            placeholder="(same as format)"
            onChange={(revit_format) => onChange({ ...field, revit_format })}
          />
        </>
      )}
      <span>qa</span>
      <OptionalSelect
        value={field.qa}
        options={QA_MODES}
        absentLabel="default"
        title="QA comparison override"
        onChange={(qa) => onChange({ ...field, qa })}
      />
      <ListControls list={fields} index={index} onChange={onList} />
    </Row>
  );
}
