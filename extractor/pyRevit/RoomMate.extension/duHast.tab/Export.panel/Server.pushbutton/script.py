"""
Choose the RoomMate server the other buttons talk to.

Usage:

- Run this script and type the server's address, for example 192.168.0.97 or
  mac.local:5151 (the port is 5151 unless you typed another).
- The address is checked before it is kept. It is kept in this model (so whoever opens the
  model needs no set-up) and on this computer (for a model that does not have it yet).
- Type 'local' for the server on this computer, 'reset' to forget the kept address.
- A computer that has never been given an address is sent here by the other buttons.

The server has to have been started so that other computers can reach it
(its --bind option); it prints the address when it starts.
"""

from pyrevit import revit, script, forms

logger = script.get_logger()
output = script.get_output()

doc = revit.doc
uiapp = __revit__

from room_m.room_mate import server_entry

server_entry(doc, uiapp, output, forms)
