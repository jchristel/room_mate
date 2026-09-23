"""
An extension to export ffe data to room mate, identified against Spaces
rather than Rooms.

Usage:

- Run this script in a services model that holds Spaces and no Room
  elements -- the FFE button resolves nothing there, because it reads
  FamilyInstance.get_Room(phase) and this model has none.
"""

# pyrevit stuff
from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

# get the revit document
doc = revit.doc
uiapp = __revit__

# import ffe exporter
from room_m.room_mate import ffe_by_space_export_entry

# exports ffe data, identified by space, from the selected models
ffe_by_space_export_entry(doc, uiapp, output, forms)
