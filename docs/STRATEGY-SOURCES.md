# Roommate — Sources

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Server](STRATEGY-SERVER.md) · [Browser](STRATEGY-BROWSER.md) ·
[MCP](STRATEGY-MCP.md) · [Authored](STRATEGY-AUTHORED.md) ·
[Security](STRATEGY-SECURITY.md)

Everything that supplies raw data into the pipeline: the Revit/pyRevit
producer, and dRofus (external reference data, today's only other source).
Two different origins, same discipline — extract raw, let the server
interpret. See the [Index](STRATEGY.md) for the "Revit extracts, Rust
processes" principle and its disciplines (extract-dumb, ElementId
stringification, schema versioning), which apply to every source, not just
Revit.

## Implemented

- **Room properties: a flat, source-native map (v5).** `Room.properties:
  Map<String, { value, storage_type }>` — one bag per room, keyed by whatever
  property name the producer's own source uses. Replaces the original v3
  typed `builtin`/`custom` split, which assumed Revit's parameter set was a
  fixed, guaranteed schema — an assumption that stops holding the moment a
  second source (e.g. IFC, whose property sets are optional and
  exporter-dependent) becomes plausible. `CustomValue::as_f64` does lazy,
  best-effort numeric coercion, content-first, hint-guided.
- **Settings-driven canonical property mapping.** `[[builtin_properties]]`
  (`canonical`, `by_source: {source → raw name}`) resolves a stable name like
  `"Area"` to the right raw property name per producer, without a Rust code
  change — the seam that matters once names diverge across sources (a second
  producer, or a non-English Revit UI). No entry for a name/source falls back
  to matching the name verbatim, which is exactly today's single-source
  behaviour. Implemented server-side (`settings.rs`, `contract.rs`'s
  `lookup_property`; see [Server](STRATEGY-SERVER.md)), but it exists entirely
  because sources vary — that's why it's documented here.
- **`model.source`.** Every payload declares which producer created it (e.g.
  `"revit"`) — the key the mapping above resolves against. A plain string, not
  a closed Rust enum: adding a source is a settings-file change, not a
  recompile.
- **`room_boundary`: the extractor now reads one *document* setting.** Until
  this, the extractor read only elements. It now also reports the document's
  `SpatialElementBoundaryLocation` — whether room boundaries sit on wall
  centrelines or finish faces — as an optional envelope field, per model. This
  stays inside "keep the extractor dumb on purpose" (STRATEGY.md): reading a
  document option is *extraction*, not computation, and it is the second
  document-level fact to ride the envelope after `model_to_shared`. Why it has
  to come from the source rather than be configured: the source **already knows
  it authoritatively**, and a project can legitimately mix both regimes because
  each linked model carries its own setting — so a project-level declaration
  could only ever be a fallback, which is exactly what `[areas]
  boundary_location` is. Consumed by `service::areas` (which sizes its wall zone
  by it) and `service::adjacency` (which defaults its gap tolerance from it); an
  extractor that does not send it is a normal state, not a defect. Produced in
  `extractor/pyRevit/` (`room_mate.py` reads it per document, `post_rooms.py`
  maps it to the wire): Revit's four boundary locations collapse to the
  contract's two, because the only thing downstream cares about is whether
  neighbouring rooms tile or are separated by a real gap. See
  [Server](STRATEGY-SERVER.md).
