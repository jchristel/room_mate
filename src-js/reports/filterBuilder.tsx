// The filter builder: groups of conditions, and the sentence they read as.
//
// Two things here are rules rather than styling, and both come from
// docs/STRATEGY-REPORTS.md:
//
// 1. **A negative condition on the associated side is offered only inside an
//    `all` group.** It asks about the room's whole set — "no ceiling of this
//    type" — which has no single reading inside an `any` group, and picking one
//    silently is worse than not offering it.
// 2. **A negative condition on the ROOM side warns about blanks.** A room with
//    no Department fails "Department is not Living", because a missing value is
//    not evidence of a different one. The action beside the warning rewrites the
//    condition as `any(is not Living, is blank)` rather than changing the rule.

import { useState } from "react";

import {
  addTo,
  complete,
  condition,
  describe,
  group,
  isNegative,
  keepBlanksToo,
  NEGATIVE,
  NO_VALUE,
  OPS,
  opLabel,
  replaceNode,
  setMode,
  type Condition,
  type FieldType,
  type FilterNode,
  type Group,
  type Side,
} from "./filter.js";

export interface FieldOption {
  side: Side;
  name: string;
  type: FieldType;
  /** Heading in the picker: "Room", "Ceiling", "Join". */
  sideLabel: string;
}

interface Props {
  root: Group;
  fields: FieldOption[];
  entityOne: string;
  onChange: (next: Group) => void;
}

export function FilterBuilder({ root, fields, entityOne, onChange }: Props) {
  return (
    <section className="group">
      <h2>Filter</h2>
      <GroupEditor node={root} depth={0} parent={null} root={root} fields={fields} entityOne={entityOne} onChange={onChange} />
      <div className="fsummary">
        <span className="k">Reads as</span>
        <code>{describe(root, entityOne)}</code>
      </div>
    </section>
  );
}

function GroupEditor({
  node,
  depth,
  parent,
  root,
  fields,
  entityOne,
  onChange,
}: {
  node: Group;
  depth: number;
  parent: Group | null;
  root: Group;
  fields: FieldOption[];
  entityOne: string;
  onChange: (next: Group) => void;
}) {
  const first = fields[0] as FieldOption;
  return (
    <div className="fgroup">
      <div className="fgroup-head">
        {depth === 0 ? "Include rows that match" : "Match"}
        <select
          aria-label="Match all or any"
          value={node.mode}
          onChange={(e) => onChange(setMode(root, node.id, e.target.value as "all" | "any"))}
        >
          <option value="all">all</option>
          <option value="any">any</option>
        </select>
        of these conditions
        <span className="spacer" />
        {parent && (
          <button
            className="icon"
            type="button"
            aria-label="Remove group"
            onClick={() => onChange(replaceNode(root, node.id, null))}
          >
            ×
          </button>
        )}
      </div>

      {node.items.length === 0 && <p className="foot fadd">No conditions yet, so every row is included.</p>}

      {node.items.map((item, i) => {
        const connector = i === 0 ? "where" : node.mode === "all" ? "and" : "or";
        return item.kind === "group" ? (
          <div className="nest" key={item.id}>
            <span className={`conn${i ? " op" : ""}`}>{connector}</span>
            <GroupEditor
              node={item}
              depth={depth + 1}
              parent={node}
              root={root}
              fields={fields}
              entityOne={entityOne}
              onChange={onChange}
            />
          </div>
        ) : (
          <ConditionEditor
            key={item.id}
            node={item}
            connector={connector}
            isOp={i > 0}
            parentMode={node.mode}
            root={root}
            fields={fields}
            entityOne={entityOne}
            onChange={onChange}
          />
        );
      })}

      <div className="fadd">
        <button
          type="button"
          onClick={() => onChange(addTo(root, node.id, condition(first.side, first.name, first.type)))}
        >
          + condition
        </button>
        {depth < 2 && (
          <button
            type="button"
            onClick={() =>
              onChange(
                addTo(
                  root,
                  node.id,
                  group(node.mode === "all" ? "any" : "all", [condition(first.side, first.name, first.type)]),
                ),
              )
            }
          >
            + group ({node.mode === "all" ? "any" : "all"} of)
          </button>
        )}
      </div>
    </div>
  );
}

