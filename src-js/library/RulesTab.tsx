// Tag rules: where a tag goes, learned from tags in Revit views (plan decision 24).
// Left: the rules, narrowed by words. Right: the selected rule's answer and the samples it was
// worked out from, each with where it was read. A rule can be deleted (the next push of a sheet
// that has the tag makes it again from that sheet's samples alone).
//
// Offsets are paper millimetres in the element's own frame: x along its facing, y to its left.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import * as api from "./api.js";
import { boundaryText, degrees, filterRules, mm, ruleLabel, sampleSource, scopeText, shortTime } from "./model.js";
import { ListPane, Row, RowList, SearchBox } from "../ui/ListPane.js";
import { MultiPanel, MultiToggle, useSelection } from "../ui/Selection.js";
import type { TabProps } from "./SheetsTab.js";
import type { RuleSummary, TagRule } from "./types.js";
import { message } from "../ui/api.js";
import { batches } from "../ui/batch.js";
import { saveJson } from "../ui/file.js";
import { fileNameFor, outcomeText, parseFile, sum, toFile, type ImportOutcome } from "./rulesExchange.js";


export function RulesTab({ project, onError, onNotice }: Omit<TabProps, "rooms">) {
  const [rows, setRows] = useState<RuleSummary[] | null>(null);
  const [rule, setRule] = useState<TagRule | null>(null);
  const [text, setText] = useState("");
  const selection = useSelection();
  const [busy, setBusy] = useState(false);
  const [names, setNames] = useState<ReadonlyMap<string, string>>(new Map());

  const refresh = useCallback(() => api.listRules(project).then(setRows).catch((e: unknown) => onError(message(e))), [project, onError]);

  useEffect(() => {
    setRows(null);
    setRule(null);
    void refresh();
    api.listBlueprintNames(project).then(setNames).catch(() => setNames(new Map()));
  }, [project, refresh]);

  const shown = useMemo(() => filterRules(rows ?? [], text), [rows, text]);

  const ticked = useMemo(() => (rows ?? []).filter((r) => selection.picked.has(r.id)), [rows, selection.picked]);
  const isOn = (id: string) => (selection.multi ? selection.picked.has(id) : id === rule?.id);

  const removeTicked = async () => {
    if (!window.confirm(`Delete ${ticked.length} rule${ticked.length === 1 ? "" : "s"}? Tasks already made keep their copy.`)) return;
    setBusy(true);
    const failed: string[] = [];
    const left: string[] = [];
    for (const r of ticked) {
      const d = await api.deleteRule(project, r.id);
      if (!d.ok) {
        failed.push(`${ruleLabel(r)}: ${d.error}`);
        left.push(r.id);
      }
    }
    setBusy(false);
    selection.set(left);
    if (rule && ticked.some((r) => r.id === rule.id && !left.includes(r.id))) setRule(null);
    onError(failed.length > 0 ? `Not deleted: ${failed.join("; ")}` : null);
    onNotice(`${ticked.length - failed.length} rule${ticked.length - failed.length === 1 ? "" : "s"} deleted.`);
    void refresh();
  };

  // Export what is ticked (multi-selection), else what the filter shows. A rule is read whole: the list holds summaries.
  const exportTargets = selection.multi && ticked.length > 0 ? ticked : shown;
  const exportFile = async () => {
    setBusy(true);
    onError(null);
    try {
      const rules: TagRule[] = [];
      for (let i = 0; i < exportTargets.length; i += 6) rules.push(...(await Promise.all(exportTargets.slice(i, i + 6).map((r) => api.getRule(project, r.id)))));
      saveJson(fileNameFor(project, rules.length), toFile(rules, project));
      onNotice(`Exported ${rules.length} rule${rules.length === 1 ? "" : "s"}.`);
    } catch (e) {
      onError(message(e));
    }
    setBusy(false);
  };

  const fileInput = useRef<HTMLInputElement>(null);
  const importFile = async (f: File | undefined) => {
    if (!f) return;
    onError(null);
    onNotice(null);
    const parsed = parseFile(await f.text());
    if (!parsed.ok) return onError(parsed.error);
    const { rules, exported_from } = parsed.file;
    const from = exported_from === project ? "this project" : `"${exported_from}"`;
    if (!window.confirm(`Import ${rules.length} rule${rules.length === 1 ? "" : "s"} from ${from}? They are taken in as generic rules: samples join a rule of the same key, and a sample already held by another rule moves to the one it is filed under now.`)) return;
    setBusy(true);
    const done: ImportOutcome[] = [];
    for (const group of batches(rules)) {
      const r = await api.importRules(project, group);
      if (!r.ok) {
        setBusy(false);
        onError(`${r.error}${done.length > 0 ? ` (${sum(done).samples} samples were taken in before that)` : ""}`);
        void refresh();
        return;
      }
      done.push(r.value);
    }
    setBusy(false);
    onNotice(`Imported: ${outcomeText(sum(done))}.`);
    void refresh();
  };

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
      <ListPane
        label="Tag rules"
        actions={
          <>
            <button type="button" disabled={busy} onClick={() => fileInput.current?.click()}>
              Import
            </button>
            <button type="button" disabled={busy || exportTargets.length === 0} onClick={() => void exportFile()}>
              Export{selection.multi && ticked.length > 0 ? ` ${exportTargets.length}` : ""}
            </button>
            <input
              ref={fileInput}
              type="file"
              accept=".json,application/json"
              hidden
              onChange={(e) => {
                void importFile(e.target.files?.[0]);
                e.target.value = "";
              }}
            />
            <MultiToggle selection={selection} />
          </>
        }
        search={<SearchBox value={text} onChange={setText} placeholder="Filter: type code, tag, view" label="Filter rules" />}
      >
        {rows === null ? <p className="hint">Reading the rules…</p> : null}
        {rows?.length === 0 ? <p className="hint">No rules yet. Rules are learned from the tags on sheets exported with the Sheets or Blueprint button.</p> : null}
        {rows && rows.length > 0 && shown.length === 0 ? <p className="hint">No rule matches.</p> : null}
        <RowList>
          {shown.map((r) => (
            <Row key={r.id} on={isOn(r.id)} onClick={() => (selection.multi ? selection.toggle(r.id) : void open(r.id))}>
              <b>{ruleLabel(r)}</b>
              <span>
                {r.key.viewport_type} · {r.tag_kind} · {scopeText(r.key, names)} · {r.sample_count} sample{r.sample_count === 1 ? "" : "s"}
              </span>
            </Row>
          ))}
        </RowList>
      </ListPane>
      <main>
        {selection.multi ? (
          <MultiPanel noun="rule" shown={shown.length} ticked={ticked.length} busy={busy} onAll={() => selection.set(shown.map((r) => r.id))} onNone={() => selection.set([])}>
            <button type="button" className="danger" disabled={busy || ticked.length === 0} onClick={() => void removeTicked()}>
              Delete {ticked.length}
            </button>
          </MultiPanel>
        ) : rule ? (
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
              <dt>Scope</dt>
              <dd>{scopeText(rule.key, names)}</dd>
              <dt>Tag</dt>
              <dd>
                {rule.key.tag_category} · {rule.key.tag_family} · {rule.key.tag_type} ({rule.tag_kind})
              </dd>
              <dt>Head</dt>
              <dd>
                {mm(rule.placement.head.x)} mm along, {mm(rule.placement.head.y)} mm left (paper)
              </dd>
              {rule.placement.boundary ? (
                <>
                  <dt>Against the room</dt>
                  <dd>{boundaryText(rule.placement.boundary)}</dd>
                </>
              ) : null}
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
