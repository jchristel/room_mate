# HANDOVER — Area aggregation: declare the boundary location, then partition the wall zone

**Status:** decisions settled in conversation; **the artifact fixes have landed,
the two structural changes are not started.** This document is the brief for the
next session. It also records *why* the current approach keeps producing
artifacts, so nobody re-derives that.

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

The precedent is exact: **`model_to_shared`** (see [STRATEGY.md](STRATEGY.md),
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

## References (verify before citing — these are leads, not confirmed titles)

Most useful first:

- **Space-boundary attribution (BIM/energy).** Deriving which space owns which
  piece of wall is the *second-level space boundary* problem every IFC→EnergyPlus
  / gbXML translator solves (`IfcRelSpaceBoundary`). Look up Bazjanac's LBNL work
  on IFC→EnergyPlus geometry transformation, and Rose & Bazjanac, *An algorithm to
  generate space boundaries for building energy simulation* (~2015). This field
  assumes face-of-wall spaces and solves ownership combinatorially — i.e.
  Decision 3.
- **Mathematical morphology.** Serra, *Image Analysis and Mathematical
  Morphology* (1982); Soille, *Morphological Image Analysis*. Closing is a named
  operation and its corner behaviour is documented — we were not inventing.
- **Cartographic generalization / building aggregation.** Aggregating polygons
  separated by small gaps while preserving right angles is a core generalization
  operator. Leads: Regnauld on building grouping; Damen, van Kreveld & Spaan on
  extending morphological operators for building generalization; Meulemans /
  Buchin / Speckmann on rectilinear **schematization** and area-preserving
  simplification. "Building squaring/regularization" is the post-step our
  `sharpen_bevels` crudely reimplements.
- **Footprint of a point/polygon set.** Alpha shapes (Edelsbrunner & Mücke) —
  formally related to closing; Galton & Duckham, *What is the region occupied by a
  set of points?* (GIScience 2006); Duckham et al., *Efficient generation of
  simple polygons for characterising the shape of a set of points* (Pattern
  Recognition, 2008). These name the ambiguity instead of burying it in a
  tolerance — the rigorous version of the "convex hull minus others" idea.
- **Straight skeleton / generalized Voronoi.** Aichholzer & Aurenhammer. The
  principled way to split a gap between two polygons; relevant only as
  arbitration, and note the handover it came from deliberately rejected medial
  splitting as a primary policy.
- **Offsetting robustness.** Chen & McMains, *Polygon offsetting by computing
  winding numbers*; the Clipper library's join-type/miter-limit documentation —
  why bevel and miter behave as they do.

---

## Definition of done

**Commit what exists**

- [x] The `areas.rs` artifact fixes (spike guards, room clip, overlap pass, 4
      tests) are committed on `areas-spike-guards` (`16938b3`) — a strict
      improvement regardless of what follows, with the unresolved overlap thread
      recorded in the commit message.
- [ ] Merge that branch (or fold it into the Decision 1–3 work if you prefer one
      landing).

**Decision 1 — envelope**

- [ ] `room_boundary` optional field on the model envelope; no schema bump; every
      existing payload still valid and unchanged in meaning.
- [ ] Extractor stamps it once per document.
- [ ] Ingest accepts absence silently (normal case), and echoes the resolved
      value like other envelope facts.

**Decision 2 — settings**

- [ ] `[areas]` block on `ProjectSettings`: `measurement_standard`,
      `max_wall_thickness`, optional `boundary_location` fallback.
- [ ] Boot validation with specific messages; default preserves today's output.
- [ ] `adjacency::WALL_MAX_FT` reads the same declared value — one quantity, one
      source.
- [ ] Contradiction warning (declared regime vs measured room gaps).
- [ ] Resolved standard + regime echoed on `/areas`.

**Decision 3 — partition**

- [ ] Wall zone built once per level; connected components labelled.
- [ ] Components assigned by bounding groups; two-group bands land at the common
      ancestor; over-wide/unenclosed stay voids.
- [ ] Inline tests: additivity exact (parent = Σ children + newly-assigned
      bands); no sibling overlap **by construction**; courtyard still open;
      centreline and finish-face inputs give the same answer for the same
      building.
- [ ] `foreign_near` and `resolve_sibling_overlaps` deleted.

**Validation**

- [ ] Diagnostic harness committed and run over **all** House A levels, clean on
      all five checks.
- [ ] Timing measured on `big-plate` before/after (the partition should be
      faster; confirm rather than assume — STRATEGY.md).

---

## Docs to update on landing

- **[STRATEGY.md](STRATEGY.md)** — `room_boundary` in "The upload envelope",
  beside `model_to_shared`.
- **[STRATEGY-SERVER.md](STRATEGY-SERVER.md)** — rewrite the `/areas` bullet for
  the partition; record the declared-regime settings and the standard echo.
- **[STRATEGY-SOURCES.md](STRATEGY-SOURCES.md)** — the extractor now reads one
  document-level setting.
- **[STRATEGY-BROWSER.md](STRATEGY-BROWSER.md)** — only if the wire shape moves;
  the even-odd overlay should survive unchanged.
- **[handover-hierarchical-void-closure.md](handover-hierarchical-void-closure.md)**
  — add a line pointing here: its erode-to-empty classifier is superseded by the
  combinatorial assignment, for the same reason its weld pass was.
- **[docs/README.md](README.md)** — Open handovers row.
