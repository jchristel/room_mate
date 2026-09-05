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
Capture the raw duHast FFE (item) export, plus the Revit facts the export
cannot carry, so the six questions gating `docs/PLAN-ffe.md` are answered off a
measurement instead of off a reading of duHast's source.

    pyRevit  ->  run this script against the model
    outputs  ->  scripts/fixtures/ffe-raw.json
                 scripts/fixtures/ffe-probe.json
                 scripts/fixtures/doors-raw-control.json
                 scripts/fixtures/doors-probe-control.json

Then, on any machine:

    python scripts/analyse_ffe_probe.py

**This script decides nothing.** It collects, and it counts; every judgement --
is this level wrong, is that instance a nested leaf, did the export drop
something that matters -- happens in the analyser, offline, where it can be
re-run and argued with. That split is the whole point: the expensive half needs
Revit and must be right first time, the cheap half is where the thinking goes.

THE SIX QUESTIONS, and where each is answered
  Q1  Does `get_Room(phase)` populate?   -> `rooms_by_phase` on every record.
      The plan's kill condition. FFE and rooms live in one file, so Revit should
      know which room an item is in; if it does not, there is no join to perform
      and the entity carries little rooms do not already have.
  Q2  What did the export drop, per category?  -> `collected` against `exported`.
      `populate_data_item_object` returns None for any instance whose Location
      is not a `LocationPoint`, and the caller does not append it, so the loss
      leaves no hole. `location_class` on each record is what makes the drop
      attributable rather than merely countable.
  Q3  The category histogram.  -> one collector pass per category, counted
      separately. All nine of duHast's defaults are walked, `OST_GenericModel`
      included, because that is what the plan commits to pushing.
  Q4  The super-component matrix.  -> `super_component_category_name` per
      record. `nested_opening_ids`' test is "is the parent the same category",
      which is well defined for ONE category and, measured on House A, catches
      5.6% of the nested population across nine.
  Q5  Is the bbox-derived level the level the item names?  -> both are recorded
      (`level_id_property` against `level_by_bbox_*`), never reconciled here.
  Q6  What unit is each numeric field in?  -> the export is written verbatim,
      and every geometric fact the probe reads is in decimal feet. The analyser
      divides one by the other; a ratio near 304.8 is the answer.

WHY DOORS ARE EXPORTED TOO
The plan rests on "an item is NOT a door", which is a claim about what is
absent, and absence is the one thing a single export cannot demonstrate. The
committed `scripts/fixtures/doors-raw.json` cannot serve as the control: it was
captured months ago from an older duHast, so a field missing from it proves
nothing about today. A door control taken from THIS document with THIS duHast in
the same pass makes the comparison direct. It is written to
`doors-*-control.json` and never over the committed fixture, which
`gen_doors_sample.py` still depends on.

WHAT THIS PROBE ADDS THAT ITS WINDOWS SIBLING DID NOT
- `location_class`, because the FFE export's silent drop is keyed on it and no
  other question can be answered until that population is known;
- BOTH duHast bounding boxes for the same instance -- the oriented one that
  keeps the rotation and the solids one that walks `GetSubComponentIds()`. The
  plan's upstream change U2 asks for the two to be merged; recording both is
  what turns "furniture nests more than doors do" from an assertion into a
  measurement.

WHY THIS IS NOT A ROW IN `probe_windows_export.py`
That script says adding FF&E is "a row, not a branch", and it was nearly right:
`rooms_by_phase`, `phase_membership` and the entity spec table all take an FFE
row unchanged. What does not fit is everything either side of it. FFE is nine
categories rather than one, so the collector, the histogram and the drop count
are all per category; its export getter carries no category at all; and three of
that probe's questions (curtain-wall panels, sill and head heights, host walls)
are meaningless for furniture. The small generic helpers below are therefore
DUPLICATED rather than imported, per the house rule that a shared helper is
copied per module rather than hoisted -- and because these two scripts are run
by exec'ing their text inside pyRevit, where a cross-script import is the
fragile thing in the room. The windows probe is also the record of a completed
measurement that the archived windows plan cites by name; renaming or reshaping
it would make that account wrong about a file that still exists.

