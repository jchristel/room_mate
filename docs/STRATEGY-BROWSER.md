# Roommate — Browser

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[MCP](STRATEGY-MCP.md) · [Authored](STRATEGY-AUTHORED.md) ·
[Entities](STRATEGY-ENTITIES.md) · [Security](STRATEGY-SECURITY.md)

The SVG viewer: how it renders, how it's expected to grow, and how the fetch
side should shape future server endpoints.

## Implemented

- **SVG floor-plan rendering.** Draws room outlines per level from the
  `/rooms` payload.
- **Global scope pickers: project, milestone, building — the single-project
  viewer.** One set of header `<select>`s carrying the scope of the whole
  page (see [Server](STRATEGY-SERVER.md)'s `/projects` /
  `/projects/{id}/buildings` / `/projects/{id}/milestones`); the level picker
  is per-zone (see the zones bullet below — zones differ in level and colour,
  never data). Scope was per-zone for a while under the multizone viewer;
  HANDOVER-ui-layout.md Decision 1 deliberately reversed that: cross-project
  side-by-side display was an emergent capability nobody used, and making
  scope global deleted the entire focus model (no `activeZoneId`, no
  active-zone border, no "whose scope does the URL persist" question) and
  collapsed polling, the colour-plan read, and validation state to one each.
  **Poll once, fan out:** each 2s tick issues one scoped `/rooms` fetch
  against one revision cursor and distributes the payload to every zone —
  deleting the bug class where two zones on the same project could drift a
  tick apart. A future multi-project comparator gets its own page (the
  `comparison.html` precedent), not a mode flag here.
  Building auto-hides when it has ≤1 option and auto-selects when
  there's exactly one real choice, so the common single-building dev case
  shows no picker at all. **Project is the exception: it's shown whenever any
  project exists, single option included** (hidden only at zero, where there's
  no name to state). Which project you're looking at isn't a choice, it's the
  scope of everything on screen — and the ≤1 rule left that anonymous exactly
  when it's least obvious: several projects registered but only one holding
  rooms is the normal state while the others are onboarded, since `/projects`
  lists only projects with stored snapshots. The level picker lists floors
  highest-elevation-first (a
  `<select>` has no CSS-driven reversal the old button stack relied on, so
  it's sorted explicitly at render time). A building option the server flags
  `ambiguous` (another building shares its name — legitimate, since buildings
  are distinct by `(code, name)`) renders as "Name (CODE)" so the two options
  are distinguishable; a same-name entry with no code stays the bare name,
  its code-bearing twin carrying the visible distinction.
- **Scoped polling — once per page.** `poll()` builds `/rooms`'s URL from the
  global project/building/milestone selection every tick and fans the payload
  out to every zone (`ingestAll`); the scope pickers refresh on the same 2s
  cadence (gated by a shallow id-list diff so they don't fight an in-progress
  selection), which is also how a newly-pushed project shows up without a
  page reload.
- **Doors are served but not drawn — deliberately, and the fetch decision is
  already made.** `/doors` ships with footprints, both room references and the
  full property set, and nothing in the viewer reads it. That is scope, not an
  oversight: doors landed as a data pipeline first, so the server side could be
  verified against real geometry before any pixel depended on it.

  When a viewer is built, **doors get their own fetch, not a ride on the 2s room
  poll** — the `/adjacency` precedent above, and for the stronger of its two
  reasons. `/adjacency` is on-demand because it is expensive; doors are separate
  because they are *independently versioned and independently pushed*: a doors
  snapshot has its own `taken_at`, its own milestone pins
  (`door_attachments`), and its own `revision`, so folding it into the room
  poll's payload would make one revision stand for two lineages that move apart.
  The `revision` field on `/doors` exists for exactly the same "has anything
  actually changed?" comparison `/rooms` uses, so a second poll costs a string
  compare, not a re-render.

  One thing a renderer will need that the server already provides and a reader
  might not expect: **every door carries its `model_id`**, because room ids are
  unique only within a model. Resolving a door to the rooms it connects means
  matching within the same model — a viewer that flattens door references the
  way `/rooms` flattens room ids would silently link the wrong rooms.
- **Room labels: configurable, always-rendered, correctly layered.** `addLabel`
  renders `room.label` (the server-resolved, ordered field list — see
  [Server](STRATEGY-SERVER.md)'s `room_label` setting) instead of hardcoding
  `room.name`/`room.id`; the first field is the large primary line, any
  further fields stack below as smaller accent-colored lines, generalizing
  the old fixed two-line layout to however many fields are configured. Two
  bugs fixed alongside this: (1) labels no longer silently disappear on small
  rooms — the old `fontSize < baseFont * 0.25` cutoff (a floor-wide threshold
  that dropped a label outright rather than just shrinking it) is gone; a
  label now always renders clamped to fit its own room, however small, and
  zoom can't recover a dropped label anyway since panning/zooming never
  re-invokes rendering. (2) `renderLevel` now appends every room's polygons
  in one pass, then every room's labels in a second pass — SVG has no
  reliable z-index (paint order is DOM order, full stop), so the old
  per-room interleaved loop let a later room's opaque polygon paint over an
  earlier room's label whenever their screen-space boxes were anywhere
  close, which got worse on bigger plans with more rooms.
- **Bottom region, band 1: results.** Tabular output lives in a page-level
  region below the plans, not in overlays covering them (HANDOVER-ui-layout
  Decision 2). It holds one block per computed result — QA and hierarchy
  areas — each collapsed to a one-line summary strip by default (`▸ QA · ⚠ 3`)
  and expanding on click: a fully hidden band gets forgotten, an always-open
  one eats plan area. **One instance per page, never per zone** — both blocks
  are scope-derived, and a region that multiplied with zone count would stop
  being a stable place users can point at. The region carries **one height
  budget** that expanded blocks divide between them, each scrolling
  internally, so a long mismatch list can never squeeze its sibling to
  nothing. **Expanding a band-1 block takes space from the grid, never from
  the plans** (measured: plans unchanged while the grid yields the
  difference); only an explicit drag resizes the plans. An empty region
  reserves no height at all.
  **Two drag handles, because there are two independent questions.** The
  region's top edge sets its *total* height against the plans; a divider
  between band 1 and band 2 splits that total between the results and the
  table, leaving the plans untouched. The divider appears only when a band-1
  block is actually expanded — there is nothing to redistribute between a
  collapsed summary strip and the grid — and releases any dragged height when
  the last block collapses. Band 1 is capped at "the region minus the grid's
  own minimum" (its toolbar, sticky header and two rows, measured rather than
  guessed), which is what stops a tall results table starving the grid; a
  percentage cap can't express that, since the floor depends on the grid's
  own chrome.
- **Bottom region, band 2: the source-data grid.** A read-only table over the
  rooms of the current scope — **every level, not the levels on screen**.
  That is deliberate and worth restating because a level filter feels like it
  "should" follow the plans: with several zones on different levels there is
  no single answer, and it would make the grid's contents depend on
  presentation state, reintroducing exactly the scope/presentation blur the
  global-scope migration removed. Filtering by level stays available as an
  ordinary per-column filter, driven by the user.
  Model-derived and dRofus-derived columns are **grouped and tinted, never
  interleaved** — [Sources](STRATEGY-SOURCES.md) keeps dRofus a distinct
  sub-object ("store raw, join late") precisely because the two have
  different lifecycles, and a flat table would imply a single source of truth
  the data model deliberately doesn't have. A per-source toggle shows either
  or both. Reference columns come from the response's own per-source
  `reference_labels` set, **not** a union of the rooms' joined fields, so a
  column that matched no room in scope still appears (and one with no Revit counterpart in row 2 of
  the CSV renders visibly *unmapped* rather than being hidden — the same
  honesty the coverage report applies). An unmatched room simply has empty
  dRofus cells, never an error.
  **Row windowing from the start**, because the row count is the design
  constraint: `big-plate` is 5,046 rooms on a *single* level and the grid
  spans every level in scope. Only the visible slice is ever in the DOM, with
  two spacer rows standing in for the rest of the scroll height — measured at
  **28 rows in the DOM against 10,092 rows of data**. This is the
  one-dimensional case of `paintLevel`'s cull units (precompute each item's
  extent, render only what is in view) rather than a second pattern invented
  for the same problem; `table-layout: fixed` is load-bearing, since
  content-derived widths would jitter as rows swap in and out.
  Sorting (numeric when both sides parse) and per-column filters are
  client-side; the header is rebuilt only when the *column set* changes, so a
  filter keystroke never detaches the input being typed into. CSV export
  follows the existing precedent — visible columns, every filtered row,
  client-side, no server endpoint.
  **The two bands inform each other.** `compute_validation` keys its
  field-level findings by (room, field), which in this grid *is* a (row,
  column) address — so a mismatch is marked on the disagreeing cell in place,
  not only listed above; room-level findings (no link value, unmatched,
  duplicate) have no column and mark the Id cell instead. Clicking a band-1
  entry scrolls band 2 to that room rather than repeating its values.
  **Stacked, not side by side — settled with real data**, as the handover
  asked. On a 50-column project the grid needs ~4,700px and has ~1,350px;
  giving band 1 a 30% left column would cut visible columns from 10 to 7,
  while the areas table's 544px minimum wouldn't fit the column it was given
  — two horizontal scrollbars and a worse grid. Band 2 is the scarcer
  resource, and stacking protects it.
