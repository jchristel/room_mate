# RoomMate

Revit model data → a Rust server → a browser floor-plan viewer. **Seven
entities** — rooms, doors, windows, ceilings, floors, FF&E and spaces — are
extracted from Revit, pushed as a versioned JSON contract, joined against
external reference data (dRofus is the common one; the pipeline is keyed on N
sources), classified into a project's own hierarchy, and served to a viewer that
draws plans, aggregates areas, reconciles each entity against the model and
graphs which rooms share a wall — plus a reports page that tabulates any of it.

![Screen Shot](images/Room_Mate_Splash.PNG)

**Start with [docs/](docs) — [Architecture & Strategy](docs/STRATEGY.md) is the
index.** The design rationale lives there, not here; this file is orientation
only.

## Layout

The top level maps to the pipeline, so where a thing lives tells you which stage
it belongs to:

| | |
|---|---|
| [`extractor/`](extractor) | The **producer**. `pyRevit/` holds the IronPython that runs inside Revit and pushes to the server. |
| [`src/`](src) | The **Rust server** — ingest, storage, the reference join, classification, and the geometry services (`areas`, `adjacency`, `room_locator`). Two binaries: the axum HTTP server and an MCP server over the same read logic. |
| [`static/`](static) | What the server serves: four **generated and committed** builds — the viewer (`index.html`, `viewer.js`, `viewer.css`), `vendor/renderer.bundle.js`, `settings/` and `reports/` — so a fresh clone runs with no node installed. Nothing here is hand-written any more except `common.js`, `graph.js` and `tokens.css`; edit `src-js/` and rebuild. |
| [`src-js/`](src-js) | The frontend source, built by Vite: the **viewer** (`viewer/`), its **WebGL plan renderer** (`renderer/`), the **settings page** (`settings/`) and the **reports page** (`reports/`), all React where they are not the renderer. Where new frontend code lands; see [Coding Conventions](docs/CODING-CONVENTIONS.md). |
| [`settings/`](settings) | Server config, and one TOML per project (classification tiers, sources, area policy). |
| [`scripts/`](scripts) | Dev tooling run *against* this repo: fixture generators, `fixtures/` (sample data to push or upload), `check_areas.py` (the areas diagnostic), `weekly_review.py` (the docs-vs-code drift check), `module_stats.py` (measures the module plan's cells; `--check` reports drift), and the `probe_*`/`analyse_*` pairs that settle an entity's open questions before it is built. Not shipped. |
| [`docs/`](docs) | Strategy docs, coding conventions, and handovers (landed ones in `docs/Superseded/`). |
| [`installer/`](installer) | The **Windows package** — an Inno Setup script and the launcher the Start Menu shortcut runs, producing one `RoomMate-Setup.exe` with no prerequisites. See [installer/README.md](installer/README.md). |

## The extractor and the server move together

`extractor/` and `src/` share one versioned wire contract, and the rule is
stated there: **update the extractor and the server together — there is no
transition window.** A producer on the wrong version is rejected loudly rather
than silently misparsed. Each entity carries its own version, all under
[`src/contract/`](src/contract): rooms are at `SUPPORTED_SCHEMA` (v7), and doors,
windows, ceilings, floors, FF&E and spaces each have their own constant beside
it.

That is the reason the extractor lives in this repo rather than beside the other
Revit tooling: a contract change becomes one commit instead of two repos
drifting. The cost of *not* having it here is on record — `room_boundary` was
added to the upload envelope, and the server accepted, resolved and echoed it,
while nothing sent it: for as long as the two halves sat apart, every model fell
back to a *guessed* boundary regime. Closing that was a handful of lines on the
producer once both halves were in one place. See
[Area calculation](docs/STRATEGY-AREA-CALCULATION.md).

Two constraints follow from where the extractor runs:

- **IronPython 2.7, inside Revit** — not CPython 3. Modern syntax and most
  linters do not apply, which is also why it does not live in `scripts/`.
- **CI does not cover it.** `.github/workflows/rust.yml` builds and tests the
  Rust crate only; there is no Python check. Changes there are verified by
  running them against a real model.
- **Its pyRevit buttons live outside this repository, over a *copy* of
  `extractor/pyRevit/room_m`.** So a change here is inert until it is copied
  there, and nothing checks the two are in step — `diff -rq` between them is the
  only check there is.

## Running it

```bash
cargo run -- --server-settings settings/server.toml --project-settings settings/projects
```

Serves the three pages and the API on `http://127.0.0.1:5151` (`--port`, or
`$PORT`, moves it; the host is loopback-only by design — see
`DEFAULT_HTTP_HOST`). [Server](docs/STRATEGY-SERVER.md) covers the endpoints,
[Browser](docs/STRATEGY-BROWSER.md) the viewer.

## The three pages

| | |
|---|---|
| **`/`** — the viewer | The plan: rooms drawn per storey with the door, window, FF&E, space and ceiling/floor layers over them, the hierarchy-area rollups, the adjacency graph, and the QA band that says whether the project reconciles. Each zone chooses what it draws and what a click can reach; a click on stacked elements lists them rather than guessing; every one of the seven kinds has a panel, and a grid row selects its room and brings it into view. |
| **`/settings/`** — settings | Every project setting, including the ones that only ever existed in TOML: the coordinate anchor, the area policy, the door/window/FF&E/space policies, the hierarchy exclusions. It holds the settings object as it read it and sends it back, so it cannot silently drop what it does not show — the bug its hand-written predecessor shipped. |
| **`/reports/`** — reports | Tabular reports: a **schedule** of any entity, any entity **by room**, the **QA checks** (reference data, unresolved openings, unmatched spaces and rooms), and **milestone comparison**, which used to be its own page. Rows and CSV are rendered by the server, so a download and an MCP host get the same bytes. |

## The API, in one screen

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
| `/projects/{id}/reports/columns` | What a report over one entity may name: the property names this project's snapshots carry with Revit's own value type, the record's `$intrinsics`, joined reference labels, and the join's measures. Read from each snapshot's property dictionary — a tail read, not a parse. |
| `/projects/{id}/reports` (**POST**) | Build one report — a schedule or a by-room table — projected to the columns asked for. `?format=csv` renders the same rows as CSV. A POST that reads, because the definition does not fit a query string. |
| `/projects/{id}/comparison` (**POST**) | Diff a baseline milestone against others. Same shape, same reason. |
| `/api/reports/projects/{project}[/{report}]` | Saved reports: list, read, save (PUT) and delete. One JSON document per report beside the project settings — a saved report is a question somebody kept, not a policy, so it is kept apart from the settings a policy lives in. |
| `/api/settings/projects[/{id}]` | Read and save project settings (PUT), which hot-swaps the running registry. |

Writes are the ingest routes the extractor pushes to (`/rooms`, `/doors`,
`/windows`, `/ceilings`, `/floors`, `/ffe`, `/spaces`, each with a `/stream`
sibling) and `/projects/{id}/reference/{source}` for a reference CSV. Every read
route above has a matching **MCP tool** in the second binary — see
[MCP](docs/STRATEGY-MCP.md) and [docs/mcp-host-setup.md](docs/mcp-host-setup.md).

`static/` is found **beside the executable**, falling back to the working
directory — which is what the line above relies on. See `main.rs`'s
`viewer_root` before moving either.

## Installing it

For a machine that is not a development one, [`installer/`](installer) builds a
single `RoomMate-Setup-<version>.exe`: both binaries, the viewer, and a seeded
per-user data folder, with no Rust, node or runtime to install first.

```powershell
winget install JRSoftware.InnoSetup   # build machine only, once
.\installer\build.ps1
```

It installs per-user (no admin), keeps the app under
`%LOCALAPPDATA%\Programs\RoomMate` and everything writable — settings and the
snapshot store — under `%LOCALAPPDATA%\RoomMate`, so an upgrade never touches
an edited project file. [installer/README.md](installer/README.md) has the
reasoning.
