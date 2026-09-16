//! The **geometric room attribution** for ceilings and floors: which rooms a
//! surface lies over, and how much of each.
//!
//! ## Why this is not `room_locator`
//!
//! Every other dependent entity asks "which room contains this point". A door
//! is in a wall, an item stands somewhere, and both have an authored reference
//! that geometry only ever fills in for. A ceiling or a floor is a *surface*:
//! it has no point to test, it can lie over more than one room, and **nothing
//! in Revit says which room it belongs to**. So the question here is not
//! containment but overlap, and the answer is not a fallback but the only
//! answer there is.
//!
//! Split out of `service::surfaces` because it is the half that is pure
//! geometry -- no store, no settings, no scope -- and it is where every
//! measured case lives as a test. Together the two would sit well past the
//! ~500-line split line.
//!
//! ## One tolerance, the same for both entities, and no sliver rule
//!
//! An overlap counts when its mean width, `2 x area / perimeter`, is at least
//! [`MIN_OVERLAP_MEAN_WIDTH_FT`] -- **10 mm**. That is a **precision
//! tolerance, not a judgement about which overlaps matter**. It rejects two
//! things that are not a surface being in a room: rounding in the boolean op
//! where a surface and a room only share an edge (duHast's own
//! `_intersect_ceiling_vs_room` guards the same case in shapely, where
//! `intersects()` is true for polygons that merely touch), and modelling
//! imprecision -- a finish edge a few millimetres past the room boundary along
//! a whole wall.
//!
//! **A WIDTH, because an area cannot catch the second.** A 3 mm strip along a
//! 3 m wall is 0.009 sqm and along a 30 m corridor 0.09 sqm, so any area line
//! either lets long walls through or drops real small rooms. The width of that
//! strip is 3 mm whatever the wall's length. Measured on RHH (2026-09-16) with
//! a 0.001 sqm area tolerance in its place: 243 of the pairs it admitted were
//! 0.001-0.01 sqm with a median width of 3-4 mm, and 559 more were
//! 0.01-0.1 sqm with a median of 30-40 mm.
//!
//! **Sliver rules were built here and removed (2026-09-16).** Ceilings used to
//! drop an overlap under 0.5% of the ceiling, and floors one narrower than a
//! wall (1.5 ft) unless it was half the room or half the floor, with a 1 sqft
//! absolute minimum under both. Each drew a line through a population with no
//! gap in it, and each needed another escape for the case the last one missed:
//! on RHH the ceiling rule dropped 17 rooms at least half covered by a ceiling
//! (8 of them wholly) because each was under 0.5% of it, and the floor rule
//! dropped 43% of a 9 sqft water niche. Deciding which real overlaps
//! count is a policy about the model, not a property of the arithmetic, so the
//! read reports every real overlap and carries what a consumer needs to apply
//! its own: `fraction_of_element`, `fraction_of_room` and `mean_width` ride
//! every row. A strip of floor under a wall into a neighbour IS reported, as a
//! small `fraction_of_room` with a `mean_width` under the wall thickness, and
//! `rooms` stays ordered largest overlap first, so `rooms[0]` did not change
//! for any RHH ceiling or floor when the rules went.
//!
//! What that costs, knowingly: a **degenerate export** -- House A's two ceilings
//! of 0.33 and 0.18 sqft, RHH's three under 5 sqft -- attributes to the room it
//! lies in. It is an export fault rather than imprecision, so it is the QA
//! report's to surface, not this tolerance's to hide.

use geo::{Area, BooleanOps, BoundingRect, Coord, Euclidean, Intersects, Length, LineString, MultiPolygon, Polygon};
use serde::Serialize;

use crate::contract::{Loop, Surface};

use super::room_locator::{Candidate, LEVEL_EPS_MM};

/// The narrowest overlap, in feet, that counts as a surface being in a room:
/// **10 mm** of mean width, a precision tolerance and nothing more.
///
/// Stated in millimetres because that is the size of the imprecision, and
/// converted because every coordinate on the wire is in feet. Shared by
/// ceilings and floors, because imprecision is a property of the geometry, not
/// of the category. Anything wider is a real overlap and is reported -- see the
/// module header for why a width and not an area, and why deciding which real
/// overlaps matter is not done here.
pub const MIN_OVERLAP_MEAN_WIDTH_FT: f64 = 10.0 / MM_PER_FT;

