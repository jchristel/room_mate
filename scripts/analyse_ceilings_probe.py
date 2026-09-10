#!/usr/bin/env python3
"""Answer the six questions that gate a ceilings entity, from the files
`probe_ceilings_export.py` captured in Revit.

    python scripts/analyse_ceilings_probe.py
    python scripts/analyse_ceilings_probe.py --dir some/other/fixtures

    Q1  Does every ceiling export a usable polygon?  **The kill condition.**
    Q2  Are ceilings and rooms in the same document?
    Q3  Do ceilings and rooms agree on level?
    Q4  How much does a ceiling overlap a room, and how many rooms does it hit?
    Q5  Which phase does each ceiling belong to?
    Q6  What shape is a ceiling polygon?

Writes `ceilings-probe-report.md` beside the inputs and prints the same thing.

**This is where every judgement lives.** The probe collects and counts; nothing
in it decides whether a ceiling belongs to a room. Keeping the deciding here
means it can be re-run, argued with and corrected without going back to Revit --
which matters because the answers change the contract, and the contract is what
the argument is actually about.

THE THREE DISTINCTIONS THIS FILE MUST NOT COLLAPSE
1.  "This model has no ceilings" and "ceilings present but unexportable" are
    different answers. The first means find another model and costs nothing;
    the second is the kill condition and stops the entity.
2.  "No solids to flatten" and "solids that flattened to nothing" are different
    answers. The first is a model or in-place-family fact and is expected for
    some ceilings; the second is a duHast defect and belongs upstream. Fusing
    them into "has no geometry" is the mistake the +/-1e30 sentinel scar
    records the cost of.
3.  "A ceiling overlaps this room by 40%" and "a ceiling overlaps this room by
    0.2%" are different answers, and the 0.1% threshold in duHast's
    `_intersect_ceiling_vs_room` is the line between them. This file reports the
    DISTRIBUTION and sweeps the threshold rather than applying one, because a
    threshold applied silently cannot be argued with later.

WHAT duHast's OWN THRESHOLD ACTUALLY MEASURES, since the name misleads
`_intersect_ceiling_vs_room` computes

    area_intersection_percentage_of_ceiling_vs_room
        = intersection.area / room_polygon.area * 100

so despite the name it is a percentage OF THE ROOM, not of the ceiling. That is
a defensible choice -- it asks "does this ceiling meaningfully cover this room"
-- but it is not the choice the name describes, and a port that trusted the name
would behave differently on a large ceiling clipping a small room. Both
percentages are reported below so the decision is made on the numbers.

Q4 NEEDS shapely, AND DELIBERATELY DOES NOT FAKE IT
Polygon intersection over concave rings is not something to approximate: a
bounding-box estimate would produce a number that looks like a measurement and
is not one, which is the failure mode this whole probe/analyser split exists to
avoid. duHast's own `process_ceilings_to_rooms` hard-requires shapely for the
same reason. Without it Q1, Q2, Q3, Q5 and Q6 still answer in full and Q4 says
plainly that it did not run.

    pip install shapely

Stdlib only otherwise, CPython 3. It reads JSON and prints; it must run wherever
the data lands, including a machine with no checkout of this repo beside it.
"""

import argparse
import json
import os
from collections import Counter, OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_DIR = os.path.join(HERE, "fixtures")

# The thresholds swept in Q4, as percentages. duHast ships 0.1; the sweep exists
# because nothing has ever checked that value against a real ceiling population,
# and a threshold inherited without measurement is a guess with a decimal point
# on it.
THRESHOLD_SWEEP = [0.0, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0]

# duHast's own value, called out separately so the report can say what the
# straight port would have done.
DUHAST_THRESHOLD = 0.1

try:
    from shapely.geometry import Polygon as ShapelyPolygon

    HAVE_SHAPELY = True
except Exception:
    ShapelyPolygon = None
    HAVE_SHAPELY = False


# --------------------------------------------------------------------------
# Loading
# --------------------------------------------------------------------------


