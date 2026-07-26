# HANDOVER — Area aggregation: declare the boundary location, then partition the wall zone

> **Superseded — the live document is
> [STRATEGY-AREA-CALCULATION.md](../STRATEGY-AREA-CALCULATION.md).** All three
> decisions here are built and merged, and the design they settled now lives in
> that strategy doc along with the remaining open items. Keep reading this one
> for *how the decisions were reached* — the artifacts that forced them, the four
> things the implementation learned that the brief did not know, and the measured
> before/after. Do not treat its "not started" language as current.

**Status: all three decisions built and verified against House A.** One item
open: nothing yet *sends* `room_boundary`. (When this was written the Revit
extractor was in another repository; it now lives in `extractor/pyRevit/`, so
the field is addressable here — see the root README) — the server accepts, resolves, uses and
echoes it, and every model currently resolves through the project fallback. See
the Definition of Done for what was measured. Written as a brief and kept as
one: the reasoning above the DoD is still the reasoning, not a retrospective.

**Four things the implementation learned that this brief did not know.** They
are recorded inline at the relevant sections too, but they are the parts worth
reading first:

1. **Connected components do not work as the assignment unit** (Decision 3). On
   a real floor the wall network is *one* connected component — every band meets
   every other at a junction — so "assign each component by the groups bounding
   it" sends the entire floor's wall area to the common ancestor of everything,
   i.e. nowhere. What the brief was reaching for is achieved instead by
   intersecting each prefix's *own* close against a per-level wall zone; the
   ownership rule then falls out of the arithmetic. See Decision 3 below.
2. **`resolve_sibling_overlaps` should be kept, not retired.** The brief lists it
   for deletion. It is still needed, for a reason the brief did not identify: at
   a wall junction where four rooms meet, the small square between them is
   bounded by all four, and two sibling groups can each legitimately close over
   it. That is the design's own "bounded by two groups, so owned by neither" case
   — so withdrawing from both and letting the ancestor claim it is the rule, not
   arbitration. It is now junction-sized rather than whole-footprint.
   `foreign_near` **is** deleted, as the brief says.
3. **The two regimes agree on the building, not on the department — and the
   reason turns out to be a standards question.** A centreline room's polygon
   already contains half of every wall bounding it; a finish-face room contains
   none. So redrawing a building to finish face moves half of each
   *inter-department* wall from the departments up to their common ancestor.
   Totals match; the tier breakdown does not. **That "half to each side" is
   exactly IPMS 3's rule for a demising wall** — so the centreline path is
   accidentally standards-conformant and the finish-face path is not. The two
   conventions differ only in how one class of wall is split between the two
   children bounding it, and the difference cancels one tier up. See the
   References section; making finish face conformant is an additive
   redistribution, not a rewrite.
4. **`max_wall_thickness` must reject zero** (Decision 2 says "non-positive
   fails", and it was right, but for a reason worth stating). Zero is the value a
   reader reaches for to mean "centreline" — and accepting it would let a
   project-wide policy override a per-model fact, which is exactly what Decision
   1 exists to prevent. Centreline is expressed by the *regime*, which resolves
   the gap to zero on its own. `adjacency`'s `?wall_max=0` stays valid: a caller
   asking a question is not a project stating a policy.

---

## The short version

`service::areas` aggregates rooms into per-tier footprints with a **morphological
close** at a hardcoded radius (`MAX_WALL_FT / 2`, i.e. 0.75 ft). It has to guess
which way the model was drawn, because nothing on the wire says.

Real models turned out to be **face of wall** (House A: measured 0.317 ft gaps
between rooms), but Revit supports **wall centreline** too — and a project can
mix them, because the setting is per document and a project can have several
linked models.

**Every artifact chased so far is downstream of that guess:** bevelled corners,
45° chamfers, a million-foot spike, and sibling footprints overlapping. The
radius is sized for the worst case, so it is too big for models that need none.

Three decisions, in the order they should be built:

| # | Decision | Home |
|---|---|---|
| 1 | **Boundary location is a model fact** — the extractor declares it | the upload **envelope**, like `model_to_shared` |
| 2 | **Measurement standard + wall-thickness ceiling are project policy** | per-project **settings** |
| 3 | **Replace per-group closing with a wall-zone partition** | `service::areas` rewrite |

---

## Where things stand right now (read this first)

Landed on `main`: PR #6 (the close, the `polygons`/`holes` wire shape, the
even-odd viewer overlay, `scripts/gen_samples.py`) and PR #7 (bevel joins,
union-back corner restore, direction-keyed de-bevel).

