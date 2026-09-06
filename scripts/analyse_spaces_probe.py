#!/usr/bin/env python3
"""Answer the six questions that gate `docs/Superseded/PLAN-spaces.md`, from the files
`probe_spaces_export.py` captured in Revit.

    python scripts/analyse_spaces_probe.py
    python scripts/analyse_spaces_probe.py --dir some/other/fixtures

    Q1  Does duHast produce a usable outer loop for a space bounded by a LINKED
        model?  **This is the kill condition.**
    Q2  How do the enclosure states divide the population?
    Q3  Does each document have spaces at all, and can it say so honestly?
    Q4  Is the linking key unique, and does it match?
    Q5  What does the systematic area difference look like, and where does it
        cross a percentage threshold?
    Q6  Which phase does each element belong to?

Writes `spaces-probe-report.md` beside the inputs and prints the same thing.

**This is where every judgement lives.** The probe collects and counts; nothing
in it decides whether a space is unenclosed or merely unmeasured. Keeping the
deciding here means it can be re-run, argued with and corrected without going
back to Revit -- which matters because the answers change the plan, and the plan
is what the argument is actually about.

THE THREE DISTINCTIONS THIS FILE MUST NOT COLLAPSE
1.  "No spaces in this model" and "spaces present but unmeasurable" are
    different answers. The first means find another model and costs nothing; the
    second is the kill condition and stops the plan.
2.  `Unenclosed` (Revit reports no boundary: a MODEL defect) and `Unmeasured`
    (Revit reports boundaries and duHast produced no polygon anyway: a PIPELINE
    defect) are different answers. PLAN-spaces D5 exists because fusing them
    into "has no geometry" would be a sentinel, and the +/-1e30 scar says what
    that costs.
3.  "This document was audited and holds no spaces" and "this document was never
    captured" are different answers -- D7's whole distinction. The index file is
    what separates them, which is why this reads a stated document list rather
    than globbing the directory.

Stdlib only, CPython 3. It reads JSON and prints; it must run wherever the data
lands, including a machine with no checkout of this repo beside it.
"""

import argparse
import json
import os
from collections import Counter, OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_DIR = os.path.join(HERE, "fixtures")

# Square feet per square metre, for the unit check in Q5. Revit's internal area
# is always square feet; the `Area` PARAMETER is in the document's display
# units. A ratio near 1 says the parameter is square feet, near this constant
# says square metres.
SQFT_PER_SQM = 10.7639104

# How far a measured unit ratio may sit from a candidate and still be called
# that unit. Loose, because the question is "which of two units", not "how
# precise".
UNIT_TOLERANCE = 0.02

# The share of spaces in the `Unmeasured` state at which the plan stops. Not
# zero: one unmeasurable space in a thousand is a bug report, not a reason to
# abandon an entity. Ten percent is a population, and a population means the
# linked-boundary fallback does not carry this case.
UNMEASURED_BLOCK_PCT = 10.0

# Below this share of spaces matching a room on the key, the key is the open
# question rather than the entity. Not a kill condition -- the entity is still
# right, the CSV probe just answers the key question faster than PR B would.
KEY_MATCH_WEAK_PCT = 90.0

# Room-area buckets for the Q5 crossover table, in the document's own display
# units. Chosen to straddle the sizes where a boundary-regime difference stops
# being a rounding error and starts being tens of percent: a riser, a cupboard,
# a WC, a room.
AREA_BUCKETS = [
    (0.0, 2.0, "under 2"),
    (2.0, 5.0, "2 to 5"),
    (5.0, 10.0, "5 to 10"),
    (10.0, 25.0, "10 to 25"),
    (25.0, 100.0, "25 to 100"),
    (100.0, None, "over 100"),
]


