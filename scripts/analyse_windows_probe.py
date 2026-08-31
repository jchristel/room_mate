#!/usr/bin/env python3
"""Answer the three questions that block the windows plan, from the files
`probe_windows_export.py` captured in Revit.

    python scripts/analyse_windows_probe.py
    python scripts/analyse_windows_probe.py --dir some/other/fixtures

    Q1  Does every field the plan assumes actually arrive, populated -- and is
        the window record structurally identical to a door's?
    Q2  How many windows are curtain-wall panels or nested components, and do
        those carry room references or a Mark?
    Q3  How many windows report a level that is not the storey they serve?

Writes `windows-probe-report.md` beside the inputs and prints the same thing.

**This is where every judgement lives.** The probe collects and counts; nothing
in it decides whether a curtain-wall panel is a window or whether a level is
wrong. Keeping the deciding here means it can be re-run, argued with and
corrected without going back to Revit -- which matters because the answers
change the plan, and the plan is what the argument is actually about.

Stdlib only, CPython 3. It reads JSON and prints; it must run wherever the data
lands, including a machine with no checkout of this repo beside it.
"""

import argparse
import json
import os
import sys
from collections import Counter, OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_DIR = os.path.join(HERE, "fixtures")

# Revit's *uninitialized* BoundingBoxXYZ arrives as min +1e30 / max -1e30 and
# passes duHast's own guards, so it reads as a plausible footprint rather than
# an absent one. Tested by magnitude, never by equality with 1e30: the value
# crosses a float round-trip, and the point is to catch the class.
SENTINEL_MAGNITUDE = 1e20

# How close two elevations must be to count as the same level, in decimal feet.
# Generous on purpose -- a level at 3000.0000001 mm is the same level.
LEVEL_EPS = 0.01

# The contract fields the plan assumes a window carries, and where each is read
# from in the raw export. This table IS the question: if a row comes back empty
# on windows and full on doors, the shared-record premise is wrong and PR A does
# not start. Paths are dotted; `[]` means "the first entry of a list".
CONTRACT_FIELDS = OrderedDict(
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
        ("design_set", "design_set_and_option.set_name"),
        ("room_calculation_point", "room_calculation_point"),
        ("bounding_box_min_z", "bounding_box_min_z"),
        ("bounding_box_max_z", "bounding_box_max_z"),
    ]
)


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
    windows_raw = os.path.join(directory, "windows-raw.json")
    windows_probe = os.path.join(directory, "windows-probe.json")
    missing = [p for p in (windows_raw, windows_probe) if not os.path.isfile(p)]
    if missing:
        raise SystemExit(
            "missing {}\n\nRun scripts/probe_windows_export.py from pyRevit "
            "first; it writes these into scripts/fixtures/.".format(
                ", ".join(os.path.basename(m) for m in missing)
            )
        )

    control = os.path.join(directory, "doors-raw-control.json")
    control_note = "same document, same duHast"
    if not os.path.isfile(control):
        control = os.path.join(directory, "doors-raw.json")
        control_note = "committed fixture, OLDER duHast - a weaker comparison"
    doors_raw = load(control) if os.path.isfile(control) else None

    return {
        "windows_raw": load(windows_raw),
        "windows_probe": load(windows_probe),
        "doors_raw": doors_raw,
        "control_note": control_note if doors_raw else "no door control available",
    }


def records(raw, list_key):
    return (raw or {}).get(list_key, []) or []


# --------------------------------------------------------------------------
# Q1 - field coverage and the structural diff
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


