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

## Releasing it

Push a tag and [`release.yml`](../.github/workflows/release.yml) builds the
installer and attaches it to the release:

```bash
git tag 0.0.1-beta.4 && git push origin 0.0.1-beta.4
```

It creates the release if the tag was pushed from a terminal, and uploads into
it when the release already exists because it was published from the web UI.
A `workflow_dispatch` run builds and uploads the installer as a **run artifact**
and publishes nothing — the way to exercise the whole path without spending a
tag.

Two things the workflow does that are not obvious:

- **It runs `cargo test` and rebuilds the renderer bundle**, because a tag push
  triggers *neither* `rust.yml` nor `frontend.yml` — both are scoped to branches
  and pull requests. The tag usually points at a commit that already passed on
  main, but "usually" is not a gate, and this is the one workflow that hands a
  binary to somebody.
- **It passes the tag to `build.ps1` as the version**, rather than letting it
  fall back to `Cargo.toml`. The crate now follows the same format, but it is
  maintained by hand and records the version last *released* — not the one
  being cut. Tagging `0.0.1-beta.5` without remembering to edit the crate first
  would stamp `beta.4` onto it, and since `AppId` is fixed on purpose, Windows
  would read that as an upgrade of a thing whose version never moved.

### The version is two versions

Inno needs both, and only one of them may be free text. `AppVersion` is the
display string — the setup filename and the Add/Remove Programs entry — and
takes `0.0.1-beta.4` happily. `VersionInfoVersion` is the Windows *file version*
resource, which must be purely numeric with at most four parts and is a
**compile error**, not a warning, when it is not. `build.ps1` derives the second
from the first, folding a trailing pre-release number into the fourth component:

| Tag | Filename | File version |
|---|---|---|
| `0.0.1-beta.4` | `RoomMate-Setup-0.0.1-beta.4.exe` | `0.0.1.4` |
| `v1.2.3` | `RoomMate-Setup-1.2.3.exe` | `1.2.3` |
| `2.0.0-rc.7` | `RoomMate-Setup-2.0.0-rc.7.exe` | `2.0.0.7` |

That fold is why `beta.3` and `beta.4` are distinguishable once the tag text has
been stripped away — otherwise both report `0.0.1`, and "which build is actually
installed?" has no answer on the machine that has it.

### One installer replaces three assets

Releases up to `0.0.1-beta.3` carried `roommate.exe`, `mcp.exe` and
`static.zip` loose, which left whoever downloaded them to assemble the layout
and discover two mandatory settings flags on their own. Nothing stops you
attaching those as well, but the installer is the supported path.

The setup is **not code-signed**, and downloading it from a release is exactly
the path that attaches a Mark-of-the-Web, so SmartScreen warns on first run.
Fine for hand-delivery; a certificate is the fix if this gets distributed
widely.

## What the install looks like, and why it is split

| | |
|---|---|
| `%LOCALAPPDATA%\Programs\RoomMate` | `roommate.exe`, `mcp.exe`, `static/`, the launcher, the settings template |
| `%LOCALAPPDATA%\RoomMate` | `settings/` (edited by the settings UI) and `data/snapshots/` (the store) |
| `%APPDATA%\pyRevit\Extensions\RoomMate.extension` | the pyRevit toolbar, optional component |

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

## The pyRevit toolbar, and the duHast inside it

The installer's second component is the producer: the pyRevit extension that
pushes from Revit. It is assembled by
[`build-extension.ps1`](build-extension.ps1) from three sources that are
deliberately not one:

| | |
|---|---|
| bundles | `extractor/pyRevit/RoomMate.extension` — buttons, icons, manifests |
| `room_m` | `extractor/pyRevit/room_m`, the extractor's one source |
| duHast | fetched at the commit in `extractor/duhast.lock` |

A checkout is therefore **not** a loadable extension — `lib/` is empty in the
repo. `.\installer\build-extension.ps1 -Deploy` assembles one and writes it
where pyRevit looks, which is the development loop; `-DuHastPath <path to
src\duHast>` builds it against a local duHast instead of the pinned commit.

**Why duHast is shipped rather than found.** duHast decides what an export
*means* — an item's level, a door's footprint, whether a hole in a floor is a
hole. Each of those was wrong in an older duHast, each was fixed upstream, and
in every case the export still looked successful. A copy of unknown age answers
those questions differently and says nothing about it. So the version is pinned,
the python is shipped (without duHast's DLLs, which nothing `room_m` imports
needs), and both packages are stamped with a `_build_info.py` recording the
commit they came from.

**The check that decides is at run time, not install time.** The wizard scans
pyRevit's extension folders and warns when it finds another duHast, but what it
sees is a snapshot: which copy a Revit session actually imports depends on
`sys.path` order and on what is already loaded. So `room_m.utils.provenance`
prints the RoomMate build, the duHast build and the path it came from at the top
of **every** run, and warns when that is not the copy the build intended. The
buttons also declare a clean engine, so each run starts from its own `lib/`
rather than inheriting another extension's already-imported duHast.

**The tab is called duHast; the extension is called RoomMate.** pyRevit merges
ribbon tabs by *tab* name (`create_ribbon_tab(..., update_if_exists=True)`), so
the panel joins the duHast tab where that extension is installed and creates the
tab on its own where it is not. The extension name must stay different: pyRevit
keys its parse cache on it (`cache_<name>`), and command ids are built from the
folder names inside the extension, so a second `duHast-2025.extension` would
share both with the real one.

**Upgrades wipe the extension folder** (`[InstallDelete]`) rather than merging
into it, because a module deleted in a newer version would otherwise stay behind
and keep importing. Nothing there is user data — the whole folder came from an
installer — so it also goes on uninstall, unlike the settings and the store.

The component is unticked automatically when pyRevit is not installed, and
installing it does not edit `pyRevit_config.ini`: `%APPDATA%\pyRevit\Extensions`
is scanned without being registered.

## Two things this cannot check for you

- **The renderer bundle.** `static/vendor/renderer.bundle.js` is generated and
  committed so packaging needs no node. `build.ps1` fails if it is missing, but
  it cannot tell whether it is *current* — that is
  [`frontend.yml`](../.github/workflows/frontend.yml), which rebuilds and diffs
  it. Touched `src-js/`? Run `npm run build` before packaging.
- **Whether the pinned duHast is the RIGHT one.** `build.ps1` verifies that the
  commit fetched is the commit named, and that every duHast module `room_m`
  imports is present in it — but not that the pin is current, or that an export
  built with it is correct. Bumping `extractor/duhast.lock` is a change worth a
  probe, not a version bump.

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
