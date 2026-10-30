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
The extraction side of an FF&E ITEM -- what Revit knows that the duHast export
does not carry.

**Deliberately shorter than `room_m.utils.openings`, and the difference is the
point.** An opening pass reads four facts from Revit -- both room references,
the insertion point and the facing -- because the export's own room references
are unusable and its placement had to agree with them. An item pass reads two,
and only one of them is really from Revit:

- the **room**, for the run's single phase, because the export's `rooms` field
  is a list unioned across every phase with nothing saying which entry belongs
  to which, and its third fallback is a geometric lookup computed producer-side
  where the server can no longer report it as derived;
- the **category**, because `DataItem` has no field for it at all -- despite
  `get_all_item_data` walking nine categories and discarding which one each
  instance came from.

Everything else an item needs is in the export and is read from there:
position, rotation, level, type, both property tiers, and (once upstream change
U1 ships) the footprint. That is the standing rule from the door work -- an
extractor that re-measures what the export already carries silently discards
whatever the producer sent, which is exactly how a correct duHast fix produced a
byte-identical bad export.

**There is no nested-component filter here, and its absence is a decision.**
`room_m.utils.openings.nested_opening_ids` drops a door leaf at the producer and
is right to: "is this door leaf a door" has one answer, always. "Is this
component an item" does not -- a joinery handle is not, a chair nested in a
workstation group might be -- so it is a project convention, and it lives in the
server's `[ffe] nested_components`, applied at read time where it can be changed
after the fact and where the count of what it removed is reported. See
`docs/Superseded/PLAN-ffe.md` D10.

The measurement behind that: on House A, 179 of 647 instances had a
super-component, and the same-category test doors use would have caught 10 of
them -- an item's parent is usually in a DIFFERENT category (87 furniture
components of casework, 70 generic models inside electrical fixtures). Nor does
the doors discriminator transfer: 97.8% of those components named a room, where
a nested door component named none.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing
one, not even inside a docstring.
"""

from room_m.utils.generic import (
    elements_in_phase,
    phase_by_name,
)

from duHast.Revit.Family.family_instance_facts import get_item_facts


def items_in_phase(doc, phase_name, categories):
    """The ids of every instance of `categories` that exists in `phase_name`.

    The RANGE test, not the equality test rooms use -- an item is placed in one
    phase and may be demolished in a later one, so it exists across a span, the
    same shape a door or a window has. Running items through the room predicate
    would return nothing, silently, which is the failure that cost five empty
    pushes to find and is written down here for the fourth time rather than
    assumed to be common knowledge.

    Unioned across nine categories where an opening filters one, which is the
    only structural difference. `elements_in_phase` raises on an unknown phase
    name, and that guard is the one thing standing between a typo and another
    empty snapshot -- so it is called per category rather than guarded once and
    bypassed."""
    allowed = set()
    for category in categories:
        allowed |= elements_in_phase(doc, phase_name, category)
    return allowed


def item_facts(doc, phase_name, categories, identify_by="room"):
    """`{item id: {"room", "owner_spaces", "category", "family_name", "host"}}` for every instance of
    `categories` (duHast `get_item_facts`), with every id as the string the wire uses.

    **ONE spatial reference, not two.** `identify_by` picks which: `"room"` (the default)
    reads `get_Room(phase)`, for a document that holds the rooms FF&E serves; `"space"`
    reads `get_Space(phase)`, for a services model that holds Spaces and no Room elements
    at all. Never both per instance: this walk is already the slow half of a push (House A
    alone is 647 Revit calls). `owner_spaces` is a list on the wire even though this only
    ever populates zero or one, following `owner_rooms`'s shape.

    The category is `instance.Category.Name` ("Furniture", not "OST_Furniture"): what a
    `?category=` filter is written against. Family name and host are read from Revit because
    the export carries neither (probed on 16,161 RHH items); absent means unread."""
    if identify_by not in ("room", "space"):
        raise ValueError("identify_by must be 'room' or 'space', got {!r}".format(identify_by))
    phase = phase_by_name(doc, phase_name)
    facts = {}
    for item_id, found in get_item_facts(doc, phase, categories, identify_by).items():
        reference = None if found["reference"] is None else str(found["reference"])
        host = found["host"]
        if host is not None:
            host = {"kind": host["kind"], "id": str(host["id"]), "face_hosted": host["face_hosted"]}
        facts[str(item_id)] = {
            "room": reference if identify_by == "room" else None,
            "owner_spaces": [reference] if identify_by == "space" and reference else [],
            "category": found["category"],
            "family_name": found["family_name"],
            "host": host,
        }
    return facts
