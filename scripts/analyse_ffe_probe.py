#!/usr/bin/env python3
"""Answer the six questions that gate `docs/PLAN-ffe.md`, from the files
`probe_ffe_export.py` captured in Revit.

    python scripts/analyse_ffe_probe.py
    python scripts/analyse_ffe_probe.py --dir some/other/fixtures

    Q1  Does `get_Room(phase)` populate on a model that holds both the rooms
        and the FFE?  **This is the kill condition.**
    Q2  What did the export drop, per category, and what were those instances?
    Q3  What is the category histogram across duHast's eight defaults?
    Q4  What does the super-component matrix look like across those eight?
    Q5  Does the bbox-derived level agree with the level the instance names?
    Q6  What unit is each numeric field in?

Writes `ffe-probe-report.md` beside the inputs and prints the same thing.

**This is where every judgement lives.** The probe collects and counts; nothing
in it decides whether a level is wrong or whether a dropped instance matters.
Keeping the deciding here means it can be re-run, argued with and corrected
without going back to Revit -- which matters because the answers change the
plan, and the plan is what the argument is actually about.

THE DISTINCTION THIS FILE MUST NOT COLLAPSE
"No FFE in this model" and "FFE present but unresolvable" are different
answers, and `docs/PLAN-ffe.md` says so before the data arrives precisely so
that the analyser cannot quietly merge them. The first means find another model
and costs nothing; the second is the kill condition and stops the plan. A
verdict that reported "0 items name a room" for a model containing no items
would be technically true and would end the work for the wrong reason.

Stdlib only, CPython 3. It reads JSON and prints; it must run wherever the data
lands, including a machine with no checkout of this repo beside it.
"""

import argparse
import json
import os
from collections import Counter, OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_DIR = os.path.join(HERE, "fixtures")

# Millimetres per foot. Q6 divides the export's numbers by the probe's, which
# are in Revit's internal decimal feet; a ratio landing here says the export
# converted, and a ratio of 1 says it did not.
MM_PER_FOOT = 304.8

# How far a measured ratio may sit from a candidate unit and still be called
# that unit. Loose, because the comparison is over real coordinates whose
# magnitudes vary and the question is "which of two units", not "how precise".
UNIT_TOLERANCE = 0.02

# Below this share of items naming a room in the best phase, the join is thin
# enough to be worth reading the tables before starting. Not the kill
# condition -- that is zero -- because a model part-way through being furnished
# is an ordinary state and the plan should survive it.
THIN_ROOM_REFERENCE_PCT = 50.0

# The fields `contract::items::Item` assumes, and where each is read from in the
# raw export. This table IS the question D1 turns on. Two entries are expected
# to come back EMPTY today and that is the finding rather than a failure:
# `loops` lands only once upstream change U1 ships, and `category` is not in the
# export at all -- `DataItem` has no field for it, which is why the extractor
# must take it from its own collector pass.
ITEM_FIELDS = OrderedDict(
    [
        ("id", "instance_properties.id"),
        ("level_id", "level.id"),
        ("level_offset", "level.offset_from_level"),
        ("insertion_point", "location_point.translation_coord"),
        ("facing", "location_point.rotation_coord"),
        ("loops", "polygon[].outer_loop"),
        ("type_id", "type_properties.id"),
        ("type_name", "type_properties.name"),
        ("properties", "instance_properties.properties"),
        ("type_properties", "type_properties.properties"),
        ("super_component_id", "super_component_id"),
        ("phasing", "phasing.created"),
        ("design_set", "design_set_and_option.set_name"),
        ("rooms_duhast", "rooms"),
    ]
)

# The same for a door, so the control answers the same table. Deliberately NOT
# the same paths: that is the point. A door carries `polygon`, `from_room` and
# `to_room` and the Z extents; an item carries `location_point` and `rooms`.
DOOR_FIELDS = OrderedDict(
    [
        ("id", "instance_properties.id"),
        ("level_id", "level.id"),
        ("loops", "polygon[].outer_loop"),
        ("type_id", "type_properties.id"),
        ("type_name", "type_properties.name"),
        ("properties", "instance_properties.properties"),
        ("type_properties", "type_properties.properties"),
        ("from_room", "from_room"),
        ("to_room", "to_room"),
        ("super_component_id", "super_component_id"),
        ("phasing", "phasing.created"),
        ("room_calculation_point", "room_calculation_point"),
        ("bounding_box_min_z", "bounding_box_min_z"),
        ("bounding_box_max_z", "bounding_box_max_z"),
    ]
)

