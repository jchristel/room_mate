# Plan — shortest path between rooms, in three steps

**Status: proposed, nothing built.** Moves to `Superseded/` when step 3 lands,
and what survives goes into [Entities](STRATEGY-ENTITIES.md) (the door
connectivity graph is already listed there as deferred) and
[Authored](STRATEGY-AUTHORED.md) (connections are its first kind).

**The goal:** pick a start room and an end room on the plan and see the shortest
route between them.

1. **Doors only.** Rooms are connected where a door joins them. Bays and anything
   else without a door are simply not connected yet.
2. **Authored connections.** The user states how a bay (or any room) connects to
   another, on the same level.
3. **Vertical connections.** The user states which rooms connect between levels.

Each step is shippable alone and each one makes the previous one's gaps visible
rather than hiding them: step 1 reports exactly which rooms it could not reach,
and that list is step 2's worklist.

## What already exists, and what this must not duplicate

- **Every door names both rooms, model-qualified.** `/doors` rows carry
  `room_origin.from_room` / `.to_room` as `SideOrigin` (authored, derived by
  geometry, or unresolved), and `.room()` gives a `RoomRef { model_id, room_id }`.
  Step 1 reads that; it does not re-resolve anything.
- **`/adjacency` is not this graph.** It is shared-wall geometry, same level only,
  and its nodes carry a **bare `room_id`** with no model. The connectivity graph
  must use `RoomRef` keys throughout, because a room id is unique only within a
  model. Adjacency stays as it is; it is borrowed in step 2 only to *suggest*
  candidates (see below).
- **Scope, milestones, building and the property filter** are already one
  mechanism (`OpeningScope`, `RoomScope`). The graph is built over exactly the
  rooms and doors those return, so a milestone or building view works for free.
- **A new kind of read is a service module, an HTTP adapter and an MCP tool**,
  nothing else (`service/` never imports `axum` or `rmcp`).

## Step 1 — doors only, with a two-click picker

### Server: `service::connectivity`

One transport-agnostic module, `GET /projects/{id}/connectivity`.

- **Nodes:** the project's rooms from `assemble_rooms`, keyed by `RoomRef`, each
  carrying name, level and a plan point.
- **Edges:** one per door whose two sides both resolve to a room. A door with
  exactly one resolved side is an **exit** on that room, reported on the node and
  never invented into an edge to a made-up "outside" node. A door with no
  resolved side is reported as unattached. **Signal, not error** throughout.
- **Weights:** the distance a person walks, `|centroid(a) − door| + |door −
  centroid(b)|`, in the project-local frame (feet). `metric=hops` is the
  alternative, and costs one line.
- **Search:** Dijkstra. Written by hand (about 50 lines) unless `petgraph` is
  already a transitive dependency; neither `petgraph` nor a pathfinding crate is
  in `Cargo.toml` today, and adding a dependency for one function is the thing to
  avoid.
- **Parameters:** the usual `building`, `milestone` and `filter` (the filter
  applies to *doors*, so `Fire Rating!=...` or a width test removes a door from
  the graph without a new feature), plus `from` and `to`.
- **Response:** `nodes`, `edges`, `components` (a list of room sets), `isolated`
  (rooms in a component of one), `exits`, and, when `from` and `to` are given,
  `path`: an ordered list of rooms and the doors between them, the total
  length, and the geometry **grouped by level** so the viewer can draw each
  zone's slice without a second request. `path: null` with a reason is a
  *finding* ("no door route; `from` is in component 3, `to` in component 7"),
  never a 404.
- **Room addressing on the wire:** `from`/`to` are `model_id` plus `room_id`
  pairs. A bare room id is accepted when exactly one model has it and is refused
  with the candidates listed when two do, because a silent guess is the failure
  this codebase keeps designing against.

### Viewer: the two-click route tool

The user's requirement: **start and end are chosen by clicking, and pan, zoom and
search must work at every stage of those two clicks.**

The plan already separates a click from a drag (`CLICK_SLOP_PX` in
`gestures.ts`), so panning and zooming never produce a pick. That is most of the
requirement; the design must simply not break it.

- **A route mode in the page store, not in a zone.** `route: null | { start:
  RoomRef | null, end: RoomRef | null }`. Selection is already page state for the
  same reason ("one room can appear in two zones"), and the two endpoints may well
  be on different levels, shown in different zones.
