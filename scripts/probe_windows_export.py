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
Capture the raw duHast WINDOW export, plus the Revit facts the export cannot
carry, so the three questions blocking the windows plan can be answered off a
measurement instead of off a reading of duHast's source.

    pyRevit  ->  run this script against the model
    outputs  ->  scripts/fixtures/windows-raw.json
                 scripts/fixtures/windows-probe.json
                 scripts/fixtures/doors-raw-control.json
                 scripts/fixtures/doors-probe-control.json

Then, on any machine:

    python scripts/analyse_windows_probe.py

**This script decides nothing.** It collects, and it counts; every judgement --
is a curtain-wall panel a window, is a level wrong, is the record identical to a
door's -- happens in the analyser, offline, where it can be re-run and argued
with. That split is the whole point: the expensive half needs Revit and must be
right first time, the cheap half is where the thinking goes.

WHY DOORS ARE EXPORTED TOO
The plan rests on "a window is a door with a different category". The committed
`scripts/fixtures/doors-raw.json` cannot test that: it was captured months ago
from an older duHast, so a field missing from it proves nothing about today. A
door control taken from THIS document with THIS duHast in the same pass makes
the comparison direct. It is written to `doors-*-control.json` and never over
the committed fixture, which `gen_doors_sample.py` still depends on.

WHAT THE PROBE ADDS THAT THE EXPORT DOES NOT HAVE
- room references per phase, read through the same `room_reference` accessor the
  real extractor uses, so the answer here is the answer production would get;
- whether the instance is a curtain-wall panel -- duHast has this test for doors
  (`Revit/Doors/doors.py`) and does NOT have it for windows, which is the
  unquantified risk the plan calls C3;
- the world Z extents and sill height against the level elevations, which is
  what "the reported level is not the storey it serves" (C4) is measured from;
- the super-component's category, which is the nested-leaf test that found 2236
  false doors on the last job.

REVIT AND duHast IMPORTS ARE DEFERRED into the functions that need them, so this
file parses, lints and reads on a machine with no Revit -- which is where it
will be edited. Only the standard library is imported at module scope.

