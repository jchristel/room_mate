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


# The Revit work (phase ordering and membership, levels) is duHast's; what is here is the shape
# the exporters and the windows want it in.
from duHast.Revit.Common.element_id import get_el_id_int
from duHast.Revit.Common.phases import (
    get_all_phases_in_order,
    get_ids_of_elements_in_phase,
    get_phase_by_name,
)
from duHast.Revit.Levels.levels import get_levels_in_model


def element_id_str(element_id):
    """An ElementId as the string the wire uses (duHast knows which API name this Revit has)."""
    return str(get_el_id_int(element_id))


def document_phases(doc):
    """This document's phases as an ORDERED list of `{"id", "name"}`.

    The order is the entire point: "does this element exist in phase X" is a
    range test over the phase *sequence*, not an equality check, and `doc.Phases`
    is the only place that sequence exists. Ids are per-document (the same phase
    name has a different id in every model), so the name is what crosses between
    documents and the id is only ever used within one."""
    return [{"id": element_id_str(phase_id), "name": name} for phase_id, name in get_all_phases_in_order(doc)]


def document_levels(doc):
    """This document's levels as a list of `{"id", "name", "elevation"}`, lowest first. `id` is the
    per-document Revit id as text, which is what `level_id` on every translated element is, and what a
    level-scoped push names (`utils.phase_filter.in_scope`); `elevation` is in feet."""
    levels = []
    for level in get_levels_in_model(doc):
        try:
            levels.append({"id": element_id_str(level.Id), "name": level.Name, "elevation": level.Elevation})
        except Exception:
            continue
    return sorted(levels, key=lambda x: (x["elevation"], x["name"]))


def common_phase_names(docs):
    """The phase names every one of `docs` has, in the first document's order: the only names a run can
    be scoped to (see `utils.ui.choose_phase`)."""
    per_doc = [[p["name"] for p in document_phases(d)] for d in docs]
    if not per_doc or not per_doc[0]:
        return []
    return [name for name in per_doc[0] if all(name in names for names in per_doc[1:])]


def phase_by_name(doc, phase_name):
    """This document's `Phase` object for `phase_name`.

    Needed because `FamilyInstance.FromRoom` is indexed by a *Phase*, not by a
    name or an id - and the name is what crosses between documents, so the
    lookup has to happen per document. Raises for an unknown name, on the same
    fail-loudly terms as `elements_in_phase`."""
    phase = get_phase_by_name(doc, phase_name)
    if phase is None:
        raise ValueError(
            "model has no phase named '{}' (it has: {})".format(
                phase_name, ", ".join(p.Name for p in doc.Phases))
        )
    return phase


def elements_in_phase(doc, phase_name, category):
    """The element ids of `doc`'s elements of `category` that exist in
    `phase_name`, as strings matching the ids the export carries.

    The filter runs here, client-side, because only the live document has the
    phase ordering the range test needs - the server never re-evaluates the
    predicate, which is why the ordered phase list is not on the wire at all.
    It also means strictly *less* extraction, the axis that actually pays.

    **Not rooms.** A room does not span a range of phases, it belongs to one,
    and running rooms through the range test returned nothing at all (see
    `room_m.utils.rooms.rooms_in_phase`). Every other built-then-demolished
    element uses this.

    Raises when the document has no phase of that name: a model that cannot be
    scoped to the chosen phase must fail loudly rather than push everything."""
    return set(str(i) for i in get_ids_of_elements_in_phase(doc, phase_name, category))


# Entity keys whose label is not "the key minus a trailing s": "ffe" is an
# acronym, not a plural, and its by-space sibling compounds that. Every other
# key in `ENTITY_EXPORTERS` is a regular plural and needs no entry here.
_IRREGULAR_ENTITY_LABELS = {
    "ffe": "FF&E",
    "ffe_by_space": "FF&E (by space)",
}


def entities_label(entities):
    """The chosen entities as the noun phrase in "<...> data" -- singular, since
    most names are regular plurals and "rooms data" reads as a typo. An entity
    in `_IRREGULAR_ENTITY_LABELS` is looked up rather than singularised, for the
    ones that are not regular plurals at all."""
    return " and ".join(_IRREGULAR_ENTITY_LABELS.get(entity, entity[:-1]) for entity in entities)
