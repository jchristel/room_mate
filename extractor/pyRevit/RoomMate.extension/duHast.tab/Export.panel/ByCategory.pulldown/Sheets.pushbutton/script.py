"""
Exports the sheets you choose to RoomMate's sheet library.

Usage:

- Run this script, pick the model and the project, then pick the sheets.
- The room each sheet was made for is set afterwards, in the viewer. To name rooms at
  the same time, use the Blueprint button.
- A sheet is the same sheet if its Revit sheet id and the model's title match. If its
  sheet number changed, the push is held until you accept or reject it in RoomMate.
"""

from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

doc = revit.doc
uiapp = __revit__

from room_m.room_mate import sheets_export_entry

sheets_export_entry(doc, uiapp, output, forms)
