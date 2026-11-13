# The API, in one screen

Moved out of the README so the landing page stays short.

Reads, all GET unless noted:

| | |
|---|---|
| `/rooms` | Every model's rooms merged, with their joined reference data and classification. The viewer's main read. |
| `/doors`, `/windows` | Openings, each with the room the project's attribution policy gives it. |
| `/ceilings`, `/floors` | Surfaces, each with the rooms it lies over — geometric, derived per read, stored nowhere. |
| `/ffe` | Items, each with the room it sits in. |
| `/spaces` | Services spaces, per model. |
| `/projects`, `/projects/{id}/buildings`, `/projects/{id}/snapshots`, `/projects/{id}/milestones` | What exists. |
| `/projects/{id}/validation` | The QA reconciliation: rooms against reference data, openings and items against rooms, spaces against rooms, and whether the models agree on a phase. |
| `/projects/{id}/areas`, `/projects/{id}/adjacency` | Hierarchy-area rollups, and which rooms share a wall. |
| `/projects/{id}/connections` | What people have authored: **open areas** (with the walls they keep closed), **vertical links** and **saved routes** and **saved reaches**. GET reads, PUT replaces the whole document and names the version it read (409 if it moved). A save that leaves out `routes` or `reaches` keeps the saved ones. See [the plan](PLAN-connectivity.md). |
| `/projects/{id}/connectivity` | Which rooms a door joins, the rooms no door reaches, and (with `from` and `to`) the shortest route between two rooms, by a chosen `method` (door to door by default; the answer lists them with their sources), optionally from and to a point inside each room (`from_x`/`from_y`, `to_x`/`to_y`), and optionally for an object of a given `width_mm` and `height_mm`: the answer's `clearance` says the narrowest and lowest points, what could not be checked and the largest object that fits. Doors, the open areas people drew and the vertical links they drew; see [the plan](PLAN-connectivity.md). |
| `POST /projects/{id}/reach` | A walking-distance grid per storey from one or more start rooms (`sources`), out to `limit_mm` on a `spacing_mm` grid, by the `exact` (default) or `grid` method; `ignore_cuts` treats open areas' walls as open. Each cell is whole mm, `-1` for no room or `-2` for beyond the limit. The same graph as the routes. MCP tool `get_reach`. |
| `/projects/{id}/stack` | For one room, the rooms over and under it, ranked, as candidates for a vertical link. A suggestion only; nothing is stored. |
| `/projects/{id}/reports/columns` | What a report over one entity may name: the property names this project's snapshots carry with Revit's own value type, the record's `$intrinsics`, joined reference labels, and the join's measures. Read from each snapshot's property dictionary — a tail read, not a parse. |
| `/projects/{id}/reports` (**POST**) | Build one report — a schedule or a by-room table — projected to the columns asked for. `?format=csv` renders the same rows as CSV. A POST that reads, because the definition does not fit a query string. |
| `/projects/{id}/comparison` (**POST**) | Diff a baseline milestone against others. Same shape, same reason. |
| `/api/reports/projects/{project}[/{report}]` | Saved reports: list, read, save (PUT) and delete. One JSON document per report beside the project settings — a saved report is a question somebody kept, not a policy, so it is kept apart from the settings a policy lives in. |
| `/projects/{id}/blueprints[/{blueprint}]`, `/projects/{id}/assignments` | Authored room layouts and where they are assigned. A blueprint's revisions are append-only; a push from Revit waits as a **pending** revision until a person accepts it. Resolving a blueprint against another room (`.../resolve`, `.../solve` for many) answers placements and findings and stores nothing: an assignment is a request, never a result. |
| `/projects/{id}/sheets`, `/projects/{id}/tagrules` | The room layout sheet library (kept by sheet identity, one file per sheet, bound to at most one blueprint) and the tag rules learned from tags in Revit views. `.../sheets/{sheet}/elevations` says which wall each elevation looks at; `.../sheets/suggest` offers sheets for a room and never applies one. |
| `/projects/{id}/tasks`, `POST /projects/{id}/roomtasks` | Room tasks, as records (`GET` lists, one is read in full, dismiss and resend). A Revit session **claims** a model's queued tasks with `POST .../roomtasks` (optionally only some `task_ids`, which is what the Tasks window sends) and reports each with `.../roomtasks/report`. `POST .../tasks/rls-preview` shows what a room layout sheet task would say, and stores nothing. `POST .../tasks/rls-set-preview` (`room_ids`, optional `sheet_id`) does the same for a SET of adjacent rooms: whether they make one, the lead room, the library sheets ranked against the set as one room, and its views (one plan, four elevations and a schedule per room, one 3D view) with their paper sizes. |
| `/api/settings/projects[/{id}]` | Read and save project settings (PUT), which hot-swaps the running registry. The list is also the Revit exporter's project picker, so each entry carries `model_phases` (model id to the phase the server holds for it, left out when nothing is stored) and the Export and Blueprint windows preselect the phase without a request per project. |

Writes are the ingest routes the extractor pushes to (`/rooms`, `/doors`,
`/windows`, `/ceilings`, `/floors`, `/ffe`, `/spaces`, each with a `/stream`
sibling) and `/projects/{id}/reference/{source}` for a reference CSV. Every read
route above has a matching **MCP tool** in the second binary — see
[MCP](STRATEGY-MCP.md) and [docs/mcp-host-setup.md](mcp-host-setup.md).

`static/` is found **beside the executable**, falling back to the working
directory — which is what `cargo run` from the repository root relies on. See `main.rs`'s
`viewer_root` before moving either.