- **Data validation in band 1: summary strip, highlighting, CSV export.** The
  block's collapsed strip carries what the old header badge did (`⚠ N`, `✓`,
  hidden entirely when dRofus isn't configured); expanding it lists
  [Server](STRATEGY-SERVER.md)'s six dRofus health checks
  (missing/duplicate link values, unmatched-in-dRofus, property mismatches,
  and the two Revit-side presence checks, `fields_absent_in_revit` /
  `fields_empty_in_revit`), plus an always-shown, non-error **field
  coverage** section (which dRofus columns this pass actually checks, and
  against which Revit property). Coverage is built and rendered separately
  from the issue sections specifically so it survives the "No issues found"
  collapse instead of disappearing with it, and it stays out of the badge
  count — it's a config reference, not a data-quality problem. Fetched only
  when the project selection changes or via the block's own Refresh button —
  deliberately not on the 2s room poll, since this is an on-demand check, not
  something to watch update live. Two things layered on top, both entirely
  client-side: (1) rooms with any issue (across all six checks) get a
  distinct fill (`.room.error`, a new `--error` CSS variable) *only while the
  block is expanded* — `showErrors` is page state tracking that expansion
  (it was per-zone panel visibility before the region existed) and
  triggers a `refit: false` re-render, so opening/closing it never disturbs
  the current pan/zoom. (2) A "Download CSV" button builds a `room_id,error`
  CSV directly from the already-fetched report (one row per issue, so a room
  with several issues appears several times) and triggers a browser download
  — no server endpoint for this, matching "keep axum a pure JSON API": a CSV
  is just a presentation reshuffle of data the browser already has.
- **Milestone picker.** A header `<select>` sitting immediately after project,
  ahead of building: "Latest"
  (the default, no filter) plus one option per milestone from
  `GET /projects/{id}/milestones`, labelled `name (date)`. Refreshes on the
  same 2s cadence as the project/building pickers, gated by its own
  signature diff; hidden when the project defines no milestones — unlike the
  other pickers' `<=1` rule, "Latest" alone is not a choice worth a picker.
  Selecting one adds `milestone=` to the `/rooms` poll URL (only alongside
  its project — a milestone is a per-project name); a selected milestone
  that disappears (deleted/renamed in settings) falls back to Latest rather
  than keeping a filter the server would answer with nothing. The validation
  badge stays latest-based regardless of the milestone selection (see
  [Server](STRATEGY-SERVER.md)).
