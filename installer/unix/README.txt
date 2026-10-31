RoomMate for Mac and Linux
==========================

The server, the viewer and the two MCP servers. There is no Revit tooling here: the pyRevit
toolbar that exports from Revit is Windows-only, and a Windows machine pushes its data to this
one over the network.

Install (for you only; no admin rights):

  tar xzf roommate-<version>-<os>-<arch>.tar.gz
  cd roommate-<version>-<os>-<arch>
  ./install.sh

Then:

  roommate                 starts the server and opens the viewer (Ctrl-C stops it)
  roommate --bind 0.0.0.0  lets other computers on a network you trust reach it. There is NO
                           authentication, so never on the internet. A Revit machine then pushes
                           to http://<this machine>:5151

Where things go (change the root with ROOMMATE_HOME):

  ~/.local/share/roommate/app        replaced on every install
  ~/.local/share/roommate/settings   yours; an install never overwrites it
  ~/.local/share/roommate/data       the pushed snapshots; an install never touches it

Claude Code
-----------

install.sh registers the RoomMate data server with Claude Code if it is installed (skip it with
--no-claude). Redo it, or add the others, with register-claude-mcp.sh in the app folder:

  register-claude-mcp.sh                 the data server, against this install
  register-claude-mcp.sh --drofus        asks for region, database, project and API key
  register-claude-mcp.sh --url http://<address>:5152/mcp --token <secret>
                                         use a RoomMate server somebody else shares
  register-claude-mcp.sh --remove        take them back out

To share the MCP server with colleagues, run on this machine:

  ROOMMATE_MCP_TOKEN=<a shared secret> ~/.local/share/roommate/app/mcp \
    --server-settings ~/.local/share/roommate/settings/server.toml \
    --project-settings ~/.local/share/roommate/settings/projects --http 5152 --bind 0.0.0.0

Binaries are not code-signed. install.sh clears the macOS download quarantine for the install.
