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
Which ROOM does an element point at, in a given phase.

Its own module rather than part of `room_m.utils.rooms`, because it answers a
different question. `rooms.py` extracts rooms as *entities* -- which rooms exist,
and in which phase. This answers, for some other element, which room it is
associated with. Every entity that hangs off a room needs the second question and
none of them need the first, so keeping them apart is what stops `doors.py`
(and, later, `windows.py` and `ffe.py`) from importing the room extractor to
reach one helper.

The phase-indexed properties are reached by name off whatever instance the caller
passes (duHast does the reading), which is also what makes one function serve every category.
"""

from duHast.Revit.Family.family_instance_facts import get_room_or_space_id


def room_reference(instance, phase, which):
    """One of `instance`'s room references for `phase`, as an id string, or None (duHast
    `get_room_or_space_id`).

    `which` is the property name -- `"FromRoom"` or `"ToRoom"` for the wall-hosted
    categories (doors, windows), `"Room"` or `"Space"` for an instance that sits *in* one
    room or space rather than between two (FF&E). An element with no room on that side is a
    normal state (an external door), so None here is data, not a failure."""
    room_id = get_room_or_space_id(instance, phase, which)
    return None if room_id is None else str(room_id)
