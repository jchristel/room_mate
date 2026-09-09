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
Capture the raw duHast ceiling and room exports, plus the Revit facts neither
export carries, so a ceilings entity is designed off a measurement rather than
off a reading of duHast's source.

    pyRevit  ->  open the FEDERATED HOST and run this script; it walks the
                 host and every loaded link, narrowed by DOCUMENT_PREFIXES
    outputs  ->  scripts/fixtures/ceilings-raw-<document>.json
                 scripts/fixtures/ceilings-probe-<document>.json
                 scripts/fixtures/rooms-raw-<document>.json
                 scripts/fixtures/rooms-probe-<document>.json
                 scripts/fixtures/ceilings-probe-index.json

Then, on any machine:

    python scripts/analyse_ceilings_probe.py

**This script decides nothing.** It collects, and it counts; every judgement --
is this ceiling in that room, is a 0.4% overlap real, is a missing polygon a
model defect or a pipeline one -- happens in the analyser, offline, where it
can be re-run and argued with.

WHY THIS ENTITY NEEDS A PROBE MORE THAN THE LAST THREE DID
Doors, windows and FF&E all carry an authored room reference, so geometry was
only ever the opt-in fallback `[doors] room_resolution` names. **A Revit ceiling
has no room parameter at all, and a room has no ceiling parameter.** The join is
geometric or it does not exist, which moves two ordinary design questions onto
the critical path: whether the geometry is there, and whether the two
populations are even in the same document.

THE QUESTIONS, and where each is answered
  Q1  Does every ceiling export a usable polygon?  -> `collected_count` against
      `exported_count`, and per element `solid_count` / `is_in_place`.
      **The kill condition.** `populate_data_ceiling_object` returns None when
      `get_2d_points_from_solid` yields nothing, and `get_all_ceiling_data`
      then drops the ceiling from the export entirely. A dropped ceiling is
      indistinguishable downstream from one nobody modelled, so the drop RATE
      decides whether this entity can report honestly at all. `solids.py` says
      the walk "does not work with in place elements", and duHast keeps a
      separate `get_in_place_ceiling_family_instances` collector, so in-place
      ceilings are the first suspected cause rather than a surprise.
  Q2  Are ceilings and rooms in the SAME document?  -> the per-document counts
      in the index. This decides the single largest contract question: an
      element joining on a room ID is model-scoped EVERYWHERE per CLAUDE.md,
      and spaces are the one exception precisely because they are keyed and
      cross-document. If a project keeps ceilings in an interiors model and
      rooms in the architectural one, a model-scoped join matches nothing --
      and unlike spaces there is no key to widen to, only geometry. Answering
      this after the contract is written is how that gets discovered late.
  Q3  Do ceilings and rooms agree on LEVEL?  -> `level_name`, `level_id` and
      `height_offset_ft` per ceiling, against the same fields per room.
      duHast's own `process_ceilings_to_rooms` buckets both by `level.name`
      before intersecting, so a name that disagrees across documents silently
      produces zero matches. RoomMate joins storeys by name AND elevation for
      exactly this reason; the probe records both so the analyser can say which
      would have worked.
  Q4  How much does a ceiling overlap a room, and how many rooms does it hit?
      -> not measured here. The polygons are captured verbatim and the analyser
      intersects them, because that is the number the 0.1% threshold in
      `_intersect_ceiling_vs_room` is calibrated against, and a threshold
      chosen inside the collector could never be re-argued.
  Q5  Which phase does each ceiling belong to?  -> `phase_created` and
      `phase_demolished`, read through `PHASE_CREATED` / `PHASE_DEMOLISHED`.
      A ceiling is BUILT in one phase and demolished in a later one, so it
      takes the doors range test and NOT the rooms equality test on
      `ROOM_PHASE`. CLAUDE.md records that running rooms through the range test
      returns nothing, silently; this is the same trap facing the other way,
      and the probe records the parameters that decide it rather than assuming.
  Q6  What SHAPE is a ceiling polygon?  -> counted by the analyser over the raw
      export. `convert_solid_to_flattened_2d_points` returns one entry per
      solid volume, each an outer ring followed by its holes -- so a ceiling
      can be several disjoint polygons, which a room's `loops` (one outer plus
      holes) cannot express. Whether that is theoretical or routine decides the
      contract's geometry field.