- **Colour plans: client-side room colouring, a picker + persisted config.**
  A fifth header `<select>` — "No colour" (flat, the default) plus one option
  per plan from the project's `settings.colour_plans` — lets the viewer colour
  rooms by a per-project, user-authored rule. **All colour math is client-side**
  (`colourForRoom` in `index.html`); the server stores `colour_plans` verbatim
  and computes nothing — the same "axum stays a pure JSON API" line that keeps
  CSV export and QA rendering out of the server. This is also *why* the viewer
  makes its one read of `/api/settings/projects/{id}` here (on project change,
  not every tick — re-fetching would fight the picker): colour plans have no
  other delivery channel, and reusing the settings read endpoint adds zero
  server surface. `ColourPlan.active` sets the picker's default; "No colour"
  always overrides, so it's a default, not a forced application. Palettes are a
  hand-picked JS constant of ColorBrewer schemes in `common.js`, sampled
  piecewise-linearly. (Originally "no d3/npm, the page stays zero-build"; npm
  now exists, but the constant stays — it is eight literal arrays shared with
  `graph.js`, and a dependency would buy nothing.) Fill is applied as an
  inline `style.fill` (a `fill` *presentation attribute* loses to the `.room`
  CSS rule; inline style wins), precedence selected-plan > error highlight >
  default `--fill`; "No colour" leaves the class fill untouched, preserving
  today's look and hover. A room the plan can't colour — missing/unparseable
  property, ratio-by-zero, a value in a gap between bands — renders a "no data"
  grey, never an error. **All three modes are wired:**
  - *property compare* — compare two room properties (`A op B`) → match /
    diverging / bands. The number→colour step (`Colouring`) is kept separate
    from the number-derivation step so a future milestone-compare mode (same
    property across two snapshots) reuses match/diverging/bands untouched.
  - *hierarchy* — categorical hue per parent tier, tint/shade per child. Reads
    each room's server-resolved `classification` path (already on the payload —
    no client re-derivation): `tiers[0]` → a distinct qualitative hue per value
    (Set2/Paired), `tiers[1]` → a lightened tint of it. An undefined parent
    tier → grey.
  - *date-range* — proximity of a date-typed property to `near_date`: after it
    → a fixed blue, at/before → green (near) to red (far), auto-scaled to the
    level's furthest past. Dates parse by an optional strftime `format` — the
    *same* pattern the dRofus date column uses, since Revit room dates originate
    from dRofus (the editor pre-fills it from the project's `date`-typed
    `drofus_fields`) — falling back to native ISO-8601 when omitted; an
    unparseable value → grey.
