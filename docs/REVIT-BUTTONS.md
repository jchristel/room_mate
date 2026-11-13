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

Opens one window to push Revit data to the RoomMate server. Check what to export (rooms, doors, windows, FF&E by room or by space, spaces, ceilings, floors), then choose the project, the phase, and either the whole model or only chosen levels. The phase is preselected from what the server already holds for those models, and your last project, choices and levels are remembered. Beside doors, windows and each FF&E row, **Include Shared Nested Families** decides whether a family nested in another one is pushed as an element of its own: off by default for doors and windows (a nested leaf or pane is not a door or a window), on by default for FF&E (the project setting then decides what reads show, and a create-RLS task can tag a nested part such as a shower rail). Off for FF&E means nested items are not sent at all. Either way, each door, window and FF&E item tells the server which shared nested families it holds, at every depth. A progress bar shows the work, each result is one readable line, and the run ends with a single Finished line.

### <img src="../extractor/pyRevit/RoomMate.extension/duHast.tab/RoomMate.panel/Blueprint.pushbutton/Icon.png" alt="" width="40" align="absmiddle"> Blueprint

Pushes a room and its room layout sheets to the server as a blueprint. Choose the project and phase, check the rooms, name each blueprint and pick its sheets by hand or with a matching rule such as "sheet number is part of room number". A room needs a sheet and a name before it can be pushed, and rows that share a blueprint name are shaded. The server captures the room's contents itself, so push the rooms and FF&E first; a name the project already has becomes a pending revision to accept in the viewer.

*Before it pushes, it checks the project's settings on the server: with no **Tag type code** set, tags on items (FF&E) cannot be kept, so it warns and asks whether to push anyway.*

### <img src="../extractor/pyRevit/RoomMate.extension/duHast.tab/RoomMate.panel/Sheets.pushbutton/Icon.png" alt="" width="40" align="absmiddle"> Sheets

Sends the sheets you choose to the server's sheet library. A filter, "Check all matched" and a matching rule help choose the sheets and the rooms each one covers. The rooms only supply the size, area and item count the sheet is ranked on later; none is linked to it. A progress bar and one readable line per sheet report the result.

### <img src="../extractor/pyRevit/RoomMate.extension/duHast.tab/RoomMate.panel/Tasks.pushbutton/Icon.png" alt="" width="40" align="absmiddle"> Tasks

Carries out the room tasks the server holds for the open model. The window lists the queued tasks, filterable by room, kind and level; Refresh asks the server again without losing what you checked, and the project and filters are remembered. The checked tasks are claimed and all checked before anything is written; populate and update tasks place the items, and create-RLS tasks make the sheets, views, schedule and tags, each task in one transaction group. A new 3D view hides the walls between its camera and the room (the server names the room's edges to clear; every straight wall with a face within 200 mm of one goes) and makes the items along them 70% transparent, in that view only. A room drawn at an angle to its template's room gets its plan and 3D views turned so it sits on the sheet the way the template's room does (the server chooses the turn from the walls; not yet run in Revit from a task). Every step is written to `RoomMate.log`, and placing items has not yet been run in Revit.

A **create RLS set** task (several adjacent rooms laid out together in the viewer's RLS set mode) makes all its sheets in one transaction group: one plan round every room, four elevations per room, a schedule per room and one 3D view, each viewport where the layout put it (measured from the sheet's bottom left corner). The sheets and the set's plan and 3D view are named by the **Sets of rooms** templates in RLS settings (falling back to the single-room ones); the elevations and schedules by the single-room templates, from their own room. Not yet run in Revit.

### <img src="../extractor/pyRevit/RoomMate.extension/duHast.tab/RoomMate.panel/RlsSettings.pushbutton/Icon.png" alt="" width="40" align="absmiddle"> RLS settings

Sets how a room layout sheet is made in the open model: the title block per sheet size, tags, detail numbering, the sheet number and name templates, and a **Views** group. The Views group holds the viewport type, which applies to every view, then a group for each kind of view (plan, elevation, 3D, schedule) with its view type (the schedule base for a schedule), its **View name** template and its **Title on sheet** template. The Elevation group also has **Place 2D grid**: a line based detail family drawn along the bottom of every new elevation, as long as the elevation is wide, as the sheet's last step. Both take fixed text, `{Room Parameter}` and `{Seq_Counter}` (1, 2, 3 ... per view of that kind on the sheet); the elevation view name must use `{Seq_Counter}` so each elevation gets its own name, and an empty title keeps Revit's. A model set up with the older single view name shows it split per kind, with the old endings (PLAN, ELEV n, 3D, SCHEDULE), until it is saved. A **Sets of rooms** group names a set of adjacent rooms (a create RLS set task): its sheet number and name, plan and 3D view names and their titles on sheet, each empty to use the single-room one. They read the lead (largest) room's parameters, `{Rooms:Number}` for every room's (01.06, 01.07), and `{Sheet_Counter}` for the sheet's place in the set, which the sheet number needs when a set has more than one sheet. The settings are kept in the model itself and checked against it, with anything the model lacks marked "(not in this model)". The Tasks button reads them, and opens this window itself when they are missing or incomplete. No server is needed.

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
- **Remembered choices** are kept in `%APPDATA%\RoomMate\prefs.json` (the Export, Sheets, Blueprint and Tasks
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