/// Exact by definition: 1 ft = 304.8 mm.
const MM_PER_FT: f64 = 304.8;

/// One room a surface lies over, with the measurements that justify the claim.
///
/// **Every number rides the wire**, because they answer different questions
/// and only `mean_width` took part in deciding this entry, against
/// [`MIN_OVERLAP_MEAN_WIDTH_FT`].
/// `fraction_of_element` is coverage of the surface, `fraction_of_room` is
/// coverage of the room -- "is this room fully ceiled", which a finishes
/// take-off wants -- and `mean_width` is the overlap's shape. A consumer that
/// wants to ignore slivers or strips applies its own line to these three; the
/// read no longer draws one for it.
#[derive(Debug, Clone, Serialize)]
pub struct SurfaceRoom {
    pub room_id: String,
    /// The model the room belongs to. A room id is unique only *within* a
    /// model, so a bare id here would be ambiguous the moment a project holds
    /// two architectural models — the trap `OpeningResponse::owner_rooms` has
    /// its `_qualified` sibling for.
    pub model_id: String,
    /// Overlap area in square feet.
    pub overlap_area: f64,
    /// Overlap as a fraction of the whole surface. Named for the element rather
    /// than for either entity, because the same field is served on `/ceilings`
    /// and `/floors`; it was `fraction_of_ceiling` while ceilings were alone.
    pub fraction_of_element: f64,
    pub fraction_of_room: f64,
    /// `2 x overlap_area / overlap perimeter`, in feet: the width of the
    /// overlap if it were a strip. A strip of floor reaching under a wall into
    /// a neighbour reads narrower than the wall, which a percentage cannot
    /// show -- a strip is a shape, not a proportion.
    pub mean_width: f64,
}

/// A surface's plan footprint: the UNION of every piece it exports.
///
/// **The union, and not the first piece or the largest one** -- the rule House A
/// taught wrongly and RHH corrected. duHast exports a slab once per horizontal
/// face of its solid, so on House A a ceiling arrived as its top face, its
/// bottom face and some edge slivers, and the largest piece was the whole
/// ceiling. RHH's pieces are genuinely disjoint instead: against the union,
/// taking the first loses 44.0% of ceiling area on average (99.2% at worst) and
/// taking the largest still loses 25.7%, with 31 of 48 losing over 5%.
///
/// The union is the one operation correct on both. It collapses House A's
/// duplicated faces back to a single face -- `A ∪ A = A` -- and keeps RHH's
/// separate pieces, so neither document needs a special case. It is also what
/// fills an island back into the hole around it: duHast exports an island as a
/// piece of its own, never as a ring inside a hole. A SUM would have been wrong
/// on House A for exactly the reason the union is not.
pub fn surface_shape(surface: &Surface) -> Option<MultiPolygon<f64>> {
    let mut merged: Option<MultiPolygon<f64>> = None;
    for piece in &surface.polygons {
        let Some(outer) = piece.loops.first() else {
            continue;
        };
        if outer.points.len() < 3 {
            continue;
        }
        // Holes ARE carried here, unlike the room side. A room's hole is a
        // column or a shaft -- small, and `room_locator::outline_of` drops it so
        // a probe landing on a column still resolves. A surface's hole is a
        // light well, a void over an atrium or a stair opening, and is routinely
        // large: RHH exports 173 holed ceiling polygons. Keeping them makes both
        // the overlap and the `fraction_of_element` denominator measure the
        // surface that is actually there.
        let holes: Vec<LineString<f64>> =
            piece.loops.iter().skip(1).filter(|l| l.points.len() >= 3).map(ring).collect();
        let piece = MultiPolygon::from(vec![Polygon::new(ring(outer), holes)]);
        merged = Some(match merged {
            None => piece,
            Some(acc) => acc.union(&piece),
        });
    }
    merged.filter(|m| !m.0.is_empty())
}

fn ring(l: &Loop) -> LineString<f64> {
    LineString::from(l.points.iter().map(|p| Coord { x: p.x, y: p.y }).collect::<Vec<_>>())
}