def load(path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def load_inputs(directory):
    """The index, and every probe and raw file it names.

    Reads the index rather than globbing: a fixtures directory accumulates runs,
    and an analyser that picked up yesterday's services model beside today's
    architectural one would report a key match against a population nobody
    captured -- and would look entirely plausible doing it."""
    index_path = os.path.join(directory, "spaces-probe-index.json")
    if not os.path.isfile(index_path):
        raise SystemExit(
            "missing spaces-probe-index.json\n\nRun scripts/probe_spaces_export.py "
            "from pyRevit first; it writes the index and the per-document files "
            "into scripts/fixtures/."
        )
    index = load(index_path)

    documents = []
    for entry in index.get("documents", []):
        slug = entry.get("slug")
        loaded = {"summary": entry, "entities": {}}
        for entity in ("spaces", "rooms"):
            probe_path = os.path.join(directory, "{}-probe-{}.json".format(entity, slug))
            raw_path = os.path.join(directory, "{}-raw-{}.json".format(entity, slug))
            if not os.path.isfile(probe_path):
                continue
            probe = load(probe_path)
            # The export is REDUCED as it is read, never retained. A real run is
            # hundreds of megabytes -- RHH's four services models alone are
            # 141 MB of space polygons. Holding all of them parsed, to answer one
            # integer and a handful of properties per element, is how an analyser
            # that works on a house dies on a hospital.
            outer_points, properties = reduce_export(
                load(raw_path) if os.path.isfile(raw_path) else None,
                probe.get("list_key", entity),
            )
            loaded["entities"][entity] = {
                "probe": probe,
                "outer_points": outer_points,
                "properties": properties,
            }
        documents.append(loaded)

    return {"index": index, "documents": documents}


# --------------------------------------------------------------------------
# Joining the export to the probe
# --------------------------------------------------------------------------


def reduce_export(raw, list_key):
    """The two things this file ever asks a raw export: how many points each
    element's OUTER loop has, and what its properties say.

    **Property values come from the EXPORT, not from the probe's own parameter
    reads, and the difference is not cosmetic.** The probe reads
    `LookupParameter("Area").AsValueString()`, which on RHH returns `"71 m2"` --
    display-formatted, unit-suffixed and rounded to the whole number. duHast's
    export of the same space carries `71.27892877719862`. The server stores the
    export's value, so an analysis run on the probe's is measuring something no
    consumer will ever see, and quantising every small space by up to half a
    square metre before comparing it. Fixing that here rather than re-running in
    Revit is the point of the probe/analyser split.

    Keyed on the id inside `instance_properties`, the same place
    `translate_room` reads it from. Zero points and absent are folded together:
    `translate_room` drops both, so for every question asked here they mean the
    same thing."""
    outer_points = {}
    properties = {}
    for record in (raw or {}).get(list_key, []) or []:
        instance = record.get("instance_properties") or {}
        element_id = instance.get("id")
        if element_id is None:
            continue
        element_id = str(element_id)
        polygons = record.get("polygon") or []
        outer = (polygons[0] or {}).get("outer_loop") or [] if polygons else []
        outer_points[element_id] = len(outer)
        properties[element_id] = dict(
            (entry.get("name"), entry.get("value"))
            for entry in (instance.get("properties") or [])
            if isinstance(entry, dict)
        )
    return outer_points, properties


# --------------------------------------------------------------------------
# Q1 and Q2 -- enclosure
# --------------------------------------------------------------------------


def classify(element, points):
    """One element's enclosure state.

    Ordered so the cheapest, least ambiguous test comes first, and so the two
    states PLAN-spaces D5 refuses to fuse stay apart:

    - `unplaced`     no Location. Dropped by the extractor (D4); not a finding.
    - `unenclosed`   placed, no area, no boundary segments. A MODEL defect.
    - `redundant`    placed, no area, but boundary segments exist. Revit's own
                     third state -- an overlapping or duplicated space. Reported
                     separately here and folded into `Unenclosed` by D5's
                     three-state contract, because the distinction is one the
                     modeller acts on and the server cannot.
    - `unmeasured`   placed, has area, and NO exported outer loop. A PIPELINE
                     defect, and the one the kill condition counts.
    - `enclosed`     an exported outer loop.
    """
    if element.get("location_is_none"):
        return "unplaced"

    if points > 0:
        return "enclosed"

    area = element.get("area_internal_sqft") or 0.0
    segments = element.get("boundary_segment_count") or 0
    if area <= 0.0:
        return "redundant" if segments > 0 else "unenclosed"
    return "unmeasured"


def enclosure_rows(documents):
    """Per services document, the enclosure histogram, plus every element
    classified once so later questions can reuse it."""
    rows = []
    classified = []
    for doc in documents:
        entry = doc["entities"].get("spaces")
        if not entry:
            continue
        probe = entry["probe"]
        points = entry["outer_points"]
        counts = Counter()
        for element in probe.get("elements", []):
            state = classify(element, points.get(element.get("id"), 0))
            counts[state] += 1
            classified.append(
                {
                    "document": probe.get("document"),
                    "state": state,
                    "element": element,
                }
            )
        rows.append(
            {
                "document": probe.get("document"),
                "total": len(probe.get("elements", [])),
                "counts": counts,
            }
        )
    return rows, classified


# --------------------------------------------------------------------------
# Q3 -- presence, and whether the collector can be trusted
# --------------------------------------------------------------------------


def presence_rows(documents):
    """Per document and entity: what this probe counted, what duHast counted,
    and what the export produced.

    A `duhast_collector_error` is U1 caught in the act. A silent disagreement
    between the two counts, with no error, is worse: it means the swallow
    happened somewhere this probe cannot see."""
    rows = []
    for doc in documents:
        for entity in ("spaces", "rooms"):
            entry = doc["entities"].get(entity)
            if not entry:
                continue
            probe = entry["probe"]
            rows.append(
                {
                    "document": probe.get("document"),
                    "entity": entity,
                    "collected": probe.get("collected_count"),
                    "duhast": probe.get("duhast_collected_count"),
                    "exported": probe.get("exported_count"),
                    "error": probe.get("duhast_collector_error"),
                }
            )
    return rows


# --------------------------------------------------------------------------
# Q4 -- the key
# --------------------------------------------------------------------------


def key_of(element, properties, key_property):
    """One element's key value, preferring the EXPORT's property over the
    probe's own parameter read, for `reduce_export`'s reason."""
    value = (properties.get(element.get("id")) or {}).get(key_property)
    if value is None:
        value = element.get("key_value")
    return ("" if value is None else str(value)).strip()


def key_index(entries, key_property):
    """Key value -> list of (document, element), over the given entity entries.

    Hierarchy-blind, which is what the QA report will be (PLAN-spaces D3). A key
    appearing twice is recorded rather than resolved: D2 says ambiguity is
    reported and never guessed."""
    index = OrderedDict()
    for entry in entries:
        probe = entry["probe"]
        for element in probe.get("elements", []):
            value = key_of(element, entry["properties"], key_property)
            index.setdefault(value, []).append((probe.get("document"), element))
    return index


def match_one_way(space_index, room_index):
    """ONE services model's spaces against the pooled rooms.

    **Scoped to one space document, and that is the correction RHH forced.**
    PLAN-spaces D2 said "project-wide across every model holding rooms", which
    quietly assumed one space per room. RHH has one services file PER SERVICE,
    each covering the whole building, so a room number legitimately names a
    mechanical space AND a hydraulic one AND an electrical one. Pooling them
    reported 3046 duplicate keys and made the expected shape of the data into
    the loudest finding in the report. Duplication WITHIN one services model is
    the real ambiguity; duplication across them is the design."""
    space_keys = set(k for k in space_index if k)
    room_keys = set(k for k in room_index if k)
    return {
        "space_total": sum(len(v) for v in space_index.values()),
        "space_blank": len(space_index.get("", [])),
        "matched": sorted(space_keys & room_keys),
        "space_only": sorted(space_keys - room_keys),
        "space_duplicates": sorted(k for k in space_keys if len(space_index[k]) > 1),
    }


# --------------------------------------------------------------------------
# Q5 -- area, its unit, and the crossover
# --------------------------------------------------------------------------


def as_float(value):
    try:
        return float(str(value).strip())
    except (TypeError, ValueError):
        return None


def area_unit(probe, properties):
    """Which unit this document's exported `Area` is in, measured rather than
    assumed: the median of exported / internal across its elements.

    This is the check that stops the whole area comparison being nonsense. The
    server compares stored property values, so an architectural model exporting
    square metres against a services model exporting square feet would disagree
    by 10.76x on every single pair -- a hundred-percent findings rate that says
    nothing about the data."""
    ratios = []
    for element in probe.get("elements", []):
        stated = as_float((properties.get(element.get("id")) or {}).get("Area"))
        internal = element.get("area_internal_sqft")
        if stated is None or not internal:
            continue
        ratios.append(stated / internal)
    if not ratios:
        return {"unit": "unknown", "ratio": None, "samples": 0}
    ratios.sort()
    median = ratios[len(ratios) // 2]
    if abs(median - 1.0) <= UNIT_TOLERANCE:
        unit = "square feet"
    elif abs(median - (1.0 / SQFT_PER_SQM)) <= UNIT_TOLERANCE:
        unit = "square metres"
    else:
        unit = "unrecognised"
    return {"unit": unit, "ratio": round(median, 5), "samples": len(ratios)}


def area_deltas(space_index, room_index, matched, state_by_element):
    """Per matched key, the relative area difference, measured the way D8 says:
    `|space - room| / room`, against the ROOM.

    Uses each side's internal square feet rather than its parameter, so the
    number is a geometry difference and not a unit difference. The unit check
    above is what tells the reader whether the SERVER would see the same
    number.

    **Only `enclosed` spaces are measured, and excluding the rest is not
    tidying.** An unenclosed space has an area of zero, so pairing it with a
    real room yields a 100% difference -- which is the enclosure finding from Q2
    arriving a second time wearing an area costume. Left in, it drags the median
    of whichever size bucket it lands in, and this table exists precisely to be
    read as a distribution: `tolerance_min` is chosen from it. A geometry
    tolerance calibrated on spaces that have no geometry would be set by the
    defect it is meant to see past.
    """
    out = []
    excluded = Counter()
    for key in matched:
        space_doc, space = space_index[key][0]
        room = room_index[key][0][1]
        state = state_by_element.get((space_doc, space.get("id")))
        if state != "enclosed":
            excluded[state or "unknown"] += 1
            continue
        space_area = space.get("area_internal_sqft")
        room_area = room.get("area_internal_sqft")
        if space_area is None or room_area is None:
            excluded["no area recorded"] += 1
            continue
        if not room_area:
            out.append({"key": key, "document": space_doc, "room": room_area,
                        "space": space_area, "delta_pct": None})
            continue
        delta = abs(space_area - room_area) / room_area * 100.0
        out.append(
            {
                "key": key,
                "document": space_doc,
                "room": room_area,
                "space": space_area,
                "delta_pct": round(delta, 1),
            }
        )
    return out, excluded


# Candidate thresholds for the sweep below. Percentages are what a reader
# reaches for; the floors are in the room document's display units, and exist
# because a percentage alone flags whichever size band sits above it.
SWEEP_PCTS = [20.0, 30.0, 50.0, 100.0]
SWEEP_MINS = [0.0, 1.0, 2.0, 5.0]


def threshold_sweep(deltas, unit_ratio):
    """How many pairs each `(tolerance_pct, tolerance_min)` combination would
    flag.

    The table that turns "flag past 30%, say" into a number. D8's two thresholds
    are ANDed, so a floor removes the small-room band without touching the large
    disagreements -- and the only way to choose either is to see what each costs
    in findings a person has to read."""
    scale = unit_ratio if unit_ratio else 1.0
    rows = []
    for pct_threshold in SWEEP_PCTS:
        row = [pct_threshold]
        for floor in SWEEP_MINS:
            flagged = 0
            for entry in deltas:
                if entry["delta_pct"] is None:
                    continue
                room_display = entry["room"] * scale
                space_display = entry["space"] * scale
                if entry["delta_pct"] <= pct_threshold:
                    continue
                if abs(space_display - room_display) <= floor:
                    continue
                flagged += 1
            row.append(flagged)
        rows.append(row)
    return rows


def bucket_of(area_value):
    for low, high, label in AREA_BUCKETS:
        if area_value >= low and (high is None or area_value < high):
            return label
    return AREA_BUCKETS[-1][2]


def crossover_rows(deltas, unit_ratio):
    """The Q5 table: median and worst relative difference per room-size bucket.

    The number `tolerance_min` should be set from. A single project-wide
    percentage flags small rooms and nothing else, because the boundary-regime
    delta scales with perimeter while the value scales with area -- so the
    honest output is per size band, not one number."""
    scale = unit_ratio if unit_ratio else 1.0
    buckets = OrderedDict((label, []) for _l, _h, label in AREA_BUCKETS)
    incomparable = 0
    for row in deltas:
        if row["delta_pct"] is None:
            incomparable += 1
            continue
        buckets[bucket_of(row["room"] * scale)].append(row)

    rows = []
    for label, entries in buckets.items():
        if not entries:
            rows.append([label, 0, "-", "-", "-"])
            continue
        pcts = sorted(e["delta_pct"] for e in entries)
        median = pcts[len(pcts) // 2]
        worst = pcts[-1]
        over_30 = sum(1 for p in pcts if p > 30.0)
        rows.append([label, len(entries), median, worst, over_30])
    return rows, incomparable


# --------------------------------------------------------------------------
# Q6 -- phases
# --------------------------------------------------------------------------


def phase_rows(documents):
    rows = []
    for doc in documents:
        for entity in ("spaces", "rooms"):
            entry = doc["entities"].get(entity)
            if not entry:
                continue
            probe = entry["probe"]
            counts = Counter(
                (element.get("phase_name") or "(none)")
                for element in probe.get("elements", [])
            )
            for phase, count in counts.most_common():
                rows.append([probe.get("document"), entity, phase, count])
    return rows


# --------------------------------------------------------------------------
# The verdict
# --------------------------------------------------------------------------


def pct(n, total):
    return 0.0 if not total else round(100.0 * n / total, 1)


def verdict(totals, presence, keys, has_rooms):
    """The one line the plan actually waits on.

    Ordered so the cheap, recoverable answers come first: a document with no
    spaces is not a finding about spaces, and a collector that raised is a
    finding about duHast. Only when neither is true does a geometry-less
    population become the kill condition."""
    if totals["spaces"] == 0:
        return (
            "NO SPACES IN THESE DOCUMENTS - nothing was collected in "
            "OST_MEPSpaces anywhere in the run. This is NOT the kill condition; "
            "select a services model that has spaces in it and re-run."
        )

    errored = [r for r in presence if r["error"]]
    if errored:
        return (
            "INVALID - duHast's collector raised on {} document/entity pair(s), "
            "which is upstream item U1 caught in the act. Every count below is "
            "measuring what THIS script collected, not what a push would. Land "
            "U1 and re-run.".format(len(errored))
        )

    unmeasured = pct(totals["unmeasured"], totals["placed"])
    if unmeasured >= UNMEASURED_BLOCK_PCT:
        return (
            "BLOCKED - {}% of placed spaces ({} of {}) have area and boundary "
            "segments in Revit and NO exported polygon. duHast's outer-loop "
            "fallback does not carry the linked-boundary case, so spaces arrive "
            "geometry-less: PLAN-spaces D5 and D6 both collapse. This is the "
            "kill condition. Fix the fallback upstream before PR B.".format(
                unmeasured, totals["unmeasured"], totals["placed"]
            )
        )

    if not has_rooms:
        return (
            "PARTIAL - {} spaces measured cleanly, but no document in this run "
            "held rooms, so Q4 and Q5 are unanswered. Q1, Q2, Q3 and Q6 stand. "
            "Re-run with the architectural model selected as well.".format(
                totals["spaces"]
            )
        )

    matched_pct = pct(len(keys["matched"]), len(keys["matched"]) + len(keys["space_only"]))
    if matched_pct < KEY_MATCH_WEAK_PCT:
        return (
            "KEYS WEAK - only {}% of space keys match a room ({} matched, {} "
            "unmatched, {} blank). The entity is still the right shape; the KEY "
            "is the open question, and a CSV export answers it faster than PR B "
            "would.".format(
                matched_pct,
                len(keys["matched"]),
                len(keys["space_only"]),
                keys["space_blank"],
            )
        )

    notes = []
    if keys["duplicates_within"] or keys["room_duplicates"]:
        notes.append(
            "the key is NOT unique ({} keys duplicated inside a single services "
            "model, {} duplicated across the room sources) - D2's ambiguity "
            "reporting stops being an invariant check and becomes a finding".format(
                keys["duplicates_within"], len(keys["room_duplicates"])
            )
        )
    if totals["unmeasured"]:
        notes.append(
            "{} space(s) still unmeasured - below the block threshold, but each "
            "one is a space the viewer cannot draw".format(totals["unmeasured"])
        )
    if notes:
        return "PROCEED WITH CARE - {}. Read the tables.".format("; ".join(notes))

    return (
        "CLEAR - {} spaces across {} document(s), {}% matching a room on '{}'. "
        "PR B may start.".format(
            totals["spaces"], totals["space_documents"], matched_pct, keys["key_property"]
        )
    )


def table(headers, rows):
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join(["---"] * len(headers)) + "|"]
    for row in rows:
        out.append("| " + " | ".join(str(c) for c in row) + " |")
    return "\n".join(out)


# --------------------------------------------------------------------------
# The report
# --------------------------------------------------------------------------


def build_report(data):
    index = data["index"]
    documents = data["documents"]
    key_property = index.get("key_property", "Number")

    lines = []

    def add(text=""):
        lines.append(text)

    encl_rows, classified = enclosure_rows(documents)
    presence = presence_rows(documents)

    # Which documents play which role. A document is a SPACE source if it holds
    # spaces and a ROOM source if it holds rooms -- and RHH proved those are not
    # exclusive: `RHH-JHA-EL-MDL-HOS` holds 2945 spaces AND 3418 rooms of its
    # own, numbered `1`, `2`, `3`... against the architects' `ENG137`. Pooling
    # those into the room side put 3418 rooms that name nothing into the
    # unmatched column. `--room-docs` is how a reader says which documents are
    # actually the room authority; without it the table below shows the
    # composition so the pollution is at least visible rather than averaged in.
    space_entries = []
    room_entries = []
    for doc in documents:
        for entity, bucket, wanted in (
            ("spaces", space_entries, data["space_docs"]),
            ("rooms", room_entries, data["room_docs"]),
        ):
            entry = doc["entities"].get(entity)
            if not entry or not entry["probe"].get("elements"):
                continue
            title = entry["probe"].get("document") or ""
            if wanted and wanted not in title:
                continue
            bucket.append(entry)

    room_index = key_index(room_entries, key_property)
    per_space_doc = []
    for entry in space_entries:
        index_one = key_index([entry], key_property)
        result = match_one_way(index_one, room_index)
        result["document"] = entry["probe"].get("document")
        result["index"] = index_one
        per_space_doc.append(result)

    keys = {
        "key_property": key_property,
        "matched": sorted(set(k for r in per_space_doc for k in r["matched"])),
        "space_only": sorted(set(k for r in per_space_doc for k in r["space_only"])),
        "space_blank": sum(r["space_blank"] for r in per_space_doc),
        "duplicates_within": sum(len(r["space_duplicates"]) for r in per_space_doc),
        "room_duplicates": sorted(
            k for k in room_index if k and len(room_index[k]) > 1
        ),
    }

    state_counts = Counter(c["state"] for c in classified)
    totals = {
        "spaces": sum(r["total"] for r in encl_rows),
        "space_documents": len(encl_rows),
        "placed": sum(v for k, v in state_counts.items() if k != "unplaced"),
        "unmeasured": state_counts.get("unmeasured", 0),
    }
    has_rooms = any("rooms" in doc["entities"] for doc in documents)

    add("# Spaces probe report")
    add()
    add("Probe v{}, key property `{}`, {} document(s).".format(
        index.get("probe_version"), key_property, len(documents)
    ))
    add()
    add("> **{}**".format(verdict(totals, presence, keys, has_rooms)))
    add()

    # ---- Q3 first: it decides whether anything below can be trusted ----
    add("## Q3 - Presence, and whether the collector can be trusted")
    add()
    add("`collected` is this script's own `FilteredElementCollector`; `duHast` is")
    add("`get_all_spaces` / `get_all_rooms`. **They must agree.** A disagreement,")
    add("or an error, is upstream item U1: a collector that answers `[]` on")
    add("failure cannot tell 'no spaces' from 'the collector broke', which is the")
    add("one question requirement 1 rests on.")
    add()
    add(table(
        ["document", "entity", "collected", "duHast", "exported", "collector error"],
        [
            [r["document"], r["entity"], r["collected"], r["duhast"], r["exported"],
             r["error"] or "-"]
            for r in presence
        ],
    ))
    add()

    # ---- Q1 and Q2 ----
    add("## Q1 and Q2 - Enclosure, and the kill condition")
    add()
    add("`unmeasured` is the kill condition's population: placed, non-zero area,")
    add("boundary segments present, and NO exported polygon. `unenclosed` is a")
    add("model defect and an expected finding; `redundant` is Revit's own third")
    add("state (no area, but boundaries) and folds into `unenclosed` under D5.")
    add()
    add(table(
        ["document", "total", "enclosed", "unenclosed", "redundant", "unmeasured", "unplaced"],
        [
            [
                r["document"], r["total"],
                r["counts"].get("enclosed", 0),
                r["counts"].get("unenclosed", 0),
                r["counts"].get("redundant", 0),
                r["counts"].get("unmeasured", 0),
                r["counts"].get("unplaced", 0),
            ]
            for r in encl_rows
        ],
    ))
    add()
    add("Unmeasured share of placed spaces: **{}%** (block at {}%).".format(
        pct(totals["unmeasured"], totals["placed"]), UNMEASURED_BLOCK_PCT
    ))
    add()

    unmeasured_samples = [c for c in classified if c["state"] == "unmeasured"][:15]
    if unmeasured_samples:
        add("Unmeasured spaces (first {}), so the families can be looked at:".format(
            len(unmeasured_samples)
        ))
        add()
        add(table(
            ["document", "id", "key", "area (sqft)", "boundary loops", "segments"],
            [
                [
                    c["document"],
                    c["element"].get("id"),
                    c["element"].get("key_value") or "-",
                    round(c["element"].get("area_internal_sqft") or 0.0, 2),
                    c["element"].get("boundary_loop_count"),
                    c["element"].get("boundary_segment_count"),
                ]
                for c in unmeasured_samples
            ],
        ))
        add()

    # ---- Q4 ----
    add("## Q4 - The key")
    add()
    if not has_rooms:
        add("No document in this run held rooms, so there is nothing to match")
        add("against. Re-run with the architectural model selected.")
        add()
    else:
        add("**One row per services model, matched against the pooled rooms.**")
        add("Not one project-wide pool: a services file per service means a room")
        add("number legitimately names a space in each of them, so duplication")
        add("ACROSS services models is the design and only duplication WITHIN one")
        add("is ambiguity. `duplicate` below counts the latter.")
        add()
        add(table(
            ["services model", "spaces", "blank key", "distinct", "duplicate",
             "matched", "unmatched"],
            [
                [
                    r["document"], r["space_total"], r["space_blank"],
                    len(r["matched"]) + len(r["space_only"]),
                    len(r["space_duplicates"]),
                    len(r["matched"]),
                    len(r["space_only"]),
                ]
                for r in per_space_doc
            ],
        ))
        add()
        add("Room sources, and how much of each is reachable by any space:")
        add()
        room_by_doc = OrderedDict()
        for value, entries in room_index.items():
            for doc_title, _element in entries:
                stats = room_by_doc.setdefault(doc_title, {"total": 0, "matched": 0})
                stats["total"] += 1
                if value and value in set(keys["matched"]):
                    stats["matched"] += 1
        add(table(
            ["room model", "rooms", "named by some space", "named by none"],
            [
                [title, s["total"], s["matched"], s["total"] - s["matched"]]
                for title, s in room_by_doc.items()
            ],
        ))
        add()
        if keys["space_only"]:
            add("Space keys matching no room (first 20): `{}`".format(
                "`, `".join(keys["space_only"][:20])
            ))
            add()

    # ---- Q5 ----
    add("## Q5 - Area, its unit, and where a percentage threshold crosses")
    add()
    unit_rows = []
    for entity, entries in (("spaces", space_entries), ("rooms", room_entries)):
        for entry in entries:
            unit = area_unit(entry["probe"], entry["properties"])
            unit_rows.append([
                entry["probe"].get("document"), entity, unit["unit"], unit["ratio"],
                unit["samples"],
            ])
    add("The exported `Area`'s unit per document, measured against Revit's")
    add("internal square feet. **These must agree across documents**: the server")
    add("compares stored property values, so a square-metre model against a")
    add("square-foot one disagrees by 10.76x on every pair -- a 100% findings rate")
    add("that says nothing about the data.")
    add()
    add(table(["document", "entity", "unit", "ratio", "samples"], unit_rows))
    add()

    if has_rooms and keys["matched"]:
        room_unit = area_unit(
            room_entries[0]["probe"], room_entries[0]["properties"]
        ) if room_entries else None
        state_by_element = dict(
            ((c["document"], c["element"].get("id")), c["state"]) for c in classified
        )
        deltas = []
        excluded = Counter()
        for result in per_space_doc:
            one, one_excluded = area_deltas(
                result["index"], room_index, result["matched"], state_by_element
            )
            deltas.extend(one)
            excluded.update(one_excluded)
        rows, incomparable = crossover_rows(deltas, (room_unit or {}).get("ratio"))
        add("Relative area difference `|space - room| / room`, by room size, in")
        add("the room document's display units. **This is the table")
        add("`tolerance_min` is chosen from.** A percentage threshold alone flags")
        add("whichever bucket sits above it, and that is usually the smallest one:")
        add("the boundary-regime delta scales with perimeter while the value")
        add("scales with area.")
        add()
        add(table(
            ["room area", "pairs", "median delta %", "worst delta %", "over 30%"],
            rows,
        ))
        add()
        if incomparable:
            add("{} matched pair(s) had a zero room area and are `incomparable` -".format(
                incomparable
            ))
            add("neither pass nor mismatch, which is the state D8 gives them.")
            add()
        if excluded:
            add("Excluded from the table above, because a space with no geometry")
            add("would report a 100% area difference and drag its bucket's median -")
            add("that is the Q2 enclosure finding, already counted once: {}.".format(
                ", ".join(
                    "{} {}".format(count, state) for state, count in sorted(excluded.items())
                )
            ))
            add()
        worst = sorted(
            (d for d in deltas if d["delta_pct"] is not None),
            key=lambda d: d["delta_pct"],
            reverse=True,
        )[:15]
        sweep = threshold_sweep(deltas, (room_unit or {}).get("ratio"))
        add("What each threshold pair would actually flag, out of {} comparable".format(
            len([d for d in deltas if d["delta_pct"] is not None])
        ))
        add("pairs. Columns are `tolerance_min` in the room document's display")
        add("units; D8 ANDs the two, so a floor removes the small-room band")
        add("without touching a large disagreement.")
        add()
        add(table(
            ["tolerance_pct"] + ["min {}".format(m) for m in SWEEP_MINS],
            sweep,
        ))
        add()
        if worst:
            add("Worst {} pairs:".format(len(worst)))
            add()
            add(table(
                ["key", "services model", "room (sqft)", "space (sqft)", "delta %"],
                [[w["key"], w["document"], round(w["room"], 2), round(w["space"], 2),
                  w["delta_pct"]]
                 for w in worst],
            ))
            add()

    # ---- Q6 ----
    add("## Q6 - Phases")
    add()
    add("Read through `ROOM_PHASE`, the parameter a space shares with a room -")
    add("which is why PLAN-spaces D10 can use the rooms equality filter rather")
    add("than the doors range test. A services model naming its phase differently")
    add("from the architectural model is not an error, but it is worth seeing")
    add("before someone configures a push.")
    add()
    add(table(["document", "entity", "phase", "elements"], phase_rows(documents)))
    add()

    problems = []
    for doc in documents:
        for entity, entry in doc["entities"].items():
            for problem in entry["probe"].get("serialisation_problems", []) or []:
                problems.append([
                    entry["probe"].get("document"), entity, problem.get("path"),
                    problem.get("type"),
                ])
    if problems:
        add("## Serialisation problems")
        add()
        add("Fields duHast could not convert. A field that will not serialise is a")
        add("field the contract cannot carry, whatever the plan assumed about it.")
        add()
        add(table(["document", "entity", "path", "type"], problems[:30]))
        add()

    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--dir", default=DEFAULT_DIR, help="directory holding the probe files")
    parser.add_argument("--out", default=None, help="report path (default: <dir>/spaces-probe-report.md)")
    # Role filters, because "holds rooms" does not mean "is the room authority".
    # RHH's electrical model holds 3418 rooms of its own, numbered on a scheme
    # nothing else shares; pooled into the room side they are 3418 rooms that
    # name nothing. A substring is enough to say which documents count.
    parser.add_argument("--room-docs", default=None,
                        help="only treat documents whose title contains this as room sources")
    parser.add_argument("--space-docs", default=None,
                        help="only treat documents whose title contains this as space sources")
    args = parser.parse_args(argv)

    data = load_inputs(args.dir)
    data["room_docs"] = args.room_docs
    data["space_docs"] = args.space_docs
    report = build_report(data)

    out = args.out or os.path.join(args.dir, "spaces-probe-report.md")
    # Binary, for the LF reason `probe_spaces_export.write_json` gives.
    with open(out, "wb") as handle:
        handle.write(report.encode("utf-8"))
    print(report)
    print("wrote {}".format(out))


if __name__ == "__main__":
    main()
