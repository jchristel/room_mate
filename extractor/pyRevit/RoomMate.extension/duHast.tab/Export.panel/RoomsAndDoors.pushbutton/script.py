"""
Exports ROOMS AND DOORS from the selected model(s) and pushes both to RoomMate.

The entry point is still called rooms_export_entry, which reads as rooms only.
It is not: a click here pushes both entities, and that is what the button name
says. Use the By Category pulldown to push one of them on its own.

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

# import the combined rooms-and-doors export entry point from the library
from room_m.room_mate import rooms_export_entry

rooms_export_entry(doc=doc, uiapp=uiapp, output=output, forms=forms)
