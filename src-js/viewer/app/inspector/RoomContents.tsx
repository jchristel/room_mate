// "In this room": the doors, windows and FF&E a room owns.
//
// **No server call, and that is the design rather than an optimisation.** The
// element payloads the poll already holds carry each element's owning room, so
// this is an inversion of data the page has — the same client-side reshuffle
// rule that keeps CSV export and area tabulation off the server. It matters
// more here than tidiness: `/ffe` is 133 MB on RHH, so a panel that fetched on
// the first room click would stall the page on a click that looks free.
//
// **The payloads hold only the storeys on screen**, so a room on a storey no
// zone shows has had its contents not fetched at all — and "none in this room"
// would be a false finding. Such a room says so instead.

import { elementsOnStorey, layerState, type ElementOf } from "../layers.js";
import { select } from "../store.js";
import { useViewer } from "../useViewer.js";
import { Note, Row } from "./parts.js";
import type { ElementEntity } from "../../elementUrls.js";
import type { Room } from "../../../renderer/types.js";

/** How many element rows one entity lists before it stops. A room with 400
 *  items is a scroll, not a reading — and the tally above always states the
 *  TRUE count, so the cap hides rows and never a number. */
const LIMIT = 15;

const SPECS: readonly {
  entity: ElementEntity;
  title: string;
  label: (e: ElementOf[ElementEntity]) => string;
  detail: (e: ElementOf[ElementEntity]) => string;
}[] = [
  { entity: "doors", title: "Doors", label: (d) => named(d), detail: (d) => id(d) },
  { entity: "windows", title: "Windows", label: (w) => named(w), detail: (w) => id(w) },
  {
    entity: "ffe",
    title: "FF&E",
    // Category first: on a furnished room the type names repeat, and the
    // category is what separates six chairs from six luminaires.
    label: (i) => {
      const item = i as { category?: string };
      return (item.category ? `${item.category} · ` : "") + named(i);
    },
    detail: (i) => id(i),
  },
];

function named(e: unknown): string {
  const el = e as { type_name?: string | null; id: string };
  return el.type_name || el.id;
}

function id(e: unknown): string {
  return (e as { id: string }).id;
}

/**
 * Which rooms an element is attributed to, as `{room_id, model_id}` refs.
 *
 * `owner_rooms_qualified` rather than `owner_rooms`, and the difference is the
 * whole correctness of this panel: a room id is unique only within its model,
 * so matching on a bare id attributes one model's doors to another model's
 * same-numbered room — in the one direction nobody would notice. The qualified
 * list carries the model with each id, which makes the ambiguity DETECTABLE
 * even though `/rooms` does not serve a room's model, so it cannot be resolved.
 */
function ownerRefs(element: unknown): { room_id: string; model_id: string | null }[] {
  const el = element as { owner_rooms_qualified?: { room_id: string; model_id: string | null }[]; owner_rooms?: string[] };
  if (Array.isArray(el.owner_rooms_qualified) && el.owner_rooms_qualified.length) return el.owner_rooms_qualified;
  return (el.owner_rooms ?? []).map((room_id) => ({ room_id, model_id: null }));
}

export function RoomContents({ room, storeyShown }: { room: Room; storeyShown: boolean }) {
  const { layersRevision } = useViewer();
  void layersRevision; // re-read when a layer's payload moves

  return (
    <>
      <h4>In this room</h4>
      <div className="insp-rows">
        {SPECS.map((spec) => {
          const state = layerState(spec.entity);
          const matched: unknown[] = [];
          const models = new Set<string>();
          if (storeyShown && state === "loaded") {
            for (const element of elementsOnStorey(spec.entity, room.level_id ?? null).kept) {
              let hit = false;
              for (const ref of ownerRefs(element)) {
                if (ref.room_id !== room.id) continue;
                hit = true;
                if (ref.model_id) models.add(ref.model_id);
              }
              if (hit) matched.push(element);
            }
            matched.sort((a, b) => spec.label(a as never).localeCompare(spec.label(b as never)));
          }

          // A read that answered 200 with an empty list means the same to a
          // reader as one that answered 204 — and RHH proves it matters: it has
          // no windows snapshot at all, so `/windows` answers 200 with an empty
          // list. Reporting that as "none in this room" would put a fact about
          // the PROJECT into a sentence about the room, on all 3,013 of them.
          const tally = !storeyShown
            ? "storey not on screen"
            : state === "pending"
              ? "not loaded yet"
              : state === "error"
                ? "could not be read"
                : state === "empty"
                  ? "none in this scope"
                  : matched.length
                    ? `${matched.length}`
                    : "none in this room";

          return (
            <div key={spec.entity}>
              <Row label={spec.title} value={tally} />
              {matched.slice(0, LIMIT).map((element) => (
                <Row
                  key={id(element)}
                  label={spec.label(element as never)}
                  value={spec.detail(element as never)}
                  onClick={() =>
                    select(spec.entity === "doors" ? "door" : spec.entity === "windows" ? "window" : "item", id(element))
                  }
                />
              ))}
              {matched.length > LIMIT ? <Row label={`+${matched.length - LIMIT} more`} value="" /> : null}
              {models.size > 1 ? (
                <Note>
                  Claimed by {models.size} models — a room id is unique only within its model, and `/rooms` does not say
                  which this room is in.
                </Note>
              ) : null}
            </div>
          );
        })}
      </div>
    </>
  );
}