Written to run on IronPython 2.7 and CPython 3 alike: `.format()` rather than
f-strings, no type hints, and every file written in binary so the line endings
are LF on Windows too (`.gitattributes` enforces LF, and a text-mode write here
would silently produce CRLF).
"""

import io
import json
import os
import sys


# Where the outputs go by default: beside the door fixture they will be compared
# against. Resolved from this file's own location rather than the working
# directory, which pyRevit sets to somewhere unrelated.
def default_out_dir():
    """`scripts/fixtures/` in this checkout, or None when the path cannot be
    resolved -- pyRevit normally sets `__file__`, but a script pasted into its
    console has none, and guessing a directory to write megabytes into is worse
    than saying so."""
    here = globals().get("__file__")
    if not here:
        return None
    return os.path.join(os.path.dirname(os.path.abspath(here)), "fixtures")


def ensure_room_m_importable():
    """Put `extractor/pyRevit` on `sys.path` so `room_m.utils` imports.

    The probe reuses the extractor's phase and room-reference helpers rather
    than reimplementing them. That is not tidiness: a second implementation of
    "which room is this element's ToRoom in this phase" could disagree with the
    one that ships, and then the measurement would be of the probe rather than
    of the model. If the package cannot be found the script stops -- a probe
    that silently answered with its own logic is the failure being avoided.
    """
    here = globals().get("__file__")
    if not here:
        return
    root = os.path.dirname(os.path.dirname(os.path.abspath(here)))
    candidate = os.path.join(root, "extractor", "pyRevit")
    if os.path.isdir(candidate) and candidate not in sys.path:
        sys.path.insert(0, candidate)


# The two entities probed, and the ONLY place either is named. Each row is
# (category name on BuiltInCategory, duHast getter module/function, the export's
# list key). Adding FF&E later is a row, not a branch.
ENTITY_SPECS = {
    "windows": {
        "category": "OST_Windows",
        "module": "duHast.Revit.Windows.Export.to_data_window",
        "getter": "get_all_window_data",
        "list_key": "window",
        "sides": ("FromRoom", "ToRoom"),
    },
    "doors": {
        "category": "OST_Doors",
        "module": "duHast.Revit.Doors.Export.to_data_door",
        "getter": "get_all_door_data",
        "list_key": "door",
        "sides": ("FromRoom", "ToRoom"),
    },
}

# Doors are the control, so their outputs must not land on the committed
# fixture `gen_doors_sample.py` reads.
OUT_SUFFIX = {"windows": "", "doors": "-control"}

# Bumped whenever the probe's OUTPUT or its conversion changes. Printed at
# startup because this script is run by exec'ing its text, so nothing else
# tells a reader which copy actually ran -- and a stale copy has already
# cost two runs, both diagnosed from a traceback that named the old line
# numbers. v2: plainify replaced duHast's all-or-nothing serializer.
PROBE_VERSION = 2


# --------------------------------------------------------------------------
# Small Revit accessors. Each one answers None rather than raising, because a
# single unreadable window must cost its own field and not the run: the probe
# exists to describe a model nobody has looked at yet, so "this family does not
# have that parameter" is an expected outcome, not an error.
# --------------------------------------------------------------------------


def _element_id_str(element_id):
    """Deferred re-export of the extractor's accessor, so ids here and ids on
    the wire are spelled the same way."""
    from room_m.utils.generic import element_id_str

    return element_id_str(element_id)


def _param_as_string(element, builtin_name):
    """One built-in parameter as a display string, or None."""
    from Autodesk.Revit.DB import BuiltInParameter

    try:
        definition = getattr(BuiltInParameter, builtin_name, None)
        if definition is None:
            return None
        param = element.get_Parameter(definition)
        if param is None:
            return None
        value = param.AsString()
        if value is None:
            value = param.AsValueString()
        return value
    except Exception:
        return None


def _param_as_double(element, builtin_name):
    """One built-in parameter as a float in Revit's internal units (decimal
    feet for a length), or None."""
    from Autodesk.Revit.DB import BuiltInParameter

    try:
        definition = getattr(BuiltInParameter, builtin_name, None)
        if definition is None:
            return None
        param = element.get_Parameter(definition)
        if param is None or not param.HasValue:
            return None
        return float(param.AsDouble())
    except Exception:
        return None


def _bbox_z_extents(element):
    """The instance's world Z range as `(min, max)` in decimal feet.

    The WORLD axis-aligned box on purpose, unlike the oriented one duHast
    measures for the footprint: the question here is "how high off the level
    does this sit", and height is the one dimension an axis-aligned box gets
    right regardless of the wall's angle."""
    try:
        box = element.get_BoundingBox(None)
        if box is None:
            return None, None
        return float(box.Min.Z), float(box.Max.Z)
    except Exception:
        return None, None


def _location_point(element):
    """`{x, y, z}` in decimal feet, or None for an element Revit gave no
    `LocationPoint`. Z is kept here although the contract drops it -- the
    storey question needs an elevation and this is the cheapest one."""
    try:
        location = getattr(element, "Location", None)
        point = getattr(location, "Point", None) if location is not None else None
        if point is None:
            return None
        return {"x": float(point.X), "y": float(point.Y), "z": float(point.Z)}
    except Exception:
        return None


def _facing_plan(element):
    """`FacingOrientation` projected to plan and normalised, or None when it has
    no plan component. The same rule and the same reason as
    `room_m.utils.doors.door_through_wall_normal` -- restated rather than
    imported because that helper is bound to the door module today, and the
    probe must not be the thing that refactors it."""
    import math

    try:
        facing = getattr(element, "FacingOrientation", None)
        if facing is None:
            return None
        x, y = float(facing.X), float(facing.Y)
        length = math.sqrt(x * x + y * y)
        if length < 1e-9:
            return None
        return {"x": x / length, "y": y / length}
    except Exception:
        return None


