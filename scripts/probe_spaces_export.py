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
Capture the raw duHast space and room exports, plus the Revit facts neither
export carries, so the questions gating `docs/PLAN-spaces.md` are answered off a
measurement instead of off a reading of duHast's source.

    pyRevit  ->  open the FEDERATED HOST and run this script; it walks the
                 host and every loaded link, narrowed by DOCUMENT_PREFIXES
    outputs  ->  scripts/fixtures/spaces-raw-<document>.json
                 scripts/fixtures/spaces-probe-<document>.json
                 scripts/fixtures/rooms-raw-<document>.json
                 scripts/fixtures/rooms-probe-<document>.json
                 scripts/fixtures/spaces-probe-index.json

Then, on any machine:

    python scripts/analyse_spaces_probe.py

**This script decides nothing.** It collects, and it counts; every judgement --
is this space unenclosed or merely unmeasured, does this key match, is that area
difference a finding or a boundary convention -- happens in the analyser,
offline, where it can be re-run and argued with.

THE QUESTIONS, and where each is answered
  Q1  Does duHast produce a usable outer loop for a space bounded by a LINKED
      model?  -> `exported_outer_points` against `area_internal_sqft` and
      `boundary_segment_count` on every record.
      **The kill condition.** Spaces are the first entity RoomMate extracts
      whose boundaries come from a link rather than from its own document, and
      `get_2d_points_from_revit_room` reaches for `_outer_loop_via_solid`
      exactly there. If that fallback does not carry the case, every space
      arrives geometry-less and PLAN-spaces D5 and D6 both collapse.
  Q2  How do the four enclosure states divide the population?  -> `location_is_none`,
      `area_internal_sqft`, `boundary_segment_count` and the export's own loop,
      recorded separately and never fused. D5 turns on the difference between
      `Unenclosed` (a model defect) and `Unmeasured` (a pipeline defect), and a
      probe that reported one number for both would settle D5 by accident.
  Q3  Does each document have spaces AT ALL, and can it say so honestly?
      -> `collected_count` (this script's own collector) against
      `duhast_collected_count` (duHast's). They should agree. They will not when
      `get_all_spaces` swallows an exception and answers `[]`, which is upstream
      item U1 and the reason requirement 1 cannot be trusted until it lands.
  Q4  Is the linking key unique, and does it match?  -> `key_value` on every
      space and every room, from the same property-resolution the extractor
      ships. The analyser does the matching; this file only records the values.
  Q5  What does the systematic area difference look like, and where does it
      cross a percentage threshold?  -> `area_internal_sqft` here, in Revit's
      own square feet, against the `Area` the EXPORT carries. The export's is
      what the server stores; v1 read the parameter instead and got a
      display-formatted, whole-number-rounded string.
  Q6  Which phase does each element belong to?  -> `phase_name` per element,
      read through `BuiltInParameter.ROOM_PHASE`, which is the parameter a space
      shares with a room. Confirms the equality filter, and makes an
      arch-versus-services phase-name difference visible before it surprises
      anyone.

WHY ROOMS ARE EXPORTED TOO, AND WHY THIS RUN IS MULTI-DOCUMENT
Unlike every previous probe, the two populations this one compares live in
DIFFERENT Revit documents -- rooms in the architectural model, spaces in each
services model, which bounds them against the architectural model as a link.
Q4 and Q5 are therefore cross-document questions and cannot be answered from a
single file. The run captures every document it resolves and the analyser
joins them, which is also the honest rehearsal of what the server will do.

On a real federated project none of these documents are OPEN -- they are all
links inside one host, which is why `resolve_documents` walks links rather
than offering a picker.

Every output is named after its document. The FFE and windows probes write
fixed filenames per entity, which is safe only because they probe one document
at a time -- run either of them over a multiselect and the last document
silently overwrites the rest. That is tolerable there and fatal here.

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

    The probe reuses the extractor's property helpers rather than
    reimplementing them. That is not tidiness: Q4 and Q5 measure values the
    server will later compare, so they have to be resolved by the code that
    will resolve them in production. A second implementation of "what is this
    room's Number" would measure the probe instead of the model.
    """
    here = globals().get("__file__")
    if not here:
        return
    root = os.path.dirname(os.path.dirname(os.path.abspath(here)))
    candidate = os.path.join(root, "extractor", "pyRevit")
    if os.path.isdir(candidate) and candidate not in sys.path:
        sys.path.insert(0, candidate)


# The two entities probed, stated as data. Both are SpatialElements, which is
# the finding PLAN-spaces D1 rests on: one `probe_element` serves both, and the
# only per-entity facts are the category, the duHast getter and the list key the
# export writes (duHast keys its file by `data_type`).
ENTITY_SPECS = {
    "spaces": {
        "category": "OST_MEPSpaces",
        "module": "duHast.Revit.Spaces.Export.to_data_space",
        "getter": "get_all_space_data",
        "collector": ("duHast.Revit.Spaces.spaces", "get_all_spaces"),
        "list_key": "space",
    },
    "rooms": {
        "category": "OST_Rooms",
        "module": "duHast.Revit.Rooms.Export.to_data_room",
        "getter": "get_all_room_data",
        "collector": ("duHast.Revit.Rooms.rooms", "get_all_rooms"),
        "list_key": "room",
    },
}

# The property whose value links a space to its room. `Number` is the default
# because it is what a services model normally copies from the architectural
# room, but it is an argument all the way down: PLAN-spaces D8 makes the key a
# per-project setting, so a probe that hard-coded one would be measuring a
# guess. Pass another through `main(key_property="...")`.
DEFAULT_KEY_PROPERTY = "Number"

# The properties compared on both sides, beyond the key. `Area` is here because
# Q5 exists; `Name` because it is the other property everyone reaches for, and
# a text comparison costs nothing to measure alongside a numeric one.
COMPARED_PROPERTIES = ["Name", "Area"]

# Title prefixes narrowing a federated host's links to the models worth
# reading. Empty takes every loaded link. RHH ran with
# ["RHH-HDR-AR-MDL", "RHH-JHA-"].
DOCUMENT_PREFIXES = []

# Bumped whenever the probe's OUTPUT or its conversion changes. Printed at
# startup because this script is run by exec'ing its text, so nothing else tells
# a reader which copy actually ran.
#
# v2, after the RHH run: documents are resolved by walking the host's LINKS
# rather than picking from open documents, and the per-element `property_*`
# reads are gone -- they were display-formatted and rounded, and the export
# carries the value the server actually stores. A v1 capture still analyses,
# because the analyser falls back to `key_value` when an export has no
# properties; its `property_*` fields are simply ignored.
PROBE_VERSION = 2


# --------------------------------------------------------------------------
# Small Revit accessors. Each one answers None rather than raising, because a
# single unreadable element must cost its own field and not the run: the probe
# exists to describe models nobody has looked at yet, so "this space has no such
# parameter" is an expected outcome, not an error.
# --------------------------------------------------------------------------


def _element_id_str(element_id):
    """Deferred re-export of the extractor's accessor, so ids here and ids on
    the wire are spelled the same way."""
    from room_m.utils.generic import element_id_str

    return element_id_str(element_id)


def _param_as_string(element, builtin_name):
    from Autodesk.Revit.DB import BuiltInParameter

    try:
        parameter = element.get_Parameter(getattr(BuiltInParameter, builtin_name))
    except Exception:
        return None
    if parameter is None:
        return None
    try:
        return parameter.AsString() or parameter.AsValueString()
    except Exception:
        return None


def _phase_name(doc, element):
    """The phase a SpatialElement BELONGS to, via `ROOM_PHASE`.

    A room belongs to one phase and a space does too -- the same parameter on
    both, which is the whole reason PLAN-spaces D10 can say the phase filter is
    the rooms one. Read here rather than taken from the export so Q6 is
    answerable even for an element the export dropped."""
    from Autodesk.Revit.DB import BuiltInParameter

    try:
        parameter = element.get_Parameter(BuiltInParameter.ROOM_PHASE)
        if parameter is None:
            return None
        phase = doc.GetElement(parameter.AsElementId())
        return phase.Name if phase is not None else None
    except Exception:
        return None


def _location_is_none(element):
    """Whether the element is UNPLACED.

    This is the one fact that separates an unplaced space from an unenclosed
    one, and it is not in the export: both arrive with an empty outer loop, so
    `translate_room` cannot tell them apart and drops both. PLAN-spaces D4 puts
    the classification here instead."""
    try:
        return element.Location is None
    except Exception:
        return None


def _area_internal(element):
    """`SpatialElement.Area`, in Revit's internal square feet.

    Recorded beside the `Area` PARAMETER rather than instead of it. The
    parameter is what `properties_to_map` stores and therefore what the server
    would compare; this one is a float that always parses, so Q5 still has a
    number when the parameter is formatted with a unit suffix."""
    try:
        return float(element.Area)
    except Exception:
        return None


def _boundary_facts(element):
    """How many boundary loops and segments Revit reports for this element.

    The measurement Q1 turns on. A space with segments and no exported loop is
    `Unmeasured` -- duHast saw boundaries and produced no polygon anyway -- while
    a space with no segments at all is genuinely `Unenclosed`. Fusing the two
    into "has no geometry" is exactly what PLAN-spaces C1 rejected.

    The default `SpatialElementBoundaryOptions` is used deliberately: it is what
    duHast's own geometry walk uses, so a disagreement here is a disagreement
    about the same question rather than about two boundary regimes.
    """
    from Autodesk.Revit.DB import SpatialElementBoundaryOptions

    try:
        loops = element.GetBoundarySegments(SpatialElementBoundaryOptions())
    except Exception:
        return {"boundary_loop_count": None, "boundary_segment_count": None}
    if loops is None:
        return {"boundary_loop_count": 0, "boundary_segment_count": 0}
    loop_count = 0
    segment_count = 0
    for loop in loops:
        loop_count += 1
        for _segment in loop:
            segment_count += 1
    return {"boundary_loop_count": loop_count, "boundary_segment_count": segment_count}


def _level_facts(element):
    try:
        level = element.Level
    except Exception:
        return {"level_name": None, "level_id": None}
    if level is None:
        return {"level_name": None, "level_id": None}
    try:
        return {"level_name": level.Name, "level_id": _element_id_str(level.Id)}
    except Exception:
        return {"level_name": None, "level_id": None}


# --------------------------------------------------------------------------
# Collection
# --------------------------------------------------------------------------


def collect_elements(doc, category_name):
    """Every element of one category, collected by this script directly.

    Deliberately NOT through duHast's `get_all_spaces`, and that is the whole
    of Q3: `get_all_spaces` catches every exception and returns `[]`, so it
    cannot distinguish "this model has no spaces" from "the collector failed" --
    in the one function requirement 1 rests on. Counting both ways is how the
    probe catches that in the act rather than inheriting it.
    """
    from Autodesk.Revit.DB import BuiltInCategory, FilteredElementCollector

    category = getattr(BuiltInCategory, category_name, None)
    if category is None:
        return None
    return list(FilteredElementCollector(doc).OfCategory(category).ToElements())


def duhast_collected_count(doc, spec):
    """How many elements duHast's own collector reports, and whether it raised.

    Returns `(count, error)`. An error here is a finding, not a failure of the
    run: it is the exception `get_all_spaces` would have swallowed.
    """
    module_name, getter_name = spec["collector"]
    try:
        module = __import__(module_name, globals(), locals(), [getter_name])
        collected = getattr(module, getter_name)(doc)
        return (len(list(collected)), None)
    except Exception as error:
        return (None, str(error))


def probe_element(doc, element, key_property, compared_properties):
    """Every Revit fact about one SpatialElement that its export does not carry.

    **Compared property values are NOT read here, and v1 was wrong to.** It read
    them with `LookupParameter(...).AsValueString()`, which on RHH returned
    `"71 m2"` for a space duHast exports as `71.27892877719862`: display
    formatted, unit suffixed, and rounded to the whole number. The server stores
    the export's value, so calibrating anything on the probe's would set a
    threshold from the rounding. The analyser reads them from the export.

    `key_value` stays, because the key is often a builtin (`ROOM_NUMBER`) that
    reads back exactly, and because an element the export dropped still needs
    one. The analyser prefers the export and falls back to this.
    """
    facts = {
        "id": _element_id_str(element.Id),
        "location_is_none": _location_is_none(element),
        "area_internal_sqft": _area_internal(element),
        "phase_name": _phase_name(doc, element),
        "key_value": _param_as_string(element, "ROOM_NUMBER")
        if key_property == "Number"
        else _named_param(element, key_property),
        "key_property": key_property,
    }
    facts.update(_boundary_facts(element))
    facts.update(_level_facts(element))
    return facts


def _named_param(element, name):
    """One parameter by its user-facing name, as a string.

    By name rather than by `BuiltInParameter` because the key is user config
    (D8): a project may link on a shared parameter that has no builtin at all.
    `LookupParameter` is the accessor that spans both."""
    try:
        parameter = element.LookupParameter(name)
    except Exception:
        return None
    if parameter is None:
        return None
    try:
        return parameter.AsString() or parameter.AsValueString()
    except Exception:
        return None


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

    A trimmed sibling of `probe_ffe_export.plainify`, duplicated rather than
    imported: these two scripts are run by exec'ing their text into pyRevit, so
    an import between them would resolve to whatever happens to be on the path.
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

    Every output is named after its document because this is the first
    multi-document probe: rooms and spaces live in different files, so a fixed
    filename per entity would have the last services model overwrite the rest
    and the analyser would report a clean single-model run over the wreckage."""
    slug = re.sub(r"[^A-Za-z0-9._-]+", "-", title or "document").strip("-")
    return (slug or "document")[:80]


# --------------------------------------------------------------------------
# Per document
# --------------------------------------------------------------------------


def probe_document(doc, out_dir, key_property, compared_properties):
    """Export and probe one document for whichever entities it holds.

    A document is probed for BOTH entities regardless of what it is supposed to
    be. A services model that turns out to hold rooms, or an architectural model
    that holds spaces, is exactly the surprise worth finding now rather than
    during PR E -- and the cost of asking is one collector pass.
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
        # spaces" is the finding requirement 1 is built to report, and a probe
        # that skipped the file would leave the analyser unable to tell it from
        # a document nobody selected -- the same distinction PLAN-spaces D7
        # draws between "no spaces" and "not audited".
        raw = run_export(doc, spec) if elements else {spec["list_key"]: []}
        raw_plain, problems = plainify(raw, entity)
        if problems:
            print(
                "  WARNING: {} field(s) would not convert; see "
                "serialisation_problems in the probe file".format(len(problems))
            )

        records = [
            probe_element(doc, element, key_property, compared_properties)
            for element in elements
        ]

        probe = {
            "probe_version": PROBE_VERSION,
            "entity": entity,
            "list_key": spec["list_key"],
            "document": doc.Title,
            "path": getattr(doc, "PathName", None),
            "key_property": key_property,
            "compared_properties": list(compared_properties),
            "serialisation_problems": problems,
            # Q3, and the reason there are two counts. They should agree; when
            # duHast's is 0 or None and this one is not, U1 has been caught.
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

    **The RHH run's first correction.** The draft picked from *open* documents,
    and on a real federated project none of these are open: the architectural
    models and the services models are all links inside one host. A picker over
    open documents finds exactly one document and the cross-document half of the
    probe answers nothing."""
    from duHast.Revit.Links.links import get_link_docs

    docs = [host]
    for _name, link_doc in get_link_docs(
        host, link_names_filter=[], inverse_filter=True
    ).items():
        docs.append(link_doc)
    return docs


def resolve_documents(prefixes=None):
    """The documents to probe: the host and its links, narrowed by title prefix.

    Multi-document is not a convenience here, it is the shape of the question.
    Q4 and Q5 compare spaces in a services model against rooms in an
    architectural one, so a single-document run can answer Q1, Q2, Q3 and Q6 and
    nothing else -- which the analyser says out loud rather than reporting a 0%
    key match.

    `prefixes` is how a federated host with dozens of links is narrowed to the
    ones worth reading (`["RHH-HDR-AR-MDL", "RHH-JHA-"]` on the RHH run). Empty
    or None takes every loaded link, which is right for a small project and
    ruinous on a large one -- so it is an argument, not a constant.
    """
    try:
        host = __revit__.ActiveUIDocument.Document  # noqa: F821 - pyRevit global
    except Exception:
        raise RuntimeError(
            "no Revit document: run this from pyRevit with the federated host "
            "open, or call probe_document(doc, out_dir, 'Number') yourself"
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


def main(out_dir=None, key_property=DEFAULT_KEY_PROPERTY, compared_properties=None):
    out_dir = out_dir or default_out_dir()
    if not out_dir:
        raise RuntimeError(
            "cannot resolve an output directory (no __file__): pass one, e.g. "
            "main(out_dir=r'C:\\\\temp\\\\probe')"
        )
    compared_properties = list(compared_properties or COMPARED_PROPERTIES)

    print(
        "probe_spaces_export v{} (key property: {})".format(PROBE_VERSION, key_property)
    )
    index = {
        "probe_version": PROBE_VERSION,
        "key_property": key_property,
        "compared_properties": compared_properties,
        "documents": [],
    }
    for doc in resolve_documents(DOCUMENT_PREFIXES):
        print("probing {} -> {}".format(doc.Title, out_dir))
        written, summary = probe_document(doc, out_dir, key_property, compared_properties)
        for path in written:
            print("  wrote {}".format(path))
        index["documents"].append(summary)

    # The index exists so the analyser reads a stated list rather than globbing
    # a directory: a fixtures folder accumulates runs, and an analyser that
    # picked up yesterday's services model beside today's would report a key
    # match against a population nobody captured.
    print("  wrote {}".format(
        write_json(os.path.join(out_dir, "spaces-probe-index.json"), index)
    ))
    print("done. Now run: python scripts/analyse_spaces_probe.py")


sys.path += [r"C:\Users\janchristel\Documents\GitHub\SampleCodeRevitBatchProcessor-NET8\src", r"C:\Users\janchristel\Documents\GitHub\SampleCodeRevitBatchProcessor-NET8\Samples\pyRevit\Extensions\duHast-2025.extension\duHast.tab\lib"]

if __name__ == "__main__":
    main(r"C:\Users\janchristel\Documents\GitHub\room_mate\temp")
