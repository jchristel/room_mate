# Plan — viewer selection: surfaces, stacked picks, grid → plan

Four viewer behaviours (G4 added 2026-09-20). Asked for on 2026-09-19. This plan is **closed-scope**:
it ends when the four goals below are met, and anything not written under
"Goals" is out of it. Archive to `Superseded/` when it lands.

## Goals

Each goal is done when every one of its criteria holds, verified by driving
the page (House A and RHH), not by reading the diff.

### G1 — Ceilings, floors and spaces are selectable and inspected

1. With a layer switched on, a ceiling, floor or space under the pointer can be
   selected (through the pick menu, G2). A layer that is off is never
   selectable.
2. The selected element is marked on the plan across its **whole** footprint:
   every piece of a multi-piece ceiling or floor, holes cut out.
3. The side panel shows:
   - **ceiling / floor**: type name, id, level, model, height offset as sent;
     the rooms it lies over, largest first, each with `fraction_of_element` and
     `fraction_of_room`; "(unattributed)" when that list is empty; instance and
     type properties as two sections, through the existing property filter.
   - **space**: name, number, id, level, model, enclosure; its properties and
     joined reference data, through the same filter.
4. An element no longer in the current payload (storey switched, layer turned
   off) shows the "not in the current scope" note the door panel already uses.

### G2 — Stacked objects are listed and pickable

1. Everything under a point comes back as one ordered list:
   doors → windows → FF&E → room → spaces → ceilings → floors, and **smallest
   first within each layer**. Only layers switched on contribute.
2. **Left click is unchanged**: it selects the first entry. **Right click** on
   the plan opens a pick menu at the pointer listing every entry — kind, then
   name or type name, then id — even when there is only one. Right click on
   empty plan opens nothing, and the browser's own context menu is suppressed
   only where the pick menu opens.
3. Hovering an entry marks that element on the plan; leaving it removes the
   mark. Clicking an entry selects it and closes the menu.
4. Escape, a click outside, a pan, a zoom or a storey switch closes the menu
   and leaves no hover mark behind.

### G3 — A grid row selects its room

1. Clicking a grid row selects that room, exactly as a plan click on it does
   (mark in every zone drawing it, room panel, adjacency).
2. A room whose storey no zone shows is still selected; the plan shows nothing
   and the panel says "storey not on screen", as it already does.
3. The selected room's row is marked in the grid, whichever way it was selected,
   and the mark survives scrolling the virtualised grid.

4. **Pan is the default.** A zone showing the room's storey, where the room is
   not wholly inside the view, pans so the room is centred; zoom is unchanged.
   A zone where the room is already wholly visible does not move. Each zone is
   decided on its own.
5. A checkbox in the grid header, **"Pan to room"**, checked by default.
   Unchecked, a row click highlights only (criteria 1–3, no pan).

### G4 — An outline layer's selection colour is a project setting

A space, ceiling or floor marks in the accent and ignores `[appearance]`,
where a room, door, window and item each read a `--sel-<entity>` custom
property the project sets (A3 shipped it that way because the settings
contract offers these three a `line` colour and no `selection` one).

1. `[appearance] spaces`, `ceilings` and `floors` each take a `selection`
   colour, like `rooms` does, with a control on the settings page.
2. `.surface-selected-mark` reads it per entity, falling back to the accent,
   so a project that sets nothing looks exactly as it does today.

## Non-goals

Stated so they are not drifted into: click-to-cycle and Tab-key cycling (the
menu replaces both); a count badge or any other "there is more here" hint
outside the menu; ceilings/floors in the room panel's "in this room"; a space →
room join in the panel; length units (a separate exercise); zooming to a
room, and panning on any selection that did not come from the grid; switching
a zone's storey to reach a room; remembering the "Pan to room" checkbox across
reloads; scrolling the grid to a plan selection; persisting a selection across
reloads; multi-select; selection marks in the SVG
export; the ceilings/floors QA reports; and any server or wire change **other
than G4's three settings fields**, which are settings, not entity data.

## Phases

**A is plan-side TypeScript and lands before the viewer moves to React. B is the
move, and is not part of this plan. C is page code and is written once, on the
React page — nothing in C is built in `static/index.html`.**

### A — plan-side groundwork (`src-js/renderer/`), one PR each

- **A1. One selection setter, one hover setter.** `PlanRenderer.setSelection(
  sel: { kind, id } | null)` replaces `setSelection(roomId)`,
  `setDoorSelection`, `setWindowSelection`, `setItemSelection` and their four
  private fields; `setHover` takes the same `{ kind, id }` instead of a room id,
  because the menu previews any kind. `#drawMarks` dispatches on kind.
  `applySelection` and the hover call in `static/index.html` adapt. No
  behaviour change — verified by selecting and hovering each kind in the old
  page.
