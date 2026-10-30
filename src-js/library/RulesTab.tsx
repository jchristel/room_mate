// Tag rules: where a tag goes, learned from tags in Revit views (plan decision 24).
// Left: the rules, narrowed by words. Right: the selected rule's answer and the samples it was
// worked out from, each with where it was read. A rule can be deleted (the next push of a sheet
// that has the tag makes it again from that sheet's samples alone).
//
// Offsets are paper millimetres in the element's own frame: x along its facing, y to its left.

import { useCallback, useEffect, useMemo, useState } from "react";

import * as api from "./api.js";
import { degrees, filterRules, mm, ruleLabel, sampleSource, shortTime } from "./model.js";
import type { TabProps } from "./SheetsTab.js";
import type { RuleSummary, TagRule } from "./types.js";

const message = (e: unknown): string => String((e as Error).message ?? e);

export function RulesTab({ project, onError, onNotice }: Omit<TabProps, "rooms">) {
  const [rows, setRows] = useState<RuleSummary[] | null>(null);
  const [rule, setRule] = useState<TagRule | null>(null);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => api.listRules(project).then(setRows).catch((e: unknown) => onError(message(e))), [project, onError]);

  useEffect(() => {
    setRows(null);
    setRule(null);
    void refresh();
  }, [project, refresh]);

  const shown = useMemo(() => filterRules(rows ?? [], text), [rows, text]);

  const open = async (id: string) => {
    onError(null);
    onNotice(null);
    try {
      setRule(await api.getRule(project, id));
    } catch (e) {
      onError(message(e));
    }
  };

  const remove = async () => {
    if (!rule || !window.confirm(`Delete the rule for ${ruleLabel(rule)} in ${rule.key.viewport_type} views? Tasks already made keep their copy.`)) return;
    setBusy(true);
    const r = await api.deleteRule(project, rule.id);
    setBusy(false);
    if (!r.ok) return onError(r.error);
    onError(null);
    onNotice("Rule deleted.");
    setRule(null);
    void refresh();
  };

  return (
    <div className="page">
      <aside className="left" aria-label="Tag rules">
        <input type="search" placeholder="Filter: family, type, tag, view" value={text} onChange={(e) => setText(e.target.value)} aria-label="Filter rules" />
        {rows === null ? <p className="hint">Reading the rules…</p> : null}
        {rows?.length === 0 ? <p className="hint">No rules yet. Rules are learned from the tags on sheets exported with the Sheets or Blueprint button.</p> : null}
        {rows && rows.length > 0 && shown.length === 0 ? <p className="hint">No rule matches.</p> : null}
        <ul>
          {shown.map((r) => (
            <li key={r.id}>
              <button type="button" className={r.id === rule?.id ? "on" : ""} aria-pressed={r.id === rule?.id} onClick={() => void open(r.id)}>
                <b>{ruleLabel(r)}</b>
                <span>
                  {r.key.viewport_type} · {r.tag_kind} · {r.sample_count} sample{r.sample_count === 1 ? "" : "s"}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </aside>
      <main>
        {rule ? (
          <div className="detail">
            <div className="toolbar">
              <strong>{ruleLabel(rule)}</strong>
              <span className="sub">{rule.key.viewport_type}</span>
              <span className="spacer" />
              <button type="button" className="danger" disabled={busy} onClick={() => void remove()}>
                Delete
              </button>
            </div>
            <dl className="facts">
              <dt>Tagged</dt>
              <dd>
                {rule.key.tagged_category} · {rule.key.type_code || "(no code)"}
                {rule.key.code_parameter ? ` (${rule.key.code_parameter})` : ""}
              </dd>
              <dt>Tag</dt>
              <dd>
                {rule.key.tag_category} · {rule.key.tag_family} · {rule.key.tag_type} ({rule.tag_kind})
              </dd>
              <dt>Head</dt>
              <dd>
                {mm(rule.placement.head.x)} mm along, {mm(rule.placement.head.y)} mm left (paper)
              </dd>
              <dt>Rotation</dt>
              <dd>
                {degrees(rule.placement.rotation)}° from the facing · {rule.placement.orientation}
              </dd>
              <dt>Leader</dt>
              <dd>
                {rule.placement.has_leader ? `yes · ${rule.placement.leader_end_condition}` : "no"}
                {rule.placement.elbow ? ` · elbow ${mm(rule.placement.elbow.x)}, ${mm(rule.placement.elbow.y)}` : ""}
                {rule.placement.end ? ` · end ${mm(rule.placement.end.x)}, ${mm(rule.placement.end.y)}` : ""}
              </dd>
              <dt>Updated</dt>
              <dd>{shortTime(rule.updated_at)}</dd>
            </dl>
            <h3>Samples ({rule.samples.length})</h3>
            <p className="hint">The answer above is the median of these. Pushing a sheet again replaces that sheet's samples.</p>
            <table>
              <thead>
                <tr>
                  <th>Read from</th>
                  <th>Scale</th>
                  <th>Head (mm)</th>
                  <th>Rotation</th>
                  <th>Leader</th>
                </tr>
              </thead>
              <tbody>
                {rule.samples.map((s, i) => (
                  <tr key={i}>
                    <td>{sampleSource(s)}</td>
                    <td>1:{s.view_scale}</td>
                    <td>
                      {mm(s.head.x)}, {mm(s.head.y)}
                    </td>
                    <td>{degrees(s.rotation)}°</td>
                    <td>{s.has_leader ? "yes" : "no"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="hint empty">Choose a rule on the left.</p>
        )}
      </main>
    </div>
  );
}
