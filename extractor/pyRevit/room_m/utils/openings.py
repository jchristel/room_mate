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
The extraction side of an OPENING -- a door, a window, and whatever wall-hosted
category comes next.

One module rather than one per category, because measurement said so: a
full-depth diff of real duHast exports from two documents found a window record
and a door record structurally identical, and the four facts read here are read
the same way for both. `category` is the only argument that varies.

The examples in the comments below cite DOORS, deliberately. They are the cases
that decided each rule, and replacing a measured door figure with a vaguer
statement about openings would trade evidence for symmetry. Where windows differ
in degree rather than in kind it is said at the function concerned.
"""

from duHast.Revit.Family.family_instance_facts import (
    get_nested_instance_ids,
    get_opening_placements,
)

from room_m.utils.generic import (
    phase_by_name,
    elements_in_phase,
)


def _as_text(value):
    return None if value is None else str(value)


def opening_placements(doc, phase_name, category):
    """`{opening id: {"from_room", "to_room", "insertion_point", "normal"}}` for
    every instance of `category` in `doc`, read from the Revit API for the chosen
    phase (duHast `get_opening_placements`), with every id as the string the wire uses.

    `category` is a `BuiltInCategory` -- `OST_Doors`, `OST_Windows`.

    **Read from Revit rather than taken from the duHast export, deliberately.** The
    export carries `from_room`/`to_room` as arrays with one entry per phase,
    tagged with a `phase_id` that appears nowhere else in the file and cannot
    be resolved against anything on the wire. `FromRoom[phase]` takes the phase and
    answers exactly one room, which is also what makes the reference one-to-one, as
    the contract's `Option<String>` claims.

    **One pass, four facts**, so they describe the same door in the same phase. An
    opening whose room lookup raises is recorded with neither reference rather than
    aborting the model; the server's QA reports it as an opening with no room reference."""
    phase = phase_by_name(doc, phase_name)
    placements = {}
    for opening_id, facts in get_opening_placements(doc, phase, category).items():
        placements[str(opening_id)] = {
            "from_room": _as_text(facts["from_room"]),
            "to_room": _as_text(facts["to_room"]),
            "insertion_point": facts["insertion_point"],
            "normal": facts["normal"],
        }
    return placements


def openings_in_phase(doc, phase_name, category):
    """The ids of `category`'s instances in `phase_name`.

    The RANGE test, not the equality test rooms use. A door or a window is built
    in one phase and may be demolished in a later one, so it exists across a
    span; a room belongs to exactly one. Running an opening through the room
    predicate returns nothing, silently -- the failure that cost five empty
    pushes to find."""
    return elements_in_phase(doc, phase_name, category)


def nested_opening_ids(doc, category):
    """The ids of instances that are a *component of another instance of the same
    category* (a door leaf modelled as a nested shared family, its hardware, a
    window's glazing), and so are not openings at all (duHast `get_nested_instance_ids`).

    Measured on a facade file: 113 of 205 collected doors were components (55%),
    against 6 of 158 windows; on another job 2236 of 4134 exported "doors" were, and
    not one carried a room reference, which made a data artifact read as a modelling
    gap. The test is the parent's category, not its family or name.

    Returned as its own set rather than subtracted from `openings_in_phase`'s,
    deliberately: an empty push has to be able to say which of the two emptied it."""
    return set(str(i) for i in get_nested_instance_ids(doc, category))