WHY ROOMS ARE EXPORTED TOO
Q2, Q3 and Q4 all compare the two populations, and on a federated project they
may live in different documents. The run captures every document it resolves
and the analyser joins them, which is also the honest rehearsal of what the
server will do.

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
import os
import re
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

    The probe reuses the extractor's id helper rather than reimplementing it,
    for the reason the spaces probe gives: ids recorded here are compared
    against ids on the wire, so they have to be spelled by the code that will
    spell them in production.
    """
    here = globals().get("__file__")
    if not here:
        return
    root = os.path.dirname(os.path.dirname(os.path.abspath(here)))
    candidate = os.path.join(root, "extractor", "pyRevit")
    if os.path.isdir(candidate) and candidate not in sys.path:
        sys.path.insert(0, candidate)


# The two entities probed, stated as data. Unlike the spaces probe these are NOT
# the same Revit class -- a room is a SpatialElement and a ceiling is not -- so
# the per-entity facts include which accessor set applies.
ENTITY_SPECS = {
    "ceilings": {
        "category": "OST_Ceilings",
        "module": "duHast.Revit.Ceilings.Export.to_data_ceiling",
        "getter": "get_all_ceiling_data",
        "collector": (
            "duHast.Revit.Ceilings.ceilings",
            "get_all_ceiling_instances_in_model_by_category",
        ),
        "list_key": "ceiling",
        "kind": "ceiling",
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
# reading. Empty takes every loaded link. RHH ran the spaces probe with
# ["RHH-HDR-AR-MDL", "RHH-JHA-"]; ceilings may well need the interiors models
# too, which is Q2 restated as a setting.
DOCUMENT_PREFIXES = []

# Bumped whenever the probe's OUTPUT or its conversion changes. Printed at
# startup because this script is run by exec'ing its text, so nothing else tells
# a reader which copy actually ran.
PROBE_VERSION = 1


# --------------------------------------------------------------------------
# Small Revit accessors. Each one answers None rather than raising, because a
# single unreadable element must cost its own field and not the run: the probe
# exists to describe models nobody has looked at yet, so "this ceiling has no
# such parameter" is an expected outcome, not an error.
# --------------------------------------------------------------------------


def _element_id_str(element_id):
    """Deferred re-export of the extractor's accessor, so ids here and ids on
    the wire are spelled the same way."""
    from room_m.utils.generic import element_id_str

    return element_id_str(element_id)


def _phase_facts(doc, element):
    """The phase a ceiling was BUILT in and the one it was demolished in.

    Two parameters, not one, and that is the finding Q5 exists to confirm. A
    room BELONGS to a phase (`ROOM_PHASE`, an equality test); a ceiling is
    created in one and may be demolished in a later one, which is the doors
    range test. Recording both means the analyser can say whether the range
    test is required or merely harmless on this model.
    """
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
    """`ROOM_PHASE`, for a room. Kept separate from `_phase_facts` deliberately:
    fusing the two would hide that the two entities answer phase through
    different parameters, which is the whole of Q5."""
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
    """The level a ceiling or room states, by name, id and elevation.

    Read from the element rather than taken from the export so Q3 has an answer
    for an element the export DROPPED -- which, for ceilings, is the population
    Q1 is about. A room answers `.Level` directly and a ceiling answers
    `LevelId`, so both are tried because one probe serves both.
    """
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


def _height_offset(element):
    """`CEILING_HEIGHTABOVELEVEL_PARAM`, in Revit's internal feet.

    The parameter duHast's `get_level_data` uses for a ceiling, recorded raw.
    It matters twice: it is what places the ceiling plane above its level, and
    two ceilings stacked over one room (a bulkhead over a flat soffit) will
    differ here and nowhere else -- so it is the only field that could separate
    them if Q4 turns up a room with more ceilings than it should have.
    """
    from Autodesk.Revit.DB import BuiltInParameter

    try:
        parameter = element.get_Parameter(
            BuiltInParameter.CEILING_HEIGHTABOVELEVEL_PARAM
        )
        if parameter is None:
            return None
        return float(parameter.AsDouble())
    except Exception:
        return None


def _geometry_facts(element):
    """How many solids the element's geometry walk finds, and whether it is an
    in-place family.

    This is Q1's instrument, and it is deliberately the SAME walk
    `get_2d_points_from_solid` performs rather than a summary of the export:
    when the export drops a ceiling, this says whether it dropped it because
    there were no solids to flatten (a model or API problem) or because the
    flatten produced nothing from solids that existed (a duHast problem). Those
    two send a reader to opposite places, which is the distinction the spaces
    probe had to make between `Unenclosed` and `Unmeasured`.
    """
    from Autodesk.Revit.DB import Options, Solid

    facts = {"solid_count": None, "is_in_place": None, "class_name": None}
    try:
        facts["class_name"] = type(element).__name__
        # An in-place ceiling is a FamilyInstance sitting in OST_Ceilings, and
        # `solids.py` states plainly that its walk does not handle those. duHast
        # keeps `get_in_place_ceiling_family_instances` as a separate collector
        # for the same reason, so the class name IS the test.
        facts["is_in_place"] = facts["class_name"] == "FamilyInstance"
    except Exception:
        pass
    try:
        geometry = element.get_Geometry(Options())
        count = 0
        for item in geometry:
            if type(item) is Solid:
                count += 1
        facts["solid_count"] = count
    except Exception:
        facts["solid_count"] = None
    return facts


def _room_measurements(element):
    """`Area` and whether the room is placed, in Revit's own units.

    Carried over from the spaces probe unchanged in meaning: an unplaced room
    exports no polygon, and a ceiling that overlaps nothing because the ROOM is
    missing must not be counted against Q1.
    """
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
    """Every element of one category, collected by this script directly.

    Deliberately NOT through duHast's collector, for the reason the spaces
    probe gives: a collector that catches every exception and answers `[]`
    cannot distinguish "this model has no ceilings" from "the collector
    failed". Counting both ways is how the probe catches that in the act.
    """
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
    """How many elements duHast's own collector reports, and whether it raised.

    Returns `(count, error)`. An error here is a finding, not a failure of the
    run: it is the exception the collector would otherwise have swallowed.
    """
    module_name, getter_name = spec["collector"]
    try:
        module = __import__(module_name, globals(), locals(), [getter_name])
        collected = getattr(module, getter_name)(doc)
        return (len(list(collected)), None)
    except Exception as error:
        return (None, str(error))


def probe_element(doc, element, kind):
    """Every Revit fact about one element that its export does not carry.

    Property VALUES are not read here. The spaces probe learned that the hard
    way -- `AsValueString` returns display-formatted, unit-suffixed, rounded
    text, and the server stores what the EXPORT says -- so the analyser reads
    values from the raw export and this records only what the export omits.
    """
    facts = {"id": _element_id_str(element.Id)}
    facts.update(_level_facts(doc, element))
    if kind == "ceiling":
        facts.update(_geometry_facts(element))
        facts.update(_phase_facts(doc, element))
        facts["height_offset_ft"] = _height_offset(element)
    else:
        facts.update(_room_measurements(element))
        facts["room_phase"] = _room_phase(doc, element)
    return facts


def run_export(doc, spec):
    """duHast's own export for this entity, verbatim, wrapped exactly as the
    extractor wraps it -- so what the analyser reads is what a push would send."""
    from duHast.Data.Utils.data_to_file import build_json_for_file

    module = __import__(spec["module"], globals(), locals(), [spec["getter"]])
    data = getattr(module, spec["getter"])(doc)
    return build_json_for_file({spec["list_key"]: data}, "{}".format(doc.Title))


# --------------------------------------------------------------------------
# Serialisation
# --------------------------------------------------------------------------


def plainify(value, path="", problems=None):
    """Convert duHast's objects into JSON-safe values, recording what it could
    not convert rather than dropping it.

    A sibling of `probe_spaces_export.plainify`, duplicated rather than
    imported: these scripts are run by exec'ing their text into pyRevit, so an
    import between them would resolve to whatever happens to be on the path.
    """
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
    """Write `payload` as UTF-8 JSON with LF endings.

    Binary mode on purpose: `.gitattributes` enforces LF and a text-mode write
    on Windows would produce CRLF in a file destined for the repo -- the trap
    `CLAUDE.md` records under Traps."""
    text = json.dumps(payload, indent=2, ensure_ascii=False, sort_keys=True)

    directory = os.path.dirname(os.path.abspath(path))
    if directory and not os.path.isdir(directory):
        os.makedirs(directory)
    with io.open(path, "wb") as handle:
        handle.write(text.encode("utf-8"))
    return path


def slugify(title):
    """A filename-safe stem for a document title.

    Every output is named after its document, for the reason the spaces probe
    states: a fixed filename per entity has the last document silently
    overwrite the rest, and the analyser then reports a clean single-model run
    over the wreckage."""
    slug = re.sub(r"[^A-Za-z0-9._-]+", "-", title or "document").strip("-")
    return (slug or "document")[:80]


# --------------------------------------------------------------------------
# Per document
# --------------------------------------------------------------------------


def probe_document(doc, out_dir):
    """Export and probe one document for whichever entities it holds.

    A document is probed for BOTH entities regardless of what it is supposed to
    be. Q2 IS this question: nobody has yet established that a project's
    ceilings and its rooms share a file, and an interiors model holding
    ceilings against an architectural model holding rooms is exactly the
    surprise worth finding now rather than after the contract is written.
    """
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

        # An empty population is still written out. "This document holds no
        # ceilings" is a finding, and a probe that skipped the file would leave
        # the analyser unable to tell it from a document nobody selected.
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
            # Q1 lives in the gap between these three numbers. `collected_count`
            # is what the model holds, `duhast_collected_count` what duHast's
            # collector admits to, and `exported_count` what survived
            # `populate_data_ceiling_object` returning None.
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
    """The host document plus every loaded link.

    The correction the RHH spaces run forced: on a real federated project none
    of these documents are OPEN, they are links inside one host, so a picker
    over open documents finds exactly one file and every cross-document
    question answers nothing."""
    from duHast.Revit.Links.links import get_link_docs

    docs = [host]
    for _name, link_doc in get_link_docs(
        host, link_names_filter=[], inverse_filter=True
    ).items():
        docs.append(link_doc)
    return docs


def resolve_documents(prefixes=None):
    """The documents to probe: the host and its links, narrowed by title prefix.

    `prefixes` is how a federated host with dozens of links is narrowed to the
    ones worth reading. Empty or None takes every loaded link, which is right
    for a small project and ruinous on a large one -- so it is an argument, not
    a constant.
    """
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
        # A host with no links, or a duHast without `get_link_docs`, is still
        # worth probing -- it just answers the single-document subset.
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

    print("probe_ceilings_export v{}".format(PROBE_VERSION))
    index = {"probe_version": PROBE_VERSION, "documents": []}
    for doc in resolve_documents(DOCUMENT_PREFIXES):
        print("probing {} -> {}".format(doc.Title, out_dir))
        written, summary = probe_document(doc, out_dir)
        for path in written:
            print("  wrote {}".format(path))
        index["documents"].append(summary)

    # The index exists so the analyser reads a stated list rather than globbing
    # a directory: a fixtures folder accumulates runs, and an analyser that
    # picked up yesterday's interiors model beside today's would report an
    # overlap rate against a population nobody captured.
    print("  wrote {}".format(
        write_json(os.path.join(out_dir, "ceilings-probe-index.json"), index)
    ))
    print("done. Now run: python scripts/analyse_ceilings_probe.py")


sys.path += [r"C:\Users\janchristel\Documents\GitHub\SampleCodeRevitBatchProcessor-NET8\src", r"C:\Users\janchristel\Documents\GitHub\SampleCodeRevitBatchProcessor-NET8\Samples\pyRevit\Extensions\duHast-2025.extension\duHast.tab\lib"]

if __name__ == "__main__":
    main(r"C:\Users\janchristel\Documents\GitHub\room_mate\temp")
