# Blueprints, stage 0 findings: House A and RHH

Part of [PLAN-blueprints.md](PLAN-blueprints.md). Measured 2026-10-19 with throwaway scripts in
`scripts/blueprint_stage0/` (`bp0.py` is the capture and resolve sketch; `survey.py` and `exp.py`
run House A from the checked-in data; `rhh.py`, `rhh_exp.py`, `rhh_exp3.py` run RHH from the
production snapshot store, with the paths hard-coded). **Nothing here is product
code.** Only the geometry and grammar questions of stage 0 are answered; the pyRevit versus C#
spike needs Revit and has not been done.

## What House A can and cannot test

House A is one house: 32 rooms in one model, 819 FF&E items. **It has no repeated room type**, so
it cannot say whether a recipe reproduces a real sibling layout. The only plausible functional
siblings are BED 01 and BED 02 (both 4-edge rectangles, area ratio 1.05), the three WCs and the
two storage rooms. Resolving BED 01 onto BED 02 puts items a median 994 mm from where BED 02's
real items of the same type are, which is no better than the no-recipe baseline (1,009 mm):
the two bedrooms are laid out differently, not badly mapped. Pooled over all 94 room pairs with
two shared types, every recipe policy lands 1.86 to 2.06 m from the real item (baseline 2.00 m);
most pairs are unrelated rooms, so this number measures nothing about the model.
**The repeated-layout question needs RHH** (patient rooms), which is the next section.

## What was proven

- **Capture then resolve is exact.** Resolving a captured recipe back onto its own room reproduces
  all 548 items to 0.000 mm under all four policies.
- **Edge choice matters.** Choosing the nearest edge by segment distance failed on 6 items in
  notched rooms (up to 967 mm off), because the item's projection falls beyond that edge's
  span. The rule that works: among edges whose span contains the item and that it lies on the
  room side of, take the one nearest in the normal direction. No item then needs a corner anchor.
- **The raw outline cannot be the reference.** 25 of 32 rooms have vertices that are not corners
  (collinear points, a 195 mm jog): raw vertex counts run to 26, against at most 16 after
  simplifying (30 mm tolerance). Edge counts, edge mapping and "same shape" all have to use the
  **simplified outline**, which the blueprint must therefore store.

## Shapes

Of the 27 rooms that hold items, **16 are plain rectangles**, 7 are orthogonal with more edges
(L, U and notched rooms), and 4 are not orthogonal. Of all 32, 19 are 4-edge orthogonal. A v1
limited to rectangles covers about 60% of rooms with contents.

## What the data held

Of 819 items: 176 are nested components (`super_component_id`), 75 belong to no room, 20 have no
footprint; **548 remain**. Of those, **235 (43%) are under 100 mm thick** (switches, sockets,
rails, blinds): they sit on the wall line and are located by wall and height, not by footprint.
They need `z` and an edge, and they must not take part in overlap checks. The other 313 are
the layout proper. Nested components must be excluded (they move with their parent).

## Size and proportion change

Scaling a room in its own axes and resolving the 313 items of the 21 rooms with at least three
(counts are items outside the room / new overlapping pairs, over all 21 rooms):

| Scale (along x across) | fixed from start | fixed from nearer end | percent | hybrid |
|---|---|---|---|---|
| 1.15 x 1.15 | 0 / 3 | 0 / 3 | 0 / 0 | 0 / 0 |
| 1.60 x 1.00 | 0 / 1 | 0 / 2 | 0 / 0 | 0 / 0 |
| 0.85 x 0.85 | 64 / 36 | 18 / 166 | 37 / 256 | 18 / 256 |
| 1.30 x 0.80 | 50 / 21 | 12 / 132 | 31 / 207 | 11 / 208 |
| 1.00 x 0.60 | 98 / 47 | 46 / 197 | 55 / 250 | 21 / 336 |

*Hybrid* is the plan's rule: an item within 300 mm of a corner keeps that gap; an item within
150 mm of its wall keeps that gap; anything else becomes a percentage. Findings:

- **Growing a room is safe; shrinking is not.** No policy fails a larger room.
- **No policy is free when a room shrinks.** Fixed rules push items out of the room;
  percentages keep them in and stack them on each other. Hybrid keeps the most items inside.
  So both findings must be reported (decision 3), never refused.
- **The overlaps are runs.** At 0.85 x 0.85, 183 of the 256 new overlaps are between items that
  were within 50 mm of each other before the change, 181 are casework against casework, and
  BED 01 alone gives 90 (wardrobes with shelves and drawers inside them that are not flagged as
  nested). An independent percentage for each cabinet in a run compresses the run, which
  cannot be what anyone wants.

## RHH: the repeated-layout test

