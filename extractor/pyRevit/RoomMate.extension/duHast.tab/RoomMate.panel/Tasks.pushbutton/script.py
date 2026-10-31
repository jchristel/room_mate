"""
Run the room tasks RoomMate holds for this model: place the items, make the room layout sheets.

Usage:

- Run this script and pick the model and the RoomMate project.
- Every queued task of the model is fetched at once and handed to the code for its kind:
  populate and update tasks place the items first, then room layout sheet tasks make the sheets.
- Each kind checks its whole list before anything is written. Tasks that cannot run are reported
  to the server with the reason; warnings are put to you once.
- If the model's RLS settings are not complete, the settings window opens before the first sheet
  (see the RLS settings button).
- Every step is written to RoomMate.log, in the RoomMate folder of your application data; if a run stops
  without a word, the last line of that file says where.
"""

from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

doc = revit.doc
uiapp = __revit__

from room_m.room_mate import tasks_entry

tasks_entry(doc, uiapp, output, forms)
