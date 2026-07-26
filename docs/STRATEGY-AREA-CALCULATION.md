# RoomMate — Area calculation

How a room set becomes a per-tier area figure, why that is harder than it looks,
and what the number does and does not mean.

This has its own document because it is the only part of the pipeline where the
**definition of the output is contested**. Everywhere else the server reports
what the model says; here it has to decide what a wall belongs to, and reasonable
standards disagree. Two prior designs were reversed before this one, both because
the definition was implicit — so the definition is written down first, and the
algorithm second.

- Implementation: `src/service/areas.rs`. Endpoint: `GET /projects/{id}/areas`
  (see [Server](STRATEGY-SERVER.md)).
- The regime it depends on rides the upload envelope (see [STRATEGY.md](STRATEGY.md)
  "The upload envelope") and its policy lives in project settings.
- Design history and the measured before/after:
  [HANDOVER-areas-boundary-location.md](Superseded/HANDOVER-areas-boundary-location.md).

---

## 1. What the number is

An **aggregated room footprint**: room area, plus the wall bands the group
encloses, minus genuine voids. It is **not** net room area and **not** a
standards-defined gross. Whether it coincides with a standard depends on the
model's boundary regime — see §6, which is the section to read before quoting a
figure to anyone external.

Three properties are non-negotiable, and everything else is subordinate to them:

1. **Measured, never summed.** Each tier's area is the area of that tier's own
   polygon. Summing children mishandles the wall between them, in whichever
   direction the convention pushes it.
2. **Exactly additive.** `parent = Σ children + the bands the parent is the
   first tier to enclose`. Asserted to 0.05 ft² by
   `test_tier_areas_are_exactly_additive`. An area schedule that does not add up
   is not an area schedule.
3. **No double counting.** No two groups at one tier may claim the same square
   foot, and no wall may be counted at two tiers.

## 2. The two regimes, and why the server cannot guess

Revit's `SpatialElementBoundaryLocation` decides where a room's boundary sits:

- **Centreline** — rooms tile edge-to-edge. Each room polygon already contains
  **half of every wall bounding it**. There is no gap and nothing to fill.
- **Finish face** — rooms float inside their walls. Neighbours are separated by
  roughly a partition's thickness, and a room polygon contains **none** of its
  walls.

A single project mixes both, because the setting is per document and a project
has several linked models. Sizing one tolerance for both is what the first two
designs did, and every artifact they produced — bevelled corners, 45° chamfers,
a 1,052,070 ft spike, overlapping siblings — was downstream of that guess.

So the regime is **declared, not inferred**: `room_boundary` on the upload
envelope, per model, resolved per *level* (level dedup can put two models on one
level; a disagreement widens to finish face). A centreline level runs at gap
**zero** — the close is never invoked, and the artifact class cannot arise
because the operation that produces it does not execute.

When the declared regime contradicts the measured room gaps, the server **logs
and continues**. It cannot know which of the two is wrong, and refusing to answer
would remove the view that makes the problem visible.

## 3. The wall zone

One object, built once per level:

```text
wall_zone = (close(all rooms, gap/2) ∪ all rooms) − all rooms
```

It is every gap narrow enough to be a wall, and nothing else. Three properties
fall out of that sentence rather than being enforced:

- **It contains no room.** A group's share is a subset of it, so a footprint can
  never reach inside a neighbour's room. This deleted a dedicated clip pass.
- **It contains no void wider than `gap`.** A close at radius `gap/2` cannot
  bridge anything wider, so a courtyard, atrium or lightwell is never in the set.
  "Wider than a wall stays open" is arithmetic, not a rule — which is why the
  earlier erode-to-empty void classifier is gone.
- **Its outer boundary is the rooms' own exact boundary** wherever it faces open
  space, because the original geometry is unioned back before the subtraction.
  There are no chipped corners to repair.

A group's share is whatever its **own** rooms close over, intersected with that
ceiling:

```text
fill(P)      = close(rooms under P, gap/2) ∩ wall_zone
footprint(P) = rooms under P ∪ fill(P)
```

## 4. Why the ownership rule is a theorem, not a policy

The design rule is *a wall between two groups belongs to neither; it fills at
their common ancestor.* That is not layered on top of the formula — it is what
the formula computes. Writing `φ` for the close:

- **`φ` is increasing** (`X ⊆ Y ⟹ φ(X) ⊆ φ(Y)`). Applied to
  `rooms(P) ⊆ rooms(parent)`, that gives `fill(P) ⊆ fill(parent)` — the nesting
  that makes areas additive.
- **`φ` does not distribute over union**: `φ(X ∪ Y) ⊇ φ(X) ∪ φ(Y)`, strictly
  wherever a gap between `X` and `Y` closes. **That strict part is the shared
  wall.**

Concretely: the band between room `a` of group A and room `b` of group B is not
in `fill(A)` — dilating A reaches into it, but the erode pulls straight back out
because nothing merged across. Same for B. Their ancestor holds both rooms, its
close merges, and it claims the band exactly once.

> **A correction worth keeping.** Two earlier documents said "closing is not
> idempotent". Closing **is** idempotent (`φ(φ(X)) = φ(X)`), along with extensive
> and increasing — those three are its definition (Serra 1982; Soille).
> Non-distribution over union is the property that was meant, and it is the
> stronger claim. What genuinely is not idempotent is `geo`'s **bevel-join
> offset approximation**, which is not even extensive, since a bevel only ever
> cuts a corner. That is why every tier is computed from the raw rooms rather
> than from the tier below it: it keeps each footprint exactly one approximation
> deep instead of compounding one per tier.

## 5. The one place it needs help

Where four rooms meet at a `+` junction, the small square at the centre is
bounded by all four. If two belong to A and two to B, and the wall is thin
enough that each pair's diagonal closes, **both** fill it. That is a genuine
double count — and the design rule already answers it: bounded by two groups, so
owned by neither. `resolve_sibling_overlaps` withdraws it from both and the
ancestor picks it up, which is what keeps additivity exact.

This is the design's own rule applied to one shape, not arbitration. It is
junction-sized; the whole-footprint arbitration the previous design needed is
gone.

## 6. Relationship to measurement standards — read before quoting a figure

`measurement_standard` is declared per project and **echoed on every response**,
because an area figure without its definition is what standards exist to prevent.
Today it is `null` on every real project, which is honest and not yet useful.

The convention implemented here is a **house convention**. Compared wall by wall:

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
  without any code intending to — a centreline room polygon contains half of
  each bounding wall, which *is* IPMS 3's rule, arrived at by modelling
  convention. `test_centreline_and_finish_face_agree_on_the_same_building` is
  where that is pinned down.
- **DIN 277 wants a different response, not a different rule.** All walls are
  Konstruktions-Grundfläche, a category of its own (`BGF = NRF + KGF`), never
  attributed to an occupier. The wall zone already *is* that quantity, so
  reporting it as its own figure is a response-shape change with no geometry
  behind it.

**Therefore: do not set `measurement_standard = "IPMS3"` on a finish-face
project** until the redistribution in §8 exists. On a centreline project it can
be set today.

## 7. Where this sits against the literature

