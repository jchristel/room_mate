# RoomMate — Area calculation

What the area figure means, how it relates to the published measurement
standards, and what is still open.

This has its own document because it is the only part of the pipeline where the
**definition of the output is contested**. Everywhere else the server reports
what the model says; here it has to decide what a wall belongs to, and reasonable
standards disagree. **The algorithm is not described here** — it lives in
`src/service/areas.rs`, whose module header carries the wall zone, the ownership
theorem and the junction correction in full. This document carries the part no
code comment can settle: what the number may be *called*, and what is unfinished.

## The three properties everything else is subordinate to

Any change to `service::areas` is measured against these, so they are stated here
rather than only asserted by a test:

1. **Measured, never summed.** Each tier's area is the area of that tier's own
   polygon. Summing children mishandles the wall between them, in whichever
   direction the convention pushes it.
2. **Exactly additive.** `parent = Σ children + the bands the parent is the
   first tier to enclose`, asserted to 0.05 ft². An area schedule that does not
   add up is not an area schedule.
3. **No double counting.** No two groups at one tier may claim the same square
   foot, and no wall may be counted at two tiers.

Additivity is the constraint that is *not* in the geometry literature (see
below), and it is the one most likely to be broken by a well-meant local fix.

## Relationship to measurement standards — read before quoting a figure

The number is an **aggregated room footprint**: room area, plus the wall bands
the group encloses, minus genuine voids. It is **not** net room area and **not**
a standards-defined gross. `measurement_standard` is declared per project and
echoed on every response, because an area figure without its definition is what
standards exist to prevent.

The convention implemented is a **house convention**. Compared wall by wall:

| wall | IPMS 3 | here |
|---|---|---|
| internal to one group | included in full | included in full — **agrees** |
| external, or against a standard facility (stair, lift, WC, plant) | excluded; measured to the internal dominant face | never enters the wall zone — **agrees** |
| **demising, between two occupiers** | to the **centre-line**; apportioned **equally** between the two | attributed **wholly to their common ancestor** — **differs** |

**The difference cancels one tier up.** Sub-departments A1 and A2 inside
department A sharing wall `W`: IPMS 3 gives each `W/2`, so `A = W`; this gives
each `0` and fills `W` at A, so `A = W`. Every tier at or above a wall's common
ancestor agrees, and so does the building total. The disagreement is confined to
how one class of wall is split between the two children bounding it.

Two consequences:

- **On a centreline model this server already reports IPMS 3 department areas**,
  without any code intending to — a centreline room polygon contains half of each
  bounding wall, which *is* IPMS 3's rule, arrived at by modelling convention.
- **DIN 277 wants a different response, not a different rule.** All walls are
  Konstruktions-Grundfläche, a category of its own (`BGF = NRF + KGF`), never
  attributed to an occupier. The wall zone already *is* that quantity, so
  reporting it as its own figure is a response-shape change with no geometry
  behind it.

**Therefore: do not set `measurement_standard = "IPMS3"` on a finish-face
project** until the redistribution below exists. On a centreline project it can
be set today.

## Where this sits against the literature