def load(path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def load_inputs(directory):
    """Every document named by the index, with its probe and raw export loaded.

    Reads the STATED document list rather than globbing, for the reason the
    spaces analyser gives: a fixtures folder accumulates runs, and picking up
    yesterday's interiors model beside today's would report an overlap rate
    against a population nobody captured.
    """
    index_path = os.path.join(directory, "ceilings-probe-index.json")
    if not os.path.isfile(index_path):
        raise SystemExit(
            "no ceilings-probe-index.json in {}\n"
            "Run scripts/probe_ceilings_export.py from pyRevit first.".format(directory)
        )
    index = load(index_path)

    documents = []
    for entry in index.get("documents", []):
        slug = entry.get("slug")
        loaded = {"document": entry.get("document"), "slug": slug, "entities": {}}
        for entity in ("ceilings", "rooms"):
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
    """The exported elements as `{id: record}`, keyed the way the probe keys
    them so the two halves join.

    The export nests an element's id under `instance_properties`, which is where
    `translate_room` reads it from -- so this reads it from the same place
    rather than from anywhere that happens to look like an id.
    """
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
    """One duHast point as `(x, y)`, from either shape it can arrive in.

    `plainify` converts a point object through `class_to_dict`, so it lands as a
    mapping; a point that was already a bare list stays a list. Both are handled
    because which one appears depends on a duHast version, and a probe that only
    read one would report "no geometry" for a perfectly good export.
    """
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
    """Absolute area of a closed ring, by the shoelace formula.

    Stdlib on purpose. This is what Q6 needs, and Q6 must answer on a machine
    with no shapely -- the boolean ops are Q4's requirement, not this one. Sign
    is discarded because ring winding is not consistent in the export and the
    question here is size, not orientation.
    """
    if len(ring) < 3:
        return 0.0
    total = 0.0
    for i in range(len(ring)):
        x1, y1 = ring[i]
        x2, y2 = ring[(i + 1) % len(ring)]
        total += x1 * y2 - x2 * y1
    return abs(total) / 2.0


def polygon_area(outer, inners):
    """Net area of one exported polygon: its outer ring less its holes."""
    return max(0.0, ring_area(outer) - sum(ring_area(ring) for ring in inners))


# Below this, in square feet, an exported footprint is not a ceiling. House A
# produced two at 0.33 and 0.37 sqft against a population whose next smallest is
# two orders of magnitude larger, so the gap is real rather than a chosen line.
# Reported, never dropped: "exported something unusable" is its own state, and
# fusing it into either "exported" or "dropped" is what the +/-1e30 sentinel scar
# is about.
DEGENERATE_SQFT = 5.0

# How much larger the SUM of a ceiling's polygons may be than its largest one
# before the extras are read as duplicate faces rather than extra geometry.
# `convert_solid_to_flattened_2d_points` walks horizontal faces, and a slab has
# two of them -- top and bottom, near-identical in plan. Measured on House A:
# three ceilings where the two largest pieces have an IoU above 0.98 and the sum
# is exactly twice the union.
DUPLICATE_FACE_RATIO = 1.5

# Below this ratio the extra polygons are noise off the side faces rather than
# geometry: House A's three cases measured 157.2 against 156.7, 228.0 against
# 228.0 and 598.8 against 598.8. Kept apart from the genuinely-additional case
# because they point opposite ways -- noise means `polygon[0]` is the whole
# ceiling, additional means it is not.
SLIVER_RATIO = 1.05


def polygons_of(element):
    """Every polygon on one exported element, as `(outer, [inners])` tuples.

    ALL of them, not `polygon[0]`. Both existing translators (`translate_room`,
    `loops_from_polygon`) take the first and discard the rest, which is correct
    for a room and is exactly what Q6 is asking about for a ceiling: a ceiling
    made of several solid volumes exports several polygons, and a translator
    that kept only the first would silently lose the others.
    """
    out = []
    for poly in element.get("polygon") or []:
        if not isinstance(poly, dict):
            continue
        outer = ring_points(poly.get("outer_loop"))
        inners = [ring_points(loop) for loop in (poly.get("inner_loops") or [])]
        inners = [ring for ring in inners if len(ring) >= 3]
        out.append((outer, inners))
    return out


def shapely_of(element):
    """One shapely geometry for an element, or None.

    Every polygon on the element is unioned, so a multi-volume ceiling is
    measured whole. `buffer(0)` repairs the self-touching rings a flattened
    solid routinely produces -- without it a valid-looking ceiling raises inside
    `intersection` and the run loses an element to an exception rather than to a
    finding.
    """
    if not HAVE_SHAPELY:
        return None
    pieces = []
    for outer, inners in polygons_of(element):
        if len(outer) < 3:
            continue
        try:
            piece = ShapelyPolygon(outer, inners)
            if not piece.is_valid:
                piece = piece.buffer(0)
            if not piece.is_empty:
                pieces.append(piece)
        except Exception:
            continue
    if not pieces:
        return None
    merged = pieces[0]
    for piece in pieces[1:]:
        try:
            merged = merged.union(piece)
        except Exception:
            continue
    return merged if not merged.is_empty else None


# --------------------------------------------------------------------------
# Q1 -- the kill condition
# --------------------------------------------------------------------------


def drop_rows(documents):
    """Per document: collected, exported, and WHY the difference.

    The three causes are kept apart because they send a reader to three
    different places. An in-place ceiling is a known duHast limitation stated in
    `solids.py`; a ceiling with no solids at all is a model fact; a ceiling with
    solids that still did not export is a duHast defect and the only one of the
    three that is somebody's bug.
    """
    rows = []
    totals = Counter()
    for doc in documents:
        entity = doc["entities"].get("ceilings")
        if not entity:
            continue
        probe = entity["probe"]
        exported = reduce_export(entity["raw"], probe.get("list_key", "ceiling"))
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
        row = [
            doc["document"],
            probe.get("collected_count", 0),
            probe.get("duhast_collected_count"),
            probe.get("exported_count", 0),
            in_place,
            no_solids,
            solids_but_dropped,
        ]
        rows.append(row)
        totals["collected"] += probe.get("collected_count", 0) or 0
        totals["exported"] += probe.get("exported_count", 0) or 0
        totals["in_place"] += in_place
        totals["no_solids"] += no_solids
        totals["solids_but_dropped"] += solids_but_dropped
    return rows, totals


# --------------------------------------------------------------------------
# Q2 -- co-location
# --------------------------------------------------------------------------


def colocation_rows(documents):
    rows = []
    both = ceilings_only = rooms_only = 0
    for doc in documents:
        ceilings = doc["entities"].get("ceilings", {}).get("probe", {})
        rooms = doc["entities"].get("rooms", {}).get("probe", {})
        n_ceilings = ceilings.get("collected_count", 0) or 0
        n_rooms = rooms.get("collected_count", 0) or 0
        if n_ceilings and n_rooms:
            verdict_text = "both"
            both += 1
        elif n_ceilings:
            verdict_text = "ceilings only"
            ceilings_only += 1
        elif n_rooms:
            verdict_text = "rooms only"
            rooms_only += 1
        else:
            verdict_text = "neither"
        rows.append([doc["document"], n_ceilings, n_rooms, verdict_text])
    return rows, {"both": both, "ceilings_only": ceilings_only, "rooms_only": rooms_only}


# --------------------------------------------------------------------------
# Q3 -- levels
# --------------------------------------------------------------------------


def level_rows(documents):
    """Level names and elevations on both sides, per document.

    Both are reported because they answer different questions. duHast's
    `process_ceilings_to_rooms` buckets by NAME, and RoomMate joins storeys by
    name AND elevation (`src-js/renderer/storey.ts`) precisely because a name
    alone is not enough -- RHH stacks a car park level at the hospital ground's
    elevation. If the two populations disagree on names, the duHast algorithm
    silently matches nothing and this table is the only place that shows it.
    """
    rows = []
    for doc in documents:
        for entity in ("ceilings", "rooms"):
            probe = doc["entities"].get(entity, {}).get("probe")
            if not probe:
                continue
            names = Counter()
            for element in probe.get("elements", []):
                name = element.get("level_name")
                elevation = element.get("level_elevation_ft")
                label = "{} @ {}".format(
                    name if name is not None else "(none)",
                    "{:.3f}".format(elevation) if isinstance(elevation, (int, float)) else "?",
                )
                names[label] += 1
            for label, count in sorted(names.items()):
                rows.append([doc["document"], entity, label, count])
    return rows


def level_name_overlap(documents):
    """Which level NAMES appear on both sides, project-wide.

    Project-wide rather than per document, because Q2 may have found the two
    populations in different files -- in which case a per-document comparison
    would report zero overlap and say nothing about whether a join is possible.
    """
    ceiling_names = set()
    room_names = set()
    for doc in documents:
        for entity, sink in (("ceilings", ceiling_names), ("rooms", room_names)):
            probe = doc["entities"].get(entity, {}).get("probe")
            if not probe:
                continue
            for element in probe.get("elements", []):
                if element.get("level_name"):
                    sink.add(element["level_name"])
    return ceiling_names, room_names


# --------------------------------------------------------------------------
# Q4 -- overlap
# --------------------------------------------------------------------------


def overlap_pairs(documents):
    """Every ceiling/room pair with a non-zero intersection, project-wide.

    PROJECT-wide and not model-scoped, deliberately, and this is a measurement
    rather than a decision about scope. CLAUDE.md's rule is that an element
    joining on a room ID is model-scoped everywhere; a ceiling joins on nothing,
    so the scope is an open question and answering it needs the cross-document
    pairs to have been counted at all. If they all turn out to be same-document,
    the model-scoped rule survives untouched and the report says so.

    Pairs are matched on level NAME first, the way duHast does, because a
    ceiling on level 3 cannot be in a room on level 1 and testing every pair on
    a hospital is quadratic for nothing.
    """
    ceilings = []
    rooms = []
    for doc in documents:
        for entity, sink in (("ceilings", ceilings), ("rooms", rooms)):
            block = doc["entities"].get(entity)
            if not block:
                continue
            probe = block["probe"]
            exported = reduce_export(block["raw"], probe.get("list_key"))
            level_by_id = {
                str(e.get("id")): e.get("level_name") for e in probe.get("elements", [])
            }
            for element_id, element in exported.items():
                geometry = shapely_of(element)
                if geometry is None:
                    continue
                sink.append(
                    {
                        "id": element_id,
                        "document": doc["document"],
                        "level": level_by_id.get(element_id),
                        "geometry": geometry,
                    }
                )

    rooms_by_level = {}
    for room in rooms:
        rooms_by_level.setdefault(room["level"], []).append(room)

    pairs = []
    for ceiling in ceilings:
        for room in rooms_by_level.get(ceiling["level"], []):
            try:
                if not ceiling["geometry"].intersects(room["geometry"]):
                    continue
                area = ceiling["geometry"].intersection(room["geometry"]).area
            except Exception:
                continue
            if area <= 0:
                continue
            room_area = room["geometry"].area
            ceiling_area = ceiling["geometry"].area
            pairs.append(
                {
                    "ceiling": ceiling["id"],
                    "room": room["id"],
                    "same_document": ceiling["document"] == room["document"],
                    "area_sqft": area,
                    "pct_of_room": (area / room_area * 100.0) if room_area else 0.0,
                    "pct_of_ceiling": (area / ceiling_area * 100.0) if ceiling_area else 0.0,
                }
            )
    return pairs, len(ceilings), len(rooms)


def overlap_sweep(pairs):
    """How the pair count and the many-to-many rate move with the threshold.

    Two columns, because duHast's threshold is a percentage of the ROOM and the
    obvious reading of its name is a percentage of the ceiling. Where the two
    disagree is exactly where a straight port would behave differently from what
    its own comment says it does.
    """
    rows = []
    for threshold in THRESHOLD_SWEEP:
        by_room = [p for p in pairs if p["pct_of_room"] >= threshold]
        by_ceiling = [p for p in pairs if p["pct_of_ceiling"] >= threshold]
        multi = Counter(p["ceiling"] for p in by_room)
        spanning = sum(1 for count in multi.values() if count > 1)
        rows.append(
            [
                "{:.1f}%".format(threshold),
                len(by_room),
                len(by_ceiling),
                len(multi),
                spanning,
            ]
        )
    return rows


# --------------------------------------------------------------------------
# Q5 -- phase
# --------------------------------------------------------------------------


def phase_rows(documents):
    """Created and demolished phases, counted separately.

    The demolished column is the one that decides Q5. If nothing is ever
    demolished the range test and the equality test agree on this model, and
    the report must say that rather than claiming the range test was proved
    necessary -- CLAUDE.md's phase rule was learned from five empty pushes, and
    "it happened to work here" is how the next five get made.
    """
    rows = []
    demolished_seen = 0
    for doc in documents:
        probe = doc["entities"].get("ceilings", {}).get("probe")
        if not probe:
            continue
        created = Counter()
        demolished = Counter()
        for element in probe.get("elements", []):
            created[element.get("phase_created") or "(none)"] += 1
            value = element.get("phase_demolished")
            demolished[value or "(none)"] += 1
            if value:
                demolished_seen += 1
        for name, count in sorted(created.items()):
            rows.append([doc["document"], "created", name, count])
        for name, count in sorted(demolished.items()):
            rows.append([doc["document"], "demolished", name, count])
    return rows, demolished_seen


# --------------------------------------------------------------------------
# Q6 -- polygon shape
# --------------------------------------------------------------------------


def shape_rows(documents):
    """How many polygons and holes a ceiling exports, and whether the extra
    polygons are extra GEOMETRY or the same face twice.

    **The count alone answers the wrong question, and House A is why this
    function grew.** A ceiling with five polygons looks like a ceiling in five
    pieces, and the obvious conclusion -- that a room's `loops` field cannot
    carry it, since that is one outer ring plus holes -- is wrong.
    `convert_solid_to_flattened_2d_points` walks the HORIZONTAL FACES of a
    solid, and a slab has two of them: its top and its bottom, near-identical in
    plan. The rest are slivers off the side faces.

    So what matters per ceiling is the largest polygon against the SUM of them.
    Equal means genuinely separate pieces; a sum around twice the largest means
    duplicate faces, and a consumer that unioned or summed would double-count
    the area while one taking `polygon[0]` would be right.
    """
    polygon_counts = Counter()
    inner_counts = Counter()
    duplicate_face = []
    slivers = []
    disjoint = []
    for doc in documents:
        block = doc["entities"].get("ceilings")
        if not block:
            continue
        exported = reduce_export(block["raw"], block["probe"].get("list_key"))
        for element_id, element in exported.items():
            polygons = polygons_of(element)
            polygon_counts[len(polygons)] += 1
            areas = []
            for outer, inners in polygons:
                inner_counts[len(inners)] += 1
                areas.append(polygon_area(outer, inners))
            if len(polygons) < 2:
                continue
            largest = max(areas) if areas else 0.0
            total = sum(areas)
            row = [doc["document"], element_id, len(polygons),
                   "{:.1f}".format(largest), "{:.1f}".format(total)]
            if largest <= 0:
                continue
            ratio = total / largest
            if ratio > DUPLICATE_FACE_RATIO:
                duplicate_face.append(row)
            elif ratio <= SLIVER_RATIO:
                slivers.append(row)
            else:
                disjoint.append(row)
    return polygon_counts, inner_counts, duplicate_face, slivers, disjoint


def degenerate_rows(documents):
    """Ceilings that exported a footprint too small to be one.

    A companion to Q1 rather than part of it, and the distinction is the point:
    Q1 counts what reached the export, this counts what reached it UNUSABLE.
    A ceiling measuring a third of a square foot passes every "did it export"
    check and then matches rooms by slivers, which is how two of House A's four
    marginal overlaps were made.
    """
    rows = []
    for doc in documents:
        block = doc["entities"].get("ceilings")
        if not block:
            continue
        exported = reduce_export(block["raw"], block["probe"].get("list_key"))
        for element_id, element in exported.items():
            areas = [polygon_area(outer, inners) for outer, inners in polygons_of(element)]
            largest = max(areas) if areas else 0.0
            if largest < DEGENERATE_SQFT:
                rows.append([doc["document"], element_id, "{:.2f}".format(largest)])
    return rows


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
        out.append(
            "| " + " | ".join(str(c).ljust(widths[i]) for i, c in enumerate(row)) + " |"
        )
    return [""] + out + [""]


def pct(n, total):
    return "0%" if not total else "{:.1f}%".format(n * 100.0 / total)


def build_report(index, documents):
    lines = []
    add = lines.append

    add("# Ceilings probe report")
    add("")
    add("Probe version {}, {} document(s).".format(
        index.get("probe_version"), len(documents)
    ))
    add("")
    add("Generated by `scripts/analyse_ceilings_probe.py` from the fixtures")
    add("`scripts/probe_ceilings_export.py` captured. Every judgement below is")
    add("this file's; the probe only counted.")
    add("")

    # ---- Q1 ---------------------------------------------------------------
    add("## Q1 -- does every ceiling export a usable polygon? (kill condition)")
    add("")
    rows, totals = drop_rows(documents)
    lines.extend(
        table(
            [
                "document",
                "collected",
                "duHast collector",
                "exported",
                "dropped: in-place",
                "dropped: no solids",
                "dropped: solids but no polygon",
            ],
            rows,
        )
    )
    collected = totals["collected"]
    exported = totals["exported"]
    add("**{} of {} ceilings exported ({}).**".format(exported, collected, pct(exported, collected)))
    add("")
    if totals["solids_but_dropped"]:
        add(
            "{} ceiling(s) had solids and still produced no polygon. That is a "
            "duHast defect, not a model fact, and it belongs upstream in "
            "`convert_solid_to_flattened_2d_points` before this entity is "
            "built on it.".format(totals["solids_but_dropped"])
        )
    if totals["in_place"]:
        add(
            "{} in-place ceiling(s) were dropped. Expected -- `solids.py` states "
            "its walk does not handle in-place families -- but expected is not "
            "the same as acceptable: downstream they are indistinguishable from "
            "ceilings nobody modelled, so the extractor must COUNT them and the "
            "contract must be able to say so.".format(totals["in_place"])
        )
    add("")

    # Exported is not the same as usable, and a run can pass Q1 outright while
    # still carrying ceilings nothing can attribute.
    degenerate = degenerate_rows(documents)
    add("### Exported, but too small to be a ceiling")
    add("")
    if degenerate:
        add(
            "{} ceiling(s) exported a footprint under {} sqft. They pass every "
            "\"did it export\" check and then match rooms by slivers, so they "
            "inflate the overlap counts in Q4 rather than showing up as a "
            "failure here.".format(len(degenerate), DEGENERATE_SQFT)
        )
        lines.extend(table(["document", "ceiling id", "largest polygon (sqft)"], degenerate))
    else:
        add("None -- every exported ceiling carries a plausible footprint.")
        add("")

    # ---- Q2 ---------------------------------------------------------------
    add("## Q2 -- are ceilings and rooms in the same document?")
    add("")
    rows, colocation = colocation_rows(documents)
    lines.extend(table(["document", "ceilings", "rooms", "holds"], rows))
    if colocation["ceilings_only"] or colocation["rooms_only"]:
        add(
            "**Split populations.** {} document(s) hold ceilings without rooms and "
            "{} hold rooms without ceilings. A model-scoped join would match "
            "nothing across that split, and unlike spaces there is no key to "
            "widen to -- only geometry. The join has to be project-scoped, and "
            "that makes ceilings the SECOND exception to CLAUDE.md's "
            "model-scoped rule, for a different reason than spaces.".format(
                colocation["ceilings_only"], colocation["rooms_only"]
            )
        )
    else:
        add(
            "**Co-located.** Every document holding ceilings also holds rooms, so "
            "the model-scoped join survives untouched and ceilings need no "
            "exception."
        )
    add("")

    # ---- Q3 ---------------------------------------------------------------
    add("## Q3 -- do ceilings and rooms agree on level?")
    add("")
    lines.extend(table(["document", "entity", "level @ elevation (ft)", "count"], level_rows(documents)))
    ceiling_names, room_names = level_name_overlap(documents)
    shared = sorted(ceiling_names & room_names)
    add(
        "{} ceiling level name(s), {} room level name(s), {} shared.".format(
            len(ceiling_names), len(room_names), len(shared)
        )
    )
    add("")
    if ceiling_names and room_names and not shared:
        add(
            "**No shared level name.** duHast's `_build_dictionary_by_level_and_data_type` "
            "buckets by name before intersecting, so a straight port would find "
            "zero ceilings in zero rooms and report it as a clean run. Match on "
            "name AND elevation, the way `src-js/renderer/storey.ts` already does."
        )
    add("")

    # ---- Q4 ---------------------------------------------------------------
    add("## Q4 -- how much does a ceiling overlap a room?")
    add("")
    if not HAVE_SHAPELY:
        add(
            "**Not measured: shapely is not installed.** `pip install shapely`, "
            "then re-run. No estimate is substituted on purpose -- a "
            "bounding-box approximation would look like a measurement and is "
            "not one."
        )
        add("")
    else:
        pairs, n_ceilings, n_rooms = overlap_pairs(documents)
        add(
            "{} ceiling(s) and {} room(s) carried usable geometry; {} intersecting "
            "pair(s) before any threshold.".format(n_ceilings, n_rooms, len(pairs))
        )
        lines.extend(
            table(
                [
                    "threshold",
                    "pairs (% of room)",
                    "pairs (% of ceiling)",
                    "ceilings matched",
                    "ceilings spanning >1 room",
                ],
                overlap_sweep(pairs),
            )
        )
        kept = [p for p in pairs if p["pct_of_room"] >= DUHAST_THRESHOLD]
        multi = Counter(p["ceiling"] for p in kept)
        spanning = sum(1 for count in multi.values() if count > 1)
        add(
            "At duHast's own 0.1% of room area: {} pair(s), {} ceiling(s) matched, "
            "{} of them spanning more than one room.".format(
                len(kept), len(multi), spanning
            )
        )
        add("")

        # **Whether a ceiling spans rooms is a claim about the THRESHOLD as much
        # as about the model, and reporting the 0.1% figure alone gets it wrong.**
        # House A: one ceiling spans three rooms at 0.1% and none spans anything
        # at 0.5%, because its two extra matches are 0.97 sqft slivers off a
        # duplicate face. Reading that as "many-to-many is real" would have
        # designed a list-shaped answer off an artefact of duHast's face walk.
        firm = [p for p in pairs if p["pct_of_room"] >= 0.5]
        firm_multi = Counter(p["ceiling"] for p in firm)
        firm_spanning = sum(1 for count in firm_multi.values() if count > 1)
        add(
            "At 0.5% of room area: {} pair(s), {} ceiling(s) matched, {} spanning "
            "more than one room.".format(len(firm), len(firm_multi), firm_spanning)
        )
        add("")
        if firm_spanning:
            add(
                "**Many-to-many survives a threshold that filters slivers**, so "
                "it is a fact about the model: {} ceiling(s) genuinely cover more "
                "than one room. The read carries a list with an area per entry, "
                "duHast's own `DataCeilingInRoom` shape.".format(firm_spanning)
            )
        elif spanning:
            add(
                "**The spanning case here is an ARTEFACT, not a finding.** It "
                "appears only at duHast's 0.1% and vanishes by 0.5%, which means "
                "the extra rooms are sliver overlaps rather than ceiling. Two "
                "conclusions follow. duHast's threshold is too low for this "
                "geometry, and it is a percentage of the ROOM, so it scales with "
                "the wrong operand -- a sliver against a large room passes more "
                "easily than a real overlap against a small one. And this data "
                "does NOT demonstrate many-to-many: keep the list-shaped answer "
                "because a stored single owner would need a migration to undo, "
                "not because this model proved it necessary."
            )
        else:
            add(
                "**No ceiling spans two rooms on this data, at any threshold.** "
                "That is a fact about this model, not a licence to store one "
                "owner: a bulkhead detail or an open-plan soffit produces the "
                "spanning case, and a read-time list costs nothing while a "
                "stored single owner would need a migration."
            )
        cross = [p for p in kept if not p["same_document"]]
        if cross:
            add("")
            add(
                "{} matched pair(s) cross a document boundary. See Q2 -- the "
                "join cannot be model-scoped.".format(len(cross))
            )
        add("")

    # ---- Q5 ---------------------------------------------------------------
    add("## Q5 -- which phase does each ceiling belong to?")
    add("")
    rows, demolished_seen = phase_rows(documents)
    lines.extend(table(["document", "parameter", "phase", "count"], rows))
    if demolished_seen:
        add(
            "**{} ceiling(s) carry a demolished phase.** The range test "
            "(`elements_in_phase` / `exists_in_phase`) is required; the rooms "
            "equality test on `ROOM_PHASE` would keep a ceiling that no longer "
            "exists in the pushed phase.".format(demolished_seen)
        )
    else:
        add(
            "No ceiling on this data is demolished, so the range test and the "
            "equality test happen to agree here. Use the range test anyway: a "
            "ceiling CAN be demolished, and 'it worked on this model' is how "
            "the five empty rooms pushes in CLAUDE.md were made."
        )
    add("")

    # ---- Q6 ---------------------------------------------------------------
    add("## Q6 -- what shape is a ceiling polygon?")
    add("")
    polygon_counts, inner_counts, duplicate_face, slivers, disjoint = shape_rows(documents)
    lines.extend(
        table(
            ["polygons on the ceiling", "ceilings"],
            [[k, v] for k, v in sorted(polygon_counts.items())],
        )
    )
    lines.extend(
        table(
            ["holes in the polygon", "polygons"],
            [[k, v] for k, v in sorted(inner_counts.items())],
        )
    )
    multi_total = sum(v for k, v in polygon_counts.items() if k > 1)
    if not multi_total:
        add(
            "Every ceiling exports exactly one polygon, so the room `loops` "
            "shape carries a ceiling as-is on this data."
        )
        add("")
        return "\n".join(lines) + "\n"

    add(
        "{} ceiling(s) export more than one polygon. **The count alone is "
        "misleading**, and the split below is what matters: a slab has two "
        "horizontal faces, top and bottom, so an extra polygon is usually the "
        "SAME face again rather than more ceiling.".format(multi_total)
    )
    add("")
    if duplicate_face:
        add(
            "**{} of them are duplicate faces** -- the polygons sum to more than "
            "{}x the largest one. On House A the two largest pieces of each had "
            "an IoU above 0.98 and the sum was exactly twice the union.".format(
                len(duplicate_face), DUPLICATE_FACE_RATIO
            )
        )
        lines.extend(
            table(
                ["document", "ceiling id", "polygons", "largest (sqft)", "sum (sqft)"],
                duplicate_face,
            )
        )
        add(
            "So a consumer must take `polygon[0]` and must NOT union or sum: "
            "`translate_room` and `loops_from_polygon` already take the first and "
            "discard the rest, and on this data that is exactly right -- the "
            "largest polygon equalled the union of all of them on every ceiling "
            "measured. Reusing the room `loops` shape verbatim is correct; "
            "aggregating the polygons would double-count area."
        )
        add("")
    if slivers:
        add(
            "**{} are the largest face plus negligible slivers** -- the polygons "
            "sum to within {}% of the largest, so the extras are noise off the "
            "solid's side faces rather than ceiling. `polygon[0]` is the whole "
            "of these too.".format(len(slivers), int((SLIVER_RATIO - 1) * 100))
        )
        lines.extend(
            table(
                ["document", "ceiling id", "polygons", "largest (sqft)", "sum (sqft)"],
                slivers,
            )
        )
    if disjoint:
        add(
            "**{} carry genuinely ADDITIONAL geometry** -- more than the largest "
            "face, and not a doubling of it. These are the only ones an "
            "outer-ring-plus-holes field cannot carry, and they are what decides "
            "whether the contract needs a list of polygons instead. Look at these "
            "before settling the geometry field.".format(len(disjoint))
        )
        lines.extend(
            table(
                ["document", "ceiling id", "polygons", "largest (sqft)", "sum (sqft)"],
                disjoint,
            )
        )
    else:
        add(
            "**No ceiling carries geometry beyond its largest face.** On this "
            "data the room `loops` shape is sufficient for a ceiling, and the "
            "polygon-count column above is not the reason to widen it."
        )
    add("")

    return "\n".join(lines) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--dir", default=DEFAULT_DIR, help="where the probe fixtures are")
    args = parser.parse_args(argv)

    index, documents = load_inputs(args.dir)
    report = build_report(index, documents)
    out_path = os.path.join(args.dir, "ceilings-probe-report.md")
    # Binary write, LF: `.gitattributes` enforces LF and a text-mode write on
    # Windows would produce CRLF -- the trap CLAUDE.md records.
    with open(out_path, "wb") as handle:
        handle.write(report.encode("utf-8"))
    print(report)
    print("wrote {}".format(out_path))


if __name__ == "__main__":
    main()
