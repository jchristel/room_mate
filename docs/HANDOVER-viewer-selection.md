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
| C3 | Ceiling, floor and space inspectors (G1) | PR #169, CI green when last checked |
| C4 | Room panel's contents chooser (G6) | **next** |
| C5 | Grid row → plan, with pan (G3) | not started |
| C6 | Selection colour for the outline layers (G4) | not started |
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

## What C4 needs, specifically

`RoomContents.tsx` has a `SPECS` table of doors/windows/FF&E, each joined
through `owner_rooms_qualified`. Ceilings and floors join differently — each
surface carries the `rooms` it covers, so the panel inverts that list — and
they default OFF, so the section reads as it does today until asked. The menu
is the same component C1 and C2 use (`fields-panel` styling, `layer-menu`
anchoring).