def _super_component(element):
    """`(id, category id, category name)` of the instance containing this one,
    or three Nones. Comparing the two CATEGORIES is the nested-leaf test: a door
    inside a door is a leaf, a window inside a door is a different statement
    about the model."""
    try:
        parent = getattr(element, "SuperComponent", None)
        if parent is None:
            return None, None, None
        category = getattr(parent, "Category", None)
        if category is None:
            return _element_id_str(parent.Id), None, None
        return _element_id_str(parent.Id), _element_id_str(category.Id), category.Name
    except Exception:
        return None, None, None


def _host_facts(element):
    """`{id, class, is_curtain_wall}` for the element's host, or None.

    `is_curtain_wall` is read off the host wall's `CurtainGrid`, which is None
    on a basic wall. It is one of two independent signals for the curtain-wall
    question -- the other is the symbol test below -- and they are recorded
    separately rather than merged, because a window hosted in a curtain wall and
    a window whose TYPE is a curtain panel are not the same population and the
    analyser should be free to find that out."""
    try:
        host = getattr(element, "Host", None)
        if host is None:
            return None
        grid = None
        try:
            grid = getattr(host, "CurtainGrid", None)
        except Exception:
            grid = None
        return {
            "id": _element_id_str(host.Id),
            "class": type(host).__name__,
            "is_curtain_wall": grid is not None,
        }
    except Exception:
        return None


# --------------------------------------------------------------------------
# Document-level context
# --------------------------------------------------------------------------


def curtain_panel_symbol_ids(doc):
    """The ids of family symbols that behave as curtain-wall panels.

    A transcription of duHast's `get_curtain_wall_door_symbols`
    (`Revit/Doors/doors.py`), which has **no window equivalent** --
    `Revit/Windows/windows.py` stops at the plain `OST_Windows` collector. That
    absence is exactly the risk the plan records as C3, so the test is done here
    to size it rather than assumed away.

    The test is Revit's own: a symbol whose `GetSimilarTypes()` includes any
    curtain-panel symbol is one, because Revit groups panel-substitutable types
    together. Cheaper and more honest than matching family names."""
    from Autodesk.Revit.DB import (
        BuiltInCategory,
        ElementCategoryFilter,
        FamilySymbol,
        FilteredElementCollector,
    )

    panels = (
        FilteredElementCollector(doc)
        .OfClass(FamilySymbol)
        .WherePasses(ElementCategoryFilter(BuiltInCategory.OST_CurtainWallPanels))
    )
    panel_ids = set(_element_id_str(p.Id) for p in panels)
    if not panel_ids:
        return set()

    matched = set()
    for category in (BuiltInCategory.OST_Windows, BuiltInCategory.OST_Doors):
        symbols = (
            FilteredElementCollector(doc)
            .OfClass(FamilySymbol)
            .WherePasses(ElementCategoryFilter(category))
        )
        for symbol in symbols:
            try:
                similar = set(_element_id_str(i) for i in symbol.GetSimilarTypes())
            except Exception:
                continue
            if similar & panel_ids:
                matched.add(_element_id_str(symbol.Id))
    return matched