Landed on branch `areas-spike-guards` (commit `16938b3`, `areas.rs` +313/−34) —
the artifact fixes, kept separate from the two structural changes below because
they stand on their own. It adds:

- `corner_of_chamfer` — the **spike guards**. Without them, two near-parallel
  flanking edges intersect at near-infinity: House A's Outdoor department got a
  vertex at **y = −1,052,070 ft**, an area of **537,716 ft² against 2,678 ft² of
  actual rooms (200×)**, overlapping every other footprint. Guards: non-parallel
  flanks, intersection must extend `a→b` past `b` and land before `d` along
  `c→d`, and must sit within `max(2·chord, MAX_WALL_FT)` of both chord ends.
- `LevelRooms` + `foreign_near` — clips a group's footprint against **other
  groups' rooms**, so a close can never swallow a neighbour's room (a riser
  narrower than the close radius was the failing case).
- `resolve_sibling_overlaps` — withdraws contested area from **both** siblings,
  on the design's own rule that a wall between two groups belongs to neither.
- `union_of_rooms` / `close_clip_clean` / `dissolve_footprints(…, foreign, dirs)`
  — plumbing for the above; `dirs` is now computed **per level** (not per group)
  because clipping introduces edges from neighbours' rooms, which are real
  geometry and must not be de-bevelled as if invented.
- Four new tests: `test_sharpen_bevels_never_spikes_on_near_parallel_flanks`,
  `test_footprints_stay_within_room_bounds`,
  `test_sibling_footprints_do_not_overlap`,
  `test_footprint_does_not_claim_a_neighbours_room`.

**220 lib tests pass** (27 in `areas`), `areas.rs` clippy-clean.

### Verified state on House A (real data, face-of-wall)

Measured with a throwaway diagnostic (see "Test harness" below — it is worth
recreating as a committed tool):

- **Spikes: none.** No footprint vertex escapes its own rooms' bbox by >1.2 ft.
- **Areas: all 12 groups ratio 1.00–1.01** against summed net room area.
- **Chamfers: down to ≤0.12 ft**, except one 1.10 ft edge on Outdoor whose
  flanks are exactly parallel — a genuine jog, correctly left alone.

### The one unresolved thread

> **Resolved, and never diagnosed — which was the right outcome.** Decision 3
> made the whole class unreachable rather than answering (a) or (b): a group's
> fill is now a subset of the level wall zone, and sibling claims on a junction
> come out of both by rule. The harness reports **no sibling overlap at all** on
> House A. Neither candidate explanation was ever tested, and neither needed to
> be — which is what "do not keep patching this" was pointing at. For the record,
> (b) is disprovable by inspection anyway: `resolve_sibling_overlaps(&mut
> current)` ran *before* `emit`, so the emitted geometry always was the resolved
> one. That leaves (a), untested and now moot.

**Sibling footprints still overlap: 4 pairs, 0.06–3.1 ft².** They are long thin
slivers along shared walls (~0.26 ft wide, close to the 0.317 ft wall), so they
are *inside the wall band*, not inside any room — which is why `foreign_near`
does not catch them.

`resolve_sibling_overlaps` was added to remove exactly this and **did not fully
work**, and the reason is not understood:

- the rings are **simple** — a brute-force segment-crossing check found no
  self-intersections, so invalid geometry is ruled out;
- the grid sampler (0.25 ft, even-odd point-in-polygon, holes honoured) reports
  overlap where `geo`'s `intersection` apparently returned empty.

**Do not keep patching this.** Two candidate explanations worth ten minutes each
before abandoning: (a) the sampler counts points on a shared boundary as interior
to both, and the "overlap" is a coincidence of touching edges rather than real
area — check by measuring `intersection().unsigned_area()` in Rust for the exact
House A pair rather than sampling in Python; (b) `resolve_sibling_overlaps` runs
per tier on `current`, but the *emitted* `AreaGroup` for the bottom tier is built
from `current` **before** stage 2 — confirm the emitted geometry is the resolved
one. If (a) holds, there is no bug and the sampler is lying; that is the cheapest
outcome and worth ruling out first.

