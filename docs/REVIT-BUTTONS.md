# The Revit plugin: the RoomMate panel

RoomMate's Revit side is a **pyRevit extension**, `RoomMate.extension`
([`extractor/pyRevit`](../extractor/pyRevit)). It adds the **RoomMate** panel to the **duHast** tab of
Revit's ribbon, with six buttons that push data to the RoomMate server and carry out the tasks it holds. It
runs as IronPython inside Revit 2025 or newer and needs pyRevit; the Windows installer ships it as an optional
component, and for development `installer\build-extension.ps1 -Deploy` assembles it into
`%APPDATA%\pyRevit\Extensions` (`-DuHastPath` builds against a local duHast). Restart Revit, or reload
pyRevit, after installing or rebuilding: Revit keeps the old code in memory until then.

Each section below is the button's icon, then a four-sentence summary.

### <img src="../extractor/pyRevit/RoomMate.extension/duHast.tab/RoomMate.panel/ExportAll.pushbutton/Icon.png" alt="" width="40" align="absmiddle"> Export

Opens one window to push Revit data to the RoomMate server. Check what to export (rooms, doors, windows, FF&E by room or by space, spaces, ceilings, floors), then choose the project, the phase, and either the whole model or only chosen levels. The phase is preselected from what the server already holds for those models, and your last project, choices and levels are remembered. A progress bar shows the work, each result is one readable line, and the run ends with a single Finished line.

### <img src="../extractor/pyRevit/RoomMate.extension/duHast.tab/RoomMate.panel/Blueprint.pushbutton/Icon.png" alt="" width="40" align="absmiddle"> Blueprint

Pushes a room and its room layout sheets to the server as a blueprint. Choose the project and phase, check the rooms, name each blueprint and pick its sheets by hand or with a matching rule such as "sheet number is part of room number". A room needs a sheet and a name before it can be pushed, and rows that share a blueprint name are shaded. The server captures the room's contents itself, so push the rooms and FF&E first; a name the project already has becomes a pending revision to accept in the viewer.

### <img src="../extractor/pyRevit/RoomMate.extension/duHast.tab/RoomMate.panel/Sheets.pushbutton/Icon.png" alt="" width="40" align="absmiddle"> Sheets

Sends the sheets you choose to the server's sheet library. A filter, "Check all matched" and a matching rule help choose the sheets and the rooms each one covers. The rooms only supply the size, area and item count the sheet is ranked on later; none is linked to it. A progress bar and one readable line per sheet report the result.

### <img src="../extractor/pyRevit/RoomMate.extension/duHast.tab/RoomMate.panel/Tasks.pushbutton/Icon.png" alt="" width="40" align="absmiddle"> Tasks

Carries out the room tasks the server holds for the open model. The window lists the queued tasks, filterable by room, kind and level, and Refresh asks the server again without losing what you checked. The checked tasks are claimed and all checked before anything is written; populate and update tasks place the items, and create-RLS tasks make the sheets, views, schedule and tags, each task in one transaction group. Every step is written to `RoomMate.log`, and placing items has not yet been run in Revit.

### <img src="../extractor/pyRevit/RoomMate.extension/duHast.tab/RoomMate.panel/RlsSettings.pushbutton/Icon.png" alt="" width="40" align="absmiddle"> RLS settings

Sets how a room layout sheet is made in the open model: the title block per sheet size, view types, schedule base, tags, and the number, name and view-name templates. The settings are kept in the model itself and checked against it, with anything the model lacks marked "(not in this model)". The Tasks button reads them, and opens this window itself when they are missing or incomplete. No server is needed.

### <img src="../extractor/pyRevit/RoomMate.extension/duHast.tab/RoomMate.panel/Server.pushbutton/Icon.png" alt="" width="40" align="absmiddle"> Server

Chooses which RoomMate server this computer and this model push to. Type an address such as `192.168.0.97` or `mac.local:5151`, `local` for the server on this computer, or `reset` to forget it. The address is checked by asking that server for its projects before it is kept, then saved in the model and in `server.json`. Every push window shows the address in its blue header, so you can see where a push will go.

## What every window does the same way

- **Check, not tick.** Lists have "Check all" and "Check none" (or "Check all matched" where a filter or rule
  decides what matches). "Check all matched" in the Sheets and Blueprint windows checks only what has a
  match, and says how many it left.
- **Filters** take words that must all appear; a leading `-` excludes (`101 -roof`).
- **Matching rules** (Sheets and Blueprint) are written with `{Parameter}` placeholders, as the sheet and view
  naming templates are: `Sheet {Sheet Number}` *is part of* room `{Number}`. Any sheet parameter can be
  matched with any room parameter, and a side that cannot be filled (a blank or missing parameter) never
  matches. The window names a placeholder that no sheet or room has.
- **The server** is shown in the blue header of every window that pushes or reads from it.
- **Remembered choices** are kept in `%APPDATA%\RoomMate\prefs.json` (the Export, Sheets and Blueprint
  windows). A missing or unreadable file only means the window starts blank.

## What you read when a run finishes

A push prints one line per result with `[ok]`, `[FAILED]`, `[HELD]` (accepted but held back, for example a
phase that disagrees with the one the model is fixed to) or `[cancelled]`, then one **Finished** line that says
whether it completed with no problems. When the server refuses something, the line says the HTTP status and
points at the log: `%APPDATA%\RoomMate\RoomMate.log`, where every step is recorded.

## Where things are kept

| What | Where |
|---|---|
| The server address | The open model (extensible storage) and `%APPDATA%\RoomMate\server.json`; `ROOMMATE_SERVER` overrides both |
| RLS settings | The open model (extensible storage) |
| Remembered window choices | `%APPDATA%\RoomMate\prefs.json` |
| The log | `%APPDATA%\RoomMate\RoomMate.log` |

What has and has not been run in Revit is in [The extractor](EXTRACTOR.md); the rules behind each button are
in [`CLAUDE.md`](../CLAUDE.md).
