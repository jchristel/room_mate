# Roommate — Server

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Browser](STRATEGY-BROWSER.md) ·
[MCP](STRATEGY-MCP.md) · [Authored](STRATEGY-AUTHORED.md) ·
[Security](STRATEGY-SECURITY.md)

The Rust/axum process: what it stores, how it derives data at read time, and
how it's configured. Code is a library crate (`lib.rs`) split across `src/`
modules (`contract`, `settings`, `drofus`, `classify`, `state`, `storage`,
`bootstrap`, `service`, `handlers`, `settings_api`) plus two binaries —
`main.rs` (this HTTP server) and `bin/mcp.rs` (see [MCP](STRATEGY-MCP.md)) —
each module carrying its rationale in a header, all with unit tests.

## Implemented

- **Module split.** One `main.rs` refactored into per-concern modules.
  `lookup_property` sits in `contract` (next to the types it inspects) so
  `drofus` and `classify` depend on the contract, not on each other — both the
  dRofus join and the classifier call through this one function, so neither
  assumes a tier or a source. It resolves names through the per-source mapping
  described in [Sources](STRATEGY-SOURCES.md); this doc covers what consumes
  the resolved value.
- **Service layer.** The derive/assemble logic behind the read-side endpoints
  (dRofus join, classification, validation assembly) lives in `service/`, not
  in `handlers`. `handlers` is now a thin Axum adapter layer: extract params,
  call one `service` function, translate the result to HTTP. `service/` never
  imports `axum` — the seam is dependency direction, not a framework — which
  is exactly what let the MCP server ([MCP](STRATEGY-MCP.md)) call the same
  functions `handlers` does without touching this layer at all. Ingest
  (`POST /rooms`, `/rooms/stream`) has no derive logic worth sharing and stays
  entirely in `handlers`. See Superseded/HANDOVER-service-layer.md.

  **Deferred gap:** `service::validation::compute_validation` still resolves
  each room's dRofus link value with its own direct `lookup_property` call
  rather than going through `service::rooms::assemble_room`'s join —
  unchanged from before this extraction (no regression), but it means the
  "future features reuse the join" benefit HANDOVER-service-layer.md
  anticipates for F&E validation isn't wired up yet. Left alone deliberately:
  validation's duplicate-link-value detection and missing-vs-unmatched
  distinction are structurally different from "assemble one room for
  display," so routing through `assemble_room` now would mean either losing
  that distinction or paying for classification/label work validation
  doesn't use — a speculative abstraction with no current payoff.
  `assemble_room` stays private (`rooms.rs`-only) until F&E validation is
  actually being built and its real needs are known; at that point, widen its
  visibility to `pub(crate)` and decide there whether it fits.
- **Classification hierarchy.** N-tier `[[hierarchy]]` from settings,
  validated at startup (a tier naming neither `code_property` nor
  `name_property` is a startup error, and duplicate tier names are also a
  startup error — needed once a tier name like `"Building"` is looked up by
  name, not just position). Resolves a full-depth path per room with latching
  `undefined` fill once a tier runs out of data — never a truncated path, so a
  partially-classified room stays visualizable rather than dropped. Resolved
  fresh per request, not cached. No general `/hierarchy` endpoint yet
  (deferred), though the "Building" tier now has its own consumer — see below.