/// Every ring's length, holes included -- a hole's edge is as much boundary as
/// the outside's, and leaving it out would read a slab full of shafts as wider
/// than it is.
fn perimeter(shape: &MultiPolygon<f64>) -> f64 {
    shape
        .0
        .iter()
        .map(|p| Euclidean.length(p.exterior()) + p.interiors().iter().map(|r| Euclidean.length(r)).sum::<f64>())
        .sum()
}

/// Which rooms one surface lies over, largest overlap first.
///
/// Same model only: the caller hands over one model's candidates. CLAUDE.md's
/// rule is that an element joining on a room id is model-scoped everywhere,
/// and spaces are the single exception because they join on a project-unique
/// *key*. A surface joins on neither -- it joins on geometry -- so it follows
/// the conservative rule, which RHH's ceilings confirmed and its fit-out
/// floors did too. **Its base-build model has not been exported**, and slabs
/// there beside fit-out models holding the rooms is exactly the layout that
/// would break it.
pub fn attribute(surface: &Surface, elevation: f64, rooms: &[Candidate]) -> Vec<SurfaceRoom> {
    let Some(shape) = surface_shape(surface) else {
        return Vec::new(); // unmeasurable surface: exported, but nothing to place
    };
    let surface_area = shape.unsigned_area();
    if surface_area <= 0.0 {
        return Vec::new();
    }
    // A bounding-box reject before the boolean op. `/ceilings` measured 33 s on
    // RHH with every ceiling intersected against every room on its storey, and
    // a floor plate is the worst case for that: one slab, every room. Nearly
    // every pair on a storey is disjoint, and a rect test settles those for
    // nothing. It cannot change an answer -- boxes that miss cannot hold
    // polygons that overlap.
    let Some(bounds) = shape.bounding_rect() else {
        return Vec::new();
    };

    let mut out: Vec<SurfaceRoom> = Vec::new();
    for room in rooms {
        if (room.elevation - elevation).abs() > LEVEL_EPS_MM {
            continue;
        }
        if !room.outline.bounding_rect().is_some_and(|r| r.intersects(&bounds)) {
            continue;
        }
        let overlap_shape = shape.intersection(&room.outline);
        let overlap = overlap_shape.unsigned_area();
        let edge = perimeter(&overlap_shape);
        // An empty intersection has no perimeter, and reads as zero width.
        let mean_width = if edge > 0.0 { 2.0 * overlap / edge } else { 0.0 };
        if mean_width < MIN_OVERLAP_MEAN_WIDTH_FT {
            continue;
        }
        let room_area = room.outline.unsigned_area();
        let fraction_of_element = overlap / surface_area;
        let fraction_of_room = if room_area > 0.0 { overlap / room_area } else { 0.0 };
        out.push(SurfaceRoom {
            room_id: room.reference.room_id.clone(),
            model_id: room.reference.model_id.clone(),
            overlap_area: overlap,
            fraction_of_element,
            fraction_of_room,
            mean_width,
        });
    }
    // Largest first, so the room a surface mostly belongs to reads first and a
    // consumer wanting a single owner can take `rooms[0]` without inventing its
    // own rule. Then by room id. **The tie-break is not decoration**: RHH has
    // ceilings lying over six rooms with byte-identical overlap areas (one
    // covers six at exactly 86.448 sqft), and `partial_cmp` alone leaves their
    // order to whatever the scan produced. `rooms[0]` is documented as a usable
    // single owner, so an arbitrary winner among equals would make that answer
    // unreproducible between two reads of the same data.
    out.sort_by(|a, b| {
        b.overlap_area
            .partial_cmp(&a.overlap_area)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.room_id.cmp(&b.room_id))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Point2D, SurfacePolygon};
    use crate::service::room_locator::RoomRef;

    /// An axis-aligned rectangle as a `Loop`, in feet.
    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Loop {
        Loop {
            points: vec![
                Point2D { x: x0, y: y0 },
                Point2D { x: x1, y: y0 },
                Point2D { x: x1, y: y1 },
                Point2D { x: x0, y: y1 },
            ],
        }
    }

    /// A surface from its PIECES -- each piece an outer ring plus any holes.
    fn surface(id: &str, pieces: Vec<Vec<Loop>>) -> Surface {
        Surface {
            id: id.to_string(),
            level_id: "L1".to_string(),
            height_offset: None,
            polygons: pieces.into_iter().map(|loops| SurfacePolygon { loops }).collect(),
            properties: Default::default(),
            type_properties: Default::default(),
            type_id: None,
            type_name: None,
        }
    }

    fn room(id: &str, x0: f64, y0: f64, x1: f64, y1: f64, elevation: f64) -> Candidate {
        Candidate {
            reference: RoomRef { model_id: "M1".to_string(), room_id: id.to_string() },
            outline: Polygon::new(ring(&rect(x0, y0, x1, y1)), vec![]),
            elevation,
        }
    }

    #[test]
    fn test_ceiling_inside_one_room_is_attributed_to_it() {
        // 10x10 ceiling wholly inside a 12x12 room.
        let c = surface("c1", vec![vec![rect(1.0, 1.0, 11.0, 11.0)]]);
        let rooms = [room("r1", 0.0, 0.0, 12.0, 12.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].room_id, "r1");
        assert!((out[0].overlap_area - 100.0).abs() < 1e-6);
        assert!((out[0].fraction_of_element - 1.0).abs() < 1e-9, "wholly inside");
        // Coverage is the OTHER question and must not be confused with it.
        assert!((out[0].fraction_of_room - 100.0 / 144.0).abs() < 1e-9);
        // 2 x 100 / 40: a 10x10 square is 5 ft "wide" as a strip.
        assert!((out[0].mean_width - 5.0).abs() < 1e-9);
    }

    /// **What the tolerance is for.** Two polygons that share an edge touch
    /// without overlapping, and a boolean op on them can return a sliver of
    /// rounding rather than exactly nothing. That is not the surface being in
    /// the room, whichever entity it is.
    #[test]
    fn test_rounding_along_a_shared_edge_is_not_an_overlap() {
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0)]]);
        let touching = [room("r1", 10.0, 0.0, 20.0, 10.0, 0.0)];
        assert!(attribute(&c, 0.0, &touching).is_empty(), "an edge is not an area");

        // 1e-7 ft past the wall line along 10 ft: rounding, not a width.
        let grazing = [room("r1", 10.0 - 1e-7, 0.0, 20.0, 10.0, 0.0)];
        assert!(attribute(&c, 0.0, &grazing).is_empty(), "rounding is not an overlap");
    }

    /// **Why the tolerance is a width.** A finish edge 3 mm past the room
    /// boundary along a 10 m wall is 0.03 sqm -- thirty times a 0.001 sqm area
    /// tolerance -- and it is still imprecision, not the surface being in the
    /// neighbouring room. Its width is 3 mm however long the wall is.
    #[test]
    fn test_a_few_millimetres_along_a_long_wall_is_not_an_overlap() {
        let mm = 1.0 / 304.8;
        let wall = 10_000.0 * mm;
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 20.0, wall)]]);
        let rooms = [room("r1", 20.0 - 3.0 * mm, 0.0, 40.0, wall, 0.0)];
        assert!(attribute(&c, 0.0, &rooms).is_empty(), "3 mm is imprecision, whatever the length");
    }

    /// The tolerance is 10 mm, and just above it is a real overlap.
    #[test]
    fn test_the_tolerance_is_ten_millimetres_of_width() {
        assert!((MIN_OVERLAP_MEAN_WIDTH_FT * 304.8 - 10.0).abs() < 1e-12);
        let mm = 1.0 / 304.8;
        let wall = 10_000.0 * mm;
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 20.0, wall)]]);
        let rooms = [room("r1", 20.0 - 12.0 * mm, 0.0, 40.0, wall, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 1, "12 mm is over the line");
        assert!(out[0].mean_width >= MIN_OVERLAP_MEAN_WIDTH_FT);
    }

    /// **A sliver is reported, not dropped** -- with the measurements that say
    /// it is one. House A's ceiling 2563038 reaching 0.97 sqft into two
    /// neighbours is this shape; the read used to drop it at 0.5% of the
    /// ceiling, and now a consumer draws that line from `fraction_of_element`.
    #[test]
    fn test_a_sliver_of_a_large_ceiling_is_reported_with_its_fraction() {
        // A 6,000 sqft ceiling reaching 0.2 ft into a room along 40 ft: 8 sqft.
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 400.0, 15.0)]]);
        let rooms = [room("r1", 0.0, 15.0 - 0.2, 40.0, 30.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 1);
        assert!((out[0].overlap_area - 8.0).abs() < 1e-4, "8 sqft, give or take the boolean op");
        assert!(out[0].fraction_of_element < 0.005, "the measurement a consumer filters on");
    }

    /// **The cost accepted with the sliver rules' removal.** House A exported two
    /// ceilings of 0.33 and 0.18 sqft, each wholly inside a room. That is an
    /// export fault, well above rounding, so it attributes; reporting it is the
    /// QA report's job.
    #[test]
    fn test_a_degenerate_surface_is_attributed_like_any_other() {
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 0.6, 0.55)]]); // 0.33 sqft
        let rooms = [room("r1", -10.0, -10.0, 10.0, 10.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 1);
        assert!((out[0].fraction_of_element - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_surface_spanning_two_rooms_reports_both_largest_first() {
        // 20x10 ceiling over two rooms: 6ft of it in r_small, 14ft in r_big.
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 20.0, 10.0)]]);
        let rooms = [
            room("r_small", 0.0, 0.0, 6.0, 10.0, 0.0),
            room("r_big", 6.0, 0.0, 20.0, 10.0, 0.0),
        ];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].room_id, "r_big", "largest overlap first, so rooms[0] is a usable single owner");
        assert!((out[0].overlap_area - 140.0).abs() < 1e-6);
        assert!((out[1].overlap_area - 60.0).abs() < 1e-6);
    }

    /// A surface duHast could not measure is exported with an empty polygon
    /// rather than dropped. It must attribute to nothing and must not panic.
    #[test]
    fn test_unmeasurable_surface_is_attributed_to_nothing() {
        let c = surface("c1", vec![]);
        let rooms = [room("r1", 0.0, 0.0, 10.0, 10.0, 0.0)];
        assert!(attribute(&c, 0.0, &rooms).is_empty());
        assert!(attribute(&c, 0.0, &rooms).is_empty());
    }

    /// A degenerate ring -- fewer than three points -- is the other empty shape,
    /// and `surface_shape` has to reject it before `geo` sees it.
    #[test]
    fn test_two_point_loop_is_not_a_polygon() {
        let c = surface(
            "c1",
            vec![vec![Loop {
                points: vec![Point2D { x: 0.0, y: 0.0 }, Point2D { x: 1.0, y: 1.0 }],
            }]],
        );
        let rooms = [room("r1", 0.0, 0.0, 10.0, 10.0, 0.0)];
        assert!(attribute(&c, 0.0, &rooms).is_empty());
    }

    /// Storeys are separated by elevation, not by level id -- a `Level.id` is
    /// per document. A room directly below a surface must not claim it.
    #[test]
    fn test_a_room_on_another_storey_does_not_claim_the_surface() {
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0)]]);
        let rooms = [room("r_below", 0.0, 0.0, 10.0, 10.0, -3000.0)];
        assert!(attribute(&c, 0.0, &rooms).is_empty(), "same plan position, different storey");
    }

    /// Equal overlaps must order deterministically, or `rooms[0]` -- documented
    /// as a usable single owner -- is a different room between two reads of the
    /// same data. RHH has a ceiling over six rooms at identical area.
    #[test]
    fn test_equal_overlaps_break_the_tie_on_room_id() {
        // One 10x10 ceiling split exactly in half by two rooms: 50 sqft each.
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0)]]);
        let forward = [
            room("b_room", 0.0, 0.0, 5.0, 10.0, 0.0),
            room("a_room", 5.0, 0.0, 10.0, 10.0, 0.0),
        ];
        let reversed = [
            room("a_room", 5.0, 0.0, 10.0, 10.0, 0.0),
            room("b_room", 0.0, 0.0, 5.0, 10.0, 0.0),
        ];
        let one = attribute(&c, 0.0, &forward);
        let two = attribute(&c, 0.0, &reversed);
        assert_eq!(one.len(), 2);
        assert!((one[0].overlap_area - one[1].overlap_area).abs() < 1e-9, "the areas really are equal");
        assert_eq!(one[0].room_id, "a_room", "the tie breaks on room id, not on scan order");
        assert_eq!(
            one.iter().map(|r| r.room_id.as_str()).collect::<Vec<_>>(),
            two.iter().map(|r| r.room_id.as_str()).collect::<Vec<_>>(),
            "candidate order must not change the answer",
        );
    }

    /// **House A's shape.** duHast exports one polygon per horizontal face, so a
    /// slab arrives as its top and its bottom -- the same ring twice. The union
    /// must collapse them (`A ∪ A = A`); a sum would report double the area and
    /// a surface covering twice the room it actually covers.
    #[test]
    fn test_duplicate_faces_are_not_double_counted() {
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0)], vec![rect(0.0, 0.0, 10.0, 10.0)]]);
        let rooms = [room("r1", 0.0, 0.0, 10.0, 10.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 1);
        assert!((out[0].overlap_area - 100.0).abs() < 1e-6, "the duplicate face must not double the area");
        assert!((out[0].fraction_of_element - 1.0).abs() < 1e-9, "and the surface is 100 sqft, not 200");
    }

    /// **RHH's shape, and the reason the field is a list.** Two disjoint pieces
    /// are one surface. Taking the first would lose the second entirely -- 44%
    /// of ceiling area on average across RHH's 48 multi-polygon ceilings, 99.2%
    /// at worst -- and taking the largest would still lose the smaller piece.
    #[test]
    fn test_disjoint_pieces_are_one_surface() {
        // 100 sqft at the origin and 25 sqft well away from it.
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0)], vec![rect(50.0, 0.0, 55.0, 5.0)]]);
        let rooms = [
            room("r_big", -1.0, -1.0, 11.0, 11.0, 0.0),
            room("r_far", 49.0, -1.0, 56.0, 6.0, 0.0),
        ];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 2, "both pieces attribute, so the surface is in both rooms");
        assert_eq!(out[0].room_id, "r_big", "largest overlap first");
        assert!((out[0].overlap_area - 100.0).abs() < 1e-6);
        assert!((out[1].overlap_area - 25.0).abs() < 1e-6);
        // The denominator is the WHOLE surface, both pieces: 125 sqft.
        assert!((out[0].fraction_of_element - 100.0 / 125.0).abs() < 1e-9);
    }

    /// A hole is a light well, a void over an atrium, a stair opening, and RHH
    /// exports 173 holed ceiling polygons. It must come out of the surface's own
    /// area, or a slab with a large void reports covering ground it does not.
    #[test]
    fn test_a_hole_is_subtracted_from_the_surface() {
        // 10x10 outer with a 4x4 void punched out of the middle: 100 - 16 = 84.
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0), rect(3.0, 3.0, 7.0, 7.0)]]);
        let rooms = [room("r1", 0.0, 0.0, 10.0, 10.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 1);
        assert!((out[0].overlap_area - 84.0).abs() < 1e-6, "the void is not the surface");
        assert!(
            (out[0].fraction_of_element - 1.0).abs() < 1e-9,
            "all of the surface that exists is in the room"
        );
    }

    /// **An island inside a hole is its own piece, and the union puts it back.**
    /// duHast classifies a face's loops by containment: a loop inside a hole is
    /// not a second hole but a new outer ring, exported as a separate piece. A
    /// floor sketched as a ring with a platform in its opening therefore covers
    /// the platform, and the room under the platform owns it.
    #[test]
    fn test_an_island_in_a_hole_is_part_of_the_surface() {
        // 20x20 with a 10x10 opening, and a 4x4 island inside the opening.
        let c = surface(
            "f1",
            vec![
                vec![rect(0.0, 0.0, 20.0, 20.0), rect(5.0, 5.0, 15.0, 15.0)],
                vec![rect(8.0, 8.0, 12.0, 12.0)],
            ],
        );
        let rooms = [room("r_under_island", 7.0, 7.0, 13.0, 13.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 1, "the island is floor, so the room under it is on it");
        assert!((out[0].overlap_area - 16.0).abs() < 1e-6, "the island, and not the opening around it");
        assert!((out[0].fraction_of_element - 16.0 / 316.0).abs() < 1e-9, "400 - 100 + 16");
    }

    /// **The plate case, which is why a fraction of the element was never right
    /// for floors.** A small room wholly on a storey-sized slab is 0.27% of the
    /// slab -- under the old ceiling line -- and it is on that slab.
    #[test]
    fn test_a_small_room_on_a_large_slab_is_attributed() {
        // 200x150 = 30,000 sqft slab; an 8x10 = 80 sqft store room on it.
        let slab = surface("slab", vec![vec![rect(0.0, 0.0, 200.0, 150.0)]]);
        let rooms = [room("store", 50.0, 50.0, 58.0, 60.0, 0.0)];
        let out = attribute(&slab, 0.0, &rooms);
        assert_eq!(out.len(), 1);
        assert!((out[0].fraction_of_room - 1.0).abs() < 1e-9);
        assert!(out[0].fraction_of_element < 0.005);
    }

    /// **A strip under a wall is reported, and reads as one.** A finish floor
    /// drawn to the far face of a 1 ft wall reaches 1 ft into the neighbour
    /// along the whole wall: 15 sqft. The read used to drop it on its width;
    /// now it rides the list behind the room the floor serves, with the
    /// `mean_width` that says what it is.
    #[test]
    fn test_a_strip_under_a_wall_is_reported_behind_the_room_it_serves() {
        // The floor serves x 0..10; the neighbour is x 9..24, so a 1 ft strip.
        let floor = surface("f1", vec![vec![rect(0.0, 0.0, 10.0, 15.0)]]);
        let rooms = [
            room("home", 0.0, 0.0, 9.0, 15.0, 0.0),
            room("neighbour", 9.0, 0.0, 24.0, 15.0, 0.0),
        ];
        let out = attribute(&floor, 0.0, &rooms);
        assert_eq!(out.iter().map(|r| r.room_id.as_str()).collect::<Vec<_>>(), vec!["home", "neighbour"]);
        assert!((out[1].overlap_area - 15.0).abs() < 1e-6);
        assert!(out[1].mean_width < 1.5, "narrower than a wall: the measurement a consumer filters on");
    }

    /// A narrow room wholly on a floor is on it. A 1 ft x 6 ft riser.
    #[test]
    fn test_a_narrow_room_wholly_covered_is_attributed() {
        let slab = surface("slab", vec![vec![rect(0.0, 0.0, 100.0, 100.0)]]);
        let rooms = [room("riser", 10.0, 10.0, 11.0, 16.0, 0.0)];
        let out = attribute(&slab, 0.0, &rooms);
        assert_eq!(out.len(), 1);
        assert!((out[0].fraction_of_room - 1.0).abs() < 1e-9);
    }

    /// **House A's hob, measured.** `HOB_90mm` (floor `4594251`) is 7.4 sqft
    /// with a mean width of 0.21 ft, 92% of it inside WC 00.03 and 7.5% of the
    /// room. The first floor sliver rule reported it as belonging to no room.
    #[test]
    fn test_a_narrow_floor_wholly_in_one_room_is_attributed() {
        // A 0.25 x 12.4 ft hob in a 10 x 10 WC, reaching 2.4 ft past its wall.
        let hob = surface("hob", vec![vec![rect(0.0, 5.0, 12.4, 5.25)]]);
        let rooms = [room("wc", 0.0, 0.0, 10.0, 10.0, 0.0)];
        let out = attribute(&hob, 0.0, &rooms);
        assert_eq!(out.len(), 1, "the hob is in the WC");
        assert!(out[0].fraction_of_element > 0.8);
    }

    /// The bounding-box reject must never change an answer: an L-shaped room
    /// whose box overlaps the surface's while the polygons themselves miss is
    /// still correctly unattributed, by the boolean op the box let through.
    #[test]
    fn test_overlapping_boxes_are_not_an_overlap() {
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 4.0, 4.0)]]);
        let l_room = Candidate {
            reference: RoomRef { model_id: "M1".to_string(), room_id: "l".to_string() },
            outline: Polygon::new(
                LineString::from(vec![
                    (5.0, -2.0),
                    (20.0, -2.0),
                    (20.0, 20.0),
                    (-2.0, 20.0),
                    (-2.0, 5.0),
                    (5.0, 5.0),
                ]),
                vec![],
            ),
            elevation: 0.0,
        };
        assert!(attribute(&c, 0.0, &[l_room]).is_empty());
    }
}
