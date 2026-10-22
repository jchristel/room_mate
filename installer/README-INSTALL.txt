RoomMate
========

RoomMate turns Revit room and door data into a browser floor-plan viewer.


Starting it
-----------

Use the RoomMate shortcut in the Start Menu. It starts the server and opens
the viewer at http://127.0.0.1:5151 once the server is actually up.

Closing the black console window stops the server. Launching the shortcut
again while it is already running just opens the viewer.

The server listens on the loopback address only -- it is reachable from this
machine and from nowhere else on the network. That is deliberate: there is no
authentication of any kind.


Where things are
----------------

Program files   %LOCALAPPDATA%\Programs\RoomMate
Your data       %LOCALAPPDATA%\RoomMate
pyRevit toolbar %APPDATA%\pyRevit\Extensions\RoomMate.extension

Under your data folder:

  settings\server.toml        storage location, edit if you want it elsewhere
  settings\projects\*.toml    one file per project: classification, sources
  data\snapshots\             every pushed model

Upgrading never overwrites anything in the data folder, and uninstalling never
deletes it. If you want a clean slate, delete %LOCALAPPDATA%\RoomMate -- the
next start recreates the settings from a fresh template.

One sample project is seeded so the settings page has something to open. Edit
it, rename it, or add your own -- a project is just a file in this folder, and
the settings page writes them for you.


Getting data in
---------------

The store starts empty, so the viewer has nothing to draw until a model is
pushed to it. Pushes come from the pyRevit extension that runs inside Revit;
it posts to http://127.0.0.1:5151 unless told otherwise, so the server has to be
running when you push.

Revit on a different computer
-----------------------------

To push from Revit on another computer on the same (trusted) network, start the
server so that it listens on the network. The server has NO authentication:
anyone who can reach its port can read and change the data, so use this only on
a closed network.

    RoomMate.ps1 -Bind 0.0.0.0           (or: roommate.exe ... --bind 0.0.0.0)

The server prints the addresses it can be reached at, for example
http://192.168.0.97:5151. Give that address to the pyRevit side. Allow incoming
connections to the server in the firewall of the computer it runs on. Use a
fixed address (or the computer's .local name) so it does not change. A DNS name
other than the computer's own must be listed with --allow-host.

If you installed the pyRevit toolbar component, the buttons are on the duHast
tab, in the Export panel: "Rooms + Doors" pushes both, and the By Category
pulldown pushes one entity at a time. Restart Revit after installing to see
them. Revit 2025 or newer, and pyRevit has to be installed already.

The toolbar carries its own copy of duHast, so it does not matter whether you
have duHast installed separately. Every export starts by printing which
RoomMate and which duHast it is running, and where that duHast was loaded
from. If an export ever looks wrong, that is the first line to read -- and the
first thing to quote when reporting it.

Uninstalling removes the toolbar. Your settings and pushed data stay.


The MCP server
--------------

mcp.exe in the program folder exposes RoomMate's read side to an MCP host such
as Claude Desktop. It is not started by the shortcut -- the host launches it.
Point the host at:

  Command:  %LOCALAPPDATA%\Programs\RoomMate\mcp.exe
  Args:     --server-settings %LOCALAPPDATA%\RoomMate\settings\server.toml
            --project-settings %LOCALAPPDATA%\RoomMate\settings\projects

Use full expanded paths -- most hosts do not expand environment variables.
The read-only tools work with no server running. Add
--server-url http://127.0.0.1:5151 for the one tool that uploads reference
data, which forwards to the running server.