Either way, Decision 3 makes the whole class go away, which is why it is the
recommended destination rather than more arbitration.

---

## Decision 1 — the extractor declares the boundary location

**Not a project setting.** Revit already knows this
(`SpatialElementBoundaryLocation`, and the document's Area & Volume Computations
setting). Asking a human to re-assert it in TOML duplicates an authoritative fact
and invites getting it wrong.

The precedent is exact: **`model_to_shared`** (see [STRATEGY.md](../STRATEGY.md),
"The upload envelope") is a document-level `ProjectLocation` fact the producer
stamps once per model, `Option`-al, and adding it **did not bump the schema**.
Do the same:

```json
"model": { "id": "…", "name": "…", "source": "revit" },
"room_boundary": "finish_face"        // "centreline" | "finish_face", optional
```

Per **model**, not per project — this is the only way to honour the real case
that a project **mixes both**, since each linked model carries its own setting.

- Optional and defaulted, so every existing payload stays valid (no schema bump).
- Extractor reads it once per document and stamps it on the envelope; keep the
  extractor dumb otherwise (STRATEGY.md "Keep the extractor dumb on purpose" —
  reading one document setting is extraction, not computation).
- A project-settings value (Decision 2) is the **fallback/override** for models
  whose extractor predates this field.

### Why this is the highest-value change

For a **centreline** model the close radius collapses to a float-noise epsilon.
No meaningful offsetting happens, so **bevels, chamfers, spikes and sibling
overlaps cannot arise at all.** Declaring the regime does not merely improve the
tolerance — for that half of the world it deletes the entire artifact class.

For **finish face** a thickness ceiling is still needed (real walls vary), but it
becomes a declared project value instead of a constant someone picked.

---

## Decision 2 — measurement standard and thickness are project policy

Revit does not know these and should not. Which standard applies is contractual;
the width above which a gap stops being a wall and becomes a void is a project
judgement.

```toml
# settings/projects/<id>.toml
[areas]
measurement_standard = "IPMS3"       # what the number MEANS
max_wall_thickness   = 0.5           # ft; wider gaps are voids, not walls
boundary_location    = "finish_face" # fallback only; the envelope wins when present
```

Goes on `ProjectSettings` (the server uses it — unlike client-only
`colour_plans`), beside `hierarchy_exclusions`, which is the closest precedent.

**This also fixes a live drift risk:** `areas::MAX_WALL_FT` and
`adjacency::WALL_MAX_FT` are the **same physical quantity** in two modules with
two constants. One declared value, two consumers.

### Guards (per the codebase's own discipline)

- **Sane default** so an un-migrated project behaves exactly as today.
- **Validate at boot** ("loud startup over silent no-op"): an unknown standard or
  a non-positive thickness fails the boot with a specific message.
- **Warn on contradiction** ("signal, not error"): a model declaring
  `finish_face` whose rooms all tile edge-to-edge — or the reverse — is a
  diagnostic worth logging, not something to silently obey. Cheap to detect: the
  same gap measurement that found House A's 0.317 ft.
- **Echo the resolved standard and regime on the `/areas` response.** An area
  figure without its definition is precisely what measurement standards exist to
  prevent.

### Standards worth reading before picking a default

IPMS (International Property Measurement Standards), RICS *Code of Measuring
Practice*, DIN 277, SIA 416, BOMA. They already specify whether to measure to
centreline, finish face or dominant face, and how to attribute a shared
partition. **Check whether the current convention — "a wall between two groups
belongs to neither, and fills at the common ancestor" — matches any recognised
definition before tuning further**, because everything else rests on it.

---

## Decision 3 — partition the wall zone instead of closing per group

The reformulation, and the reason the artifacts keep coming back: **we are using
an area operator to answer a topology question.** Morphological closing is not
idempotent or overlap-safe when applied independently to overlapping
neighbourhoods, so each group's close bulges into shared walls and the fixes are
all after-the-fact arbitration.

Do almost no geometry instead:

1. Build the level's wall zone **once**: `close(all rooms) − all rooms`.
2. Split it into **connected components**. Each component is a wall band, or a
   void.
3. Assign each component **combinatorially**, by which rooms bound it:
   - bounded by rooms of one group → that group;
   - bounded by two or more groups → their **common ancestor**;
   - not enclosed, or wider than `max_wall_thickness` → a **void**, unassigned.

By construction this gives what the current code only approximates:

- **no overlaps** — components are disjoint;
- **exact additivity** — every piece of area is assigned exactly once;
- **corner artifacts on the single outer boundary only**, not once per group per
  tier — which is the whole of the present problem;
- the governing invariant expressed **directly** rather than via a radius.

Likely **cheaper** than today: one union plus one difference per level, plus
component labelling, versus a close (two buffers) plus a clip plus an
intersection pass per group per tier.

This is also the shape the **BIM/energy-modelling** world uses for the same
question — see references.

### What carries over, what goes

- Keep: `dedup_collinear`, the `polygons`/`holes` wire shape, the even-odd viewer
  overlay, `Case A`/`Case B` exclusions, per-level partitioning, `gen_samples.py`.
- Keep as the **outer-boundary** step only: `morphological_close`,
  `sharpen_bevels` + `corner_of_chamfer` (with their spike guards).
- Retire: `foreign_near` and `resolve_sibling_overlaps` — both exist purely to
  undo per-group closing, and the partition makes them unnecessary.

---

## Test harness — "test all House A floor plans" (explicitly requested)

> **Built: [`scripts/check_areas.py`](../../scripts/check_areas.py).** All five
> checks below, plus a sixth the brief did not ask for and that turned out to be
> the cheapest of the lot — **tier additivity**, which is pure arithmetic on
> numbers the response already carries, no geometry at all. Checks 1 and 5 are
> also inline Rust tests, as asked. Stdlib only, and it takes no `--level`
> argument: checking every level is the lesson, not a default.
>
> Two implementation notes worth keeping. The overlap check is still a **grid
> sampler**, because an exact clipper does not belong in a diagnostic — but it
> now uses a **half-open** point-in-polygon rule, which is precisely the fix for
> candidate (a) above: without it, every shared wall reads as a phantom overlap
> one grid step wide. And the invented-direction check needs a length floor,
> since boolean ops leave sub-millimetre stubs at junctions that are not the 45°
> chamfers it is looking for.

The throwaway Python diagnostic that found the spike should become a **committed
tool**, because it caught in seconds what visual inspection missed. It ran
against the live server (`/areas` + `/rooms`) and checked, per level and per
tier:

