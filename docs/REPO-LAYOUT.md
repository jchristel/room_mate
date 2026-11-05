# Repository layout

Moved out of the README so the landing page stays short.

The top level maps to the pipeline, so where a thing lives tells you which stage
it belongs to:

| | |
|---|---|
| [`extractor/`](../extractor) | The **producer and the Revit-side worker**. `pyRevit/` holds the IronPython that runs inside Revit: it pushes to the server and claims its tasks (`RoomMate.extension` is the `RoomMate` panel, described button by button in [the Revit plugin](REVIT-BUTTONS.md); `room_m` is the code). `tests/` runs its pure parts on CPython. `duhast.lock` pins the duHast it ships with. |
| [`src/`](../src) | The **Rust server** — ingest, storage, the reference join, classification, the geometry services (`areas`, `adjacency`, `room_locator`), and the authored data (blueprints, assignments, tasks, the sheet library, tag rules). Two binaries: the axum HTTP server and an MCP server over the same read logic. |
| [`crates/`](../crates) | Workspace members: `roommate-shared` (the settings types, shared with the frontend through generated TypeScript) and `drofus-mcp`, a **standalone** MCP server over the dRofus Project API (it depends on nothing else here and could be copied out as its own repository). |
| [`static/`](../static) | What the server serves: **generated and committed** builds — the viewer (`index.html`, `viewer.js`, `viewer.css`), `vendor/renderer.bundle.js`, and the pages `settings/`, `reports/`, `blueprints/`, `library/` and `tasks/` — so a fresh clone runs with no node installed. Nothing here is hand-written any more except `common.js` and `tokens.css`; edit `src-js/` and rebuild. |
| [`src-js/`](../src-js) | The frontend source, built by Vite: the **viewer** (`viewer/`), its **WebGL plan renderer** (`renderer/`), and the pages **settings**, **reports**, **blueprints**, **library** (room layout sheets and tag rules) and **tasks**, all React where they are not the renderer. Where new frontend code lands; see [Coding Conventions](CODING-CONVENTIONS.md). |
| [`samples/house-a/`](../samples/house-a) | **A sample project to run the server against**: a small house as Revit pushed it, with a blueprint and its assignment. Built by `scripts/make_sample.py` and read by a test, so it stays in the current format. |
| [`settings/`](../settings) | Server config, and one TOML per project (classification tiers, sources, area policy). Every available setting, with a sample of each, is in the [settings reference](SETTINGS.md). |
| [`scripts/`](../scripts) | Dev tooling run *against* this repo: fixture generators, `fixtures/` (sample data to push or upload), `check_areas.py` (the areas diagnostic), `weekly_review.py` (the docs-vs-code drift check), `module_stats.py` (measures the module plan's cells; `--check` reports drift), and the `probe_*`/`analyse_*` pairs that settle an entity's open questions before it is built. Not shipped. |
| [`docs/`](docs) | Strategy docs, coding conventions and plans. |
| [`installer/`](../installer) | The **Windows package** — an Inno Setup script and the launcher the Start Menu shortcut runs, producing one `RoomMate-Setup.exe` with no prerequisites. See [installer/README.md](../installer/README.md). |