- **Reference-source loader + join, generalized to N named sources.**
  `Sources.reference: BTreeMap<String, ReferenceSourceConfig>` — the map key
  IS the join namespace (`[sources.reference.drofus]`, `[sources.reference.doors]`,
  ...), replacing what used to be a single hardcoded `drofus` field. Each
  source's CSV is a two-header-row export read into a keyed map (`by_id:
  BTreeMap<String, ReferenceRecord>`, in `src/drofus.rs` — `ReferenceData`/
  `ReferenceRecord`, renamed from `DrofusData`/`DrofusRecord` when this
  generalized); joined onto rooms at `/rooms` response assembly as its own
  sub-object per source, leaving the stored snapshot raw. `RoomResponse`
  flattens both `room` and its `reference: BTreeMap<String, ReferenceRecord>`
  onto the same JSON object, so a room's wire shape carries one top-level key
  per *joined* source (`room.drofus`, `room.doors`, ...) exactly as the
  single-source shape did before — a project with only dRofus configured is
  byte-identical on the wire to before this generalized. `ReferenceOrigin`
  (`#[serde(tag = "type")]`) still has the same two variants per source —
  `File { path }` and `Upload` — an `Api` variant later still slots in with
  no other consumer touched. The loader itself is **byte-source-agnostic**
  (`load_reference_from_reader`, with path and bytes wrappers; the bytes path
  strips a leading UTF-8 BOM, which Excel CSV exports routinely carry and the
  csv crate does not strip) — which source feeds it is dispatched per entry
  in `bootstrap::load_project_bundle`, where the store is in scope. Row 2's
  non-link columns are also retained, as `reconciliation: BTreeMap<String,
  String>` (reference field label → the Revit property it corresponds to) —
  see [Server](STRATEGY-SERVER.md)'s data validation report, currently the
  only consumer and still scoped to the source literally named `"drofus"`
  (see the capability boundary below).
- **A reference source as an uploaded, snapshotted source (`type =
  "upload"`).** A project declaring `[sources.reference.<name>] type =
  "upload"` takes that source's data from `POST /projects/{id}/reference/{name}`
  (raw `text/csv` body, drag-and-drop on the settings page or any HTTP
  client — `/projects/{id}/drofus` is kept as a permanent alias of the
  `name = "drofus"` case, both routes hitting the same source-keyed handler);
  each accepted upload is stored as a dated snapshot in the `SnapshotStore`
  (`<root>/<project>/reference/<name>/<taken_at>.csv`), the latest one
  hydrated at startup and hot-swapped in after each upload. The snapshot id
  rides the shared upload envelope's rules via `?taken_at=` (see
  [Index](STRATEGY.md)). A project with the upload source but no upload yet
  is a legitimate "not configured yet" state, not a startup error — its
  `fields` get shape-only validation until the first CSV supplies a label
  set. `Milestone.reference_snapshots: BTreeMap<String, String>` (source name
  → pinned snapshot id) is the storage groundwork milestones use to pin a
  source's data as it stood at a milestone — dRofus was the first (and for a
  long time only) entry; it's now one key among however many sources a
  project configures.
- **Per-source, per-column type/QA declarations (`ReferenceSourceConfig.fields`).**
  Each reference source owns its own field list — a project with two sources
  (say dRofus and a door schedule) declares two independent column
  vocabularies, not one shared list. One declaration per column — `label`
  (matches row 1), an optional `type` (`string` default, `numeric`, or
  `date`), an optional `format` (required, and only meaningful, when `type =
  "date"` — a chrono strftime-style pattern, since a reference export
  typically hands dates back as formatted text, e.g. `"6/29/2026 5:01:01 PM
  +10:00"`, not a structured value), an optional `revit_format` (a second
  strftime pattern for the *Revit* side of a date comparison, when the room
  property renders dates differently from the reference column — absent
  means `format` covers both sides), and an optional `qa` override (`exact`
  forces string comparison even when both sides parse as numbers; `ignore`
  excludes the column from comparison *and* the coverage report entirely).
  Deliberately **one** table answering "what is this column" per source, not
  two: the QA override started life as its own standalone list
  (`drofus_field_overrides`/`CompareMode`) until a colour-rooms-by-date
  feature idea came up that needs to actually parse a column's type, not
  just skip it in QA — a second, separate "what is this column" table would
  only have drifted from the first, so the override was folded into this
  more general per-column declaration instead. `type`/`format`/`revit_format`
  now have their first consumer: [Server](STRATEGY-SERVER.md)'s validation
  report parses a `date`-declared column's values with the declared
  pattern(s) and compares the parsed instants instead of the raw strings
  (the colour-rooms-by-date viewer feature that motivated typing the column
  is still unbuilt, and the validation report itself is still dRofus-only —
  see the capability boundary below). Everything is validated at startup,
  per source: a `date` field needs a `format`, a `format`/`revit_format` on
  anything else is almost certainly a mistake, each pattern must be a valid
  strftime string, and every `label` must actually exist in that source's
  loaded CSV.
