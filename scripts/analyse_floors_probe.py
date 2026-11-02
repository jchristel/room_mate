#!/usr/bin/env python3
"""Check the floors entity's unmeasured rules against what
`probe_floors_export.py` captured in Revit.

    python scripts/analyse_floors_probe.py
    python scripts/analyse_floors_probe.py --dir some/other/fixtures

    F1  Does every floor export a usable polygon?
    F2  Are floors and rooms in the same document?  **The scope question.**
    F3  Is the exported footprint the floor Revit measures?
    F4  Where do floors sit relative to their level?
    F5  How many floors are structural, and which phases are they in?
    F6  Does the floor sliver rule hold?  **The threshold question.**

Writes `floors-probe-report.md` beside the inputs and prints the same thing.

**Floors shipped before this ran, which is the reverse of ceilings, and this
file is written to be able to say the shipped rules are wrong.** The contract,
the union and the export path were measured on ceilings and carry over by
construction -- duHast's `to_data_floor` is `to_data_ceiling` with two names
swapped. What did not carry over is below, and each question reports the
numbers that would overturn a rule rather than a verdict that confirms it:

- F2 decides whether the server's model-scoped join can attribute floors at
  all on a project that keeps slabs and rooms in different documents.
- F6 re-runs the attribution the server does, under BOTH sliver rules and
  duHast's own, so the disagreement between them is visible pair by pair. The
  constants are copied from `src/service/surface_attribution.rs` and must move
  with it.

F3 AND F6 NEED shapely, AND DELIBERATELY DO NOT FAKE IT -- the ceilings
analyser's rule. Without it F1, F2, F4 and F5 answer in full, F3 reports the
Revit area against the stdlib sum of the pieces (which a duplicated face
inflates, and says so), and F6 says plainly that it did not run.

    pip install shapely

**Validity is measured BEFORE repair**, unlike the ceilings analyser, which
`buffer(0)`s every invalid piece and never counts them. A ring invalid as
exported is the signature of the suspected edge-orientation fault in duHast's
`convert_edge_arrays_into_list_of_points`, and repairing it first is how that
went unmeasured for a whole entity.

Stdlib only otherwise, CPython 3.
"""

import argparse
import json
import os
from collections import Counter, OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_DIR = os.path.join(HERE, "fixtures")

# The ceiling and floor sliver rules as they were in `src/service/
# surface_attribution.rs` until 2026-09-16, when both were replaced by one
# 10 mm mean-width tolerance. F6 compares these retired rules, which is the
# question this analyser was written to answer; they are NOT the server's
# rule any more, so an F6 result is history, not a prediction of `/floors`.
MIN_OVERLAP_AREA = 1.0
MIN_FRACTION_OF_CEILING = 0.005
MIN_FLOOR_MEAN_WIDTH_FT = 1.5
MIN_ROOM_COVER_FOR_NARROW = 0.5
MIN_FLOOR_COVER_FOR_NARROW = 0.5

# duHast's `_intersect_floor_vs_room`: 0.1% of the ROOM.
DUHAST_PCT_OF_ROOM = 0.1

# The server's `LEVEL_EPS_MM` (50) in the probe's unit, feet.
LEVEL_EPS_FT = 50.0 / 304.8

# How far the union of the exported pieces may differ from `HOST_AREA_COMPUTED`
# before F3 calls it a mismatch. Revit's figure is the TOP face; the export
# keeps the LOWER face of each equal-area pair, so a tapered edge or a sloped
# top legitimately differs by a little.
AREA_TOLERANCE = 0.02

# An offset this large, in feet, is a floor that may belong to another storey.
# A storey is rarely under 8 ft; 3 ft is well past any finish build-up or
# depressed slab and well short of a storey.
SUSPECT_OFFSET_FT = 3.0

MEAN_WIDTH_BINS = [0.25, 0.5, 1.0, 1.5, 2.0, 3.0, 5.0]

try:
    from shapely.geometry import Polygon as ShapelyPolygon
    from shapely.ops import unary_union

    HAVE_SHAPELY = True
except Exception:
    ShapelyPolygon = None
    unary_union = None
    HAVE_SHAPELY = False


# --------------------------------------------------------------------------
# Loading
# --------------------------------------------------------------------------


