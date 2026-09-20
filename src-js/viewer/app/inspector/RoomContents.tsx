// "In this room": the doors, windows, FF&E, ceilings and floors a room owns,
// and the chooser that says which of those it lists.
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
//
// **Two joins, not one** (C4). A door, window or item is ATTRIBUTED to a room
// and names it; a ceiling or floor covers rooms and names them with how much
// of each. So the first three are read forwards and the surfaces are that list
// inverted — which is also why a surface row shows its share of the room
// rather than an id: the fraction is what this join knows and the other does
// not, and the id is one click away on the surface's own panel.

import { DropMenu } from "../DropMenu.js";
import { elementsOnStorey, layerState, type ElementOf } from "../layers.js";
import { select, setRoomContents, type ContentsEntity, type SelectionKind } from "../store.js";
import { useViewer } from "../useViewer.js";
import { Note, Row } from "./parts.js";
import type { Room } from "../../../renderer/types.js";

/** How many element rows one entity lists before it stops. A room with 400
 *  items is a scroll, not a reading — and the tally above always states the
 *  TRUE count, so the cap hides rows and never a number. */
const LIMIT = 15;

/**
 * One room reference, from either direction of the join.
 *
 * `model_id` is not decoration: a room id is unique only within its model, so
 * matching on a bare id attributes one model's doors to another model's
 * same-numbered room — in the one direction nobody would notice. Carrying the
 * model makes that ambiguity DETECTABLE even though `/rooms` does not serve a
 * room's model, so it still cannot be resolved.
 */
interface RoomRef {
  room_id: string;
  model_id: string | null;
  /** How much of the ROOM this element covers. Surfaces only — an opening is
   *  attributed to a room rather than measured against it. */
  fraction_of_room?: number;
}

const SPECS: readonly {
  entity: ContentsEntity;
  kind: SelectionKind;
  title: string;
  refs: (e: ElementOf[ContentsEntity]) => readonly RoomRef[];
  label: (e: ElementOf[ContentsEntity]) => string;
  detail: (e: ElementOf[ContentsEntity], ref: RoomRef) => string;
}[] = [
  { entity: "doors", kind: "door", title: "Doors", refs: ownerRefs, label: named, detail: id },
  { entity: "windows", kind: "window", title: "Windows", refs: ownerRefs, label: named, detail: id },
  {
    entity: "ffe",
    kind: "item",
    title: "FF&E",
    refs: ownerRefs,
    // Category first: on a furnished room the type names repeat, and the
    // category is what separates six chairs from six luminaires.
    label: (i) => {
      const item = i as { category?: string };
      return (item.category ? `${item.category} · ` : "") + named(i);
    },
    detail: id,
  },
  { entity: "ceilings", kind: "ceiling", title: "Ceilings", refs: coveredRefs, label: named, detail: coverage },
  { entity: "floors", kind: "floor", title: "Floors", refs: coveredRefs, label: named, detail: coverage },
];

function named(e: unknown): string {
  const el = e as { type_name?: string | null; id: string };
  return el.type_name || el.id;
}

function id(e: unknown): string {
  return (e as { id: string }).id;
}

/** Which rooms an opening or item is attributed to.
 *
 *  `owner_rooms_qualified` rather than `owner_rooms`: the qualified list
 *  carries the model with each id, which is what makes the ambiguity above
 *  detectable. The bare list is the fallback for a server too old to send it. */
function ownerRefs(element: unknown): readonly RoomRef[] {
  const el = element as { owner_rooms_qualified?: RoomRef[]; owner_rooms?: string[] };
  if (Array.isArray(el.owner_rooms_qualified) && el.owner_rooms_qualified.length) return el.owner_rooms_qualified;
  return (el.owner_rooms ?? []).map((room_id) => ({ room_id, model_id: null }));
}

/** Which rooms a ceiling or floor lies over — the same join read the other
 *  way round. Attribution is derived at read time and model-scoped, so the
 *  entry always carries the model and the ambiguity note applies unchanged. */
function coveredRefs(element: unknown): readonly RoomRef[] {
  return (element as { rooms?: RoomRef[] }).rooms ?? [];
}

/** A surface row's value: how much of THIS room it covers.
 *
 *  Not the surface's id, and not `fraction_of_element`. "Is this room fully
 *  ceiled" is what the panel is being read for; how much of the ceiling the
 *  room takes is a question about the ceiling, and its own panel answers it. */
function coverage(_element: unknown, ref: RoomRef): string {
  const f = ref.fraction_of_room;
  if (f == null) return "";
  return `${(f * 100).toFixed(f >= 0.1 ? 0 : 1)}% of the room`;
}

export function RoomContents({ room, storeyShown }: { room: Room; storeyShown: boolean }) {
  const { layersRevision, roomContents } = useViewer();
  void layersRevision; // re-read when a layer's payload moves

  const chosen = SPECS.filter((s) => roomContents[s.entity]);

  return (
    <>
      <div className="insp-section-head">
        <h4>In this room</h4>
        <DropMenu label={`Lists: ${chosen.length}`} title="Which related types this panel lists">
          {SPECS.map((spec) => (
            <label key={spec.entity}>
              <input
                type="checkbox"
                checked={roomContents[spec.entity]}
                onChange={(e) => setRoomContents(spec.entity, e.target.checked)}
              />{" "}
              {spec.title}
            </label>
          ))}
          {/* Named rather than omitted. A space is the one entity here with
              nothing to join on — it carries no room reference at all — and a
              silently missing option reads as an oversight, where a disabled
              one saying why is a reported state. */}
          <label className="menu-off" title="A space carries no room reference, so there is nothing to list it by">
            <input type="checkbox" disabled checked={false} readOnly /> Spaces — no room reference
          </label>
        </DropMenu>
      </div>
      <div className="insp-rows">
        {chosen.map((spec) => {
          const state = layerState(spec.entity);
          const matched: { element: unknown; ref: RoomRef }[] = [];
          const models = new Set<string>();
          if (storeyShown && state === "loaded") {
            for (const element of elementsOnStorey(spec.entity, room.level_id ?? null).kept) {
              let hit: RoomRef | null = null;
              for (const ref of spec.refs(element)) {
                if (ref.room_id !== room.id) continue;
                hit = hit ?? ref;
                if (ref.model_id) models.add(ref.model_id);
              }
              if (hit) matched.push({ element, ref: hit });
            }
            matched.sort((a, b) => spec.label(a.element as never).localeCompare(spec.label(b.element as never)));
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
              <Row className="insp-tally" label={spec.title} value={tally} />
              {matched.slice(0, LIMIT).map(({ element, ref }) => (
                <Row
                  key={id(element)}
                  label={spec.label(element as never)}
                  value={spec.detail(element as never, ref)}
                  onClick={() => select(spec.kind, id(element))}
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
        {/* None chosen is a state the reader put the panel in, so it says so
            rather than showing a heading over nothing. */}
        {chosen.length ? null : <Note>Nothing chosen — the menu above says what this section can list.</Note>}
      </div>
    </>
  );
}