- **Transport: HTTP POST to localhost.** Revit add-ins run in-process on .NET;
  POST is simplest, most debuggable, language-agnostic, and the same
  `HttpClient` carries over to a future C# add-in. Alternatives considered:
  WebSocket (only if the server needs to push updates back), named pipe
  (lowest latency, more fiddly cross-language), file watch (crude but simple).
  The cost the split adds is **serialization overhead** — extract, JSON-encode,
  send, decode — almost always worth it for the decoupling, but the thing to
  measure on a huge model.
- **Snapshot id is the producer's to state, the server's to fill.** The
  upload envelope's `snapshot.taken_at` (an RFC3339 UTC date-time — see
  [Index](STRATEGY.md) "The upload envelope") may be omitted, in which case
  the server mints one at ingest and reports it in the response. The Revit
  producer keeps supplying its own deliberately: its timestamp says when the
  model was *read*, which receipt time can't. A future upload type with no
  meaningful read-time just leaves it blank.
- **Gzip + NDJSON streaming push (`post_rooms.py`).** FFE exports run >100 MB
  uncompressed, too large to hold as one JSON string client-side or buffer
  whole server-side. `post_payload_stream` (the path `room_mate.py` actually
  calls) never builds a second full `rooms` list or one giant `json.dumps`
  string: it gzip-compresses a line-delimited stream — one envelope line
  (`build_envelope`), then one line per room, translated (`translate_room`)
  and written as each is read off the duHast export — straight to
  `POST /rooms/stream`. Peak memory is therefore one room, not the whole
  export (see [Server](STRATEGY-SERVER.md)'s matching streaming-ingest note).
  The older fully-buffered `translate()`/`post_payload` pair (whole payload in
  one dict, one `StringContent` POST to `/rooms`) is kept only because it's
  what regenerates `settings/test_snapshot.json` and suits small/manual
  pushes — `translate()` is now `build_envelope` + a loop over
  `translate_room`, so both paths share one translation, not two to keep in
  sync.
- **The model→shared transform is stamped on the envelope, not per room.** The
  duHast export carries the shared-coordinate placement on *every* geometry
  object (`DataGeometryBase.translation_coord` / `rotation_coord`), but it's one
  document-level `ProjectLocation` fact repeated. `room_mate.py` reads it once
  per model (`get_coordinate_system_translation_and_rotation(doc)`), reduces it
  to the 2D affine, and puts it on the envelope as `model_to_shared` (see
  [Index](STRATEGY.md) "The upload envelope"); `translate_room` therefore
  deliberately drops the per-polygon copy, keeping room geometry raw model-space
  points. Because it rides the envelope, the streaming path carries it on line 1
  with no per-room scan. Georeferencing Phase 1 — see
  `docs/Superseded/HANDOVER-georeferencing.md`.

## Why sources need reconciling, not just parsing

A typed `BuiltinProperties` struct (v3) made sense while Revit's Room schema
was the only schema: Revit guarantees a fixed set of built-in parameters on
every Room, so a non-`Option` typed field was a correct, not just convenient,
model. That guarantee is *not* transferable to a second source. IFC property
sets (Psets) are optional and exporter-dependent — the same concept (e.g.
area) can live in `Pset_SpaceCommon.NetFloorArea` from one tool, be named
differently, or be absent from another. So "guaranteed present" stops being
true even for what feels like a core field.

That's why the wire shape moved to one flat, source-native map, with
reconciliation pushed to a settings-driven, per-source name table rather than
Rust types: a second source is a settings-file change (a new `by_source` entry
per canonical property, keyed by that source's name), not a new struct field.
The tradeoff is real — `properties.builtin.area: f64` was a compile-time
guarantee; a flat map with a runtime-resolved name is not — but that guarantee
was never something IFC (or any second source) could actually promise, so
keeping it in the type system was enforcing a fiction.

## Reference: the reference-source CSV format

Every `File`/`Upload` reference source is CSV, not JSON — this is a
format-of-origin choice each such source makes, not something JSON-native
inputs need. dRofus is the first instance and still the running example:

```
DrofusRoomId,   NetArea,     Department,  ...   ← row 1: dRofus property names
RevitDrofusKey, d_net_area,  d_dept,      ...   ← row 2: matching Revit param names
<key value>,    <value>,     <value>,     ...   ← row 3+: data
```

The two header rows are the join spec and must both be retained:

- **Row 2, column 0** names the Revit room property whose *value* holds this
  source's linking id — constant for the whole file, read once at load
  (`ReferenceData.link_property`).
- **Row 1** is this source's field labels — the display layer for the joined
  data, and retained in full as `ReferenceData.all_labels` regardless of
  whether row 2 mapped a given column (needed so [Server](STRATEGY-SERVER.md)'s
  coverage report can show an unmapped column as "not checked" rather than
  omitting it silently; its second consumer is `/rooms`' per-project
  `drofus_labels` — see Server and the capability boundary below — which
  serves the full column set to tabular clients that could otherwise only
  union per-room joined fields). Row 2's other columns are the Revit param
  names those fields correspond to, kept for reconciliation.

The link is a direct value match and each source's ids are unique, so the
loader builds a flat `Map<String, ReferenceRecord>` per source — no collision
handling needed.

**A capability boundary worth naming.** `/rooms`' `drofus_labels` (the *full*
column vocabulary, including columns no room matched) and
`/projects/{id}/validation`'s whole `ValidationResponse` (the QA coverage
report) both still resolve specifically against the source named `"drofus"`
— a deliberate scope-out when the settings/loader/join layers generalized to
N sources, not an oversight. A second configured source (e.g. `doors`) is
independently joinable, filterable, and comparable (`doors.Mark=101A` works
end to end), and both `static/index.html` and `static/comparison.html`
discover it from the rooms it actually joined onto — but neither gets a QA
coverage report, and neither can show a column that matched zero rooms in
the current scope (there is no `doors_labels` to fall back on). Generalizing
`drofus_labels`/`ValidationResponse` to a per-source shape is the natural
follow-up once a second source actually needs either.

