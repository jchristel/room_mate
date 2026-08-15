# Roommate — Architecture & Strategy

The index to the strategy docs, plus the principles that govern what gets built
next. **These documents record outstanding work and the rules that constrain it,
not what already ships** — what ships is documented where it is implemented, in
`src/` module headers, `src-js/`, `extractor/pyRevit/`, and `CLAUDE.md` for the
invariants that are expensive to rediscover.

- **This doc** — the split principle that governs all three layers, and the
  disciplines that keep it clean.
- **[Sources](STRATEGY-SOURCES.md)** — the Revit/pyRevit producer and the
  reference sources joined onto an entity. Open: an API-polled origin, a second
  producer, incremental extraction — plus why extraction cost decides the
  optimization axis.
- **[Server](STRATEGY-SERVER.md)** — the Rust/axum process. Open: deferred
  endpoints and storage backends, an owning level above project, a coordinate
  datum shared across projects.
- **[Area calculation](STRATEGY-AREA-CALCULATION.md)** — what the area figure
  means and how it relates to IPMS 3 and DIN 277. Its own doc because it is the
  one place where the *definition of the output* is contested rather than read
  off the model, and two designs were reversed over exactly that. **Read it
  before quoting an area to anyone external.**
- **[Browser](STRATEGY-BROWSER.md)** — the plan viewer. Open: serving and then
  consuming the placement transform, level-of-detail, the framework fork — plus
  the hybrid renderer's coordinate/paint-order invariant.
- **[MCP](STRATEGY-MCP.md)** — the stdio MCP server. Open: resources and prompts,
  document tools.
- **[Authored](STRATEGY-AUTHORED.md)** — *nothing here is built.* How data the
  user authors — manual room connections, uploaded documents, their extracted
  text, and the hierarchy scopes that bind them — will be stored, pinned by
  milestones, and served. Read it before building any of that.
- **[Entities](STRATEGY-ENTITIES.md)** — what makes something a primary entity
  rather than a reference source, and what a second entity proved comes for free.
  Open: the door connectivity graph, design options, FFE.
- **[Security](STRATEGY-SECURITY.md)** — the threat model for the near-future
  shift from a loopback bind to a LAN-reachable server. Mostly unbuilt by design.
  **Read it before widening the bind past `127.0.0.1`.**

Implementation rules live in [Coding Conventions](CODING-CONVENTIONS.md).

A change touching more than one layer should update every doc it touches — that
is the cost of the split, and worth it for how much easier each doc is to read in
isolation the rest of the time.

## The core architectural principle: Revit extracts, Rust processes

The guiding split is that the Revit side does **only data extraction** and the
Rust side does **all processing**. The reasoning matters more than the rule:

- **Revit's API is the one thing that cannot be moved or parallelized.** It is
  single-threaded by design and must be called from Revit's main thread, via
  in-process IronPython (Python 2.7 on the CLR — interpreted, effectively no JIT
  for hot loops, no real threading). Whatever touches the live model is stuck on
  the slow side regardless of anything else.
- Therefore the win is to make that side do **as little as possible**: pull raw
  geometry and properties, serialize, hand off. Every piece of logic kept off the
  Revit side is logic that escapes the single-threaded, interpreted constraint.
- Rust is where the project is free: compiled, multicore, strongly typed, and
  decoupled from a Revit session. Processing server-side means geometry
  algorithms run without Revit open, are unit-testable in isolation, and can
  reprocess stored payloads without re-extracting.

### Disciplines that keep the split clean

- **Keep the extractor dumb on purpose.** Resist computing "just one thing" in
  IronPython because the data is right there. Every computed field there is logic
  in the slow language, untested, and duplicated if Rust needs it too. Extract
  raw inputs; derive everything downstream. (Reading a *document setting* is
  extraction, not computation — that is why the boundary regime rides the
  envelope.)
- **The contract carries raw data, not interpreted data.** Send coordinates, not
  computed areas. Send level ids and elevations, not pre-sorted orderings. The
  more the JSON is primitives, the less the two sides are coupled to each other's
  assumptions.
- **Version the schema, and know what forces a bump.** An *optional* additive
  field does not: a payload that was valid stays valid and means the same thing.
  A field that becomes **required at ingest** does, because a previously-valid
  payload now errors — which is exactly the test a bump exists for. Strictness
  belongs in the handler and permissiveness in the type, since stored snapshots
  re-parse at every boot and a hard requirement there would stop the server
  hydrating its own store.
- **Ids and `ElementId` values are 64-bit ints at the source, strings on the
  wire.** Revit 2024+ made `ElementId` 64-bit; IronPython 2.7 can truncate a
  large id across the CLR boundary — especially via the deprecated 32-bit
  `IntegerValue`, which fails *silently* with a wrapped number rather than an
  error. Read `.Value` and `str()` it at extraction, never touch `IntegerValue`,
  and parse to `i64` server-side only where the width is safe.
- **Every upload type rides the same envelope**, and resolves its snapshot id
  through the same `contract::ensure_taken_at` / `validate_snapshot_id` pair —
  never a reimplementation. A raw-body upload with nowhere to put a `taken_at`
  passes it as a query parameter and obeys the same rules. This is the rule that
  makes a *new* upload kind cheap, and it is the one most likely to be quietly
  broken by a kind that "doesn't quite fit".

### A caveat to stay honest about

**Before optimizing, measure where the seconds actually go.** Measured so far:
~840 rooms extracted in ~11 s, almost entirely Revit API time — so agonizing over
a 50 ms Rust algorithm while extraction runs for eleven seconds is the wrong end
of the pipe. See [Sources](STRATEGY-SOURCES.md) for what that implies for the
slow side.

The Rust performance argument stopped being theoretical once real geometry
arrived, and the way it played out is the template. Adjacency's naive O(n²)
pairing was left in place *until measured*, measured at ~22 s on a 5,000-room
level, and then given a uniform bounding-box grid — dropping it to ~2.5 s, most
of which is the shared room assembly it calls first. Rayon was still not reached
for: the grid removed the quadratic term, and threading a near-linear pass would
trade determinism for little.

Two related notes:

- "Old language" is not the real issue; *interpreted and CLR-hosted* is. The
  reason to reach for Rust is compiled performance plus real threads, not age.
- **Parallelism has a threshold.** Threading a few hundred rooms can be slower
  than a tight single-threaded loop once overhead is counted. Rayon pays off at
  scale — thousands of independent elements. Measure before parallelizing, and
  prefer removing an asymptotic term to adding threads.
