// The right pane: what is selected, and its dimension constraints.
//
// One item selected shows its recipe and lets a person change it; several selected offer the bulk
// actions (remove, add back, and importing the placement settings from another revision, so a
// whole run can be brought back in one step). Every change goes to the DRAFT, which becomes a new
// revision when saved: nothing here edits a revision that exists.
//
// "Remove from blueprint" marks the item excluded and keeps it, so it can be added back and an
// earlier revision's settings can be imported onto it.

import { useState } from "react";

import {
  alongUnit,
  alongValue,
  followersOf,
  importPlacements,
  moveToEdge,
  offUnit,
  offValue,
  setAlongMode,
  setAlongValue,
  setExcluded,
  setFacing,
  setOffMode,
  setOffValue,
} from "./draft.js";
import type { Pt } from "./geometry.js";
import type { AlongMode, BlueprintItem, OffMode, Revision } from "./types.js";

export interface InspectorProps {
  items: readonly BlueprintItem[];
  ring: readonly Pt[];
  selected: ReadonlySet<string>;
  revisions: readonly Revision[];
  viewing: number;
  readOnly: boolean;
  onItems(next: BlueprintItem[], message?: string): void;
}

const ALONG_MODES: { value: AlongMode; label: string }[] = [
  { value: "fixed_from_start", label: "fixed, from the start" },
  { value: "fixed_from_end", label: "fixed, from the end" },
  { value: "percent", label: "percentage of the edge" },
];
const OFF_MODES: { value: OffMode; label: string }[] = [
  { value: "fixed", label: "fixed distance from the wall" },
  { value: "percent", label: "percentage of the room's depth" },
];

const mm = (n: number): string => `${Math.round(n)} mm`;

export function Inspector(props: InspectorProps) {
  const { items, selected, readOnly } = props;
  const chosen = items.filter((i) => selected.has(i.id));
  if (chosen.length === 0) {
    return (
      <aside className="inspector">
        <p className="hint">
          Select an item on the plan to see how it is placed. Shift-click adds to the selection. Edge numbers are on the plan: a recipe
          refers to them.
        </p>
      </aside>
    );
  }
  return (
    <aside className="inspector">
      {chosen.length === 1 ? <One {...props} item={chosen[0] as BlueprintItem} /> : <Many {...props} chosen={chosen} />}
      {readOnly ? <p className="hint">A pending revision cannot be edited. Accept it, or reject it, on the timeline.</p> : null}
    </aside>
  );
}