REVIT AND duHast IMPORTS ARE DEFERRED into the functions that need them, so this
file parses, lints and reads on a machine with no Revit -- which is where it
will be edited. Only the standard library is imported at module scope.

Written to run on IronPython 2.7 and CPython 3 alike: `.format()` rather than
f-strings, no type hints, ASCII only (IronPython will not parse a file with an
em-dash in it, not even in a docstring), and every file written in binary so the
line endings are LF on Windows too (`.gitattributes` enforces LF, and a
text-mode write here would silently produce CRLF).
"""

import io
import json
import math
import os
import sys


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
    "which room is this instance in, in this phase" could disagree with the one
    that ships, and then the measurement would be of the probe rather than of
    the model. Q1 is the kill condition, so it is the one answer that must be
    production's answer.
    """
    here = globals().get("__file__")
    if not here:
        return
    root = os.path.dirname(os.path.dirname(os.path.abspath(here)))
    candidate = os.path.join(root, "extractor", "pyRevit")
    if os.path.isdir(candidate) and candidate not in sys.path:
        sys.path.insert(0, candidate)


# duHast's DEFAULT_ITEM_CATEGORIES, transcribed rather than imported so this
# file still parses with no Revit present. Transcribed lists drift, so the
# probe CHECKS itself against duHast at run time (see `resolve_item_categories`)
# and records any difference rather than silently measuring the wrong set.
#
# **That guard is now load-bearing rather than defensive.** `OST_Casework` was
# added on 2026-09-05 in BOTH places, and the two have to move together: the
# probe walks this list while `run_export` calls `get_all_item_data(doc)` with
# no argument, so a list that has drifted has the probe counting one population
# and the export reporting another. The analyser refuses to interpret a run
# where they differ.
#
# Nine, `OST_GenericModel` included, because that is what PLAN-ffe D6 commits to
# pushing. Whether that was wise is exactly what Q3 is for -- and on House A it
# was the largest category of the eight, at 201 of 647.
FFE_CATEGORY_NAMES = [
    "OST_Furniture",
    "OST_FurnitureSystems",
    # The House A finding, and the reason this list changed: 87 of 179 nested
    # instances were furniture components of CASEWORK, which was not exported at
    # all. The handles shipped and the joinery they belong to did not.
    "OST_Casework",
    "OST_MechanicalEquipment",
    "OST_ElectricalEquipment",
    "OST_ElectricalFixtures",
    "OST_PlumbingFixtures",
    "OST_SpecialityEquipment",
    "OST_GenericModel",
]

# The two entities probed. FFE carries a LIST of categories where an opening
# carries one, and `"Room"` where an opening carries a pair -- the two
# differences PLAN-ffe D1 and D4 turn on, stated here as data.
ENTITY_SPECS = {
    "ffe": {
        "categories": FFE_CATEGORY_NAMES,
        "module": "duHast.Revit.Family.Export.to_data_item",
        "getter": "get_all_item_data",
        "list_key": "item",
        "sides": ("Room",),
    },
    "doors": {
        "categories": ["OST_Doors"],
        "module": "duHast.Revit.Doors.Export.to_data_door",
        "getter": "get_all_door_data",
        "list_key": "door",
        "sides": ("FromRoom", "ToRoom"),
    },
}

# Doors are the control, so their outputs must not land on the committed
# fixture `gen_doors_sample.py` reads.
OUT_SUFFIX = {"ffe": "", "doors": "-control"}

# Bumped whenever the probe's OUTPUT or its conversion changes. Printed at
# startup because this script is run by exec'ing its text, so nothing else tells
# a reader which copy actually ran -- and a stale copy has already cost two runs
# on the windows probe, both diagnosed from a traceback naming old line numbers.
#
# v2: `OST_Casework` joined the category list (D6/U4), so a v1 capture and a v2
# capture describe different populations and their counts must not be compared.
# The recorded `probe_categories` says which was walked, and the analyser
# compares that against the export's own list rather than against this number --
# but a version that did not move would leave two files on disk looking alike.
PROBE_VERSION = 2


