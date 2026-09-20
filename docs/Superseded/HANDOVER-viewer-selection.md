# Handover — the viewer's selection work (phase C)

**Phase C is complete: all eight steps are merged, and this file is the
record.** It was written 2026-09-20, mid-flight, because the session that had
the context ran out of it; the notes below are what each step cost to learn.
The plan it served is beside it in this directory. **Nothing here is live** —
what shipped is documented by the code.

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
| C6 | Selection colour for the outline layers (G4) | merged, #173 |
| C7 | Property chooser on every panel and the grid (G7) | merged, #174 |
| C8 | Hover property per entity, from settings (G8) | merged, #175 |

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

## What C6 left behind

- **No renderer change was needed.** `#ring` already tags the mark with a
  `spaces` / `ceilings` / `floors` modifier class, exactly as it does for
  doors; C6 is three CSS rules, three `set()` calls and one Rust field.
- **Three variables, not one for the class the three share.** A ceiling and the
  floor under it are routinely selected in turn, and one colour for both would
  make the two marks indistinguishable at the moment a reader is comparing
  them. The pick list's hover preview follows the selection colour, which is
  the rule doors/windows/FF&E already use.
- **The Rust doc said "neither filled nor pickable", and half of it had gone
  stale.** C2 gave these layers a pick index; the type now records that
  `selection` arrived because the viewer grew and `fill` stays absent for a
  structural reason (the layer is an outline BECAUSE the comparison with the
  room beneath is the question it exists to answer).
- **The Appearance hint no longer counts its controls.** It said "pin all
  fifteen" against sixteen; it now says "every one of them", because a count in
  prose beside a grid that grows is a thing that drifts twice.
- **Driving this needs your OWN server, and the port is the obstacle.** Another
  session's `roommate` holds port 5151 AND `target/debug/roommate.exe`, so a
  second one fails to link with "Access is denied (os error 5)". A temporary
  `.claude/launch.json` entry with `--port 5153` and `--target-dir target/c6`
  works; expect a full cold compile (~6 min) and delete both afterwards.
- **House A holds no spaces, ceilings or floors in this store** (all three read
  200 with an empty list), so anything about the surface layers has to be
  driven on RHH. LEVEL 6 again.
- Verified end to end: saved through the settings page into
  `settings/projects/*.toml`, served by `/api/settings/resolve`, applied as
  `--sel-*`, and read by the mark — ceiling `rgb(0,160,255)`, floor
  `rgb(0,192,96)`, space `rgb(255,0,192)`, hover preview matching. Reset back
  to "theme" and the mark returned to `rgb(180,84,31)`, the accent.

## What C7 left behind

- **The store records what is turned OFF, not what is chosen**, per scope. A
  set of chosen names is a snapshot: the next door of the same kind carrying
  one extra property would have it silently absent. `keepChosen` holds the
  reasoning and the test; "All" writes an EMPTY set rather than every name, and
  that is what keeps a later arrival on.
- **A panel's chooser works on property NAMES; the grid's works on COLUMNS.**
  On a panel a name carried by both the model and a joined source is one entry,
  which is exactly the granularity of the name filter beside it. In the grid
  two sources' columns are two columns, so entries are keyed by column key and
  grouped under a source heading.
- **`Filters` is on every panel now.** It was room-and-space only, on the
  grounds that "an opening's two short tiers have nothing to filter" — stale
  since the element panels grew full instance and type sections, which had
  been applying hide-empty invisibly ever since. A door offers 162 names.
- **Every inspector drop-down anchors RIGHT**, and the rule is now on
  `#inspector .fields-panel` rather than on C4's one menu — the chooser first
  shipped with half its list off the edge of the window, because the panel is a
  22rem column pinned to that edge. It is also capped at 21rem wide:
  `VisionPanel_FullyGlazed_Height` does not wrap.
- Driven on House A (grid columns, room panel, door panel) and RHH LEVEL 6
  (ceiling 64 names, space 67). Checked: the room's hidden set does not reach
  the door's; unticking `Workset` on a door drops it from BOTH tiers, where it
  appears once in the menu; the "N of M shown" line moved 22 → 18 of **45**
  when four were hidden, so it still counts against what the room carries;
  a name filter of "fire" and an unticked `Fire Rating` compose rather than
  fight.

## What C8 left behind

- **`[hover]` is one flat struct where `Appearance` needed three.** Appearance
  is shaped by what each entity DRAWS, which genuinely differs; every entity
  carries properties, so every entity can answer this question, and splitting
  it would have been shape for its own sake.
- **The tier rule is `tieredValue` in `viewer/properties.ts`, with tests.** It
  is the contract's `lookup_property` rule restated for the page — instance,
  then type, and a tier only wins when it holds something — and it shares
  `isEmptyPropValue` with hide-empty, so Revit's literal `"None"` counts as
  absent in both tiers. Measured on RHH: `Fire Rating` is absent on a door's
  instance and `"None"` on its type, so that door falls back to its type name;
  `Classification Number` is absent on the instance and real on the type, and
  the tooltip reads `23.30.10.00`.
- **Every kind has a tooltip now**, where only rooms did. Six of the seven a
  hover can land on since C2 answered with nothing at all.
- **Free text, no datalist, and that is a statement about the server.** It
  knows the ROOM property vocabulary — that is what the room-label picker uses
  — and knows nothing about a door's or a ceiling's, so suggesting room names
  against every row would offer names that cannot work.
- Driven on RHH LEVEL 6: room → `Inpatient Unit #1 - Medical Short Stay (28
  Beds)`, ceiling → `2700.0`, door → `23.30.10.00`, and FF&E (nothing set) →
  `Generic Models · FIRT-032`, which is the unchanged name-else-id fallback.

## What is left

Nothing in this plan. Two things it deliberately did not do, stated here so
they are found rather than rediscovered:

- **None of these preferences survive a reload** — not the visibility menu, the
  selection filter, the contents chooser, "Pan to room" or the property
  chooser. The durable version of "which properties matter for this project" is
  project settings, and the plan's critique 14 says what would justify moving
  one there: readers re-making the same choice every morning, not a
  `localStorage` key.
- **No overlay layer reaches the SVG export**, selection marks included. Also
  pre-existing, also stated in CLAUDE.md.
