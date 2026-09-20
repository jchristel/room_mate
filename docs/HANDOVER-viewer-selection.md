# Handover — the viewer's selection work (phase C)

Written 2026-09-20, mid-flight, because the session that had the context ran
out of it. **Read `PLAN-viewer-selection.md` first** — this file says only
where the work got to, what the plan does not, and what is worth not
rediscovering.

## Where it got to

Phase A (renderer groundwork) and phase B (the viewer's move to React) are
**done and merged**; B is archived as `Superseded/PLAN-viewer-react.md`. Phase
C is eight steps, each its own PR, merged before the next starts.

| Step | What | State |
|---|---|---|
| C1 | Per-zone visibility menu (G5) | merged, #167 |
| C2 | Per-zone selection filter + pick list (G2) | merged, #168 |
| C3 | Ceiling, floor and space inspectors (G1) | merged, #169 |
| C4 | Room panel's contents chooser (G6) | merged, #171 |
| C5 | Grid row → plan, with pan (G3) | merged, #172 |
| C6 | Selection colour for the outline layers (G4) | **next** |
| C7 | Property chooser on every panel and the grid (G7) | not started |
| C8 | Hover property per entity, from settings (G8) | not started |

## How this work is run

The user asked for: **implement a step, open a PR, merge it, then start the
next.** Every PR runs `cargo test` / `cargo fmt --check` / `cargo clippy
--all-targets -- -D warnings` and `npm run typecheck && npm test && npm run
build`, with the rebuilt output committed.

**Drive the page, do not read the diff.** Every bug worth having found in this
work was found that way and would have passed review: a picker reverted by an
in-flight poll, a plan drawn many times its panel height, a grid with all 3,013
RHH rows in the DOM, element layers that never repainted, an adjacency canvas
sized 300×150. Three of those are one lesson — **measure after layout, then
keep measuring with a `ResizeObserver`.**

House A is the quick check; **RHH is the honest one** (3,013 rooms, ten models,
storey-scoped reads, slow on a debug build — allow 15 s after a project switch
and ~20 s after switching an overlay layer on).

## Things that cost time to learn

- **`roommate` on the console** is the page's read-only handle:
  `getState()`, `zones()` (each with `.renderer.debugState()`),
  `elementsOnStorey(entity, levelId)`, `layerPayload(entity)`. It exists
  because a React page has no globals and this work is verified by driving it.
- **Driving React controls from the console** needs the native value setter:
  `Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), "value").set
  .call(el, v)` then `dispatchEvent(new Event("change", {bubbles:true}))`. A
  plain `el.value = v` does not reach React. Menu labels contain a leading
  space, so match with `includes`, not `startsWith`.
- **`setPointerCapture` throws on a synthetic pointer event**, which silently
  aborts a drag handler. Drag handles must be tested with a real pointer.
- **The browser pane scales**: convert with `x * 800 / innerWidth` before
  clicking by coordinate, or read the element's own rect.
- **Two viewer branches will conflict on the built output** (`static/viewer.js`
  and friends are committed). Rebase, rebuild, commit — do not hand-merge.
- **CI only runs for PRs targeting `main`.** A stacked PR gets no checks until
  its base merges, and retargeting alone does not trigger them: rebase and
  force-push, or close and reopen the PR.

## Decisions already taken, with their reasons

- **No right click, anywhere.** A left click selects when one ticked kind is
  under it and lists them when several are. **With the defaults this lists on
  most clicks straight onto an element**, because a door, window or item sits
  inside a room and both kinds are ticked. That is the filter's job; unticking
  Rooms in a zone gives one-click FF&E. The user has been told and left it as
  specified — if it reads as noise, change the DEFAULT, not the rule.
- **Layers are per zone; the READS are not.** An element layer is fetched while
  any zone shows it. The menu is a drawing decision, never a cost control.
- **The spaces read carries no `?model=`.** The model is a per-zone choice and
  one request cannot serve two zones, so filtering happens at paint.
- **`PICK_FIRST` is gone from the renderer.** It was a hard-coded stand-in for
  the selection filter, and two rules for one question is one too many.
- **Spaces cannot be listed in the room panel** (C4): a space carries no room
  reference at all. The menu shows them disabled, naming why.
- **Nothing here persists across reloads** — not the filter, the menus, the
  chooser or "Pan to room". The durable version of "which properties matter"
  is project settings, which is a different feature (see the plan's critique).

## What C4 left behind

- **The three menus are one component now** (`app/DropMenu.tsx`): a button with
  its own summary, a panel of arbitrary children, and the outside-click close.
  C1 and C2 were moved onto it rather than a third copy being written.
- **Opening one menu closes the others**, and the reason is worth keeping: the
  button calls `stopPropagation` so a click on it is not also a plan click,
  which stops the NATIVE event at React's root container — so it never reaches
  the `document` listener another menu is waiting on. `DropMenu` keeps a set of
  open menus' closers instead. That bug shipped in C1/C2 and was only visible
  with Layers and Select open together.
- **`layerWanted` has two askers now.** A zone DRAWING a layer, and the room
  panel LISTING it: ticking Ceilings in the chooser while no zone draws them is
  what fetches `/ceilings`, or the section would read "not loaded yet" for ever.
  Pinned in `app/store.test.ts` — the one rule here that costs a 9 MB read per
  viewer if it regresses.
- **This local store's House A has no ceilings and no floors** (`/ceilings?
  project=House A` is a 200 with an empty list), so the surface join can only be
  driven on RHH. LEVEL 6 is a good one: 165 ceilings, 191 floors, and rooms with
  two ceilings at 37% and 60%.
- **A surface row shows its share of the ROOM, not its id.** `fraction_of_room`
  is what this join knows that the opening join does not, and the id is one
  click away on the surface's own panel. Sliver overlaps are listed, as the
  server intends — a floor at 0.5% of a room is a strip under a wall, and which
  overlaps matter is the reader's policy.

## What C5 left behind

- **The pan rule is `viewer/panTo.ts`, and it is pure so it can be tested.**
  `viewCentredOn(view, target)` returns `null` for "already wholly inside",
  which is the half that matters: clicking down a department keeps the plan
  still for the rooms on screen. It also returns `null` for an already-centred
  room LARGER than the view, which can never pass the containment test — a
  second click on such a row would otherwise re-commit an identical rect.
- **Copying out of the grid is protected by POINTER DISTANCE, not by
  `window.getSelection()`.** The selection test looks like the direct one and
  is not: the press starting the next click has collapsed the selection in the
  DOM but not always by the time the handler runs, so the first click after a
  copy was swallowed. Measured on House A, then replaced with a `mousedown`
  position compared against the `click` (`DRAG_SLOP`, 4 px).
- **`panToRoom` honours linked views by deciding ONCE.** Per zone is the plan's
  rule and it is right unlinked; linked, one rect serves every zone, so
  deciding per zone would have the first commit broadcast and the next zone's
  answer fight it. Linked, the first zone showing the room decides and the loop
  returns.
- **The grid's selected row is derived as rows render**, never written onto a
  node — the table is windowed, so a row that scrolls out and back is a
  different element. It is also what makes a PLAN click mark its grid row for
  free. `#gridTable tbody tr.target` was dead CSS from the old page and is now
  `tr.selected`.
- Driven on House A (one zone, two zones on different storeys, linked and not)
  and on RHH (3,013 rows: the mark survives scrolling 40,000 px away and back;
  a plan click marks the row at index ~2,140).

## What C6 needs, specifically

`[appearance] spaces`, `ceilings` and `floors` each gain a `selection` colour
beside their existing `line`, with a control on the settings page — a type
error there until there is one, which is the point of the generated types.
`--sel-spaces` / `--sel-ceilings` / `--sel-floors` are set by the appearance
module and `.surface-selected-mark` reads them, falling back to the accent so a
project that sets nothing looks exactly as it does today. Remember `cargo test`
rewrites `src-js/settings/generated/` and those files are committed.
