# Blueprints, stage 0 findings: House A

Part of [PLAN-blueprints.md](PLAN-blueprints.md). Measured 2026-10-19 with throwaway scripts in
`scripts/blueprint_stage0/` (`survey.py`, `exp.py`; `bp0.py` is the capture and resolve sketch;
the snapshot path is hard-coded to the checked-in House A data). **Nothing here is product
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
**The repeated-layout question needs RHH** (patient rooms), run through the same scripts.

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

## Consequences for the plan

1. **Decision 4 (no item-to-item relations in v1) should be revisited** for one case: an item
   that abuts another (a run of casework, a unit inside a wardrobe). The smallest form is a
   "follows the previous item" anchor, or a group that is placed as one.
2. **The blueprint stores the simplified outline**, and edge counts and mapping use it.
3. **Thin wall devices are a class of their own**: edge, height, no footprint checks.
4. **Hybrid is the default rule**, with the 300 mm and 150 mm thresholds user-adjustable.
5. **Rectangles first** is defensible on this data (about 60%), but L and notched rooms
   are common enough (7 of 27) that edge mapping on a different count must be a finding, not
   a dead end.
6. **Re-run on RHH before stage 2.** That is the test the model has not yet had.
