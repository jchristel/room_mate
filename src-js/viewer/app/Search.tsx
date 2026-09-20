// The search box and its field picker.
//
// One query and one set of enabled fields for the whole page: every zone
// highlights the same match set and differs only in which level's matches are
// on screen. The match set itself goes into the store, so the zones apply it
// to what is ALREADY drawn — a search can match thousands of rooms, and a
// keystroke must not re-upload a level.

import { useEffect, useMemo, useState } from "react";

import { availableSearchFields, computeMatches, searchFieldLabel } from "../search.js";
import { detectReferenceSources } from "../properties.js";
import { setSearch } from "./store.js";
import { useViewer } from "./useViewer.js";

export function Search() {
  const { payload, search } = useViewer();
  const [open, setOpen] = useState(false);
  const sources = useMemo(() => detectReferenceSources(payload), [payload]);
  const fields = useMemo(() => availableSearchFields(payload?.rooms ?? [], sources), [payload, sources]);

  // A field the page has not seen before defaults to ON — the same "new field
  // → on" rule the grid's source toggles follow. A field that arrived with a
  // new project and was silently off would make the search quietly narrower
  // than it says it is.
  useEffect(() => {
    if (!fields.length) return;
    setSearch((prev) => {
      const next = new Set(prev.fields);
      for (const f of fields) if (!prev.seen.has(f)) next.add(f);
      return { fields: next, seen: new Set([...prev.seen, ...fields]) };
    });
  }, [fields]);

  // The match set is recomputed here rather than in the store, because it
  // depends on the payload AND the field picker AND the query — and the zones
  // only ever read the answer.
  useEffect(() => {
    const matches = computeMatches(payload?.rooms ?? [], search.query, search.fields, sources);
    setSearch(() => ({ matches, active: search.query.trim().length > 0 }));
  }, [payload, search.query, search.fields, sources]);

  return (
    <div className="search">
      <input
        type="search"
        placeholder="Search rooms…"
        autoComplete="off"
        value={search.query}
        onChange={(e) => setSearch(() => ({ query: e.target.value }))}
      />
      <button
        className="ctl"
        title="Choose which fields to search"
        onClick={(e) => {
          e.stopPropagation();
          setOpen(!open);
        }}
      >
        Fields ▾
      </button>
      <div className={`fields-panel${open ? "" : " hidden"}`} onClick={(e) => e.stopPropagation()}>
        <div className="fields-actions">
          <a onClick={() => setSearch((prev) => ({ fields: new Set(fields), seen: prev.seen }))}>All</a>
          <a onClick={() => setSearch(() => ({ fields: new Set() }))}>None</a>
        </div>
        {fields.map((f) => (
          <label key={f}>
            <input
              type="checkbox"
              checked={search.fields.has(f)}
              onChange={(e) =>
                setSearch((prev) => {
                  const next = new Set(prev.fields);
                  if (e.target.checked) next.add(f);
                  else next.delete(f);
                  return { fields: next };
                })
              }
            />{" "}
            {searchFieldLabel(f, sources)}
          </label>
        ))}
      </div>
    </div>
  );
}
