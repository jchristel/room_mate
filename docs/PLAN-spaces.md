# RoomMate — Spaces implementation plan

> **Status: PR A run against RHH (kill condition CLEARED); B1, B2 and C built.**
> Spaces can be exported from Revit and pushed. Read, QA and viewer are still
> ahead, and the pyRevit button is owed outside this repository.
> The decisions below were written from a reading of duHast's source and are
> left standing rather than edited into agreement with the data;
> [As measured](#as-measured--rhh-2026-09-06) records what the probe found,
> including the three predictions it inverted and the one that would have made
> the first push silently wrong. Read that section before trusting any figure
> above it. Nothing else is built.
>
> **This records the design agreed
> before any code, so the implementation does not re-derive it and the open
> questions are open *before* the work rather than after — the discipline the
> phasing, windows and FF&E plans were all written under.
>
> **PR A is an instrument, not a feature, and its kill condition is real.**
> Spaces are the first entity whose boundaries come from a *linked* model, and
> the enclosure design below depends on a measurement nobody has taken. Do not
> start PR B before [the probe](#pr-a--the-probe-and-what-it-decides) reports.
>
> **Reviewed once, 2026-09-06**, which settled three of the nine critique items
> and changed two decisions. The check is **recurring**, so C8 resolves in this
> plan's favour; the key is **unique project-wide**, so C4's ambiguity guard is
> an invariant check rather than an expected finding; and the report compares
> **all rooms against all spaces, ignoring hierarchy**, which reversed C2 and
> gave D8 a tolerance it did not have.

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Entities](STRATEGY-ENTITIES.md) · [Sources](STRATEGY-SOURCES.md) ·
[Server](STRATEGY-SERVER.md) · [Browser](STRATEGY-BROWSER.md) ·
[MCP](STRATEGY-MCP.md) · [Conventions](CODING-CONVENTIONS.md)

## The demand

Verify, across a set of **services** models, that:

1. each model has spaces **at all**; and
2. every space matches a room on a linking key, agreeing on a **chosen** set of
   properties.

**Rooms and spaces live in different Revit documents** — rooms in the
architectural model, spaces in each services model, which bounds them against
the architectural model as a link. That single fact decides most of what
follows, because every existing entity-to-room join in this codebase is
model-scoped and this one cannot be.

## Why this is an entity and not a reference source

[Entities](STRATEGY-ENTITIES.md) sets a three-part test: extracted from the
model, carries its own geometry and identity, and other data joins onto it. A
space passes all three. The rejected alternative — exporting spaces to CSV per
model and joining them onto rooms as `[sources.reference.spaces]` — fails
requirement 1 outright and cannot be repaired: `ReferenceData` is one flat
`by_id` and one `link_property` **per project**, with no model dimension, so
"which model has no spaces" is not a question it can be asked. That is the
`has_room_snapshot` failure mode exactly: an empty thing reading as a present
one.

The CSV route stays useful as a **one-off probe** (see PR A), and as the answer
if the demand turns out to be a single audit rather than a standing gate.

## The finding: a space is a room, and duHast already exports one

Read from `SampleCodeRevitBatchProcessor-NET8/src/duHast`, the live copy.

| Fact | Room | Space |
|---|---|---|
| Collector | `Revit/Rooms/rooms.py::get_all_rooms` (`OST_Rooms`) | `Revit/Spaces/spaces.py::get_all_spaces` (`OST_MEPSpaces`) |
| Export | `to_data_room.py::get_all_room_data` | `to_data_space.py::get_all_space_data` |
| Record | `DataRoom` | `DataSpace` — `DataRoom` minus `ceilings`/`floors` |
| Geometry | `get_2d_points_from_revit_room` | **the same function**, on the SpatialElement |
| Phase | `BuiltInParameter.ROOM_PHASE` | the same parameter |
| Level, properties | `Element.Name`, `get_instance_properties` | identical |
| Type tier | none — a SpatialElement has no type | identical |

So the fifth entity is the *cheapest* one yet on the extractor side and the
**most expensive on the join side**, which inverts every previous entity's cost
profile. Doors, windows and FF&E were hard to extract and trivial to attribute
(same model, an authored reference, an id). A space is trivial to extract and
its attribution is a cross-model match on a value a human chose.

Two upstream hazards found while reading, both in duHast:

- **`get_all_spaces` catches every exception and returns `[]`.** "This model has
  no spaces" and "the collector failed" become the same answer — in the one
  function requirement 1 rests on. **U1, and it blocks PR C.**
- **`get_not_enclosed_rooms` has a misplaced paren**:
  `(boundary_segments == None or len(boundary_segments)) == 0` evaluates the
  `or` first, so the `None` case is never counted. Not on this plan's path
  (see D4), but it is the helper someone will reach for. **U2, not blocking.**

## Decisions

### D1 — `SnapshotKind::Spaces`, sharing the `Room` record

The envelope is its own (`spaces` element list, `SPACES_SCHEMA_VERSION = 1`),
because a stored snapshot names its element list after its own entity and every
file on disk already says so. The **record** is `contract::Room`, reused, per
"share it unless sharing would change a serde key" and "share it unless sharing
would make a field mean nothing": `id`, `name`, `level_id`, `loops` and
`properties` all mean for a space exactly what they mean for a room, and a space
has no type tier to leave permanently empty. This is the windows split
(`Opening` shared, envelopes separate), not the FF&E one.

`SnapshotKind::ALL` gains a fifth member; `position`'s exhaustive match is the
compile-time guard that catches forgetting it.

### D2 — The match is key-based and project-scoped, and that does not weaken the model-scoped rule

Every existing cross-entity join resolves a **room id**, which is unique only
within a model — hence the model-scoped discipline in `CLAUDE.md`. The
space-to-room match resolves a **user-chosen property value**, which is a
different mechanism with different failure modes (ambiguity rather than
dangling). Say so in the module header, so nobody reads this as licence to widen
the opening join.

Matching is project-wide across every model holding rooms. A key value appearing
on more than one room, or more than one space, is **ambiguous and reported**,
never guessed — `comparison::DuplicateKeyValue`'s precedent, which exists for
the same reason: an arbitrary user key carries no uniqueness guarantee.

### D3 — The reconciliation is QA, reusing comparison's matcher

`ValidationResponse` gains `spaces: SpaceReport`, beside `openings` and `items`.
It is a per-project reconciliation asking "do these two sets agree", which is
what `/projects/{id}/validation` is for. It is **not** an extension of
`/projects/{id}/comparison`: that is milestone-shaped (a baseline, pins, a star
diff), and pushing an entity axis through it is the same second-axis move
multi-phase comparison was rejected for.

What it reuses from `service::comparison` rather than rewriting: `index_by_key`
(the key index and its duplicate detection), `values_agree` and `field_config`
(numeric-adaptive comparison with per-field overrides), and
`rooms::resolve_presence` for the `source.property` vocabulary — so a
`drofus.`-qualified name is comparable on the room side with no new plumbing.

**The comparison is symmetric and hierarchy-blind:** every room in the project
against every space in the project, both unmatched directions reported as full
lists, and no scoping by building or level. A room's hierarchy is a room-side
concept that a space has no counterpart for (D9); introducing it here would
scope one side of a two-sided comparison, which is how a set difference turns
into a wrong answer rather than a smaller one.

### D4 — Unplaced spaces are dropped; placed-but-unenclosed are pushed

Confirmed the same is true of rooms today, and deliberately left alone:
`translate_room` returns `None` on an empty outer loop, and duHast's
`populate_data_room_object` returns `None` when it produces no polygon. **Both
drop unplaced and unenclosed rooms alike, and cannot tell them apart.** Changing
that would change the element count of every room snapshot ever taken; it is out
of scope here, and belongs in [Entities](STRATEGY-ENTITIES.md) if it is ever
wanted.

The distinction spaces need is **not in the export** — an unplaced space and an
unenclosed one both arrive with an empty outer loop. So the extractor reads it
from Revit, which is the "read what the export does not contain" case that room
references and facing direction already occupy: `Location is None` for unplaced
(drop), and `Area` plus `GetBoundarySegments` for the rest.

### D5 — Enclosure is **stated by the extractor**, never inferred from empty loops

```rust
pub enum Enclosure { Enclosed, Unenclosed, Unmeasured }
```

carried as `Option<Enclosure>` on the shared record — absent on rooms (which
never reach the wire in any other state), always set on a space.

Three states, not two, and the third is the point. `Unmeasured` is *the model
says this space is bounded and has area, and the pipeline produced no polygon
anyway* — a **pipeline** defect. `Unenclosed` is a **model** defect. Inferring
either from `loops.is_empty()` would fuse them into one sentinel, and the `±1e30`
scar says exactly what that costs: a value nothing can distinguish from a real
one arrives looking plausible. See [C1](#c1) — this decision started life the
other way round.

Empty `loops` still ride the wire, on `loops_from_polygon`'s precedent: a
geometry-less element still has properties and a key, and QA must see it.

### D6 — Geometric verification is out of v1, with a reason rather than a shrug

No point-in-room or boundary-overlap check between a space and its room in v1.
It requires `model_to_shared` to be numerically right across the architectural
and services files, and `RoomResolution::Project`'s own header records that
nothing has ever checked that transform against a real survey — two models that
never had shared coordinates set up emit identity transforms and stack silently.

What v1 *does* get from carrying the geometry: the viewer can draw spaces over
rooms, and a human sees the drift in a second. Automate it in v2, opt-in,
reporting disagreement and never overriding the key match — `room_resolution`'s
shape exactly.

### D7 — Presence: three states, and the empty push is the answer

Do **not** copy `reject_empty_rooms` (422), and do **not** copy the doors
producer's run-scoped empty-push refusal. Both exist because an empty push is
evidence of a broken producer; here it is the finding being bought. The report
distinguishes:

- **not audited** — the project knows this model, and no spaces snapshot exists;
- **no spaces** — a spaces snapshot exists and holds zero elements (the finding);
- **audited** — spaces present, match report follows.

That is `opening_report`'s pending-versus-dangling distinction on a different
axis: the difference between "never asked" and "asked, and the answer was none".
It only holds if U1 lands, because today a collector failure also answers zero.

### D8 — Settings: `[spaces]`, with **pairs** rather than names

```toml
[spaces]
comparison_key = "Number"      # the space property holding the link value
room_key = "Number"            # the room property it matches (defaults to comparison_key)

[[spaces.compared_properties]]
space = "Name"
room = "Name"                  # defaults to `space`
type = "string"                # FieldType, reused
qa = "exact"                   # CompareMode, reused

[[spaces.compared_properties]]
space = "Area"
room = "Area"
type = "numeric"
tolerance_pct = 30.0           # flag only past this relative difference
tolerance_min = 2.0            # ...and past this absolute one, in the property's own units
```

`comparison_properties` is a `Vec<String>` because it compares *one* vocabulary
across time. This compares *two* vocabularies, so a pair is the honest shape —
the third instance of "which canonical property names exist at all" not
generalising, after `Mark` on a door and `Mark` on a room. Empty by default:
nothing is compared until someone chooses, following `comparison_properties`'
own precedent.

**A sibling type, `SpaceFieldConfig`, not a widened `ReferenceFieldConfig`.**
`label` means nothing for a pair — you need two names — which is the same test
that made `Item` a sibling of `Opening` rather than a widening of it. If the
dRofus path later wants a tolerance it is a two-line copy; do not pre-emptively
widen the shared type for a need only this entity has stated.

#### The tolerance is new machinery, and its three edges are decided here

Nothing in the codebase has a tolerance today. `CompareMode` is `Exact` or
`Ignore`, and `numeric_match` compares at the **lesser stated precision** of the
two sides — which is right for a unit-conversion artifact (`"1.5"` against
`"1.49999935417"`) and useless for two independently measured areas. Room `Area`
arrives as a bare numeric string (`"113.89405096216444"` on the House A sample),
so it parses; it simply never agrees.

- **Denominator: the room.** `|space - room| / room`. The architectural model is
  the authored side and the one a services model is checked *against*, so
  "varies from the room by more than 30%" is measured against the room. Any
  symmetric denominator (max, mean) makes a stated 30% mean something different
  in each direction, which is worse than an arbitrary choice made loudly.
- **A zero room value is its own state**, neither pass nor mismatch. A relative
  difference against zero is undefined, and it is guaranteed to occur — an
  unenclosed space has `Area` 0 (D5), so the states this plan invents on one
  side meet the arithmetic on the other. Report it as `incomparable`, with both
  values, and let it be read alongside the enclosure finding that caused it.
- **Both thresholds must be exceeded to flag**, and `tolerance_min` defaults to
  0 so the simple case stays a plain percentage. The reason it exists is [C3](#c3):
  a percentage alone systematically flags small rooms, and a project's risers and
  cupboards are exactly what would fill the report.

The finding carries `space_value`, `room_value` and `delta_pct`, not a boolean —
a threshold nobody can see the distribution behind cannot be tuned, and tuning
it is the whole point of calling this a sanity check.

`ReferenceEntity` gains `Spaces`, so `[sources.reference.<name>] entity =
"spaces"` is a settings line — dRofus can then verify spaces and rooms as two
independent sets.

### D9 — `/spaces` is scoped by project, model, milestone and filter — not by `?building=`

A room's building comes from its own hierarchy properties. A space's would come
either from properties nobody has checked exist on the services side, or from
the matched room — which is a join, and a join is not a scope. Deferred with
that reason rather than guessed at.

### D10 — A fourth extractor entry point, and a fourth button owed

`spaces_export_entry`, one line over `export_entry(..., entities)`, plus one
`ENTITY_EXPORTERS` row. **Do not widen `rooms_export_entry`** — its button lives
outside this repository, so a change there keeps succeeding while silently
altering what every existing run pushes. A spaces run selects the services
documents, which is a different document set from a rooms run; nothing else
about a run changes.

The phase filter is the **rooms** one — `ROOM_PHASE` equality, not the door
range test — because a space belongs to one phase. `rooms_in_phase` generalises
to take a collector; running spaces through `elements_in_phase` would return
nothing, silently, which is what five empty pushes bought the knowledge of.

### D11 — A disagreeing spaces push is **quarantined**, not refused

Found while reading the ingest spine for PR B, and it is the one place spaces
must not simply reuse the openings path.

`check_opening_ingest` refuses a doors or windows push that names a different
phase from its lineage, and the refusal message says exactly why: *"activating it
would re-phase the model while its rooms stayed on the old phase, leaving these
openings' room references pointing at rooms from another phase."* **That reason
does not exist for a space.** A space carries no room id — it matches by a
user-chosen key, project-wide, across models (D2) — so re-phasing a spaces
lineage strands nothing. Reusing the openings check would emit a true-sounding
error whose stated cause is false.

Worse, it would trap this exact project. RHH's mechanical model keeps its spaces
in `Future`; if it were ever pushed once under `New Construction` — which the
one-phase-per-run rule makes easy, and which yields precisely one space — the
lineage would be phased and **no correct push could ever replace it**, because
the escape hatch for the openings rule is "re-phase the model with a rooms push
first" and a services model has no rooms to push.

So spaces take the **rooms** rule: quarantine (202) and promote. The rooms
justification transfers intact — a differently-phased push is a correct export of
a different phase, real data the user may want to switch the model to — and the
doors justification for refusing does not transfer at all.

An unphased push stays a 422 for every entity, unchanged: it was never filtered,
so there is nothing worth activating.

## Rejected, recorded so they are not re-proposed

- **Spaces inside the rooms envelope with a per-record discriminator.** The
  cheap version of D1. It breaks per-kind lineages and milestone pins, and makes
  "this model has no spaces" indistinguishable from "this model has no rooms" —
  destroying requirement 1 in the act of implementing it. The same reasoning
  that rejected `/openings?category=window`.
- **Spaces as a reference source on rooms.** Fails the entity test on the model
  dimension (above), and forecloses the "no rooms available" case a primary
  entity serves by construction.
- **Extending `/projects/{id}/comparison` to diff two entity kinds.** See D3.
- **Changing the room extractor to keep unenclosed rooms.** See D4.

## PR A — the probe, and what it decides

`scripts/probe_spaces_export.py` (collects, in Revit) and
`scripts/analyse_spaces_probe.py` (decides, offline), against **at least two
real services models**, with the architectural model's rooms captured in the
same pass so the key match is measured rather than assumed.

**Both are drafted and neither has met a Revit document.** The analyser has been
run end to end against synthetic input covering every state it classifies, which
is what caught it counting an unenclosed space's zero area as a 100% area
finding — the Q2 defect arriving a second time in the Q5 table, dragging the
median of the very distribution `tolerance_min` is chosen from. That class of
bug is why the analyser is a separate file that can be re-run offline.

Unlike its siblings, this probe is **multi-document**: every output is named
after its document, and an index file names the run. `probe_ffe_export.py` and
`probe_windows_export.py` write fixed filenames per entity, so a multiselect run
of either has the last document silently overwrite the rest. Harmless there,
fatal here — and worth fixing in those two when someone next opens them.

Per space, record: `Location is None`; `Area`; `len(GetBoundarySegments(...))`;
whether duHast produced a non-empty outer loop; the phase; the candidate key
value. Per model: total spaces, and whether the collector raised.

**Verdict lines, and the middle two matter most:**

| Verdict | Meaning | Consequence |
|---|---|---|
| `NO SPACES` | the document holds none | find another document; costs nothing |
| `BLOCKED` | spaces present, and a material share are `Unmeasured` — bounded, non-zero area, no polygon | **the kill condition.** Spaces are bounded by a link, and if duHast's `_outer_loop_via_solid` fallback does not carry that case, every space arrives geometry-less and D5 and D6 collapse |
| `KEYS WEAK` | spaces measure fine, but under ~90% match a room on the intended key | the entity is still right; the *key* is the open question, and the CSV probe answers that faster |
| `CLEAR` | measured, and keys match | PR B starts |

Also measure, because it is what the area threshold has to be **chosen** from:
the distribution of `|space - room| / room` across every matched pair, plotted
against room area. MEP spaces and architectural rooms are frequently computed to
different `SpatialElementBoundaryLocation` regimes, and half a wall thickness
across every pair is not a data error — it is a difference in what was measured.

The number to look for is **the room area at which that systematic delta crosses
30%**, because it is not a constant: the delta scales with perimeter and the
error with area. A 5 x 4 m room measured to wall centre instead of finish face
gains about 9%; a 1.2 x 1.2 m riser gains about 36% from the same 100 mm. So a
percentage threshold does not flag "the rooms that disagree" — it flags "the
small rooms", until `tolerance_min` is set from this measurement.

## PR sequence

| PR | Contents | Gate |
|---|---|---|
| **A** | the probe and its analyser | verdict `CLEAR` or `KEYS WEAK` |
| **U1** | duHast: `get_all_spaces` stops swallowing exceptions | before C |
| **B1** | `contract::spaces`, `SnapshotKind::Spaces` and its manifest index, `Enclosure`, `ReferenceEntity::Spaces` | `cargo test`, fmt, clippy |
| **B2** | `/spaces` and `/spaces/stream` ingest, the per-kind pending slot, and D11's quarantine | as above |
| **C** | extractor: `utils/spaces.py`, `exporters/spaces.py`, `post_spaces.py`, `spaces_export_entry`, one `ENTITY_EXPORTERS` row | the shipping translation over the captured RHH export |
| **D** | read: `GET /spaces`, filter grammar, MCP `get_spaces` (**tool count 20 to 21**, in `bin/mcp.rs`'s header and STRATEGY-MCP.md both) | |
| **E** | QA: `SpaceReport`, `[spaces]` settings, the match and the property diff | |
| **F** | viewer: a spaces layer and its toggle | driven in the browser, not read in the diff |

F is last for the reason the FF&E viewer PR proved: a fifth draw layer is where
two bugs appeared that the tests did not catch, and they were separable only
because nothing else landed with them.

## As measured — RHH, 2026-09-06

14 documents, walked as **links from a federated host** rather than as open
documents — the first correction, and it changed the probe: `pick_document` over
open documents finds none of these, so `resolve_documents` became a link walk
filtered by title prefix.

**10,570 spaces across four services models** (`ME` 1533, `HY` 3035, `FR` 3057,
`EL` 2945), one per service covering the whole building, against **3,119 rooms
across seven architectural models**.

### The kill condition is cleared, decisively

**10,395 of 10,570 spaces (98.3%) carry an exported polygon, and exactly one is
`Unmeasured`.** duHast's `_outer_loop_via_solid` fallback carries the
linked-boundary case, which was the single thing this probe existed to find out.
The rest divides as 114 `Unenclosed`, 3 `Redundant` and 57 `Unplaced` — all
ordinary findings at hospital scale. D5 and D6 stand.

Units also agree everywhere: every document exports `Area` in square metres
(ratio 0.0929 against internal square feet), so the 10.76x trap Q5 was built to
catch is not present on this project.

### What the data inverted

- **`GetBoundarySegments` is useless in a services model, and D4 leaned on it.**
  96.3% of spaces (10,182 of 10,570) report **zero** boundary segments while
  producing a perfectly good polygon, because their bounding elements are in the
  linked architectural model. The `Unenclosed`-versus-`Redundant` split D5
  describes is therefore unobservable there — the 3 `Redundant` all came from the
  one services model that has some native boundaries. **The extractor must
  classify enclosure from `Area` and the export's own polygon, not from segment
  count.** `Perimeter` exports as `0.0` for the same reason, and is not a
  substitute.

- **Matching cannot be one project-wide pool, which is what D2 said.** One
  services file per service means a room number legitimately names a space in
  each of them. Pooled, that reported **3,046 duplicate keys** and made the
  expected shape of the data into the loudest finding in the report. Per services
  model, the real number is **31** keys duplicated inside a single model — small,
  actionable, and exactly the invariant check D2 wanted.

- **"Every model holding rooms" is not the room authority.** `RHH-JHA-EL-MDL-HOS`
  holds 2,945 spaces **and 3,418 rooms of its own**, numbered `1`, `2`, `3`
  against the architects' `ENG137`. Pooled into the room side they are 3,418
  rooms that name nothing, and they buried the 44 architectural rooms that
  genuinely have no space. The room side needs an explicit scope — the analyser
  grew `--room-docs` for it, and the server will need the settings equivalent.
  This is the room-side scope [C2](#c2) said a space cannot supply, arriving from
  a direction C2 did not consider.

### The finding that would have made the first push silently wrong

**`RHH-JHA-ME-MDL-HOS` has 1,532 of its 1,533 spaces in a phase called
`Future`.** Every other services model is `New Construction`; the architectural
models use `New Construction` and `Future Expansion`. No two disciplines spell
the phase the same way.

`SnapshotEnvelope::phase` is **one per run, not one per model** — `choose_phase`
offers only names common to every selected document, by construction. So a run
over all four services models under `New Construction` pushes 3,035 + 3,057 +
2,945 + **1** spaces, and nothing anywhere says so: the producer's empty-push
refusal is run-scoped, and the run is very far from empty. That is the five
empty pushes again, silent and partial rather than loud and empty.

Two consequences, and neither is a contract change — the one-phase-per-run rule
is load-bearing and correctly argued where it lives:

- **A spaces run is per phase, so RHH needs two.** Recorded here so the first
  push does not discover it.
- **The push summary must report a per-model element count**, so a model
  contributing 1 of its 1,533 spaces is visible at the moment it happens. Cheap,
  producer-side, and it serves requirement 1 directly.

### What the key and the area threshold actually measure

Per services model, against the architects' rooms only:

| services model | spaces | distinct keys | duplicated within | matched | unmatched |
|---|---|---|---|---|---|
| `ME` | 1533 | 1507 | 3 | 1503 | 4 |
| `HY` | 3035 | 3016 | 11 | 3016 | 0 |
| `FR` | 3057 | 3037 | 11 | 3037 | 0 |
| `EL` | 2945 | 2936 | 6 | 2881 | 55 |

**The key works.** Blank keys: zero, in all four. `HY` and `FR` match every
single space to a room.

The inverse is the more useful number, and it is where requirement 1 lands: of
3,090 distinct architectural room keys, **`ME` names only 1,503 — 1,587 rooms
have no mechanical space at all**, against 53 for `FR` and 74 for `HY`. `ME` is
half a building behind, which is a finding no amount of per-space checking would
have produced.

[C3](#c3) predicted the area crossover and the data is almost exactly the
predicted shape — the median relative difference falls monotonically with room
size:

| room area (m2) | pairs | median delta % | over 30% |
|---|---|---|---|
| under 2 | 1889 | 14.0 | 244 |
| 2 to 5 | 2383 | 10.3 | 78 |
| 5 to 10 | 1873 | 7.9 | 32 |
| 10 to 25 | 2551 | 6.0 | 24 |
| 25 to 100 | 1191 | 3.8 | 30 |
| over 100 | 179 | 0.7 | 8 |

A flat 30% flags 416 pairs of 10,066, **244 of them (59%) sub-2 m2 rooms** —
which is the noise `tolerance_min` exists to remove. The analyser now sweeps the
combinations:

| `tolerance_pct` | min 0 | min 1 | min 2 | min 5 |
|---|---|---|---|---|
| 20% | 1019 | 396 | 163 | 92 |
| 30% | 416 | 183 | **127** | 81 |
| 50% | 138 | 94 | 86 | 56 |
| 100% | 44 | 44 | 43 | 25 |

**`tolerance_pct = 30`, `tolerance_min = 2` gives 127 findings from 10,066
pairs** — 1.3%, a report a person will actually read. The worst are not tolerance
questions at all: `ENG661` is a 0.62 m2 room against a 110 m2 space sharing its
number. 219 pairs are `incomparable` (zero room area), which is the state D8
gives them.

### What the probe got wrong about itself

`property_Area` came back as `"71 m2"` — `AsValueString()` is display-formatted,
unit-suffixed and **rounded to the whole number**, so it quantises every small
space by up to half a square metre before anything compares it. duHast's export
of the same space carries `71.27892877719862`, and that is what the server
stores. The analyser now reads compared properties from the export and ignores
the probe's own parameter reads; the probe's `property_*` fields are redundant
and should be dropped rather than fixed.

Cheap to say now, expensive if it had reached PR E: an area check calibrated on
rounded values would have set its threshold from the rounding.

## Self-critique

<a id="c1"></a>
**C1 — The first draft inferred "unenclosed" from `loops.is_empty()`, and that
was a sentinel.** It is cheap, needs no serde change, and is wrong for the one
reason that matters here: a space bounded *only by a linked model* is the normal
case in a services file, and duHast reaches for `_outer_loop_via_solid` exactly
there. If that fallback ever fails, an enclosed space arrives with empty loops
and is reported as a model defect it does not have. **This changed D5** from
derived to stated, and added the `Unmeasured` third state so the two causes stay
separated. It is also why PR A's kill condition is what it is.

**C2 — REVERSED ON REVIEW. The room-to-space direction is a full list after
all.** The draft argued it had to be counts-per-model, on the assumption that an
architectural project holds thousands of rooms of which only some are serviced,
so a flat "unmatched rooms" list would be noise that gets the whole report
ignored. **That assumption was wrong for these projects:** the two sets are
meant to be 1:1 across the project, so an unmatched room is a finding of exactly
the same standing as an unmatched space, and suppressing it to a count would
hide half the answer — which is `reference_unmatched`'s original lesson,
unmodified after all. Both directions are full lists; the per-model tallies stay
as a **summary beside** them, not instead of them.

Worth keeping from the wrong version: if a project ever *is* partially serviced,
this report degrades into thousands of unmatched rooms and stops being read.
That is a real failure mode, it is simply not this project's — and the fix, when
it is wanted, is a room-side scope, which is the one thing D9 says a space
cannot supply.

<a id="c3"></a>
**C3 — `Area` is the property everyone compares first, and a percentage
threshold alone flags the wrong rooms.** The review settled the *mechanism* (a
user-set threshold, "flag past 30%") and that mechanism turns out to have a
sharp edge: the boundary-regime delta scales with perimeter while the value
scales with area, so the same 100 mm costs a 20 m2 room ~9% and a 1.4 m2 riser
~36%. A single project-wide percentage therefore reports every small room and
nothing about whether its data is right. **This is what added `tolerance_min`**
to D8 and gave PR A its crossover measurement. It is not a reason to distrust
the threshold — it is the reason the threshold needs a floor underneath it.

**C4 — RESOLVED ON REVIEW: the key is unique project-wide.** The draft worried
that room numbers are usually unique per *building*, so two buildings with a
`101` each would make every `101` ambiguous and could dominate the report. Not
the case here. The ambiguity reporting in D2 stays regardless, and its
justification changes rather than disappearing: it is now a **cheap invariant
check** — if duplicates ever appear, something upstream broke the guarantee this
matching relies on, and that is worth hearing about immediately rather than
discovering as a wrong match. Composite keys stay deferred, now with a reason
rather than a hope.

**C5 — Requirement 1 has a hole this plan cannot close, and it should be stated
rather than papered over.** The server knows only the models that have pushed
*something*. A services model that has never pushed at all is invisible to
`SpaceReport` — it is not "not audited", it is not there. So the complete answer
needs both halves: the report answers "of the models this project knows, which
have no spaces", and the run's own summary answers "of the documents you
selected, which held none". Neither alone is complete, and this is the same
residual cost already recorded for the doors producer.

**C6 — Three pyRevit buttons are now owed outside this repository** (windows,
FF&E, and now spaces). The server can be complete and correct and still
unusable. Sequence the button work explicitly with PR C rather than letting it
be discovered after.

**C7 — The cost profile inverts, and the estimate should not be read off FF&E.**
FF&E cost one `ENTITY_EXPORTERS` row and a new record. Spaces cost almost
nothing to extract and introduce the first cross-model value join, a new
comparison vocabulary, a tri-state enclosure concept and a per-model presence
report. The work is in PR E, not PR C, which is the opposite of every prior
entity.

**C8 — RESOLVED ON REVIEW: the check is recurring, so the entity is the right
shape.** The draft's honest position was that a one-off audit on hand-exported
models makes the CSV win and this plan over-engineering. It is a **standing
gate** across many services models and repeated issues, which is the condition
under which the entity pays for itself — the per-model presence answer, the
push riding the same run as everything else, and anything else ever being able
to join onto a space.

The CSV keeps one job it is better at, and PR A is where it does it: answering
"do these keys actually match" this week, on one export, before the schema is
committed to.

**C9 — `Option<Enclosure>` on the shared record is a field rooms never set**,
which is exactly the shape D1's own rule warns about. It is accepted here
because the field is *meaningful* for a room — rooms are unenclosed all the time
— and absent only because the room extractor drops them (D4). If D4 is ever
revisited, the field is already the right one.