1. **Spikes** — any footprint vertex outside its own rooms' bbox by > a wall.
2. **Area sanity** — footprint ÷ summed net room area; flag outside ~0.95–1.6.
3. **True overlap** — polygon intersection between siblings (not bbox: concave
   footprints' bboxes overlap legitimately).
4. **Invented directions** — output edge orientations absent from the input.
5. **Ring validity** — self-intersection check.

Two of these (1 and 5) are cheap invariants that belong as **inline Rust tests**
over the real fixtures too, not just an external script. Run it across **every**
House A level, not one — the spike was on LEVEL 00 while the visual work had been
on LEVEL 01.

---

## References (verified)

> **All checked.** The brief's list was marked "leads, not confirmed titles";
> every lead in it turned out to be a real paper, and the details are filled in
> below. Two of them say something the brief did not anticipate, marked **⚠**.
> The full comparison of this implementation against the literature lives in
> [STRATEGY-AREA-CALCULATION.md](../STRATEGY-AREA-CALCULATION.md); this is the
> bibliography.

**Space-boundary attribution (BIM/energy) — the closest existing field.**
Deriving which space owns which piece of wall is the *second-level space
boundary* problem every IFC→EnergyPlus / gbXML translator solves
(`IfcRelSpaceBoundary`).

- Rose, C. M. & Bazjanac, V., *An algorithm to generate space boundaries for
  building energy simulation*, **Engineering with Computers** 31(1), 2015 (online
  2013), DOI [10.1007/s00366-013-0347-5](https://doi.org/10.1007/s00366-013-0347-5).
  Graph-theoretic, IFC in, EnergyPlus geometry out, and explicitly accounts for
  construction material configuration as well as geometry.
- Bazjanac, V., *Space Boundary Requirements for Modeling of Building Geometry
  for Energy and Other Performance Simulation* (LBNL) — the requirements paper
  behind the above.
- LBNL's **Space Boundary Tool (SBT-1)** is the reference implementation:
  <https://simulationresearch.lbl.gov/projects/space-boundary-tool>.

**Mathematical morphology.** Serra, J., *Image Analysis and Mathematical
Morphology* (Academic Press, 1982); Soille, P., *Morphological Image Analysis:
Principles and Applications*.

> **⚠ The brief got a property wrong, and so did the first implementation.**
> Closing **is** idempotent — `φ(φ(X)) = φ(X)` — and it is also *extensive* and
> *increasing*; those three are its textbook definition. The property actually
> being relied on is that closing **does not distribute over union**,
> `φ(X ∪ Y) ⊇ φ(X) ∪ φ(Y)`, strictly wherever a gap closes. That strict part is
> the shared wall, so the ownership rule is a morphology theorem rather than a
> policy. Both `service::areas` and this document said "not idempotent"; both are
> corrected.

**Cartographic generalization / building aggregation.**

- Damen, J., van Kreveld, M. & Spaan, B., *High quality building generalization
  by extending the morphological operators*, 11th ICA Workshop on Generalization
  and Multiple Representation, Montpellier, June 2008.

> **⚠ Directly applicable to the open chamfer residual.** They apply closing
> followed by opening (or the reverse) to get elimination, detail removal and
> aggregation in one operator — and they observe that *some short edges remain*,
> which they clean up with **short-edge elimination**. That is the standard
> answer to precisely the leftover chords `sharpen_bevels` is trying to
> reconstruct corners from. Worth trying before the T-vertex fix recorded in the
> DoD: dropping a sub-threshold edge and letting its neighbours meet is a far
> simpler operation than reconstructing the corner it cut, and it has no spike
> failure mode.

- Regnauld, N., on building grouping/typification, and Meulemans / Buchin /
  Speckmann on rectilinear schematization and area-preserving simplification —
  the "building squaring/regularization" family `sharpen_bevels` reimplements.

**Footprint of a point set.** Names the ambiguity instead of burying it in a
tolerance. Related to closing, but none of these carry a partition or an
additivity constraint, so they characterise the problem rather than solve this
one.

- Edelsbrunner, H., Kirkpatrick, D. & Seidel, R., *On the shape of a set of
  points in the plane* (alpha shapes), IEEE Trans. Information Theory, 1983;
  Edelsbrunner & Mücke, *Three-dimensional alpha shapes*, 1994.
- Galton, A. & Duckham, M., *What Is the Region Occupied by a Set of Points?*,
  GIScience 2006, LNCS 4197, DOI
  [10.1007/11863939_6](https://doi.org/10.1007/11863939_6). Compares footprint
  methods against nine general criteria — the right frame for arguing about
  which footprint definition a project wants.
- Duckham, M., Kulik, L., Worboys, M. & Galton, A., *Efficient generation of
  simple polygons for characterizing the shape of a set of points in the plane*,
  **Pattern Recognition** 41(10), 2008, pp. 3224–3236, DOI
  [10.1016/j.patcog.2008.03.023](https://doi.org/10.1016/j.patcog.2008.03.023).
  Delaunay-based, one normalized parameter, O(n log n).

**Straight skeleton / generalized Voronoi.** Aichholzer, O. & Aurenhammer, F.,
*Straight Skeletons for General Polygonal Figures in the Plane*, COCOON '96,
Hong Kong, LNCS, pp. 117–126. The principled way to split a gap between two
polygons — and, per the standards note below, the shape IPMS 3 actually asks
for. The brief deliberately rejected medial splitting as a primary policy; that
decision now has a cost attached to it and should be revisited deliberately.

**Offsetting robustness.** Chen, X. & McMains, S., *Polygon Offsetting by
Computing Winding Numbers*, ASME IDETC/CIE 2005, paper DETC2005-85513,
pp. 565–575 — offsets multiple non-overlapping polygons with holes in
O((n+k) log n), discarding invalid loops by winding rule. Also the Clipper
library's join-type and **miter-limit** documentation: capping the miter ratio
and falling back to bevel is the canonical fix for the runaway spike that
`corner_of_chamfer` guards against by hand.

### Measurement standards — the answer to this brief's own question

Decision 2 asked: *check whether the current convention — "a wall between two
groups belongs to neither, and fills at the common ancestor" — matches any
recognised definition, because everything else rests on it.*

**Short answer: IPMS 3 is one localised rule away; DIN 277 wants a different
output shape.** Comparing wall by wall rather than as a whole philosophy:

| wall | IPMS 3 | this implementation |
|---|---|---|
| internal to one group | included in full | included in full — **agrees** |
| external, or against a standard facility (stair, lift, WC, plant) | excluded; measured to the internal dominant face | never enters the wall zone (nothing on the far side to close against) — **agrees** |
| **demising, between two occupiers** | to the **centre-line**: the wall is included and **apportioned equally** between the two | attributed **wholly to their common ancestor**; neither child carries any — **differs** |

**And the difference cancels one tier up.** Sub-departments A1 and A2 inside
department A, sharing wall `W`: IPMS 3 gives A1 `W/2` and A2 `W/2`, so
`A = W`; this implementation gives both `0` and fills `W` at A, so `A = W`. Same
figure. Every tier at or above a wall's common ancestor agrees, and so does the
building total. The two conventions disagree only about the **distribution among
the two children that bound the wall**, and only for genuine demising walls.

So this is not a different measurement philosophy — it is one redistribution
step, on one class of wall, at one tier. Three consequences:

1. **On a centreline model this server already produces IPMS 3 department
   areas**, without any code intending to. A centreline room polygon contains
   half of every wall bounding it, which *is* IPMS 3's rule, arrived at by
   modelling convention. That is exactly what
   `test_centreline_and_finish_face_agree_on_the_same_building` pins down when it
   asserts each department loses `half_shared_wall` on the finish-face side.
2. **Making the finish-face path conformant is an addition, not a rewrite.** The
   bands A and B jointly enclose but neither encloses alone are constructible
   from machinery already here —
   `close(rooms(A) ∪ rooms(B)) ∩ zone − fill(A) − fill(B)` — one extra close per
   sibling pair, bbox-rejected, splitting the result 50/50. Three-way junctions
   need the same withdrawal `resolve_sibling_overlaps` already does. It would be
   a `measurement_standard`-driven redistribution applied *after* the partition,
   leaving additivity and the wall zone untouched.
3. **A DIN 277 project wants a different response, not a different rule.** KGF
   is a reported category in its own right (`BGF = NRF + KGF`), so the honest
   output is the wall zone's area **as its own figure** beside the room areas,
   rather than folded into any tier. The wall zone is already computed and
   already means exactly that — a response-shape change, not a geometry one.

None of this is a defect in what was built: the convention is deliberate,
internally consistent and exactly additive. But it is a **house convention**, so
`measurement_standard = "IPMS3"` should not be set on a **finish-face** project
until (2) is built. On a centreline project it can be set today.

---

## Definition of done

**Commit what exists**

- [x] The `areas.rs` artifact fixes (spike guards, room clip, overlap pass, 4
      tests) are committed on `areas-spike-guards` (`16938b3`) — a strict
      improvement regardless of what follows, with the unresolved overlap thread
      recorded in the commit message.
- [x] Merged as PR #8 (`3946edc`), and the Decision 1–3 work builds on it.

**Decision 1 — envelope**

- [x] `room_boundary` optional field on the model envelope
      (`contract::RoomBoundary`, `"centreline"` | `"finish_face"`); no schema
      bump; every existing payload still valid and unchanged in meaning.
- [ ] **Extractor stamps it once per document — not done. No longer blocked:**
      when this was written the extractor lived in another repository; it is now
      `extractor/pyRevit/`, and `post_rooms.py`'s `build_envelope` already stamps
      `model_to_shared` the same way, so `room_boundary` is one field beside it. The server half is complete, so this is one field on the
      producer's envelope whenever that repo is next touched; until then every
      model resolves through the project fallback, which is the designed-for
      state, not a broken one. The wire spelling is fixed by
      `test_ingest_response_wire_keys`.
- [x] Ingest accepts absence silently (normal case) and echoes the **resolved**
      value on `IngestResponse.room_boundary` — resolved, not declared, because
      the interesting case is the one the producer did not state. Both the
      buffered and streamed routes carry it (`test_ingest_stores_room_boundary_on_both_routes`
      — the stream path rebuilds the payload field by field, so a forgotten field
      there would be a silent regime downgrade on the largest models).

**Decision 2 — settings**

- [x] `[areas]` block on `ProjectSettings`: `measurement_standard`,
      `max_wall_thickness`, optional `boundary_location` fallback
      (`settings::AreaPolicy`).
- [x] Boot validation with specific messages; default preserves today's output
      exactly (1.5 ft, no standard, no fallback — and `skip_serializing_if` so an
      untouched project file gains no `[areas]` section on its next save).
      An unknown standard fails in the TOML parse with serde naming the accepted
      spellings, which is a better message than a hand-rolled check.
- [x] `adjacency` reads the same declared value — **one quantity, one source.**
      Both `areas::MAX_WALL_FT` and `adjacency::WALL_MAX_FT` are gone. The
      adjacency default is now regime-aware: zero when every level in scope is
      centreline, the declared thickness otherwise, and an explicit `?wall_max=`
      still overrides.
- [x] Contradiction warning (declared regime vs measured room gaps) —
      `areas::warn_on_regime_contradiction`, a bounded nearest-neighbour sample
      per level, logged and never fatal.
- [x] Resolved standard + per-level gap echoed on `/areas`
      (`measurement_standard`, `wall_gap_by_level`).

**Decision 3 — partition**

- [x] Wall zone built once per level. **Connected-component labelling was
      dropped** — see the status note: the wall network is one component on a
      real floor, so it cannot be the assignment unit. Each prefix intersects its
      own close against the zone instead, which yields the same ownership rule.
- [x] Bands land at the common ancestor by construction (a band with one group's
      rooms on one side only is never in that group's fill); over-wide and
      unenclosed gaps stay voids **arithmetically** — a close at radius `gap/2`
      cannot fill anything wider than `gap`, so no width test exists to get wrong.
- [x] Inline tests: additivity exact to 0.05 ft²
      (`test_tier_areas_are_exactly_additive`); no sibling overlap
      (`test_sibling_footprints_do_not_overlap`); courtyard still open
      (`test_parent_keeps_courtyard_open`, `test_ring_of_rooms_keeps_courtyard_open`);
      centreline and finish-face inputs agree on the same building
      (`test_centreline_and_finish_face_agree_on_the_same_building` — which also
      pins the per-department difference, see status note 3); rings simple in
      both regimes (`test_emitted_rings_are_simple`).
- [x] `foreign_near` deleted. **`resolve_sibling_overlaps` kept** — see status
      note 2; it is the design's own rule applied to wall junctions, not
      arbitration, and it is what keeps additivity exact.

**Validation**

- [x] Diagnostic harness committed (`scripts/check_areas.py`) and run over **all**
      House A levels. Six checks: spikes, area ratio, true sibling overlap,
      invented edge directions, ring validity, tier additivity. Stdlib only, and
      it takes no `--level` argument by design.
- [x] Clean on five of the six checks, across all three levels:
      **no spikes** (the 537,716 ft² / 200× Outdoor footprint is gone — LEVEL 00
      Outdoor now measures 2,682 ft² at ratio 1.00); **every group 1.00–1.01×**
      its rooms' net area; **no sibling overlap at all** — the 4 pairs at
      0.06–3.1 ft² the previous version could not resolve are gone by
      construction, which retires that open thread without ever having to decide
      whether the old sampler was lying; **all rings simple**; **additivity
      clean**. `big-plate` is clean on all six.
- [x] Two pre-existing defects the harness surfaced on the **`showcase` fixture**,
      unchanged by this work and identical on both builds — recorded here because
      they will otherwise be mistaken for regressions from it: `North Tower /
      Outpatient / Ward A` reports a **0.0 ft² footprint against 448 ft² of
      rooms**, and both towers report 0.93× (the towers' shortfall is exactly the
      same 448 ft², so the two are one bug). Not investigated — out of scope here,
      and `showcase` is generated by `scripts/gen_showcase.py`, so the fixture
      itself is a suspect before the service is.
- [ ] **Invented directions got worse, and this is the one honest cost of the
      change.** Chamfers the de-bevel fails to remove, before → after, counting
      edges longer than 0.05 ft:

      | project | before | after |
      |---|---|---|
      | House A (real) | 2 | **3** |
      | `sample-project` | 1 | **2** |
      | `showcase` | 0 | **14** |
      | `big-plate` | 0 | **0** |

      Every one is a chord of `gap/2 · √2` ≈ 1.06 ft or shorter, sitting on a
      footprint outline where the close filled a notch; areas are unaffected to
      ~0.03%, and nothing else regressed. `showcase` is the worst because it is a
      generated fixture full of identical notches, so one unhandled shape recurs
      fourteen times rather than once. Cosmetic, but not nothing, and left
      **unfixed rather than tuned away** because the two obvious fixes were tried
      and both cost more than they bought:

      - *De-bevel the wall zone instead of the footprints* — makes House A worse
        (5, not 3). A wall band is a thin sliver, so its two long sides are
        parallel, and `corner_of_chamfer`'s parallel-flank guard (the one that
        exists to stop the million-foot spike) correctly refuses to sharpen
        there. The de-bevel belongs on the emitted geometry.
      - *`dedup_collinear` before sharpening* — clears all fourteen on
        `showcase`, and is **destructive on House A**: a 20.8 ft invented
        diagonal, one department down to 0.72× its rooms' area, and a 2.5 ft²
        sibling overlap back. Reverted.

      The actual fix, for whoever picks this up: make `sharpen_bevels` walk past
      collinear neighbours to find a chamfer's *true* flanks, instead of assuming
      the adjacent vertices are them. The boolean ops plant T-vertices that split
      a flank in two, which is why the reconstructed corner lands somewhere
      implausible and the distance guard — correctly — refuses it. Fix the
      sharpener, do not pre-simplify the ring underneath it.
- [x] Timing measured on `big-plate` before/after, and it is not uniformly
      faster — the brief guessed "likely cheaper", and that holds only where
      there is grouping to amortise the level-wide pass against:
      `big-plate` (132 groups) **6.0 s → 1.12 s**, `sample-project` (10 groups
      over 7 levels) 0.43 s → 0.71 s, `House A` 0.35 s → 0.35 s. The expensive
      case is the one that was unusable, so the trade is the right way round.
      A single-group fast path was tried and reverted: measured, it moved
      neither number, because the cost is the boolean difference over the room
      set rather than the close.

**Still open**

- [ ] The extractor field (above). No longer outside this repo — see `extractor/pyRevit/post_rooms.py`.
- [ ] **No model has yet been read in the `centreline` regime end to end.** Every
      fixture and every real model on hand resolves to finish face, so the
      zero-gap path is covered by unit tests and by nothing else. The first
      genuinely centreline export is worth running the harness against, because
      that path skips the close entirely and its failure mode would be silence,
      not an artifact.
- [ ] Pick and declare a `measurement_standard` for the real projects. The
      machinery is there and every response currently says `null`, which is
      honest but not useful. The standards worth reading before choosing are
      listed above; the specific question to answer first is whether the
      convention this code implements — a wall between two groups belongs to
      neither and fills at the common ancestor — matches any recognised
      definition.

---

## Docs to update on landing

- **[STRATEGY.md](../STRATEGY.md)** — `room_boundary` in "The upload envelope",
  beside `model_to_shared`.
- **[STRATEGY-SERVER.md](../STRATEGY-SERVER.md)** — rewrite the `/areas` bullet for
  the partition; record the declared-regime settings and the standard echo.
- **[STRATEGY-SOURCES.md](../STRATEGY-SOURCES.md)** — the extractor now reads one
  document-level setting.
- **[STRATEGY-BROWSER.md](../STRATEGY-BROWSER.md)** — only if the wire shape moves;
  the even-odd overlay should survive unchanged.
- **[handover-hierarchical-void-closure.md](handover-hierarchical-void-closure.md)**
  — add a line pointing here: its erode-to-empty classifier is superseded by the
  combinatorial assignment, for the same reason its weld pass was.
- **[docs/README.md](../README.md)** — Open handovers row.
