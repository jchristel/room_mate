# RoomMate — Windows implementation plan

> **Superseded — built and merged (PRs #88–#93, 2026-08-31 to 2026-09-04).** The
> live pointers are the code: `contract::openings` for the shared record,
> `contract::windows` for the envelope, `service::openings::OpeningKind` for the
> per-entity dispatch, and `room_m.post_entity` (named `post_openings` until
> FF&E generalised it) on the producer side. Kept as
> the record of *why* the design looks as it does — and, in
> [As built](#as-built), of the three things measurement changed and the three
> findings nobody predicted.

**Status: built.** Windows go Revit → extractor → server → storage → QA → MCP →
plan. This records the design agreed before any code, so the implementation did
not re-derive it and the open questions were open *before* the work rather than
after.

Part of the Roommate strategy docs: [Index](../STRATEGY.md) ·
[Entities](../STRATEGY-ENTITIES.md)

---

## The question this plan existed to answer

**What does a third primary entity cost?** [Entities](../STRATEGY-ENTITIES.md)
had claimed that the envelope, the store, the property machinery, the filter
grammar and the geometric room locator all generalize, and that only geometry
semantics, connectivity/ownership and the property vocabulary are per entity. It
named FFE as the test of that bet. Windows arrived first and tested it instead.

## The finding that shaped everything

duHast's window export is the door export, not a sibling of it. Diffed with
`door`/`window` collapsed to one token:

| File pair | Real differences |
|---|---|
| `data_door.py` ↔ `data_window.py` | none whatsoever |
| `to_data_door.py` ↔ `to_data_window.py` | copyright year, a dead import, a docstring typo |
| `doors.py` ↔ `windows.py` | windows lack the curtain-wall discrimination doors have |

So the plan's central recommendation was **generalise, do not clone**: a third
`SnapshotKind` over a shared record, not a parallel stack. Cloning meant copying
~440 real lines of contract, ~770 of read assembly and the door halves of
validation and comparison — the parallel-method-set failure `CLAUDE.md` names
about `put_doors`, at ten times the scale.

**Rejected, recorded so they are not re-proposed:**

- `/openings?category=window`. `SnapshotKind` is a storage *path component* and a
  deliberately closed enum; milestone pins are per-kind; the extractor pushes one
  bucket per entity. A query parameter keeps one lineage and breaks "a model
  pushes windows and no doors" as an independently versioned thing.
- Windows as a reference source on doors. Fails
  [Entities](../STRATEGY-ENTITIES.md)' own three-part test: windows carry their
  own geometry, their own identity, and things join onto them.

## The line that decided every subsequent question

**Share it unless sharing would change a serde key.**

That single rule settled the contract split (record shared, envelope not), the
read result types (`Assembled` → `DoorsResult` / `WindowsResult`), the ingest
responses (`IngestOutcome` → `door_count` / `window_count`), and the extractor's
push modules (`post_openings`, now `post_entity`, + an `OpeningPush` row each,
now `EntityPush`). It was overruled
exactly once, deliberately — see [As built](#as-built).

## What was measured before any code moved

The premise — "a window record is a door record" — came from reading duHast's
source, which is how the `±1e30` sentinel, the axis-aligned footprint and the
unresolvable `phase_id` all got believed before. So the first PR was an
instrument, not a feature: `scripts/probe_windows_export.py` (collects, in
Revit) and `scripts/analyse_windows_probe.py` (decides, offline).

Two documents, captured in one pass with one duHast: a house (48 windows, 26
doors) and a facade file that links its interiors (158 windows, 191 doors). A
full-depth key-path diff found **zero structural differences in either
direction**, and every field the plan assumed was populated for both.

---

## As built

Three of the plan's nine criticisms did not survive contact with the data, and
three findings arrived that nobody had predicted. Both lists are the reason this
document is worth keeping.

### What measurement inverted

- **Curtain-wall panels were a DOOR problem.** The plan's largest open risk was
  that duHast discriminates curtain-wall doors and has no window equivalent, so a
  facade model might report thousands of curtain-wall panels as windows. It holds
  25 curtain-panel symbols and 51 curtain-wall-hosted **doors** — and zero
  curtain-wall windows. No window filter was added, and the commit says why
  rather than leaving a silent gap.

- **"One-sided is the norm" was too weak.** The prediction was that windows would
  usually name a room on one side. In the facade file, **0 of 158 windows and 0
  of 191 doors** carried a reference on either side: Revit cannot resolve a room
  across a link, so it is structurally impossible rather than merely unpopulated.
  That promoted `[windows] room_resolution` out of "deferred" and into the
  server PR — without it, such a model's openings can never be attributed at all.

- **The door-glyph bug was not a bug.** The viewer PR had been sequenced last to
  avoid landing a second glyph set on top of an unexplained rendering failure.
  There was no failure; the constraint was lifted.

### Three findings nobody predicted

- **A hostless opening destroyed an entire export.** `Base.to_json` calls
  `json.dumps` with no `default=` over a `class_to_dict()` ending in
  `else: return obj`. An unhosted element gets an invalid `LevelId`,
  `get_level_data` calls `Element.Name.GetValue()` on nothing, and `encode_utf8`
  returns a non-string argument *unchanged* — so a CLR descriptor reaches
  serialization and raises, aborting the whole document. One skylight, two
  terrace sills. The guard walks and marks only the leaf; catching around
  `serialize_utf` would drop a real opening over one field.

- **duHast silently drops curtain-wall doors.** 14 of 205 collected doors were
  missing from a facade export, **every one a curtain-wall door family**; of 51
  curtain-wall-hosted doors only 37 survive. `populate_data_door_object` returns
  None for anything it cannot measure and the caller does not append it, so the
  loss is invisible from the export alone. Independent of windows, and still
  needing an upstream fix.

- **`level_id` needed an absent story doors never required.** Both models held
  hostless openings carrying `-1`. The contract keeps `level_id` a required
  `String`; what changed is that `locate` now answers `UnknownLevel` rather than
  `NoCandidate`, which means *the probe ran and found open air* — the ordinary
  answer for an external opening.

### The one place the serde-key rule was overruled

`/validation` changed shape: `doors: DoorReport` became
`openings: { "doors": {…}, "windows": {…} }`, one shared report under both keys
with the entity as **data**. Keeping the door key names meant a parallel
`WindowReport` — but the entity was not only in the report's own fields
(`door_id` in six element types, five more in `DoorDiscrepancyCounts`), so
mirroring it was ~250 lines that had to be kept in lockstep by hand and would
grow with every future finding. Keyed like `sources`, so a reader navigates both
halves the same way, and a third opening entity costs a map key rather than
fifteen types.

Blast radius was checked rather than assumed: the viewer reads none of those
fields, and the only in-repo consumer was the MCP tool description.

### What a third entity actually cost

| Shared, one implementation | Genuinely per entity |
|---|---|
| the `Opening` record | schema version (doors 2, windows 1) |
| read assembly, filter, scoping, milestones | the element key (`doors` / `windows`) |
| ingest sinks, commit, phase gate | ingest response naming |
| the geometric room locator | policy defaults, argued not inherited |
| the QA report and every finding type | the glyph, and what it means |
| the extractor's placement pass and push | one `OpeningPush` row each |

The bet [Entities](../STRATEGY-ENTITIES.md) made held. Windows needed the two
dependent-entity additions it named — model-scoped reference resolution, and the
phase envelope — and nothing else structural.

### Two things worth knowing that are not in the code

- **A tool description can be false without a word of it changing.** The window
  record is identical, so `get_windows` could have reused `get_doors`' text
  verbatim and every sentence would still have parsed as true. It would still
  have been wrong: an agent applying the doors reading to a facade model reports
  a correct model as broken. The rule now lives in
  [MCP](../STRATEGY-MCP.md).

- **The viewer bug the unit tests could not find.** 15 glyph tests passed, the
  geometry was provably correct, `debugState` reported the layer present — and
  the plan drew nothing, because the window mesh never got `setView`. Invisible
  to every test and to the type system; visible in one screenshot.

## Where the surviving open items went

Per [the archive rule](../README.md#archive), nothing live is left here. The
four items this work did not close moved into
[Entities](../STRATEGY-ENTITIES.md)' **Deferred** section, which owns them:

- the pyRevit button for `windows_export_entry`, still to be wired outside this
  repository;
- verifying the windows extractor against a live Revit document — the same
  standing the doors extractor has, and the reason the probe scripts exist;
- windows in milestone comparison, cut from the first pass with the pins already
  stored;
- the probe's curtain-wall symbol test, which disagrees with its host-based
  sibling and should not be relied on until diagnosed.

One item is not RoomMate's to close and so is recorded here only: **duHast drops
curtain-wall doors**, and needs an upstream fix.
