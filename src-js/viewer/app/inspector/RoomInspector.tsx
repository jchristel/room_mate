// The room panel: identity, classification, what is in the room, the model
// properties and every joined reference source.
//
// The order is deliberate. "What is in here" sits ABOVE the 45 model property
// rows because it is a first-order question about the room, and those rows are
// what a reader scrolls past to reach anything else.

import { applyFilters, detectReferenceSources, propertyRows, referenceRows, sourceDisplayName } from "../../properties.js";
import { useViewer } from "../useViewer.js";
import { Filters } from "./Filters.js";
import { Head, Note, Section } from "./parts.js";
import { RoomContents } from "./RoomContents.js";
import type { Selection } from "../store.js";

export function RoomInspector({ selection }: { selection: Selection }) {
  const { payload, inspector, zones } = useViewer();
  const room = payload?.rooms?.find((r) => r.id === selection.id);
  if (!room) return <Note>Room {selection.id} is not in the current scope.</Note>;

  const levels = payload?.levels ?? [];
  const levelName = levels.find((l) => l.id === room.level_id)?.name ?? room.level_id ?? "";
  // The element payloads hold only the storeys on screen, so a room reached
  // from somewhere other than a zone (the grid, a search) may have no contents
  // fetched at all. That is a different answer from "none".
  const storeyShown = zones.some((z) => z.levelId === room.level_id);

  // An `undefined` tier is NAMED rather than shown as an empty value: it means
  // "the classifier ran and this room fell in the undefined bucket", which is a
  // different statement from "no data". Exempt from hide-empty for that reason.
  const classification = (room.classification ?? []).map(
    (t) => [t.tier, t.undefined ? "(undefined)" : t.name || t.code || ""] as const,
  );
  const all = propertyRows(room.properties);
  const shown = applyFilters(all, inspector);
  const sources = detectReferenceSources(payload);

  return (
    <>
      <Head kind="room" title={room.name || room.id} sub={`${room.id} · ${levelName}`} zoneId={selection.zoneId}>
        <Filters />
      </Head>
      <Section title="Classification" rows={applyFilters(classification, inspector, { hideEmpty: false })} />
      {/* Deliberately NOT run through the property filters: they exist for
          property NAMES, and letting them cut element rows would make a room
          look empty because someone was searching for a property. */}
      <RoomContents room={room} storeyShown={storeyShown} />
      <Section title="Model" rows={shown} source="model" />
      {sources.map((name) => {
        const rows = referenceRows(room, name);
        const display = sourceDisplayName(name);
        // Absent is a real, common state — an unmatched room, or a source with
        // no join for this project — and says something different from
        // "matched but blank". So it is named, never an empty section.
        if (rows === null) {
          return (
            <div key={name}>
              <h4>{display}</h4>
              <Note>Not joined — this room matched no {display} record.</Note>
            </div>
          );
        }
        return <Section key={name} title={display} rows={applyFilters(rows, inspector)} source={name} />;
      })}
      <div className="insp-note insp-count">
        {shown.length} of {all.length} model properties shown
      </div>
    </>
  );
}
