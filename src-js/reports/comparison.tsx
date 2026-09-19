// Milestone comparison — the page `static/comparison.html` used to be, moved
// here under the Between-milestones family and deleted there in the same
// change, the way `settings.html` went.
//
// **The save rule it inherits is the important part.** The whole settings
// object is held as it was read and sent back with only `comparison_key` and
// `comparison_properties` changed. Rebuilding that JSON field by field is what
// silently emptied every milestone's pins from the old settings page, and this
// form edits the same file.

import { useCallback, useEffect, useState } from "react";

import { apiGet, apiSend } from "./common.js";
import type {
  ComparisonResponse,
  Milestone,
  MilestonesResponse,
  Settings,
  SettingsResponse,
} from "./types.js";

interface Props {
  projectId: string;
}

export function ComparisonReport({ projectId }: Props) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [key, setKey] = useState("");
  const [properties, setProperties] = useState<string[]>([]);
  const [draft, setDraft] = useState("");
  const [milestones, setMilestones] = useState<Milestone[]>([]);
  const [baseline, setBaseline] = useState<string | null>(null);
  const [others, setOthers] = useState<string[]>([]);
  const [saveMsg, setSaveMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [runMsg, setRunMsg] = useState<string | null>(null);
  const [result, setResult] = useState<ComparisonResponse | null>(null);

  useEffect(() => {
    let live = true;
    setResult(null);
    setSaveMsg(null);
    setRunMsg(null);
    setBaseline(null);
    setOthers([]);

    apiGet<SettingsResponse>(`/api/settings/projects/${encodeURIComponent(projectId)}`)
      .then((d) => {
        if (!live) return;
        setSettings(d.settings);
        setKey(d.settings.comparison_key ?? "");
        setProperties([...(d.settings.comparison_properties ?? [])]);
      })
      .catch((e: unknown) => live && setSaveMsg({ ok: false, text: String((e as Error).message ?? e) }));

    apiGet<MilestonesResponse>(`/projects/${encodeURIComponent(projectId)}/milestones`)
      .then((d) => live && setMilestones(d.milestones ?? []))
      .catch(() => live && setMilestones([]));

    return () => {
      live = false;
    };
  }, [projectId]);

  const save = useCallback(async () => {
    if (!settings) {
      setSaveMsg({ ok: false, text: "no settings loaded for this project" });
      return;
    }
    // Only the two comparison fields are touched; everything else goes back
    // exactly as it was read.
    const body: Settings = { ...settings, comparison_key: key.trim() || null, comparison_properties: [...properties] };
    const res = await apiSend("PUT", `/api/settings/projects/${encodeURIComponent(projectId)}`, body);
    if (res.ok) {
      setSettings((JSON.parse(res.text) as SettingsResponse).settings);
      setSaveMsg({ ok: true, text: "saved and applied live" });
    } else {
      // The server's 422 is the real validator — show it verbatim.
      setSaveMsg({ ok: false, text: res.text });
    }
  }, [settings, key, properties, projectId]);

  const run = useCallback(async () => {
    if (!baseline || !others.length) return;
    setRunMsg(null);
    const res = await apiSend("POST", `/projects/${encodeURIComponent(projectId)}/comparison`, { baseline, others });
    if (!res.ok) {
      setRunMsg(res.text);
      setResult(null);
      return;
    }
    setResult(JSON.parse(res.text) as ComparisonResponse);
  }, [baseline, others, projectId]);

  return (
    <>
      <section className="group">
        <h2>Comparison key</h2>
        <p className="foot">
          The room property whose value identifies &ldquo;the same room&rdquo; across milestones. Its own setting, not a
          reference source&rsquo;s link property. Without one, no comparison can run.
        </p>
        <div className="field">
          <input type="text" value={key} placeholder="e.g. Number" onChange={(e) => setKey(e.target.value)} />
        </div>
      </section>

      <section className="group">
        <h2>Compared properties</h2>
        <p className="foot">
          Diffed on rooms present in both a milestone and the baseline. A property absent on the other side is reported
          as missing, not as a difference.
        </p>
        <div className="chips">
          {properties.length === 0 && <span className="foot">none — only room add/remove will be reported</span>}
          {properties.map((name, i) => (
            <span className="chip" key={`${name}-${i}`}>
              {name}
              <button
                type="button"
                aria-label={`Remove ${name}`}
                onClick={() => setProperties(properties.filter((_, j) => j !== i))}
              >
                ×
              </button>
            </span>
          ))}
        </div>
        <div className="add-row">
          <input
            type="text"
            value={draft}
            placeholder="property name"
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== "Enter") return;
              e.preventDefault();
              const v = draft.trim();
              if (v && !properties.includes(v)) setProperties([...properties, v]);
              setDraft("");
            }}
          />
          <button
            className="action"
            type="button"
            onClick={() => {
              const v = draft.trim();
              if (v && !properties.includes(v)) setProperties([...properties, v]);
              setDraft("");
            }}
          >
            Add
          </button>
        </div>
        <div className="add-row">
          <button className="action primary" type="button" onClick={() => void save()}>
            Save config
          </button>
          {saveMsg && <span className={`msg ${saveMsg.ok ? "ok" : "err"}`}>{saveMsg.text}</span>}
        </div>
      </section>

      <section className="group">
        <h2>Milestones</h2>
        {milestones.length === 0 ? (
          <p className="foot">This project defines no milestones. Add some in settings first.</p>
        ) : (
          <div className="milestones">
            <span className="hd">Baseline</span>
            <span className="hd">Compare</span>
            <span />
            {milestones.map((m) => (
              <MilestoneRow
                key={m.name}
                milestone={m}
                baseline={baseline}
                others={others}
                onBaseline={() => {
                  setBaseline(m.name);
                  setOthers(others.filter((o) => o !== m.name));
                }}
                onOther={(on) => setOthers(on ? [...others, m.name] : others.filter((o) => o !== m.name))}
              />
            ))}
          </div>
        )}
        <div className="add-row">
          <button className="action primary" type="button" disabled={!baseline || !others.length} onClick={() => void run()}>
            Compare
          </button>
          {runMsg && <span className="msg err">{runMsg}</span>}
        </div>
      </section>

      {result && <ComparisonResult data={result} />}
    </>
  );
}

