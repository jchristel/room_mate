// The ceiling and floor panel.
//
// **One panel for both**, where doors and windows got two. The difference is
// real rather than a judgement call: a door and a window are the same record
// carrying different MEANINGS — an absent side means external for one and is
// normal for the other — while a ceiling and a floor are the same record
// meaning the same things. The server serves both from one `Surface` type, and
// the only field that reads differently is the height offset, which is to a
// ceiling's underside and a floor's top. So the panel says which.
//
// **Every room the surface covers, largest first**, with both fractions: one
// says how much of the SURFACE this room takes, the other how much of the ROOM
// this surface covers — "is this room fully ceiled", which a finishes take-off
// wants. Neither is a filter: which overlaps matter is the reader's policy, and
// the server deliberately stopped drawing that line.

import { applyFilters, keepChosen, propertyRows } from "../../properties.js";
import { findElement, typePropertiesOf } from "../layers.js";
import { nameItems } from "../PropertyChooser.js";
import { useViewer } from "../useViewer.js";
import { Filters } from "./Filters.js";
import { Head, Note, Section } from "./parts.js";
import type { Selection } from "../store.js";
import type { Ceiling, Room } from "../../../renderer/types.js";

/** What a surface's `rooms` entry carries. Not in the renderer's types: the
 *  plan draws a ring and never reads the attribution. */
interface SurfaceRoom {
  room_id: string;
  model_id: string;
  overlap_area: number;
  fraction_of_element: number;
  fraction_of_room: number;
  mean_width: number;
}

export function SurfaceInspector({ selection }: { selection: Selection }) {
  const { payload, inspector, hiddenProperties } = useViewer();
  const entity = selection.kind === "ceiling" ? "ceilings" : "floors";
  const found = findElement(entity, selection.id);
  if (!found) {
    return <Note>That {selection.kind} is not in the current scope.</Note>;
  }

  const surface = found.element as Ceiling & {
    model_id?: string;
    rooms?: SurfaceRoom[];
    properties?: Room["properties"];
  };
  // A surface's `level_id` is its OWN model's, and a `Level.id` is per
  // document — so a model that pushes no rooms, or one whose ids lost the
  // dedup race, has no entry in the rooms' level list. Naming it as the
  // model's own id beats printing a bare number that looks like a name.
  const declared = (payload?.levels ?? []).find((l) => l.id === surface.level_id);
  const levelName = declared ? declared.name : surface.level_id ? `level ${surface.level_id} (this model's id)` : "";
  const rooms = payload?.rooms ?? [];
  const roomName = (id: string) => {
    const r = rooms.find((x) => x.id === id);
    return r ? r.name || r.id : `${id} (not in scope)`;
  };
  const pct = (f: number) => `${(f * 100).toFixed(f >= 0.1 ? 0 : 1)}%`;

  // A ceiling and a floor keep SEPARATE chooser state although one component
  // draws both: they are different records with different properties, and the
  // shared panel is an economy in the code rather than a claim about the
  // question a reader is asking.
  const instanceAll = propertyRows(surface.properties);
  const typeAll = Object.entries(typePropertiesOf(found.payload, surface as { type_properties_ref?: number | null }))
    .map(([k, v]) => [k, v?.value ?? ""] as const)
    .sort((a, b) => a[0].localeCompare(b[0]));
  const hide = hiddenProperties[selection.kind];

  const covered = surface.rooms ?? [];
  const pieces = (surface.polygons ?? []).filter((p) => p.loops?.[0]?.points?.length).length;
  const holes = (surface.polygons ?? []).reduce((n, p) => n + Math.max(0, (p.loops?.length ?? 0) - 1), 0);

  return (
    <>
      <Head
        kind={selection.kind}
        title={surface.type_name || surface.id}
        sub={`${surface.id} · ${levelName}`}
        zoneId={selection.zoneId}
      >
        <Filters scope={selection.kind} items={nameItems(instanceAll, typeAll)} />
      </Head>
      <Section
        title="Placement"
        rows={[
          ["Model", surface.model_id ?? "(unknown)"],
          // The offset is to a CEILING's underside and a FLOOR's top. Naming
          // the edge is the whole reason this row is not just a number: two
          // surfaces stacked in one plan position are told apart by it.
          [
            selection.kind === "ceiling" ? "Height above level (underside)" : "Height above level (top)",
            // Feet, as sent — units are their own exercise — but rounded for
            // reading: the wire carries a float conversion, and 9.84251968
            // says nothing 9.84 does not.
            surface.height_offset == null ? "(not carried)" : `${round2(surface.height_offset)} ft`,
          ],
        ]}
      />
      {/* EMPTY IS A REPORTED STATE, not a gap: a ceiling over a stairwell, an
          external soffit, or a slab on a level carrying no rooms legitimately
          belongs to nothing — 8 of House A's 30 ceilings, 17 of RHH's 1,833. */}
      {covered.length ? (
        <Section
          title={`Rooms covered (${covered.length})`}
          rows={covered.map(
            (r) =>
              [
                roomName(r.room_id),
                `${pct(r.fraction_of_element)} of it · ${pct(r.fraction_of_room)} of the room`,
              ] as const,
          )}
        />
      ) : (
        <>
          <h4>Rooms covered</h4>
          <Note>
            (unattributed) — it lies over no room on its storey. An external soffit, a slab on a level with no rooms, or
            a surface the exporter could not measure.
          </Note>
        </>
      )}
      <Section
        title="Geometry"
        rows={[
          // A LIST of pieces, unioned — never one piece. RHH's multi-piece
          // ceilings are genuinely disjoint, so the count is worth seeing.
          ["Pieces", pieces ? String(pieces) : "none — the exporter measured no polygon"],
          ["Holes", String(holes)],
        ]}
      />
      <Section title="Instance properties" rows={applyFilters(keepChosen(instanceAll, hide), inspector)} />
      <Section title="Type properties" rows={applyFilters(keepChosen(typeAll, hide), inspector)} />
    </>
  );
}

/** At most two decimals, with no trailing zeros. */
function round2(n: number): string {
  return String(Math.round(n * 100) / 100);
}