- **A2. The stack.** `DoorIndex.allAt` and `RoomIndex.allAt` return every hit
  (the ring tests already visit them — `doorAt` just keeps the last). Ring area
  is computed once at index build, not per click. `PlanRenderer.pickAllAt`
  returns the ordered list; `pickAt` becomes its first entry, so the old page
  gets smallest-first **immediately**. Tests in `spatial.test.ts`: order
  across layers, smallest-first within one, a hole excludes, empty point.
- **A3. Surfaces and spaces in the stack.** A `SurfaceIndex` (Flatbush over
  piece boxes; a point hits a surface when it is in any piece's outer ring and
  none of that piece's holes) for ceilings and floors, and the same over a
  space's `loops`. Built from the lists the paint already filtered, so what is
  pickable is what is drawn. `Pick` gains `space`, `ceiling`, `floor`. Marks
  draw as one SVG `<path>` per element with a subpath per ring (stroke only,
  so no fill rule is needed). **`pickAt` excludes
  the three new kinds** — the old page's click handler would otherwise fall
  through to `selectRoom(hit.room.id)` on an external soffit and throw. Only
  `pickAllAt` returns them, and nothing in the old page calls it.

### B — viewer to React (prerequisite, own plan, as soon as possible)

Parity port, no new behaviour. What this plan needs of it: selection stays one
`{ kind, id, zoneId }` value; inspectors stay a kind → component table. The
port also retires the "trigger for adopting React on a live page is still
unmet" line in `STRATEGY-BROWSER.md`, which the decision made stale.

### C — the features (React viewer), one PR each

- **C1. Pick menu (G2).** A component anchored at the pointer, fed by
  `pickAllAt`. Entry hover → `setHover`, entry click → select. Closes on the
  events in G2.4; clamps to the zone so it never opens off-screen.
- **C2. Inspectors (G1).** One component for ceiling and floor (one record on
  the wire), one for space. Type properties through the existing
  `typePropertiesOf`.
- **C3. Grid → plan (G3).** Row click → `selectRoom(id)`; the row's selected
  class derives from the selection when rows render. When "Pan to room" is
  checked, each zone whose storey holds the room tests the room's bounding box
  against its view and, if not wholly inside, re-centres the view on it. The
  view is page state (the page owns pan/zoom and hands it to `setView`), so
  this needs no renderer change.
- **C4. A selection colour for the three outline layers (G4).** A Rust field
  each on `[appearance] spaces/ceilings/floors` beside their `line`, a control
  on the settings page (a type error there until there is one, which is the
  point of the generated types), `--sel-spaces` / `--sel-ceilings` /
  `--sel-floors` set by `applyAppearance`, and the `.surface-selected-mark`
  modifiers reading them with the accent as the fallback. **After the React
  move, not before**, and last of the four: `cargo test` regenerates
  `src-js/settings/generated/` and the control goes on the settings page, so
  this is settings work either way — but doing it while the viewer is still
  hand-written means writing the viewer half twice.

Every PR: `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, `npm run typecheck && npm test && npm run build`, with the rebuilt
bundle committed.

## Critique

1. **Nothing on the plan says a right click would find more.** Decided: the
   menu is opened on right click (not on every multi-entry left click, which
   would be nearly every click, the room being under every item, door and
   window). The cost is discoverability, and a hint outside the menu is a
   non-goal.
2. **A2 changes what the old page's first click selects, with no switch.** It
   touches every FF&E pick, not only stacked ones: an item nested wholly inside
   a larger one (a worktop inside a joinery unit) now answers first. The menu
   is the way to the larger one, and it only arrives with C1.
3. **Surfaces are never the first pick where a room exists.** The room stays one
   click; ceilings, floors and spaces are reachable only through the menu.
4. **The hover mark is shared by the menu and ordinary plan hover.** A pointer
   resting over the plan while the menu is open must not fight the menu's
   preview; the menu owns hover while it is open.
5. **The list is taken at open time.** A poll landing while the menu is open
   does not rebuild it; an entry whose element has gone selects into the "not
   in the current scope" note (G1.4), rather than the menu shifting under the
   pointer.
6. **Unenclosed spaces cannot be picked** — they have no `loops`. The same
   stated limitation the spaces layer already has.
7. **The panel resolves a surface's rooms by bare id**, the same ambiguity the
   "in this room" section already reports. `SurfaceRoom` carries `model_id`, so
   it is flagged the same way; resolving it needs the deferred `/rooms` change.
8. **A grid row click competes with selecting cell text.** Only a click with no
   text selection selects the room, so copying a value out of the grid still
   works.
9. **A room larger than the view cannot be "wholly inside" it.** Centring it
   still shows its middle, and repeated clicks on that row do not keep moving
   the plan, because the centre does not change. Zoom-to-fit would answer it
   and is a non-goal.
10. **The checkbox resets to checked on every load.** Deliberately not
    remembered — a stated non-goal, not an oversight.
11. **The pick menu needs a pointer with a right button.** Touch and pen have
    no right click, so on those the menu is unreachable and G1's surfaces with
    it. The viewer is a desktop tool; accepted.

## Questions

None open. Q1–Q5 were answered on 2026-09-19 and are folded into the goals and
non-goals above.
