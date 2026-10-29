"""
Make room layout sheets from the RoomMate tasks queued for this model.

Usage:

- Run this script and pick the model and the RoomMate project.
- If the model's RLS settings are not complete, the settings page opens first (see the
  RLS settings button); the script stops if they are still incomplete afterwards.
- The queued room layout sheet tasks of this model are fetched and checked. Tasks that cannot
  run are reported to the server with the reason; warnings are put to you once.
- Each remaining task makes one sheet with its plan view, elevations, schedule and tags.
"""

from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

doc = revit.doc
uiapp = __revit__

from room_m.room_mate import rls_create_entry

rls_create_entry(doc, uiapp, output, forms)
