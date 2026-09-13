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
//! geometry -- no store, no settings, no scope -- and it is where both entities'
//! thresholds and every measured case live as tests. Together the two would sit
//! well past the ~500-line split line.
//!
//! ## One absolute threshold, and one sliver rule PER ENTITY
//!
//! [`MIN_OVERLAP_AREA`] is shared: it exists for the degenerate *element*, which
//! overlaps a room by 100% of itself and so defeats any relative test. What
//! differs is how a sliver is recognised, and it has to, because the two
//! entities differ in SIZE relative to a room.
//!
//! **A ceiling is roughly room-sized**, so a sliver is a small fraction of the
//! ceiling ([`SliverRule::FractionOfSurface`], measured on House A and RHH --
//! see [`MIN_FRACTION_OF_CEILING`]).
//!
//! **A floor may be a whole plate.** A structural slab under a hospital storey
//! is tens of thousands of square feet, and against that every WC and store
//! room is a "small fraction of the floor" -- the ceiling rule at 0.5% of a
//! 30,000 sqft slab discards every room under 150 sqft, silently. duHast's own
//! `_intersect_floor_vs_room` divides by the ROOM instead, which fixes that
//! case and breaks the other: a strip of finish floor reaching under a wall
//! into a small neighbour is a large fraction of it. Neither operand
//! recognises a sliver, because a sliver is a SHAPE -- long and thin -- not a
//! proportion. So floors use [`SliverRule::NarrowOverlap`]: the overlap's mean
//! width, `2 x area / perimeter`, which for a strip is its width whatever its
//! length and whatever the slab's size.
//!
//! **Measured once, on House A (2026-09-13, 85 floors), and it moved.** The
//! same geometry was attributed by the server under both rules: the floor rule
//! kept 93 pairs and the ceiling rule 114, every floor-rule pair among them.
//! Of the 21 the floor rule dropped, 14 are strips -- mean width 0.16-1.46 ft,
//! a minor share of both the floor and the room. The other 7 were WRONG: a hob,
//! a stair surround, a step, a small slab, each 92-100% inside one room and
//! narrow only because the element itself is narrow. The first escape looked at
//! the room; these needed one for the FLOOR, [`MIN_FLOOR_COVER_FOR_NARROW`].
//!
//! **What House A could not test**: it has no slab large enough for the ceiling
//! rule to drop a room (its largest is 1,618 sqft), so the case that forced a
//! per-entity rule is still reasoned rather than seen. And the 1.5 ft line does
//! not sit in a gap -- the narrowest pair kept is 1.52 ft, the widest dropped
//! 1.46 -- so it is a line through a thin population, not a measured boundary.

use geo::{Area, BooleanOps, BoundingRect, Coord, Euclidean, Intersects, Length, LineString, MultiPolygon, Polygon};
use serde::Serialize;

use crate::contract::{Loop, Surface};
use crate::settings::AreaPolicy;

use super::room_locator::{Candidate, LEVEL_EPS_MM};

/// The smallest overlap, in square feet, that counts as a surface being in a
/// room. It exists for the degenerate *surface*, which no relative test can
/// reject because it overlaps by 100% of itself.
///
/// Measured on ceilings: House A exported two footprints of 0.33 and 0.18 sqft
/// against a population whose next smallest is two orders of magnitude larger,
/// and the false and genuine overlaps are separated by an **18x gap** (0.97
/// below, 17.76 above). RHH did not move it: its 3 degenerate ceilings are
/// 2.09-4.47 sqft and match rooms only by slivers. Shared with floors because
/// a degenerate footprint is a property of the export, not of the category.
pub const MIN_OVERLAP_AREA: f64 = 1.0;

/// The smallest fraction of a **ceiling** that must lie in a room for the room
/// to own it.
///
/// 0.005 against a measured population where genuine matches are 98.7-100% and
/// slivers are 0.414% (House A's ceiling `2563038` reaching 0.97 sqft into two
/// neighbours). The gap is three orders of magnitude, so the value is not
/// delicate. **A fraction of the ceiling, not of the room**: duHast's own
/// `_intersect_ceiling_vs_room` divides by the ROOM's area despite naming its
/// variable for the ceiling, so a sliver against a large room passes more
/// easily than a real overlap against a small one.
///
/// Ceilings only -- see the module header for why this operand is wrong for a
/// floor.
pub const MIN_FRACTION_OF_CEILING: f64 = 0.005;

