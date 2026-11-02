// What a node is in the adjacency graph: a room, or a hierarchy area.
//
// The `/adjacency` payload is always a ROOM graph — that is the only granularity
// the geometry supports, since a shared wall is a fact about two room boundaries.
// A node here is either one of those rooms, or an **area group**: every room
// sharing a classification-path prefix at one tier, collapsed to a single node,
// with the room edges between two groups summed into one weighted edge.
//
// The aggregation is done here rather than server-side because it is a pure
// relabelling of a payload the client already holds: the nodes carry their
// classification path, the edges carry their shared length, and grouping by
// `pathKey` is exactly what `/areas` does to produce a group. A second endpoint
// would re-derive the same geometry to answer a question this file can answer by
// summing. It also means switching granularity is a re-layout, not a refetch.
//
// **Group identity is `areas.ts`'s, imported, never restated.** A footprint
// clicked on the plan carries `areaKey(group)`, and the graph must name the same
// node by the same string. When this lived in `static/graph.js` it read a copy of
// `pathKey` from `common.js`, and the copy joined with "/" where `areas.ts` joins
// with ">": identical at tier 0, where there is one segment and no separator, and
// matching nothing below it. Every department focused an empty graph. One
// function is the only way two views cannot disagree about that.

import type { ClassificationTier } from "../../renderer/types.js";
import { pathKey, tierLabel } from "../areas.js";

/** The `/adjacency` response, as far as the graph reads it. */
export interface AdjacencyPayload {
  nodes: AdjacencyNode[];
  edges: { a: string; b: string; shared_length: number }[];
  /** The wall tolerance the server applied, in feet. */
  wall_max?: number;
}

export interface AdjacencyNode {
  room_id: string;
  name?: string | null;
  level_id: string;
  classification?: ClassificationTier[] | null;
  centroid: { x: number; y: number };
}

/** One node of the view, in either granularity. */
export interface ViewNode {
  id: string;
  name: string;
  levelId: string;
  classification: ClassificationTier[];
  centroid: { x: number; y: number };
  /** How many rooms the node stands for: 1 for a room, the member count for an
   *  area, which the label shows so a group of one and of ninety differ. */
  rooms: number;
}

export interface ViewEdge {
  a: string;
  b: string;
  weight: number;
}

/** The graph at one granularity. Everything downstream — breadth-first search,
 *  layout, drawing, hit testing — reads this and never the payload, so room and
 *  area granularity are one code path with two builders. */
export interface GraphView {
  nodesById: Map<string, ViewNode>;
  edges: ViewEdge[];
  neighbours: Map<string, { id: string; weight: number }[]>;
}

/** The view at `groupDepth`: null for rooms, else the classification depth
 *  whose groups are the nodes. */
export function buildView(payload: AdjacencyPayload | null, groupDepth: number | null): GraphView {
  const nodesById = new Map<string, ViewNode>();
  let edges: ViewEdge[] = [];
  if (payload) {
    edges = groupDepth == null ? roomView(payload, nodesById) : areaView(payload, groupDepth, nodesById);
  }
  const neighbours = new Map<string, { id: string; weight: number }[]>([...nodesById.keys()].map((id) => [id, []]));
  for (const e of edges) {
    const a = neighbours.get(e.a);
    const b = neighbours.get(e.b);
    // An edge naming a node that is not in the view cannot happen from either
    // builder, but a defensive skip beats a thrown TypeError inside a rAF.
    if (!a || !b) continue;
    a.push({ id: e.b, weight: e.weight });
    b.push({ id: e.a, weight: e.weight });
  }
  return { nodesById, edges, neighbours };
}

/** Room granularity: the payload verbatim, renamed onto the view's fields. */
function roomView(payload: AdjacencyPayload, nodesById: Map<string, ViewNode>): ViewEdge[] {
  for (const n of payload.nodes) {
    nodesById.set(n.room_id, {
      id: n.room_id,
      name: n.name ?? "",
      levelId: n.level_id,
      classification: n.classification ?? [],
      centroid: n.centroid,
      rooms: 1,
    });
  }
  return payload.edges.map((e) => ({ a: e.a, b: e.b, weight: e.shared_length }));
}

/**
 * Area granularity: one node per (level, path prefix at `groupDepth`), and one
 * edge per pair of groups carrying the SUM of the room edges between them.
 * Summing is the honest aggregate — two departments are adjacent by however
 * much wall they actually share, and the thickness scale then reads the same
 * way it does for rooms.
 */