Four interior models (`INT_LG-G`, `L1-L2`, `L3-L5`, `L6-RF`), latest snapshots: 2,683 rooms (204
dropped for no usable outline) and 37,520 FF&E items kept, after excluding 14,131 nested
components, 1,017 with no room and 808 with no footprint. Rooms are grouped by name with the
trailing code removed. Per group, 120 random ordered pairs; a recipe captured from one room is
resolved onto the other and each resolved item is compared with the nearest real item of the same
type name in the target (non-wall items only, greedy one-to-one matching). The rotation is chosen
by shape (edge lengths plus turn angles), never by looking at the answer. A mirrored source is
the source reflected, which is what a "mirrored copy" blueprint would hold.

**The rooms that repeat most are not rectangles.** `1 BED B` (47 rooms) and `1 BED A` (25) are
9-edge angled rooms, `ENS-IN` (62) and `ENS-OUT` (44) are 5-edge angled. The rectangles are the
toilets, store and utility rooms (WCST, RESUS, BMEQ, WCAC), where the layout varies from room to room.

| Group | Plain fit | Mirror image only | No fit | Edge count differs | Within 150 mm, plain pairs | Within 150 mm, mirror-only pairs |
|---|---|---|---|---|---|---|
| 1 BED B | 23% | 12% | 11% | 54% | 74% (27) | 67% (15) |
| 1 BED A | 13% | 6% | 40% | 41% | 88% (16) | 0% (7) |
| ENS-IN | 26% | 37% | 3% | 35% | 94% (31) | 75% (44) |
| ENS-OUT | 42% | 43% | 0% | 14% | 92% (51) | 96% (50) |

(Shares of the 120 pairs; counts of pairs in brackets. "Within 150 mm" is the share of shape-compatible pairs whose median item error is under 150 mm, hybrid rule.)

- **A recipe reproduces a real sibling layout.** On shape-compatible patient-room pairs the median
  item is 27 to 97 mm from its real counterpart, and in 74 to 96% of pairs (all but `1 BED A`
  mirrored) the median is under 150 mm. This is the test House A could not give.
- **The rule for the same-name rooms is not "same name, same layout".** The rectangular groups
  fit in shape but not in layout (within 150 mm in 12 to 31% of plain pairs; median errors from
  0.4 to 2.4 m). A blueprint is something the user chooses for a standardised type, not something
  to infer from a room name.
- **Handedness is common.** Mirror images are 37% and 43% of the ensuite pairs and 12% of `1 BED B`.
  With no mirror option (decision 7) a standard room needs a left and a right blueprint. That
  works (a reflected source resolves within 150 mm for 75% and 96% of ensuite mirror pairs), but
  the resolver should recognise the mirror case and say so, and the editor should offer a
  **create mirrored copy** action, which makes a second blueprint where the user can swap the
  handed families.
- **Edge count is the wrong test for "same shape".** Even among the patient rooms, 14 to 54% of
  pairs differ in edge count, because of 0.3 to 0.5 m jogs at columns and risers. Simplifying the
  outline at 350 mm instead of 30 mm cuts that (`1 BED B` from 54% to 16%; all 1,560 pairs from 597
  to 428) and raises the compatible pairs from 617 to 837. The cost is that items then fall
  outside the simplified outline (`1 BED B`: 1% of items to 21%), because the jogs are where wall
  items sit. So **map edges on the coarse outline, check findings against the true one.**
- **Policy barely matters at sibling sizes.** Where siblings differ in size by more than about 3%
  per edge, hybrid is 10 to 20 mm better than percent or fixed-from-nearer-end
  (`1 BED B` 105 / 117 / 128 mm, `ENS-IN` 96 / 115 / 110 mm). The House A scaling runs, where the size
  change is large, are what decides the policy.

## Consequences for the plan

1. **Decision 4 (no item-to-item relations in v1) should be revisited** for one case: an item
   that abuts another (a run of casework, a unit inside a wardrobe). The smallest form is a
   "follows the previous item" anchor, or a group that is placed as one.
2. **The blueprint stores the simplified outline** (see 6 for the two tolerances).
3. **Thin wall devices are a class of their own**: edge, height, no footprint checks.
4. **Hybrid is the default rule**, with the 300 mm and 150 mm thresholds user-adjustable.
5. **Rectangles first is wrong.** The standardised, repeated rooms (patient rooms, ensuites)
   are 5 to 9-edge angled polygons, and the recipe model handled them. The reference outline is a
   general polygon; rectangles are a case, not the scope. (This answers review question 2.)
6. **Mapping uses shape, not edge count**: edge lengths and turn angles, on a coarse outline
   (about 350 mm), while findings are checked against the true outline. Both outlines are stored.
7. **Add a "create mirrored copy" action** and have the resolver report "this room is the mirror
   image of the blueprint" instead of resolving badly. This keeps decision 7 and makes handedness
   a normal, cheap thing to author.
8. **Doors, not used here, should anchor the rotation.** Two rotations of a symmetric outline
   score the same on shape; the door position in the blueprint is what picks the right one.
9. **Not yet tested:** type matching here is by type name only (no family name yet), the
   comparison is on non-wall items, and nothing was run in Revit.
