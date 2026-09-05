//! Which room is a wall-hosted element actually in — answered from geometry,
//! for the cases the model does not answer itself.
//!
//! **This module knows nothing about doors.** It takes a point, an optional plan
//! direction, and a set of candidate rooms; it answers which room lies on each
//! side. That is deliberate rather than speculative generalisation: windows have
//! the same `FromRoom`/`ToRoom` shape a door does, and an FF&E instance has a
//! single `Room` — one side rather than two. Keeping the primitives at the
//! boundary means the next category needs the glue, not the geometry. The
//! extractor already made the same split for the same reason:
//! `room_reference(instance, phase, which)` takes the property name as an
//! argument because that is the only thing that varies per category.
//!
//! ## Why a probe, and not point-in-polygon
//!
//! A door sits **in** a wall. Under the finish-face regime its insertion point
//! lands in the gap between two rooms' boundaries, so testing whether that point
//! is inside a room answers "no room" for very nearly every door — the check
//! looks like it works, returns nothing, and would read as "the geometry does
//! not know" rather than as a bug.
//!
//! So the point is stepped off the wall before it is tested: `+normal` reaches
//! the to-side, `−normal` the from-side. That direction convention is
//! **measured, not assumed** — all 20 two-sided doors in the House A export put
//! `to_room` on the `+normal` side and `from_room` behind it.
//!
//! ## What this deliberately does not do
//!
//! It does not reconcile a room reference against the door's *swing*. A door is
//! attributed to the room it **serves**, which the modeller decides, and that is
//! not always the room it opens into — a cupboard off a corridor swings into the
//! corridor and belongs to the cupboard (2 of the 26 House A doors are
//! deliberately that shape). `CLAUDE.md` forbids reconciling those two, and this
//! does not: "is the element physically beside this room" and "does it open into
//! this room" are different claims. The cupboard door's insertion point is still
//! on the cupboard/corridor boundary; a room that has *moved* fails the first
//! test while the second is untouched.

use geo::{Contains, Coord, LineString, Point, Polygon};

use crate::contract::{Loop, Point2D, Room};

/// Why a side could not be resolved. Every one of these is a *reported state*,
/// not a failure — and the first is usually correct rather than a problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unresolved {
    /// The probe landed in no room. **Normally the right answer**: an external
    /// door has open air on one side, and 6 of the 26 House A doors are
    /// one-sided. Never a finding on its own.
    NoCandidate,
    /// Two or more rooms contain the probe point. Overlapping rooms, or two
    /// models whose rooms occupy the same space — resolve nothing rather than
    /// pick, "signal, not error".
    Ambiguous,
    /// No plan direction to step along, so there is nothing to probe. A guessed
    /// direction is worse than none, because nothing downstream could tell it
    /// from a measured one.
    NoDirection,
    /// Neither an insertion point nor a footprint to derive one from.
    NoPosition,
    /// The opening names a level nothing in scope carries an elevation for, so
    /// there is no axis to compare rooms on and nothing was probed at all.
    ///
    /// **Its own state rather than `NoCandidate`, because the two send a reader
    /// to opposite places.** `NoCandidate` says the probe ran and landed in open
    /// air, which for an external opening is the right answer and no finding.
    /// This says the probe never ran, and the cause is upstream: the element is
    /// unhosted, so Revit gave it an invalid `LevelId` and the export carried
    /// `-1`. Measured on real models -- one skylight in a house, two terrace
    /// sills in a facade file -- so it is rare, real, and exactly the kind of
    /// thing somebody would otherwise spend an afternoon looking for in the
    /// geometry.
    ///
    /// Reported, never refused: the element is still a real opening with
    /// properties and an id, and `level_id` stays a required `String` on the
    /// contract carrying whatever the export said. "Signal, not error."
    UnknownLevel,
}

/// One side's answer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "room")]
pub enum Located {
    /// The room the probe landed in, as `(model id, room id)` — qualified
    /// because a room id is unique only within its model, and a probe may reach
    /// a room in a linked model.
    Found(RoomRef),
    Unresolved(Unresolved),
}

/// A room, named unambiguously.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct RoomRef {
    pub model_id: String,
    pub room_id: String,
}

/// Both sides of one wall-hosted element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sides {
    /// The room behind the element, against its facing direction.
    pub from: Located,
    /// The room the element faces.
    pub to: Located,
}