# --------------------------------------------------------------------------
# Small Revit accessors. Each one answers None rather than raising, because a
# single unreadable instance must cost its own field and not the run: the probe
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


def _param_as_element_id(element, builtin_name):
    """One built-in parameter as an element id string, or None.

    Its own accessor rather than `_param_as_string`, because the level question
    (Q5) is an identity comparison and a display NAME cannot answer it: two
    levels in one document may legitimately share a name, and the bbox rule
    yields an id."""
    from Autodesk.Revit.DB import BuiltInParameter

    try:
        definition = getattr(BuiltInParameter, builtin_name, None)
        if definition is None:
            return None
        param = element.get_Parameter(definition)
        if param is None or not param.HasValue:
            return None
        value = param.AsElementId()
        if value is None:
            return None
        text = _element_id_str(value)
        # Revit's "no element" id. Returned as None so a reader is not left
        # deciding whether -1 is an id or a sentinel.
        return None if text in (None, "-1") else text
    except Exception:
        return None


def _location_facts(element):
    """`{class, point}` -- what KIND of location this instance has, and its
    point when it has one.

    **The single most important field this probe collects**, and the reason is
    that its absence is invisible everywhere else. `populate_data_item_object`
    returns None for anything that is not a `LocationPoint`, and
    `get_all_item_data` does not append what it did not populate, so a
    line-based family leaves no hole in the export. Recording the class here
    turns "the export has fewer items than the model" into "these N instances,
    in these categories, were line-based", which is the difference between a
    scheduling decision and a mystery.

    The point is in decimal feet, Revit's internal units, and Z is kept although
    the contract drops it -- the storey question (Q5) needs an elevation and
    this is the cheapest one."""
    try:
        location = getattr(element, "Location", None)
        if location is None:
            return {"class": None, "point": None}
        class_name = type(location).__name__
        point = getattr(location, "Point", None)
        if point is None:
            return {"class": class_name, "point": None}
        return {
            "class": class_name,
            "point": {"x": float(point.X), "y": float(point.Y), "z": float(point.Z)},
        }
    except Exception:
        return {"class": None, "point": None}