The closest existing field is **second-level space boundaries** in BIM/energy
modelling (`IfcRelSpaceBoundary`) — "which space owns which piece of wall" is
exactly the question, and IFC→EnergyPlus translators have solved it for years
(Rose & Bazjanac 2015; LBNL's Space Boundary Tool). The important divergence:
that field **splits** a shared wall geometrically, usually at the centreline.
This design refuses to split, and gives the band to the ancestor.

What is *not* in that literature is the constraint that actually drove this
design: **exact additivity up a classification hierarchy**. That comes from area
scheduling and cost planning, not from geometry. Alpha shapes and characteristic
shapes (Galton & Duckham 2006; Duckham et al. 2008) characterise "the footprint
of a set" but carry no partition and no additivity. Space boundaries carry
adjacency but no hierarchy. The construction here is effectively a nested
(laminar) family of sets over the wall zone, indexed by tree depth.

Two places the literature has a better answer that was not taken, both recorded
with citations in the handover's References section:

- **Junction handling.** A generalized Voronoi / straight-skeleton decomposition
  (Aichholzer & Aurenhammer, COCOON '96) would partition a junction exactly and
  need no withdrawal pass at all. `fill(P) = close(rooms(P)) ∩ zone` is a good
  proxy for "bounded on both sides by P", and §5 is precisely where the proxy
  frays.
- **The residual chamfers.** Damen, van Kreveld & Spaan (ICA 2008) apply the same
  morphological operators to building generalization, observe that short edges
  survive, and clean them with **short-edge elimination** — rather than trying to
  reconstruct the corner each chord cut, which is what `sharpen_bevels` does and
  where the remaining artifacts come from. Relatedly, `corner_of_chamfer`'s
  distance cap is a hand-rolled **miter limit**; the standard answer is
  miter-with-a-limit at offset time (Clipper), not bevel-then-reconstruct.

## 8. Open

- **Nothing sends `room_boundary` yet**, so every model resolves through the
  project fallback — a designed-for state, not a broken one. This was previously
  blocked on the extractor being in another repository; it now lives in
  `extractor/pyRevit/`, where `post_rooms.py`'s `build_envelope` already stamps
  the sibling `model_to_shared`. Adding it is one field beside that one, and it
  is the change that turns the boundary regime from a guess into a fact.
- **No model has been read in the centreline regime end to end.** That path skips
  the close entirely, so its failure mode is silence rather than an artifact.
- **IPMS 3 redistribution for finish-face projects** (§6). Constructible from
  what is already here: the bands A and B jointly enclose but neither encloses
  alone are `close(rooms(A) ∪ rooms(B)) ∩ zone − fill(A) − fill(B)`, one extra
  close per sibling pair, split 50/50, with three-way junctions handled by the
  withdrawal that already exists. It is a `measurement_standard`-driven step
  *after* the partition; additivity and the wall zone are untouched.
- **DIN 277 KGF as its own reported figure** (§6).
- **No project has declared a `measurement_standard`.** Every `/areas` response
  says `null`, which is honest but not useful — the machinery exists and nobody
  has used it. The question to answer *first* is not "which standard do we
  like" but the one §6 raises: the convention this code implements (a wall
  between two groups belongs to neither and fills at the common ancestor) is a
  **house convention** matching neither IPMS 3 nor DIN 277, so declaring a
  standard on a finish-face project would be a claim the numbers do not support.
  Centreline projects are the exception and can be marked IPMS 3 today.
- **No project has chosen a `max_wall_thickness` against a real model** — every
  one runs on the 1.5 ft default, which was inherited from the constant it
  replaced rather than measured. This is now **one value driving two services**
  (`areas` sizes its wall zone by it, `adjacency` defaults its gap tolerance from
  it), so it is probably the highest-leverage open item here. House A measures
  0.317 ft wall gaps, so 0.5 ft is the obvious candidate for it — but see
  [HANDOVER-adjacency.md](HANDOVER-adjacency.md), because raising the value is
  what risks adjacency false positives, and that is where the consequences of
  getting it wrong show up first.
- **Residual chamfers**: 3 on House A, 2 on `sample-project`, 14 on the synthetic
  `showcase`, all ≤1.06 ft and cosmetic. Two fixes were tried and reverted with
  measurements; see the handover DoD. Try short-edge elimination or a miter limit
  before the T-vertex fix.
- **Verification**: `scripts/check_areas.py` — six checks (spikes, area ratio,
  true sibling overlap, invented edge directions, ring validity, tier
  additivity), run against a live server, across **every** level. Two of the six
  are also inline Rust tests.