/// One candidate room, already placed in whatever frame the caller is working
/// in. Prepared once per read and probed many times.
pub struct Candidate {
    pub reference: RoomRef,
    /// Outer boundary only. A room's own hole (a column, a shaft) is not
    /// somewhere an element can be, and treating it as a hole would make a probe
    /// that lands on a column read as "no room".
    pub outline: Polygon<f64>,
    /// Level elevation, in whatever unit the level list states — millimetres
    /// today; see `LEVEL_EPS_MM`. The axis a probe never crosses. Compared by
    /// elevation rather than by `level_id`, because level ids are per-document
    /// and a linked model names the same floor with a different one.
    pub elevation: f64,
}

/// Build a room's outer polygon. `None` for geometry that cannot form one
/// (fewer than three points — an unplaced room).
///
/// A third copy of the same six lines `areas` and `adjacency` each carry, per
/// the house rule that a shared helper is duplicated per module rather than
/// hoisted. What differs between the three is what they do next, not this.
pub fn outline_of(room: &Room) -> Option<Polygon<f64>> {
    let outer = room.loops.first()?;
    if outer.points.len() < 3 {
        return None;
    }
    Some(Polygon::new(ring(outer), vec![]))
}

fn ring(l: &Loop) -> LineString<f64> {
    LineString::from(l.points.iter().map(|p| Coord { x: p.x, y: p.y }).collect::<Vec<_>>())
}

/// How close two levels must be to count as the same floor, in **the units the
/// level list is stated in** — which are millimetres, not feet.
///
/// Levels are matched by elevation because ids are per-document. The intent has
/// always been "far tighter than any storey height and far looser than the float
/// noise a transform introduces", so that this separates "the same floor named
/// twice" from "the floor above" without being sensitive to either.
///
/// **This constant was `LEVEL_EPS_FT = 0.5` and was doing neither job.** Level
/// elevations arrive in millimetres, because duHast's `to_data_level_building`
/// runs them through `convert_imperial_feet_to_metric_mm` — House A's LEVEL 00
/// is `110250.0` on the wire — while every polygon on the same wire is decimal
/// feet. Nothing was ever wrong arithmetically, because an elevation is only
/// ever compared to another elevation and never to geometry; what was wrong was
/// the constant, which read as half a foot and behaved as **half a millimetre**.
/// Two models naming one floor 0.4 mm apart failed to match, silently, as
/// `UnknownLevel`.
///
/// 50 mm is the same *intent* the old value stated: two orders of magnitude
/// below any storey height, and far above the float noise of an affine
/// transform through `model_to_shared`.
///
/// **The mixed units are not fixed here, deliberately.** Changing the wire would
/// re-scale every level in every snapshot already stored, which is a migration;
/// this is a constant that was mislabelled. The mix is real and worth knowing
/// about — see `docs/PLAN-ffe.md` D9, which found it — and the name now says
/// which side of it this value lives on.
pub const LEVEL_EPS_MM: f64 = 50.0;

/// Where an element sits, in the frame its candidates are in.
pub struct Placement {
    pub point: Point2D,
    /// Unit vector through the wall, along the direction the element faces.
    /// `None` for an element with no plan direction (a hatch in a floor), which
    /// is why `Unresolved::NoDirection` exists.
    pub normal: Option<Point2D>,
    pub elevation: f64,
}

/// Resolve both sides of one element.
///
/// `probe_ft` is how far off the wall to step, and the caller supplies it from
/// `[areas] max_wall_thickness` via `AreaPolicy::wall_gap_ft` — the same single
/// quantity `areas` sizes its wall zone by and `adjacency` uses as its default
/// gap. A third consumer of one number, rather than a third constant that could
/// drift from the other two.
///
/// A **centreline** model has a gap of zero: neighbouring rooms already tile, so
/// their boundaries are the same line and any positive step lands cleanly inside
/// one of them. The caller passes a small positive epsilon in that case rather
/// than zero, or the probe would test the boundary itself, where containment is
/// undefined.
pub fn locate(placement: &Placement, candidates: &[Candidate], probe_ft: f64) -> Sides {
    let Some(normal) = placement.normal else {
        return Sides {
            from: Located::Unresolved(Unresolved::NoDirection),
            to: Located::Unresolved(Unresolved::NoDirection),
        };
    };
    let step = |sign: f64| {
        Point::new(
            placement.point.x + normal.x * probe_ft * sign,
            placement.point.y + normal.y * probe_ft * sign,
        )
    };
    Sides {
        from: probe(step(-1.0), placement.elevation, candidates),
        to: probe(step(1.0), placement.elevation, candidates),
    }
}