def document_levels(doc):
    """Every level as `{id, name, elevation}` in decimal feet, ordered by
    elevation.

    Ordered because C4 is answered by comparing a window against the level
    ABOVE the one it names, and "the level above" only exists in a sorted
    list."""
    from Autodesk.Revit.DB import FilteredElementCollector, Level

    levels = []
    for level in FilteredElementCollector(doc).OfClass(Level).ToElements():
        try:
            # BOTH elevations, because they are not the same number and the
            # difference is not academic. On House A `Elevation` reads 361-382 ft
            # (survey based) while element bounding boxes sit at 4-20 ft, and
            # comparing the two directly made 33 of 47 correctly-placed windows
            # look a storey low. Recording both lets the analyser pick the frame
            # that matches the geometry instead of inferring the offset.
            project_elevation = None
            try:
                project_elevation = float(level.ProjectElevation)
            except Exception:
                project_elevation = None
            levels.append(
                {
                    "id": _element_id_str(level.Id),
                    "name": level.Name,
                    "elevation": float(level.Elevation),
                    "project_elevation": project_elevation,
                }
            )
        except Exception:
            continue
    levels.sort(key=lambda l: l["elevation"])
    return levels


def build_context(doc):
    """Everything read once per document rather than once per element."""
    from room_m.utils.generic import document_phases

    phases = document_phases(doc)
    levels = document_levels(doc)
    return {
        "phases": phases,
        "phase_objects": list(doc.Phases),
        "phase_order_by_id": dict((p["id"], i) for i, p in enumerate(phases)),
        "levels": levels,
        "level_by_id": dict((l["id"], l) for l in levels),
        "curtain_symbol_ids": curtain_panel_symbol_ids(doc),
    }


# --------------------------------------------------------------------------
# Per-element probe
# --------------------------------------------------------------------------


def rooms_by_phase(element, context, sides):
    """This element's room references in EVERY phase.

    Every phase, not the one a user picked, because the probe must not need a
    phase dialog to be useful and because "which phases carry a reference at
    all" is itself one of the things being measured. The analyser picks a phase
    offline.

    Read through the extractor's own `room_reference`, so a disagreement between
    this and production is impossible by construction."""
    from room_m.utils.room_refs import room_reference

    out = []
    for index, phase in enumerate(context["phase_objects"]):
        entry = {
            "phase_index": index,
            "phase_id": context["phases"][index]["id"],
            "phase_name": context["phases"][index]["name"],
        }
        for side in sides:
            try:
                entry[side.lower()] = room_reference(element, phase, side)
            except Exception as error:
                entry[side.lower()] = None
                entry[side.lower() + "_error"] = str(error)
        out.append(entry)
    return out


def phase_membership(element, context):
    """`{created, demolished, exists_in}` -- the phase names this element was
    built in, demolished in, and exists in.

    `exists_in` runs the extractor's `exists_in_phase` predicate over the phase
    sequence rather than re-deriving the rule, for the reason
    `ensure_room_m_importable` gives. Doors and windows are both
    built-then-demolished categories, so the range test is correct for each --
    which is the one place rooms differ and is worth not forgetting."""
    from room_m.utils.generic import exists_in_phase

    order = context["phase_order_by_id"]
    try:
        created = order.get(_element_id_str(element.CreatedPhaseId))
        demolished = order.get(_element_id_str(element.DemolishedPhaseId))
    except Exception:
        return {"created": None, "demolished": None, "exists_in": []}

    names = [p["name"] for p in context["phases"]]
    return {
        "created": names[created] if created is not None else None,
        "demolished": names[demolished] if demolished is not None else None,
        "exists_in": [
            names[i] for i in range(len(names)) if exists_in_phase(created, demolished, i)
        ],
    }


