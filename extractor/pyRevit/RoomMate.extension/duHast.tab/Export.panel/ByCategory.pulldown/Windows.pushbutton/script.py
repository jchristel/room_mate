"""
Exports WINDOWS ONLY from the selected model(s) and pushes them to RoomMate.

A window in a facade package usually names no room at all, and is attributed by
geometry or not at all -- an unattributed window is a reported state, not a
failed export.

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

# import the windows-only export entry point from the library
from room_m.room_mate import windows_export_entry

# push windows, and only windows
result = windows_export_entry(doc, uiapp, output, forms)