/// The narrowest mean width, in feet, an overlap may have before it reads as a
/// strip under a wall rather than a floor in a room.
///
/// The project default for `[areas] max_wall_thickness`, and deliberately
/// that constant rather than a number of its own: a finish floor drawn to a
/// wall's far face reaches into the neighbour by at most one wall thickness,
/// and that is the widest strip this rule exists to reject. The DEFAULT and
/// not the project's value, because attribution is request-independent -- the
/// same rule `service::surfaces` follows in never reading `room_resolution` --
/// and reading a setting here would let an areas tolerance silently change
/// which rooms own which floors.
pub const MIN_FLOOR_MEAN_WIDTH_FT: f64 = AreaPolicy::DEFAULT_MAX_WALL_THICKNESS_FT;

/// How much of a ROOM a narrow overlap must cover to be kept anyway.
///
/// The escape the mean-width test needs: a riser cupboard or a duct-sized room
/// wholly under a slab is narrow, and it is still a room on that floor. Half
/// of it is the line between "this room is on this floor" and "this floor
/// grazes this room".
pub const MIN_ROOM_COVER_FOR_NARROW: f64 = 0.5;

/// How much of the FLOOR a narrow overlap must be to be kept anyway.
///
/// The escape House A showed was missing. A strip under a wall is a sliver of
/// the floor it belongs to; a hob, a stair surround or a step is narrow because
/// the element is, and lies wholly in one room. Measured: 7 floors at 92-100%
/// of themselves inside one room were reported roomless without it, and every
/// strip the rule still drops is at most 24% of its floor.
pub const MIN_FLOOR_COVER_FOR_NARROW: f64 = 0.5;

/// How a sliver is recognised -- the one thing that differs between the two
/// entities' attribution. See the module header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliverRule {
    /// Ceilings: too small a fraction of the surface.
    FractionOfSurface,
    /// Floors: too narrow, unless it is most of the room or most of the floor.
    NarrowOverlap,
}