def load(path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def load_inputs(directory):
    """Every document named by the index, with its probe and raw export."""
    index_path = os.path.join(directory, "floors-probe-index.json")
    if not os.path.isfile(index_path):
        raise SystemExit(
            "no floors-probe-index.json in {}\n"
            "Run scripts/probe_floors_export.py from pyRevit first.".format(directory)
        )
    index = load(index_path)

    documents = []
    for entry in index.get("documents", []):
        slug = entry.get("slug")
        loaded = {"document": entry.get("document"), "slug": slug, "entities": {}}
        for entity in ("floors", "rooms"):
            probe_path = os.path.join(directory, "{}-probe-{}.json".format(entity, slug))
            raw_path = os.path.join(directory, "{}-raw-{}.json".format(entity, slug))
            if not os.path.isfile(probe_path):
                continue
            loaded["entities"][entity] = {
                "probe": load(probe_path),
                "raw": load(raw_path) if os.path.isfile(raw_path) else None,
            }
        documents.append(loaded)
    return index, documents


def reduce_export(raw, list_key):
    """The exported elements as `{id: record}`, keyed by `instance_properties.id`
    -- where the extractor reads the id from."""
    if not raw:
        return {}
    out = OrderedDict()
    for element in raw.get(list_key, []) or []:
        props = element.get("instance_properties") or {}
        element_id = props.get("id")
        if element_id is None:
            continue
        out[str(element_id)] = element
    return out


# --------------------------------------------------------------------------
# Geometry
# --------------------------------------------------------------------------


def point_xy(point):
    """One duHast point as `(x, y)`, from a mapping or a list."""
    if isinstance(point, dict):
        x = point.get("x")
        y = point.get("y")
    elif isinstance(point, (list, tuple)) and len(point) >= 2:
        x, y = point[0], point[1]
    else:
        return None
    try:
        return (float(x), float(y))
    except (TypeError, ValueError):
        return None


def ring_points(loop):
    points = [point_xy(p) for p in (loop or [])]
    return [p for p in points if p is not None]


def ring_area(ring):
    """Absolute shoelace area of a closed ring."""
    if len(ring) < 3:
        return 0.0
    total = 0.0
    for i in range(len(ring)):
        x1, y1 = ring[i]
        x2, y2 = ring[(i + 1) % len(ring)]
        total += x1 * y2 - x2 * y1
    return abs(total) / 2.0


def polygons_of(element):
    """Every piece on one exported element, as `(outer, [inners])` tuples."""
    out = []
    for poly in element.get("polygon") or []:
        if not isinstance(poly, dict):
            continue
        outer = ring_points(poly.get("outer_loop"))
        inners = [ring_points(loop) for loop in (poly.get("inner_loops") or [])]
        inners = [ring for ring in inners if len(ring) >= 3]
        out.append((outer, inners))
    return out


def pieces_as_shapely(element):
    """`(valid_pieces, invalid_count)`: every piece as a shapely polygon,
    repaired where invalid, and how many needed repair.

    The count is taken BEFORE `buffer(0)`. See the module docstring.
    """
    pieces = []
    invalid = 0
    for outer, inners in polygons_of(element):
        if len(outer) < 3:
            continue
        try:
            piece = ShapelyPolygon(outer, inners)
            if not piece.is_valid:
                invalid += 1
                piece = piece.buffer(0)
            if not piece.is_empty:
                pieces.append(piece)
        except Exception:
            invalid += 1
    return pieces, invalid


def floor_shape(element):
    """The union of a floor's pieces -- what `surface_shape` computes on the
    server -- plus the count of pieces that were invalid as exported."""
    pieces, invalid = pieces_as_shapely(element)
    if not pieces:
        return None, invalid
    shape = unary_union(pieces)
    return (shape if not shape.is_empty else None), invalid


def room_outline(element):
    """A room's OUTER ring only, as the server's `room_locator::outline_of`
    takes it: a room's hole is a column, and the server drops it."""
    polygons = polygons_of(element)
    if not polygons or len(polygons[0][0]) < 3:
        return None
    try:
        shape = ShapelyPolygon(polygons[0][0])
        if not shape.is_valid:
            shape = shape.buffer(0)
        return shape if not shape.is_empty else None
    except Exception:
        return None


# --------------------------------------------------------------------------
# F1 -- export completeness
# --------------------------------------------------------------------------


def drop_rows(documents):
    """Per document: collected, exported, exported-but-empty, and why the rest
    went missing -- the ceilings analyser's three causes, kept apart."""
    rows = []
    for doc in documents:
        block = doc["entities"].get("floors")
        if not block:
            continue
        probe = block["probe"]
        exported = reduce_export(block["raw"], probe.get("list_key", "floor"))
        empty = sum(1 for element in exported.values() if not polygons_of(element))
        in_place = no_solids = solids_but_dropped = 0
        for element in probe.get("elements", []):
            if str(element.get("id")) in exported:
                continue
            if element.get("is_in_place"):
                in_place += 1
            elif not element.get("solid_count"):
                no_solids += 1
            else:
                solids_but_dropped += 1
        rows.append([
            doc["document"],
            probe.get("collected_count", 0),
            probe.get("duhast_collected_count"),
            probe.get("exported_count", 0),
            empty,
            in_place,
            no_solids,
            solids_but_dropped,
        ])
    return rows


# --------------------------------------------------------------------------
# F2 -- co-location
# --------------------------------------------------------------------------


def colocation_rows(documents):
    rows = []
    verdicts = Counter()
    for doc in documents:
        floors = doc["entities"].get("floors", {}).get("probe", {})
        rooms = doc["entities"].get("rooms", {}).get("probe", {})
        n_floors = floors.get("collected_count", 0) or 0
        n_structural = sum(1 for e in floors.get("elements", []) if e.get("is_structural"))
        n_rooms = rooms.get("collected_count", 0) or 0
        if n_floors and n_rooms:
            verdict = "both"
        elif n_floors:
            verdict = "FLOORS ONLY"
        elif n_rooms:
            verdict = "rooms only"
        else:
            verdict = "neither"
        verdicts[verdict] += 1
        rows.append([doc["document"], n_floors, n_structural, n_rooms, verdict])
    return rows, verdicts


# --------------------------------------------------------------------------
# F3 -- footprint fidelity
# --------------------------------------------------------------------------


def fidelity(documents):
    """Exported area against Revit's own, per floor, and invalid rings by
    whether the floor has curved edges."""
    buckets = Counter()
    worst = []
    invalid_by_curve = Counter()
    for doc in documents:
        block = doc["entities"].get("floors")
        if not block:
            continue
        probe = block["probe"]
        facts = {str(e.get("id")): e for e in probe.get("elements", [])}
        exported = reduce_export(block["raw"], probe.get("list_key", "floor"))
        for element_id, element in exported.items():
            fact = facts.get(element_id, {})
            host = fact.get("host_area_sqft")
            curved = (fact.get("curved_edge_count") or 0) > 0
            if not polygons_of(element):
                buckets["empty polygon"] += 1
                continue
            if HAVE_SHAPELY:
                shape, invalid = floor_shape(element)
                measured = shape.area if shape is not None else 0.0
                if invalid:
                    invalid_by_curve["with arcs" if curved else "straight edges only"] += 1
            else:
                measured = sum(ring_area(o) - sum(ring_area(i) for i in inn) for o, inn in polygons_of(element))
            if not host:
                buckets["no Revit area"] += 1
                continue
            ratio = measured / host
            if abs(ratio - 1.0) <= AREA_TOLERANCE:
                buckets["within {:.0f}%".format(AREA_TOLERANCE * 100)] += 1
            elif ratio < 1.0:
                buckets["SHORT of Revit"] += 1
                worst.append((ratio, doc["document"], element_id, measured, host))
            else:
                buckets["OVER Revit"] += 1
                worst.append((ratio, doc["document"], element_id, measured, host))
    worst.sort(key=lambda row: abs(row[0] - 1.0), reverse=True)
    return buckets, worst[:15], invalid_by_curve


RING_EPS = 1e-6


def distinct_ring(ring):
    """A ring with consecutive duplicate vertices and a repeated closing vertex
    removed. Consecutive duplicates are benign -- duHast emits them where two
    collinear edges meet -- and a segment test that kept them would report every
    such ring as self-intersecting, which is what the first reading of House A
    did (15 floors "invalid", 1 actually crossing)."""
    out = []
    for point in ring:
        if out and abs(out[-1][0] - point[0]) < RING_EPS and abs(out[-1][1] - point[1]) < RING_EPS:
            continue
        out.append(point)
    if len(out) > 1 and abs(out[0][0] - out[-1][0]) < RING_EPS and abs(out[0][1] - out[-1][1]) < RING_EPS:
        out.pop()
    return out


def ring_cause(ring):
    """'ok', 'degenerate' (under three distinct points: a sliver off a side
    face) or 'crossing' (two non-adjacent edges properly cross). Exact, stdlib,
    and deliberately NOT shapely's `is_valid`, which fuses the three."""
    ring = distinct_ring(ring)
    n = len(ring)
    if n < 3:
        return "degenerate"

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    for i in range(n):
        a, b = ring[i], ring[(i + 1) % n]
        for j in range(i + 2, n):
            if i == 0 and j == n - 1:
                continue
            c, d = ring[j], ring[(j + 1) % n]
            d1, d2, d3, d4 = cross(a, b, c), cross(a, b, d), cross(c, d, a), cross(c, d, b)
            if d1 * d2 < -RING_EPS and d3 * d4 < -RING_EPS:
                return "crossing"
    return "ok"


def point_in_ring(ring, point):
    """Even-odd ray cast -- NOT duHast's quadrant winding test, whose +2 case is
    the thing being diagnosed."""
    x, y = point
    inside = False
    for i in range(len(ring)):
        x1, y1 = ring[i]
        x2, y2 = ring[(i + 1) % len(ring)]
        if (y1 > y) != (y2 > y) and x1 + (y - y1) * (x2 - x1) / (y2 - y1) > x:
            inside = not inside
    return inside


def interior_point(ring):
    """A point strictly inside `ring`: an edge midpoint nudged inward."""
    signed = 0.0
    for i in range(len(ring)):
        signed += ring[i][0] * ring[(i + 1) % len(ring)][1] - ring[(i + 1) % len(ring)][0] * ring[i][1]
    for i in range(len(ring)):
        (x1, y1), (x2, y2) = ring[i], ring[(i + 1) % len(ring)]
        length = ((x2 - x1) ** 2 + (y2 - y1) ** 2) ** 0.5
        if length < 1e-3:
            continue
        nx, ny = -(y2 - y1) / length, (x2 - x1) / length
        if signed < 0:
            nx, ny = -nx, -ny
        candidate = ((x1 + x2) / 2 + nx * 0.01, (y1 + y2) / 2 + ny * 0.01)
        if point_in_ring(ring, candidate):
            return candidate
    return None


def geometry_faults(documents):
    """Three faults a union cannot undo, each measured without shapely.

    - **Holes exported as islands.** A piece lying inside a larger piece's outer
      ring while the larger one lists no hole around it, AND the larger piece
      minus the nested ones comes to Revit's own area. The area identity is
      what separates this from a step or a recess (a smaller face at another
      height, which is floor and which the union rightly keeps). The union then
      fills the hole back in. Diagnosed on House A as duHast's
      `geometry.adjust_delta` skipping the x-intercept test for `delta == +2`.
    - **Crossing rings**, split by whether the floor has arcs -- the
      edge-orientation suspect in `convert_edge_arrays_into_list_of_points`.
    - **Faces lost**: the stdlib SUM of every piece below Revit's area. A sum
      can only overstate the union, so a sum that falls short is a floor whose
      horizontal faces did not all reach the export.
    """
    islands = []
    crossing = Counter()
    lost = []
    degenerate = 0
    for doc in documents:
        block = doc["entities"].get("floors")
        if not block:
            continue
        probe = block["probe"]
        facts = {str(e.get("id")): e for e in probe.get("elements", [])}
        exported = reduce_export(block["raw"], probe.get("list_key", "floor"))
        for element_id, element in exported.items():
            fact = facts.get(element_id, {})
            host = fact.get("host_area_sqft") or 0.0
            pieces = [(distinct_ring(o), [distinct_ring(h) for h in inn]) for o, inn in polygons_of(element)]
            causes = set()
            for outer, holes in pieces:
                for ring in [outer] + holes:
                    cause = ring_cause(ring)
                    causes.add(cause)
                    degenerate += cause == "degenerate"
            if "crossing" in causes:
                crossing["with arcs" if (fact.get("curved_edge_count") or 0) > 0 else "straight edges only"] += 1

            nets = [ring_area(o) - sum(ring_area(h) for h in hs) for o, hs in pieces]
            total = sum(nets)
            if host and total < host * (1.0 - 0.10):
                lost.append([doc["document"], element_id, len(pieces), fact.get("horizontal_face_count"),
                             "{:.1f}".format(total), "{:.1f}".format(host)])

            if len(pieces) < 2 or not host:
                continue
            largest = max(range(len(pieces)), key=lambda i: nets[i])
            outer, holes = pieces[largest]
            if len(outer) < 3:
                continue
            nested = []
            for i, (other, _) in enumerate(pieces):
                if i == largest or len(other) < 3:
                    continue
                probe_point = interior_point(other)
                if probe_point and point_in_ring(outer, probe_point) and not any(
                        point_in_ring(h, probe_point) for h in holes if len(h) >= 3):
                    nested.append(i)
            if nested:
                as_holes = nets[largest] - sum(nets[i] for i in nested)
                if abs(as_holes - host) <= host * AREA_TOLERANCE:
                    islands.append([doc["document"], element_id, len(nested),
                                    "{:.1f}".format(nets[largest]), "{:.1f}".format(as_holes), "{:.1f}".format(host)])
    return islands, crossing, lost, degenerate


def host_level_mismatches(documents):
    """Floors whose exported `level.id` is not the host level Revit states.

    **The storey is the host Level, by decision (2026-09-13)** -- never a level
    derived from elevation plus offset. So the one thing to check is that the
    export carries it: duHast's `get_level_data` reads `LevelId`, and this
    compares that against the level the probe read off the element itself.
    """
    rows = []
    for doc in documents:
        block = doc["entities"].get("floors")
        if not block:
            continue
        facts = {str(e.get("id")): e for e in block["probe"].get("elements", [])}
        for element_id, element in reduce_export(block["raw"], block["probe"].get("list_key", "floor")).items():
            exported = (element.get("level") or {}).get("id")
            host = facts.get(element_id, {}).get("level_id")
            if exported is not None and host is not None and str(exported) != str(host):
                rows.append([doc["document"], element_id, exported, host])
    return rows


def storey_of_top(documents):
    """Floors whose TOP (level elevation + offset) is clearly on another storey:
    at or above the next level up, or more than half a storey below their own.

    **Reported, never reassigned.** The server joins a floor to its host
    level's rooms, and that is the decision rather than a gap: a level derived
    from elevation is the FF&E trap in another field. This table exists so a
    reader knows which attributions come from a floor hosted a storey away --
    the House A roof build-ups hosted on LEVEL 01 at +11.3 to +12.5 ft are the
    case -- and can filter on `height_offset`, which rides every row.

    A band, not "the highest level at or below the top", and House A is why: a
    finish floor or joist zone sits a few inches below its level as a matter of
    course, and the strict test put 20 ordinary LEVEL 01 floors on LEVEL 00. The
    clear cases are the roof build-ups hosted on the level below at a storey's
    offset -- in plan they lie over that level's rooms, so a join on the host
    level reads a roof as those rooms' floor.

    The storeys are the levels the document's own elements name, which is all
    the probe carries.
    """
    rows = []
    for doc in documents:
        elements = []
        for entity in ("floors", "rooms"):
            probe = doc["entities"].get(entity, {}).get("probe")
            if probe:
                elements.extend(probe.get("elements", []))
        levels = sorted(set(e.get("level_elevation_ft") for e in elements if e.get("level_elevation_ft") is not None))
        names = dict((e.get("level_elevation_ft"), e.get("level_name")) for e in elements)
        probe = doc["entities"].get("floors", {}).get("probe")
        if not probe or len(levels) < 2:
            continue
        for element in probe.get("elements", []):
            base = element.get("level_elevation_ft")
            offset = element.get("height_offset_ft")
            if base is None or offset is None or base not in levels:
                continue
            k = levels.index(base)
            top = base + offset
            verdict = None
            if k + 1 < len(levels) and top >= levels[k + 1] - LEVEL_EPS_FT:
                verdict = "at or above {}".format(names.get(levels[k + 1]))
            elif k > 0 and top <= (base + levels[k - 1]) / 2.0:
                verdict = "over half a storey below, toward {}".format(names.get(levels[k - 1]))
            if verdict:
                rows.append([doc["document"], element.get("id"), element.get("level_name"),
                             "{:.2f}".format(offset), verdict])
    return rows


# --------------------------------------------------------------------------
# F4 / F5 -- offsets, structural, phase
# --------------------------------------------------------------------------


def offset_rows(documents):
    bins = Counter()
    suspect = []
    for doc in documents:
        probe = doc["entities"].get("floors", {}).get("probe")
        if not probe:
            continue
        for element in probe.get("elements", []):
            offset = element.get("height_offset_ft")
            if offset is None:
                bins["(none)"] += 1
                continue
            magnitude = abs(offset)
            if magnitude < 0.01:
                bins["0"] += 1
            elif magnitude < 1.0:
                bins["< 1 ft"] += 1
            elif magnitude < SUSPECT_OFFSET_FT:
                bins["1-{:.0f} ft".format(SUSPECT_OFFSET_FT)] += 1
            else:
                bins[">= {:.0f} ft".format(SUSPECT_OFFSET_FT)] += 1
                suspect.append([doc["document"], element.get("id"), element.get("level_name"),
                                "{:.2f}".format(offset), element.get("is_structural")])
    return bins, suspect


def structure_phase_rows(documents):
    rows = []
    for doc in documents:
        probe = doc["entities"].get("floors", {}).get("probe")
        if not probe:
            continue
        counts = Counter()
        for element in probe.get("elements", []):
            structural = element.get("is_structural")
            counts["structural" if structural else ("not structural" if structural is False else "no flag")] += 1
            if element.get("phase_demolished"):
                counts["demolished"] += 1
            if element.get("is_in_place"):
                counts["in place"] += 1
        rows.append([doc["document"], counts["structural"], counts["not structural"],
                     counts["no flag"], counts["in place"], counts["demolished"]])
    return rows


# --------------------------------------------------------------------------
# F6 -- attribution under three rules
# --------------------------------------------------------------------------


def storey_key(fact):
    return fact.get("level_name"), fact.get("level_elevation_ft")


def same_storey(a, b):
    if a[0] != b[0] or a[1] is None or b[1] is None:
        return False
    return abs(a[1] - b[1]) <= LEVEL_EPS_FT


def attribution(documents):
    """Every floor/room pair on one storey with a real overlap, project-wide,
    judged by the ceiling rule, the floor rule and duHast's.

    PROJECT-wide, so F2's consequence shows as numbers: the `same document`
    column is what the server's model-scoped join can see, and every kept pair
    outside it is an attribution the server will not make.
    """
    floors = []
    rooms = []
    for doc in documents:
        for entity, sink in (("floors", floors), ("rooms", rooms)):
            block = doc["entities"].get(entity)
            if not block:
                continue
            probe = block["probe"]
            facts = {str(e.get("id")): e for e in probe.get("elements", [])}
            for element_id, element in reduce_export(block["raw"], probe.get("list_key")).items():
                fact = facts.get(element_id, {})
                if entity == "floors":
                    shape, _ = floor_shape(element)
                else:
                    shape = room_outline(element)
                if shape is None:
                    continue
                sink.append({
                    "id": element_id,
                    "document": doc["document"],
                    "storey": storey_key(fact),
                    "shape": shape,
                    "bounds": shape.bounds,
                    "structural": fact.get("is_structural"),
                })

    pairs = []
    for floor in floors:
        fx0, fy0, fx1, fy1 = floor["bounds"]
        for room in rooms:
            if not same_storey(floor["storey"], room["storey"]):
                continue
            rx0, ry0, rx1, ry1 = room["bounds"]
            if rx1 < fx0 or rx0 > fx1 or ry1 < fy0 or ry0 > fy1:
                continue
            try:
                overlap = floor["shape"].intersection(room["shape"])
            except Exception:
                continue
            area = overlap.area
            if area < MIN_OVERLAP_AREA:
                continue
            fraction_of_element = area / floor["shape"].area
            fraction_of_room = area / room["shape"].area if room["shape"].area else 0.0
            mean_width = 2.0 * area / overlap.length if overlap.length else 0.0
            pairs.append({
                "floor": floor["id"],
                "room": room["id"],
                "room_document": room["document"],
                "same_document": floor["document"] == room["document"],
                "structural": floor["structural"],
                "area": area,
                "fraction_of_element": fraction_of_element,
                "fraction_of_room": fraction_of_room,
                "mean_width": mean_width,
                "ceiling_rule": fraction_of_element >= MIN_FRACTION_OF_CEILING,
                "floor_rule": not (mean_width < MIN_FLOOR_MEAN_WIDTH_FT
                                   and fraction_of_room < MIN_ROOM_COVER_FOR_NARROW
                                   and fraction_of_element < MIN_FLOOR_COVER_FOR_NARROW),
                "duhast_rule": fraction_of_room * 100.0 >= DUHAST_PCT_OF_ROOM,
            })
    return pairs, floors, rooms


def width_histogram(pairs):
    """Mean width of the overlaps that do NOT cover most of their room -- the
    population the floor rule decides. The line at 1.5 ft is only a good line
    if it falls in a gap here."""
    counts = Counter()
    for pair in pairs:
        if pair["fraction_of_room"] >= MIN_ROOM_COVER_FOR_NARROW:
            continue
        width = pair["mean_width"]
        label = ">= {}".format(MEAN_WIDTH_BINS[-1])
        for edge in MEAN_WIDTH_BINS:
            if width < edge:
                label = "< {}".format(edge)
                break
        counts[label] += 1
    order = ["< {}".format(edge) for edge in MEAN_WIDTH_BINS] + [">= {}".format(MEAN_WIDTH_BINS[-1])]
    return [[label, counts[label]] for label in order]


# --------------------------------------------------------------------------
# Report
# --------------------------------------------------------------------------


def table(headers, rows):
    if not rows:
        return ["", "_(nothing to report)_", ""]
    widths = [len(str(h)) for h in headers]
    for row in rows:
        for i, cell in enumerate(row):
            widths[i] = max(widths[i], len(str(cell)))
    out = [
        "| " + " | ".join(str(h).ljust(widths[i]) for i, h in enumerate(headers)) + " |",
        "|" + "|".join("-" * (w + 2) for w in widths) + "|",
    ]
    for row in rows:
        out.append("| " + " | ".join(str(c).ljust(widths[i]) for i, c in enumerate(row)) + " |")
    return [""] + out + [""]


def pct(n, total):
    return "0%" if not total else "{:.1f}%".format(n * 100.0 / total)


def build_report(index, documents):
    lines = []
    add = lines.append
    extend = lines.extend

    add("# Floors probe report")
    add("")
    add("Probe version {}, {} document(s). shapely: {}.".format(
        index.get("probe_version"), len(documents), "yes" if HAVE_SHAPELY else "NO -- F3 partial, F6 skipped"))
    add("")

    duhast = index.get("duhast_self_check") or {}
    if duhast.get("point_in_polygon_fixed") is False:
        add("> **STALE duHast.** This run executed a duHast whose point-in-polygon test still has the "
            "`+2` quadrant bug (`{}`), fixed upstream 2026-09-13, so holes may be exported as separate "
            "polygons. A console that exec'd the probe keeps the old module loaded: restart Revit and "
            "re-run.".format(duhast.get("geometry_module")))
        add("")
    elif not duhast:
        add("_This run did not record which duHast it executed (probe predates the self-check)._")
        add("")

    add("## F1 -- does every floor export a usable polygon?")
    extend(table(
        ["document", "collected", "duHast collector", "exported", "exported EMPTY",
         "missing: in place", "missing: no solids", "missing: solids but dropped"],
        drop_rows(documents)))
    add("`exported EMPTY` is a floor duHast could not measure and still sent -- the "
        "state the contract reports. `solids but dropped` is a duHast defect; an "
        "older duHast on the extension path is the first suspect.")
    add("")

    add("## F2 -- are floors and rooms in the same document? (scope)")
    rows, verdicts = colocation_rows(documents)
    extend(table(["document", "floors", "of which structural", "rooms", "holds"], rows))
    if verdicts["FLOORS ONLY"]:
        add("**{} document(s) hold floors and no rooms.** The server attributes a floor "
            "only to rooms in its own document, so every floor there reports no room. "
            "F6's `cross-document` column is how much attribution that costs.".format(
                verdicts["FLOORS ONLY"]))
    else:
        add("No document holds floors without rooms: the model-scoped join can see every "
            "floor's rooms, as it could every ceiling's.")
    add("")

    add("## F3 -- is the exported footprint the floor Revit measures?")
    buckets, worst, invalid_by_curve = fidelity(documents)
    extend(table(["exported area against HOST_AREA_COMPUTED", "floors"], sorted(buckets.items())))
    if not HAVE_SHAPELY:
        add("Measured as the SUM of the pieces (no shapely), which a duplicated top/bottom "
            "face doubles -- read `OVER` with that in mind.")
    extend(table(["ratio", "document", "floor", "exported sqft", "Revit sqft"],
                 [["{:.3f}".format(r), d, f, "{:.1f}".format(m), "{:.1f}".format(h)]
                  for r, d, f, m, h in worst]))
    if HAVE_SHAPELY:
        add("Floors with at least one piece INVALID AS EXPORTED (before repair):")
        extend(table(["edges", "floors"], sorted(invalid_by_curve.items())))
        add("Invalid pieces concentrated on floors WITH ARCS point at duHast's "
            "`convert_edge_arrays_into_list_of_points`, which tessellates an edge in its "
            "own direction rather than the loop's; the fix is `face.GetEdgesAsCurveLoops()`. "
            "Invalid pieces on straight-edged floors point elsewhere.")
    add("")
    islands, crossing, lost, degenerate = geometry_faults(documents)
    add("**Holes exported as islands** -- the larger piece minus the nested ones is Revit's area:")
    extend(table(["document", "floor", "nested pieces", "exported outer sqft", "outer minus nested", "Revit sqft"],
                 islands))
    add("Each of these is filled back in by the union, so the floor covers its own opening. On House A "
        "both were duHast's `geometry.adjust_delta` returning `+2` without the x-intercept test "
        "(The Building Coder's original applies it to +2 and -2); corrected, duHast's own classifier "
        "gives Revit's area to the tenth of a square foot. Fixed upstream 2026-09-13: a probe run on a "
        "duHast with the fix should list nothing here.")
    add("")
    add("Floors with a properly CROSSING ring (stdlib, consecutive duplicate vertices ignored):")
    extend(table(["edges", "floors"], sorted(crossing.items())))
    add("{} degenerate ring(s) of under three distinct points -- slivers off faces admitted as "
        "horizontal by `FaceNormal.Z != 0.0`.".format(degenerate))
    add("")
    add("**Faces lost** -- even the SUM of the pieces, which can only overstate the union, is under "
        "90% of Revit's area:")
    extend(table(["document", "floor", "pieces", "horizontal faces", "sum sqft", "Revit sqft"], lost))
    add("")

    add("## F4 -- where do floors sit relative to their level?")
    bins, suspect = offset_rows(documents)
    extend(table(["|height_offset|", "floors"], sorted(bins.items())))
    extend(table(["document", "floor", "level", "offset ft", "structural"], suspect[:30]))
    add("A floor at a storey-sized offset is still joined to its HOST level's rooms -- by decision.")
    add("")
    add("Exported level against the host Level Revit states (the storey IS the host level):")
    mismatches = host_level_mismatches(documents)
    if mismatches:
        extend(table(["document", "floor", "exported level", "host level"], mismatches))
    else:
        add("")
        add("Every exported floor carries its host level.")
        add("")
    add("Floors whose TOP is clearly on another storey of their own document -- at or above the next "
        "level, or more than half a storey below their own. Reported, not reassigned: they are joined "
        "to their host level's rooms, and `height_offset` is how a consumer tells them apart:")
    extend(table(["document", "floor", "host level", "offset ft", "top is"], storey_of_top(documents)))
    add("")

    add("## F5 -- structural, in-place and demolished")
    extend(table(["document", "structural", "not structural", "no flag", "in place", "demolished"],
                 structure_phase_rows(documents)))
    add("")

    add("## F6 -- does the floor sliver rule hold?")
    if not HAVE_SHAPELY:
        add("_Skipped: needs shapely. Nothing here is estimated._")
        return "\n".join(lines) + "\n"
    pairs, floors, rooms = attribution(documents)
    add("{} floors and {} rooms with geometry; {} pairs overlap by at least {} sqft on one storey.".format(
        len(floors), len(rooms), len(pairs), MIN_OVERLAP_AREA))
    add("")
    kept = Counter()
    for pair in pairs:
        for rule in ("ceiling_rule", "floor_rule", "duhast_rule"):
            if pair[rule]:
                kept[(rule, pair["same_document"])] += 1
    extend(table(
        ["rule", "kept, same document", "kept, cross-document (server cannot see)"],
        [[rule, kept[(rule, True)], kept[(rule, False)]]
         for rule in ("floor_rule", "ceiling_rule", "duhast_rule")]))

    floor_not_ceiling = [p for p in pairs if p["floor_rule"] and not p["ceiling_rule"]]
    ceiling_not_floor = [p for p in pairs if p["ceiling_rule"] and not p["floor_rule"]]
    add("**Kept by the floor rule, dropped by the ceiling rule: {}** -- rooms that are a "
        "small fraction of a large floor. {} of them are on structural floors.".format(
            len(floor_not_ceiling), sum(1 for p in floor_not_ceiling if p["structural"])))
    add("")
    add("**Kept by the ceiling rule, dropped by the floor rule: {}** -- narrow overlaps. "
        "The largest are listed; each should be a strip under a wall, and any that is "
        "not is a case against the rule.".format(len(ceiling_not_floor)))
    ceiling_not_floor.sort(key=lambda p: p["area"], reverse=True)
    extend(table(
        ["floor", "room", "sqft", "mean width ft", "% of room", "% of floor"],
        [[p["floor"], p["room"], "{:.2f}".format(p["area"]), "{:.2f}".format(p["mean_width"]),
          "{:.1f}".format(p["fraction_of_room"] * 100), "{:.2f}".format(p["fraction_of_element"] * 100)]
         for p in ceiling_not_floor[:20]]))
    add("Mean width of overlaps covering under half their room -- the population the "
        "floor rule decides. The {} ft line is a good one only if it sits in a gap:".format(
            MIN_FLOOR_MEAN_WIDTH_FT))
    extend(table(["mean width ft", "pairs"], width_histogram(pairs)))

    # Keyed with the document: a room id is unique only within its model.
    rooms_with_floor = set((p["room_document"], p["room"]) for p in pairs if p["floor_rule"] and p["same_document"])
    add("Rooms with at least one floor under the server's rule: {} of {} ({}).".format(
        len(rooms_with_floor), len(rooms), pct(len(rooms_with_floor), len(rooms))))
    return "\n".join(lines) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--dir", default=DEFAULT_DIR, help="where the probe fixtures are")
    args = parser.parse_args(argv)

    index, documents = load_inputs(args.dir)
    report = build_report(index, documents)
    out_path = os.path.join(args.dir, "floors-probe-report.md")
    # Binary write, LF -- the `.gitattributes` trap CLAUDE.md records.
    with open(out_path, "wb") as handle:
        handle.write(report.encode("utf-8"))
    print(report)
    print("wrote {}".format(out_path))


if __name__ == "__main__":
    main()
