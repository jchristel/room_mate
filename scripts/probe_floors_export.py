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
Capture the raw duHast floor and room exports, plus the Revit facts neither
export carries, so the floors entity's two unmeasured rules can be checked
against a real project rather than against a reading of duHast's source.

    pyRevit  ->  open the FEDERATED HOST and run this script; it walks the
                 host and every loaded link, narrowed by DOCUMENT_PREFIXES
    outputs  ->  <out>/floors-raw-<document>.json
                 <out>/floors-probe-<document>.json
                 <out>/rooms-raw-<document>.json
                 <out>/rooms-probe-<document>.json
                 <out>/floors-probe-index.json

Then, on any machine:

    python scripts/analyse_floors_probe.py --dir <out>

**This script decides nothing.** It collects, and it counts; every judgement
happens in the analyser, offline, where it can be re-run and argued with.

WHY FLOORS SHIPPED BEFORE THIS PROBE RAN, AND WHAT IT IS FOR
Ceilings were probed first and built second. Floors were built on the ceilings
stack -- duHast's `to_data_floor` is `to_data_ceiling` with the category and the
offset parameter swapped, so the export, the contract and the union rule were
already measured. Two things were NOT, and each is a place the ceilings
evidence does not transfer:

  F2  Are floors and rooms in the SAME document?  The ceilings probe found
      them co-located in every RHH document. A slab is exactly the element a
      base-build model holds while fit-out models hold the rooms, and the
      server's join is model-scoped -- so if this answers "no", every slab in
      such a project reports no room, correctly by the rule and uselessly.
  F6  Does the floor sliver rule hold?  The ceiling rule (a fraction of the
      element) cannot serve a storey-sized slab, so floors use the overlap's
      mean width instead. That was reasoned, not measured. The raw polygons
      captured here are what the analyser re-runs both rules over.