/// One room a surface lies over, with the measurements that justify the claim.
///
/// **Every number rides the wire**, because they answer different questions
/// and only one of them decided this entry. `fraction_of_element` is coverage
/// of the surface, `fraction_of_room` is coverage of the room -- "is this room
/// fully ceiled", which a finishes take-off wants -- and `mean_width` is what
/// the floor rule tested. A consumer re-arguing a threshold needs all three.
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
    /// overlap if it were a strip. See [`MIN_FLOOR_MEAN_WIDTH_FT`].
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
/// the conservative rule, which RHH's ceilings confirmed. **Floors have not
/// been probed**, and a base-build model holding slabs beside fit-out models
/// holding rooms is exactly the layout that would break it.
pub fn attribute(surface: &Surface, elevation: f64, rooms: &[Candidate], rule: SliverRule) -> Vec<SurfaceRoom> {
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
        if overlap < MIN_OVERLAP_AREA {
            continue;
        }
        let room_area = room.outline.unsigned_area();
        let fraction_of_element = overlap / surface_area;
        let fraction_of_room = if room_area > 0.0 { overlap / room_area } else { 0.0 };
        let edge = perimeter(&overlap_shape);
        let mean_width = if edge > 0.0 { 2.0 * overlap / edge } else { 0.0 };

        let sliver = match rule {
            SliverRule::FractionOfSurface => fraction_of_element < MIN_FRACTION_OF_CEILING,
            SliverRule::NarrowOverlap => {
                mean_width < MIN_FLOOR_MEAN_WIDTH_FT
                    && fraction_of_room < MIN_ROOM_COVER_FOR_NARROW
                    && fraction_of_element < MIN_FLOOR_COVER_FOR_NARROW
            }
        };
        if sliver {
            continue;
        }
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

    const CEILING: SliverRule = SliverRule::FractionOfSurface;
    const FLOOR: SliverRule = SliverRule::NarrowOverlap;

    #[test]
    fn test_ceiling_inside_one_room_is_attributed_to_it() {
        // 10x10 ceiling wholly inside a 12x12 room.
        let c = surface("c1", vec![vec![rect(1.0, 1.0, 11.0, 11.0)]]);
        let rooms = [room("r1", 0.0, 0.0, 12.0, 12.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms, CEILING);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].room_id, "r1");
        assert!((out[0].overlap_area - 100.0).abs() < 1e-6);
        assert!((out[0].fraction_of_element - 1.0).abs() < 1e-9, "wholly inside");
        // Coverage is the OTHER question and must not be confused with it.
        assert!((out[0].fraction_of_room - 100.0 / 144.0).abs() < 1e-9);
        // 2 x 100 / 40: a 10x10 square is 5 ft "wide" as a strip.
        assert!((out[0].mean_width - 5.0).abs() < 1e-9);
    }

    /// The measured false positive that `MIN_FRACTION_OF_CEILING` exists for:
    /// House A's ceiling 2563038 reaching 0.97 sqft into two neighbours, 0.414%
    /// of itself. A fraction-of-ROOM test at duHast's 0.1% keeps these.
    #[test]
    fn test_sliver_of_a_large_ceiling_is_not_attributed() {
        // A 6,000 sqft ceiling reaching 0.2 ft into a room along 40 ft: 8 sqft,
        // clear of MIN_OVERLAP_AREA and 0.13% of the ceiling.
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 400.0, 15.0)]]);
        let rooms = [room("r1", 0.0, 15.0 - 0.2, 40.0, 30.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms, CEILING);
        let overlap = 40.0 * 0.2;
        assert!(overlap >= MIN_OVERLAP_AREA, "this case is the FRACTION guard, not the area one");
        assert!(overlap / 6000.0 < MIN_FRACTION_OF_CEILING);
        assert!(out.is_empty(), "a sliver is not a ceiling being in a room");
    }

    /// The other measured false positive, and the reason one threshold cannot
    /// do both jobs: House A exported two ceilings of 0.33 and 0.18 sqft, and
    /// each lies 100% inside a room. No fraction test can reject them.
    #[test]
    fn test_degenerate_surface_is_rejected_on_absolute_area() {
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 0.6, 0.55)]]); // 0.33 sqft
        let rooms = [room("r1", -10.0, -10.0, 10.0, 10.0, 0.0)];
        assert!(
            attribute(&c, 0.0, &rooms, CEILING).is_empty(),
            "wholly inside, so only the area guard can reject it"
        );
        assert!(attribute(&c, 0.0, &rooms, FLOOR).is_empty(), "and the guard is shared with floors");
    }

    #[test]
    fn test_surface_spanning_two_rooms_reports_both_largest_first() {
        // 20x10 ceiling over two rooms: 6ft of it in r_small, 14ft in r_big.
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 20.0, 10.0)]]);
        let rooms = [
            room("r_small", 0.0, 0.0, 6.0, 10.0, 0.0),
            room("r_big", 6.0, 0.0, 20.0, 10.0, 0.0),
        ];
        let out = attribute(&c, 0.0, &rooms, CEILING);
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
        assert!(attribute(&c, 0.0, &rooms, CEILING).is_empty());
        assert!(attribute(&c, 0.0, &rooms, FLOOR).is_empty());
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
        assert!(attribute(&c, 0.0, &rooms, CEILING).is_empty());
    }

    /// Storeys are separated by elevation, not by level id -- a `Level.id` is
    /// per document. A room directly below a surface must not claim it.
    #[test]
    fn test_a_room_on_another_storey_does_not_claim_the_surface() {
        let c = surface("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0)]]);
        let rooms = [room("r_below", 0.0, 0.0, 10.0, 10.0, -3000.0)];
        assert!(attribute(&c, 0.0, &rooms, CEILING).is_empty(), "same plan position, different storey");
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
        let one = attribute(&c, 0.0, &forward, CEILING);
        let two = attribute(&c, 0.0, &reversed, CEILING);
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
        let out = attribute(&c, 0.0, &rooms, CEILING);
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
        let out = attribute(&c, 0.0, &rooms, CEILING);
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
        let out = attribute(&c, 0.0, &rooms, CEILING);
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
        let out = attribute(&c, 0.0, &rooms, FLOOR);
        assert_eq!(out.len(), 1, "the island is floor, so the room under it is on it");
        assert!((out[0].overlap_area - 16.0).abs() < 1e-6, "the island, and not the opening around it");
        assert!((out[0].fraction_of_element - 16.0 / 316.0).abs() < 1e-9, "400 - 100 + 16");
    }

    /// **The case that forbids reusing the ceiling rule for floors.** A small
    /// room wholly on a storey-sized slab is a tiny fraction of the slab, so
    /// `MIN_FRACTION_OF_CEILING` discards it; the floor rule keeps it.
    #[test]
    fn test_a_small_room_on_a_large_slab_is_attributed() {
        // 200x150 = 30,000 sqft slab; an 8x10 = 80 sqft store room on it.
        let slab = surface("slab", vec![vec![rect(0.0, 0.0, 200.0, 150.0)]]);
        let rooms = [room("store", 50.0, 50.0, 58.0, 60.0, 0.0)];
        const { assert!(80.0 / 30_000.0 < MIN_FRACTION_OF_CEILING, "under the ceiling line by construction") };

        assert!(attribute(&slab, 0.0, &rooms, CEILING).is_empty(), "the ceiling rule loses it");
        let out = attribute(&slab, 0.0, &rooms, FLOOR);
        assert_eq!(out.len(), 1, "the floor rule does not");
        assert!((out[0].fraction_of_room - 1.0).abs() < 1e-9);
    }

    /// **The floor sliver: a strip under a wall.** A finish floor drawn to the
    /// far face of a 1 ft wall reaches 1 ft into the neighbour along the whole
    /// wall. That is 15 sqft -- far above `MIN_OVERLAP_AREA`, and a sizeable
    /// fraction of both a small floor and a small room -- and it is still not
    /// that floor being in that room. Only its SHAPE says so.
    #[test]
    fn test_a_strip_under_a_wall_is_not_attributed_to_the_neighbour() {
        // The floor serves x 0..10; the neighbour is x 9..24, so a 1 ft strip.
        let floor = surface("f1", vec![vec![rect(0.0, 0.0, 10.0, 15.0)]]);
        let rooms = [
            room("home", 0.0, 0.0, 9.0, 15.0, 0.0),
            room("neighbour", 9.0, 0.0, 24.0, 15.0, 0.0),
        ];
        let out = attribute(&floor, 0.0, &rooms, FLOOR);
        assert_eq!(out.iter().map(|r| r.room_id.as_str()).collect::<Vec<_>>(), vec!["home"]);

        // And the ceiling rule would have kept it: 15 sqft is 10% of the element.
        let ceiling = attribute(&floor, 0.0, &rooms, CEILING);
        assert_eq!(ceiling.len(), 2, "which is why the rule is per entity");
    }

    /// The escape: a narrow room wholly on a floor is still on it. A 1 ft x 6 ft
    /// riser has a mean width under the strip line, and covering all of it is
    /// what says it is a room rather than a graze.
    #[test]
    fn test_a_narrow_room_wholly_covered_is_attributed() {
        let slab = surface("slab", vec![vec![rect(0.0, 0.0, 100.0, 100.0)]]);
        let rooms = [room("riser", 10.0, 10.0, 11.0, 16.0, 0.0)];
        let out = attribute(&slab, 0.0, &rooms, FLOOR);
        assert_eq!(out.len(), 1);
        assert!(out[0].mean_width < MIN_FLOOR_MEAN_WIDTH_FT, "narrow by the strip test");
        assert!(out[0].fraction_of_room >= MIN_ROOM_COVER_FOR_NARROW, "and kept for covering the room");
    }

    /// **House A's hob, measured.** `HOB_90mm` (floor `4594251`) is 7.4 sqft
    /// with a mean width of 0.21 ft, 92% of it inside WC 00.03 and 7.5% of the
    /// room. Narrow, and a minor part of the room -- but it is the whole floor,
    /// in one room, which a strip under a wall never is. Without the floor-cover
    /// escape it was reported as belonging to no room.
    #[test]
    fn test_a_narrow_floor_wholly_in_one_room_is_attributed() {
        // A 0.25 x 12.4 ft hob in a 10 x 10 WC, reaching 2.4 ft past its wall.
        let hob = surface("hob", vec![vec![rect(0.0, 5.0, 12.4, 5.25)]]);
        let rooms = [room("wc", 0.0, 0.0, 10.0, 10.0, 0.0)];
        let out = attribute(&hob, 0.0, &rooms, FLOOR);
        assert_eq!(out.len(), 1, "the hob is in the WC");
        assert!(out[0].mean_width < MIN_FLOOR_MEAN_WIDTH_FT, "narrow by the strip test");
        assert!(out[0].fraction_of_room < MIN_ROOM_COVER_FOR_NARROW, "and a minor part of the room");
        assert!(
            out[0].fraction_of_element >= MIN_FLOOR_COVER_FOR_NARROW,
            "kept for being most of the floor"
        );
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
        assert!(attribute(&c, 0.0, &[l_room], FLOOR).is_empty());
    }
}
