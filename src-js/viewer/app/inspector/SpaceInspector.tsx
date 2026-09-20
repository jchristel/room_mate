// The MEP space panel.
//
// A space is a ROOM-shaped record from a services model, so this panel is the
// room panel's shape — properties, joined reference data — minus the two things
// a space has no answer for: it lists no contents, because nothing is
// attributed to a space, and it names no room, because **a space carries no
// room reference at all**. The QA report's key match is a different question
// and lives there.
//
// **`model_id` leads**, where a room's panel barely mentions one. One services
// file per service means the same number legitimately names a space in each of
// them, so "which discipline is this" is the first question about a space and
// the last about a room.

import {
  applyFilters,
  detectReferenceSources,
  keepChosen,
  propertyRows,
  referenceRows,
  sourceDisplayName,
} from "../../properties.js";
import { nameItems } from "../PropertyChooser.js";
import { findElement } from "../layers.js";
import { useViewer } from "../useViewer.js";
import { Filters } from "./Filters.js";
import { Head, Note, Section } from "./parts.js";
import type { Selection } from "../store.js";
import type { Room, Space } from "../../../renderer/types.js";

/** How an enclosure state reads, and what it means for the drawing.
 *
 *  The two states most worth seeing are exactly the ones with no geometry to
 *  see, so the panel says why the plan shows nothing rather than leaving a
 *  reader to conclude the layer is broken. */
const ENCLOSURE: Record<string, string> = {
  enclosed: "enclosed — bounded, and drawn",
  unenclosed: "unenclosed — no boundary resolved, so nothing is drawn",
  unmeasured: "unmeasured — no area, so nothing is drawn",
};

export function SpaceInspector({ selection }: { selection: Selection }) {
  const { payload, inspector, hiddenProperties } = useViewer();
  const found = findElement("spaces", selection.id);
  if (!found) return <Note>That space is not in the current scope.</Note>;

  const space = found.element as Space & { properties?: Room["properties"] };
  // A services model's level ids are its own — they never match the
  // architectural model's, which is why the storey join is by name and
  // elevation. So an unmatched id is named as the model's rather than printed
  // bare, where it would read as a level nobody can find.
  const declared = (payload?.levels ?? []).find((l) => l.id === space.level_id);
  const levelName = declared ? declared.name : space.level_id ? `level ${space.level_id} (this model's id)` : "";
  // A space's own reference sources are scoped to `spaces` on the server (R4),
  // so they are whatever THIS record carries rather than the rooms' list.
  const sources = detectReferenceSources({ rooms: [space as unknown as Room] });
  const drawn = !!space.loops?.[0]?.points?.length;

  const propsAll = propertyRows(space.properties);
  const referenceLists = sources.map((name) => referenceRows(space as unknown as Room, name));
  const hide = hiddenProperties["space"];

  return (
    <>
      <Head kind="space" title={space.name || space.id} sub={`${space.id} · ${levelName}`} zoneId={selection.zoneId}>
        <Filters scope="space" items={nameItems(propsAll, ...referenceLists.map((r) => r ?? []))} />
      </Head>
      <Section
        title="Placement"
        rows={[
          ["Model", space.model_id ?? "(unknown)"],
          [
            "Enclosure",
            space.enclosure
              ? (ENCLOSURE[space.enclosure] ?? space.enclosure)
              : // A snapshot pushed before the field existed says nothing about
                // enclosure, which is not the same as being unenclosed.
                "(not carried — exported before the field existed)",
          ],
          ["Drawn", drawn ? "yes" : "no — this space has no polygon"],
        ]}
      />
      <Section title="Properties" rows={applyFilters(keepChosen(propsAll, hide), inspector)} source="model" />
      {sources.map((name, i) => {
        const rows = referenceLists[i];
        const display = sourceDisplayName(name);
        if (rows == null) return null;
        return <Section key={name} title={display} rows={applyFilters(keepChosen(rows, hide), inspector)} source={name} />;
      })}
      <Note>
        A space is not attributed to a room — it carries no room reference at all — so nothing here names one. The QA
        report matches spaces to rooms by key, which is a different question.
      </Note>
    </>
  );
}
