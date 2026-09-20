# Plan — viewer selection: layer menus, stacked picks, grid → plan

Eight viewer behaviours. Asked for on 2026-09-19, revised 2026-09-20 — the
revision replaced the right-click pick menu with a per-zone **selection
filter**, folded the eight header toggles into a per-zone **visibility menu**,
and gave the room panel a chooser of its own. G7 and G8 were added the same
day, after C3 shipped: a property chooser on every panel and the grid, and a
per-entity hover property in project settings. This plan is **closed-scope**: it
ends when the six goals below are met, and anything not written under "Goals"
is out of it. Archive to `Superseded/` when it lands.

**Phase A (the plan-side groundwork) and phase B (the viewer's move to React)
are both done.** What is left is phase C, below.

## Goals

Each goal is done when every one of its criteria holds, verified by driving
the page (House A and RHH), not by reading the diff.

### G1 — Ceilings, floors and spaces are selectable and inspected

1. With a layer switched on, a ceiling, floor or space under the pointer can be
   selected (through the pick list, G2). A layer that is off is never
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

### G2 — Stacked objects are listed on a LEFT click, filtered per zone

**Revised 2026-09-20: no right click, and no special mouse behaviour at all.**
What decides whether something can be picked is a per-zone **selection filter**
— the same drop-down shape as G5's visibility menu, listing every object type
with a checkbox.

1. Everything under a point comes back as one ordered list:
   doors → windows → FF&E → room → spaces → ceilings → floors, and **smallest
   first within each layer**. A type contributes only when the zone's layer is
   ON **and** its selection filter is checked.
2. **One match, left click selects it.** More than one, the click opens a pick
   list at the pointer naming each — kind, then name or type name, then id.
   Empty space clears the selection, as it does today.
3. **Hover marks what a click would select**, which is the first entry of the
   same filtered list, so hover and click can never disagree.
4. Hovering an entry in the list marks that element; clicking one selects it
   and closes the list. Escape, a click outside, a pan, a zoom or a storey
   switch closes it and leaves no hover mark behind.
5. The filter defaults to **doors, windows, FF&E and rooms checked, the three
   outline layers unchecked**: the kinds a click could reach before it existed.
   **That does list on a click straight onto a door, window or item**, since
   each sits inside a room and both kinds are ticked — which is the filter's
   job rather than a side effect. A reader who wants one-click FF&E unticks
   Rooms in that zone; one comparing an item against its room leaves both on.
   Clicking bare floor still selects the room, because only the room is there.

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

### G5 — One visibility menu per zone

The eight header toggles — Labels, Rooms, Doors, Windows, FF&E, Spaces,
Ceilings, Floors — are a row of buttons that wraps onto three lines on a narrow
window, and they are **page state**, so two zones cannot be compared with
different layers on.

1. One drop-down per zone, in its toolbar, with a checkbox per layer. The
   button says how many layers are on, so the state is readable without
   opening it.
2. **Per zone**: each zone paints its own set. Two zones on one storey, one
   showing ceilings and one not, is the comparison this makes possible.
3. Each entry keeps its storey-match suffix — "(by elevation)", "(all levels)"
   — computed for THAT zone's storey, because a fallback nobody can see is this
   area's recurring failure.
4. The spaces model picker moves into the menu, beside the Spaces entry, and
   appears only when the project has more than one services model.
5. An element layer is FETCHED while any zone shows it, and not otherwise —
   the read is scope-wide, so it cannot be per zone.

### G6 — The room panel chooses which contents it lists

"In this room" lists doors, windows and FF&E, always, in that order.

1. A drop-down on the room panel, the same shape as G5's, choosing which
   related types the section lists.
2. Ceilings and floors join the options, since each carries the rooms it covers.
   They default OFF, so the panel reads as it does today until asked.
3. **Spaces are not an option**, and the menu says why rather than omitting
   them silently: a space carries no room reference at all, so there is nothing
   to list it by. (The QA report's key match is a different question.)
4. The choice is page state, not per room: it is a question about how the panel
   reads, and a reader clicking room after room is comparing the same thing.

### G7 — Every panel and the grid choose which properties they show

A room carries 45 properties on House A and a reader wants six of them. The
inspector has a name filter and a hide-empty box, which narrow a list nobody
chose; the grid has per-column filters and source toggles, which narrow rows
and groups rather than columns.

1. A **Customize** control on each property panel — room, door, window, item,
   ceiling/floor, space — listing every property that kind offers, with a
   checkbox each and All / None actions.
2. The same control over the grid's COLUMNS, beside its existing source
   toggles: those switch a whole source on and off, this picks within one.
3. **Per KIND, not per element**: a reader clicking door after door is
   comparing the same six fields, and a choice that reset per element would be
   a choice they had to make again every click.
4. Everything checked by default, so an untouched panel reads exactly as it
   does today, and the name filter and hide-empty box keep working over
   whatever survives the chooser.
5. **Not persisted**, like every other view preference here. The durable
   version of "which properties matter" is project settings (`room_label`
   already is one), and that is a different feature — see the critique.

### G8 — A hover shows the property the project chose

Hovering a room shows its name, or its id. Nothing else has a tooltip at all,
and since C2 a hover can land on any of seven kinds.

1. A settings-page section defining, per entity, **which property a hover
   shows** — rooms, doors, windows, FF&E, spaces, ceilings, floors.
2. The viewer reads it with the rest of `[appearance]`'s settings fetch and
   shows that property's value in the tooltip.
3. **Unset is the current behaviour**, not an empty tooltip: name, else id. A
   property the element does not carry falls back the same way, because a
   blank tooltip reads as a broken hover rather than as an absent value.

## Non-goals

Stated so they are not drifted into: **right-click behaviour of any kind**
(dropped in the revision); click-to-cycle and Tab-key cycling; a count badge or
any other "there is more here" hint outside the pick list; a space → room join
anywhere; length units (a separate exercise); zooming to a room, and panning on
any selection that did not come from the grid; switching a zone's storey to
reach a room; remembering the "Pan to room" checkbox, the selection filter or
the visibility menu across reloads; scrolling the grid to a plan selection;
persisting a selection across reloads; multi-select; selection marks in the SVG
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

### B — viewer to React (**done 2026-09-20**, `docs/Superseded/PLAN-viewer-react.md`)

Parity port, no new behaviour. What this plan needed of it held: selection is
one `{ kind, id, zoneId }` value in `src-js/viewer/app/store.ts`, and the
inspectors are a kind → component table (`app/inspector/Inspector.tsx`), so C1
and C2 add entries rather than branches. Two notes for phase C:

- **Spaces, ceilings and floors are already pickable** (phase A) and the panel
  says their inspector is still to come — that placeholder is what C2 replaces.
- **A grid row does NOT select its room.** The port left it out on purpose,
  because adding behaviour inside a parity slice would have made the comparison
  against the old page meaningless. C3 adds it.

### C — the features (React viewer), one PR each, merged before the next starts

Ordered so each step stands on the one before: the visibility menu settles the
per-zone state shape, the selection filter reuses its component, and the
inspectors are what the filter makes reachable.

- **C1. The per-zone visibility menu (G5).** `layers`, `showRooms` and
  `showLabels` move from page state onto `ZoneRow`; one `LayerMenu` component
  replaces the eight header buttons and lives in the zone toolbar. The element
  polls gate on "any zone shows it" rather than one flag. The storey-match
  suffix is computed per zone, and the spaces model picker moves inside.
  The SVG export follows its OWN zone's labels toggle, which it already did.
- **C2. The selection filter and the pick list (G2).** A second menu of the
  same component, per zone, over the seven kinds; `pickAllAt` filtered by it;
  one match selects, several open a list at the pointer. Hover marks the first
  filtered entry, so hover and click agree. `PICK_FIRST` in
  `src-js/renderer/gl/spatial.ts` goes: it was the stand-in for exactly this
  filter, and keeping both would mean two rules for one question.
- **C3. Ceiling, floor and space inspectors (G1).** One component for ceilings
  and floors (one record on the wire), one for spaces, registered in the
  inspector table beside the existing four. Rooms covered, with
  `fraction_of_element` and `fraction_of_room`; "(unattributed)" spelled out.
- **C4. The room panel's contents chooser (G6).** The `ROOM_CONTENTS`-style
  table gains ceilings and floors, each joined through the surface's own
  `rooms` list; a menu (the C1 component again) chooses which types show.
  Spaces appear in the menu as a disabled entry naming why they cannot.
- **C5. Grid → plan (G3).** Row click → `selectRoom(id)`; the row's selected
  class derives from the selection when rows render. With "Pan to room"
  checked, each zone whose storey holds the room tests its bounding box
  against the view and re-centres if it is not wholly inside. The view is page
  state owned by the zone registry, so this needs no renderer change.
- **C6. A selection colour for the three outline layers (G4).** A Rust field
  each on `[appearance] spaces/ceilings/floors`, a control on the settings page
  (a type error there until there is one, which is the point of the generated
  types), `--sel-spaces` / `--sel-ceilings` / `--sel-floors` set by the
  appearance module, and `.surface-selected-mark` reading them with the accent
  as the fallback. Last, because it is settings work either way.
- **C7. The property chooser (G7).** One component over a list of property
  names with All / None, reused by every inspector and by the grid's columns.
  The chosen set lives per KIND in the store, beside the inspector's filter
  state, and defaults to everything — an untouched panel is unchanged.
- **C8. The hover property (G8).** A settings block per entity, a control on
  the settings page, and the tooltip reading it. Last, because it is the same
  settings-plus-generated-types work as C6 and should follow it rather than
  interleave.

Every PR: `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, `npm run typecheck && npm test && npm run build`, with the rebuilt
bundle committed — and driven in the browser on House A and RHH.

## Critique

1. **The list opens on any click straight onto an element, at the defaults.**
   The room is under every door, window and item, so with both ticked a click
   on a chair offers two entries where it used to select the chair. That is the
   cost of dropping the right click, and the filter is what pays it — untick
   Rooms in that zone for one-click FF&E. **Measured on House A**: clicking a
   chair lists "ITEM · FQBS-015" and "ROOM · RAMPUS 00.01". If that reads as
   noise in practice, the cheap answer is a different default (Rooms off), not
   a different rule.
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
11. **Per-zone layers make one comparison possible and another harder.** Two
    zones with different layers is the point, but "is FF&E on?" stops having
    one answer, and a reader looking at the wrong zone's menu will think a
    layer failed to draw. The count on the button is the mitigation.
12. **Per-zone layers do not make the READS per zone.** An element layer is
    fetched while any zone shows it, so switching ceilings on in one zone costs
    the fetch for every storey on screen. That is the storey-scoped read's
    shape and not worth changing; it is stated so nobody reads the menu as a
    cost control.
13. **The room panel's chooser cannot offer spaces**, because a space carries
    no room reference — the one entity here with nothing to join on. A disabled
    entry naming that is better than silence, and it is the same "reported
    state" discipline the rest of this plan follows.
14. **A property chooser is not the durable answer, and should not pretend to
    be.** "Which properties matter for this project" already has a server-side
    home — `room_label`, and the reports page's column sets — and a chooser
    that vanished on reload would be infuriating if it were meant to be that.
    It is not: it is a reading aid for one session, like the name filter beside
    it. If it turns out readers re-make the same choice every morning, that is
    the signal to move it into project settings, not to add a `localStorage`
    key.
15. **The chooser and the filter can contradict each other**, and the panel
    must not hide that. Unticking a property in the chooser while a name
    filter matches it leaves the section emptier than the filter explains; the
    "N of M shown" line is what keeps that honest, and it counts against the
    property list the ELEMENT has, not against the chosen subset.
16. **A hover property per entity is seven more settings fields**, and every
    one of them is a property NAME that may not exist on any element — the
    same class as a colour plan naming a missing property, which greys a room.
    A hover falls back to name-else-id rather than showing nothing, so a typo
    costs a reader the feature rather than the tooltip.
17. **Ceilings and floors in the room panel are a join the page computes**, not
    one the server serves: each surface lists the rooms it covers, so the panel
    inverts that list. It is the same client-side reshuffle as the other three
    types, and it is bounded by the storeys on screen for the same reason.

## Questions

None open.

Q1–Q5 were answered on 2026-09-19 and are folded into the goals and non-goals
above. The 2026-09-20 revision answered three more, and the answers are the
reason this plan changed shape rather than growing:

- **The eight layer toggles are a drop-down, per zone** (G5), not a row of
  page-level buttons.
- **No right click.** A left click selects when one thing is under it and lists
  when several are, and what counts as "under it" is a per-zone selection
  filter (G2).
- **The room panel chooses its own contents** (G6), which is what brought
  ceilings and floors into a section that deliberately did not have them.
