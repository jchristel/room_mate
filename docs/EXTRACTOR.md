# The extractor and the Revit-side status

Moved out of the README so the landing page stays short.

## The extractor and the server move together

`extractor/` and `src/` share one versioned wire contract, and the rule is
stated there: **update the extractor and the server together — there is no
transition window.** A producer on the wrong version is rejected loudly rather
than silently misparsed. Each entity carries its own version, all under
[`src/contract/`](../src/contract): rooms are at `SUPPORTED_SCHEMA` (v7), and doors,
windows, ceilings, floors, FF&E and spaces each have their own constant beside
it.

That is the reason the extractor lives in this repo rather than beside the other
Revit tooling: a contract change becomes one commit instead of two repos
drifting. The cost of *not* having it here is on record — `room_boundary` was
added to the upload envelope, and the server accepted, resolved and echoed it,
while nothing sent it: for as long as the two halves sat apart, every model fell
back to a *guessed* boundary regime. Closing that was a handful of lines on the
producer once both halves were in one place. See
[Area calculation](STRATEGY-AREA-CALCULATION.md).

Two constraints follow from where the extractor runs:

- **IronPython 2.7, inside Revit** — not CPython 3. Modern syntax and most
  linters do not apply, which is also why it does not live in `scripts/`. The tests run on CPython 3,
  so `extractor/tests/test_py2_syntax.py` fails on what Python 2 cannot parse (`print` used as a value,
  f-strings, `nonlocal`, `yield from`): such a line passes every other test and fails only when Revit imports it.
- **CI covers only what can run without Revit.** `.github/workflows/extractor.yml`
  checks that the extension bundles (no non-ASCII byte, every entry point exists), runs
  the pure parts on CPython (`extractor/tests`: the task planner, the RLS planner, the
  settings forms, the geometry) and checks the duHast pin. Anything that calls
  Revit is verified by running it against a real model.
- **Its pyRevit buttons live in this repository**
  (`extractor/pyRevit/RoomMate.extension`, the `RoomMate` panel on the duHast tab) and the installer ships them.
  A checkout is not loadable as it stands: `installer\build-extension.ps1 -Deploy`
  assembles an extension (`room_m` plus a pinned duHast, or a local one with `-DuHastPath`) into
  `%APPDATA%\pyRevit\Extensions`. The Tasks button, which makes room layout sheets
  from the server's tasks, has run in Revit end to end (2026-11-01); see [`CLAUDE.md`](../CLAUDE.md) for what has and has not.

## Revit-side status

The `RoomMate` panel (on the duHast tab) has six buttons: **Export** (one window: what to
push, the project, the phase, whole model or chosen levels), **Blueprint**, **Sheets**,
**Tasks**, **RLS settings** and **Server**. Each is described, with its icon, in
[the Revit plugin](REVIT-BUTTONS.md). The extension is built with
`installer\build-extension.ps1 -Deploy` (`-DuHastPath` uses a local duHast instead of the pinned one).

| | State |
|---|---|
| Pushing the seven entities | Built and in daily use. The Export, Sheets and Blueprint windows share their rules (`match_rule`, `prefs`, `phase_hint`), report one readable line per result with one Finished line (`report`), and were checked in Revit on 2026-11-05. |
| **Tasks → room layout sheets** | **Run in Revit end to end** (2026-11-01 onwards). A window lists the open model's queued tasks (filter by room, kind and level); the checked ones are claimed, checked as a whole before anything is made, and each sheet is built in one transaction group: the plan view, the elevations (each marker and its views placed from the template, the elevation line moved by its view pointer, cropped to the room's floor and the template's height), a 3D view with a section box at the template's height on the plan crop's footprint, the schedule, viewport types, titles and detail numbers, the annotation crop, and the room tag. Everything it needs is set in **RLS settings** (stored in the model). |
| Tasks → 3D views see into the room | **Run in Revit** (2026-11-08): the walls between the camera and the room are hidden (edges named by the server, walls found by a face within 200 mm) and the items along them are 70% transparent. |
| Tasks → plan crop | **Run in Revit** (2026-11-08): the plan keeps the template crop's offsets from its room on every side, and a 3D view's section box takes the same footprint. |
| Tasks → Title on Sheet | **Run in Revit** (2026-11-08): a template per kind of view. |
| Tasks → placing items | Built; **not run in Revit**. |
| Tags on items, FF&E tags in elevations | Prototype. Plan and elevation tags follow rules learned by type code (an elevation's by whether the item is seen front on or from the side, measured from the item's box centre). A nested part is tagged only when a rule exists for its type code, and that **works in Revit** (2026-11-08). |
| Shared nested families on export | Built. A choice per entity in the Export window, and every door, window and FF&E item lists the nested families it holds. **Run in Revit for FF&E** (2026-11-08); doors and windows not checked. |

What each Revit-side rule rests on (including the measured facts about elevation markers and
`Level.Elevation`) is in [`CLAUDE.md`](../CLAUDE.md); the design is in
[the blueprints plan](PLAN-blueprints.md).