function areaView(payload: AdjacencyPayload, groupDepth: number, nodesById: Map<string, ViewNode>): ViewEdge[] {
  const groupOf = new Map<string, string>(); // room id -> group id
  for (const n of payload.nodes) {
    const path = n.classification ?? [];
    // A room classified shallower than this tier belongs to no group here —
    // the same rooms `/areas` reports at no group. Dropping it beats inventing
    // a group for it, and it takes its edges with it rather than wiring them
    // to something arbitrary.
    if (path.length <= groupDepth) continue;
    const id = `${n.level_id}|${pathKey(path, groupDepth)}`;
    groupOf.set(n.room_id, id);
    const existing = nodesById.get(id);
    if (!existing) {
      nodesById.set(id, {
        id,
        // The group's own tier value. Two groups under different parents can
        // read alike; the label is the leaf because that is what the areas
        // overlay and band both show, and the full path would not fit.
        name: tierLabel(path[groupDepth]),
        levelId: n.level_id,
        classification: path.slice(0, groupDepth + 1),
        // Mean of the member rooms' centroids. Only ever used to seed a
        // bearing, so an unweighted mean is enough — this is not an area
        // centroid and must not be read as one.
        centroid: { x: n.centroid.x, y: n.centroid.y },
        rooms: 1,
      });
    } else {
      existing.centroid.x += n.centroid.x;
      existing.centroid.y += n.centroid.y;
      existing.rooms += 1;
    }
  }
  for (const g of nodesById.values()) {
    g.centroid.x /= g.rooms;
    g.centroid.y /= g.rooms;
  }

  // Nested by the ordered pair rather than keyed on a joined string: a group id
  // already contains a level id and a joined path, so there is no separator left
  // that two ids cannot fake between them, and a collision here would silently
  // merge two different pairs of departments into one edge.
  const summed = new Map<string, Map<string, number>>();
  for (const e of payload.edges) {
    const ga = groupOf.get(e.a);
    const gb = groupOf.get(e.b);
    // A wall inside one group is not a relationship BETWEEN groups: it is what
    // the group's own footprint already dissolved.
    if (!ga || !gb || ga === gb) continue;
    const [a, b] = ga < gb ? [ga, gb] : [gb, ga];
    let row = summed.get(a);
    if (!row) summed.set(a, (row = new Map()));
    row.set(b, (row.get(b) ?? 0) + e.shared_length);
  }
  const edges: ViewEdge[] = [];
  for (const [a, row] of summed) for (const [b, weight] of row) edges.push({ a, b, weight });
  // Deterministic order, like the server's: the drawing order of overlapping
  // edges should not depend on Map iteration.
  edges.sort((x, y) => (x.a === y.a ? (x.b < y.b ? -1 : 1) : x.a < y.a ? -1 : 1));
  return edges;
}

/**
 * The colour key for a classification at `tierDepth`: its code, else its name,
 * else null for an undefined or absent tier — drawn in the neutral fill, never
 * an error colour, the plan's "no data" stance. An area aggregated ABOVE the
 * colour tier has no value there, so it falls to the neutral fill rather than
 * borrowing one member room's colour.
 */
export function colourKey(classification: readonly ClassificationTier[], tierDepth: number): string | null {
  const tv = classification[tierDepth];
  if (!tv || tv.undefined) return null;
  return tv.code ?? tv.name ?? null;
}

/**
 * The sorted colour keys, built from the PAYLOAD's rooms rather than the view's
 * nodes: the same department then gets the same palette index on every fetch,
 * at every depth, and in both granularities. A colour that changed as rooms came
 * and went — or as the reader switched from rooms to areas — would be worse
 * than useless.
 */
export function colourKeys(payload: AdjacencyPayload | null, tierDepth: number): string[] {
  const keys = new Set<string>();
  for (const n of payload?.nodes ?? []) {
    const k = colourKey(n.classification ?? [], tierDepth);
    if (k != null) keys.add(k);
  }
  return [...keys].sort();
}

/** The tier labels the colour picker offers, from the ROOMS' own paths, so the
 *  picker offers every tier even when the view is aggregated to a shallow one. */
export function tierNames(payload: AdjacencyPayload | null): string[] {
  const names: string[] = [];
  for (const n of payload?.nodes ?? []) (n.classification ?? []).forEach((t, i) => (names[i] ??= t.tier));
  return names;
}