And three the ceilings probe never asked:

  F3  Is the exported footprint the floor Revit measures?  A floor carries
      `HOST_AREA_COMPUTED`, which a ceiling's probe never compared against. The
      union of the exported pieces should match it; a shortfall is a dropped
      face, an excess a duplicated one, and a ring that is INVALID before any
      repair is the suspected edge-orientation fault in
      `convert_edge_arrays_into_list_of_points` (it tessellates each edge in its
      own direction, not the loop's -- a reversed arc then zig-zags).
  F4  Where do floors sit relative to their level?  `height_offset_ft` is to
      the floor's TOP. A slab hosted on the level below at a full storey's
      offset would be joined to the wrong storey's rooms.
  F5  How many are structural?  `FLOOR_PARAM_IS_STRUCTURAL`, so the report can
      say whether a room lying on two floors is a slab plus a finish.

REVIT AND duHast IMPORTS ARE DEFERRED into the functions that need them, so this
file parses, lints and reads on a machine with no Revit.

Written to run on IronPython 2.7 and CPython 3 alike: `.format()` rather than
f-strings, no type hints, ASCII only, and every file written in binary so the
line endings are LF on Windows too.
"""

import io
import json
import os
import re
import sys


def default_out_dir():
    """`scripts/fixtures/` in this checkout, or None when `__file__` is unset."""
    here = globals().get("__file__")
    if not here:
        return None
    return os.path.join(os.path.dirname(os.path.abspath(here)), "fixtures")


def ensure_room_m_importable():
    """Put `extractor/pyRevit` on `sys.path` so `room_m.utils` imports, so ids
    here are spelled by the code that spells them on the wire."""
    here = globals().get("__file__")
    if not here:
        return
    root = os.path.dirname(os.path.dirname(os.path.abspath(here)))
    candidate = os.path.join(root, "extractor", "pyRevit")
    if os.path.isdir(candidate) and candidate not in sys.path:
        sys.path.insert(0, candidate)


# The two entities probed, stated as data.
ENTITY_SPECS = {
    "floors": {
        "category": "OST_Floors",
        "module": "duHast.Revit.Floors.Export.to_data_floor",
        "getter": "get_all_floor_data",
        "collector": (
            "duHast.Revit.Floors.floors",
            "get_all_floor_instances_in_model_by_category",
        ),
        "list_key": "floor",
        "kind": "floor",
    },
    "rooms": {
        "category": "OST_Rooms",
        "module": "duHast.Revit.Rooms.Export.to_data_room",
        "getter": "get_all_room_data",
        "collector": ("duHast.Revit.Rooms.rooms", "get_all_rooms"),
        "list_key": "room",
        "kind": "room",
    },
}

# Title prefixes narrowing a federated host's links to the models worth
# reading. Empty takes every loaded link. For F2 on RHH, include the base-build
# model beside the interiors ones -- that pairing is the question.
DOCUMENT_PREFIXES = []

# Bumped whenever the probe's OUTPUT or its conversion changes.
PROBE_VERSION = 2


def duhast_self_check():
    """Which duHast this run ACTUALLY executed, tested by behaviour rather than
    read off disk.

    **Why a behaviour test and not a file check.** On 2026-09-13 duHast's
    point-in-polygon fix (`geometry.adjust_delta`, the +2 quadrant case) was on
    disk in every copy pyRevit could load, and a floors probe run an hour later
    still exported holes as islands: the file was fixed, the MODULE was not. A
    script exec'd into a console keeps `duHast` in `sys.modules` for as long as
    the console lives, so nothing short of running the loaded code says which
    version ran. This asks it one question the old code answers wrongly -- is
    (2, 3) inside the clockwise triangle (0,10), (10,0), (0,0)? -- and records
    the module's file beside the answer.

    Recorded in the index and never raised: the run is still worth keeping, and
    the analyser is where a stale duHast becomes a warning.
    """
    from collections import namedtuple

    uv = namedtuple("UV", "U V")
    out = {"geometry_module": None, "point_in_polygon_fixed": None, "error": None}
    try:
        from duHast.Revit.Common.Geometry import geometry

        out["geometry_module"] = getattr(geometry, "__file__", None)
        triangle = [uv(0.0, 10.0), uv(10.0, 0.0), uv(0.0, 0.0)]
        out["point_in_polygon_fixed"] = bool(geometry.is_point_within_polygon(triangle, uv(2.0, 3.0)))
    except Exception as error:
        out["error"] = str(error)
    if out["point_in_polygon_fixed"] is False:
        print(
            "  WARNING: the duHast this run loaded is STALE -- point-in-polygon still has the +2 "
            "quadrant bug ({}). Restart Revit before trusting any polygon with holes.".format(
                out["geometry_module"]
            )
        )
    return out


# --------------------------------------------------------------------------
# Small Revit accessors. Each answers None rather than raising: one unreadable
# element costs its own field, not the run.
# --------------------------------------------------------------------------


def _element_id_str(element_id):
    from room_m.utils.generic import element_id_str

    return element_id_str(element_id)


def _phase_facts(doc, element):
    """The phase a floor was BUILT in and the one it was demolished in -- the
    range test's two inputs, recorded so the analyser can say whether the
    range test was required or merely harmless on this model."""
    from Autodesk.Revit.DB import BuiltInParameter

    out = {"phase_created": None, "phase_demolished": None}
    for field, builtin in (
        ("phase_created", "PHASE_CREATED"),
        ("phase_demolished", "PHASE_DEMOLISHED"),
    ):
        try:
            parameter = element.get_Parameter(getattr(BuiltInParameter, builtin))
            if parameter is None:
                continue
            phase = doc.GetElement(parameter.AsElementId())
            out[field] = phase.Name if phase is not None else None
        except Exception:
            continue
    return out


def _room_phase(doc, element):
    """`ROOM_PHASE`, for a room -- the equality test's input."""
    from Autodesk.Revit.DB import BuiltInParameter

    try:
        parameter = element.get_Parameter(BuiltInParameter.ROOM_PHASE)
        if parameter is None:
            return None
        phase = doc.GetElement(parameter.AsElementId())
        return phase.Name if phase is not None else None
    except Exception:
        return None


def _level_facts(doc, element):
    """The level an element states, by name, id and elevation. A room answers
    `.Level`, a floor `LevelId`; both are tried."""
    level = None
    try:
        level = element.Level
    except Exception:
        level = None
    if level is None:
        try:
            level = doc.GetElement(element.LevelId)
        except Exception:
            level = None
    if level is None:
        return {"level_name": None, "level_id": None, "level_elevation_ft": None}
    try:
        return {
            "level_name": level.Name,
            "level_id": _element_id_str(level.Id),
            "level_elevation_ft": float(level.Elevation),
        }
    except Exception:
        return {"level_name": None, "level_id": None, "level_elevation_ft": None}


def _double_parameter(element, builtin_name):
    """One BuiltInParameter as a raw double in Revit's internal units, or None."""
    from Autodesk.Revit.DB import BuiltInParameter

    try:
        parameter = element.get_Parameter(getattr(BuiltInParameter, builtin_name))
        if parameter is None or not parameter.HasValue:
            return None
        return float(parameter.AsDouble())
    except Exception:
        return None


def _structural(element):
    """`FLOOR_PARAM_IS_STRUCTURAL` as a bool, or None when the floor has no such
    parameter -- an in-place floor, for one."""
    from Autodesk.Revit.DB import BuiltInParameter

    try:
        parameter = element.get_Parameter(BuiltInParameter.FLOOR_PARAM_IS_STRUCTURAL)
        if parameter is None or not parameter.HasValue:
            return None
        return parameter.AsInteger() == 1
    except Exception:
        return None


def _geometry_facts(element):
    """How many solids the element's top-level geometry holds, whether it is an
    in-place family, and whether any of its horizontal faces has a CURVED edge.

    The curved-edge count is F3's discriminator. The suspected fault in
    `convert_edge_arrays_into_list_of_points` only bites where an edge runs
    against its loop, and on a straight edge that merely shifts the ring by one
    vertex -- harmless. On an ARC it reverses the tessellated points inside a
    forward-walking loop and the ring crosses itself. So an invalid ring on a
    floor with arcs points at that function; an invalid ring on a floor made of
    straight edges points somewhere else.
    """
    from Autodesk.Revit.DB import Line, Options, PlanarFace, Solid

    facts = {
        "solid_count": None,
        "is_in_place": None,
        "class_name": None,
        "horizontal_face_count": None,
        "curved_edge_count": None,
    }
    try:
        facts["class_name"] = type(element).__name__
        facts["is_in_place"] = facts["class_name"] == "FamilyInstance"
    except Exception:
        pass
    try:
        geometry = element.get_Geometry(Options())
        solids = [item for item in geometry if type(item) is Solid]
        facts["solid_count"] = len(solids)
        faces = 0
        curved = 0
        for solid in solids:
            for face in solid.Faces:
                if type(face) is not PlanarFace or face.FaceNormal.Z == 0.0:
                    continue
                faces += 1
                for loop in face.EdgeLoops:
                    for edge in loop:
                        curve = edge.AsCurve()
                        if not isinstance(curve, Line):
                            curved += 1
        facts["horizontal_face_count"] = faces
        facts["curved_edge_count"] = curved
    except Exception:
        pass
    return facts


def _room_measurements(element):
    """`Area` and whether the room is placed, in Revit's own units."""
    out = {"area_internal_sqft": None, "location_is_none": None}
    try:
        out["area_internal_sqft"] = float(element.Area)
    except Exception:
        pass
    try:
        out["location_is_none"] = element.Location is None
    except Exception:
        pass
    return out


# --------------------------------------------------------------------------
# Collection
# --------------------------------------------------------------------------


def collect_elements(doc, category_name):
    """Every element of one category, collected directly rather than through
    duHast, so a collector failure cannot pass for an empty model."""
    from Autodesk.Revit.DB import BuiltInCategory, FilteredElementCollector

    category = getattr(BuiltInCategory, category_name, None)
    if category is None:
        return None
    return list(
        FilteredElementCollector(doc)
        .OfCategory(category)
        .WhereElementIsNotElementType()
        .ToElements()
    )


def duhast_collected_count(doc, spec):
    """How many elements duHast's own collector reports, and whether it raised."""
    module_name, getter_name = spec["collector"]
    try:
        module = __import__(module_name, globals(), locals(), [getter_name])
        collected = getattr(module, getter_name)(doc)
        return (len(list(collected)), None)
    except Exception as error:
        return (None, str(error))


def probe_element(doc, element, kind):
    """Every Revit fact about one element that its export does not carry
    usably. Property VALUES are read from the raw export by the analyser."""
    facts = {"id": _element_id_str(element.Id)}
    facts.update(_level_facts(doc, element))
    if kind == "floor":
        facts.update(_geometry_facts(element))
        facts.update(_phase_facts(doc, element))
        facts["height_offset_ft"] = _double_parameter(element, "FLOOR_HEIGHTABOVELEVEL_PARAM")
        facts["host_area_sqft"] = _double_parameter(element, "HOST_AREA_COMPUTED")
        facts["is_structural"] = _structural(element)
    else:
        facts.update(_room_measurements(element))
        facts["room_phase"] = _room_phase(doc, element)
    return facts


def run_export(doc, spec):
    """duHast's own export for this entity, verbatim, wrapped exactly as the
    extractor wraps it."""
    from duHast.Data.Utils.data_to_file import build_json_for_file

    module = __import__(spec["module"], globals(), locals(), [spec["getter"]])
    data = getattr(module, spec["getter"])(doc)
    return build_json_for_file({spec["list_key"]: data}, "{}".format(doc.Title))


# --------------------------------------------------------------------------
# Serialisation -- duplicated from the ceilings probe rather than imported:
# these scripts are exec'd into pyRevit, where an import between them resolves
# to whatever happens to be on the path.
# --------------------------------------------------------------------------


def plainify(value, path="", problems=None):
    """duHast objects to JSON-safe values, recording what would not convert."""
    if problems is None:
        problems = []
    if value is None or isinstance(value, (bool, int, float)):
        return value, problems
    try:
        text_types = (str, unicode)  # noqa: F821 - IronPython 2.7
    except NameError:
        text_types = (str,)
    if isinstance(value, text_types):
        return value, problems

    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            out[str(key)], problems = plainify(item, "{}.{}".format(path, key), problems)
        return out, problems

    if isinstance(value, (list, tuple)):
        out = []
        for index, item in enumerate(value):
            converted, problems = plainify(item, "{}[{}]".format(path, index), problems)
            out.append(converted)
        return out, problems

    to_dict = getattr(value, "class_to_dict", None)
    if callable(to_dict):
        try:
            return plainify(to_dict(), path, problems)
        except Exception as error:
            problems.append({"path": path, "type": type(value).__name__, "error": str(error)})
            return "<class_to_dict failed: {}>".format(type(value).__name__), problems

    attributes = getattr(value, "__dict__", None)
    if isinstance(attributes, dict) and attributes:
        return plainify(attributes, path, problems)

    problems.append({"path": path, "type": type(value).__name__, "repr": repr(value)})
    return "<unconvertible: {}>".format(type(value).__name__), problems


def write_json(path, payload):
    """UTF-8 JSON with LF endings, written in binary."""
    text = json.dumps(payload, indent=2, ensure_ascii=False, sort_keys=True)

    directory = os.path.dirname(os.path.abspath(path))
    if directory and not os.path.isdir(directory):
        os.makedirs(directory)
    with io.open(path, "wb") as handle:
        handle.write(text.encode("utf-8"))
    return path


def slugify(title):
    """A filename-safe stem for a document title, so documents never overwrite
    each other's outputs."""
    slug = re.sub(r"[^A-Za-z0-9._-]+", "-", title or "document").strip("-")
    return (slug or "document")[:80]


# --------------------------------------------------------------------------
# Per document
# --------------------------------------------------------------------------


def probe_document(doc, out_dir):
    """Export and probe one document for BOTH entities regardless of what it is
    supposed to be -- F2 is exactly the question of which documents hold
    which."""
    ensure_room_m_importable()
    slug = slugify(doc.Title)
    written = []
    summary = {
        "document": doc.Title,
        "path": getattr(doc, "PathName", None),
        "slug": slug,
        "entities": {},
    }

    for entity, spec in sorted(ENTITY_SPECS.items()):
        elements = collect_elements(doc, spec["category"])
        if elements is None:
            print(
                "  WARNING: this Revit version has no BuiltInCategory {}".format(
                    spec["category"]
                )
            )
            continue

        duhast_count, collector_error = duhast_collected_count(doc, spec)

        # An empty population is still written out: "this document holds no
        # floors" is F2's finding.
        raw = run_export(doc, spec) if elements else {spec["list_key"]: []}
        raw_plain, problems = plainify(raw, entity)
        if problems:
            print(
                "  WARNING: {} field(s) would not convert; see "
                "serialisation_problems in the probe file".format(len(problems))
            )

        records = [probe_element(doc, element, spec["kind"]) for element in elements]

        probe = {
            "probe_version": PROBE_VERSION,
            "entity": entity,
            "list_key": spec["list_key"],
            "document": doc.Title,
            "path": getattr(doc, "PathName", None),
            "serialisation_problems": problems,
            "collected_count": len(elements),
            "duhast_collected_count": duhast_count,
            "duhast_collector_error": collector_error,
            "exported_count": len(raw.get(spec["list_key"], []) or []),
            "elements": records,
        }

        written.append(
            write_json(os.path.join(out_dir, "{}-raw-{}.json".format(entity, slug)), raw_plain)
        )
        written.append(
            write_json(os.path.join(out_dir, "{}-probe-{}.json".format(entity, slug)), probe)
        )
        summary["entities"][entity] = {
            "collected_count": len(elements),
            "duhast_collected_count": duhast_count,
            "duhast_collector_error": collector_error,
            "exported_count": probe["exported_count"],
        }

    return written, summary


# --------------------------------------------------------------------------
# Entry point
# --------------------------------------------------------------------------


def linked_documents(host):
    """The host document plus every loaded link."""
    from duHast.Revit.Links.links import get_link_docs

    docs = [host]
    for _name, link_doc in get_link_docs(
        host, link_names_filter=[], inverse_filter=True
    ).items():
        docs.append(link_doc)
    return docs


def resolve_documents(prefixes=None):
    """The host and its links, narrowed by title prefix."""
    try:
        host = __revit__.ActiveUIDocument.Document  # noqa: F821 - pyRevit global
    except Exception:
        raise RuntimeError(
            "no Revit document: run this from pyRevit with the federated host "
            "open, or call probe_document(doc, out_dir) yourself"
        )

    try:
        docs = linked_documents(host)
    except Exception as error:
        print("  WARNING: could not read links ({}); probing the host only".format(error))
        docs = [host]

    if not prefixes:
        return docs
    return [d for d in docs if any(prefix in d.Title for prefix in prefixes)]


def main(out_dir=None):
    out_dir = out_dir or default_out_dir()
    if not out_dir:
        raise RuntimeError(
            "cannot resolve an output directory (no __file__): pass one, e.g. "
            "main(out_dir=r'C:\\\\temp\\\\probe')"
        )

    print("probe_floors_export v{}".format(PROBE_VERSION))
    index = {"probe_version": PROBE_VERSION, "documents": []}
    index["duhast_self_check"] = duhast_self_check()
    for doc in resolve_documents(DOCUMENT_PREFIXES):
        print("probing {} -> {}".format(doc.Title, out_dir))
        written, summary = probe_document(doc, out_dir)
        for path in written:
            print("  wrote {}".format(path))
        index["documents"].append(summary)

    print("  wrote {}".format(
        write_json(os.path.join(out_dir, "floors-probe-index.json"), index)
    ))
    print("done. Now run: python scripts/analyse_floors_probe.py --dir <that directory>")


sys.path += [r"C:\Users\janchristel\Documents\GitHub\SampleCodeRevitBatchProcessor-NET8\src", r"C:\Users\janchristel\Documents\GitHub\SampleCodeRevitBatchProcessor-NET8\Samples\pyRevit\Extensions\duHast-2025.extension\duHast.tab\lib"]

if __name__ == "__main__":
    main(r"C:\Users\janchristel\Documents\GitHub\room_mate\temp")