function MilestoneRow({
  milestone,
  baseline,
  others,
  onBaseline,
  onOther,
}: {
  milestone: Milestone;
  baseline: string | null;
  others: string[];
  onBaseline: () => void;
  onOther: (on: boolean) => void;
}) {
  return (
    <>
      <input
        type="radio"
        name="baseline"
        checked={baseline === milestone.name}
        onChange={onBaseline}
        aria-label={`Baseline ${milestone.name}`}
      />
      <input
        type="checkbox"
        checked={others.includes(milestone.name)}
        disabled={baseline === milestone.name}
        onChange={(e) => onOther(e.target.checked)}
        aria-label={`Compare ${milestone.name}`}
      />
      <span>
        {milestone.name} <span className="date">{milestone.date}</span>
      </span>
    </>
  );
}

function ComparisonResult({ data }: { data: ComparisonResponse }) {
  if (!data.comparison_key_configured) {
    return (
      <section className="group">
        <div className="note">
          No comparison key configured for this project. Set one above — the room property that identifies the same room
          across milestones — save, then compare.
        </div>
      </section>
    );
  }

  return (
    <section className="group">
      <h2>Result</h2>
      <p className="foot">
        Baseline &ldquo;{data.baseline}&rdquo; · matched on {data.comparison_key} ·{" "}
        {data.compared_properties.length ? `properties: ${data.compared_properties.join(", ")}` : "no properties compared"}
      </p>

      {data.baseline_duplicate_key_values.length > 0 && (
        <div className="note">
          <strong>Baseline has ambiguous keys</strong> (excluded from matching):{" "}
          {data.baseline_duplicate_key_values.map((d) => `${d.value} [${d.room_ids.join(", ")}]`).join("; ")}
        </div>
      )}

      {data.comparisons.map((cmp) => {
        const quiet =
          !cmp.rooms_added.length &&
          !cmp.rooms_removed.length &&
          !cmp.changed_rooms.length &&
          !cmp.duplicate_key_values.length;
        return (
          <div className="cmp" key={cmp.milestone}>
            <h3>
              {cmp.milestone} <span className="base">vs {data.baseline}</span>
            </h3>
            {quiet && <p className="foot">No differences from the baseline.</p>}

            {cmp.rooms_added.length > 0 && (
              <Group title={`Rooms added (${cmp.rooms_added.length})`}>
                <ul>
                  {cmp.rooms_added.map((r) => (
                    <li key={r}>{r}</li>
                  ))}
                </ul>
              </Group>
            )}
            {cmp.rooms_removed.length > 0 && (
              <Group title={`Rooms removed (${cmp.rooms_removed.length})`}>
                <ul>
                  {cmp.rooms_removed.map((r) => (
                    <li key={r}>{r}</li>
                  ))}
                </ul>
              </Group>
            )}
            {cmp.changed_rooms.length > 0 && (
              <Group title={`Rooms changed (${cmp.changed_rooms.length})`}>
                <ul>
                  {cmp.changed_rooms.map((room) => (
                    <li key={room.room_id}>
                      <strong>room {room.room_id}</strong> ({data.comparison_key} {room.key})
                      <ul>
                        {room.differences.map((d) => (
                          <li key={d.property}>
                            {d.property}: <span className="val">{d.baseline_value || '""'}</span>
                            <span className="arrow">→</span>
                            <span className="val">{d.other_value || '""'}</span>
                          </li>
                        ))}
                        {room.missing_properties.map((m) => (
                          <li key={m.property}>
                            {m.property}: <span className="val">{m.baseline_value}</span>
                            <span className="miss"> · missing</span>
                          </li>
                        ))}
                        {/* An unjoined source rendered as nothing would be the silent
                            no-op this line exists to remove. */}
                        {(room.unjoined_sources ?? []).map((s) => (
                          <li key={s}>
                            <span className="miss">{`${s}: no ${s} record matched this room in this milestone`}</span>
                          </li>
                        ))}
                      </ul>
                    </li>
                  ))}
                </ul>
              </Group>
            )}
            {cmp.duplicate_key_values.length > 0 && (
              <Group title="Ambiguous keys (excluded)">
                <p className="msg err">
                  {cmp.duplicate_key_values.map((d) => `${d.value} [${d.room_ids.join(", ")}]`).join("; ")}
                </p>
              </Group>
            )}
          </div>
        );
      })}
    </section>
  );
}

function Group({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="grp">
      <h4>{title}</h4>
      {children}
    </div>
  );
}
