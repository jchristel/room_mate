"""
Apply the room tasks queued on the RoomMate server for this model.

Usage:

- Make the tasks in RoomMate (the Tasks page), then run this script in the model they are for.
- Pick the project. The queued tasks for this model are claimed.
- Every task is checked before anything is written; tasks that cannot run are reported and
  skipped. If some only have warnings you are asked once whether to proceed with them.
- Each task is written in one transaction group, so one that fails leaves the model unchanged.
"""

from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

doc = revit.doc
uiapp = __revit__

from room_m.room_mate import apply_tasks_entry

apply_tasks_entry(doc, uiapp, output, forms)