# The export fields whose absence is disqualifying. `loops` is NOT here: the
# plan schedules U1 to add it and states that RoomMate ships correct without it,
# so blocking on it would contradict the plan this analyser serves.
REQUIRED_ITEM_FIELDS = ("id", "level_id", "type_id", "type_name", "insertion_point")


# --------------------------------------------------------------------------
# Loading
# --------------------------------------------------------------------------


def load(path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def load_inputs(directory):
    """The four files, with the door control falling back to the committed
    fixture.

    The fallback is reported rather than silent: `doors-raw.json` was captured
    from an older duHast, so a field missing there proves nothing about today.
    A comparison run against it is still worth having -- it is just a weaker
    claim, and the report has to say which claim it is making."""
    ffe_raw = os.path.join(directory, "ffe-raw.json")
    ffe_probe = os.path.join(directory, "ffe-probe.json")
    missing = [p for p in (ffe_raw, ffe_probe) if not os.path.isfile(p)]
    if missing:
        raise SystemExit(
            "missing {}\n\nRun scripts/probe_ffe_export.py from pyRevit first; "
            "it writes these into scripts/fixtures/.".format(
                ", ".join(os.path.basename(m) for m in missing)
            )
        )

    control = os.path.join(directory, "doors-raw-control.json")
    control_note = "same document, same duHast"
    if not os.path.isfile(control):
        control = os.path.join(directory, "doors-raw.json")
        control_note = "committed fixture, OLDER duHast - a weaker comparison"
    doors_raw = load(control) if os.path.isfile(control) else None
    if doors_raw is None:
        control_note = "absent - the 'an item is not a door' claim is untested here"

    return {
        "ffe_raw": load(ffe_raw),
        "ffe_probe": load(ffe_probe),
        "doors_raw": doors_raw,
        "control_note": control_note,
    }


def records(raw, list_key):
    if not raw:
        return []
    return raw.get(list_key) or []


# --------------------------------------------------------------------------
# Field coverage and structure
# --------------------------------------------------------------------------


def dig(record, path):
    """Follow a dotted path, where `[]` takes the first list entry. Returns None
    for anything absent -- absence and a null value are folded together on
    purpose, because for this question they mean the same thing."""
    node = record
    for part in path.split("."):
        if part.endswith("[]"):
            node = node.get(part[:-2]) if isinstance(node, dict) else None
            if not isinstance(node, list) or not node:
                return None
            node = node[0]
        else:
            node = node.get(part) if isinstance(node, dict) else None
        if node is None:
            return None
    return node


def is_populated(value):
    """Populated means "carries information": an empty list, an empty string and
    duHast's -1 sentinel for "no super component" all count as absent."""
    if value is None:
        return False
    if isinstance(value, (list, dict, str)):
        return len(value) > 0
    if value == -1:
        return False
    return True


def field_coverage(entity_records, fields):
    """Per contract field: how many records carry it populated."""
    rows = []
    for field, path in fields.items():
        n = sum(1 for r in entity_records if is_populated(dig(r, path)))
        rows.append(
            {
                "field": field,
                "path": path,
                "count": n,
                "pct": pct(n, len(entity_records)),
            }
        )
    return rows


def key_paths(record, prefix="", depth=2):
    """The record's key paths to `depth`, for the structural diff. Two levels is
    enough to separate "the field is missing" from "the field is there and
    shaped differently", and deep enough to reach `level.id` without drowning in
    every property name."""
    out = set()
    if not isinstance(record, dict) or depth <= 0:
        return out
    for key, value in record.items():
        path = prefix + key
        out.add(path)
        if isinstance(value, dict):
            out |= key_paths(value, path + ".", depth - 1)
        elif isinstance(value, list) and value and isinstance(value[0], dict):
            out |= key_paths(value[0], path + "[].", depth - 1)
    return out


def structural_diff(item_records, door_records):
    """Key paths present on one entity and not the other, unioned across every
    record so an optional field on one instance is not read as a schema
    difference.

    For windows this table was expected to be EMPTY and was. Here it is expected
    to be full, in both directions, and that expectation is the plan's central
    claim -- so a surprisingly short table is as interesting as a long one."""
    items = set()
    for r in item_records:
        items |= key_paths(r)
    doors = set()
    for r in door_records:
        doors |= key_paths(r)
    return sorted(items - doors), sorted(doors - items)


# --------------------------------------------------------------------------
# Q1 - room references
# --------------------------------------------------------------------------


def room_reference_by_phase(probe):
    """Per phase: how many probed instances name a room.

    The kill condition reads off this table and nothing else. It is per phase
    because RoomMate pushes exactly one phase and the probe cannot know which
    one a run will pick -- so the answer is "the best phase available", and a
    model whose furniture is all in one phase looks correct rather than looking
    like a partial failure."""
    elements = list(probe.get("elements", {}).values())
    phases = probe.get("phases", [])
    sides = [s.lower() for s in probe.get("sides", ["room"])]

    rows = []
    for index, phase in enumerate(phases):
        named = 0
        errors = 0
        for element in elements:
            entries = element.get("rooms_by_phase") or []
            entry = next((e for e in entries if e.get("phase_index") == index), None)
            if entry is None:
                continue
            if any(entry.get(side + "_error") for side in sides):
                errors += 1
            if any(entry.get(side) for side in sides):
                named += 1
        rows.append(
            {
                "phase": phase.get("name"),
                "named": named,
                "pct": pct(named, len(elements)),
                "errors": errors,
            }
        )
    return rows, len(elements)


# --------------------------------------------------------------------------
# Q2 - what the export dropped
# --------------------------------------------------------------------------


def dropped_by_export(probe, item_records):
    """Instances the collector saw that the duHast export does not contain,
    broken down by category and by location class.

    **The location class is the whole point.** `populate_data_item_object`
    returns None for anything that is not a `LocationPoint`, so the expected
    answer is that every dropped instance is a `LocationCurve` or has no
    location at all. Anything dropped that DOES have a `LocationPoint` is a
    second, unexplained loss -- and that one is a finding the plan did not
    predict."""
    exported_ids = set()
    for record in item_records:
        value = dig(record, "instance_properties.id")
        if value is not None:
            exported_ids.add(str(value))

    elements = probe.get("elements", {})
    dropped = [e for eid, e in elements.items() if str(eid) not in exported_ids]

    by_category = Counter(e.get("collected_as") for e in dropped)
    by_location = Counter((e.get("location") or {}).get("class") for e in dropped)
    unexplained = [
        e
        for e in dropped
        if (e.get("location") or {}).get("class") == "LocationPoint"
    ]
    return {
        "dropped": dropped,
        "by_category": by_category,
        "by_location_class": by_location,
        "unexplained": unexplained,
        "collected": len(elements),
        "exported": len(exported_ids),
    }


# --------------------------------------------------------------------------
# Q4 - the super-component matrix
# --------------------------------------------------------------------------


def super_component_matrix(probe):
    """`(child category, parent category) -> count` for every nested instance.

    `nested_opening_ids` asks "is the parent the same category", which is a
    yes/no for one category and a matrix for eight. The off-diagonal entries are
    the interesting ones: a generic model inside a furniture item is a different
    statement about the model from a chair inside a chair, and the plan cannot
    say which rule to use until it can see which shapes actually occur."""
    matrix = Counter()
    nested = []
    top = []
    for element in probe.get("elements", {}).values():
        if element.get("super_component_id"):
            nested.append(element)
            parent = element.get("super_component_category_name") or "?"
            matrix[(element.get("category_name") or "?", parent)] += 1
        else:
            top.append(element)

    def with_a_room(group):
        n = 0
        for element in group:
            entries = element.get("rooms_by_phase") or []
            if any(e.get("room") for e in entries):
                n += 1
        return n

    same = sum(n for (child, parent), n in matrix.items() if child == parent)
    return {
        "matrix": matrix,
        "nested": len(nested),
        "top": len(top),
        "same_category": same,
        # The two discriminators, measured rather than inherited. The doors-era
        # test was "a component carries neither a room reference nor a Mark";
        # only one half of that transfers, and knowing WHICH is the whole
        # question of how the filter is written.
        "nested_with_room": with_a_room(nested),
        "top_with_room": with_a_room(top),
        "nested_with_mark": sum(1 for e in nested if e.get("mark")),
        "top_with_mark": sum(1 for e in top if e.get("mark")),
    }


# --------------------------------------------------------------------------
# Q5 - the level heuristic
# --------------------------------------------------------------------------


def named_level(element):
    """The level an instance names, or None.

    Three properties, because family hosting types disagree about which is
    populated. `"-1"` is Revit's "no element" id and is folded into None: an
    instance whose `LevelId` is invalid names no level, and counting it as one
    would turn the heuristic's best case into its worst."""
    for key in (
        "level_id_property",
        "level_id_family_param",
        "level_id_schedule_param",
    ):
        value = element.get(key)
        if value not in (None, "", "-1", -1):
            return str(value)
    return None


def level_agreement(probe, item_records):
    """How often the level **duHast exported** matches the level the instance
    itself names.

    **Against the export, not against a re-derivation, and the difference was a
    factor of six.** An earlier version of this function re-ran duHast's rule
    here -- last level at or below the bounding box minimum Z -- and compared
    that. It reported 184 disagreements on House A where the export itself
    disagrees on 29, because the rule was being fed Revit's own element box
    while duHast feeds it the SOLIDS box, and those differ on 184 instances for
    reasons that have nothing to do with levels (see `bounding_box_disagreement`).
    Re-deriving a producer's rule to check the producer measures the
    re-derivation. The export is the thing shipping, so the export is the thing
    compared.

    The re-derived value is still reported, clearly labelled, because it is the
    only evidence for HOW a disagreement arises once one is found. It is a
    diagnostic, never the count.

    An instance naming no level at all is its own row, and on House A it is 92
    of 647: for those the derived value is not a disagreement, it is the only
    answer there is -- which is the case FOR the heuristic and must not be
    counted against it."""
    exported = {}
    for record in item_records:
        value = dig(record, "instance_properties.id")
        if value is not None:
            exported[str(value)] = dig(record, "level.id")

    rows = {
        "agree": 0,
        "disagree": 0,
        "no_named_level": 0,
        "no_exported_level": 0,
        "not_exported": 0,
        "fallback_below_all_levels": 0,
        "rederived_disagrees": 0,
    }
    disagreements = []
    for eid, element in probe.get("elements", {}).items():
        if element.get("level_by_bbox_was_fallback"):
            rows["fallback_below_all_levels"] += 1
        named = named_level(element)
        derived = element.get("level_by_bbox")
        if named is not None and derived is not None and str(derived) != named:
            rows["rederived_disagrees"] += 1

        if str(eid) not in exported:
            rows["not_exported"] += 1
            continue
        shipped = exported[str(eid)]
        if shipped in (None, "", "-1", -1):
            rows["no_exported_level"] += 1
            continue
        if named is None:
            rows["no_named_level"] += 1
            continue
        if str(shipped) == named:
            rows["agree"] += 1
        else:
            rows["disagree"] += 1
            element = dict(element)
            element["level_exported"] = shipped
            disagreements.append(element)
    return rows, disagreements


# --------------------------------------------------------------------------
# Q6 - units
# --------------------------------------------------------------------------


def unit_ratios(probe, item_records):
    """The export's coordinates divided by the probe's, which are decimal feet.

    Self-proving, which is why it is worth doing rather than asserting: the same
    instance is measured twice in two places, so a ratio near 304.8 says the
    export converted to millimetres and a ratio near 1 says it did not. Nothing
    here reads duHast's source, so a change upstream shows up as a changed
    number rather than as a stale comment.

    Only X is compared, and only where both readings are far enough from zero
    for the division to mean anything -- a coordinate at the project origin
    divides into noise."""
    by_id = {}
    for record in item_records:
        value = dig(record, "instance_properties.id")
        if value is not None:
            by_id[str(value)] = record

    samples = []
    for eid, element in probe.get("elements", {}).items():
        record = by_id.get(str(eid))
        if record is None:
            continue
        point = (element.get("location") or {}).get("point")
        exported = dig(record, "location_point.translation_coord")
        if not point or not isinstance(exported, dict):
            continue
        feet = point.get("x")
        other = exported.get("x")
        if feet is None or other is None or abs(feet) < 1.0:
            continue
        samples.append(other / feet)

    if not samples:
        return {"n": 0, "median": None, "reading": "no comparable coordinates"}

    samples.sort()
    median = samples[len(samples) // 2]
    if abs(median - MM_PER_FOOT) / MM_PER_FOOT < UNIT_TOLERANCE:
        reading = "millimetres (D9 holds: the export converts, RoomMate divides)"
    elif abs(median - 1.0) < UNIT_TOLERANCE:
        reading = "decimal feet (D9 is WRONG for this field - no conversion needed)"
    else:
        reading = "neither mm nor feet - read the samples before writing any converter"
    return {"n": len(samples), "median": round(median, 4), "reading": reading}


# --------------------------------------------------------------------------
# U2 - the two bounding boxes
# --------------------------------------------------------------------------


def bounding_box_disagreement(probe):
    """Where duHast's oriented box and its solids box describe different
    volumes, restricted to instances that actually have sub-components.

    The measurement behind upstream change U2. The oriented box does not walk
    `GetSubComponentIds()` and the solids box does, so an instance with no
    sub-components should show no difference at all -- if it does, the cause is
    something other than nesting and U2 is not the whole fix.

    Compared on the Z extent alone. The two boxes are measured in DIFFERENT
    FRAMES -- the oriented one in the instance's own, with its placement on the
    transform -- so their X and Y are not comparable without applying that
    transform, while height is height in both."""
    nested = 0
    differing = 0
    without_subs_differing = 0
    worst = []
    for element in probe.get("elements", {}).values():
        boxes = element.get("duhast_bounding_boxes") or {}
        oriented, solids = boxes.get("oriented"), boxes.get("solids")
        if not oriented or not solids:
            continue
        subs = element.get("sub_component_count") or 0
        if subs:
            nested += 1
        height_o = oriented["max"]["z"] - oriented["min"]["z"]
        height_s = solids["max"]["z"] - solids["min"]["z"]
        delta = abs(height_o - height_s)
        if delta <= 0.01:
            continue
        if subs:
            differing += 1
        else:
            without_subs_differing += 1
        worst.append((delta, element.get("id"), element.get("type_name"), subs))
    # Sorted on the delta alone: a tuple sort would fall through to the id
    # on a tie, and an instance whose id could not be read carries None there.
    worst.sort(key=lambda entry: entry[0], reverse=True)
    return {
        "nested": nested,
        "differing_with_subs": differing,
        "differing_without_subs": without_subs_differing,
        "worst": worst[:10],
    }


# --------------------------------------------------------------------------
# Report
# --------------------------------------------------------------------------


def pct(n, total):
    return 0.0 if not total else round(100.0 * n / total, 1)


def verdict(total_items, phase_rows, coverage, category_mismatch):
    """The one line the plan actually waits on.

    Ordered so the cheap, recoverable answers come first: a model with no
    furniture is not a finding about FFE, and a probe measuring a different
    category set from the export is a finding about the probe. Only when
    neither is true does a zero become the kill condition."""
    if category_mismatch:
        return (
            "INVALID - the probe walked a different category set from the export "
            "({}). Every count below is measuring two populations. Fix the probe "
            "and re-run.".format(category_mismatch)
        )
    if total_items == 0:
        return (
            "NO FFE IN THIS MODEL - nothing was collected in any of the eight "
            "categories. This is NOT the kill condition; find a model with "
            "furniture and equipment in it and re-run."
        )

    best = max(phase_rows, key=lambda r: r["named"]) if phase_rows else None
    if best is None or best["named"] == 0:
        return (
            "BLOCKED - no item names a room in any phase, on a model that holds "
            "both. There is no join to perform, and the entity carries little "
            "rooms do not already have. This is the kill condition."
        )

    empty = [
        r["field"]
        for r in coverage
        if r["field"] in REQUIRED_ITEM_FIELDS and r["count"] == 0
    ]
    if empty:
        return (
            "BLOCKED - no item carries: {}. The Item record as D1 describes it "
            "is wrong.".format(", ".join(empty))
        )

    notes = []
    if best["pct"] < THIN_ROOM_REFERENCE_PCT:
        notes.append(
            "only {}% of items name a room in the best phase ({})".format(
                best["pct"], best["phase"]
            )
        )
    thin = [
        r["field"]
        for r in coverage
        if r["field"] in REQUIRED_ITEM_FIELDS and r["pct"] < 95.0
    ]
    if thin:
        notes.append("thinly populated: {}".format(", ".join(thin)))
    if notes:
        return "PROCEED WITH CARE - {}. Read the tables.".format("; ".join(notes))

    return (
        "PROCEED - {} items, {}% naming a room in phase '{}'. PR C may start.".format(
            total_items, best["pct"], best["phase"]
        )
    )


def table(headers, rows):
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join(["---"] * len(headers)) + "|"]
    for row in rows:
        out.append("| " + " | ".join(str(c) for c in row) + " |")
    return "\n".join(out)


def build_report(data):
    probe = data["ffe_probe"]
    item_records = records(data["ffe_raw"], probe.get("list_key", "item"))
    door_records = records(data["doors_raw"], "door")

    # A probe that walked a different category set from the export is measuring
    # two populations, so this is checked before anything is counted.
    probe_cats = probe.get("probe_categories") or []
    duhast_cats = probe.get("duhast_default_categories")
    category_mismatch = ""
    if duhast_cats is not None and sorted(probe_cats) != sorted(duhast_cats):
        category_mismatch = "probe {} vs duHast {}".format(
            len(probe_cats), len(duhast_cats)
        )

    phase_rows, total_items = room_reference_by_phase(probe)
    coverage = field_coverage(item_records, ITEM_FIELDS)
    door_coverage = field_coverage(door_records, DOOR_FIELDS)
    only_items, only_doors = structural_diff(item_records, door_records)
    drop = dropped_by_export(probe, item_records)
    matrix = super_component_matrix(probe)
    levels, level_disagreements = level_agreement(probe, item_records)
    units = unit_ratios(probe, item_records)
    boxes = bounding_box_disagreement(probe)

    lines = []
    add = lines.append

    add("# FFE probe report")
    add("")
    add("> **{}**".format(verdict(total_items, phase_rows, coverage, category_mismatch)))
    add("")
    document = probe.get("document", {})
    add("- document: `{}`".format(document.get("title")))
    add("- probe version: {}".format(probe.get("probe_version")))
    add("- door control: {}".format(data["control_note"]))
    add("- collected: {} instances / exported: {}".format(drop["collected"], drop["exported"]))
    if probe.get("unknown_category_names"):
        add(
            "- **this Revit version has no BuiltInCategory for: {}**".format(
                ", ".join(probe["unknown_category_names"])
            )
        )
    if probe.get("collected_duplicates"):
        add("- {} duplicate collection(s), first record kept".format(probe["collected_duplicates"]))
    add("")

    problems = probe.get("serialisation_problems") or []
    if problems:
        add("## Fields duHast could not serialise")
        add("")
        add(
            "A field that will not convert is a field the contract cannot carry, "
            "whatever the plan assumed about it. Without `plainify` these would "
            "have produced no export file at all."
        )
        add("")
        add(
            table(
                ["path", "type", "detail"],
                [
                    (p.get("path"), p.get("type"), p.get("error") or p.get("repr"))
                    for p in problems[:20]
                ],
            )
        )
        add("")

    add("## Q1 - does `get_Room(phase)` populate?")
    add("")
    add(
        "The kill condition. FFE and rooms live in one file, so Revit should know "
        "which room an item is in; RoomMate pushes exactly one phase, so what "
        "matters is the best row, not the total."
    )
    add("")
    add(
        table(
            ["phase", "items naming a room", "%", "lookup errors"],
            [(r["phase"], r["named"], r["pct"], r["errors"]) for r in phase_rows],
        )
    )
    add("")

    add("## Q2 - what the export dropped")
    add("")
    add(
        "`populate_data_item_object` returns None for anything that is not a "
        "`LocationPoint` and the caller does not append it, so the loss leaves no "
        "hole. Anything dropped that DOES have a `LocationPoint` is a second, "
        "unexplained loss the plan did not predict."
    )
    add("")
    add("- dropped: **{}** of {} collected".format(len(drop["dropped"]), drop["collected"]))
    add("- unexplained (had a LocationPoint and still vanished): **{}**".format(len(drop["unexplained"])))
    add("")
    if drop["by_category"]:
        add(table(["category", "dropped"], sorted(drop["by_category"].items(), key=lambda kv: str(kv[0]))))
        add("")
    if drop["by_location_class"]:
        add(
            table(
                ["location class", "dropped"],
                sorted(drop["by_location_class"].items(), key=lambda kv: str(kv[0])),
            )
        )
        add("")

    add("## Q3 - category histogram")
    add("")
    add(
        "All eight of duHast's defaults, `OST_GenericModel` included, because "
        "that is what D6 commits to pushing. Whether that was wise is what this "
        "table is for."
    )
    add("")
    collected = probe.get("collected_by_category") or {}
    add(
        table(
            ["category", "collected"],
            sorted(collected.items(), key=lambda kv: -kv[1]),
        )
    )
    add("")

    add("## Q4 - super-component matrix")
    add("")
    add(
        "`nested_opening_ids` asks 'is the parent the same category', which is a "
        "yes/no for one category and a matrix for eight. The off-diagonal rows "
        "are the ones that decide whether the filter can stay in code or has to "
        "become an `[ffe]` setting. Note U2 wants the OPPOSITE answer about the "
        "same elements: what is excluded here as a leaf is what the bounding box "
        "should be including as geometry."
    )
    add("")
    if matrix["matrix"]:
        add(
            "- nested instances: **{}** of {} ({}% of everything collected)".format(
                matrix["nested"],
                matrix["nested"] + matrix["top"],
                pct(matrix["nested"], matrix["nested"] + matrix["top"]),
            )
        )
        add(
            "- of those, parent is the SAME category: **{}**. The "
            "`nested_opening_ids` test would catch {}% of them.".format(
                matrix["same_category"], pct(matrix["same_category"], matrix["nested"])
            )
        )
        add("")
        add(
            table(
                ["child category", "parent category", "count", "same?"],
                [
                    (child, parent, n, "yes" if child == parent else "NO")
                    for (child, parent), n in sorted(
                        matrix["matrix"].items(), key=lambda kv: -kv[1]
                    )
                ],
            )
        )
        add("")
        add(
            "**Which discriminator works.** The doors finding was that a "
            "component carries neither a room reference nor a Mark. Only one "
            "half of that transfers, and the table below is why the filter "
            "cannot be written from the doors experience alone."
        )
        add("")
        add(
            table(
                ["", "names a room", "carries a Mark"],
                [
                    (
                        "nested ({})".format(matrix["nested"]),
                        "{} ({}%)".format(
                            matrix["nested_with_room"],
                            pct(matrix["nested_with_room"], matrix["nested"]),
                        ),
                        "{} ({}%)".format(
                            matrix["nested_with_mark"],
                            pct(matrix["nested_with_mark"], matrix["nested"]),
                        ),
                    ),
                    (
                        "top-level ({})".format(matrix["top"]),
                        "{} ({}%)".format(
                            matrix["top_with_room"],
                            pct(matrix["top_with_room"], matrix["top"]),
                        ),
                        "{} ({}%)".format(
                            matrix["top_with_mark"],
                            pct(matrix["top_with_mark"], matrix["top"]),
                        ),
                    ),
                ],
            )
        )
    else:
        add("No nested instances found.")
    add("")

    add("## Q5 - the level heuristic")
    add("")
    add(
        "duHast derives an item's level from its solid bounding box rather than "
        "reading it, so a ceiling-mounted item can be assigned to the storey "
        "below the one it serves -- and unlike `UnknownLevel`, that answer looks "
        "correct. It matters less for attribution than it looks (authored `Room` "
        "carries that) and more for the viewer, where a wrong level draws the "
        "item on the wrong floor plan."
    )
    add("")
    add(
        table(
            ["outcome", "items"],
            [
                ("exported level agrees with the named level", levels["agree"]),
                ("exported level DISAGREES", levels["disagree"]),
                ("instance names no level - the heuristic is the only answer", levels["no_named_level"]),
                ("export carries no level (no solid geometry)", levels["no_exported_level"]),
                ("collected but not exported", levels["not_exported"]),
                ("fell back: below every level in the document", levels["fallback_below_all_levels"]),
            ],
        )
    )
    add("")
    add(
        "Diagnostic only, **not** the count above: re-running duHast's rule "
        "against Revit's own element box rather than its solids box disagrees "
        "with the named level on {} items. The gap between that and the {} the "
        "export actually disagrees on is the measure of how badly re-deriving a "
        "producer's rule answers a question about the producer.".format(
            levels["rederived_disagrees"], levels["disagree"]
        )
    )
    add("")
    if level_disagreements:
        add("Ten worst disagreements:")
        add("")
        add(
            table(
                ["id", "family", "names", "export says", "bbox min z (ft)"],
                [
                    (
                        e.get("id"),
                        e.get("family_name") or e.get("type_name"),
                        named_level(e),
                        e.get("level_exported"),
                        round(e.get("bbox_min_z"), 3) if e.get("bbox_min_z") is not None else None,
                    )
                    for e in level_disagreements[:10]
                ],
            )
        )
        add("")

    add("## Q6 - units")
    add("")
    add(
        "The same instance measured twice: the export's X divided by the probe's, "
        "which is Revit's internal decimal feet. Self-proving, so a change "
        "upstream shows up as a changed number rather than a stale comment."
    )
    add("")
    add("- samples: {}".format(units["n"]))
    add("- median ratio: {}".format(units["median"]))
    add("- reading: **{}**".format(units["reading"]))
    add("")

    add("## D1 - is an item an opening?")
    add("")
    add(
        "The plan's central claim, tested from both directions. The coverage "
        "table says what an item actually carries; the structural diff says what "
        "each entity has that the other does not. For windows this diff was "
        "expected to be empty and was. Here it is expected to be full -- so a "
        "surprisingly short table is as interesting as a long one."
    )
    add("")
    add("**What an item carries** ({} records)".format(len(item_records)))
    add("")
    add(
        table(
            ["field", "export path", "populated", "%"],
            [(r["field"], "`" + r["path"] + "`", r["count"], r["pct"]) for r in coverage],
        )
    )
    add("")
    add("**What a door carries** ({} records, the control)".format(len(door_records)))
    add("")
    add(
        table(
            ["field", "export path", "populated", "%"],
            [(r["field"], "`" + r["path"] + "`", r["count"], r["pct"]) for r in door_coverage],
        )
    )
    add("")
    add("**Key paths on an item and not a door:** {}".format(
        ", ".join("`{}`".format(p) for p in only_items) or "none"
    ))
    add("")
    add("**Key paths on a door and not an item:** {}".format(
        ", ".join("`{}`".format(p) for p in only_doors) or "none"
    ))
    add("")

    add("## U2 - the two bounding boxes")
    add("")
    add(
        "The oriented box keeps the rotation but measures only "
        "`family_instance.get_Geometry()`; the solids box walks "
        "`GetSubComponentIds()` but is world-aligned. Compared on the Z extent "
        "alone, because the two are measured in different frames and height is "
        "the one dimension comparable without applying a transform."
    )
    add("")
    add("- instances with sub-components: **{}**".format(boxes["nested"]))
    add("- of those, boxes disagree on height: **{}**".format(boxes["differing_with_subs"]))
    add(
        "- disagree WITHOUT sub-components: **{}** (if non-zero, nesting is not "
        "the whole cause and U2 is not the whole fix)".format(
            boxes["differing_without_subs"]
        )
    )
    add("")
    if boxes["worst"]:
        add(
            table(
                ["height delta (ft)", "id", "type", "sub-components"],
                [(round(d, 3), i, t, s) for d, i, t, s in boxes["worst"]],
            )
        )
        add("")

    return "\n".join(lines) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--dir", default=DEFAULT_DIR, help="directory holding the probe files")
    parser.add_argument("--out", default=None, help="report path (default: <dir>/ffe-probe-report.md)")
    args = parser.parse_args(argv)

    data = load_inputs(args.dir)
    report = build_report(data)

    out = args.out or os.path.join(args.dir, "ffe-probe-report.md")
    # Binary, for the LF reason `probe_ffe_export.write_json` gives.
    with open(out, "wb") as handle:
        handle.write(report.encode("utf-8"))
    print(report)
    print("wrote {}".format(out))


if __name__ == "__main__":
    main()
