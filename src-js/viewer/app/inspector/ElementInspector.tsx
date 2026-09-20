// The door, window and FF&E panels.
//
// **Three panels, not one, even though a door and a window are the SAME record
// on the wire** — measured, on two documents. What differs is what the values
// MEAN, and saying so is the whole job of a panel:
//
//   - a door's absent side means external, and both absent is a finding;
//   - a window's both-absent is the NORMAL state in a facade model, because
//     Revit cannot resolve a room across a link, so the door's wording would
//     tell a reader that a correct model is broken;
//   - an item has one room rather than two sides, a category no opening
//     carries, and no swing to describe.
//
// The tiers stay SPLIT in every one of them. "This leaf is 820 wide" and "every
// door of this type is 820 wide" are different claims, and a hardware schedule
// joins against the second — flattening them would throw away the distinction
// the contract goes out of its way to keep.

import { applyFilters, propertyRows } from "../../properties.js";
import { findElement, typePropertiesOf } from "../layers.js";
import { useViewer } from "../useViewer.js";
import { Head, Note, Section } from "./parts.js";
import type { Door, Item, Room, WindowOpening } from "../../../renderer/types.js";
import type { Selection } from "../store.js";

/** A room's name for a reference held by an element. A reference the current
 *  scope does not contain is NAMED as such rather than silently blanked: the
 *  id is what a reader takes back to Revit. */
function roomNamer(rooms: readonly Room[]) {
  return (id: string | null | undefined): string => {
    if (!id) return "";
    const r = rooms.find((x) => x.id === id);
    return r ? r.name || r.id : `${id} (not in scope)`;
  };
}

export function ElementInspector({ selection }: { selection: Selection }) {
  const { payload, inspector } = useViewer();
  const entity = selection.kind === "door" ? "doors" : selection.kind === "window" ? "windows" : "ffe";
  const found = findElement(entity, selection.id);
  if (!found) return <Note>{label(selection.kind)} {selection.id} is not in the current scope.</Note>;

  const element = found.element as Door & WindowOpening & Item;
  const rooms = payload?.rooms ?? [];
  const name = roomNamer(rooms);
  const levelName =
    element.level_id === "-1"
      ? // Revit's "no element" id, which an unhosted item legitimately carries —
        // 71 of 644 when measured. Named, because "-1" reads as a bug.
        "unhosted (no level)"
      : (payload?.levels ?? []).find((l) => l.id === element.level_id)?.name ?? element.level_id ?? "";

  const instance = applyFilters(propertyRows(element.properties), inspector);
  const type = applyFilters(
    Object.entries(typePropertiesOf(found.payload, element))
      .map(([k, v]) => [k, v?.value ?? ""] as const)
      .sort((a, b) => a[0].localeCompare(b[0])),
    inspector,
  );
  const hasBox = !!element.loops?.[0]?.points?.length;

  return (
    <>
      <Head
        kind={selection.kind}
        title={element.type_name || element.id}
        sub={`${element.id} · ${levelName}`}
        zoneId={selection.zoneId}
      />
      {selection.kind === "item" ? (
        <Section
          title="Placement"
          rows={[
            ["Category", element.category || "(unknown)"],
            // One room, so one line. An opening's panel has two sides and has
            // to say which is which; there is nothing here to disambiguate.
            ["In room", element.room ? name(element.room) : "(none)"],
            [
              "Attributed to",
              element.owner_rooms?.length
                ? element.owner_rooms.map(name).join(", ")
                : "(homeless — outside every room, which is often correct)",
            ],
          ]}
        />
      ) : (
        <Section
          title="Rooms"
          rows={[
            [selection.kind === "door" ? "Opens from" : "Faces from", element.from_room ? name(element.from_room) : externalLabel(selection.kind)],
            [selection.kind === "door" ? "Opens into" : "Faces into", element.to_room ? name(element.to_room) : externalLabel(selection.kind)],
            ["Attributed to", attribution(element, name, selection.kind)],
          ]}
        />
      )}
      <Section title="Geometry" rows={geometryRows(element, selection.kind, hasBox)} />
      {element.is_component || element.super_component_id ? (
        <Section
          title="Nesting"
          rows={[
            ["Component of", element.super_component_id || "(another instance)"],
            ["Shown because", "this project sets [ffe] nested_components = include"],
          ]}
        />
      ) : null}
      <Section title="Instance properties" rows={instance} />
      <Section title="Type properties" rows={type} />
    </>
  );
}

function label(kind: string): string {
  return kind === "item" ? "Item" : kind.charAt(0).toUpperCase() + kind.slice(1);
}

function externalLabel(kind: string): string {
  return kind === "door" ? "(none — external)" : "(none)";
}

/** Who the element is attributed to, in the words its entity earns.
 *
 *  A window with NEITHER side is the normal facade-model state, not a finding,
 *  so it says so — the door wording would report a correct model as broken. */
function attribution(
  element: Door & WindowOpening,
  name: (id: string | null | undefined) => string,
  kind: string,
): string {
  if (element.owner_rooms?.length) return element.owner_rooms.map(name).join(", ");
  if (kind === "window" && !element.from_room && !element.to_room) {
    return "(no room reference — normal in a facade model)";
  }
  return "(homeless — no room attributed)";
}

/** What the plan is drawing, named explicitly — an element shown as a bare
 *  marker, or missing entirely, otherwise looks like a rendering fault when it
 *  is the honest report of what the model carried. */
function geometryRows(
  element: Door & WindowOpening & Item,
  kind: string,
  hasBox: boolean,
): (readonly [string, string])[] {
  if (kind === "item") {
    return [
      ["Footprint", hasBox ? "yes" : "none — drawn as a marker at its insertion point"],
      ["Facing", element.facing ? "yes" : "unknown — no orientation drawn"],
    ];
  }
  return [
    ["Footprint", hasBox ? "yes" : "none — drawn from its insertion point"],
    [
      "Direction",
      element.through_wall_normal ? "yes" : kind === "door" ? "unknown — no arrow drawn" : "unknown — no symbol drawn",
    ],
  ];
}