def probe_element(element, context, sides):
    """Everything the probe knows about one instance."""
    symbol = getattr(element, "Symbol", None)
    symbol_id = _element_id_str(symbol.Id) if symbol is not None else None
    parent_id, parent_category_id, parent_category_name = _super_component(element)
    own_category = getattr(element, "Category", None)
    min_z, max_z = _bbox_z_extents(element)

    level_id = None
    try:
        level_id = _element_id_str(element.LevelId)
    except Exception:
        level_id = None

    return {
        "id": _element_id_str(element.Id),
        "mark": _param_as_string(element, "ALL_MODEL_MARK"),
        "family_name": getattr(symbol, "FamilyName", None) if symbol is not None else None,
        "type_name": _param_as_string(element, "SYMBOL_NAME_PARAM"),
        "symbol_id": symbol_id,
        "category_id": _element_id_str(own_category.Id) if own_category is not None else None,
        "category_name": own_category.Name if own_category is not None else None,
        "super_component_id": parent_id,
        "super_component_category_id": parent_category_id,
        "super_component_category_name": parent_category_name,
        # The nested-leaf test, stated here rather than left to the analyser
        # because it needs both categories and only this side has them.
        "is_nested_in_same_category": (
            parent_category_id is not None
            and own_category is not None
            and parent_category_id == _element_id_str(own_category.Id)
        ),
        "is_curtain_panel_type": symbol_id in context["curtain_symbol_ids"]
        if symbol_id is not None
        else False,
        "host": _host_facts(element),
        "level_id": level_id,
        "sill_height": _param_as_double(element, "INSTANCE_SILL_HEIGHT_PARAM"),
        "head_height": _param_as_double(element, "INSTANCE_HEAD_HEIGHT_PARAM"),
        "level_offset": _param_as_double(element, "INSTANCE_ELEVATION_PARAM"),
        "bbox_min_z": min_z,
        "bbox_max_z": max_z,
        "insertion_point": _location_point(element),
        "facing": _facing_plan(element),
        "phase": phase_membership(element, context),
        "rooms_by_phase": rooms_by_phase(element, context, sides),
    }


# --------------------------------------------------------------------------
# Per-entity passes
# --------------------------------------------------------------------------


def collect_instances(doc, category_name):
    """Every placed instance of one category, by the same collector duHast
    uses -- so a count difference between the probe and the export is about the
    export's own guards and never about a different question being asked."""
    from Autodesk.Revit.DB import (
        BuiltInCategory,
        ElementCategoryFilter,
        FamilyInstance,
        FilteredElementCollector,
    )

    category = getattr(BuiltInCategory, category_name)
    return list(
        FilteredElementCollector(doc)
        .OfClass(FamilyInstance)
        .WherePasses(ElementCategoryFilter(category))
    )


def run_export(doc, spec):
    """The duHast export for one entity, exactly as the real exporter takes
    it -- no filtering, no translation. What lands on disk is what duHast
    said, so a later argument about the export is settled by re-reading this
    file rather than by re-running Revit."""
    from duHast.Data.Utils.data_to_file import build_json_for_file

    module = __import__(spec["module"], fromlist=[spec["getter"]])
    getter = getattr(module, spec["getter"])
    data = getter(doc)
    return build_json_for_file({spec["list_key"]: data}, "{}".format(doc.Title))


def run_probe(doc, spec, context):
    """The probe pass for one entity: one collector walk, one record each."""
    records = {}
    for element in collect_instances(doc, spec["category"]):
        try:
            record = probe_element(element, context, spec["sides"])
        except Exception as error:
            # A record that names its own failure, so the analyser counts it
            # instead of the element vanishing from both sides of the tally.
            record = {"id": _element_id_str(element.Id), "probe_error": str(error)}
        records[record["id"]] = record
    return records


# --------------------------------------------------------------------------
# Output
# --------------------------------------------------------------------------


# Python 2 and 3 spell their string and integer types differently, and this file
# runs on both. Resolved once here rather than guarded at each use.
try:
    STRING_TYPES = (str, unicode)  # noqa: F821 - IronPython 2.7
    NUMBER_TYPES = (int, long, float)  # noqa: F821 - IronPython 2.7
except NameError:
    STRING_TYPES = (str,)
    NUMBER_TYPES = (int, float)


