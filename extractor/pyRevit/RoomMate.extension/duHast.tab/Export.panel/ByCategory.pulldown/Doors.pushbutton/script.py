"""
Exports DOORS ONLY from the selected model(s) and pushes them to RoomMate.

Doors may be pushed before their rooms. A door names its rooms by id, and an id
is unique only within its own model, so a reference the server cannot resolve
yet is reported as pending rather than refused -- it resolves the moment that
model's rooms land.

Usage:

- Run this script and pick the model(s), the target project, and the phase.
"""

# pyrevit stuff
from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

# get the revit document
doc = revit.doc
uiapp = __revit__

# import the doors-only export entry point from the library
from room_m.room_mate import doors_export_entry

# push doors, and only doors
doors_export_entry(doc, uiapp, output, forms)