function One({ item, items, ring, revisions, viewing, readOnly, onItems }: InspectorProps & { item: BlueprintItem }) {
  const p = item.placement;
  const replace = (next: BlueprintItem, message?: string) => onItems(items.map((i) => (i.id === next.id ? next : i)), message);
  const followers = followersOf(items, item.id);
  const others = revisions.filter((r) => r.number !== viewing);
  const [from, setFrom] = useState<number | "">("");
  const num = (text: string): number | null => (text.trim() === "" || !Number.isFinite(Number(text)) ? null : Number(text));

  return (
    <>
      <h3>{item.type_name}</h3>
      <p className="sub">
        {item.family_name ? `${item.family_name} · ` : ""}
        {item.category} · {item.class === "wall_device" ? "wall device" : "free-standing"} · {mm(item.size_mm[0])} × {mm(item.size_mm[1])}
      </p>
      {item.excluded ? <p className="note">Removed from this blueprint. It is kept, and can be added back.</p> : null}
      {p.corner_anchored ? (
        <p className="note">No edge lay beside this item when it was captured, so it sits against the nearest one and its distance along runs past the end.</p>
      ) : null}

      <fieldset disabled={readOnly}>
        <legend>Placement</legend>
        <label>
          Edge
          <select value={p.edge} onChange={(e) => replace(moveToEdge(item, ring, Number(e.target.value)))}>
            {ring.map((_, i) => (
              <option key={i} value={i}>
                {i}
              </option>
            ))}
          </select>
        </label>

        <label>
          Along the edge
          <select value={p.along_mode} onChange={(e) => replace(setAlongMode(item, e.target.value as AlongMode))}>
            {ALONG_MODES.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </label>
        <label className="num">
          <input
            type="number"
            step="any"
            value={Math.round(alongValue(p) * 10) / 10}
            onChange={(e) => {
              const v = num(e.target.value);
              if (v !== null) replace(setAlongValue(item, ring, v));
            }}
          />
          <span>{alongUnit(p.along_mode)}</span>
        </label>

        <label>
          Into the room
          <select value={p.off_mode} onChange={(e) => replace(setOffMode(item, e.target.value as OffMode))}>
            {OFF_MODES.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </label>
        <label className="num">
          <input
            type="number"
            step="any"
            value={Math.round(offValue(p) * 10) / 10}
            onChange={(e) => {
              const v = num(e.target.value);
              if (v !== null) replace(setOffValue(item, v));
            }}
          />
          <span>{offUnit(p.off_mode)}</span>
        </label>

        <label>
          Facing, from the edge's inward normal
          <span className="num">
            <input
              type="number"
              step="any"
              value={Math.round(p.facing_deg * 10) / 10}
              onChange={(e) => {
                const v = num(e.target.value);
                if (v !== null) replace(setFacing(item, v));
              }}
            />
            <span>°</span>
          </span>
        </label>
        <p className="hint">
          Edge {p.edge} is {mm(p.edge_len_mm)} long; the room is {mm(p.depth_mm)} deep in front of it here. Changing the mode moves
          nothing in this room: it decides what the item does when the room is another size.
        </p>
      </fieldset>

      {item.follows ? (
        <p className="note">
          {item.follows.inside ? "Sits inside" : "Abuts"} <code>{item.follows.item}</code>, so it moves with it in another room.
        </p>
      ) : null}
      {followers.length > 0 ? <p className="note">{followers.length} item(s) follow this one.</p> : null}

      <div className="actions">
        <button type="button" disabled={readOnly} onClick={() => onItems(setExcluded(items, new Set([item.id]), !item.excluded))}>
          {item.excluded ? "Add back to blueprint" : "Remove from blueprint"}
        </button>
      </div>

      {others.length > 0 ? (
        <fieldset disabled={readOnly}>
          <legend>Import from another revision</legend>
          <select value={from} onChange={(e) => setFrom(e.target.value === "" ? "" : Number(e.target.value))}>
            <option value="">Choose a revision</option>
            {others.map((r) => (
              <option key={r.number} value={r.number}>
                r{r.number}
              </option>
            ))}
          </select>
          <button
            type="button"
            disabled={from === ""}
            onClick={() => {
              const src = revisions.find((r) => r.number === from);
              if (!src) return;
              const r = importPlacements(items, src.items, new Set([item.id]), ring.length);
              onItems(r.items, r.changed ? `Took this item's placement from r${src.number}.` : `r${src.number} places it the same way, or does not hold it.`);
            }}
          >
            Import its placement
          </button>
        </fieldset>
      ) : null}

      <details>
        <summary>
          Properties ({item.properties.length} instance, {item.type_properties.length} type)
        </summary>
        <table className="props">
          <tbody>
            {[...item.properties.map((x) => ({ ...x, tier: "instance" })), ...item.type_properties.map((x) => ({ ...x, tier: "type" }))].map((x) => (
              <tr key={`${x.tier}:${x.name}`}>
                <th title={x.tier}>{x.name}</th>
                <td>{x.value}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </>
  );
}

function Many({ chosen, items, ring, revisions, viewing, readOnly, onItems }: InspectorProps & { chosen: BlueprintItem[] }) {
  const ids = new Set(chosen.map((i) => i.id));
  const others = revisions.filter((r) => r.number !== viewing);
  const [from, setFrom] = useState<number | "">("");
  const excluded = chosen.filter((i) => i.excluded).length;
  return (
    <>
      <h3>{chosen.length} items selected</h3>
      <p className="sub">{excluded} of them removed from the blueprint</p>
      <div className="actions">
        <button type="button" disabled={readOnly} onClick={() => onItems(setExcluded(items, ids, true))}>
          Remove from blueprint
        </button>
        <button type="button" disabled={readOnly} onClick={() => onItems(setExcluded(items, ids, false))}>
          Add back
        </button>
      </div>
      {others.length > 0 ? (
        <fieldset disabled={readOnly}>
          <legend>Import placements from another revision</legend>
          <select value={from} onChange={(e) => setFrom(e.target.value === "" ? "" : Number(e.target.value))}>
            <option value="">Choose a revision</option>
            {others.map((r) => (
              <option key={r.number} value={r.number}>
                r{r.number}
              </option>
            ))}
          </select>
          <button
            type="button"
            disabled={from === ""}
            onClick={() => {
              const src = revisions.find((r) => r.number === from);
              if (!src) return;
              const r = importPlacements(items, src.items, ids, ring.length);
              onItems(r.items, `Took the placement of ${r.changed} item(s) from r${src.number}.`);
            }}
          >
            Import for these {chosen.length}
          </button>
        </fieldset>
      ) : null}
    </>
  );
}
