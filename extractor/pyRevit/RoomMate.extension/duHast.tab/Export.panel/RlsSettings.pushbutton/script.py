"""
Set how room layout sheets are made in this model.

Usage:

- Run this script and pick the model and the RoomMate project.
- A page opens in your browser with the settings the model holds now and the choices this model
  offers (title block families, view types, tags ...). Change them and press Save there.
- Come back to Revit and press Yes: the saved settings are stored in the model itself.

Needs the RoomMate server (set it with the Server button first).
"""

from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

doc = revit.doc
uiapp = __revit__

from room_m.room_mate import rls_settings_entry

rls_settings_entry(doc, uiapp, output, forms)