def _facing_plan(element):
    """`FacingOrientation` projected to plan and normalised, or None when it has
    no plan component.

    The same rule and the same reason as
    `room_m.utils.openings.opening_through_wall_normal` -- restated rather than
    imported because that helper belongs to the opening module and the probe
    must not be the thing that refactors it.

    Recorded for FFE even though PLAN-ffe D2 takes the rotation from the
    export's `location_point.rotation_coord` instead. Two readings of one fact
    are worth having exactly once: if they disagree, the decision to trust the
    export is the thing that needs revisiting, and this is the only run that
    will have both."""
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
    or three Nones.

    Comparing the two CATEGORIES is the nested-leaf test, and across nine
    categories it stops being a yes/no: a generic model inside a furniture item
    is a different statement about the model from a chair inside a chair. The
    name is carried as well as the id so the analyser can build a readable
    matrix rather than a table of numbers."""
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


def _sub_component_count(element):
    """How many nested shared instances this one contains, or None.

    The other half of the U2 question. `get_oriented_bounding_box_from_family_instance`
    measures only `family_instance.get_Geometry()` while
    `get_solids_based_bounding_box_from_family_instance` merges
    `GetSubComponentIds()`, so an instance with sub-components is exactly where
    the two boxes can disagree -- and this count says how much of the model is
    in that population before the boxes are compared at all."""
    try:
        ids = element.GetSubComponentIds()
        return 0 if ids is None else len(list(ids))
    except Exception:
        return None


def _bbox_extents(box):
    """A Revit `BoundingBoxXYZ` as `{min, max, has_transform}` in decimal feet,
    or None.

    `has_transform` rather than the transform itself: the oriented box carries
    its placement there and the axis-aligned one does not, and for the question
    being asked -- do the two boxes describe the same volume -- whether a
    transform is present is the fact that explains a difference. The full matrix
    would be four times the bytes for a question nobody has asked yet."""
    try:
        if box is None:
            return None
        transform = getattr(box, "Transform", None)
        identity = True
        if transform is not None:
            try:
                identity = bool(transform.IsIdentity)
            except Exception:
                identity = False
        return {
            "min": {"x": float(box.Min.X), "y": float(box.Min.Y), "z": float(box.Min.Z)},
            "max": {"x": float(box.Max.X), "y": float(box.Max.Y), "z": float(box.Max.Z)},
            "transform_is_identity": identity,
        }
    except Exception:
        return None


def _duhast_bounding_boxes(doc, element):
    """Both of duHast's boxes for one instance, as `{oriented, solids}`.

    The measurement behind PLAN-ffe's upstream change U2. The oriented box is
    the one U1 needs, because it is the only one that keeps the rotation -- an
    axis-aligned box of a rotated object no longer records the angle, and
    recovering it is degenerate at forty-five degrees. But it does not walk
    sub-components, so a desk whose drawers are nested shared families measures
    to the desk top alone.

    Both are recorded and neither is judged. Whether the difference matters is
    a question about THIS model's families, and the analyser answers it against
    `sub_component_count`."""
    out = {"oriented": None, "solids": None, "error": None}
    try:
        from duHast.Revit.Common.Geometry.solids import (
            get_oriented_bounding_box_from_family_instance,
        )
        from duHast.Revit.Family.family_geometry import (
            get_solids_based_bounding_box_from_family_instance,
        )
    except Exception as error:
        out["error"] = "import: {}".format(error)
        return out

    try:
        out["oriented"] = _bbox_extents(
            get_oriented_bounding_box_from_family_instance(element)
        )
    except Exception as error:
        out["error"] = "oriented: {}".format(error)
    try:
        out["solids"] = _bbox_extents(
            get_solids_based_bounding_box_from_family_instance(doc, element)
        )
    except Exception as error:
        out["error"] = "{}; solids: {}".format(out["error"] or "", error)
    return out


def _world_bbox_z(element):
    """The instance's WORLD axis-aligned Z range as `(min, max)` in decimal
    feet.

    World-aligned on purpose, unlike the oriented box above: the question here
    is "how high off the level does this sit", and height is the one dimension
    an axis-aligned box gets right regardless of how the object is rotated in
    plan. It is also the number duHast's own level heuristic reads, so Q5
    compares like with like."""
    try:
        box = element.get_BoundingBox(None)
        if box is None:
            return None, None
        return float(box.Min.Z), float(box.Max.Z)
    except Exception:
        return None, None


# --------------------------------------------------------------------------
# Document-level context
# --------------------------------------------------------------------------


def resolve_item_categories(names):
    """`[(name, BuiltInCategory)]` for the probe's category list, plus whatever
    duHast's own default list says.

    **The list above is a transcription, and transcriptions drift.** duHast is
    the authority on which categories its exporter walks, so the probe asks it
    and records any disagreement rather than measuring a set the export never
    saw. A probe that quietly walked seven categories while the export walked
    nine would report a drop that was its own."""
    from Autodesk.Revit.DB import BuiltInCategory

    resolved = []
    unknown = []
    for name in names:
        category = getattr(BuiltInCategory, name, None)
        if category is None:
            unknown.append(name)
            continue
        resolved.append((name, category))

    duhast_names = None
    try:
        from duHast.Revit.Family.Export.to_data_item import DEFAULT_ITEM_CATEGORIES

        duhast_names = [str(c).split(".")[-1] for c in DEFAULT_ITEM_CATEGORIES]
    except Exception:
        duhast_names = None

    return resolved, unknown, duhast_names


def document_levels(doc):
    """Every level as `{id, name, elevation, project_elevation}` in decimal
    feet, ordered by elevation.

    BOTH elevations, because they are not the same number and the difference is
    not academic: on House A `Elevation` reads 361-382 ft (survey based) while
    element bounding boxes sit at 4-20 ft, and comparing the two directly made
    33 of 47 correctly-placed windows look a storey low. Recording both lets the
    analyser pick the frame that matches the geometry instead of inferring the
    offset. duHast's item level heuristic reads `ProjectElevation`, so Q5 needs
    that one specifically.

    Ordered because "the level above the one it names" only exists in a sorted
    list."""
    from Autodesk.Revit.DB import FilteredElementCollector, Level

    levels = []
    for level in FilteredElementCollector(doc).OfClass(Level).ToElements():
        try:
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
    categories, unknown, duhast_names = resolve_item_categories(FFE_CATEGORY_NAMES)
    return {
        "doc": doc,
        "phases": phases,
        "phase_objects": list(doc.Phases),
        "phase_order_by_id": dict((p["id"], i) for i, p in enumerate(phases)),
        "levels": levels,
        "level_by_id": dict((l["id"], l) for l in levels),
        "ffe_categories": categories,
        "unknown_category_names": unknown,
        "duhast_default_categories": duhast_names,
    }


# --------------------------------------------------------------------------
# Per-element probe
# --------------------------------------------------------------------------


def rooms_by_phase(element, context, sides):
    """This element's room references in EVERY phase.

    Every phase, not the one a user picked, because the probe must not need a
    phase dialog to be useful and because "which phases carry a reference at
    all" is itself part of Q1. The analyser picks a phase offline.

    Read through the extractor's own `room_reference`, so a disagreement between
    this and production is impossible by construction. For FFE `sides` is the
    single name `"Room"` -- the argument `room_refs.room_reference` has carried
    since before this entity existed, for exactly this day."""
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
    sequence rather than re-deriving the rule. FFE is a built-then-demolished
    category like doors and windows, so the RANGE test is correct for it -- and
    confirming that on real data is worth the line, because running an element
    through the room predicate returns nothing, silently, which is the failure
    that cost five empty pushes."""
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


