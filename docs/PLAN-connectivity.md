# Plan — shortest path between rooms, in three steps

**Status: all three steps are built; one decision is left open: the storage type
of the authored connections** (see "Storage inputs"). This moves to
`Superseded/` when that is decided, and what survives goes into [Entities](STRATEGY-ENTITIES.md) (the door
connectivity graph is already listed there as deferred) and
[Authored](STRATEGY-AUTHORED.md) (connections are its first kind).

## Where this stands (read this first in a new session)

*As of 2026-10-08.*

**Merged to `main`:** the doors-only graph and two-click route picker (#189), open
zones and `/connections` (#190), vertical zones (#191, since removed), and manual
vertical links replacing them (#192).

Routing methods (#193) and start/end points inside rooms (#194) merged 2026-10-07/08.

**Built 2026-10-09, on branch `route-panel-saved-routes`:** a route panel, saved routes
and several routes at once. A vertical *view* (a lift or stair as a stack of rooms) is
deferred until the stacked-rooms measurement; vertical zones as stored data stay removed.

- **Steps.** A route's rooms in order, each with the hop into it (door, open zone, level
  change) and its length, and a rule where the level changes. A row takes the zone to
  that room's level and pans to it. It is a list because a route across levels is mostly
  off the level a zone shows.
- **Saved routes are requests, never paths.** `routes` in the same connections document
  as zones and links: the two rooms (model-qualified), where in them, the method and a
  `#rrggbb` colour. The path is derived on every read, so a saved route follows the
  doors, zones and links as they change, and one whose room has left the model reports
  the server's message on its chip rather than drawing a stale line. They are shared by
  the project, like zones. **A save with no `routes` key keeps the saved ones** (an
  explicit empty list clears), so a client older than this cannot silently delete them.
- **Several at once.** Which routes are shown is the viewer's own (page state, not
  stored, not kept across a reload); each zone draws every shown route's slice for its
  level, each in its colour on a paper halo so routes sharing a corridor stay readable.
  They stay drawn after the route tool is closed, and are managed from the tool's bar.
- **Not done:** routes sharing a corridor are drawn on top of each other, not offset;
  a reload shows none until they are switched on; the SVG export does not draw them;
  `get_connectivity` is unchanged and `list_connections` now carries `routes`.

**The stack view (built 2026-10-10).** "Stack view" in the Connections editor: pick a
room and see the rooms joined to it by vertical links as a column of levels, with the
next hop above and below offered. **Only levels that hold a room of the stack are
listed** (plus the one a suggestion points at), never the project's other levels: RHH has
two buildings with distinct levels and a hospital lift must not list the car park's
half-levels. A suggestion is never a write; **Link** saves one hop at a time and the
column grows and asks again. The read is `GET /projects/{id}/stack` and the MCP tool
`get_stack_candidates` (`service::stacking`). **A room may be in an open zone and in a
vertical link at once**: the records are independent and a test pins it.

**Measured on RHH 2026-10-10** (104 lift rooms, 63 stair rooms; latest rooms of the six
architectural models, a copy of the store):

- The next storey is **the nearest one with something over the room**, not the next level in
  the list: by list order only 51 of 198 lift hops found any overlap, because the level above
  a hospital lift is a car-park half-level; by "next storey with an overlapping room" 158 did.
  No notion of a building is needed, which a project need not configure.
- The room sharing the source's name stem (its name without the last word) was the top overlap
  in 146 of 158 lift hops and 70 of 75 stair hops, and a stem-mate was **never missed** when one
  existed. Lifts overlap fully or not at all, so the threshold barely matters for them; a
  stair needs the score relative to the SMALLER room, and 0.9 loses real stairs where 0.7 does not.
- 11 hops were ambiguous: several lifts under one plant room, a stair beside a pressurised
  stairwell (`STAIR PRES`), a stair over a riser. Those are shown as a choice, never preselected.
- Limits: no hand-drawn links exist on RHH to compare against, so the stem is a proxy; the probe
  used each room's outer outline and the deduplicated `/rooms` read (3,043 of 3,112 rooms).

**A route across levels was driven in the browser on RHH** (a copy, with two hand-made links
between the clinical lift lobbies on LEVEL 1 to 3): the route found 2 level changes at 80 ft, the
steps list marks both, a step switches the zone's level, and a saved route draws in every zone.
A hop between rooms stacked at one position has no length on the plan and draws as a dot; the
stack view is where it is read. `levels_between` on those links reported 3 skipped levels for a
correct floor-to-floor link, the interleaving the advisory already warns about.

**The editor draws its context.** While an open-zone editor is open, saved zones are
tinted by index under the zone being drawn, and rooms no door reaches and no zone
covers are ringed dotted: step 1's worklist, on the plan (`OpenZoneOverlay`).

**Decisions still open**

- **Storage type of the authored connections.** A JSON document beside the settings
  for now (see "Storage inputs"). Decide it from that list; the one module that
  owns the file is `src/connections.rs`.
- **Stack view follow-ups.** Candidates ignore a room's type, so `STAIR PRES` and plant rooms
  rank on name alone; a third signal (room type, `classification`) would help. A stack with
  two rooms on its top level suggests from the first only.
- **Wall clearance** as a third routing method (keep a margin off walls); the natural
  next one in "Horizontal path".
- **Dragging a start or end mark** to adjust it. Today a click sets the point.
- **Milestone pinning of connections**, and an **MCP write tool** for them. Both
  deferred on purpose; the Security doc explains why a write tool waits.
- **Doorless corridors.** About 1,050 ordinary rooms on the largest project have no
  door and are not lifts, stairs or risers. It is a data question (doors in models
  never pushed, open-plan areas, openings that are not doors) and was deliberately
  set aside.
- **`levels_between` is advisory** on a project whose buildings interleave their
  levels: a correct floor-to-floor link can report a skip.

**Practical notes that cost time to find**

- **Never test against the production store.** Copy the latest rooms and doors
  snapshot of each model, plus `project.toml`, to a temp directory and point a second
  instance at it with `--port`. Settings are not file-watched, and a server touches
  the store at startup.
- **Run a long-lived server from a copy of the exe** (`Copy-Item
  target\debug\roommate.exe` somewhere, start it with the repo as the working
  directory so `static/` is found). A running `roommate.exe` or `mcp.exe` locks the
  file and makes the next `cargo build` fail with "Access is denied".
- **Stop servers by process id** (find them with `Get-CimInstance Win32_Process`
  and match the `--port`), never by image name: that can kill someone else's.
- On this arm64 machine Python 3.12 and the GitHub CLI are installed per user (winget);
  open a new shell if `python` still resolves to the Store alias.
- The gates are in `CLAUDE.md`: `cargo test`, `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, and for `src-js/`
  `npm run typecheck && npm test && npm run build` (the built `static/` is committed).
  Clippy's `too_many_lines` is 100: split a function rather than allow it.

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

### The open zone (built; this replaced the pairwise edge)

**Status: built** (`src/connections.rs`, `service::connectivity`, the viewer's
"Open zones" editor). The first design was a pairwise edge; it was replaced by a
**zone**, a named set of rooms the user declares to be one open space, because
what a person knows is "this whole area is open", not which of 47 wall segments
inside it are. The largest cluster of doorless rooms measured was 48.

```json
{ "id": "east-bays", "name": "East bays", "kind": "open",
  "rooms": [{"model_id": "…", "room_id": "…"}, …], "note": null }
```

- **Meaning:** every wall two members share is open. The edges are derived at
  read time by running the members through the wall-sharing algorithm
  restricted to them, so a route follows the floor plan across the shared wall
  and never cuts a straight line through whatever lies between. Nothing derived
  is stored.
- **Reported, never errors:** `stale` members (not in scope), `unlinked`
  members (sharing a wall with no other member), `redundant` pairs a door
  already joins, and `reaches_doors` (false means the whole zone is still an
  island).
- **Authored data is replaced whole with a version check:** a save names the
  `taken_at` it read, and a mismatch is a 409, so two editors cannot overwrite
  each other unseen.
- **Not built:** suggestions from the adjacency graph, a "zone is mutually
  reachable regardless of walls" variant, a "these two are blocked" override,
  milestone pinning, and an MCP write tool (`list_connections` reads).
- **Step 3** reuses the record: a `vertical` kind whose members need not touch,
  with a per-level cost.

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

Recorded as the steps are built; this list is the input to the storage decision.

**After step 2:**

- A document per project: `schema_version`, `taken_at`, `zones[]`.
- A zone: `id`, `name`, `kind` (`open`), `rooms[] {model_id, room_id}`, `note`.
- A link: `id`, `a`, `b` (model-qualified rooms), `cost_ft`, `note`.
- A version stamp per save and a check against it (optimistic concurrency).
- A copy of each replaced document (20 kept): history is recovery only, not
  pinned or queryable. Nothing has yet asked for either.
- Rooms are referenced by model and id, so a renumbered room orphans a member
  and nothing re-links it.
- Nothing stores anything derived.

**After step 3:** a second list, `links`, beside `zones`. Nothing asked for history,
pinning or per-room roles. The pressure that a zone cannot tell a lift from its
lobbies was answered by making a vertical connection two rooms rather than by
adding roles, so the record stayed small.

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

## Step 3 — vertical connections (built, as manual links)

**Status: built.** A vertical connection is **one explicit link between two
rooms on different storeys, drawn by hand**, one floor-to-floor hop per link
(`src/connections.rs`, `service::connectivity`, the editor's "Vertical link"
mode). A lift or stair is a run of links (level 1 to 2, 2 to 3, ...), never a jump
from level 1 to 8.

**What it replaced, and why.** The first version of this step was a "vertical
zone": a flat list of rooms, joined across storeys by a chain. It could not tell
a lift room from the lobbies and corridors around it, so a corridor on one level
was joined straight to the next level and skipped the walk to the lift. Fixing it
needed roles inside a zone, which made the model heavier than the thing it
described. Two rooms per link has no such ambiguity, so the zone kind was removed
(a document that still names one fails to load loudly instead of being read as
an open zone).

- **The record:** `{ id, a, b, cost_ft?, note? }` with `a` and `b` model-qualified
  rooms, in a `links` list beside `zones` in the same document. `cost_ft` is the
  walking-equivalent feet for that hop (default 40, an assumption and not a
  measurement); it is per link because a lift hop and a stair hop differ.
- **Applied when it can be, reported when it cannot** (`connections.links[]`): a
  room that is no longer in scope, both rooms on one storey, or a pair a door,
  zone or earlier link already joins. It also reports `levels_between`, the number
  of storeys with rooms lying strictly between the two rooms: above 0 means the
  link skips storeys. That is reported, not refused, because the person drew it.
  **On a project whose buildings interleave their levels** (the largest one has
  car-park levels between hospital floors) a perfectly good floor-to-floor link can
  report a skip, so read it as a prompt to look and not as an error.
- **Routing:** the step has `kind: vertical` and a `link_id`, its length is the
  stated cost, and the polyline breaks into one segment per level so each viewer
  zone draws its own slice.
- **Editor:** pick the lower room, change the zone's level picker, pick the other
  room, Save link. A third pick replaces the second. Nothing is added in bulk.
- **MCP:** no new tool. `list_connections` returns both lists, and
  `get_connectivity` describes links and the `connections.links` block.

**Built: suggest the rooms stacked above and below a picked one** (the stack view, see
"Where this stands"). The signal is plan overlap scored relative to the smaller room, boosted
by a shared name stem, only the nearest storey with something over the room in each direction,
ranked with the overlap shown, so the person confirms rather than trusts. A room with no
overlapping candidate (a stair that moved) stays manual. `MIN_CANDIDATE_OVERLAP` is 0.3 and a
candidate stands alone ("clear") from 0.7.

## Horizontal path: methods, sources and what is built

**Status: built, with two methods and room to add more.** The route used to run
from room centre to door to room centre, which is not a shortest path. It is now
a parameter (`method`), chosen in the route bar, listed by the server with each
method's source, and defaulting to the recommended one.

### The problem, as the literature states it

Liu and Zlatanova (2011) name the fault precisely: networks built on room centres
(the dual-graph and cell-centre family: Lee 2001, Lorenz, Ohlbach and Stoffel
2006) do not represent natural movement, which is "looking for the direct (always
along with the shortest) way". A route through a centre-based network walks to the
middle of rooms nobody needs to enter and takes unnecessarily tortuous paths. It
also bends at every centre, so it overstates distances and draws detours.

### Methods found, and where each stands

| Method | Source | Gives the shortest path? | Status |
|---|---|---|---|
| **Door to door**, exact shortest walk inside each room | Liu and Zlatanova 2011 (ISPRS Gi4DM); visibility graph after Lozano-Perez and Wesley 1979, de Berg et al. ch. 15 | Yes, inside each room's outline | **Built, the default** |
| Room centres | Lee 2001; Lorenz et al. 2006 | No | **Built, the baseline** |
| Door to door with wall clearance (keep a margin off walls) | Motivated by the human-walking critique in the same literature | No, deliberately | Not built; the natural next option |
| Medial axis / straight skeleton (S-MAT) | Eppstein and Erickson 1999; Lee 2004 (both as cited by Liu and Zlatanova) | No | Not built; good in narrow corridors, distorts open space |
| Hybrid: straight skeleton plus visibility | Clementini and Pagliaro 2020; Mortari et al. 2019 (cited by D'Orazio and Clementini 2020) | No | Not built |
| Funnel over a triangulated room | Lee and Preparata 1984 | Yes, for a room without holes | Not built; same lengths as the visibility graph, only a speed question |
| Space subdivision (Delaunay) with obstacles | Xu, Wei and Zlatanova 2016 (ISPRS Archives XLI-B4) | Compatible with the visibility graph, per the authors | Not built; matters once furniture is modelled |
| Theta*, any-angle on a grid | Daniel, Nash, Koenig and Felner 2010 (JAIR 39) | Approximate | Not built; for open, furnished space |

**What was read and what was cited.** Liu and Zlatanova 2011 and Xu, Wei and
Zlatanova 2016 were read (the first in full, the second its method, review and
conclusions); the Theta* abstract was read. Lee and Preparata, Lozano-Perez and
Wesley, de Berg et al., Eppstein and Erickson, and the two hybrid papers are cited
through those papers and through search results and were not read here: check them
before leaning on a detail.

### Why door to door

It is the one that answers the stated problem and uses what the data already has:
door positions, and room outlines with their columns as holes. It is exact where it
claims to be, simple (a visibility graph over each room's reflex corners), and
two-level in the way the paper describes: the connection graph says WHICH rooms,
the room outlines say WHERE. The others either are not shortest (medial axis,
hybrids), need furniture or a grid this data does not have (Theta*, space
subdivision), or give the same lengths for more machinery (the funnel).

### How it is built

- `service::geodesic`: the exact shortest walk between two points of one room,
  around its corners and columns. The visibility test cuts a segment where it
  meets any edge and checks each piece, because an exact "covers" predicate
  rejected door points that sit on the outline to within rounding and turned most
  real routes into straight-line fallbacks.
- `service::routing`: the method registry (`Method`, `CATALOG`) and the search.
  Doors, zone crossings and level links become ports (two per connection, one in
  each room), joined inside a room by the exact walk and across a wall by the short
  crossing; A* with a straight-line estimate, used only when no edge is cheaper
  than the plan distance it spans (checked for level links).
- **Adding a method** is a variant of `Method`, an entry in `CATALOG`, and a
  function. The graph, the HTTP parameter (`method`), the MCP parameter and the
  viewer's picker do not change shape: the picker is built from what the server
  lists.
- A route starts and ends at each room's centre when the centre is inside the room
  (so the methods are compared on the walk and nothing else), else at a point
  surely inside it, **unless the caller gives a point** (next section).

### Start and end points inside a room (built)

A route no longer has to start and end at a room's centre. An endpoint takes an
optional plan point (`from_x`/`from_y`, `to_x`/`to_y` over HTTP and MCP, both
coordinates or neither, in the same project-local frame the route's `start`, `end`
and polylines are answered in).

- **Door to door** starts its walk at the point; the **room-centre method**
  re-measures the legs that touch it, so both honour it and stay comparable. Two
  points in one room are the straight line (centre method) or the exact walk
  (door to door), and no door is crossed.
- **A point outside its room is moved onto the room's outline and the answer says
  so** (`path.note`), rather than refused: a click is imprecise by nature and a
  refusal would be a poor answer to it.
- **The answer carries `path.start` and `path.end`**, the points actually used, so
  the viewer draws the marks where the route really began.
- **In the viewer, a click on the plan sets the point; anything else (a grid row, a
  search chip) has none and uses the centre.** The bar shows "at your click" with a
  "centre" button to drop it. The renderer answers a click in its flipped world, so
  the viewer negates Y before sending it; the click is carried through the pick list
  too, so choosing among stacked elements keeps the spot.
- **Why it matters:** between nearby rooms the walk inside the first and last room
  dominates the distance, so the point changes the route (on the largest project a
  clicked end point changed a 5-door route to an 8-door one one foot shorter). On
  long routes it changes little.
- **Not built:** dragging a mark to adjust a point after the click.

### Measured (the largest project, 24 random routes within the largest component)

- Door to door was shorter at the median (5% to 39% by sample), up to 74% shorter
  on a single route, and one route came out equal. **It can be longer than the
  centre method**, legitimately: a straight line from a centre to a door can cut
  through walls the walk has to go round.
- No route fell back to a straight line; a request cost about 0.15 s more than the
  centre method (a debug build, dominated by assembling the graph).
- Limits worth stating: the walk is the geometric shortest inside the outline, not
  where people walk; furniture is not modelled; a room whose outline is missing or
  has more than 160 reflex corners is walked in a straight line and the answer says
  so in `path.note`.

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
3. **Distance through centroids was a rough walking length, which is why routing
   is now a method.** A centroid can lie outside an L-shaped room, and a straight
   line from centroid to door can cross walls. It was a ranking figure, not
   something to quote. The door-to-door method (see "Horizontal path") replaced it
   as the default with the exact shortest walk inside each room's outline; the
   figure is still an estimate, because people do not walk the geometric shortest
   path and furniture is not modelled.
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