**Design notes on the join:**

- **Store raw, join late.** The parsed map sits in server state; it's attached
  at `/rooms` assembly, never at load — keeps `/rooms` the raw-geometry
  endpoint and leaves the Revit snapshot untouched.
- **Separate sub-object, not merged into `properties` — a lifecycle
  decision.** dRofus will eventually be polled mid-session for fresh data,
  independent of the Revit push. Fusing it into `properties` would couple two
  different-lifecycle things into one bag; a separate sub-object keeps the
  seam where that future refresh boundary actually is.
- **Unmatched key is a signal, not an error.** A room with no linking value
  just gets no dRofus data. A key present on the room but absent from the map
  is a useful mismatch — the two exports saw different model state, same
  diagnostic role as the room↔level join below.
- **A joined source is queryable under its `[sources.reference.<name>]`
  key.** `/rooms`' property filter (see [Server](STRATEGY-SERVER.md))
  namespaces a predicate's field as `<source>.<label>` — `drofus.NetArea>20`
  — where `<source>` is exactly a key of `Sources.reference`, so "what goes
  before the dot" has the same answer as the settings file. Milestone
  comparison's `comparison_key` / `comparison_properties` (see Server) are
  the **second consumer** of the same vocabulary — a name that filters
  correctly also compares correctly, resolved through the same
  `rooms::split_namespace` / `rooms::resolve_presence`, never a
  re-derivation. **The extension point a new source touches is one line of
  settings, nothing else in Rust:** the recognized-namespace set isn't a
  compiled constant anymore (the old `rooms::JOINED_SOURCES: &[&str]` is
  gone) — `SettingsRegistry::known_reference_sources()` computes it at
  settings-load time as the union of every `reference` key across every
  project's settings, and `split_namespace` / `Predicate::parse` /
  `presence_of` / `source_joined` all take that `&BTreeSet<String>` as a
  parameter instead of reading a global. `presence_of`'s single generic arm
  (`room.reference.get(source)`) already covers every source there ever
  will be, matched or not. The namespace is still reserved in the grammar
  rather than inferred — an unknown prefix is a parse error naming every
  known source (and, for the persisted comparison settings, a loud
  settings-load/save rejection), never a silent fallback to a room property,
  so a raw property literally named `Newsource.Field` can't quietly change
  meaning the day that source is added. The filter runs on the *assembled*
  room (after the join) precisely so a source's fields are reachable at all;
  consistent with "unmatched key is a signal", a room whose link value
  matched no record fails every predicate on that source, negative operators
  included — and comparison reports that unmatched state per room
  (`unjoined_sources`), not as one missing value per configured field. A
  project that simply doesn't configure a source another project does is
  **recognized but absent** for an unscoped query across both, not an error
  — same `presence_of` arm, no special case needed.
