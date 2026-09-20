// Band 1's QA block: what each reference source could not reconcile, and
// whether the models were filtered to one Revit phase.
//
// **On demand, not part of the room poll.** One report per project change or
// per Refresh — it is a health check over the whole scope, and computing it
// every two seconds would cost the server a full reconciliation for a question
// nobody asked.
//
// Each finding NAMES its room, which makes it a jump target into the grid
// rather than a place to repeat the row's detail.

import { useEffect, useState } from "react";

import { fetchJson } from "./api.js";
import { hasFindings, stripLabel, type SourceReport, type ValidationReport } from "../validation.js";
import { setShowErrors, setValidation } from "./store.js";
import { sourceDisplayName } from "../properties.js";
import { useViewer } from "./useViewer.js";

export function QaBand({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  const { scope, validation } = useViewer();
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void refresh(scope.projectId, setBusy);
  }, [scope.projectId]);

  if (!hasFindings(validation)) return null;
  const sources = Object.entries(validation?.sources ?? {});

  return (
    <div className={`result-band${open ? " open" : ""}`} id="qaBand">
      <button
        className="band-head"
        id="qaHead"
        onClick={() => {
          // Error highlighting FOLLOWS this block's expansion rather than a
          // control of its own: a reader opens QA to look at the flagged
          // rooms, and a plan that lit up on a background refresh would be a
          // change nobody asked for.
          onToggle();
          setShowErrors(!open);
        }}
      >
        {open ? "▾" : "▸"} {stripLabel(validation)}
      </button>
      {open ? (
        <div className="band-body" id="qaBody">
          <button className="refresh" disabled={busy} onClick={() => void refresh(scope.projectId, setBusy)}>
            {busy ? "Checking…" : "Refresh"}
          </button>
          {/* The export lives in /reports/: two renderings of one set of
              findings drift the first time either is fixed, so this band shows
              them next to the plan and builds none of its own. */}
          <a className="refresh" href="/reports/" id="qaReports">
            Export in Reports
          </a>
          <div id="qaContent">
            <p>
              {sources.length
                ? `${validation?.total_rooms ?? 0} room(s) checked against ${sources.length} reference source(s).`
                : "No reference source configured — nothing to reconcile."}
            </p>
            <Phases phases={validation?.phases} />
            {sources.map(([name, report]) => (
              <SourceFindings key={name} name={name} report={report} />
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}

async function refresh(projectId: string | null, setBusy: (b: boolean) => void): Promise<void> {
  if (!projectId) {
    setValidation(null);
    return;
  }
  setBusy(true);
  try {
    setValidation(await fetchJson<ValidationReport>(`/projects/${encodeURIComponent(projectId)}/validation`));
  } catch {
    // A transient failure leaves the last known report alone: a band that
    // emptied itself on a dropped request would read as "all clear".
  } finally {
    setBusy(false);
  }
}

/** Models filtered to different phases. Its own section, not a source's
 *  finding: it owes nothing to reference data, and it invalidates areas,
 *  adjacency and every cross-model count while it holds. */
function Phases({ phases }: { phases: ValidationReport["phases"] }) {
  if (!phases?.disagree) return null;
  return (
    <section className="qa-source qa-phases">
      <h2>
        Phases <span className="qa-source-meta">⚠ models disagree</span>
      </h2>
      <p>
        These models were filtered to different Revit phases, so the plan above merges rooms from more than one phase.
        Areas, adjacency and room counts across models are not comparable while this holds.
      </p>
      <ul>
        {Object.entries(phases.by_model ?? {}).map(([model, phase]) => (
          <li className="mismatch" key={model}>
            {model}: {phase ?? <span className="uncounted">not phased — rooms were never filtered</span>}
          </li>
        ))}
      </ul>
    </section>
  );
}

function SourceFindings({ name, report }: { name: string; report: SourceReport }) {
  const label = sourceDisplayName(name);
  return (
    <section className="qa-source">
      <h2>{label}</h2>
      {/* AHEAD of every list, because when this fires the lists below are all
          noise: every room is missing a link value and every row is unmatched,
          and reading them top to bottom says the data is catastrophically
          broken when the truth is that one property name never resolved. */}
      {report.link_property_absent_everywhere ? (
        <section className="config-error">
          <h3>⚠ Link property &quot;{report.link_property}&quot; does not exist on any room</h3>
          <ul>
            <li>
              Nothing can join: every room below is listed only because this lookup found no property of that name, not
              because its data is missing. Either {label}&apos;s CSV names a property the rooms do not carry, or this
              project needs a <code>[[builtin_properties]]</code> entry mapping <code>{report.link_property}</code> to
              the Revit property that holds it.
            </li>
          </ul>
        </section>
      ) : null}
      <List title="Missing link value" items={report.rooms_missing_link_value} render={(id) => <RoomRef id={id} />} />
      <List
        title="Duplicate link values"
        items={report.duplicate_link_values}
        render={(d) => (
          <>
            {d.value}:{" "}
            {d.room_ids.map((id, i) => (
              <span key={id}>
                {i ? ", " : ""}
                <RoomRef id={id} />
              </span>
            ))}
          </>
        )}
      />
      <List title="Unmatched rooms" items={report.rooms_unmatched} render={(id) => <RoomRef id={id} />} />
      <List
        title="Property mismatches"
        items={report.property_mismatches}
        render={(m) => (
          <>
            <RoomRef id={m.room_id} /> · {m.field}: {m.revit ?? "—"} vs {m.reference ?? "—"}
          </>
        )}
      />
      <List
        title="Fields absent in Revit"
        items={report.fields_absent_in_revit}
        render={(m) => (
          <>
            <RoomRef id={m.room_id} /> · {m.field}
          </>
        )}
      />
      <List
        title="Fields empty in Revit"
        items={report.fields_empty_in_revit}
        render={(m) => (
          <>
            <RoomRef id={m.room_id} /> · {m.field}
          </>
        )}
      />
      <Coverage coverage={report.field_coverage} />
    </section>
  );
}

/** A section that renders NOTHING when empty — an empty heading reads as a
 *  category that was checked and found clean, which it is not. */
function List<T>({ title, items, render }: { title: string; items: readonly T[]; render: (item: T) => React.ReactNode }) {
  if (!items.length) return null;
  return (
    <section>
      <h3>
        {title} ({items.length})
      </h3>
      <ul>
        {items.map((item, i) => (
          <li key={i}>{render(item)}</li>
        ))}
      </ul>
    </section>
  );
}

/** Which of a source's fields are actually checked. A field with no Revit
 *  property mapped is reported as "no" rather than omitted: "not checked" is a
 *  fact about the configuration, and a missing line reads as a pass. */
function Coverage({ coverage }: { coverage: SourceReport["field_coverage"] }) {
  if (!coverage?.length) return null;
  return (
    <section>
      <h3>Field coverage ({coverage.length})</h3>
      <ul>
        {coverage.map((c) => (
          <li className={c.checked ? "ok" : ""} key={c.label}>
            {c.label}: {c.checked ? "yes" : "no"}
            {c.revit_property ? ` (${c.revit_property})` : ""}
          </li>
        ))}
      </ul>
    </section>
  );
}

function RoomRef({ id }: { id: string }) {
  return <span data-room={id}>{id}</span>;
}