def field_coverage(window_records, door_records):
    """Per contract field: how many of each entity carry it populated."""
    rows = []
    for field, path in CONTRACT_FIELDS.items():
        win = sum(1 for r in window_records if is_populated(dig(r, path)))
        door = sum(1 for r in door_records if is_populated(dig(r, path)))
        rows.append(
            {
                "field": field,
                "path": path,
                "windows": win,
                "windows_pct": pct(win, len(window_records)),
                "doors": door,
                "doors_pct": pct(door, len(door_records)),
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


def structural_diff(window_records, door_records):
    """Key paths present on one entity and not the other, unioned across every
    record so an optional field on one instance is not read as a schema
    difference."""
    win = set()
    for r in window_records:
        win |= key_paths(r)
    door = set()
    for r in door_records:
        door |= key_paths(r)
    return sorted(win - door), sorted(door - win)


def geometry_health(window_records):
    """The footprint story: how many windows have a usable outer loop, how many
    are empty, and how many are the 1e30 sentinel wearing a footprint's
    clothes."""
    empty, sentinel, ok = 0, 0, 0
    for record in window_records:
        loop = dig(record, "polygon[].outer_loop")
        if not loop:
            empty += 1
        elif any(abs(c) >= SENTINEL_MAGNITUDE for pt in loop for c in pt):
            sentinel += 1
        else:
            ok += 1
    return {"ok": ok, "empty": empty, "sentinel": sentinel}


def export_phase_ids_resolve(window_records, phases):
    """Whether the export's own `to_room` / `from_room` `phase_id` values resolve
    against the document's phase list.

    The doors work concluded they "resolve against nothing on the wire", which is
    true of the wire and not necessarily of the model -- the probe captured the
    phase table, so the claim is now checkable. If they do resolve, the extractor
    could in principle read room references from the export instead of from the
    Revit API. That would still be the wrong choice (the API answers one room per
    phase directly), but it should be rejected on the record rather than on a
    belief."""
    known = set(str(p["id"]) for p in phases)
    seen, resolved = set(), set()
    for record in window_records:
        for side in ("from_room", "to_room"):
            for entry in record.get(side) or []:
                pid = str(entry.get("phase_id"))
                seen.add(pid)
                if pid in known:
                    resolved.add(pid)
    return {"seen": sorted(seen), "resolved": sorted(resolved), "known": sorted(known)}


# --------------------------------------------------------------------------
# Q2 - curtain-wall panels and nested components
# --------------------------------------------------------------------------


def has_any_room(element):
    """Whether this element names a room in ANY phase. Deliberately generous:
    the question is whether the population is room-bearing at all, and requiring
    a specific phase would mix Q2 up with the phase filter."""
    for entry in element.get("rooms_by_phase") or []:
        if entry.get("fromroom") or entry.get("toroom"):
            return True
    return False


def population_stats(elements, predicate):
    """`{count, with_mark, with_room}` for the subset matching `predicate`.

    Those three together are the nested-door test that mattered last time: 2236
    false doors carried no room reference and almost no Mark, and it was that
    pairing -- not the raw count -- that showed they were components rather than
    a modelling gap."""
    subset = [e for e in elements if predicate(e)]
    return {
        "count": len(subset),
        "with_mark": sum(1 for e in subset if e.get("mark")),
        "with_room": sum(1 for e in subset if has_any_room(e)),
    }


def curtain_and_nested(probe):
    elements = list(probe.get("elements", {}).values())
    total = len(elements)

    def hosted(e):
        host = e.get("host") or {}
        return bool(host.get("is_curtain_wall"))

    def panel_type(e):
        return bool(e.get("is_curtain_panel_type"))

    def either(e):
        return hosted(e) or panel_type(e)

    return {
        "total": total,
        "all": population_stats(elements, lambda e: True),
        "curtain_hosted": population_stats(elements, hosted),
        "curtain_panel_type": population_stats(elements, panel_type),
        "curtain_either": population_stats(elements, either),
        "ordinary": population_stats(elements, lambda e: not either(e)),
        "nested": population_stats(elements, lambda e: e.get("is_nested_in_same_category")),
        "families": Counter(
            e.get("family_name") or "(unknown)" for e in elements if either(e)
        ).most_common(10),
    }


def phase_table(probe):
    """Per phase: how many elements exist in it, and how their room references
    fall out. Reported for every phase rather than one chosen phase, so the
    analyser needs no phase argument and a model whose windows live in an
    unexpected phase is visible rather than empty.

    `one_sided` is the column to read for windows. For a door it counts the
    exceptions -- external doors, 6 of the 26 in the House A sample. For a window
    it should be most of the set, because every facade window has a room on one
    side and the weather on the other. If it is NOT most of the set, the
    references are not being populated and the geometric fallback
    (`[windows] room_resolution`) stops being optional."""
    elements = list(probe.get("elements", {}).values())
    rows = []
    for phase in probe.get("phases", []):
        name = phase["name"]
        exists = [e for e in elements if name in (e.get("phase") or {}).get("exists_in", [])]
        counts = Counter()
        for element in exists:
            entry = next(
                (r for r in element.get("rooms_by_phase") or [] if r.get("phase_name") == name),
                {},
            )
            has_from, has_to = bool(entry.get("fromroom")), bool(entry.get("toroom"))
            counts["from"] += 1 if has_from else 0
            counts["to"] += 1 if has_to else 0
            if has_from and has_to:
                counts["both"] += 1
            elif has_from or has_to:
                counts["one_sided"] += 1
            else:
                counts["neither"] += 1
        rows.append(
            {
                "phase": name,
                "exists": len(exists),
                "from_room": counts["from"],
                "to_room": counts["to"],
                "both": counts["both"],
                "one_sided": counts["one_sided"],
                "neither": counts["neither"],
            }
        )
    return rows


# --------------------------------------------------------------------------
# Q3 - level versus storey
# --------------------------------------------------------------------------


def storey_by_sill_rule(min_z, max_z, levels):
    """duHast's storey rule, applied here so the comparison is against a stated
    rule rather than against an intuition.

    The level below the sill, unless the element crosses a level -- then the
    lowest level it crosses, unless more than half its height sits below that
    level, in which case it stays on the level below. Self-scaling, so it needs
    no tolerance constant: a floor-to-ceiling window whose sill is a few
    millimetres low correctly snaps up rather than dropping a storey."""
    if min_z is None or max_z is None or not levels:
        return None

    below = [l for l in levels if l["elevation"] <= min_z + LEVEL_EPS]
    base = below[-1] if below else levels[0]

    crossed = [l for l in levels if min_z + LEVEL_EPS < l["elevation"] < max_z - LEVEL_EPS]
    if not crossed:
        return base

    lowest = crossed[0]
    height = max_z - min_z
    if height > 0 and (lowest["elevation"] - min_z) > height / 2.0:
        return base
    return lowest


def level_report(probe):
    """Every element whose reported level disagrees with the sill rule, plus the
    two coarser flags that do not depend on the rule at all."""
    levels = probe.get("levels", [])
    by_id = {str(l["id"]): l for l in levels}
    elements = list(probe.get("elements", {}).values())

    disagree, above_next, crosses, unmeasurable = [], [], [], 0
    sills = []

    for element in elements:
        min_z, max_z = element.get("bbox_min_z"), element.get("bbox_max_z")
        reported = by_id.get(str(element.get("level_id")))
        if min_z is None or max_z is None or reported is None:
            unmeasurable += 1
            continue

        sills.append(min_z - reported["elevation"])

        above = [l for l in levels if l["elevation"] > reported["elevation"] + LEVEL_EPS]
        next_up = above[0] if above else None
        if next_up is not None:
            if min_z >= next_up["elevation"] - LEVEL_EPS:
                above_next.append(element)
            elif min_z < next_up["elevation"] <= max_z:
                crosses.append(element)

        storey = storey_by_sill_rule(min_z, max_z, levels)
        if storey is not None and str(storey["id"]) != str(reported["id"]):
            disagree.append(
                {
                    "id": element["id"],
                    "mark": element.get("mark"),
                    "reported": reported["name"],
                    "sill_rule": storey["name"],
                    "sill_above_reported": round(min_z - reported["elevation"], 3),
                }
            )

    return {
        "total": len(elements),
        "unmeasurable": unmeasurable,
        "disagree": disagree,
        "above_next_level": len(above_next),
        "crosses_level": len(crosses),
        "sill_offsets": summarise(sills),
    }


def summarise(values):
    if not values:
        return None
    ordered = sorted(values)
    return {
        "min": round(ordered[0], 2),
        "median": round(ordered[len(ordered) // 2], 2),
        "max": round(ordered[-1], 2),
    }


# --------------------------------------------------------------------------
# Report
# --------------------------------------------------------------------------


def pct(n, total):
    return 0.0 if not total else round(100.0 * n / total, 1)


def verdict(coverage, geometry, structural_extra_doors):
    """The one line the plan actually waits on.

    A field the plan assumes and no window carries is disqualifying; the shared
    record is wrong and PR A does not start. Everything else is a note."""
    assumed = ("id", "level_id", "loops", "type_id", "type_name")
    empty = [r["field"] for r in coverage if r["field"] in assumed and r["windows"] == 0]
    if empty:
        return "BLOCKED - no window carries: {}. The shared record is wrong.".format(
            ", ".join(empty)
        )
    thin = [r["field"] for r in coverage if r["field"] in assumed and r["windows_pct"] < 95.0]
    if thin or geometry["sentinel"]:
        notes = []
        if thin:
            notes.append("thinly populated: {}".format(", ".join(thin)))
        if geometry["sentinel"]:
            notes.append("{} footprint(s) at the 1e30 sentinel".format(geometry["sentinel"]))
        return "PROCEED WITH CARE - {}. Read the tables.".format("; ".join(notes))
    if structural_extra_doors:
        return "PROCEED - doors carry {} path(s) windows do not; check they are unused.".format(
            len(structural_extra_doors)
        )
    return "PROCEED - the record is identical and populated. PR A may start."


def table(headers, rows):
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join(["---"] * len(headers)) + "|"]
    for row in rows:
        out.append("| " + " | ".join(str(c) for c in row) + " |")
    return "\n".join(out)


def build_report(data):
    win_probe = data["windows_probe"]
    win_records = records(data["windows_raw"], win_probe.get("list_key", "window"))
    door_records = records(data["doors_raw"], "door")

    coverage = field_coverage(win_records, door_records)
    only_windows, only_doors = structural_diff(win_records, door_records)
    geometry = geometry_health(win_records)
    q2 = curtain_and_nested(win_probe)
    q3 = level_report(win_probe)
    phases = export_phase_ids_resolve(win_records, win_probe.get("phases", []))

    lines = []
    add = lines.append

    add("# Windows probe report")
    add("")
    add("- Document: `{}`".format((win_probe.get("document") or {}).get("title")))
    add("- Collected by the probe: **{}**".format(win_probe.get("collected_count")))
    add("- Survived the duHast export: **{}**".format(win_probe.get("exported_count")))
    add("- Door control: {}".format(data["control_note"]))
    add("")
    add("> **{}**".format(verdict(coverage, geometry, only_doors)))
    add("")

    add("## Q1 - does the record arrive, and is it a door's?")
    add("")
    add(
        table(
            ["Field", "Export path", "Windows", "%", "Doors", "%"],
            [
                [r["field"], "`{}`".format(r["path"]), r["windows"], r["windows_pct"], r["doors"], r["doors_pct"]]
                for r in coverage
            ],
        )
    )
    add("")
    add(
        "Footprints: **{ok} usable**, {empty} empty, **{sentinel} at the 1e30 "
        "sentinel**.".format(**geometry)
    )
    add("")
    add("Key paths on windows only: {}".format(", ".join("`%s`" % p for p in only_windows) or "_none_"))
    add("Key paths on doors only: {}".format(", ".join("`%s`" % p for p in only_doors) or "_none_"))
    add("")
    add(
        "Export `phase_id` values seen: {} - of which resolve against the document's "
        "phases: {}.".format(phases["seen"] or "_none_", phases["resolved"] or "_none_")
    )
    add("")

    add("## Q2 - curtain-wall panels and nested components")
    add("")
    add(
        table(
            ["Population", "Count", "% of all", "With a Mark", "Names a room"],
            [
                [
                    label,
                    q2[key]["count"],
                    pct(q2[key]["count"], q2["total"]),
                    q2[key]["with_mark"],
                    q2[key]["with_room"],
                ]
                for label, key in [
                    ("All windows", "all"),
                    ("Hosted in a curtain wall", "curtain_hosted"),
                    ("Type is a curtain panel", "curtain_panel_type"),
                    ("Either curtain signal", "curtain_either"),
                    ("Ordinary (neither)", "ordinary"),
                    ("Nested in another window", "nested"),
                ]
            ],
        )
    )
    add("")
    if q2["families"]:
        add("Top curtain-wall families: " + ", ".join(
            "`{}` x{}".format(name, count) for name, count in q2["families"]
        ))
        add("")
    add("Per phase:")
    add("")
    add(
        table(
            ["Phase", "Exists", "from_room", "to_room", "Both sides", "One side", "Neither"],
            [
                [r["phase"], r["exists"], r["from_room"], r["to_room"], r["both"], r["one_sided"], r["neither"]]
                for r in phase_table(win_probe)
            ],
        )
    )
    add("")

    add("## Q3 - is the reported level the storey it serves?")
    add("")
    add("- Windows measured: **{}** ({} could not be).".format(q3["total"] - q3["unmeasurable"], q3["unmeasurable"]))
    add("- Reported level disagrees with duHast's sill rule: **{}**".format(len(q3["disagree"])))
    add("- Sits entirely above the next level up: **{}**".format(q3["above_next_level"]))
    add("- Spans a level: **{}**".format(q3["crosses_level"]))
    if q3["sill_offsets"]:
        add(
            "- Sill above its own level (ft): min {min}, median {median}, max {max}".format(
                **q3["sill_offsets"]
            )
        )
    add("")
    if q3["disagree"]:
        add("First 20 disagreements:")
        add("")
        add(
            table(
                ["Element", "Mark", "Reported level", "Sill rule says", "Sill above reported (ft)"],
                [
                    [d["id"], d["mark"] or "-", d["reported"], d["sill_rule"], d["sill_above_reported"]]
                    for d in q3["disagree"][:20]
                ],
            )
        )
        add("")

    return "\n".join(lines) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--dir", default=DEFAULT_DIR, help="where the probe wrote its JSON")
    parser.add_argument("--out", default=None, help="report path (default: <dir>/windows-probe-report.md)")
    args = parser.parse_args(argv)

    data = load_inputs(args.dir)
    report = build_report(data)

    out = args.out or os.path.join(args.dir, "windows-probe-report.md")
    # Newline "\n" explicitly: .gitattributes enforces LF and the default on
    # Windows would write CRLF into a file destined for the repo.
    with open(out, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(report)

    print(report)
    print("written: {}".format(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
