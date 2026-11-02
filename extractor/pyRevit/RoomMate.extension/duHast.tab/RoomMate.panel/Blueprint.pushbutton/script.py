"""
The blueprint push: a phase, then the rooms, then the sheets that belong to each room.

Usage:

- Run this script and pick the model, the project and the phase.
- Pick the rooms you want a blueprint from.
- For each room, pick its room layout sheets (at least one) and name the blueprint.

A blueprint is a room and its sheets together, so a room with no sheet is not pushed. The room's
contents are captured on the server from the rooms and FF&E already pushed, so push those first
with the Rooms and FFE buttons. A name the project already has becomes a pending revision, to
accept or reject in the blueprint viewer. To push sheets alone, to the library, use Sheets.
"""

from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

doc = revit.doc
uiapp = __revit__

from room_m.room_mate import blueprint_export_entry

blueprint_export_entry(doc, uiapp, output, forms)
