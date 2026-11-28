#!/usr/bin/env python
# License:
#
#
# Revit Batch Processor Sample Code
#
# BSD License
# Copyright 2026, Jan Christel
# All rights reserved.

# Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

# - Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.
# - Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.
# - Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.
#
# This software is provided by the copyright holder "as is" and any express or implied warranties, including, but not limited to, the implied warranties of merchantability and fitness for a particular purpose are disclaimed.
# In no event shall the copyright holder be liable for any direct, indirect, incidental, special, exemplary, or consequential damages (including, but not limited to, procurement of substitute goods or services; loss of use, data, or profits;
# or business interruption) however caused and on any theory of liability, whether in contract, strict liability, or tort (including negligence or otherwise) arising in any way out of the use of this software, even if advised of the possibility of such damage.
#
#
#
"""
The two measurements PLAN-blueprint-room-state.md says only a Revit run can settle.

    pyRevit  ->  run this script against the model
    outputs  ->  scripts/fixtures/room-state-probe.json

Then, on any machine:

    python scripts/analyse_room_state_probe.py

**This script decides nothing.** It collects and counts; every judgement is the analyser's,
offline, where it can be re-run and argued with.

PART A, THE HOSTING CENSUS (reads only, never writes)
    Question: which instances are "level-based or work-plane (level)", and which are hosted on
    a wall or a face? The first iteration of the room-state plan acts only on the first kind
    and flags the second (`unsupported_host`), so the rule that separates them has to come
    from what Revit says, not from a reading of the source.

    For every FF&E instance it records, side by side and never reconciled here:
      - `placement_type`   `Family.FamilyPlacementType` (OneLevelBased, OneLevelBasedHosted,
                           WorkPlaneBased, TwoLevelsBased, CurveBased, ...): a property of the
                           FAMILY, which is the better classifier if it separates the cases;
      - `host`             what the extractor ships today (`get_host_info`: category, id,
                           `face_hosted`), which is None when `FamilyInstance.Host` is None;
      - `host_class`       the .NET class of `FamilyInstance.Host` (a Level, a Wall, a
                           ReferencePlane, ...), because a work-plane family hosted on a level
                           may answer with the level rather than None, and the shipped reader
                           would then call it hosted;
      - `host_param`       the instance's `Host` parameter, as the display string the export
                           carries (`Level : LEVEL 1`);
      - `has_host_face`    `FamilyInstance.HostFace` is not None;
      - `is_component`     it is nested in another instance (those are never hosts).
    The summary groups on (placement_type, host_class, has_host_face) and gives a few ids of
    each group, so the analyser can say which combinations a rule would put on which side.

PART B, THE PLACEMENT ROUND TRIP (writes inside a transaction group that is ROLLED BACK)
    Question: does an element we place read back where we put it, to within 5 mm, in the
    numbers the server will compare? The audit judges `in_place` by comparing the recipe's
    place with the exported place, so any offset between the two is a drift the server would
    report on an element nobody touched.

    It picks a level-based symbol that already has an instance in the model (so a family the
    model carries, never one it must load), places `N_POINTS` instances at awkward
    millimetre values and turns, with the SAME helper the Tasks button uses
    (`place_instance_on_level_without_transaction_wrapper`), regenerates, and reads each back
    three ways:
      - the API (`Location.Point`, the instance transform);
      - the export's own converter (`populate_data_item_object`), which is the number the
        server stores, so this is the one that decides;
      - `UniqueId`, to show it is readable on a new element and that the export does not
        carry it yet (it is added to the extractor in build step 4 of the plan).
    Then the group is rolled back, so the model is left as it was. The script says so in the
    record and checks `doc.IsModified`-independent evidence: the placed ids no longer resolve.

    Points are near an existing instance of the same family (so far-from-origin float is
    judged where the work really is) plus one far point, 1 km out, to see whether distance
    from the origin costs anything.

REVIT AND duHast IMPORTS ARE DEFERRED into the functions that need them, so this file parses,
lints and reads on a machine with no Revit. Only the standard library is imported at module
scope.

Written to run on IronPython 2.7 and CPython 3 alike: `.format()` rather than f-strings, no
type hints, ASCII only (IronPython will not parse a file with an em-dash in it, not even in a
docstring), and every file written in binary so the line endings are LF on Windows too.
"""

import io
import json
import math
import os
import sys

FEET_TO_MM = 304.8
N_POINTS = 8
FAR_POINT_MM = 1000000.0