def plainify(value, path="", problems=None):
    """duHast data objects to plain JSON types, recording -- rather than raising
    on -- anything that will not convert.

    **duHast's own serializer cannot be used here, and the reason is worth
    keeping.** `serialize_utf` delegates to `Base.to_json`, which calls
    `json.dumps` with **no `default=` handler**, over a `class_to_dict()` whose
    inner `serialize()` ends in `else: return obj` -- so any CLR object duHast
    did not anticipate passes through untouched and then raises. That makes the
    export all-or-nothing: one unconvertible leaf on one window and the whole
    document produces no file at all. Measured against a real model on
    2026-09-01, one does (`<property# Name on Element>`), and the likely origin
    is `get_type_properties`: it assigns `encode_utf8(Element.Name.GetValue(e))`,
    and `encode_utf8` returns a non-string argument UNCHANGED, so a CLR
    descriptor lands in `type_properties.name` with nothing complaining until
    serialisation.

    An instrument whose job is to describe a model nobody has looked at yet must
    not have that failure mode. So the conversion happens here, `class_to_dict()`
    is still used for each duHast object (it is correct, and it handles the .NET
    `Int64` conversion), and the unconvertible leaf becomes a marker string plus
    an entry in `problems` naming its path. The export lands, and *which* field
    would not convert becomes a finding rather than a stack trace.

    Returns `(plain_value, problems)`."""
    if problems is None:
        problems = []

    if value is None or isinstance(value, bool):
        return value, problems
    if isinstance(value, NUMBER_TYPES) or isinstance(value, STRING_TYPES):
        return value, problems

    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            key_str = key if isinstance(key, STRING_TYPES) else str(key)
            out[key_str], problems = plainify(item, "{}.{}".format(path, key_str), problems)
        return out, problems

    if isinstance(value, (list, tuple)):
        out = []
        for index, item in enumerate(value):
            converted, problems = plainify(item, "{}[{}]".format(path, index), problems)
            out.append(converted)
        return out, problems

    # A duHast data object. `class_to_dict` already recurses its own children and
    # converts Int64, so this hands back a structure whose only possible
    # survivors are the leaves duHast did not expect -- which the recursion above
    # then catches.
    to_dict = getattr(value, "class_to_dict", None)
    if callable(to_dict):
        try:
            return plainify(to_dict(), path, problems)
        except Exception as error:
            problems.append({"path": path, "type": type(value).__name__, "error": str(error)})
            return "<class_to_dict failed: {}>".format(type(value).__name__), problems

    # `__dict__` only when it holds something. An EMPTY one means the fallback
    # learned nothing, and emitting `{}` for it would quietly turn an
    # unconvertible field into an empty object -- the field would then read as
    # "present but blank" in the coverage table, which is the one wrong answer
    # this whole function exists to avoid. A CLR property descriptor is exactly
    # that shape.
    attributes = getattr(value, "__dict__", None)
    if isinstance(attributes, dict) and attributes:
        return plainify(attributes, path, problems)

    problems.append({"path": path, "type": type(value).__name__, "repr": repr(value)})
    return "<unconvertible: {}>".format(type(value).__name__), problems


def write_json(path, payload):
    """Write `payload` as UTF-8 JSON with LF endings.

    Binary mode on purpose: `.gitattributes` enforces LF and a text-mode write
    on Windows would produce CRLF in a file destined for the repo -- the trap
    `CLAUDE.md` records under Traps.

    `payload` must already be plain (see `plainify`); nothing here converts, so a
    caller that forgets fails loudly rather than writing half a file."""
    text = json.dumps(payload, indent=2, ensure_ascii=False, sort_keys=True)

    directory = os.path.dirname(os.path.abspath(path))
    if directory and not os.path.isdir(directory):
        os.makedirs(directory)
    with io.open(path, "wb") as handle:
        handle.write(text.encode("utf-8"))
    return path