/// Which candidate contains one probe point, on this element's level.
fn probe(at: Point<f64>, elevation: f64, candidates: &[Candidate]) -> Located {
    let mut hit: Option<&Candidate> = None;
    for candidate in candidates {
        if (candidate.elevation - elevation).abs() > LEVEL_EPS_MM {
            continue;
        }
        if !candidate.outline.contains(&at) {
            continue;
        }
        if hit.is_some() {
            // A second containing room. Resolve nothing rather than pick the
            // first — "first wins" would be a stable-looking answer decided by
            // storage order.
            return Located::Unresolved(Unresolved::Ambiguous);
        }
        hit = Some(candidate);
    }
    match hit {
        Some(candidate) => Located::Found(candidate.reference.clone()),
        None => Located::Unresolved(Unresolved::NoCandidate),
    }
}

/// The position to probe from: the element's own insertion point, else the
/// centroid of its footprint.
///
/// The footprint fallback is what makes this work for an element Revit gave no
/// `LocationPoint` — the case named when this was specified. It is a fallback
/// rather than the primary because the insertion point is *placement* while a
/// footprint centroid is a derived average: for a door whose swing arc is in the
/// loop, the two are not the same point.
pub fn position_of(insertion_point: Option<Point2D>, loops: &[Loop]) -> Option<Point2D> {
    if let Some(point) = insertion_point {
        return Some(point);
    }
    let outer = loops.first()?;
    if outer.points.is_empty() {
        return None;
    }
    let n = outer.points.len() as f64;
    Some(Point2D {
        x: outer.points.iter().map(|p| p.x).sum::<f64>() / n,
        y: outer.points.iter().map(|p| p.y).sum::<f64>() / n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon<f64> {
        Polygon::new(LineString::from(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]), vec![])
    }

    fn candidate(model: &str, room: &str, outline: Polygon<f64>, elevation: f64) -> Candidate {
        Candidate {
            reference: RoomRef { model_id: model.into(), room_id: room.into() },
            outline,
            elevation,
        }
    }

    /// Two rooms separated by a 0.5 ft wall, with a door in it. The insertion
    /// point sits **in the wall**, in neither room — which is the whole reason
    /// this probes rather than testing containment directly.
    fn wall_case() -> (Placement, Vec<Candidate>) {
        let left = candidate("m1", "left", square(0.0, 0.0, 10.0, 10.0), 0.0);
        let right = candidate("m1", "right", square(10.5, 0.0, 20.0, 10.0), 0.0);
        let placement = Placement {
            point: Point2D { x: 10.25, y: 5.0 },
            normal: Some(Point2D { x: 1.0, y: 0.0 }),
            elevation: 0.0,
        };
        (placement, vec![left, right])
    }

    /// The base case, and the direction convention: `+normal` is the to-side.
    #[test]
    fn test_probe_resolves_both_sides_of_a_wall() {
        let (placement, candidates) = wall_case();
        let sides = locate(&placement, &candidates, 0.5);
        assert_eq!(sides.to, Located::Found(RoomRef { model_id: "m1".into(), room_id: "right".into() }));
        assert_eq!(sides.from, Located::Found(RoomRef { model_id: "m1".into(), room_id: "left".into() }));
    }

    /// **The point itself is in no room.** Stated as its own test because it is
    /// the failure a plain point-in-polygon implementation would have, and it
    /// would look like "the geometry does not know" rather than like a bug.
    #[test]
    fn test_the_insertion_point_alone_resolves_nothing() {
        let (placement, candidates) = wall_case();
        let at = Point::new(placement.point.x, placement.point.y);
        assert!(
            candidates.iter().all(|c| !c.outline.contains(&at)),
            "a door in a wall is inside neither room it connects"
        );
    }

    /// An external door: open air on one side. `NoCandidate` there is the
    /// correct answer and must never read as a failure.
    #[test]
    fn test_an_external_door_resolves_one_side_only() {
        let inside = candidate("m1", "hall", square(0.0, 0.0, 10.0, 10.0), 0.0);
        let placement = Placement {
            point: Point2D { x: 10.25, y: 5.0 },
            normal: Some(Point2D { x: 1.0, y: 0.0 }),
            elevation: 0.0,
        };
        let sides = locate(&placement, &[inside], 0.5);
        assert_eq!(sides.from, Located::Found(RoomRef { model_id: "m1".into(), room_id: "hall".into() }));
        assert_eq!(sides.to, Located::Unresolved(Unresolved::NoCandidate));
    }

    /// A probe reaching two overlapping rooms resolves nothing. "First wins"
    /// would be a stable-looking answer decided by storage order.
    #[test]
    fn test_overlapping_rooms_are_ambiguous_not_first_wins() {
        let a = candidate("m1", "a", square(10.5, 0.0, 20.0, 10.0), 0.0);
        let b = candidate("m2", "b", square(10.5, 0.0, 20.0, 10.0), 0.0);
        let placement = Placement {
            point: Point2D { x: 10.25, y: 5.0 },
            normal: Some(Point2D { x: 1.0, y: 0.0 }),
            elevation: 0.0,
        };
        let sides = locate(&placement, &[a, b], 0.5);
        assert_eq!(sides.to, Located::Unresolved(Unresolved::Ambiguous));
    }

    /// No plan direction, no probe — and no guess.
    #[test]
    fn test_no_direction_resolves_neither_side() {
        let (mut placement, candidates) = wall_case();
        placement.normal = None;
        let sides = locate(&placement, &candidates, 0.5);
        assert_eq!(sides.to, Located::Unresolved(Unresolved::NoDirection));
        assert_eq!(sides.from, Located::Unresolved(Unresolved::NoDirection));
    }

    /// **Levels are matched by elevation, not id.** A room directly above is a
    /// different room, and a probe must never reach it.
    ///
    /// The storey height here is **3600 mm**, and it used to be `12.0` — twelve
    /// feet, written when the epsilon was called `LEVEL_EPS_FT`. The unit
    /// confusion had reached the fixtures, which is exactly why nothing caught
    /// it: a test stating its storey in feet and an epsilon reading as feet
    /// agreed with each other and with nothing on the wire. It fails now if the
    /// epsilon is restored to a foot-scaled value, which is the guard that was
    /// missing.
    #[test]
    fn test_a_room_on_another_level_is_never_reached() {
        let upstairs = candidate("m1", "above", square(10.5, 0.0, 20.0, 10.0), 3600.0);
        let placement = Placement {
            point: Point2D { x: 10.25, y: 5.0 },
            normal: Some(Point2D { x: 1.0, y: 0.0 }),
            elevation: 0.0,
        };
        let sides = locate(&placement, &[upstairs], 0.5);
        assert_eq!(sides.to, Located::Unresolved(Unresolved::NoCandidate));
    }

    /// The same floor named twice by two linked models: elevations agree within
    /// `LEVEL_EPS_MM`, so the probe reaches it.
    ///
    /// 10 mm apart, which is the realistic disagreement — two documents whose
    /// survey base differs by a rounding. Under the old half-a-millimetre
    /// epsilon this pair would have missed, and the model would have been
    /// reported as `UnknownLevel` rather than matched.
    #[test]
    fn test_the_same_floor_in_a_linked_model_is_reached() {
        let linked = candidate("m2", "core", square(10.5, 0.0, 20.0, 10.0), 10.0);
        let placement = Placement {
            point: Point2D { x: 10.25, y: 5.0 },
            normal: Some(Point2D { x: 1.0, y: 0.0 }),
            elevation: 0.0,
        };
        let sides = locate(&placement, &[linked], 0.5);
        assert_eq!(sides.to, Located::Found(RoomRef { model_id: "m2".into(), room_id: "core".into() }));
    }

    /// A probe too short to clear the wall lands back in the gap and resolves
    /// nothing — which is why `probe_ft` comes from the project's declared wall
    /// thickness rather than a constant picked here.
    #[test]
    fn test_a_probe_shorter_than_the_wall_reaches_nothing() {
        let (placement, candidates) = wall_case();
        let sides = locate(&placement, &candidates, 0.1);
        assert_eq!(sides.to, Located::Unresolved(Unresolved::NoCandidate));
        assert_eq!(sides.from, Located::Unresolved(Unresolved::NoCandidate));
    }

    /// The insertion point wins when present; the footprint centroid is the
    /// fallback for an element Revit gave no `LocationPoint`.
    #[test]
    fn test_position_prefers_the_insertion_point_over_the_footprint() {
        let footprint = vec![Loop {
            points: vec![
                Point2D { x: 0.0, y: 0.0 },
                Point2D { x: 4.0, y: 0.0 },
                Point2D { x: 4.0, y: 2.0 },
                Point2D { x: 0.0, y: 2.0 },
            ],
        }];
        let given = position_of(Some(Point2D { x: 99.0, y: 99.0 }), &footprint).unwrap();
        assert_eq!((given.x, given.y), (99.0, 99.0));

        let derived = position_of(None, &footprint).unwrap();
        assert_eq!((derived.x, derived.y), (2.0, 1.0), "centroid of the footprint");

        assert!(position_of(None, &[]).is_none(), "no point and no footprint resolves nothing");
    }
}