- **`[sources.reference.*]` currently means "reference sources *for
  rooms*."** The join, filter, and comparison machinery above all resolve
  onto an assembled *room* — nothing in this module joins onto any other
  entity. A door schedule needs a *doors entity* first, not just another
  entry under this table: it joins by a door key onto a door, not a room,
  and no door entity exists yet (`/rooms` assembles rooms and nothing else).
  Adding `[sources.reference.doors]` today would parse and load, then
  silently no-op — configured but joined nowhere. This table generalizes
  cleanly to *more room-keyed sources* (the extension point above); it does
  **not** generalize to *sources keyed on a different entity* without that
  entity existing first. See `docs/Superseded/HANDOVER-reference-sources.md`
  for the full two-axis breakdown.
- **The frontend discovers sources from the data, not a fixed list.**
  `static/settings.html` edits `Sources.reference` as a repeatable list of
  cards (add/remove by name — no reorder, since it's map-keyed, not
  ordered), each owning its own type/path-or-upload and `fields`.
  `static/index.html` and `static/comparison.html` never hardcode a source
  name: they detect which sources are present by scanning a room's own
  flattened keys for anything shaped like `{fields: {...}}` (see
  `detectReferenceSources` in `index.html`) — that's what the capability
  boundary above actually costs a second source: full-vocabulary
  (unmatched-column) discovery only works for the one source `drofus_labels`
  still names, everything else is discovered from what actually joined.

## Open items / things to watch

- **Extraction is the dominant cost (measured).** ~840 rooms exported in ~11s
  (~13ms/room) — normal-to-good for Revit boundary extraction, and almost
  entirely Revit API time: single-threaded on Revit's main thread because it
  must be. Serialization, POST, and server storage are milliseconds against
  this. The real optimization axis for the slow side is **extracting less or
  incrementally** (fewer params, skip unneeded rooms, pull only changed rooms
  since the last snapshot — the snapshot hierarchy leaves that door open), not
  server-side speed or language choice. Only worth attacking if near-live
  updates while modeling are wanted.
- **Room ↔ level join.** Each room's `level.id` must match an `id` in the level
  export. A mismatch surfaces as rooms landing on a fallback level named by raw
  id — a useful signal that the two collectors saw different model state.
- **Level ordering source.** The viewer's slider orders by the level export's
  `elevation` field (real elevations, in mm), not by `offset_from_level` (the
  room's offset from its level, which was always 0.0 and useless for
  ordering).