- **SVG export (per-zone, one file per level).** An "Export SVGs" button on each
  zone's toolbar saves that zone's whole building — one standalone `.svg` per
  level — with no server endpoint, the same "presentation reshuffle of data the
  browser already has" line that kept CSV export and QA/colour rendering
  client-side: the browser already holds the rendered SVG, so exporting is just
  serializing DOM. A second consumer of the render path — `renderLevel` was split
  into a pure `paintLevel(svg, rooms, fitted, …)` painter (reads no zone/global
  state) plus the on-screen `renderLevel` wrapper, so the export and the live view
  can't drift. Each file is fully self-contained: framed to that level's *fitted*
  bounds (not the user's pan/zoom), an embedded `<style>` whose colours are read
  live from the resolved `:root` custom properties (so it never diverges from the
  page and needs no `tokens.css`), an opaque paper background `<rect>`, `xmlns`,
  and an explicit `viewBox`/`width`/`height` — it opens correctly in a bare
  browser tab and in Illustrator/Inkscape. Error highlighting follows the
  validation panel (`showErrors`), which is only on once validation is loaded, so
  an exported "errors" file is never a silent empty highlight; coloured rooms
  carry their resolved fill inline and survive serialization. Filenames are
  self-describing (`roomplan_<project>_<level>[_<milestone>][_errors].svg`). One
  click emits N downloads (browsers prompt once to allow multiple), staggered to
  avoid throttling. All-levels-in-one-file and raster (PNG/PDF) export remain out
  of scope.
- **Room search (header input + field picker).** One free-text query applied
  to every zone, with a "Fields ▾" checklist choosing which fields it scans:
  the `$name`/`$id` intrinsics, `$classification` (any tier's name or code),
  `$drofus` (any joined field's value), plus the union of raw property keys
  across all zones' loaded rooms (a genuinely new field defaults to on).
  Matching is a case-insensitive substring over enabled fields — the
  free-text-for-a-human matcher, deliberately distinct from the server's
  structured `?filter=` grammar for machines (see
  [Server](STRATEGY-SERVER.md)). Entirely client-side and render-free on the
  fast path: matches toggle `.match` (accent outline) and non-matches `.dim`
  on the already-rendered nodes via the cull-unit room→nodes map, debounced,
  so typing never re-paints the plan; the match set is computed over each
  zone's whole payload (level-independent), so switching level keeps the same
  matches, and each zone's meta line reports its match count.
- **Labels toggle.** A header `button.ctl` ("Labels: on/off", default on)
  switching room labels globally across every zone. Implemented as a
  `showLabels` option on `paintLevel` (the label pass becomes conditional),
  NOT a CSS class on the zone SVG — `buildLevelSvgFile` calls `paintLevel`
  directly and never sees zone-level classes, so a CSS-only toggle would be
  silently ignored by every exported file; threading the flag means **SVG
  export follows the toggle** (a labels-off export simply contains no
  `<text>` nodes — omitting beats styling away, and `exportStyleBlock`'s
  leftover `.label` rule is harmless). The toggle re-renders every zone with
  `refit: false` so pan/zoom is never disturbed, skipping zones with no
  payload yet. Cull units are unaffected (`cullZone` iterates each unit's
  nodes; nothing indexes them positionally). Deliberately **not persisted**,
  matching `linkViews` — view prefs should persist together or not at all
  (HANDOVER-ui-layout.md Decision 4). This also puts the label half of the
  open level-of-detail item (below) in place: an automatic LOD mode would
  drive the same `paintLevel` flag from zoom level rather than a button.
- **Hierarchy areas overlay + summary.** A per-zone "Areas" toggle draws the
  server's dissolved gross-area footprints (`GET /projects/{id}/areas`, see
  [Server](STRATEGY-SERVER.md)) on top of the current level, rooms ghosted
  beneath, with a tier picker (Building / Department / …) to choose which tier's
  footprints show. The overlay reuses the render path's transforms and the
  categorical `Set2` palette; each island is a `<path>` of its exterior ring plus
  any interior void rings, drawn `fill-rule="evenodd"` so a genuine courtyard or
  atrium reads as an open hole — matching the server area, which excludes it (the
  void-closure rule: walls fill, real voids stay open — see
  [Server](STRATEGY-SERVER.md)). A Case-A excluded group
  (`counted_upward: false`) reads dashed + faint rather than vanishing. Tier
  labels are fitted per group (`ringBox` against the group's largest ring,
  mirroring `addLabel`'s room-label sizing but with a more conservative 0.7
  width factor, since a bbox overstates the usable interior of the concave
  footprints dissolves produce), and a group too small for legible text gets
  **no label at all** (below `baseFont * 0.25`). That suppression threshold is
  deliberately the one *removed* from room labels as a bug — the difference is
  that a suppressed tier label loses nothing (the summary panel names every
  group), whereas a suppressed room label had no other surface. Accepted
  limitation: the threshold derives from the level's *fitted* bounds, not the
  current view, so a group suppressed at floor scale stays unlabelled however
  far you zoom (the overlay isn't re-rendered on pan/zoom); making labels
  zoom-responsive means driving `renderAreasOverlay` from the pan/zoom path
  with a view-derived `baseFont`, throttled the way `cullZone` is — deferred
  pending need. The **figures live in the bottom region's band 1**, not beside
  the plan: one table for the page covering **every level in scope**, each
  group's dissolved **footprint** area beside its summed **net** room area
  (computed client-side by shoelace over each room's loops) and their **Δ** =
  wall zones + filled voids, with per-level subtotals and an all-levels total
  — the two numbers answer different questions, and their difference is itself
  legible. The split is the region model's: the **overlay** is presentation on
  one zone's level (so its tier picker stays per-zone), while the **figures**
  are a result over the whole scope (so the band carries its own tier picker,
  and can show Department figures while a zone's overlay draws Building). The
  `/areas` fetch is shared — one dataset feeds every open overlay and the
  table. All client-side, the same "axum stays a pure JSON
  API" line as the colour maths and CSV export: the server ships coordinates and
  areas, the browser draws and tabulates. Fetched **on demand** (on toggle, and
  refreshed when new room data arrives) rather than on the 2s room poll, since
  areas are derived and heavier than a room fetch — the endpoint-vs-poll lifecycle
  call the "Endpoints follow fetch lifecycle" section describes.
  **The wall-zone rewrite** (see [Server](STRATEGY-SERVER.md)) left the
  `polygons`/`holes` wire shape untouched, so the even-odd overlay needed no
  change at all — the intended outcome, and the reason that shape was worth
  pinning down before the geometry underneath it moved. The response *gained*
  two fields the overlay currently ignores: `measurement_standard` and
  `wall_gap_by_level`. They are worth surfacing in the band-1 areas block when
  someone next touches it, because an area figure without its definition is
  exactly what a measurement standard exists to prevent, and a reader has no
  other way to tell a centreline level (walls already inside the rooms) from a
  finish-face one (walls filled to a declared thickness). Deliberately not done
  here: it is a UI decision about where a per-level fact belongs in a
  scope-level table, not a mechanical follow-on from the server change.
- **Room click-selection (page state) + the adjacency graph (band 1, canvas).**
  Clicking a room selects it: **one selection for the page**, not per zone
  (HANDOVER-ui-layout Decision 3), applied in every zone that draws that room
  while `selectedZoneId` records where the click landed — a label, not
  `activeZoneId` machinery. It rides the same cull-unit room→nodes map the search
  highlight uses, so selecting is a class toggle on already-rendered nodes, never
  a re-render, and a 4px slop distinguishes a click from a pan. This was the
  thing blocking Decision 3's inspector; the inspector now registers on
  `selectionListeners` rather than editing `selectRoom`.
- **Room inspector (right column) — implemented.** The third region of
  HANDOVER-ui-layout's model (*top: what am I looking at; right: what is this
  thing; bottom: what am I doing to it*), and the last of its decisions to be
  built. `#mainRow` wraps `<main id="zones">` and `<aside id="inspector">` at
  `minmax(0, 1fr) auto` — a wrapper because `main` *is* the zone container and JS
  rewrites its columns on every zone add/remove, so the column the handover
  believed was reserved never existed. A layout participant that reflows the
  plans, never an overlay inside `.zone-canvas` — that pattern is what the
  bottom-region work removed. Hidden with no selection, so it costs no width.
  One per page: selection is page state, and `selectedZoneId` is shown as a
  label so a room visible in two zones says which click produced it.
  **The property count is the real design problem, and it is worse than it
  looks**: a House A room carries 45 properties, and **19 of them hold no real
  value on any room** — the Revit extractor emits the literal string `"None"`
  for an unset parameter, so an emptiness test checking only for `""` hides
  nothing. A *Hide empty* toggle that treats blank and `"None"` alike is on by
  default and takes 45 → ~22 with no configuration; a name-filter box handles the
  rest. Both report *"n of m shown"* so nothing is silently dropped. Hide-empty
  is deliberately **not** applied to the classification tiers: on
  `sample-project` every room resolves to `[undefined]`, and suppressing that
  would hide the fact that the classifier ran and produced the undefined bucket,
  which `service::areas` treats as a real group. Model and dRofus stay visually
  separate per [Sources](STRATEGY-SOURCES.md), and an unmatched room is told it
  is *not joined* rather than shown an empty section. Read-only, and unpersisted
  like every other view preference.
  **A third mechanism — a checkbox property picker modelled on the search field
  panel — was specified and deliberately not built**, because the two above
  covered the cases it was for and unused UI is worse than none. If users do
  start re-picking the same columns every session, the durable answer is
  extending `room_label` in project settings (server-side, per project,
  shareable) rather than adding `localStorage` here. See
  [Superseded/HANDOVER-room-inspector.md](Superseded/HANDOVER-room-inspector.md)
  §5.3 for that design if it is ever wanted.
  On top of it, the **adjacency graph**: a third band-1 block over
  `GET /projects/{id}/adjacency` (see [Server](STRATEGY-SERVER.md)) showing what
  the selected room shares a wall with, and what those touch. **This is the first
  renderer in the project to leave SVG, and the reason is worth stating
  precisely: the trigger is continuous animation, NOT element count.** A depth-2
  graph is tens of nodes — three orders of magnitude inside SVG's comfort zone —
  but the layout settles over a run of frames, and SVG is retained-mode with no
  render loop. So the plan stays SVG and the graph takes canvas, in its own file
  (`static/graph.js`), and the page deliberately runs two renderers. The costs
  are paid explicitly: hit-testing is a nearest-node scan, the palette is read
  once from the resolved `:root` properties (the same trick `exportStyleBlock`
  uses, so it cannot fork `tokens.css`), the backing store is sized by
  `devicePixelRatio`, and there is **no export** — "Export SVGs" is a plan
  feature and raster export stays out of scope. The simulation solves **angle
  only**, each node pinned to the radius of its hop count: ring position *is* the
  message, and a set of 1-D problems settles without a cooling schedule. It stops
  when settled rather than repainting a static picture forever. **Each ring uses
  the whole circle**: ring 1 has no inward angle to be sprung toward (the focus
  is at the centre), so its nodes are spaced `2π/n`, and deeper rings keep the
  parent spring but fill the circle by inheriting their parents' spread — see
  [HANDOVER-adjacency.md](Superseded/HANDOVER-adjacency.md) "Amendments" for the bug that
  taught this. It follows **either** selectable thing on the plan: a room is a
  node, and a hierarchy footprint focuses the same graph aggregated to that
  footprint's tier — one node per area group, with the shared walls between two
  groups summed. That aggregation is client-side because it is a relabelling of a
  payload already held (group by `pathKey`, sum `shared_length`), so switching
  granularity is a re-layout and not a second endpoint. Node colour
  reuses `qualitative`, which moved to `common.js` for exactly this reason — two
  views disagreeing about a department's colour is worse than either being
  arbitrary. The wall-tolerance slider is debounced (0–900mm, mm readout, feet on
  the wire) and is a **tuning instrument, not a preference**: see Server for why
  the tolerance is a request parameter at all.
  **The default is the server's, and the viewer does not restate it.**
  `adjWallMm` starts `null`, which makes `adjacencyUrl()` **omit `wall_max`
  entirely** — omission is how the viewer *asks for* the derived default (the
  project's declared `[areas] max_wall_thickness`, narrowed to zero where every
  level in scope is centreline). The response echoes what was applied, and
  `syncWallFromServer` moves the thumb and readout to it *without* taking
  ownership, so the request stays unqualified and a settings change still reaches
  the viewer. The first drag assigns a number, and from then on the tolerance is
  sent explicitly — including `0`, which is a real tolerance and precisely why
  "unchosen" needed its own representation rather than reusing zero. A project
  switch drops back to `null`: the tolerance is a property of the project, so a
  value probed on one must not become a silent override on the next.
  This replaced a hardcoded `1.5 ft` that was sent on every request, which meant
  the derived default was **never** the one applied — the same
  one-quantity-two-declarations bug `[areas] max_wall_thickness` was created to
  delete (it replaced `areas::MAX_WALL_FT` + `adjacency::WALL_MAX_FT`),
  reappearing across the wire. It was not cosmetic: measured on `showcase`
  declaring `max_wall_thickness = 0.5`, the old viewer asked for 1.5 ft and drew
  **189** edges where the project's own declared thickness gives **75**.
  Verified live for both regimes' plumbing: unqualified request →
  `152 mm · project default`; drag → `?wall_max=1.3123` and a bare `400 mm`;
  project switch → back to `457 mm · project default`. Two deliberate details:
  the readout carries the server's exact figure while the thumb can only land on
  a `step` multiple (1.5 ft is 457.2mm, the thumb goes to 460) and is clamped to
  the 0–900mm range a project may legitimately exceed — **the text is the honest
  one**; and the `· project default` suffix exists because a bare `0 mm` on a
  centreline project reads as a broken control rather than the correct answer it
  is. One physical quantity, one declaration — see
  [Area calculation](STRATEGY-AREA-CALCULATION.md).
- **Settings page (`settings.html`).** A sibling static page, linked from the
  viewer's header, over [Server](STRATEGY-SERVER.md)'s `/api/settings` routes:
  a project-file list on the left (a file that fails to parse still gets a
  row showing its error), a form editor for identity / dRofus source /
  hierarchy / builtin properties / room label / milestones / QA fields /
  colour plans, a dRofus "check" button that dry-runs the CSV path
  server-side, and saves that go through the exact startup validation before
  landing (see Server).
  The dRofus section is a three-way source selector (`none` / `file` /
  `upload`): `file` keeps the path input + check button; `upload` shows a
  drag-and-drop zone (with a file-picker fallback) that POSTs the dropped
  CSV as a raw `text/csv` body to `/projects/{id}/drofus` — deliberately not
  `FormData`/multipart, matching the server's raw-body ingest — plus the
  stored upload history from `GET .../drofus/snapshots` with the live latest
  marked. A success refreshes the QA label dropdowns from the response's
  `labels` (no second call); the upload-mode counterpart of "check" is
  `GET .../drofus/latest`, run on editor open, where a 404 renders as a
  neutral "no upload yet" hint rather than an error. The zone is disabled
  with a "save the project first" hint while the project is unsaved, since
  the endpoint rejects unregistered projects.
  The milestones section edits name/date rows plus per-model pin dropdowns
  whose options are the snapshot ids the server actually stores
  (`GET /projects/{id}/snapshots`); a pin referencing a model or snapshot
  the store no longer has renders visibly as missing rather than being
  silently dropped — removing it is the user's call. Each milestone also
  gets a single **dRofus pin** dropdown (`— current dRofus —` plus one option
  per uploaded dRofus snapshot from `GET /projects/{id}/drofus/snapshots`),
  shown only when the project actually has uploaded dRofus snapshots to
  choose from — a `file`-sourced or upload-less project has nothing to pin,
  so the control is simply absent. The **colour plans** section edits all three
  modes: a name, an active *radio* (the browser enforces the one-active rule the
  server also validates), a mode selector, and mode-specific controls —
  *property compare*: A/B property inputs (datalist of the project's real room
  keys from `/rooms`), op, and colouring sub-mode (match tolerance / diverging
  scheme / add-remove band rows); *hierarchy*: an ordered checklist of the
  project's own hierarchy tiers (parent first) + a qualitative scheme;
  *date-range*: a date-property input, a near-date picker, a scheme, and a
  strftime format input pre-filled from the project's `date`-typed
  `drofus_fields` (blank = native ISO). A plan of a genuinely unknown mode
  (forward-compat) is shown read-only and round-trips unchanged rather than
  being clobbered on save. Same visual
  language as the viewer — once a third sibling page (`comparison.html`) appeared,
  the shared `:root` palette tokens were extracted to `static/tokens.css`
  (`<link>`ed by all three pages), and the two identical settings-API fetch
  helpers (`apiGet`/`apiSend`, used by `settings.html` and `comparison.html`) to
  `static/common.js` — which also now carries the selection-persistence helpers
  (`seedProjectId`/`persistSelection`, loaded by all three pages including the
  viewer; see "Selection persistence" below). Both are served by the same
  `ServeDir`; page-specific CSS/JS stays inline per page. These files are still
  hand-written classic scripts — the build step added in 2026-08 covers
  `src-js/` only, and `common.js` in particular stays out of it because four
  pages load it (see "UI growth").
- **Selection persistence (URL + localStorage).** The three pages are separate
  static documents linked by plain `<a href>`, so a navigation drops all
  in-memory state; previously each reseeded to `projects[0]`, so viewer → settings
  → back reset the user's project. Now the scope pick survives navigation,
  reloads, and bookmarks via two stores with a deliberate precedence, in
  `common.js`'s `seedProjectId` (read) / `persistSelection` (write): **the URL
  query wins** (a bookmarked/deep-linked `?project=…` is authoritative),
  **localStorage is the cross-page fallback seed** (one shared key,
  `roommate.project`, so a pick on any page seeds the others), and the page's own
  `projects[0]` default is the last resort. A restored id is always **validated
  against the live `/projects` list** first — a stale id falls through to the
  default, never a bad fetch. Writes use `history.replaceState` (a selection is
  not a navigation, so it adds no Back-button history). localStorage stores
  **only the project id** (the one selection every page shares); the viewer's
  building/milestone are viewer-specific and per-project, so they ride the **URL
  only** and never seed the other pages. Under global scope the URL simply
  mirrors *the* selection — the old "persist only `zones[0]`" special case
  (and the question it papered over) fell away with per-zone scope — and the
  viewer's restore also seeds localStorage (parity with the editors, whose
  restore persists via `selectProject`), so a bookmarked viewer link carries
  the project onward. Deliberately kept a small URL/localStorage fix,
  not a router or framework — the STRATEGY trigger for that ("writing the same
  state into several DOM places and watching them drift") isn't met.

## Rendering: SVG today, and when to move

SVG is the current choice and is likely right for a long time.

- **SVG stays correct** for more vector primitives — annotations, dimension
  lines, tags, highlighted adjacencies, overlays, clickable/hoverable regions —
  in the hundreds to low thousands of elements. Every element is a real DOM
  node, so hit-testing, hover, click, CSS styling, and accessibility come for
  free. This is why labels and tooltips were trivial to add.
- **The wall is the DOM**, not the feature set. Performance degrades somewhere
  in the low tens of thousands of elements (layout/repaint of a huge DOM).
  SVG also has no render loop — it is retained-mode, so continuous animation
  (dragging, live cursor feedback) fights the model.

The escalation tiers, if ever needed:

- **Canvas 2D** — immediate-mode, handles far more shapes, natural for
  draw-on-top with a render loop. Cost: lose DOM-given interactivity; rebuild
  hit-testing (point-in-polygon), hover, styling by hand.
- **WebGL / GPU** (PixiJS, regl, deck.gl-style) — hundreds of thousands of
  elements at 60fps. Real complexity; overkill unless genuinely at that scale.

The trigger to move is **not** "draw shapes on top" (well within SVG's comfort
zone) but **element count on screen** or **a need for continuous animation**.
Because the server emits geometry as data, the renderer is swappable without
touching the server or extractor — so this decision can be deferred until real
usage demands it. For many architectural-plan cases it never does.

## UI growth: toward a richer browser tool

Goal is a richer browser tool run locally (not a desktop app). The strategy:

- **Keep axum as a pure JSON API. This is the load-bearing decision.** The
  server emits data over HTTP, never HTML, and never assumes what the UI looks
  like. Holding this line keeps every later choice reversible and local.
- **~~Grow the vanilla JS until it actually hurts~~ — it now hurts, and a
  toolchain landed (2026-08-02).** The advice was right for longer than
  expected: endpoints, the inspector, filters, search and synchronized views all
  came in as plain DOM. What finally broke it was not a *feature* but the
  **renderer** — a WebGL plan layer needs polygon triangulation with holes, a
  glyph atlas and batched draw calls, all solved problems that must not be
  written again here (docs/HANDOVER-webgl-renderer.md). Third-party code implies
  npm implies a build.

  Note *which* signal fired, because it is not the one this bullet predicted.
  The predicted one was a feeling — the same state written into several DOM
  places, drifting. That never arrived. The actual trigger was a hard capability
  the page could not reach without dependencies. Worth recording: the framework
  question and the toolchain question turned out to be separate, and only the
  second has been answered.
- **A build step is not a framework, and this is still not one.** What landed is
  Vite + TypeScript over `src-js/`, emitting one committed IIFE that
  `index.html`'s existing inline script calls (docs/PLAN-webgl-renderer.md).
  There is no component model, no router, no virtual DOM, and no reactive store.
  The fork below is therefore still open — the toolchain does not pre-commit it,
  though it does lower the cost of the JS-framework branch and raise the
  relative cost of the Rust+WASM one, which would now be a second toolchain
  rather than the first.
- **When it hurts, the fork is JS framework vs. Rust+WASM.** Behind axum, either
  a JS framework (Svelte gentlest, React most-supported) or a Rust+WASM one
  (Leptos / Dioxus). The project tilts toward **Leptos / Dioxus**: the Rust
  `Room` / `Level` / processed-geometry structs can be reused directly in the
  UI, eliminating the recurring friction of re-describing a carefully versioned
  contract in TypeScript. The trade is a smaller ecosystem and fewer ready-made
  components — a fair deal for a single-developer tool valuing one language and
  shared types end to end.

### Endpoints follow fetch lifecycle, not data type

As capabilities are added, give each its own **purpose-shaped endpoint** rather
than overloading `/rooms`. When processing arrives, `/rooms` stays raw geometry
and new endpoints (`/projects/{id}/adjacency` — now shipped, singular and
project-scoped to match the `/areas` precedent — `/levels/{id}/analysis`, etc.)
carry the derived data. Small endpoints mean any future frontend composes them freely, and
no presentation assumption gets baked into the data layer.

The principle is **not** "one endpoint per data type" — it is "one endpoint per
thing fetched independently, on its own schedule, by its own consumer." The
test: *would this ever be fetched on a different trigger, or be expensive enough
that it shouldn't sit in the default payload?*

- **No → keep it in the snapshot.** Levels are a worked example: the viewer needs
  levels and rooms *together*, in the same render pass, from the same POST. They
  share a lifecycle (one export, one payload, one fetch). Splitting them would
  mean two requests that always travel together, recombined client-side, with a
  race between them — cost, no benefit. Levels stay inside the payload.
- **Yes → own endpoint.** Derived/computed data that is recomputed on a
  different trigger, sized differently, or consumed by a different part of the
  UI. The adjacency graph is now the worked example rather than the hypothetical:
  it is fetched when the room *selection* changes (and when the tolerance slider
  settles), not on the 2s poll, and it feeds a canvas rather than the SVG plan —
  both halves of the test, so it earned `/projects/{id}/adjacency`. Still
  hypothetical: per-level analysis fetched only when a level is
  selected, full detail on one room for a properties panel. `/projects` and
  `/projects/{id}/buildings` are a shipped example: they're fetched on a
  different schedule (a picker changing) than the room render, by a different
  consumer (the header, not the SVG canvas) — so they earned their own
  endpoints rather than riding inside `/rooms`.

This also means the processing layer and the endpoint that exposes it tend to
arrive in the same move: add the algorithm, add the endpoint. dRofus and
classification (see [Server](STRATEGY-SERVER.md) and
[Sources](STRATEGY-SOURCES.md)) are worked examples of the "no" branch that
already shipped: both are joined/resolved at `/rooms` response assembly rather
than given their own endpoint, because today they still share the viewer's
render pass. Each is a candidate for its own endpoint (`/drofus`, `/hierarchy`)
the moment it starts refreshing on a different trigger (a live dRofus poll) or
serving a different consumer (a hierarchy browser) than the room render.

## Open items / things to watch

- **2s-poll re-render cost — resolved.** The viewer used to re-stringify the
  whole payload every 2s to detect change. It now compares a single
  server-computed content `revision` (see [Server](STRATEGY-SERVER.md)), so a
  quiet system triggers no re-render between real pushes; the per-zone tick also
  fetches `/projects` once and runs zones concurrently. Kept here as a pointer
  since earlier notes flagged this as a risk.
- **Viewport culling on pan/zoom — implemented.** SVG clips but does not cull, so
  every room element used to cost per frame regardless of zoom. `paintLevel` now
  records each room's precomputed (Y-flipped) bbox + its nodes as a "cull unit";
  `setViewBox` schedules a `requestAnimationFrame`-throttled `cullZone` that hides
  rooms whose bbox is outside the current view (plus a 20%-of-view margin) and
  shows them again on re-entry, toggling a unit's `display` only when its on/off
  state actually changes. bboxes come from the loop points, never `getBBox` (which
  would force layout). The SVG export deliberately passes no cull-unit array — an
  exported file needs every room. Measured on the 10k-room `big-plate` fixture
  (5,046 rooms/level): a deep zoomed-in pan went from **~595 ms/frame (~2 fps)** to
  **4–15 ms/frame** (only the ~12–40 on-screen rooms drawn), verified to restore
  every room on zoom-out and to leave the export at full room count.
  A **debug kill switch** (`CULL_ENABLED`) turned the hide/show off so this
  could be re-measured rather than taken on faith whenever the renderer changed
  — see
  [HANDOVER-culling-disable-switch.md](Superseded/HANDOVER-culling-disable-switch.md).
  Re-measured 2026-07-25 with it: **16.5 ms/frame on vs 912 ms/frame off**.

  **All of the above is now history, and the switch took its own advice.** That
  handover said to delete culling if something ever made it redundant rather
  than leave it switched on, and the WebGL renderer did exactly that: a frame is
  four draw calls whatever the room count, so there is nothing off-screen worth
  hiding and no per-element cost to avoid. Culling, its kill switch, the cull
  units `paintLevel` used to collect and the `roomBBox`-per-room index that fed
  them are all deleted (2026-08-02). The *spatial* index survives in a different
  form and for a different job — a Flatbush R-tree answering the pick, in
  `src-js/renderer/gl/spatial.ts`.

  Worth keeping the number, because it is the honest reason the feature existed:
  912 ms/frame was real, and on SVG the cull was worth every line. It stopped
  being worth them when the thing it was compensating for went away.
- **Fitted-view cost at very high room counts — CLOSED 2026-08-02, by moving the
  plan to WebGL.** This was the item that justified the whole renderer exercise:
  culling helps only when geometry is off-screen, so a *fitted* view of a
  5,000-room level painted everything at ~0.5 s+/frame, and a fitted view is what
  the viewer shows on load because it auto-fits.

  Measured on `big-plate` (5,046 rooms/level, 718 of them with voids), same data,
  same machine, same interaction — a scripted pan, frame times sorted:

  | | SVG | WebGL (DPR 1) | WebGL (DPR 2) |
  |---|---|---|---|
  | **fitted pan, p50** | 534 ms | **0 ms** | **0 ms** |
  | **fitted pan, p95** | 733 ms | **1 ms** | **2 ms** |
  | zoomed pan, p95 | (see culling above) | 3 ms | 2 ms |
  | build + first paint | 1,175 ms | 1,155 ms | 988 ms |

  Budget, set before measuring, was **≤16 ms p95 on a fitted view at the target
  DPR, labels and pick included**. Held with two orders of magnitude to spare.

  Three things about that table are worth more than the headline:

  **DPR barely matters here, which was not the expectation.** The POC measured at
  DPR 1 and warned that a retina display roughly 4×'s fill cost. It does not show
  up, because this renderer is not fill-bound at these counts — the frame is four
  draw calls and a handful of uniform writes.

  **Build cost is a wash, not a regression.** An earlier reading of this made
  WebGL look 4× worse, by comparing its full build against `paintLevel`'s JS time
  alone (179 ms) while ignoring that the browser then spends ~1 s laying out and
  painting 26,451 SVG nodes. Compared end to end the two are within 2%. Roughly
  1.1 s of the WebGL side is constructing 5,046 `BitmapText` objects; geometry
  alone is ~180 ms. If level-switch latency ever needs attention, that is where
  it is, and the fix is folding glyphs into the same attribute mesh the fills use.

  **Most of the win came from one line, found by measuring rather than reasoning.**
  The first WebGL numbers were 23 ms p50 / 36 ms p95 at the fitted view — over
  budget. The cost was Pixi re-deriving the world transform of all 5,046 label
  nodes every frame, all of which move by the same amount. Marking the label
  container as a render group took it to 0/1 ms. Nothing about the geometry, the
  shaders or the index changed.

  Level-of-detail beyond this is **not needed and not built**. The grid is still
  not capped to the visible region either. Revisit only if measurement says so.
- **What moving to WebGL cost, recorded as an acceptance rather than left to be
  discovered.** Room labels are now pixels in a canvas, not `<text>` nodes. They
  are therefore **not selectable, not searchable with the browser's own find, and
  not exposed to a screen reader**. `static/graph.js` already made and stated the
  same trade for the adjacency graph, but the plan is a much bigger surface and
  the loss is correspondingly bigger, so it is written down here as a decision
  taken with the number in front of us — not as something noticed later.

  Three things blunt it, and none of them undo it: the **SVG export** still
  produces real `<text>`, so the selectable/searchable artefact exists on demand
  and is what leaves the browser; the **inspector** shows the selected room's
  properties as ordinary DOM; and **search** matches server-side data rather than
  rendered glyphs, so finding a room by name still works — it is the browser's
  own Ctrl+F over the plan that does not.

  Per-room tooltips survived the move but had to be rebuilt: every room polygon
  carried an SVG `<title>` that the browser drew for free, and WebGL has no
  equivalent, so a DOM tooltip is driven by the hover pick instead. Same text.
- **The SVG live renderer is deleted**, and with it the `RENDERER` flag that
  chose between the two. Keeping a second live renderer purely so a number could
  be re-derived would have meant maintaining it forever, and an unexercised
  fallback rots — a fallback nobody runs is a fallback nobody knows is broken.

  The *capability* survives without the code: `measureSvgPaint()` in the console
  times the **export** painter, which has to exist anyway and builds exactly the
  same geometry, into a detached `<svg>`. It measures DOM construction only —
  the browser's layout and paint of those nodes is the larger half, and is what
  a live SVG renderer additionally paid every frame — so it is a floor on the
  SVG cost, not an estimate of it. Enough to re-derive the shape of the table
  above, which is what the flag was for.

  The consequence, stated plainly: **a browser without WebGL now shows no plan.**
  That was accepted deliberately when the plan was written
  ([PLAN-webgl-renderer.md](PLAN-webgl-renderer.md), "Departures"), on the
  grounds that two live renderers is a permanent tax on every future frontend
  change.
- **Coordinates and units.** Revit internal units are decimal feet, Y-up; SVG
  is Y-down — handled by flipping Y when building geometry. Absolute units do
  not matter while the viewer auto-fits, but they will once dimensions, a scale
  bar, or north-alignment are added. The **placement** half of that is captured
  but **not yet reachable from here**: a model may carry a `model_to_shared`
  affine on its *upload* envelope (see [Index](STRATEGY.md) "The upload
  envelope"), and the server validates and stores it — but no read endpoint
  serves it, so the renderer does not merely ignore it, it cannot see it. The
  first step for any consumer below is therefore a server change (surface the
  per-model transform on `/rooms`, alongside the `boundary_by_level` precedent),
  not a browser one. Once it is served, north-alignment, a real-world scale bar, and
  the georeferencing map underlay (Phase 3 — `docs/Superseded/HANDOVER-georeferencing.md`)
  are exactly the features that consume it. Composing it correctly is a
  browser-side job — the existing Y-flip *plus* the `model_to_shared` matrix
  *plus* (for the underlay) a reprojection into the tile frame — and the server
  stays out of it: it emits the transform as data, the renderer composes the
  picture, consistent with "the server emits geometry as data, the renderer is
  swappable."
