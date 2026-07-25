# Handover: Hierarchical area aggregation with wall-void closure

## Reconciliation note (added after review + implementation)

> **Superseded again, in part** — see
> [HANDOVER-areas-boundary-location.md](HANDOVER-areas-boundary-location.md).
> The close described below produced corner and overlap artifacts on real
> face-of-wall data; the boundary regime is now **declared on the upload
> envelope** rather than inferred, and the per-tier close is to be replaced by a
> combinatorial assignment of wall-zone components. The erode-to-empty classifier
> goes the same way the weld pass did, for the same reason.


**A version of this was built — but not the pipeline below.** Reviewed against
the codebase, two of this document's premises did not survive contact:

1. The recursion, per-tier fill, and "close at the level that encloses the void"
   invariant were **already implemented** in `service/areas.rs` (as a strip-all-
   holes rule, since the real models were assumed centreline). The genuinely new
   need was face-of-wall support, which the plan screenshots confirmed is the
   real case.
2. The proposed pipeline (weld at 2–5 mm, then classify interior rings with
   erode-to-empty) **cannot fill wall bands on face-of-wall geometry**: rooms
   separated by a full-thickness wall union into *disjoint islands*, so there are
   no interior rings to classify, and a hairline weld cannot bridge a wall.

What shipped instead is a **single morphological close** per tier (`buffer(+r)`
then `buffer(−r)`, `r = MAX_WALL_FT/2`, geo's miter-join `Buffer`). It does the
bridging, the wall-band fill, and the wall-vs-void classification in one
operation — and leaves genuine voids open — with no separate weld, no erode-to-
empty test, and no new dependency. It is gap-agnostic, so centreline, finish-
face and mixed models all work. The `SplitMedial` presentation concern (hazard 3)
did not arise: areas are additive and the viewer renders the open voids directly
(even-odd path), so no separate presentation geometry was needed. Face-of-wall
sample levels (`scripts/gen_samples.py`, a lift core / service core / courtyard on
the `showcase` project) exercise it end-to-end.

The original design below is kept for its reasoning about *why* area-to-perimeter
ratios and medial-axis splits are the wrong tools — that analysis still holds and
motivated the close. Treat the pipeline steps as superseded by the note above.

## Status of this document

This describes a design agreed in conversation. **It was written without sight of the current
codebase.** The code in this project is at least two days out of date relative to the author's
working copy, so:

- Treat all type names, function names, and signatures below as *illustrative*.
- Reconcile against the actual crate API before writing anything.
- Where this document and the current code disagree about existing behaviour, the code wins —
  but check with the author before assuming the disagreement is intentional.

Do not begin implementation until you have read the current geometry crate and confirmed which
of the primitives listed under "Required primitives" actually exist.

---

## The problem

Room polygons are modelled to **face of wall**, not centre line. When rooms are unioned to form a
parent area, the wall thickness between them survives as a void in the result:

- Between sibling rooms inside one parent (e.g. two lift shafts in a lift core) → an enclosed hole.
- Between rooms belonging to *different* parents (e.g. a stair core and an adjacent mechanical
  strip) → also an enclosed hole once you go up far enough, but it does not belong to either parent
  individually.
- Where rooms were drawn nearly-but-not-exactly coincident → hairline gaps that are not enclosed
  holes at all, and which produce slivers in the output boundary.

Areas must remain additive up the hierarchy, and no parent may claim wall area it does not own.

## The design

Recursive, one pass per hierarchy level, bottom-up.

```
level 0 (raw room polygons)
  └── weld pass (ONCE, here only)
        └── level 1: union children of each parent → classify interior rings → fill wall bands
              └── level 2: union level-1 results → classify → fill
                    └── level 3: ... etc.
```

### Governing invariant

> **A void is closed at the lowest hierarchy level that fully encloses it.**

Consequences:

- A void bounded entirely by children of one parent is an interior ring of that parent's union, and
  is filled there.
- A void bounded by children of two or more parents is *not* an interior ring at either parent's
  level — it is an open notch in each. It becomes an enclosed ring only at the first common
  ancestor, and is filled there.
- Therefore **no edge-provenance tracking is needed.** The level boundary performs the adjacency
  test for free. (An earlier version of this design tagged edges with source polygon IDs to answer
  "which parents bound this hole". The recursion makes that unnecessary. Do not implement it.)

This keeps each level's area independent of its siblings, which matters for debugging. The
alternative — splitting every inter-parent band along its medial axis — was considered and
rejected for that reason.

---

## Pipeline

### Step 1 — Weld pass (level 0 only)

Purpose: close hairline gaps between near-coincident room edges so they do not become slivers.

```
for each room polygon: buffer(+ε)
unary_union
buffer(−ε)
```

- ε ≈ 2–5 mm. Small, fixed, and **completely independent of the wall-thickness threshold.**
- Use mitre joins with a high mitre limit, or corners will round off.
- **Run this once, on raw room polygons.** Re-welding at every level compounds the offset — three
  levels at 5 mm shifts boundaries by 15 mm for nothing.

### Step 2 — Union children (per level)

Standard `unary_union` of the child geometries for each parent node. Children at level 1 are welded
room polygons; at level N>1 they are the filled outputs of level N−1.

### Step 3 — Classify interior rings (per level)

For each interior ring of the union result, decide: wall band, or real void?

**Test: erode-to-empty.** Buffer the ring's polygon by −(max_wall_thickness / 2). If the result is
empty, it is a wall band. If anything survives, it is a real void (courtyard, atrium, lightwell,
unmodelled space) and must be left open.

Why this test specifically:

- Wall voids are frequently L-shaped, T-shaped, or comb-shaped where several walls meet at
  junctions. **Area-to-perimeter ratio does not discriminate these** — a T-shaped sliver scores
  close to a fatter rectangle. Do not use it.
- Minimum-width via rotating calipers fails on non-convex rings for the same reason.
- Erode-to-empty is shape-agnostic and handles branching junctions correctly.

`max_wall_thickness` is a **configured parameter**, not inferred from the geometry. Suggested
default 400 mm; expose it on the aggregation config.

### Step 4 — Fill (per level)

Rings classified as wall bands are filled — i.e. dropped from the interior ring list of the parent
polygon. Rings classified as real voids are retained.

The width guard in step 3 runs at **every** level, not just level 1. The recursion determines
*where* a void may be closed; the guard determines *whether* it should be. If a level fills a real
courtyard, that error is baked into everything above it and is invisible from higher levels.

### Step 5 — Recurse

Feed filled geometry up to the next level. Repeat steps 2–4.

---

## Diagnostics (not optional)

Log every fill. At minimum: level, parent node ID, ring area, ring bounding box, and the eroded
area that triggered the classification.

Additionally, emit a **warning** for any filled ring whose area exceeds a sanity bound (suggest:
some multiple of max_wall_thickness², configurable). A wall band that is unexpectedly large is
almost always a modelling error — a missing room, a mis-drawn boundary — and silently swallowing it
converts a data-quality problem into a wrong number that nobody notices.

---

## Known hazards

**1. Narrow rooms are not wall bands.**
Risers, small mechanical cupboards, and service columns are frequently the same width as walls.
They are safe *because they are rooms and therefore inputs* — they never appear as holes. The
danger is the weld pass absorbing them into neighbours. This is why ε must stay small and must not
be derived from max_wall_thickness. If a single parameter ever controls both, narrow rooms will
start disappearing and the cause will be very hard to find.

**2. Boundary notches are out of scope.**
Wall bands that terminate against the outer boundary are notches, not enclosed rings. They will not
be seen by step 3. Real buildings also have genuine notches in their outer boundary at the same
width as wall bands, so any rule that closes notches by width will destroy real geometry.
**Leave notch handling out.** For area purposes a notch in the outer boundary is harmless.

**3. Areas vs. drawings.**
Under the governing invariant, a wall that reads visually as one continuous line may be closed
along part of its length (where it separates siblings) and open along the rest (where it separates
different parents). The areas are correct and additive. The plan view will look inconsistent.

If the crate's consumers render output as drawings, they will need a **separate presentation pass**
that closes inter-parent bands visually without touching the area geometry. Keep the two
representations distinct. Do not attempt to find one geometry that satisfies both — that was
considered and is the main thing to avoid.

Check with the author whether the presentation pass is in scope before building it.

---

## Required primitives

Confirm these exist in the current geometry crate before starting:

| Primitive | Used by | Notes |
|---|---|---|
| `buffer` / offset with mitre joins | steps 1, 3 | mitre limit must be settable |
| `unary_union` | steps 1, 2 | over a collection of polygons |
| interior ring access | steps 3, 4 | read and rebuild polygon with a filtered ring set |
| empty-geometry test | step 3 | after negative buffer |

All four are standard in `geo` / `geo-buffer`. If the crate wraps its own geometry types, check
whether negative buffers are exposed — some wrappers only expose outward offset.

**Not required:** straight skeleton / medial axis. That was needed only for the medial-split
assignment policy, which this design does not use. If a `SplitMedial` policy is requested later it
would need to be implemented or vendored separately.

---

## Suggested config surface

```rust
pub struct VoidClosureConfig {
    /// Weld tolerance for near-coincident edges. Small, fixed. NOT related to wall thickness.
    pub weld_epsilon: Length,          // default ~3 mm

    /// Voids narrower than this are treated as wall bands and closed.
    pub max_wall_thickness: Length,    // default ~400 mm

    /// Filled rings larger than this are logged as warnings (probable modelling errors).
    pub sanity_area_warn: Area,        // default ~ (max_wall_thickness^2) * k
}
```

---

## Test cases

Derive fixtures from these; the author has plan screenshots for each and can supply them.

1. **Lift core.** Eight lift shafts around a central lobby, all face-of-wall. Every internal band is
   a sub-200 mm sliver. All should close at the lift-core level. Perimeter band around the whole
   core should *not* close at that level (it separates the core from the corridor, a different
   parent) and should close one level up.

2. **Stair core adjacent to mechanical strip.** Bands are L- and T-shaped at junctions. Confirms
   erode-to-empty succeeds where area/perimeter ratio fails. Confirms the inter-parent band stays
   open at the level where stair and mech are different parents.

3. **Comb of narrow rooms.** A large room with a row of thin rooms hanging off one side, nearly
   coincident. No enclosed holes at all — this tests the weld pass, not the fill pass. Assert the
   narrow rooms survive as distinct areas and are not absorbed.

4. **Mixed abutment.** Two parents where the shared boundary is near-coincident along part of its
   length and a genuine wall band along the rest. Assert: weld closes the coincident part, band
   stays open, no slivers in the output boundary.

5. **Real void.** A parent containing a genuine courtyard or lightwell wider than
   max_wall_thickness. Assert it survives at every level.

6. **Regression: additivity.** For any node, sum of children's areas plus filled band area equals
   the node's area. This should hold at every level and is the cheapest guard against the whole
   thing quietly going wrong.

---

## Open questions for the author

1. Is the presentation pass (hazard 3) in scope, or is this area-schedule only?
2. Is `max_wall_thickness` one global value, or does it need to vary by building / by level?
3. What should happen when a real void is detected at level 1 but the parent above expects it
   closed — warn, or is that always a modelling error to be surfaced?
