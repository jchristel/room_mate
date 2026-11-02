"""
Push Revit data to the RoomMate server from one window.

Usage:

- Run this script and pick the model(s).
- Tick what to export, choose the project and the phase.
- Push the whole model, or only the levels you tick: the server then replaces those levels and
  keeps the rest of the model as it holds it.
"""

from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

doc = revit.doc
uiapp = __revit__

from room_m.room_mate import export_all_entry

export_all_entry(doc, uiapp, output, forms)
