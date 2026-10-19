"""
The blueprint export: a phase, then the rooms, then the sheets that belong to each room.

Usage:

- Run this script and pick the model, the project and the phase.
- Pick the rooms you want a blueprint's sheets for.
- For each room, pick its room layout sheets.

It pushes sheets only. The blueprint is captured on the server from the rooms and FF&E
already pushed, so push those first with the Rooms and FFE buttons.
"""

from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

doc = revit.doc
uiapp = __revit__

from room_m.room_mate import blueprint_export_entry

blueprint_export_entry(doc, uiapp, output, forms)
