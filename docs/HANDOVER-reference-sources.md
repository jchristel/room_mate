# Handover: generalizing secondary/reference data sources

Context for picking this up in Claude Desktop. This is a Rust (axum) project —
"Roommate", a Revit → Rust → browser room viewer. The conversation was a design
review, not a code change. Nothing has been implemented yet.

## The question raised

Rooms come from a primary source (a Revit/pyRevit model) but are **augmented** by
a secondary source (dRofus reference data, joined onto rooms). Future entities like
doors will follow the same pattern: a primary model source, plus one *or more*
secondary sources (e.g. a door schedule **and** a door hardware schedule). So there
may be multiple distinct secondary sources.

Asked: what's a good way to define these? In settings? And how does the info flow?

## What the current strategy/code actually does

Reviewed `STRATEGY.md` and `STRATEGY-SOURCES.md` plus the relevant Rust.

- The **primary/producer** side is already generalized: `model.source` is a plain
  string (not a Rust enum), `[[builtin_properties]].by_source` maps a canonical
  name to each source's raw name, and the `/rooms` filter grammar namespaces fields
  as `<source>.<label>`. Adding a producer is a settings change, not a recompile.
- The **secondary/reference** side is generalized only *one* level: dRofus can be
  `File | Upload | (future Api)`. It is **not** generalized across *multiple distinct
  reference sources*. Evidence: `settings.sources.drofus` is a singular named field,
  `JOINED_SOURCES = &["drofus"]`, `drofus_fields` (not `<source>_fields`), a
  `DrofusSource` enum, `drofus_configured` on responses.

So "secondary source" is effectively hardcoded as *dRofus, singular*.

## Recommendation given

**Settings is the right home** (consistent with the existing "adding a source is a
settings change, not a recompile" principle). Promote the singular field to a keyed
collection where the key *is* the join namespace:

```toml
[sources.reference.drofus]
type = "upload"            # file | upload | api
join_key = "RevitDrofusKey"
[[sources.reference.drofus.fields]]
label = "NetArea"
type  = "numeric"

[sources.reference.doors]
type = "upload"
join_key = "DoorMark"
```

The map key (`drofus`, `doors`) replaces what today is hardcoded in three places:
`JOINED_SOURCES`, the `resolve_field` match arm, and the `drofus` response
sub-object name. All three become iteration over the map's keys.

### Flow, and what changes at each stage

Current: settings → `bootstrap::load_project_bundle` (dispatches File/Upload, loads,
validates fields) → stored in state → joined at `/rooms` assembly as a `drofus`
sub-object → queryable via `<source>.<label>` filter.

Generalized: each stage loops over N reference sources instead of one.
- **Loader** — `load_drofus_from_reader` is already byte-source-agnostic and
  two-header-CSV-shaped; generalizes cleanly *if* future schedules are also two-row
  CSVs. Decide early whether "reference source" implies two-header CSV or is
  format-open (door hardware as JSON would cost something here).
- **Fields** — `drofus_fields` → per-source field lists. The type/format/qa
  machinery is already generic; only the outer keying is dRofus-specific.
- **Join** — each source joins by its own key (see the boundary below).
- **Response** — `drofus_configured: bool` → a per-source status map.

## The key caveat — a two-dimensional generalization

There are **two axes**, solving different problems:

- **Axis 2 — multiple reference sources *for rooms*.** Free to generalize: the join
  mechanism (match a value on the room, attach a sub-object) is identical; only the
  settings key and field list differ. Buys dRofus + any other room-keyed schedule.
- **Axis 1 — the model isn't only rooms.** A door schedule joins onto a *door*, by a
  door key — not onto a room. But no door entity exists; `/rooms` assembles rooms and
  nothing knows what a door is. So a door schedule has nowhere to attach.

**The trap:** once settings reads `[sources.reference.drofus]` and `…finishes`, it
*looks* like `[sources.reference.doors]` would just work. It won't — the loader parses
it, but nothing joins door-keyed data, so it sits configured and silently no-ops.

**Where to draw the line:** building only axis 2 now is fine — doors can wait. But
**name the boundary in `STRATEGY-SOURCES.md`**:

> `[sources.reference.*]` currently means "reference sources **for rooms**." A door
> schedule needs a *doors entity* first — not just another entry under this table.

The clean end state is two-dimensional: (1) per primary entity (rooms, doors, …):
a model source + N reference sources; (2) per reference source: File/Upload/Api
origin, a join key, typed fields. Axis 2 is dimension 2; axis 1 is dimension 1.

## Constraints for whoever implements

- This is a Rust project — **add plenty of annotation/comments** to the Rust code.
- Keep answers short and concise.
- Every strategy change that touches a layer should update the matching STRATEGY doc
  (the docs are split along pipeline boundaries).

## Suggested next step (not yet started)

Draft the concrete `settings` refactor — `Sources` → a keyed
`BTreeMap<String, ReferenceSource>` — with annotations, turning `JOINED_SOURCES`,
the `resolve_field` arm, and the response sub-object name into map-key iteration,
plus a `STRATEGY-SOURCES.md` revision recording the entity boundary.
