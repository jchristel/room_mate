#!/usr/bin/env python3
"""Read `scripts/fixtures/room-state-probe.json` (from `probe_room_state.py`) and answer the two
questions PLAN-blueprint-room-state.md leaves for a Revit run.

    python scripts/analyse_room_state_probe.py [path-to-probe.json]

Judgements live here, not in the probe, so they can be re-run and argued with.
"""

import json
import os
import statistics
import sys

PLACE_TOLERANCE_MM = 5.0
TURN_TOLERANCE_DEG = 5.0

# Placement types Revit names that need no wall or face: the first iteration's scope.
LEVEL_BASED_TYPES = {"OneLevelBased", "WorkPlaneBased", "TwoLevelsBased"}


def load(path):
    with open(path, "rb") as handle:
        return json.loads(handle.read().decode("utf-8"))


def hosting(report):
    """Part A: does a rule separate level-based from wall/face-hosted, and which one."""
    records = report["hosting"]["instances"]
    groups = report["hosting"]["groups"]
    print("== A. Hosting census: {} FF&E instances ==".format(len(records)))
    for key in sorted(groups, key=lambda k: -groups[k]["count"]):
        group = groups[key]
        print("  {:5d}  ({} nested)  {}   e.g. {}".format(group["count"], group["components"], key, group["examples"]))

    hosts = [r for r in records if not r["is_component"]]

    def by_placement(record):
        return record["placement_type"] in LEVEL_BASED_TYPES

    def by_shipped_host(record):
        # what the plan said it would do: no `host` means level-based
        return record["host"] is None

    print()
    print("  Rule 1, family placement type in {}:".format(sorted(LEVEL_BASED_TYPES)))
    rule_1 = [r for r in hosts if by_placement(r)]
    print("    supported {} / {}".format(len(rule_1), len(hosts)))
    print("  Rule 2, shipped `host` is None (the extractor's reading today):")
    rule_2 = [r for r in hosts if by_shipped_host(r)]
    print("    supported {} / {}".format(len(rule_2), len(hosts)))

    disagree = [r for r in hosts if by_placement(r) != by_shipped_host(r)]
    print("  The two rules disagree on {} instance(s).".format(len(disagree)))
    shown = {}
    for r in disagree:
        key = (r["placement_type"], r["host_class"], r["has_host_face"], r["host_param"])
        shown.setdefault(key, []).append(r["id"])
    for key, ids in sorted(shown.items(), key=lambda kv: -len(kv[1])):
        print("    {}  x{}  e.g. {}".format(key, len(ids), ids[:5]))

    work_plane = [r for r in hosts if r["placement_type"] == "WorkPlaneBased"]
    if work_plane:
        classes = sorted(set((r["host_class"], r["host_category"], r["has_host_face"]) for r in work_plane))
        print("  WorkPlaneBased families answer `Host` with: {}".format(classes))
        print("    -> a Level here means the shipped `host` marks them hosted, a ReferencePlane or a face means a real host.")

    unreadable = [r for r in hosts if r["placement_type"] is None]
    if unreadable:
        print("  {} instance(s) with an unreadable placement type (the plan flags these unsupported).".format(len(unreadable)))
    nolocation = [r for r in hosts if r["location_class"] != "LocationPoint"]
    if nolocation:
        print("  {} instance(s) with no point location (the export drops them): {}".format(
            len(nolocation), sorted(set(r["location_class"] for r in nolocation))))
    print()


def round_trip(report):
    """Part B: the offset between what was asked and what the export reads."""
    trip = report["round_trip"]
    print("== B. Placement round trip ==")
    if "skipped" in trip:
        print("  SKIPPED: {}".format(trip["skipped"]))
        return
    print("  copied {} / {} (level {})".format(trip["reference_family"], trip["reference_type"], trip["level_id"]))
    print("  placed {}, rolled back: {} of them no longer resolve".format(trip["placed"], trip["rolled_back_ids_gone"]))
    if trip["rolled_back_ids_gone"] != trip["placed"]:
        print("  WARNING: the rollback did not remove everything. Check the model before saving.")
    print()
    print("  {:>5} {:>9} {:>9} {:>9} {:>8} {:>8}".format("tag", "turn", "api mm", "export mm", "turn err", "uid?"))
    export_errors = []
    api_errors = []
    turn_errors = []
    for row in trip["rows"]:
        e = row.get("export_err_mm")
        a = row.get("api_err_mm")
        t = row.get("export_turn_err_deg")
        if e is not None:
            export_errors.append(e)
        if a is not None:
            api_errors.append(a)
        if t is not None:
            turn_errors.append(abs(t))
        print("  {:>5} {:>9} {:>9} {:>9} {:>8} {:>8}".format(
            row["tag"],
            row["asked_turn_deg"],
            "n/a" if a is None else "{:.3f}".format(a),
            "n/a" if e is None else "{:.3f}".format(e),
            "n/a" if t is None else "{:.3f}".format(t),
            "yes" if row.get("unique_id") else "NO"))
    print()
    if not export_errors:
        print("  The export's converter returned nothing: nothing to judge.")
        return
    near = [row["export_err_mm"] for row in trip["rows"] if row["tag"] == "near" and "export_err_mm" in row]
    far = [row["export_err_mm"] for row in trip["rows"] if row["tag"] == "far" and "export_err_mm" in row]
    print("  position error, export vs asked: max {:.3f} mm, median {:.3f} mm".format(max(export_errors), statistics.median(export_errors)))
    if near:
        print("    near the model: max {:.3f} mm".format(max(near)))
    if far:
        print("    1 km out:       max {:.3f} mm".format(max(far)))
    if turn_errors:
        print("  turn error: max {:.3f} deg".format(max(turn_errors)))
    verdict = "HOLDS" if max(export_errors) <= PLACE_TOLERANCE_MM else "DOES NOT HOLD"
    print("  {} mm place tolerance {} (worst {:.3f} mm).".format(PLACE_TOLERANCE_MM, verdict, max(export_errors)))
    if turn_errors:
        verdict = "HOLDS" if max(turn_errors) <= TURN_TOLERANCE_DEG else "DOES NOT HOLD"
        print("  {} degree turn tolerance {}.".format(TURN_TOLERANCE_DEG, verdict))
    if max(export_errors) > PLACE_TOLERANCE_MM:
        print("  A systematic offset (same sign and size on every row) means the converter reads a point other than the")
        print("  insertion point, and the audit should compare against that, not the tolerance.")
    has_uid = any(row.get("export", {}) and row["export"].get("has_unique_id") for row in trip["rows"])
    print("  UniqueId on a new element: readable. In the export's item object: {}.".format(
        "present" if has_uid else "absent (as the plan assumes; build step 4 adds it)"))


def main(argv):
    here = os.path.dirname(os.path.abspath(__file__))
    path = argv[1] if len(argv) > 1 else os.path.join(here, "fixtures", "room-state-probe.json")
    report = load(path)
    print("document {}  revit {}  duHast {}".format(report.get("document"), report.get("revit_version"), report.get("duhast_path")))
    print()
    hosting(report)
    round_trip(report)


if __name__ == "__main__":
    main(sys.argv)