- **Project/building selection (`GET /projects`, `GET /projects/{id}/buildings`).**
  A single physical building is often split across multiple models (a subset
  of levels each, sometimes even split within one level), so the viewer needs
  to scope its view to one building's worth of models rather than everything
  ever pushed. Building has no identity or storage of its own — it's the
  hierarchy tier literally named `"Building"`, resolved via the same
  `classify_room` every room already goes through. `/projects` lists distinct
  projects across `all_snapshots()`; `/projects/{id}/buildings` resolves that
  tier for every room in a project and returns the distinct values (plus an
  "Unclassified" bucket for rooms where it didn't resolve), each keyed by an
  opaque token the browser echoes back rather than reconstructing. No tier
  named "Building" configured → `tier_configured: false`, not an error: the
  whole project is treated as one building. Distinctness is the `(code, name)`
  pair, so two buildings can legitimately share a name (different codes, or
  one resolved a name but no code — a represented state, since a tier resolves
  when *either* property is present); such entries carry `ambiguous: true` so
  a picker that renders names can disambiguate (the viewer appends the code)
  instead of showing two identical options. Nameless entries and the
  Unclassified bucket are exempt from the flag.
- **Identity envelope (v4 → v5).** Every payload carries `project` / `model` /
  `snapshot`; `model` also carries `source` (see
  [Sources](STRATEGY-SOURCES.md)). `SUPPORTED_SCHEMA = 5`, hard-required, no
  transition window. Ids are immutable/keys; names are display-only.
- **Snapshot id: RFC3339 UTC, omittable, echoed back.** The envelope is now
  explicitly the shared *upload envelope* for any future upload type (see
  [Index](STRATEGY.md) "The upload envelope"). `snapshot.taken_at` must parse
  as RFC3339 expressed in UTC (`contract::validate_snapshot_id`, 422
  otherwise — this one rule replaced the old per-character filename checks
  for `taken_at`, since no RFC3339 string can contain `/`, `\`, or `..`, and
  a non-UTC offset would corrupt lexical-max-is-newest ordering). A
  blank/omitted `snapshot` is resolved server-side (`ensure_taken_at`, UTC
  now at the producer's own microsecond precision) *before* validation, in
  both ingest paths; the ingest response carries `snapshot_taken_at` and
  `snapshot_id_generated` so a pusher always learns the id its follow-up
  uploads should attach to. The flag answers "did the server mint this id?",
  not "was a snapshot stored?" (that's `accepted`/`room_count`) — a producer
  that stamps its own `taken_at`, as the Revit one does, sees `false` on every
  successful push. Still v5: a pure relaxation, not a bump.
- **Snapshot history endpoints (`GET /projects/{id}/snapshots`,
  `GET /projects/{p}/models/{m}/snapshots/latest`).** The read side of
  snapshot identity: the first lists every stored snapshot id per model of a
  project (`{ models: [{ id, name, snapshots: [..asc], latest }] }`, soft
  empty for unknown/unregistered projects, same skip-on-read as
  `/projects`); the second answers just the latest id for one model — the
  "what do I attach this follow-up upload to" call — and 404s when there is
  none, since it names one specific resource. Backed by a new
  `SnapshotStore::list_snapshot_ids`: the manifest's `ModelEntry` now indexes
  each model's snapshot ids (`snapshots`, kept sorted, upserted per push), so
  listing history never opens the possibly->100 MB snapshot JSONs. Same
  reconciliation stance as `list_models` — filesystem wins: a file the
  manifest doesn't index (e.g. stored before this field existed) is included
  with a best-effort id recovered from its sanitised filename, a manifest id
  with no file is dropped, both warned. `MemStore` reports just its current
  latest (it keeps no history by design).
- **Multi-model store, keyed.** Snapshots keyed by `(project id, model id)`,
  fixing the multi-document overwrite bug. `/rooms` merges every model's
  latest into one flat payload by default; optional `?project=`/`?building=`
  query params (the latter matched against the same Building tier as above)
  narrow that merge to one project or building, and `?filter=` narrows it by
  room property (see the property-filter bullet below). Under an active building
  filter, a project whose hierarchy has no "Building" tier matches *nothing*,
  not everything — the caller asked for a building, and a project with no
  notion of one can't answer that question (a silently ignored filter used to
  leak such a project's entire room set into a filtered multi-project merge;
  `list_buildings`' `tier_configured: false` already tells a well-behaved
  client not to send the combination). A model contributes its
  `levels` only when it contributed at least one matching room when a
  building or property filter is active — levels are their own array from a separate
  Revit export, so a floor can legitimately have zero rooms of a given
  building right now yet still belong to it; with no filter, every scoped
  model's levels are included exactly as before. Levels are also
  deduplicated across the merge, scoped per project (two projects' "Level 1"
  never collapse into each other): a `Level.id` is only unique *within* its own
  model (same caveat as room ids), so two linked models defining "the same"
  architectural level would otherwise appear twice. Equal `name` and
  `elevation` — elevation compared with the same adaptive-precision rounding
  as the validation report's numeric comparison below, tolerant of
  cross-file float drift — collapse to one canonical level; every
  contributing room's `level_id` is remapped to it before serialization, so
  the level picker and room filtering agree on one id per real-world level.
  A dedicated per-model endpoint is still deferred.
- **Per-project, per-source reference label sets on `/rooms`
  (`reference_labels`).** The response carries each contributing project's
  reference column vocabulary — `all_labels` (every row-1 CSV label, mapped
  or not) plus `reconciliation` (label → the Revit property row 2 maps it to)
  — keyed by **project id, then source name**. Project first because the
  unscoped read merges every stored project and a source resolves per project;
  source second because two sources may both declare a label called `NetArea`,
  so the source has to be part of the address. Sourced from the same effective
  data the rooms were joined against, so a milestone's pinned snapshot reports
  the *pinned* column set, never current headers over pinned rows; a project
  with no reference source has no entry (absent, not empty, the
  `RoomResponse.reference` discipline). Additive, no schema bump
  (the viewer ignores unknown fields). Exists for tabular consumers — the
  source-data grid in `index.html` and `comparison.html`'s
  property datalist — which otherwise could only discover columns by unioning
  per-room `fields`, making a column that matched no room in scope invisible:
  precisely the column the coverage report shows as "not checked" rather than
  omitting.
- **Server-side property filter (`/rooms?filter=`).** Comma-separated
  predicates, all of which must hold: `?filter=Department=Cardiology,Area>20`
  (quote a value containing a comma). Operators `= != > >= < <= ~`, the last
  being a case-insensitive substring; `=`/`!=` inherit the stated-precision
  numeric tolerance the validation report uses, and the ordering operators are
  numeric-only (a non-numeric value is a no-match, not an error). Field names
  are *canonical* property names resolved through `lookup_property` — the same
  resolution the dRofus join, classification and the room label already use, so
  a filter means the same thing everywhere — plus `$name`/`$id` for the room's
  own fields and `drofus.<label>` for a joined dRofus field. **A room missing
  the field never matches, negative operators included**: "no Department" is
  not evidence that the Department differs, and for a joined source an
  unmatched link key is a signal, not a value. A malformed predicate is 400
  with the parser's message (`ServiceError::Invalid`, the first caller-fault
  input any read path accepts) rather than a silently empty 200 — the
  difference between a typo and a genuine no-match has to be visible.
  **This exists for programmatic callers, not the viewer**, which holds the
  whole payload and matches locally (see [Browser](STRATEGY-BROWSER.md)); the
  two matchers are deliberately different — free-text substring for a human
  eyeballing a plan, structured and typed for a machine asking a precise
  question.
- **Swappable persistence (`SnapshotStore` trait).** `FsStore` writes
  `<root>/<project-guid>/{project.toml, <model-guid>/<ts>.json}` — a two-way
  `project.toml` manifest, upsert-on-push (creates unknown project/model
  structure), full snapshot history (one file per push). The manifest is the
  *index* (readable without opening any snapshot), the snapshot files are the
  record — and list-reads reconcile the manifest against the directory tree,
  with the filesystem winning on disagreement, so a hand-edited or stale
  manifest can't hide models that exist on disk. A re-push with a duplicate
  `taken_at` is skipped with a warning rather than overwriting the snapshot
  it duplicates. `MemStore` keeps the in-memory behaviour (latest-only, no
  history) for `[storage]`-less/dev configs. A database is a future third
  impl behind the same trait.
- **Settings-file-relative paths.** Every relative path inside a settings
  file (dRofus CSV in a per-project file; storage root and test snapshot in
  `server.toml`) resolves against that settings file's own directory, not the
  process's current working directory — so the compiled exe behaves the same
  regardless of where it's launched from.
  (`static/`, served by `ServeDir::new("static")`, is the one exception: it's
  still cwd-relative, so the viewer page itself still needs the exe launched
  from the crate root, or `static/` copied alongside it.)
- **Sample dev config.** `settings/` holds a runnable example: `server.toml`
  (storage root + dev seed), `projects/sample-project.toml` (classification,
  an `upload` reference source, room label — one file per project), and a
  `test_snapshot.json` (a real v5 payload produced by `post_rooms.py`'s
  `translate()` against `test/Data/rooms.json`/`levels.json`) — `cargo run --
  --server-settings settings/server.toml --project-settings
  settings/projects` seeds and serves it with no manual POST needed.
- **Configurable room labels (`room_label`).** An ordered list of property
  names, resolved into `RoomResponse.label: Vec<String>` at response assembly
  so the viewer never hardcodes which fields it shows (see
  [Browser](STRATEGY-BROWSER.md)). `"$name"`/`"$id"` are intrinsic tokens for
  `Room`'s own fields (`lookup_property` only reads `room.properties`, so
  these can't go through it); anything else resolves through the exact same
  canonical/source mapping dRofus and classification already use. Defaults to
  `["$name", "$id"]` — today's label — so omitting the setting changes
  nothing. An unresolvable name just contributes nothing to that room's
  label, no startup validation needed. **Footgun worth knowing:** in TOML, a
  bare `key = value` after an opened `[[array-of-tables]]` section (like
  `[[builtin_properties]]`) attaches to that array's *last entry*, not back
  to the top-level table — and since `BuiltinPropertyDef` doesn't reject
  unknown fields, a misplaced `room_label` line is silently swallowed with no
  error. Top-level `Settings` keys must be declared before the first section
  header in a project settings file.

- **Project display name (`name`).** The settings file is where a project's
  human-readable name is *authored*; `project_id` stays the identity (matched
  against `RoomPayload.project.id`, and a storage path key), so the two can't
  be the same field — an id can't be renamed, a label must be. Optional:
  absent means the project displays under its id, which is what every consumer
  did before the field existed. Non-empty when present, validated at load —
  omitting the key is how you say "no name", so a blank one is a mistake, not
  a way to say it. The name reaches storage the same way it always did, via
  the producer: `/api/settings/projects` carries it, the pusher sends it back
  as `project.name` (see room_mate's `fetch_projects`), and the store's
  `project.toml` manifest mirrors it for `/projects` to serve. So the server
  never reads a name *out* of settings to answer `/projects` — that endpoint
  still reports what was pushed, and a renamed project shows its new name
  after the next push. Unlike ids, names are **not** unique across files:
  consumers that label by name disambiguate collisions themselves (the pyRevit
  picker appends the id, as `list_buildings` already flags ambiguous
  buildings).

- **Settings read/save API + UI (`/api/settings/*`, `static/settings.html`).**
  The per-project TOML files are editable from the browser: a settings page
  (sibling of the viewer, linked from its header) lists every file in the
  projects dir — a file that fails to parse still gets a row carrying its
  error, since this UI is exactly the tool you'd reach for to notice a rotten
  file — and edits identity, dRofus source, hierarchy, builtin properties,
  room label, and QA fields through a form. `settings_api.rs` mirrors the
  handler/service split inside one module: a transport-agnostic core over the
  projects dir (typed `SettingsError`) plus thin Axum adapters
  (`GET/POST /api/settings/projects`, `GET/PUT /api/settings/projects/{id}`).
  A source's label dropdowns are populated from its latest stored upload
  (`/projects/{id}/reference/{source}/latest`), not from a path dry-run — the
  `drofus-check` endpoint that did the latter is gone with the `file` origin,
  see [Security](STRATEGY-SECURITY.md). The TOML files
  remain the single source of truth: reads parse them fresh per call (no
  filename bookkeeping in `AppState`), and a save validates the candidate
  through the exact startup pipeline (`bootstrap::load_project_bundle`)
  before installing the file and hot-swapping the in-process registry — a
  file this API accepts can never fail the next boot, and a rejected save
  leaves the existing file untouched. For an `upload`-sourced project that
  validation includes the store: the `drofus_fields` labels are checked
  against the *latest stored CSV* (which is why `load_project_bundle` now
  takes the store), and saving before any upload exists is fine — shape-only
  validation until data arrives. An update cannot rename a project id;
  a second `is_default` file is rejected; saves are serialized end-to-end by
  a lock so the scan-then-write race is structurally impossible. Writes are
  HTTP-only — the MCP binary reuses the core's *read* functions but never
  writes (see [MCP](STRATEGY-MCP.md): separate process, so its write could
  not hot-swap this process's registry). Access control is the `127.0.0.1`
  bind, same trust model as ingest — see [Security](STRATEGY-SECURITY.md) for
  what changes once that bind widens to a LAN interface, including the
  settings-backup and rate-limiting invariants that keep this route's
  hostile-reachable write recoverable and bounded.

- **Data validation report (`GET /projects/{id}/validation`).** First real
  use of the pipeline surfaced a need to audit data quality, not just render
  it. Computed in one pass by the pure `compute_validation` (thin async
  wrapper does the `State`/`Path` extraction, same shape as
  `resolve_label_fields`):
  - Every room's `lookup_property` resolution against the source's link
    property (missing → `rooms_missing_link_value`); values grouped to catch
    a link value shared by more than one room (`duplicate_link_values` —
    ambiguous, so excluded from the remaining checks, since a shared link
    can't be uniquely matched to one room); each remaining room's value
    looked up in `ReferenceData.by_id` (miss → `rooms_unmatched`).
  - **And the reverse**: the source's own link values that no room resolves
    to (`reference_unmatched`). Every other check starts from a room and asks
    the source a question, so this is the only one that walks the source —
    which is exactly why it was missing until it was added deliberately. Its
    absence failed silently in the worst direction: a 200-row CSV joined
    against 50 rooms reported zero unmatched and read as clean. A value shared
    by several rooms counts as *matched* here and is reported once as a
    duplicate rather than twice in both directions. Bare link values, not room
    ids — there is no room, which is the finding — so these entries are not
    jump targets in the viewer and fill only the link-value column in the CSV
    export.
  - For a hit, every `(dRofus label, Revit property)` pair in the
    `reconciliation` map (see [Sources](STRATEGY-SOURCES.md)) is checked,
    unless that field's `drofus_fields` declaration sets `qa = "ignore"` (see
    Sources), in which case it's skipped entirely — not compared, not listed
    in `field_coverage` either, since that's a deliberate exclusion (e.g. a
    last-synchronised timestamp expected to always differ), not a coverage
    gap.
  - **Comparison is numeric-adaptive, not plain string equality.**
    `contract::numeric_match` parses both sides as `f64` and, if both parse,
    rounds each to the *lesser* of the two raw strings' stated decimal
    precision before comparing — dRofus's `"1.5"` agrees with Revit's
    `"1.49999935417"` (a unit-conversion rounding artifact) because dRofus
    only stated one decimal digit of precision, so disagreement past that
    digit isn't real. Falls back to exact (trimmed) string equality when
    either side isn't numeric, or when the field's `qa` override forces
    `"exact"`. No fixed epsilon anywhere — precision is inferred per
    comparison from the data itself, never configured.
  - **A `type = "date"` field gets a typed comparison of its own**
    (`date_match`): both sides are parsed with the declared strftime pattern
    (`format`, with an optional `revit_format` for when the Revit side
    renders dates differently from the dRofus column) — trying zoned
    datetime, then naive datetime, then bare date (midnight) — and compared
    by what they denote, so two renderings of the same moment don't
    false-flag. Two offset-aware sides compare as instants; a zoned side
    against a naive one compares the zoned side's *local* wall-clock reading
    (the naive writer most plausibly wrote local time); two naive sides
    compare directly. Same fall-back contract as `numeric_match`: if either
    side fails to parse, the comparison drops to the string path — the
    declaration is a hint, not truth.
  - **A string-equality mismatch gets one more check before it's reported:
    has the Revit side already lost the disputed character?** duHast's own
    export step (`Objects/base.py`'s `to_json_utf` → `Utilities/utility.py`'s
    `encode_ascii`) narrows every string to ASCII before it ever reaches this
    service, replacing anything outside `0x00`-`0x7F` with a literal `?` —
    e.g. an en dash arrives as `?`. dRofus keeps the original character, so a
    field that's otherwise identical false-flags on that one glyph alone. On
    a string-equality mismatch (an exact-mode field, the non-numeric
    fallback, or a date field whose values didn't parse),
    `ascii_narrowed` re-runs the comparison with the dRofus side narrowed the
    same lossy way; agreement there means the mismatch was purely an
    artifact of the export's encoding step, not real disagreement. A
    mismatch that merely *contains* a `?` without narrowing to full equality
    still fails. See `Superseded/HANDOVER_utf8.md`.
  - **The dRofus side is normalized the same way the Revit side always
    was:** a blank CSV cell reads as absent, not as a real empty-string value
    to compare against — otherwise a blank dRofus cell would false-flag
    against any real Revit value. A dRofus-side absence isn't tracked
    further: the dRofus export is the source of truth for whether a field
    has a value at all, so a field it never populated isn't this report's
    problem.
  - **Revit-side absence is split into two distinct severities** via
    `contract::PropertyPresence` (`Absent | Empty | Present`), used wherever
    `lookup_property`'s collapsed `Option<String>` isn't precise enough:
    `Absent` (the property was never extracted from Revit for this room at
    all) → `fields_absent_in_revit`, a likely mapping typo or a parameter the
    extractor never wired up, worth flagging loudly as a setup problem;
    `Empty` (the property exists but nobody filled in a value) →
    `fields_empty_in_revit`, an ordinary per-room gap. Both are only reported
    when dRofus actually has a value for that field — nothing on the Revit
    side to compare against yet isn't an error.
  - **`field_coverage`** answers "which dRofus columns does this pass
    actually check" — every `all_labels` entry (see Sources) except
    `qa = "ignore"`-declared ones, each flagged `checked` (present in
    `reconciliation`) with its mapped Revit property when so. Makes the
    previously-implicit "a blank Revit-name cell in row 2 means this column
    isn't checked" convention visible in the running server, not just
    legible from the CSV.
  - The report is **one section per configured reference source**
    (`sources`, keyed by source name), each with its own `link_property`,
    discrepancy lists and `field_coverage`, plus a cross-source
    `discrepancies` total for a collapsed header. Each source declares its own
    link property, so "which rooms resolved no link value" is a different
    question per source and cannot share a list.
  - An **empty `sources` map** (no reference source configured, none uploaded
    yet, or no registered settings) short-circuits to an empty report, not an
    error — same discipline as `tier_configured` for buildings.

- **Gzip request decompression + streaming NDJSON ingest.** FFE exports run
  >100 MB uncompressed. Two independent, composable changes: (1)
  `RequestDecompressionLayer` (tower-http) inflates any `Content-Encoding: gzip`
  request body before it reaches a handler — transparent, so an uncompressed
  sender still works unchanged, and neither `ingest_rooms` nor the JSON
  contract needed to change at all. (2) A new `POST /rooms/stream` reads the
  body as line-delimited JSON (NDJSON: line 1 is `StreamEnvelope` — everything
  in `RoomPayload` except `rooms` — every following line is one `Room`)
  instead of buffering the whole body with `Json<RoomPayload>`, so peak memory
  is one line, not the entire payload; rooms are still accumulated into a
  `Vec` before handing the assembled `RoomPayload` to the same
  `state.set_snapshot` the buffered path uses, so storage stays identical —
  only *parsing* is streamed. The buffered `/rooms` route now also carries an
  explicit `DefaultBodyLimit` (previously unset, silently capped at axum's
  2 MB default) sized well above the largest expected export, since
  `DefaultBodyLimit` measures the *decompressed* size; `/rooms/stream`
  disables the limit entirely and relies on streaming instead. See
  Superseded/HANDOVER-gzip.md / Superseded/HANDOVER-streaming.md for the
  full rationale.
  **Honest limitation carried over unchanged:** the streaming handler still
  assembles all rooms into one `Vec` before storing, so it doesn't help if
  even that in-memory room set is too large — the deferred next step is a
  `SnapshotStore::put_streaming` that writes rooms to disk as they arrive.

- **Milestones (`[[milestones]]` in project settings, `GET
  /projects/{id}/milestones`, `/rooms?milestone=`).** A milestone is a named
  date with data snapshots *explicitly pinned* to it (`attachments`: model id
  → snapshot `taken_at`), so the viewer can show the project as captured at
  that milestone instead of each model's latest push. Definitions live in
  the per-project settings TOML — not storage — because they're user-authored
  per-project metadata with the same lifecycle as hierarchy/room_label, and
  riding that file buys the whole save pipeline (validation, atomic install,
  hot-reload) for free; the settings UI edits them like any other section.
  Load-time validation: non-empty unique names (the name is the identity
  `/rooms?milestone=` matches on), a date that parses (`YYYY-MM-DD` or
  RFC3339), every pin a valid snapshot id — but NOT pin *existence*, which
  settings can't see; a pin to since-deleted data is a read-time skip+warn,
  same signal-not-error stance as an unmatched dRofus key. Read semantics
  in `assemble_rooms` follow the building-filter discipline: a project
  defining no milestone of that name contributes nothing, a model the
  milestone doesn't pin contributes nothing, and a pinned model's payload is
  the pinned snapshot loaded via `SnapshotStore::get_snapshot` — substituted
  *before* level dedup / building filter / reference join / classification, so
  every downstream step (and the building filter) composes unchanged. A
  milestone can also pin **a snapshot per reference source**
  (`reference_snapshots`, source name → `taken_at`, beside `attachments`):
  under that milestone, `assemble_rooms` joins each pinned CSV loaded from the
  store (`get_reference` + `load_reference_from_bytes`) instead of that
  source's current data, resolved once per (project, source) and memoised so an
  unscoped multi-project `?milestone=` merge never cross-joins one project's
  pinned data onto another's rooms. A
  pin whose snapshot is missing or unparseable falls back to the current
  dRofus with a warning — the same signal-not-error stance as a dangling model
  pin (the room is still served, just joined against current data). This kept
  the milestone substitution on its existing seam: it changes *which*
  `DrofusData` feeds the join, nothing downstream. The one remaining
  deliberate v1 limit: the **validation report stays latest-based** regardless
  of the milestone selection (it resolves its own dRofus link independently —
  see the Service-layer "deferred gap").

- **Colour plans (`[[colour_plans]]` in project settings) — stored verbatim,
  the server computes nothing.** A colour plan is a named, per-project
  room-colouring config the *browser* applies (see
  [Browser](STRATEGY-BROWSER.md)); the server's entire involvement is serde
  round-tripping through the settings save pipeline. It parses no property
  value for colour, computes no colour, and grows no `/colour` endpoint — the
  same "axum stays a pure JSON API" line that kept CSV export and QA rendering
  client-side; the viewer reads the plans via the existing
  `GET /api/settings/projects/{id}`. The one server responsibility is
  **light load-time validation** (`validate_colour_plans`, alongside the other
  settings-only validators in `load_settings`): at most one plan `active`, and
  a `Bands` colouring must be a sorted, disjoint partition (`[lo, hi)`, each
  band's `hi <=` the next's `lo`, open ends only at the extremes) — rejecting
  overlap/out-of-order loudly so the browser can do a simple ordered
  first-match scan; and a date-range `format`, when given, must be a valid
  strftime pattern (same dry-run as `drofus_fields`). Property names are *not*
  validated (source-native, vary — an unresolvable name just renders grey
  client-side, the `room_label` precedent). The mode/colouring enums are internally-tagged struct variants
  (`ColourMode` on `kind`, `Colouring` on `style`), which round-trip through
  toml exactly like `DrofusSource` — verified by `test_settings_toml_round_trip`.

- **Area policy (`[areas]` in project settings) — the two facts Revit does not
  know.** The opposite of colour plans: the server *uses* every field, so it
  lives on `ProjectSettings` beside `hierarchy_exclusions`, not in the
  client-only bucket. `measurement_standard` (IPMS1/2/3, DIN277, SIA416, BOMA,
  RICS) says what the reported number **means** — contractual, not derivable
  from any model — and the server computes nothing from it, only carries and
  echoes it. `max_wall_thickness` (feet) is the width above which a gap stops
  being a wall and becomes a void: a project judgement, and now the **single**
  declaration of a quantity that used to be a constant in each of two modules.
  `boundary_location` is a **fallback only**, for models whose extractor predates
  the envelope's `room_boundary`; a model that declares its own regime always
  wins, because the model is the authority and a project-wide guess must never
  override a per-model fact.

  Boot validation is loud and specific: an unknown standard fails in the TOML
  parse with serde naming the accepted spellings, and a non-finite, non-positive
  or absurd thickness fails with a message. **Zero is rejected**, deliberately
  and against the first draft of the design — it is the value a reader reaches
  for to mean "centreline", and conflating the two would let project policy
  override a model fact. The centreline case is expressed by the *regime*, which
  resolves the effective gap to zero on its own. (`adjacency`'s `?wall_max=0`
  request parameter is a third thing again: a caller asking a question, and still
  valid.) The whole block is defaulted and `skip_serializing_if`-elided, so a
  project file predating it behaves exactly as it did and gains no `[areas]`
  section on its next save.

- **Hierarchy area footprints (`GET /projects/{id}/areas`).** The first endpoint
  that does real geometry rather than property lookup: a footprint per hierarchy
  group, per level. **Its design has its own document —
  [Area calculation](STRATEGY-AREA-CALCULATION.md)** — because it is the one
  place in the pipeline where the *definition of the output* is contested rather
  than read off the model, and two prior designs were reversed over exactly that.
  Read it before touching `service::areas`. In brief:

  Each level builds a **wall zone** once —
  `(close(all rooms, gap/2) ∪ all rooms) − all rooms`, the set of gaps narrow
  enough to be walls and nothing else — and each tier takes
  `footprint(P) = rooms under P ∪ (close(rooms under P, gap/2) ∩ wall_zone)`.
  Because the zone contains no room, a footprint cannot reach into a neighbour's
  room; because a close at radius `gap/2` cannot bridge more than `gap`, a
  courtyard is never in the zone and needs no width test. The ownership rule —
  *a wall between two groups belongs to neither and fills at their common
  ancestor* — is not enforced on top of this: it is the strict part of
  `φ(X ∪ Y) ⊇ φ(X) ∪ φ(Y)`, and `φ` being increasing is what makes areas
  **exactly additive** (`parent = Σ children + newly enclosed`, asserted to
  0.05 ft²). Wall junctions are the one shape needing a withdrawal pass, which is
  the same rule applied to a square bounded by four rooms.

  **`gap` comes from the declared boundary regime, per level.** Centreline
  resolves to **zero** — rooms already tile, so no close runs and the whole
  artifact class is unreachable; finish face resolves to the project's
  `[areas] max_wall_thickness`. The regime is a model fact off the upload
  envelope (`room_boundary`, see [STRATEGY.md](STRATEGY.md)), resolved per level
  because level dedup can put two linked models on one level, and a disagreement
  widens to finish face. A declared regime contradicting the measured room gaps
  is **logged, never fatal**.

  Grouping reuses `classify_room`'s resolved path verbatim (the `undefined`
  bucket is a real group); the endpoint reuses `assemble_rooms`, so
  `?building=` / `?milestone=` scoping comes for free. Islands are kept —
  `MultiPolygon` at every tier. **Exclusions** (`[[hierarchy_exclusions]]`, on
  `ProjectSettings` since the server uses them): a `group` match withholds a
  group from every tier above it (still reported, `counted_upward: false`); a
  `rooms` match drops rooms before any geometry, and so drops the walls only they
  bounded, since the zone is built from the survivors.

  The number is an **aggregated room footprint** (room area + enclosed wall bands
  − genuine voids), not net area and not a standards gross — and on a
  finish-face model it is a **house convention** rather than IPMS 3 or DIN 277
  (STRATEGY-AREA-CALCULATION §6 has the wall-by-wall comparison and the caveat on
  setting `measurement_standard`). The standard and the per-level gap are
  **echoed on the response** (`measurement_standard`, `wall_gap_by_level`). One
  computation feeds both the plan overlay and the summary table; each island
  ships as an exterior ring plus interior void rings, so the browser's even-odd
  path makes a courtyard read as open.

  Geometry dependency: `geo` (`BooleanOps`, `Buffer`, `MultiPolygon`) — bevel
  joins, negative distance, no new crate. Measured before/after on `/areas`:
  `big-plate` (132 groups) 6.0 s → **1.12 s**; `sample-project` (10 groups over 7
  levels) 0.43 s → 0.71 s, since a level with barely one group has nothing to
  amortise the level-wide pass against. `scripts/check_areas.py` is the committed
  diagnostic, six checks across **every** level.

- **Room adjacency graph (`GET /projects/{id}/adjacency`).** The *second*
  geometry-processing service. Which rooms share a wall, and how much wall —
  node/edge out, so the browser draws a graph rather than deriving one. Scoped
  by `?building=`/`?milestone=` like `/rooms` and `/areas`, 204 on an empty
  store, single-level by decision (`level_id` sits on the **edge** anyway, so a
  future cross-level edge needs no type change). Deliberately **no `?filter=`**:
  a filtered room set would silently drop neighbours, and a graph that omits
  what a room touches is worse than no graph. `service::adjacency`.
  **The interesting part is the tolerance, and why it is a request parameter.**
  Revit's `SpatialElementBoundaryLocation` decides where a room's boundary sits,
  and both settings occur in real models: at **wall centreline** neighbours tile
  edge-to-edge and the gap between them is *zero*; at **finish face** they float
  inside their walls and the gap is the wall thickness. The algorithm handles
  both, and `?wall_max=` (decimal feet) is what spans them: at or near 0 for
  centreline, just over the thickest partition for finish face. **The default is
  now derived rather than guessed** — the envelope's `room_boundary` says which
  regime each model was drawn to, so an unrequested tolerance resolves to the
  project's `[areas] max_wall_thickness`, or to **zero** when every level in
  scope is centreline. ("Every", not "any": one finish-face level in a mixed
  scope still needs its walls spanned, and running it at zero would report its
  rooms as touching nothing.) That declared thickness is the same quantity
  `service::areas` sizes its wall zone by — it used to be two constants,
  `adjacency::WALL_MAX_FT` and `areas::MAX_WALL_FT`, holding one physical number
  in two modules and free to drift; both are gone. An explicit request still
  overrides everything, because the slider is a question a caller is asking, not
  a policy a project is stating. Hence
  **zero is a valid tolerance**, the acceptance band is closed at both ends, and
  a `COINCIDENT_EPS_FT` folds in the export noise that stops "coincident" edges
  being bit-identical. An out-of-range, negative, non-finite or unparseable value
  is **400 with a message** (`ServiceError::Invalid`, the read-path convention —
  422 is the *ingest* status here), never a silent clamp.
  Two things the naive algorithm gets wrong and this one does not: a room
  narrower than the tolerance (a shaft, a riser) would let its neighbours match
  straight *through* it, so an accepted pair is rejected when a third room
  contains the midpoint of the gap; and a Revit boundary is split at every
  bounding element, so one wall arrives as several collinear segments and a naive
  sum counts it two or three times — overlaps are merged per shared wall instead.
  The `revision` is **derived, not recomputed**: `assemble_rooms` already returns
  the snapshot-set revision, so it is hashed together with the effective
  `wall_max`. That is what makes a tolerance change register as new content, so
  the viewer's slider does not appear frozen.
  **This is the first service where the Rust-side performance argument stopped
  being theoretical.** The naive O(n²) pairing was left in place until measured
  (STRATEGY.md's discipline), measured at ~22s on a 5,000-room level, and then
  given a uniform bounding-box grid — the exact fix the "measure first" note
  reserves — dropping it to ~2.5s, of which ~2.3s is the shared `assemble_rooms`
  the endpoint calls first. Rayon was still not reached for: the grid removed the
  quadratic term, and threading a near-linear pass would trade determinism for
  little (STRATEGY.md "Parallelism has a threshold").

- **Milestone comparison (`POST /projects/{id}/comparison`,
  `static/comparison.html`).** A star diff of N milestones against one chosen
  baseline (never all-pairs): per compared milestone, the rooms added and
  removed relative to the baseline, and — on rooms present in both — value
  differences over a user-configured property set. Rooms are matched by
  `comparison_key`, a **user-chosen** property persisted in project settings,
  deliberately NOT the dRofus `link_property` and never a silent fallback to
  it or to room id: no key configured is an explicit
  `comparison_key_configured: false` result. Each side's rooms come from
  `assemble_rooms(.., milestone)`, so pinned model snapshots, level dedup, and
  the per-milestone pinned dRofus join all compose unchanged; a key value
  shared by two rooms on one side is ambiguous and excluded
  (`duplicate_key_values`), mirroring the dRofus duplicate-link guard. A read,
  but POST-shaped for its list input.
  **Comparable fields span joined sources**: `comparison_key` and
  `comparison_properties` resolve through `service::rooms`' one namespace
  vocabulary (`resolve_presence` — the same `split_namespace` /
  `JOINED_SOURCES` the `/rooms?filter=` grammar uses), so `drofus.NetArea`
  diffs the *pinned* dRofus values between milestones — dRofus drift, arguably
  the more interesting diff than Revit-vs-Revit. A room whose dRofus join
  exists on the baseline but not the compared side reports **one** per-room
  `unjoined_sources` entry instead of N per-property missing rows (losing the
  join is the change, and alone keeps the room in the report); a join *gained*
  on the compared side goes unreported, the same deliberate baseline-side
  enumeration asymmetry properties always had. The namespace half of both
  settings is validated in `bootstrap::load_project_bundle` (loud boot *and*
  save-path 422, per the shared pipeline) — a typo'd `drofuss.NetArea` used to
  yield an empty diff indistinguishable from "no changes", the silent no-op
  this closes; the property half stays unvalidated, free-text against rooms
  that may not be loaded yet. Value equality is `numeric_match` + trimmed
  strings for every source — the dRofus QA path's date/ASCII-narrowing rungs
  are chosen **per source**, because what counts as an artefact depends on
  the pipeline each side came through. An unqualified field is Revit-vs-Revit
  through the same export, so encoding/formatting artefacts are symmetric and
  cancel: numeric-adaptive, then trimmed string. A `drofus.`-qualified field
  is dRofus-vs-dRofus and gains a **date** rung ahead of those when the column
  is declared `type = "date"` — dRofus returns dates as *formatted text*, so
  two snapshots can render one instant differently if the export's format
  changed, which is a real difference this would otherwise report as a
  change. The shared, **symmetric** `contract::date_match` serves both this
  and the QA path; `validation::field_values_agree` is deliberately
  asymmetric (it narrows only its dRofus side) and is **not** reusable for a
  same-source diff. **The ASCII-narrowing rung is deliberately not applied
  to comparison at all** — it forgives duHast's `encode_ascii` step, which
  narrows *Revit* strings before they reach the server, while dRofus CSVs are
  uploaded raw and never pass through it; on a dRofus-vs-dRofus diff that
  artefact cannot arise, so the rung would only forgive genuine differences.
  `qa = "exact"` is honoured (it says *how* to compare a column);
  `qa = "ignore"` is not (it says whether the QA pass checks one, and
  comparison has its own explicit property list).

- **dRofus upload ingest (`POST /projects/{id}/drofus`) + snapshotted
  storage.** The previously-deferred dRofus-as-snapshotted-source (see
  [Sources](STRATEGY-SOURCES.md) for the source-model side). Raw `text/csv`
  body — the `/rooms/stream` raw-body precedent, no multipart dependency —
  with the snapshot id as `?taken_at=`, resolved/validated/echoed through the
  same contract functions as rooms ingest, and an explicit 32 MB
  `DefaultBodyLimit` (axum's default is a silent 2 MB). **Validate before
  store, order load-bearing:** the CSV is parsed and its labels checked
  against that source's declared fields *before* `put_reference` — a stored
  CSV is hydrated at every boot, so accepting a bad one would fail the next
  startup of both binaries. Storage:
  `<root>/<project>/reference/<source>/<taken_at>.csv` (same `:`→`-` filename
  sanitisation, `.csv` extension), indexed by a `reference_snapshots` map on
  the project manifest with the same filesystem-wins reconciliation as model
  snapshots; `reference/` is a reserved name `list_models` explicitly skips
  (else it would surface as a phantom model). Duplicate `taken_at`: skip + warn, reported as `stored: false`.
  The upload core lives in `settings_api` because that's where the mutation
  machinery already is: it runs under the same `SAVE_LOCK` as settings saves
  and shares their `reload_and_swap` tail, so an upload and a save can never
  race or diverge on how a registry is rebuilt — and the freshly-stored CSV
  goes live without a restart iff it is the store's lexical-max latest (an
  older backfill correctly doesn't displace a newer one). Bootstrap
  consequence: the store is constructed *before* the project bundles load,
  and `load_project_bundle` takes it as a parameter, since an
  `upload`-sourced project hydrates its dRofus data from the store. Read
  side: `GET /projects/{id}/drofus/snapshots` (soft-empty listing) and
  `GET /projects/{id}/drofus/latest` (parsed summary, 404 when none) via a
  new `service/drofus.rs`, both also exposed as MCP tools (see
  [MCP](STRATEGY-MCP.md)).

**Deferred (design settled, not built):** snapshot delete UI (the history
*query* now exists — see the endpoints above), per-model / `/hierarchy`
endpoints, DB backend, and an owning level above project. (The
colour-rooms-by-date-proximity viewer feature that used to sit here is now built
as the date-range colour mode — see [Browser](STRATEGY-BROWSER.md) — reading a
room's date property against the plan's own `near_date` / `format`, distinct
from `drofus_fields`' QA-side typed date comparison above.)

## Data model: project → model → snapshot → {levels, rooms}

The moment the server *stores* data rather than relaying it, "the latest
payload" stops being meaningful — latest *for what?* Stored data needs a key
saying which thing each snapshot is a version of. Without identity, two
buildings POSTed to the same server overwrite each other — the multi-document
overwrite bug, since resolved (see Implemented).

The committed hierarchy is **project → model → snapshot(timestamped) →
{levels, rooms}**. Each level earns its place; collapsing two of them forces a
later migration.

- **Project** — the human-meaningful container ("the hospital job"). Stable,
  long-lived, mostly identity + display metadata (name, number, client). Groups
  models that belong together. The level a user thinks in.
- **Model** — a single Revit file. One project routinely has several:
  architectural, structural, linked consultant models, each POSTing
  independently. This is exactly the `pick_document` multi-select case — each
  selected document is a *model* under one *project*. Collapsing model into
  project reintroduces the overwrite bug. The stable Revit identity (model GUID)
  lives here, since a GUID identifies a *file*, not a job.
- **Snapshot** — one timestamped push of one model. This is what makes it a
  *store* rather than a relay. Every export creates a snapshot; the model
  accumulates them. Keeping all (full history) vs. latest-only is a retention
  choice deferrable to later — but snapshot being its own level is what makes
  "this floor as it was last Tuesday" or "what changed since last push"
  *possible* without restructuring.
- **{levels, rooms}** — payload content scoped to a snapshot. Stays together for
  the fetch-lifecycle reason in [Browser](STRATEGY-BROWSER.md); the hierarchy
  over it is about identity and versioning, this layer is the geometry.

### Identity

Each level needs its own key, keying downward:

- **Project id** — stable, user-assigned or generated. Should be **globally
  unique** (a GUID-like key, not "project 1" scoped to nothing) — that lets a
  project be addressed, compared, or later re-parented under an owning entity
  without collision or renumbering, at no cost to take now.
- **Model id** — lean on the **Revit model GUID**: stable across renames,
  unique per file. Prefer it over file name (which forks the record on rename).
- **Snapshot id** — a timestamp is the natural key: an RFC3339 date-time in
  UTC, sourced from the export's existing `"date processed"` field so it
  reflects when the model was *read*, not when the server received it. A
  producer with no meaningful read-time may omit it and let the server mint
  one at ingest (receipt time is then the honest semantics) — the ingest
  response reports the resolved id either way.
- **Room identity is really *(model, room id)*** — raw Revit room ids are only
  unique within a model, so the same id can appear in two linked models. The
  hierarchy disambiguates them.

Keep **identity** (immutable, machine-chosen — e.g. the GUID) separate from
**display metadata** (mutable — name, number). Tie storage to the id, not the
name, so renaming in Revit does not fork the record.

### Cross-project operations, and whether a top level is needed

Comparing or moving data *between* projects does **not** require a container
above project. Those are *operations across peers*, not evidence of a shared
parent — modelling the verb (compare, move) as a noun (a new level) is the
wrong instinct. A container is justified only when things share a lifecycle or
ownership; "compare A to B" implies neither.

What cross-project operations actually need:

- **Stable, addressable identity per project** — already provided by the project
  id. Comparison and move are functions over two ids:
  `compare(projectA, projectB)`, or a move sourcing from one project id and
  writing to another. Peers reached by id, no nesting.
- **A common coordinate frame, for geometry.** The real subtlety, and *not* a
  hierarchy problem. Each project's rooms sit in their own Revit model space
  (own origin, own rotation). Comparing footprints or moving a room across
  projects is meaningless until they share a datum — a shared survey point or an
  explicit alignment transform between them. No amount of nesting solves this;
  it is a geometry problem that bites anyone assuming "same structure ⇒
  comparable." **The first half of the datum now exists:** each model may carry a
  `model_to_shared` transform on its envelope (see [Index](STRATEGY.md) "The
  upload envelope") that maps its room points from model space into the project's
  *shared* coordinate system — so the rooms of one project's linked models land
  in one frame. What that does **not** yet give you is a frame shared *across*
  projects: two survey-registered projects in the same CRS become directly
  comparable, but the general cross-project case still needs an explicit
  alignment. The transform is the enabler, deliberately shipped ahead of any
  comparison or map that consumes it (georeferencing Phase 1 — see
  `docs/Superseded/HANDOVER-georeferencing.md`); nothing numeric depends on it being present
  or correct.

**When a top level *is* justified:** a real owning entity emerges — a portfolio,
organization, or client that groups many projects, controls access, or is the
unit queried at ("all rooms across the hospital network"). That is a genuine
container with its own identity and metadata, driven by *organizational* need
(multi-tenancy, access control, rollups), not by the compare/move operations.
Absent that need, the level is dead weight. The committed structure blocks
neither path: cross-project operations can be added without a new level, and an
owning level can be added above project later without disturbing anything below
it — additive, like snapshot history.

### Storage shape

Sketched as a nested `Map<ProjectId, Project>` → `Map<ModelId, Model>` →
ordered snapshots, so future endpoints (`/projects`,
`/projects/{id}/models`, `/projects/{p}/models/{m}/snapshots/latest`) get their
URL structure for free. **As shipped, this diverges deliberately in two ways:**
(1) the store keys on a *flat* `(project, model)` tuple, not the nested map —
simpler, fixes the overwrite bug equally, and nesting only earns its place once
endpoints actually address projects and models as separate resources; (2) `GET
/rooms` merges every stored model into one flat payload so the current viewer
keeps working unchanged — a stopgap that flattens stored identity (raw room ids
collide across models), replaced by `/projects/{p}/models/{m}` once the UI
addresses one model. Both are additive to fix later, not migrations.

## Missing tier data is a first-class state, not an error

The project has two "mismatch" cases where a reference that *should* resolve
*doesn't*, and both are diagnostic signals that two data sources disagreed:
the room↔level mismatch (a room's `level_id` has no match in the level export
— see [Sources](STRATEGY-SOURCES.md)) and the dRofus key mismatch (a room's
link key is present but absent from the dRofus map — also
[Sources](STRATEGY-SOURCES.md)). Missing classification tier data looks
similar but is the *opposite* case: nothing disagreed, the room is simply
classified only partway down — expected, incomplete-by-design, not a broken
reference. So the rule: **assign the room to the highest tier it has data for,
and set every tier below to an explicit `undefined`**, never a truncated path.
Surfacing partial classification is a purpose, not a side effect — "which
rooms aren't fully classified yet" is exactly the useful view while a
classification scheme is still being built out.

**Staleness caveat:** resolved classification is a cache over a static
definition plus the current snapshot — once rooms re-push or dRofus re-polls
mid-session, it must recompute, the server-side twin of the dRofus join's own
staleness note.