# duHast's DEFAULT_ITEM_CATEGORIES, as the FF&E probe transcribes them. Transcribed so this
# file parses with no Revit; the export it measures (`get_all_item_data`) takes the same list.
FFE_CATEGORY_NAMES = [
    "OST_Furniture",
    "OST_FurnitureSystems",
    "OST_Casework",
    "OST_MechanicalEquipment",
    "OST_ElectricalEquipment",
    "OST_ElectricalFixtures",
    "OST_PlumbingFixtures",
    "OST_SpecialityEquipment",
    "OST_GenericModel",
]

# Offsets from the reference instance (mm) and turns (degrees). Fractions and odd angles on
# purpose: a round number hides a unit or rounding fault, and 37.5 / 179.9 find a turn that is
# snapped or truncated.
OFFSETS_MM = [
    (0.0, 0.0),
    (1234.567, 0.0),
    (0.0, 2345.678),
    (-3456.789, 1111.111),
    (4567.891, -2222.222),
    (123.456, 654.321),
    (-5000.0, -5000.0),
    (7000.123, 3000.456),
]
TURNS_DEG = [0.0, 37.5, 90.0, 179.9, -90.0, 270.0, 12.34, 45.0]


def default_out_dir():
    """Where the result goes: the folder named by the `ROOMMATE_PROBE_DIR` environment variable,
    else `RoomMate-probe` in the user's Documents. Never derived from the script's own location:
    a script run from the pyRevit console has no reliable one. `run` prints the path it wrote."""
    named = os.environ.get("ROOMMATE_PROBE_DIR")
    if named:
        return named
    return os.path.join(os.path.expanduser("~"), "Documents", "RoomMate-probe")


def write_json(path, payload):
    """UTF-8, LF, binary, so Windows never writes CRLF."""
    text = json.dumps(payload, indent=2, sort_keys=True)
    if not isinstance(text, bytes):
        text = text.encode("utf-8")
    with io.open(path, "wb") as handle:
        handle.write(text + b"\n")


def safe(read, default=None):
    """Run one read; a Revit call that raises costs that fact and nothing else."""
    try:
        return read()
    except Exception:
        return default


def element_int(element_id):
    """An ElementId as an int on every Revit version (`Value` became `IntegerValue`, then `Value` again)."""
    for name in ("Value", "IntegerValue"):
        got = safe(lambda: getattr(element_id, name))
        if got is not None:
            return int(got)
    return None


def class_name(obj):
    return None if obj is None else safe(lambda: obj.GetType().Name)


def host_parameter(instance):
    """The instance's `Host` parameter as the export carries it (a display string)."""

    def read():
        param = instance.LookupParameter("Host")
        if param is None:
            return None
        return param.AsString() or param.AsValueString()

    return safe(read)


def categories(doc):
    from Autodesk.Revit.DB import BuiltInCategory

    found = []
    for name in FFE_CATEGORY_NAMES:
        member = safe(lambda: getattr(BuiltInCategory, name))
        if member is not None:
            found.append(member)
    return found


def hosting_census(doc):
    """Part A: one record per FF&E family instance, and the grouped summary."""
    from Autodesk.Revit.DB import ElementCategoryFilter, FamilyInstance, FilteredElementCollector
    from duHast.Revit.Family.family_instance_facts import get_host_info

    records = []
    for category in categories(doc):
        collector = FilteredElementCollector(doc).OfClass(FamilyInstance).WherePasses(ElementCategoryFilter(category))
        for instance in collector:
            symbol = safe(lambda: instance.Symbol)
            family = safe(lambda: symbol.Family)
            host = safe(lambda: instance.Host)
            host_face = safe(lambda: instance.HostFace)
            host_info = get_host_info(instance)
            level_id = safe(lambda: instance.LevelId)
            record = {
                "id": element_int(instance.Id),
                "unique_id": safe(lambda: str(instance.UniqueId)),
                "category": safe(lambda: instance.Category.Name),
                "family": safe(lambda: str(family.Name)),
                "type": safe(lambda: str(symbol.Name)),
                "placement_type": safe(lambda: str(family.FamilyPlacementType)),
                "host": host_info,
                "host_class": class_name(host),
                "host_category": safe(lambda: host.Category.Name) if host is not None else None,
                "has_host_face": host_face is not None,
                "host_param": host_parameter(instance),
                "level_id": element_int(level_id) if level_id is not None else None,
                "is_component": safe(lambda: instance.SuperComponent) is not None,
                "location_class": class_name(safe(lambda: instance.Location)),
            }
            records.append(record)

    groups = {}
    for record in records:
        key = "{} | host={} | face={} | host_param={}".format(
            record["placement_type"],
            record["host_class"],
            record["has_host_face"],
            (record["host_param"] or "").split(":")[0].strip(),
        )
        group = groups.setdefault(key, {"count": 0, "components": 0, "examples": []})
        group["count"] += 1
        if record["is_component"]:
            group["components"] += 1
        if len(group["examples"]) < 5:
            group["examples"].append(record["id"])
    return {"instances": records, "groups": groups}


