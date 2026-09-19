// A check, rendered: its options, its findings table, and the CSV button.
//
// The options are the report's own, never project settings, and each one says
// what the check refuses to call a finding — "include pending references",
// "also list rooms with no space". A check with no options either buries its
// signal in the expected or reports a legitimate state as a fault.

import { useEffect, useMemo, useState } from "react";

import { apiGet } from "./common.js";
import { CHECKS, visibleRows, type CheckDef } from "./checkRows.js";
import { buildCsv, downloadCsv } from "./csv.js";
import type { ValidationResponse } from "./types.js";

interface Props {
  projectId: string;
  checkId: string;
}

export function CheckReport({ projectId, checkId }: Props) {
  const check = CHECKS.find((c) => c.id === checkId) as CheckDef;
  const [data, setData] = useState<ValidationResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [options, setOptions] = useState<Record<string, boolean>>({});

  // Options are per check, so switching checks resets them to that check's
  // defaults rather than carrying the last one's answers across.
  useEffect(() => {
    setOptions(Object.fromEntries(check.options.map((o) => [o.id, o.on])));
  }, [check]);

  useEffect(() => {
    let live = true;
    setData(null);
    setError(null);
    apiGet<ValidationResponse>(`/projects/${encodeURIComponent(projectId)}/validation`)
      .then((d) => live && setData(d))
      .catch((e: unknown) => live && setError(String((e as Error).message ?? e)));
    return () => {
      live = false;
    };
  }, [projectId]);

  const rows = useMemo(() => (data ? visibleRows(check, data, options) : []), [check, data, options]);
  const findings = rows.filter((r) => r.severity === "finding").length;

  return (
    <>
      <section className="group">
        <h2>What counts as a finding</h2>
        {check.options.map((o) => (
          <label className="check" key={o.id}>
            <input
              type="checkbox"
              checked={options[o.id] ?? false}
              onChange={(e) => setOptions({ ...options, [o.id]: e.target.checked })}
            />
            {o.label}
          </label>
        ))}
        <p className="foot">{check.foot}</p>
      </section>

      <section className="group">
        <div className="preview-head">
          <h2>Findings</h2>
          <span className="count">
            {data ? `${findings} finding${findings === 1 ? "" : "s"} · ${rows.length} row${rows.length === 1 ? "" : "s"}` : ""}
          </span>
          <button
            className="action"
            type="button"
            disabled={!rows.length}
            onClick={() => downloadCsv(`${check.id}-${projectId}.csv`, buildCsv(check.columns, rows))}
          >
            Export CSV
          </button>
        </div>

        {error && <p className="msg err">{error}</p>}
        {!data && !error && <p className="foot">Reading /validation…</p>}
        {data && !rows.length && <p className="foot">Nothing to report under these options.</p>}

        {rows.length > 0 && (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  {check.columns.map((c) => (
                    <th key={c}>{c}</th>
                  ))}
                  <th>Severity</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row, i) => (
                  <tr key={i} className={row.severity === "finding" ? "" : "quiet"}>
                    {check.columns.map((c) => (
                      <td key={c}>{row.cells[c] ?? ""}</td>
                    ))}
                    <td>
                      <span className={`pill ${row.severity}`}>{row.severity}</span>
                    </td>
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
