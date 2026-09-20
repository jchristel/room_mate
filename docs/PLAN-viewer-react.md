# Plan — the viewer moves to React

`static/index.html` is 5,568 lines of hand-written HTML, CSS and JavaScript:
182 top-level functions over one graph of module-scope globals. The settings
page (2026-09-12) and the reports page are React; this is the last page, and
the one the [selection plan](PLAN-viewer-selection.md) is waiting on — its
phase C is written once, here.

**A PARITY PORT. No new behaviour, no redesign, nothing renamed on screen.**
That is the whole discipline: the only way to check a 5,568-line rewrite is to
run the old page beside the new one and see the same thing. A feature added on
the way makes a difference impossible to classify, which is exactly how the
settings page lost every milestone's pins on save.

## Goals

1. `/` serves a React viewer that does **everything the current page does**,
   checked area by area against the old page still running beside it.
2. The old page is **deleted**, not kept as a fallback, and
   `STRATEGY-BROWSER.md`'s "the trigger for adopting React on a live page is
   still unmet" line goes with it.
3. The logic that is not React — colour plans, CSV, grid rows, search matching,
   storey resolution, area maths — comes out as **tested TypeScript modules**
   under `src-js/viewer/`, not as component bodies.
4. `npm run build` emits a committed bundle, and `.github/workflows/frontend.yml`
   rebuilds and fails on drift, exactly as it does for the other two pages.

## Non-goals

No new features (they are the selection plan's phase C, after this);
no visual redesign or restyle; no router; no state library; no CSS framework or
CSS-in-JS; no change to any server route, response or setting; no port of
`graph.js` (the adjacency canvas is wrapped, not rewritten); no change to
`src-js/renderer/`, which is already TypeScript and already behind a seam; no
performance work beyond keeping what the page has.

## Shape

- **The new page lives at `/viewer/` until it is finished**, built by
  `vite.viewer.config.ts` into `static/viewer/`, the reports pattern verbatim.
  Both pages run in the same server, so every slice is checked by opening the
  two side by side on the same project. **Cutover is the last PR**:
  `static/index.html` becomes a thin shell that loads the new bundle, and the
  old page is deleted in that same commit.
- **State lives in one store module plus React context**, not a library. What
  the page has today is module-scope globals and explicit listeners
  (`selectionListeners`); the port keeps that shape and makes it typed.
- **The CSS moves once, whole.** `static/index.html`'s `<style>` becomes
  `src-js/viewer/app/viewer.css`, edited only where a selector must follow a
  renamed hook. Same tokens, same look; a restyle would destroy the comparison.
- **`src-js/renderer/` is untouched.** The React host mounts a canvas and an
  SVG overlay and drives `PlanRenderer` through the seam it already has.

## Slices, in order, one PR each

Each slice ports its area to modules plus components, keeps the old page
working, and is checked against it. A slice with pure logic lands its tests in
the same PR.

1. **B1 — harness.** `vite.viewer.config.ts`, `src-js/viewer/app/`, an
   `index.html` shell at `static/viewer/`, the CSS moved whole, the page
   header/footer chrome, and the build wired into `npm run build` and
   `frontend.yml`. Renders an empty shell that says which project is selected.
2. **B2 — scope and data.** Project, building, milestone, `?` params,
   `localStorage`/URL restore, the rooms read and `ingestAll`, the
   `EntityPoll` layers (already a module), the tick. No plan yet: the shell
   reports counts and revisions.
3. **B3 — zones and the plan.** Zone creation and removal, layout, the
   renderer host, pan/zoom, view sync across zones, the level control, storey
   resolution (`storey.ts` is already a module), empty-level states.
4. **B4 — layers and appearance.** The layer toggles and their suffixes, the
   spaces model picker, colour plans, `[appearance]`, labels, rooms-off.
5. **B5 — selection and inspectors.** Selection state and listeners, the
   marks, hover and the tooltip, all five inspectors and the room contents
   panel. This is the slice the selection plan's C1/C2 build on.
6. **B6 — the grid.** Columns, filters, sort, the virtualised body, reference
   sources, the error index, CSV. The heaviest slice; `computeGridRows` and
   the CSV builder come out as tested modules.
7. **B7 — search and fields.** Matching, the fields panel, highlight/dim.
8. **B8 — the bands.** QA/validation, areas (overlay, band, CSV) and
   adjacency (wrapping `graph.js`), plus the band drags and splits.
9. **B9 — SVG export.** `paintLevel` is shared with the renderer's SVG
   painter; the export is a module with its golden test kept.
10. **B10 — cutover.** `/` serves the new page, `static/index.html` is deleted,
    `STRATEGY-BROWSER.md` updated, and the selection plan's phase B is marked
    done.

## Critique

1. **Nine slices of "not finished yet" is the real risk, not any one slice.**
   The page at `/` keeps working throughout, which is the mitigation, but the
   selection features the user actually asked for are behind all of it. A
   shorter road exists and was rejected: building C1–C3 in the old page first.
   That is the user's call, taken on 2026-09-19 ("no, do not duplicate").
2. **"Parity" has no test.** There is no golden for a page; the check is a
   human comparing two windows, and a rarely-used control (the region drag, the
   band split, `?building=` with a homeless element) is exactly what such a
   comparison misses. The slice list is ordered so the least-used things
   (B8, B9) land last, when the reviewer is most alert to them — not first,
   when the shell is new.
3. **The store is the decision most likely to be regretted.** Globals plus
   listeners port directly and read like the current page; a `useSyncExternal
   Store` per slice is a bigger change than it looks once zones hold their own
   view rects. If a slice starts fighting it, that is the signal to stop and
   write the state model down, not to reach for a library.
4. **`graph.js` is wrapped, not ported, and it is the one file that will look
   wrong afterwards** — a hand-written canvas beside a React page. Porting it
   is a second project (`docs/Superseded/HANDOVER-adjacency.md` is its
   history); wrapping it costs a `useEffect` and an imperative handle.
5. **The bundle is committed, and the viewer's is the biggest yet.** The
   frontend CI gate rebuilds and compares, so a forgotten `npm run build` is a
   red PR rather than a stale page — but the renderer bundle and the viewer
   bundle are now two committed artifacts that must both be rebuilt.
6. **RHH is the only honest test subject for B3 and B6** (3,013 rooms, 10
   models, storey-scoped reads) and it is slow on a debug build. Slices that
   touch paint or the grid are checked there as well as on House A, which is
   the cost of not shipping a viewer that is fine on 18 rooms.

## Questions

- **Q1.** `/viewer/` during the port, cutting over at the end (assumed), or
  replace `/` from B1 and accept that the page is half-ported for weeks?
- **Q2.** Is wrapping `graph.js` acceptable, or should the adjacency canvas be
  ported too (a bigger B8, and a second canvas renderer in React)?
- **Q3.** Slice order: B5 (selection and inspectors) is what the selection
  plan needs. Bring it forward, ahead of B4 and B6, so phase C can start
  sooner — or keep the order above, which finishes the plan before adding to
  it?
