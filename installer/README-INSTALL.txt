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
http://192.168.0.97:5151. On the Revit computer, click the Server button (duHast tab, RoomMate panel), type
that address and confirm; it is checked before it is kept, and kept for later runs.
Type default to go back to the server on the same computer. (The ROOMMATE_SERVER
environment variable, if set, wins over the button.) Allow incoming
connections to the server in the firewall of the computer it runs on. Use a
fixed address (or the computer's .local name) so it does not change. A DNS name
other than the computer's own must be listed with --allow-host.

If you installed the pyRevit toolbar component (the RoomMate Revit plugin), the
buttons are on the duHast tab, in the RoomMate panel:

  Export        pushes rooms, doors, windows, FF&E, spaces, ceilings and floors:
                check what to push, the project, the phase and the levels.
  Blueprint     sends a room and its room layout sheets as a blueprint.
  Sheets        sends chosen sheets to the sheet library.
  Tasks         carries out the room tasks the server holds for the open model.
  RLS settings  how room layout sheets are made in this model.
  Server        which RoomMate server this computer pushes to.

Restart Revit after installing to see them. Revit 2025 or newer, and pyRevit has
to be installed already. Each button is described at
https://github.com/jchristel/room_mate/blob/main/docs/REVIT-BUTTONS.md

The toolbar carries its own copy of duHast, so it does not matter whether you
have duHast installed separately. Every export starts by printing which
RoomMate and which duHast it is running, and where that duHast was loaded
from. If an export ever looks wrong, that is the first line to read -- and the
first thing to quote when reporting it.

Uninstalling removes the toolbar. Your settings and pushed data stay.


Claude Code
-----------

If Claude Code is installed, the setup registers the RoomMate data server with it
(the "Register ... with Claude Code" box, ticked by default), so it is there the
next time Claude Code starts. Nothing to configure. It is removed again on
uninstall. To redo it, or if Claude Code was installed afterwards:

  powershell -ExecutionPolicy Bypass -File "%LOCALAPPDATA%\Programs\RoomMate\Register-ClaudeMcp.ps1"

The dRofus server needs four things only you have (its host, database, project, API key). The host is a
dRofus region (api-no, api-eu, api-us, api-ca, api-au) or your organisation's own address (https://...).
Tick "Set up the dRofus MCP server" in the setup and it asks for them on the next page (the key
in a password box); or run the same script with -Drofus. To use a RoomMate server somebody else runs on the network, run it with
-Url http://<address>:5152/mcp and -AskToken (it asks for the secret and shows nothing; -Token <secret> works
but leaves the secret in your shell history).

The dRofus API key goes into the Windows Credential Manager (drofus-mcp.exe stores it itself; the key
is passed on standard input, never a command line) and Claude Code's own configuration never holds it.
If that is unavailable the key goes into a file only your account can read
(%LOCALAPPDATA%\RoomMate\secrets\drofus.key) and Claude Code is given the path. Use a read-only or project-scoped key you can rotate. The uninstaller
removes the stored key. A shared RoomMate secret, unlike the key, has to sit in Claude Code's configuration file
as a header, in plain text.

Other AI clients
----------------

The installer registers Claude Code only. For another client it will not edit that client's own config
file; it prints the entries and the file they go in, and you paste them:

  powershell -ExecutionPolicy Bypass -File "%LOCALAPPDATA%\Programs\RoomMate\Register-ClaudeMcp.ps1" -PrintConfig claude-desktop

  Client           -PrintConfig    Config file (Windows)                              Format
  Claude Desktop   claude-desktop  %APPDATA%\Claude\claude_desktop_config.json        JSON, "mcpServers"
  Cursor           cursor          %USERPROFILE%\.cursor\mcp.json                     JSON, "mcpServers"
  VS Code          vscode          .vscode\mcp.json, or "MCP: Open User Configuration" JSON, "servers" with "type"
  Codex CLI        codex           %USERPROFILE%\.codex\config.toml                   TOML, [mcp_servers.*]

On Mac/Linux: register-claude-mcp.sh --print-config <client>. These formats belong to the clients and can
change; RoomMate has not tested them against each client. The dRofus entry holds no API key: store it once
with  drofus-mcp.exe --store-key  (the key on standard input). A client that takes a URL and a header can also
use a shared RoomMate server (the section below) without any of this.

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

To share one MCP server with colleagues on a closed network, run it yourself on the
computer that holds the data (the host does not launch this one):

  set ROOMMATE_MCP_TOKEN=<a shared secret>        (or ROOMMATE_MCP_TOKEN_FILE=<a file holding it>)
  mcp.exe --server-settings <...\server.toml> --project-settings <...\projects> --http 5152 --bind 0.0.0.0

It prints the address to give out. Each person adds it by URL with the header
"Authorization: Bearer <the secret>". Restart it after saving settings in the UI.
See docs/mcp-host-setup.md in the repository, section 2b.

The dRofus MCP server
---------------------

drofus-mcp.exe in the program folder lets an MCP host read live dRofus data
(rooms, items, systems and the rest; read-only, nothing is ever written). The
host launches it, like mcp.exe, and the two can run side by side. Point the
host at:

  Command:  %LOCALAPPDATA%\Programs\RoomMate\drofus-mcp.exe
  Args:     --base-url https://api-no.drofus.com --database <db> --project <project>
  Env:      DROFUS_API_KEY_FILE=<path to a file that holds your dRofus API key>

Use full expanded paths. Keep the key in a file only you can read and give the host its path, not the key:
the host's configuration is plain text, and arguments show up in process listings. (DROFUS_API_KEY still works.) The host is whatever serves your
project: a dRofus regional host (api-no, api-eu, api-us, api-ca, api-au .drofus.com) or your
organisation's own domain. You can paste the address of a working request instead; the host,
database and project are taken from it. With no key every call is refused before
anything is sent, and the tool "server_info" says what is missing.