def level_by_bbox(min_z, context):
    """duHast's item level rule, re-run here so Q5 compares two answers rather
    than one answer and a guess.

    A transcription of `get_level_data_by_bounding_box`: the last level whose
    `ProjectElevation` is at or below the instance's solid bbox minimum Z, with
    the lowest level as the fallback for anything below all of them. Re-stated
    rather than imported because importing it would need the duHast helper to be
    handed a Revit element, and what is wanted here is the rule applied to a
    number the probe has already recorded -- so that a difference between this
    and the export is about the export's INPUT, not about two different rules.

    Returns `(level_id, was_fallback)`. `was_fallback` is separate because
    "below every level in the document" is a distinct and more alarming state
    than "assigned to the storey below the one it serves"."""
    if min_z is None:
        return None, False
    ordered = [l for l in context["levels"] if l.get("project_elevation") is not None]
    ordered = sorted(ordered, key=lambda l: l["project_elevation"])
    if not ordered:
        return None, False
    best = None
    for level in ordered:
        if level["project_elevation"] <= min_z:
            best = level
        else:
            break
    if best is None:
        return ordered[0]["id"], True
    return best["id"], False


def probe_element(doc, element, context, sides, category_name):
    """Everything the probe knows about one instance."""
    symbol = getattr(element, "Symbol", None)
    symbol_id = _element_id_str(symbol.Id) if symbol is not None else None
    parent_id, parent_category_id, parent_category_name = _super_component(element)
    own_category = getattr(element, "Category", None)
    own_category_id = _element_id_str(own_category.Id) if own_category is not None else None
    min_z, max_z = _world_bbox_z(element)
    derived_level, derived_was_fallback = level_by_bbox(min_z, context)

    # Three ways an instance can name its level, because family hosting types
    # disagree about which one is populated and Q5 must not turn on having
    # guessed right. Recorded separately, never merged.
    level_id = None
    try:
        level_id = _element_id_str(element.LevelId)
    except Exception:
        level_id = None

    return {
        "id": _element_id_str(element.Id),
        "collected_as": category_name,
        "mark": _param_as_string(element, "ALL_MODEL_MARK"),
        "family_name": getattr(symbol, "FamilyName", None) if symbol is not None else None,
        "type_name": _param_as_string(element, "SYMBOL_NAME_PARAM"),
        "symbol_id": symbol_id,
        "category_id": own_category_id,
        "category_name": own_category.Name if own_category is not None else None,
        "super_component_id": parent_id,
        "super_component_category_id": parent_category_id,
        "super_component_category_name": parent_category_name,
        # The nested-leaf test as `nested_opening_ids` states it, computed here
        # because it needs both categories and only this side has them. Across
        # nine categories it is a starting point rather than the answer, which
        # is why the parent's category NAME is carried beside it.
        "is_nested_in_same_category": (
            parent_category_id is not None
            and own_category_id is not None
            and parent_category_id == own_category_id
        ),
        "sub_component_count": _sub_component_count(element),
        "level_id_property": level_id,
        "level_id_family_param": _param_as_element_id(element, "FAMILY_LEVEL_PARAM"),
        "level_id_schedule_param": _param_as_element_id(
            element, "INSTANCE_SCHEDULE_ONLY_LEVEL_PARAM"
        ),
        "level_by_bbox": derived_level,
        "level_by_bbox_was_fallback": derived_was_fallback,
        "bbox_min_z": min_z,
        "bbox_max_z": max_z,
        "location": _location_facts(element),
        "facing": _facing_plan(element),
        "duhast_bounding_boxes": _duhast_bounding_boxes(doc, element),
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
    """The duHast export for one entity, exactly as the real exporter takes it
    -- no filtering, no translation. What lands on disk is what duHast said, so
    a later argument about the export is settled by re-reading this file rather
    than by re-running Revit.

    The FFE getter takes an optional category list and is called WITHOUT one, so
    the export walks duHast's defaults rather than the probe's transcription of
    them. That is deliberate: the point of the run is to measure what the real
    exporter produces, and `resolve_item_categories` already records any
    disagreement between the two lists."""
    from duHast.Data.Utils.data_to_file import build_json_for_file

    module = __import__(spec["module"], fromlist=[spec["getter"]])
    getter = getattr(module, spec["getter"])
    data = getter(doc)
    return build_json_for_file({spec["list_key"]: data}, "{}".format(doc.Title))


def run_probe(doc, spec, context):
    """The probe pass for one entity: one collector walk per category, one
    record each, keyed by element id.

    **Per category, and the counts are kept per category.** An FFE run walks
    nine, and Q2 and Q3 are both per-category questions -- a drop concentrated
    in `OST_GenericModel` and a drop spread evenly across furniture are
    different findings that a single total cannot separate.

    An element collected under two categories cannot happen (a category is a
    property of the element), but an element appearing twice would silently
    halve a count, so the first record wins and the duplicate is counted."""
    records = {}
    per_category = {}
    duplicates = 0
    for category_name in [c[0] for c in spec.get("resolved_categories", [])]:
        seen = 0
        for element in collect_instances(doc, category_name):
            try:
                record = probe_element(doc, element, context, spec["sides"], category_name)
            except Exception as error:
                # A record that names its own failure, so the analyser counts it
                # instead of the element vanishing from both sides of the tally.
                record = {
                    "id": _element_id_str(element.Id),
                    "collected_as": category_name,
                    "probe_error": str(error),
                }
            seen += 1
            if record["id"] in records:
                duplicates += 1
                continue
            records[record["id"]] = record
        per_category[category_name] = seen
    return records, per_category, duplicates


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
    export all-or-nothing: one unconvertible leaf on one instance and the whole
    document produces no file at all. Measured against a real model on
    2026-09-01, one does (`<property# Name on Element>`), and the likely origin
    is `get_type_properties`: it assigns `encode_utf8(Element.Name.GetValue(e))`,
    and `encode_utf8` returns a non-string argument UNCHANGED, so a CLR
    descriptor lands in `type_properties.name` with nothing complaining until
    serialisation.

    That failure is MORE likely here than it was for windows, not less: FFE
    walks nine categories of loose families rather than one category of
    building fabric, so the population of families nobody has round-tripped
    before is far larger.

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


def probe_document(doc, out_dir, entities=("ffe", "doors")):
    """Export and probe one document, writing two files per entity.

    Returns the list of paths written, so the caller can print them -- a probe
    whose output location is a guess is a probe whose output gets lost."""
    ensure_room_m_importable()
    context = build_context(doc)
    written = []

    if context["unknown_category_names"]:
        print(
            "  WARNING: this Revit version has no BuiltInCategory for: {}".format(
                ", ".join(context["unknown_category_names"])
            )
        )

    shared = {
        "probe_version": PROBE_VERSION,
        "document": {"title": doc.Title, "path": getattr(doc, "PathName", None)},
        "phases": context["phases"],
        "levels": context["levels"],
        # Both category lists, side by side. If they differ, every count below
        # is measuring a different population from the export and the analyser
        # must say so rather than reporting a drop that is its own.
        "probe_categories": [c[0] for c in context["ffe_categories"]],
        "duhast_default_categories": context["duhast_default_categories"],
        "unknown_category_names": context["unknown_category_names"],
    }

    for entity in entities:
        spec = dict(ENTITY_SPECS[entity])
        suffix = OUT_SUFFIX[entity]

        if entity == "ffe":
            spec["resolved_categories"] = context["ffe_categories"]
        else:
            resolved, _unknown, _defaults = resolve_item_categories(spec["categories"])
            spec["resolved_categories"] = resolved

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

        records, per_category, duplicates = run_probe(doc, spec, context)
        probe = dict(shared)
        probe["entity"] = entity
        probe["list_key"] = spec["list_key"]
        probe["sides"] = list(spec["sides"])
        # Which export fields duHast could not serialise, and where. A finding in
        # its own right: a field that will not convert is a field the contract
        # cannot carry, whatever the plan assumed about it.
        probe["serialisation_problems"] = problems
        # Both counts, side by side, because their DIFFERENCE is Q2. The export
        # drops any instance it could not populate and leaves no hole; this is
        # the only place the size of that drop is visible.
        probe["collected_count"] = len(records)
        probe["collected_by_category"] = per_category
        probe["collected_duplicates"] = duplicates
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

    Uses duHast's `pick_document` when pyRevit's forms are reachable. FFE lives
    in the same file as its rooms, so unlike the windows probe this is not
    expected to need a linked model -- but a multiselect run costs nothing and a
    model that turns out to link its furniture is exactly the surprise the probe
    exists to find."""
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


def main(out_dir=None, entities=("ffe", "doors")):
    out_dir = out_dir or default_out_dir()
    if not out_dir:
        raise RuntimeError(
            "cannot resolve an output directory (no __file__): pass one, e.g. "
            "main(out_dir=r'C:\\\\temp\\\\probe')"
        )

    print("probe_ffe_export v{} (plainify: on)".format(PROBE_VERSION))
    for doc in resolve_documents():
        print("probing {} -> {}".format(doc.Title, out_dir))
        for path in probe_document(doc, out_dir, entities):
            print("  wrote {}".format(path))
    print("done. Now run: python scripts/analyse_ffe_probe.py")


sys.path += [r"C:\Users\janchristel\Documents\GitHub\SampleCodeRevitBatchProcessor-NET8\src", r"C:\Users\janchristel\Documents\GitHub\SampleCodeRevitBatchProcessor-NET8\Samples\pyRevit\Extensions\duHast-2025.extension\duHast.tab\lib"]

if __name__ == "__main__":
    main(r"C:\Users\janchristel\Documents\GitHub\room_mate\temp")