- **Entering the mode changes what a room pick does, and nothing else.** While
  active, a room pick (plan click, pick-list entry, grid row) fills `start`, then
  `end`, instead of selecting. A third click replaces `end`; clicking a placed
  endpoint again clears it. Escape cancels. Nothing about the gestures, the level
  pickers, the zone layout or the search changes.
- **Search at any stage.** Search already highlights matches across every zone
  without re-uploading a level, and a grid row click already centres its room.
  The plan must make a search match *pickable as an endpoint*: the mode adds a
  "set as start / end" action to a match, and the grid row click does the same as
  a plan click while the mode is on. **To verify when building, not assume:**
  whether the grid reflects the search filter today, and whether Enter in the box
  can take the first match.
- **Level switching mid-route.** Because the endpoint is a `RoomRef` and not a
  screen position, changing a zone's level, building or colour plan cannot lose
  it. A start on level 2 and an end on level 5 is ordinary; the tool shows both in
  a small route panel (names, levels, a clear button) so an endpoint that is not
  on screen is still visible.
- **Drawing the route.** A polyline on the SVG overlay, one slice per zone by
  level. The overlay rule in [Browser](STRATEGY-BROWSER.md) is that it is for
  marks that add pixels without removing any, and a stroke with no fill is
  exactly that. Endpoints get a start and an end mark. A hop between levels is
  drawn as a marker on each side, not a line.
- **The poll rule applies.** The route request is a new scope, so it follows the
  existing rule that a revision only vouches for the URL that produced it:
  a changed `from`/`to` drops an answer already in flight.
- **Out of scope for step 1:** per-person constraints (wheelchair width, fire
  doors), and showing alternative routes. Both fall out of the `filter`
  parameter and a `k` parameter later.

### MCP

- **`get_connectivity`** — one tool for the one new read route, parameters
  mirroring the route (`project_id`, `building`, `milestone`, `filter`, `from`,
  `to`, `metric`). Update the header count in `src/bin/mcp.rs` and the list in
  [MCP](STRATEGY-MCP.md); `scripts/weekly_review.py` checks both.
- **The description is the product here, as it was for windows and spaces.** It
  must say, in as many words: this graph is **doors only**; a room with no door
  (a bay, an open-plan area, a shaft) appears as isolated and that is *not* a
  model fault; `path: null` is a finding about the graph, not an error; exits are
  reported, not edges; and until steps 2 and 3 land, **no route crosses a level**.
  An agent that reads "no route from the ward to the plant room" as "they are
  disconnected" would be reporting the *method's* limit as the building's.

### Done when

`cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`npm run typecheck && npm test && npm run build`, and the viewer **driven in a
browser**: pick two rooms on one level, pan and zoom between the clicks, search
for the second room and set it from the result, change level between clicks, and
confirm the polyline lands on the doors. CLAUDE.md is explicit that a frontend
change is verified by driving the page.

**Measure first.** Before step 2 is designed in earnest, run step 1 over House A
and the largest project and record how many rooms are isolated and how many
components there are. That number decides how much of step 2 is worth building as
UI and how much a list in a file would do.

## Step 2 — authored connections

### The edge

```json
{ "id": "c-…", "a": {"model_id": "…", "room_id": "…"},
  "b": {"model_id": "…", "room_id": "…"},
  "kind": "open", "note": "bay opens onto corridor" }