def probe_document(doc, out_dir, entities=("windows", "doors")):
    """Export and probe one document, writing two files per entity.

    Returns the list of paths written, so the caller can print them -- a probe
    whose output location is a guess is a probe whose output gets lost."""
    ensure_room_m_importable()
    context = build_context(doc)
    written = []

    shared = {
        "probe_version": PROBE_VERSION,
        "document": {"title": doc.Title, "path": getattr(doc, "PathName", None)},
        "phases": context["phases"],
        "levels": context["levels"],
        "curtain_panel_symbol_count": len(context["curtain_symbol_ids"]),
    }

    for entity in entities:
        spec = ENTITY_SPECS[entity]
        suffix = OUT_SUFFIX[entity]

        raw = run_export(doc, spec)
        raw_plain, problems = plainify(raw, entity)
        if problems:
            # Loud, because a silently degraded export is the one outcome worse
            # than no export: the file still parses and the analyser still runs.
            print(
                "  WARNING: {} field(s) would not convert; see "
                "serialisation_problems in the probe file".format(len(problems))
            )
        written.append(
            write_json(
                os.path.join(out_dir, "{}-raw{}.json".format(entity, suffix)), raw_plain
            )
        )

        records = run_probe(doc, spec, context)
        probe = dict(shared)
        probe["entity"] = entity
        probe["list_key"] = spec["list_key"]
        # Which export fields duHast could not serialise, and where. A finding in
        # its own right: a field that will not convert is a field the contract
        # cannot carry, whatever the plan assumed about it.
        probe["serialisation_problems"] = problems
        # Both counts, side by side, because their DIFFERENCE is a finding: the
        # export drops any instance it could not measure, and the size of that
        # drop is the first thing question 1 asks.
        probe["collected_count"] = len(records)
        probe["exported_count"] = len(raw.get(spec["list_key"], []) or [])
        probe["elements"] = records
        written.append(
            write_json(
                os.path.join(out_dir, "{}-probe{}.json".format(entity, suffix)), probe
            )
        )

    return written


# --------------------------------------------------------------------------
# Entry point
# --------------------------------------------------------------------------


def resolve_documents():
    """The documents to probe: whatever the user picks, else the active one.

    Uses duHast's `pick_document` when pyRevit's forms are reachable, so a
    LINKED model can be probed -- a facade file holding the windows while the
    rooms live elsewhere is the split-model case the plan cares about, and it is
    exactly the document a probe run from the host model would miss."""
    host = None
    try:
        host = __revit__.ActiveUIDocument.Document  # noqa: F821 - pyRevit global
    except Exception:
        host = None

    try:
        from pyrevit import forms
        from duHast.pyRevit.UI.doc_selector import pick_document

        picked = pick_document(
            host, forms, button_name="Select model(s) to probe", multiselect=True
        )
        if picked:
            return list(picked)
    except Exception:
        pass

    if host is None:
        raise RuntimeError(
            "no Revit document: run this from pyRevit with a model open, or call "
            "probe_document(doc, out_dir) yourself"
        )
    return [host]


def main(out_dir=None, entities=("windows", "doors")):
    out_dir = out_dir or default_out_dir()
    if not out_dir:
        raise RuntimeError(
            "cannot resolve an output directory (no __file__): pass one, e.g. "
            "main(out_dir=r'C:\\\\temp\\\\probe')"
        )

    print("probe_windows_export v{} (plainify: on)".format(PROBE_VERSION))
    for doc in resolve_documents():
        print("probing {} -> {}".format(doc.Title, out_dir))
        for path in probe_document(doc, out_dir, entities):
            print("  wrote {}".format(path))
    print("done. Now run: python scripts/analyse_windows_probe.py")

sys.path += [r"C:\Users\janchristel\Documents\GitHub\SampleCodeRevitBatchProcessor-NET8\src", r"C:\Users\janchristel\Documents\GitHub\SampleCodeRevitBatchProcessor-NET8\Samples\pyRevit\Extensions\duHast-2025.extension\duHast.tab\lib"]

if __name__ == "__main__":
    main(r"C:\Users\janchristel\Documents\GitHub\room_mate\temp")