function ConditionEditor({
  node,
  connector,
  isOp,
  parentMode,
  root,
  fields,
  entityOne,
  onChange,
}: {
  node: Condition;
  connector: string;
  isOp: boolean;
  parentMode: "all" | "any";
  root: Group;
  fields: FieldOption[];
  entityOne: string;
  onChange: (next: Group) => void;
}) {
  const update = (patch: Partial<Condition>) => onChange(replaceNode(root, node.id, { ...node, ...patch }));
  const sides = [...new Set(fields.map((f) => f.sideLabel))];
  const assoc = node.side !== "room";
  // See the rule at the top of this file.
  const negativeBlocked = assoc && parentMode === "any";

  return (
    <div className={`cond${complete(node) ? "" : " incomplete"}`}>
      <span className={`conn${isOp ? " op" : ""}`}>{connector}</span>
      <div className="cond-body">
        <div className="fieldcell">
          <select
            aria-label="Field"
            value={`${node.side}:${node.field}`}
            onChange={(e) => {
              const picked = fields.find((f) => `${f.side}:${f.name}` === e.target.value) as FieldOption;
              update({
                side: picked.side,
                field: picked.name,
                type: picked.type,
                // A type change invalidates the operator and the value, so both
                // reset rather than carrying a comparison that cannot run.
                op: picked.type === node.type ? node.op : "eq",
                value: picked.type === node.type ? node.value : "",
                value2: "",
              });
            }}
          >
            {sides.map((sideLabel) => (
              <optgroup label={sideLabel} key={sideLabel}>
                {fields
                  .filter((f) => f.sideLabel === sideLabel)
                  .map((f) => (
                    <option key={`${f.side}:${f.name}`} value={`${f.side}:${f.name}`}>
                      {f.name}
                    </option>
                  ))}
              </optgroup>
            ))}
          </select>
          <span className="ftype" title={node.type === "number" ? "Number" : "Text"}>
            {node.type === "number" ? "123" : "abc"}
          </span>
        </div>

        <select aria-label="Operator" value={node.op} onChange={(e) => update({ op: e.target.value })}>
          {OPS[node.type]
            .filter(([v]) => !(negativeBlocked && NEGATIVE.includes(v) && v !== node.op))
            .map(([v, label]) => (
              <option key={v} value={v}>
                {label}
              </option>
            ))}
        </select>

        <div className="valcell">
          {NO_VALUE.includes(node.op) ? (
            <span className="na">no value needed</span>
          ) : node.op === "between" ? (
            <>
              <input type="text" value={node.value} aria-label="From" onChange={(e) => update({ value: e.target.value })} />
              <span className="and">and</span>
              <input type="text" value={node.value2} aria-label="To" onChange={(e) => update({ value2: e.target.value })} />
            </>
          ) : (
            <>
              <input type="text" value={node.value} aria-label="Value" onChange={(e) => update({ value: e.target.value })} />
              {node.type === "text" && (
                <button
                  className={`icon case${node.caseSensitive ? " on" : ""}`}
                  type="button"
                  aria-pressed={node.caseSensitive}
                  title={node.caseSensitive ? "Match case: on" : "Match case: off (Aa = aa)"}
                  onClick={() => update({ caseSensitive: !node.caseSensitive })}
                >
                  Aa
                </button>
              )}
            </>
          )}
        </div>

        <button className="icon" type="button" aria-label="Remove condition" onClick={() => onChange(replaceNode(root, node.id, null))}>
          ×
        </button>

        {isNegative(node) && !assoc && (
          <div className="warn">
            Rooms where {node.field} is blank do not match this either.{" "}
            <button type="button" onClick={() => onChange(keepBlanksToo(root, node))}>
              Keep blanks too
            </button>
          </div>
        )}
        {isNegative(node) && assoc && (
          <div className="warn">
            Asks for rooms that have no matching {entityOne}, so a room with none is a match.
            {parentMode === "any" && " Move it into an “all” group: inside “any” it has no single reading."}
          </div>
        )}
      </div>
    </div>
  );
}