def pick_reference(doc):
    """A level-based (OneLevelBased) FF&E instance to place copies of, or None. Prefers one that
    is not a component, because a component cannot be placed on its own."""
    from Autodesk.Revit.DB import ElementCategoryFilter, FamilyInstance, FamilyPlacementType, FilteredElementCollector

    for category in categories(doc):
        collector = FilteredElementCollector(doc).OfClass(FamilyInstance).WherePasses(ElementCategoryFilter(category))
        for instance in collector:
            symbol = safe(lambda: instance.Symbol)
            family = safe(lambda: symbol.Family)
            if family is None or family.FamilyPlacementType != FamilyPlacementType.OneLevelBased:
                continue
            if safe(lambda: instance.SuperComponent) is not None:
                continue
            if safe(lambda: instance.Host) is not None:
                continue
            if not hasattr(instance.Location, "Point"):
                continue
            if safe(lambda: instance.LevelId) is None:
                continue
            return instance
    return None


def facing_degrees(transform):
    """Counter-clockwise degrees of the instance's local x axis in plan, as the export's basis rows give it."""
    return math.degrees(math.atan2(transform.BasisX.Y, transform.BasisX.X))


def angle_difference(a, b):
    """Smallest signed difference between two angles in degrees."""
    d = (a - b + 180.0) % 360.0 - 180.0
    return d


def export_view(doc, instance):
    """What the export's own converter says about one instance: position in mm and the facing from
    its rotation rows. None when the converter returns nothing (not a point location)."""
    from duHast.Revit.Family.Export.to_data_item import populate_data_item_object

    item = populate_data_item_object(doc, instance)
    if item is None:
        return None
    point = item.location_point.translation_coord
    rotation = item.location_point.rotation_coord
    # duHast's Matrix keeps its rows in `data`; `rows` is the row COUNT (an int)
    rows = safe(lambda: rotation.data)
    facing = None
    if rows and not isinstance(rows, int):
        facing = math.degrees(math.atan2(rows[0][1], rows[0][0]))
    level = item.level
    return {
        "x_mm": safe(lambda: float(point.x)),
        "y_mm": safe(lambda: float(point.y)),
        "z_mm": safe(lambda: float(point.z)),
        "facing_deg": facing,
        "level_id": safe(lambda: level.id),
        "offset_from_level": safe(lambda: level.offset_from_level),
        "has_unique_id": hasattr(item, "unique_id"),
    }


