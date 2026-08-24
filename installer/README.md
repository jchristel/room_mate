# Packaging RoomMate for Windows

Builds a single `RoomMate-Setup-<version>.exe`: both binaries, the viewer, and
a seeded per-user data folder. [Inno Setup](https://jrsoftware.org/isinfo.php)
is the compiler, and it is needed on the **build** machine only — the setup it
produces has no prerequisites at all, which is the point. No Rust, no node, no
Visual C++ redistributable (the binaries are MSVC-target but statically link
nothing the base OS lacks).

```powershell
winget install JRSoftware.InnoSetup   # once
.\installer\build.ps1
```

Output: `target/installer/RoomMate-Setup-<version>.exe`.

## What the install looks like, and why it is split

| | |
|---|---|
| `%LOCALAPPDATA%\Programs\RoomMate` | `roommate.exe`, `mcp.exe`, `static/`, the launcher, the settings template |
| `%LOCALAPPDATA%\RoomMate` | `settings/` (edited by the settings UI) and `data/snapshots/` (the store) |

Per-user rather than `Program Files`, so nothing needs admin rights and both
writable locations are actually writable. That is not cosmetic: the settings
page **writes project TOML files back**, and the snapshot store is written on
every push — an install under `Program Files` would put both behind UAC and
turn a routine save into a permission error.

The split is what makes upgrades safe. The app half is replaced wholesale on
every install; the data half is seeded with `onlyifdoesntexist` and marked
`uninsneveruninstall`, so an edited project file survives both an upgrade and
an uninstall. Losing a project's classification tiers to a routine upgrade is
not recoverable from anywhere else.

## The launcher is not a convenience

`RoomMate.ps1` is the shortcut's target, and it does three things the server
binary cannot do for itself:

- **Passes the settings paths.** `roommate.exe` requires `--server-settings`
  and `--project-settings`; there are no defaults, and a shortcut pointing
  straight at the exe would fail on launch.
- **Waits for the port before opening the browser.** Opening it immediately
  races startup and lands on a connection error often enough to look broken.
  A hidden watcher polls and opens once the socket answers.
- **Runs the server in the foreground.** Closing the console window stops it. A
  background start leaves an invisible process holding 5151 with no obvious way
  to stop it.

It also re-seeds settings when they are missing, which is what makes deleting
the data folder a reset rather than a broken install, and it exits early with a
browser tab if something is already listening — so double-clicking the shortcut
twice is not a port clash.

## Two things this cannot check for you

- **The renderer bundle.** `static/vendor/renderer.bundle.js` is generated and
  committed so packaging needs no node. `build.ps1` fails if it is missing, but
  it cannot tell whether it is *current* — that is
  [`frontend.yml`](../.github/workflows/frontend.yml), which rebuilds and diffs
  it. Touched `src-js/`? Run `npm run build` before packaging.
- **Which pyRevit the extractor is running.** The installer ships the server
  half only. The producer is `extractor/pyRevit/room_m`, deployed separately,
  and it shares a versioned wire contract with what is packaged here — see the
  README's "The extractor and the server move together".

## The viewer's files are found beside the exe

`main.rs`'s `viewer_root` resolves `static/` relative to the **executable**,
falling back to the working directory. Before that, a bare
`ServeDir::new("static")` was correct only when the process was launched from
the crate root — true under `cargo run`, false for every shortcut. The failure
was the expensive kind: the API kept answering and only the pages 404'd, so the
server looked healthy while the website was missing. Anything that relocates
`static/` away from `roommate.exe` reintroduces exactly that.

## What is deliberately not in here

- **No MCP host registration.** `mcp.exe` ships, but nothing writes to
  `claude_desktop_config.json` — an installer editing another application's
  config file is a surprise, and the paths are in `README-INSTALL.txt` for
  anyone who wants it. See [mcp-host-setup.md](../docs/mcp-host-setup.md).
- **No sample data.** The store starts empty. `[test_data]` is stripped from the
  installed `server.toml`; a project nobody pushed, holding rooms nobody
  exported, is confusing rather than helpful in a fresh install. For the same
  reason only `sample-project.toml` is seeded, not the whole `settings/projects`
  directory — the rest are this repo's own jobs and plate fixtures, and five
  empty projects in a fresh install read as a failed push rather than as
  examples.
- **No auto-start.** The server runs when the shortcut runs.