The closest existing field is **second-level space boundaries** in BIM/energy
modelling (`IfcRelSpaceBoundary`) — "which space owns which piece of wall" is
exactly the question, and IFC→EnergyPlus translators have solved it for years
(Rose & Bazjanac 2015; LBNL's Space Boundary Tool). The important divergence:
that field **splits** a shared wall geometrically, usually at the centreline.
This design refuses to split, and gives the band to the ancestor.

What is *not* in that literature is the constraint that actually drove the
design: **exact additivity up a classification hierarchy**. That comes from area
scheduling and cost planning, not from geometry. Alpha shapes and characteristic
shapes (Galton & Duckham 2006; Duckham et al. 2008) characterise "the footprint
of a set" but carry no partition and no additivity; space boundaries carry
adjacency but no hierarchy. The construction here is effectively a nested
(laminar) family of sets over the wall zone, indexed by tree depth.

Two places the literature has a better answer that was not taken:

- **Junction handling.** A generalized Voronoi / straight-skeleton decomposition
  (Aichholzer & Aurenhammer, COCOON '96) would partition a junction exactly and
  need no withdrawal pass at all. `fill(P) = close(rooms(P)) ∩ zone` is a good
  proxy for "bounded on both sides by P", and the `+` junction is precisely where
  the proxy frays.
- **The residual chamfers.** Damen, van Kreveld & Spaan (ICA 2008) apply the same
  morphological operators to building generalization, observe that short edges
  survive, and clean them with **short-edge elimination** — rather than trying to
  reconstruct the corner each chord cut, which is what `sharpen_bevels` does and
  where the remaining artifacts come from. Relatedly, `corner_of_chamfer`'s
  distance cap is a hand-rolled **miter limit**; the standard answer is
  miter-with-a-limit at offset time (Clipper), not bevel-then-reconstruct.

## Open

- **The centreline regime has now been read end to end, and it is mostly
  right.** A hospital job read on 2026-08-25 — the **test project** every
  measurement below comes from — is the first centreline data to reach the
  server: 6 models, 18 levels, 2880 rooms in its main building. Its data and
  its settings file are both gitignored, which is why it is unnamed here and
  why the numbers, not the labels, are what is written down. The standing
  "every fixture and every model on hand is finish face" is no longer true,
  and the path whose failure mode is *silence rather than an artifact* has
  been made to speak. What it said:

  - The declared regime matches the measured one on every level: the
    `coincident_share` cross-check in `areas.rs` logs nothing, and
    `wall_gap_by_level` is `0` for all 18. The close is genuinely skipped.
  - **335 of 361 groups report exactly 1.000× their rooms' net area** — the
    signature of the regime, since a centreline room polygon already contains
    its half-walls and the dissolve adds nothing. `adjacency` independently
    lands on `wall_max = 0` and still finds 185 edges over 147 rooms, which is
    the same fact from the other side: the rooms really do tile.
  - Additivity holds. Of 72 parents, 49 are exact or carry a surplus, 20 are
    exact to 0.00 ft², and 3 fall short — worst 21.7 ft² (0.48%), inside
    `check_areas.py`'s 0.5% slack.

  Two things it found, both **new and both centreline-specific**:

  - **10 self-intersecting rings** — 6 on the building tier itself, the rest
    spread over three departments and one sub-department. Simple rings is one
    of the two invariants stated absolutely (it is an inline Rust test as well
    as check 5), so this is a violation, not a threshold. Nothing bevels a
    centreline dissolve, so the usual suspects — `sharpen_bevels`,
    `corner_of_chamfer` — are not even running.
  - **Two groups whose footprint is BELOW their rooms' net area**, both
    sub-departments of one department: 0.89× and 0.92×. On centreline the
    expected ratio is 1.000 exactly, so `check_areas.py`'s 0.95–1.6 band is
    the wrong instrument here — it was calibrated for finish face, where the
    footprint is net *plus* wall. Read against 1.000, 15 groups are low, not
    two.

  **The two are independent, which is the trap here**: 8 of the 10
  self-intersecting groups measure exactly 1.000×, so the bad rings cost no
  area at all, and neither low-ratio group self-intersects — correlating them
  is an afternoon that finds nothing. The area loss pairs up somewhere else
  entirely: one department's circulation sub-group is **157.2 ft² short** of
  its rooms' net while its parent department is **157.2 ft² long** on the sum
  of its children, the same figure on both sides. So the second finding is not
  area vanishing but area moving **up a tier** — in a regime with no fill,
  which is the only mechanism that is supposed to add anything at a parent.

  What is still unread: the sibling-overlap check (3) has never completed on
  the test project — the pure-Python sampler is O(pairs) over 361 groups and
  was still running after half an hour. Everything above is a `--skip-overlap`
  run. Do not answer it with a finer grid; it wants an exact clipper, and the
  invariant it guards is already covered from two cheaper directions above (a
  double-claim would push children past their parent, and would read above
  1.000× on the group that swallowed).

  The slack itself deserves a second look. `check_areas.py` justifies its 0.5%
  by "the close's bevel at the level's outer boundary" — a centreline level has
  no close, so on this project the slack is unearned and those 3 shortfalls
  have no stated cause.

- **IPMS 3 redistribution for finish-face projects.** Constructible from what is
  already here: the bands A and B jointly enclose but neither encloses alone are
  `close(rooms(A) ∪ rooms(B)) ∩ zone − fill(A) − fill(B)`, one extra close per
  sibling pair, split 50/50, with three-way junctions handled by the withdrawal
  pass that already exists. It is a `measurement_standard`-driven step *after*
  the partition; additivity and the wall zone are untouched.

- **DIN 277 KGF as its own reported figure.** A response-shape change, per above.

- **No project has declared a `measurement_standard`.** Every `/areas` response
  says `null` — honest, but not useful: the machinery exists and nobody has used
  it. The question to answer first is not "which standard do we like" but the one
  the table above raises, since declaring a standard on a finish-face project
  would be a claim the numbers do not support. **The test project above is the
  first where that objection does not apply** — it is centreline, so per the
  table above `IPMS3` is declarable on it today, and it would be the first
  non-null value any consumer has ever seen. Settle the rings above first: a
  standard on a figure with a self-intersecting boundary is a claim about a
  number that is still moving.

- **No project has chosen a `max_wall_thickness` against a real model.** Every
  one runs on the 1.5 ft default, inherited from the constant it replaced rather
  than measured. This is **one value driving three services** — `areas` sizes
  its wall zone by it, `adjacency` defaults its gap tolerance from it, and
  `room_locator` sizes the step it probes across a wall by it — so it is
  probably the highest-leverage open item here. House A measures 0.317 ft wall
  gaps, making 0.5 ft the obvious candidate, but see the next item: raising the
  value is what risks adjacency false positives, and that is where the
  consequences of getting it wrong show up first.

- **Two adjacency false-positive checks, needing a model this repo does not
  have.** At a realistic tolerance, confirm `service::adjacency` does **not**
  report

  1. two rooms merely *facing each other across a corridor* as adjacent, and
  2. two rooms bridged *through* a thin service room (a riser or shaft narrower
     than the tolerance) sitting between them.

  **Neither has an `areas` analogue, and that asymmetry is the reason this is
  open.** `service::areas` rules both out *structurally* — a close at `gap/2`
  provably cannot fill a gap wider than `gap`, and the wall zone contains no
  rooms — whereas adjacency answers both with a segment-pair test plus a
  midpoint-in-third-room occlusion check, which unit tests cover and nothing else
  does.

  **House A cannot settle either, despite being real finish-face data**, and it
  is worth recording why so nobody spends an afternoon rediscovering it. It is a
  26-room detached house: no double-loaded corridor, no duct riser. A `wall_max`
  sweep saturates —

  | `wall_max` (ft) | 0 | 0.5 | 1.5 | 3 | 5 |
  |---|---|---|---|---|---|
  | edges | 34 | 47 | 51 | 51 | 51 |

  — so no pair of rooms sits 1.5–5 ft apart. There is nothing at corridor
  distance to wrongly bridge even deliberately, and a clean run would be clean
  because the hazard is absent, not because the algorithm handled it. That is a
  test that cannot fail, which is worth no confidence.

  **What is worth doing now and needs no new model:** an adjacency diagnostic
  shaped like `scripts/check_areas.py` — for each reported edge, measure the true
  gap between the two room polygons and flag any exceeding a plausible wall, plus
  any with a third room's polygon between them. Against House A that gives a real
  finish-face regression baseline and evidence for setting `max_wall_thickness`.
  It does not tick 1 and 2, which want a hospital-scale finish-face export.

  **The test project above is hospital-scale and does NOT tick them either**,
  which is worth saying because it is the obvious wrong assumption to make
  about it. It has the hazards House A lacks — double-loaded corridors, risers
  — but it is *centreline*, so its tolerance collapses to `wall_max = 0` and
  neither false positive can fire at any distance. The pairing still wanted is
  scale **and** finish face; it supplies only the first.

- **Residual chamfers**: 3 on House A, 2 on `sample-project`, 14 on the synthetic
  `showcase`, all ≤1.06 ft and cosmetic. Two fixes were tried and reverted with
  measurements. Try short-edge elimination or a miter limit before the T-vertex
  fix. **Zero on the centreline test project**, and that is a control rather
  than a fourth data point: chamfers are made by the close, and a centreline
  dissolve never runs one. It does tell you the self-intersecting rings above
  come from somewhere else.