```

Undirected, `RoomRef` on both ends, `kind` a small closed set (`open`, `bay`,
`other`; step 3 adds `vertical`). Stored as **edges, never as room edits**,
exactly as [Authored](STRATEGY-AUTHORED.md) specifies. Design the shape for step 3
now (cross-level allowed by the type, forbidden by the step-2 validator), so step
3 is a validation change and a cost, not a migration.

### Where it lives — decided for now, revisited at the end

**Decided: one JSON document per project beside the settings**, under
`<projects_dir>/connections/<project>.json`, written with the same atomic
install, `SAVE_LOCK` and `.backups/` copy that settings saves have. This is
deliberately **provisional**: the real storage type is chosen *after step 3*,
when everything that needs storing is known (edges, their kinds, a vertical
cost, anything the viewer turned out to need). Deciding it now would be deciding
on the least information anyone will have.

[Authored](STRATEGY-AUTHORED.md) specifies a versioned, milestone-pinned store
stream instead, so that document is amended in this change to say connections
start as a document and the question is open.

**What the provisional choice must preserve, so the later move is a copy and not
a redesign:**

- **One module owns the file.** A small `connections` module is the only code
  that reads or writes it; handlers and the connectivity read go through it. The
  storage type then changes behind one seam, the way `SnapshotStore` hides
  `MemStore` from `FsStore`.
- **The document is self-describing and versioned:** `schema_version`, a
  `taken_at` stamped on every save, and nothing else on disk that the edges
  depend on. Moving to a snapshot stream is then "write each saved document as one
  `taken_at` snapshot".
- **No milestone pinning yet, and the response says so:** `connections_pinned:
  false`, so nobody reads a historical route as historical.
- **Keep a list of what turns out to need storing**, in this file under "Storage
  inputs", as steps 2 and 3 are built. That list *is* the decision input.

**Known costs of the provisional choice**, accepted for now: no pinning, and the
file lives under the projects directory while snapshots live under the storage
root, so backing up only the storage root would miss it. Note this in the
installer notes if it ships before the decision.

### Storage inputs

*(Filled in as steps 2 and 3 are built: every field, kind and per-project value
that ended up needing to persist, and whether anything asked for history or
milestone pinning.)*

### Server and viewer

- `GET /projects/{id}/connections`, `PUT` (replace-whole-list, like settings;
  no patch semantics to define) and the connectivity read merges them as edges.
- **Reconciliation is read-time and loud:** an edge naming a room that is no
  longer in the model is skipped and listed under `stale_connections`; an edge
  duplicating a door is kept and flagged redundant. Authored data outliving its
  geometry is normal, not corruption.
- **A route mode sibling:** with a room selected, "connect to…" uses the same
  pick-anywhere-with-pan-zoom-search flow as step 1, writes one edge, and the
  route refreshes. The isolated-rooms list from step 1 is the worklist.
- **Suggestions, not guesses:** for an isolated room, offer the rooms it shares
  the most wall with, from `/adjacency`, as candidates to confirm. Never write an
  edge without a click. (Adjacency nodes lack a model id today; add it as an
  additive field when this lands.)

### MCP

- `list_connections` is the read tool for the new GET (one tool per read route).
- **Writes.** The MCP server's rule is no ingest and one forwarded mutation
  (`upload_reference`). Defining a bay's connection is a plausible thing to ask an
  agent to do, so a forwarded `set_connections` is defensible, on the
  `upload_reference` pattern (HTTP stays the single writer). The cost is that
  every mutating route widens the surface [Security](STRATEGY-SECURITY.md) is
  written about, and rate limiting is still unbuilt. **Recommendation: read tools
  now, a forwarded write tool as a separate, deliberate decision.**
- The route tool's description gains the authored half: connections are
  user-stated, not model-derived; `stale_connections` is a finding.

## Step 3 — vertical connections

- **The same edge, `kind: "vertical"`, across levels.** Validator: the two rooms
  must be on different storeys, judged by the storey rule in
  `src-js/renderer/storey.ts` / `rooms::dedup_levels` (**name plus elevation, never
  a level id**), because a level id is per document and a facade or services model
  would otherwise be wrongly "on the same level".
- **Cost.** A vertical edge has no plan distance. Use a setting,
  `[connectivity] vertical_cost_ft`, with a sane default, and say what it means:
  the walking-equivalent of a flight or a lift ride. It is a new settings field, so
  it needs the generated TypeScript regenerated, a control on `/settings/`, and an
  entry in [SETTINGS.md](SETTINGS.md).
- **Stacks.** A stair is one room per level, so connecting four levels is three
  edges. A lift reaches any level from any level; chained edges charge it per
  floor, which overstates a lift. Accept that in v1 and state it. Revisit with a
  `group` edge (one list of rooms, all mutually connected at the same cost) if it
  matters; it is the only reason the edge type would grow.
- **Viewer.** The "connect to…" flow must reach across levels, which the
  multi-zone layout already supports (two zones, two levels) and the route panel's
  off-screen endpoints already cover. The route drawing gains the cross-level
  marker defined in step 1.
- **Optional, cheap, after measuring:** suggest vertical candidates by plan
  overlap between rooms on adjacent storeys with matching names (stair to stair,
  lift to lift), using the existing overlap machinery. Only if step 2's
  suggestions proved useful.
- **MCP:** no new tool; the description drops the "no route crosses a level"
  caveat and gains the cost explanation.

## Critique of this plan

1. **"Doors only" is a bigger simplification than it sounds.** Open-plan
   connections, archways modelled as wall openings, and rooms whose doors resolve
   to no room all read as "not connected". Step 1 is honest about this only
   because it reports isolated rooms and unattached doors; if it reported just a
   path, it would be wrong in a way that looks right. Keep the reporting.
2. **The door's room references are only as good as the model.** `to_room`
   follows a door's orientation, and a cupboard door can legitimately belong to
   the cupboard while opening into the corridor. For connectivity, *both* sides
   are used, so this mostly doesn't matter, but a door with one side wrong
   connects the wrong pair. The QA report that reconciles authored against derived
   sides already exists; link to it from a route result that crosses a flagged
   door rather than re-checking.
3. **Distance through centroids is a rough walking length.** A centroid can lie
   outside an L-shaped room, and a straight line from centroid to door can cross
   walls. For *ranking* routes it is fine; for quoting a distance to someone it is
   not. Label the figure "approximate" and do not add visibility-graph routing
   until someone needs a real distance.
4. **The viewer tool is most of the work and most of the risk.** The server part
   of step 1 is small; the two-click flow touches the store, the gesture layer,
   the grid, the search and the overlay, and CLAUDE.md records four bugs in this
   page that no diff showed. Build it in slices, each driven in the browser, and
   budget for it.
5. **The storage choice is provisional on purpose.** The risk is the opposite of
   over-building: the document drifts into being the permanent answer because
   nothing forces the decision. The last item in the order of work is that
   decision, and it is not optional.
6. **Step 3 depends on a level model that has a known soft spot.** Levels are
   joined by name plus elevation precisely because ids differ per document and
   reference levels are sometimes mis-elevated. A "different storey" check built
   on that rule inherits its fallbacks; test it against a linked-model project, not
   only a single house.
7. **Scale is unmeasured.** A door read on a large project is not cheap (the same
   assembly costs seconds on `/ceilings`), and a route request recomputes the graph
   every time. Measure on the largest project before deciding whether the graph
   needs caching; the revision cursor the other reads use is the natural key.
8. **What is missing from the request.** Directionality (one-way and fire-exit
   doors), locked or staff-only doors, and accessibility are real routing
   questions. The `filter` on doors answers the first two cheaply and the third
   partly. Decide whether "shortest" means shortest for anyone or for a defined
   user before the response shape is frozen.

## Order of work

1. Step 1 server and MCP tool, with tests over a hand-built two-model fixture
   (cross-model door, exit, isolated room, unreachable pair).
2. Measure isolated rooms on House A and the largest project; report.
3. Step 1 viewer in slices: route state and panel, plan and pick-list entry,
   grid and search entry, overlay drawing, level-switch and zoom checks.
4. Step 2 on the JSON document (module, reads, writes, connect flow,
   suggestions, MCP read tool), recording "Storage inputs" as it goes.
5. Step 3 (validator, cost setting, cross-level flow, description update).
6. **Decide the storage type** from the "Storage inputs" list, then migrate the
   document behind the `connections` module if it moves. Amend
   [Authored](STRATEGY-AUTHORED.md) to match whichever way it goes.

## Measured after the step 1 server (2026-10-06)

Over the largest project (3,043 rooms, 1,797 doors, latest snapshots; read in
about 1 s):

- **1,627 door edges; 1,510 components; 1,379 rooms (45%) isolated.** The largest
  component is 157 rooms; only 1,300 rooms sit in a component of ten or more.
  Level 1 has 478 rooms in 239 components, so **most room pairs on one level have
  no door route.** A doors-only graph answers few "how do I get from here to
  there" questions on this project.
- **The isolated rooms are not mostly bays.** By name: 120 lifts, 79 stairs, 93
  risers, 24 bays, and 1,051 others. Lifts and stairs are step 3's rooms; risers
  are not part of any walking route; the 1,051 others are the open question.
- **Door room-resolution does not rescue it.** Setting `[doors] room_resolution`
  to `same_model` or `project` changed 1,627 edges into 1,627: authored
  references already decide almost every door, so the missing connections are not
  doors lacking a room reference. 84 doors name no room, 71 name the same room on
  both sides.
- **Points are sound:** 1,626 of 1,627 door points are the door's own insertion
  point.

**What this changes.** Step 2's worklist is 1,379 rooms, far too many to author by
hand one at a time, so step 2 needs bulk tools (select several rooms, connect to
one) and a first look at *why* 1,051 ordinary rooms have no door, which is a data
question before it is a UI one: open-plan areas, doors modelled outside the
pushed models, or archways that are not doors. Resolve that before building the
connect flow.
