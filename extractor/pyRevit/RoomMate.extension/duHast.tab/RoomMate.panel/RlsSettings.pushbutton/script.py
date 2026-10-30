"""
The RLS settings of this model in a Revit window .

Usage:

- Run this script and pick the model.
- Choose the title blocks, view types, schedule, tags and naming, then press Save.
- The settings are stored in the model, where the RLS create button reads them.
"""

from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

doc = revit.doc
uiapp = __revit__

from room_m.room_mate import rls_settings_entry

rls_settings_entry(doc, uiapp, output, forms)