def round_trip(doc):
    """Part B: place, read back three ways, roll back."""
    from Autodesk.Revit.DB import Transaction, TransactionGroup, XYZ
    from duHast.Revit.Family.family_instance_place import place_instance_on_level_without_transaction_wrapper

    reference = pick_reference(doc)
    if reference is None:
        return {"skipped": "no level-based FF&E instance with a point location found to copy"}

    symbol = reference.Symbol
    level = doc.GetElement(reference.LevelId)
    base = reference.Location.Point
    base_z = base.Z

    group = TransactionGroup(doc, "RoomMate probe: round trip")
    group.Start()
    placed = []
    try:
        trans = Transaction(doc, "RoomMate probe: place")
        trans.Start()
        try:
            for index in range(N_POINTS):
                dx, dy = OFFSETS_MM[index % len(OFFSETS_MM)]
                turn = TURNS_DEG[index % len(TURNS_DEG)]
                x = base.X + dx / FEET_TO_MM
                y = base.Y + dy / FEET_TO_MM
                point = XYZ(x, y, base_z)
                instance = place_instance_on_level_without_transaction_wrapper(doc, symbol, point, level, math.radians(turn))
                placed.append({"instance": instance, "asked": (x * FEET_TO_MM, y * FEET_TO_MM), "turn": turn, "tag": "near"})
            far = XYZ(base.X + FAR_POINT_MM / FEET_TO_MM, base.Y, base_z)
            instance = place_instance_on_level_without_transaction_wrapper(doc, symbol, far, level, math.radians(37.5))
            placed.append({"instance": instance, "asked": (far.X * FEET_TO_MM, far.Y * FEET_TO_MM), "turn": 37.5, "tag": "far"})
            doc.Regenerate()
            trans.Commit()
        except Exception:
            trans.RollBack()
            raise

        rows = []
        for entry in placed:
            instance = entry["instance"]
            api_point = safe(lambda: instance.Location.Point)
            api_x = None if api_point is None else api_point.X * FEET_TO_MM
            api_y = None if api_point is None else api_point.Y * FEET_TO_MM
            transform = safe(lambda: instance.GetTransform())
            api_facing = None if transform is None else facing_degrees(transform)
            exported = export_view(doc, instance)
            asked_x, asked_y = entry["asked"]
            row = {
                "tag": entry["tag"],
                "id": element_int(instance.Id),
                "unique_id": safe(lambda: str(instance.UniqueId)),
                "asked_x_mm": asked_x,
                "asked_y_mm": asked_y,
                "asked_turn_deg": entry["turn"],
                "api_x_mm": api_x,
                "api_y_mm": api_y,
                "api_facing_deg": api_facing,
                "export": exported,
            }
            if exported and exported.get("x_mm") is not None:
                row["export_dx_mm"] = exported["x_mm"] - asked_x
                row["export_dy_mm"] = exported["y_mm"] - asked_y
                row["export_err_mm"] = math.hypot(row["export_dx_mm"], row["export_dy_mm"])
            if exported and exported.get("facing_deg") is not None:
                row["export_turn_err_deg"] = angle_difference(exported["facing_deg"], entry["turn"])
            if api_x is not None:
                row["api_err_mm"] = math.hypot(api_x - asked_x, api_y - asked_y)
            rows.append(row)
        placed_ids = [row["id"] for row in rows]
    finally:
        group.RollBack()

    from Autodesk.Revit.DB import ElementId

    gone = 0
    for element_id in placed_ids:
        if element_id is None:
            continue
        try:
            still = doc.GetElement(ElementId(element_id))
        except Exception:
            still = None
        if still is None:
            gone += 1
    return {
        "reference_id": element_int(reference.Id),
        "reference_family": safe(lambda: str(reference.Symbol.Family.Name)),
        "reference_type": safe(lambda: str(reference.Symbol.Name)),
        "reference_unique_id": safe(lambda: str(reference.UniqueId)),
        "level_id": element_int(level.Id),
        "rows": rows,
        "placed": len(placed_ids),
        "rolled_back_ids_gone": gone,
    }


def use_duhast_at(path):
    """Put a duHast checkout's `src` folder first on `sys.path`, so the probe measures THAT duHast.

    Set `DUHAST_PATH` below, or the `ROOMMATE_DUHAST_PATH` environment variable, to the folder that
    CONTAINS the `duHast` package (`...\\src`, not `...\\src\\duHast`). Empty uses whatever the pyRevit
    session already imported. **A session that has already imported duHast keeps it** (restart Revit
    to change it), which is why `run` records where the loaded one actually came from."""
    if path and path not in sys.path:
        sys.path.insert(0, path)


# Edit this line on your machine, or set ROOMMATE_DUHAST_PATH. Leave empty to use the loaded duHast.
DUHAST_PATH = r"C:\Users\janchristel\Documents\GitHub\SampleCodeRevitBatchProcessor-NET8\src"


def run(doc, out_dir=None):
    """Both parts, written to `room-state-probe.json`. Returns the path."""
    use_duhast_at(os.environ.get("ROOMMATE_DUHAST_PATH") or DUHAST_PATH)
    out_dir = out_dir or default_out_dir()
    if not os.path.isdir(out_dir):
        os.makedirs(out_dir)

    from duHast.Revit.Common.element_id import get_el_id_int  # noqa: F401 (proves duHast is the loaded one)
    import duHast

    payload = {
        "probe": "room-state",
        "version": 1,
        "document": safe(lambda: doc.Title),
        "duhast_path": safe(lambda: list(duHast.__path__)[0]),
        "revit_version": safe(lambda: doc.Application.VersionNumber),
        "hosting": hosting_census(doc),
        "round_trip": round_trip(doc),
    }
    path = os.path.join(out_dir, "room-state-probe.json")
    write_json(path, payload)
    return path



print("wrote {}".format(run(doc)))