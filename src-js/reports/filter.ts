// The filter tree the builder edits, and the sentence it reads back as.
//
// Pure, and apart from the component, because the meanings live here: which
// operators a field type offers, what a negative condition on the associated
// side asks, and how the whole tree reads in words. The server decides the same
// things in `service::reports`; this file is what makes them visible before the
// request is sent.

export type Side = "room" | "element" | "join";
export type Mode = "all" | "any";
export type FieldType = "text" | "number";

export interface Condition {
  kind: "condition";
  id: number;
  side: Side;
  field: string;
  type: FieldType;
  op: string;
  value: string;
  value2: string;
  /** Compare text exactly. Off by default — see `Predicate::case_sensitive`. */
  caseSensitive: boolean;
}

export interface Group {
  kind: "group";
  id: number;
  mode: Mode;
  items: FilterNode[];
}

export type FilterNode = Group | Condition;

/** Operators per field type, in the order a form should offer them. */
export const OPS: Record<FieldType, [string, string][]> = {
  text: [
    ["eq", "is"],
    ["ne", "is not"],
    ["contains", "contains"],
    ["not_contains", "does not contain"],
    ["starts_with", "starts with"],
    ["ends_with", "ends with"],
    ["blank", "is blank"],
    ["has_value", "has a value"],
  ],
  number: [
    ["eq", "="],
    ["ne", "≠"],
    ["lt", "<"],
    ["le", "≤"],
    ["gt", ">"],
    ["ge", "≥"],
    ["between", "between"],
    ["blank", "is blank"],
    ["has_value", "has a value"],
  ],
};

/** The two operators that take no value, because a blank could not match one. */
export const NO_VALUE = ["blank", "has_value"];

/**
 * The operators that ask about ABSENCE on the associated side.
 *
 * On the room side these are ordinary negations. On the associated side they
 * quantify over the room's whole set — "no ceiling of this type" — which is a
 * different question from "a ceiling that is not of this type" and is why the
 * builder keeps them out of `any` groups, where the reading is ambiguous.
 */
export const NEGATIVE = ["ne", "not_contains"];

export const isNegative = (c: Condition) => NEGATIVE.includes(c.op);
export const opLabel = (type: FieldType, op: string) => OPS[type].find(([v]) => v === op)?.[1] ?? op;

let nextId = 0;
export const newId = () => (nextId += 1);

export const condition = (side: Side, field: string, type: FieldType = "text"): Condition => ({
  kind: "condition",
  id: newId(),
  side,
  field,
  type,
  op: type === "number" ? "eq" : "eq",
  value: "",
  value2: "",
  caseSensitive: false,
});

export const group = (mode: Mode, items: FilterNode[] = []): Group => ({ kind: "group", id: newId(), mode, items });

/** A condition with no value yet is left out rather than failing the filter. */
export function complete(node: FilterNode): boolean {
  if (node.kind === "group") return node.items.some(complete);
  if (NO_VALUE.includes(node.op)) return true;
  return node.value.trim() !== "" && (node.op !== "between" || node.value2.trim() !== "");
}

/** The tree as the server reads it; incomplete conditions are dropped. */
export function toWire(node: FilterNode): unknown {
  if (node.kind === "group") {
    return { mode: node.mode, items: node.items.filter(complete).map(toWire) };
  }
  return {
    side: node.side,
    field: node.field,
    op: node.op,
    value: NO_VALUE.includes(node.op) ? undefined : node.value.trim(),
    value2: node.op === "between" ? node.value2.trim() : undefined,
    case_sensitive: node.caseSensitive,
  };
}

/**
 * The filter in words.
 *
 * **An associated-side condition reads set-wise** — "the room has no ceiling
 * whose Type is X" — because that is what it means, and a reader checking their
 * own logic against a nested form has nothing else to check it with.
 */
export function describe(node: FilterNode, entityOne: string, top = true): string {
  if (node.kind === "group") {
    const live = node.items.filter(complete);
    if (!live.length) return top ? "every row" : "";
    const joined = live.map((item) => describe(item, entityOne, false)).join(node.mode === "all" ? " AND " : " OR ");
    return top || live.length === 1 ? joined : `(${joined})`;
  }

  const value = NO_VALUE.includes(node.op)
    ? ""
    : node.op === "between"
      ? ` ${node.value} and ${node.value2}`
      : node.type === "number"
        ? ` ${node.value}`
        : ` "${node.value}"${node.caseSensitive ? " (match case)" : ""}`;

  if (node.side === "room") return `Room.${node.field} ${opLabel(node.type, node.op)}${value}`;

  const positive = isNegative(node) ? (node.op === "ne" ? "eq" : "contains") : node.op;
  const label = opLabel(node.type, positive);
  const noun = node.side === "join" ? `${entityOne} whose ${node.field}` : `${entityOne} whose ${node.field}`;
  return `the room has ${isNegative(node) ? "no" : "a"} ${noun} ${label}${value}`;
}

// ---------- tree edits, all immutable so React sees a new object ----------

export function replaceNode(root: Group, id: number, next: FilterNode | null): Group {
  const walk = (node: Group): Group => ({
    ...node,
    items: node.items
      .map((item) => (item.id === id ? next : item.kind === "group" ? walk(item) : item))
      .filter((item): item is FilterNode => item !== null),
  });
  return walk(root);
}

export function addTo(root: Group, parentId: number, child: FilterNode): Group {
  const walk = (node: Group): Group => ({
    ...node,
    items: node.id === parentId ? [...node.items, child] : node.items.map((i) => (i.kind === "group" ? walk(i) : i)),
  });
  return walk(root);
}

export function setMode(root: Group, id: number, mode: Mode): Group {
  const walk = (node: Group): Group => ({
    ...node,
    mode: node.id === id ? mode : node.mode,
    items: node.items.map((i) => (i.kind === "group" ? walk(i) : i)),
  });
  return walk(root);
}

/** Wrap a condition in `any(it, is blank)` — the "keep blanks too" action. */
export function keepBlanksToo(root: Group, target: Condition): Group {
  const blank: Condition = { ...condition(target.side, target.field, target.type), op: "blank" };
  return replaceNode(root, target.id, group("any", [target, blank]));
}
