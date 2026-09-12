//! `/ceilings` fetch-side derive logic: scoping, placement, and the **geometric
//! room attribution** that is this entity's whole reason for existing.
//!
//! ## Why this is not `room_locator`
//!
//! Every other dependent entity asks "which room contains this point". A door
//! is in a wall, an item stands somewhere, and both have an authored reference
//! that geometry only ever fills in for. A ceiling is a *surface*: it has no
//! point to test, it can lie over more than one room, and **nothing in Revit
//! says which room it belongs to** — a ceiling has no room parameter and a room
//! has no ceiling parameter. So the question here is not containment but
//! overlap, and the answer is not a fallback but the only answer there is.
//!
//! The room polygons come from [`entity_scope::build_candidates`] rather than a
//! second read of the store, which is what keeps them **exactly the rooms
//! `/rooms` is serving** under the same milestone. A separate reader would
//! answer a different question about one building the moment a milestone was
//! pinned, and nothing would say so.
//!
//! ## The thresholds, and why there are two
//!
//! Measured on House A (2026-09-10, `scripts/analyse_ceilings_probe.py`). Of 26
//! intersecting ceiling/room pairs, 22 are genuine and 4 are not, and the two
//! groups are separated by an **18x gap**: every false positive is under
//! 1 sqft of overlap, every genuine match is 17.76 sqft or more.
//!
//! The four false positives are two different faults, which is why one
//! threshold cannot remove both:
//!
//! - **Slivers off a large ceiling.** Ceiling `2563038` reaches 0.97 sqft into
//!   two neighbouring rooms — 0.414% of itself. [`MIN_FRACTION_OF_CEILING`]
//!   removes these, and it is a fraction of the *ceiling* deliberately:
//!   duHast's own `_intersect_ceiling_vs_room` divides by the ROOM's area
//!   despite its variable being named for the ceiling, which scales with the
//!   wrong operand — a sliver against a large room passes more easily than a
//!   real overlap against a small one.
//! - **Ceilings that are themselves degenerate.** Two exported footprints of
//!   0.33 and 0.18 sqft, against a population whose next smallest is two orders
//!   of magnitude larger. They overlap a room by 100% *of themselves*, so no
//!   fraction test can reject them; [`MIN_OVERLAP_AREA`] does, on absolute size.
//!
//! **Constants, not settings, and that is a deliberate stopping point.** The
//! codebase's rule is to take the configuration when a second project disagrees,
//! not before — the same rule that defers type-property deduplication. One
//! model has been measured. When RHH is probed and wants different numbers,
//! these become a `[ceilings]` block and this comment becomes its rationale.
//!
//! ## What is derived and what is stored
//!
//! Nothing here is stored. Attribution is recomputed on every read, exactly as
//! `[doors] room_attribution` is, and for the same reason: changing the rule
//! changes every answer and rewrites nothing.

use std::collections::BTreeMap;

use geo::{Area, BooleanOps, Coord, LineString, MultiPolygon, Polygon};
use serde::Serialize;

use crate::contract::{Ceiling, CeilingPayload, Level, Loop};
use crate::settings::RoomResolution;
use crate::state::{AppState, ModelKey};
use crate::storage::SnapshotKind;

use super::room_locator::RoomRef;
use super::{entity_scope, ServiceError};

/// The smallest overlap, in square feet, that counts as a ceiling being in a
/// room. See the module header: it exists for the degenerate *ceiling*, which no
/// fraction test can reject because it overlaps by 100% of itself.
///
/// 1.0 sits inside an 18x measured gap (0.97 below, 17.76 above), so it is a
/// value with room on both sides rather than a line drawn through a cluster.
pub const MIN_OVERLAP_AREA: f64 = 1.0;

/// The smallest fraction of a **ceiling** that must lie in a room for the room
/// to own it. See the module header for why the operand is the ceiling and not
/// the room.
///
/// 0.005 against a measured population where genuine matches are 98.7-100% and
/// slivers are 0.414%. The gap is three orders of magnitude, so the value is
/// not delicate.
pub const MIN_FRACTION_OF_CEILING: f64 = 0.005;

/// One room a ceiling lies over, with the measurements that justify the claim.
///
/// **All three numbers ride the wire**, because they answer different
/// questions and only one of them is the attribution rule. `fraction_of_ceiling`
/// is what decided this entry; `fraction_of_room` is coverage — "is this room
/// fully ceiled" — which a finishes take-off wants and the attribution rule must
/// not silently answer for it.
#[derive(Debug, Clone, Serialize)]
pub struct CeilingRoom {
    pub room_id: String,
    /// The model the room belongs to. A room id is unique only *within* a
    /// model, so a bare id here would be ambiguous the moment a project holds
    /// two architectural models — the trap `OpeningResponse::owner_rooms` has
    /// its `_qualified` sibling for.
    pub model_id: String,
    /// Overlap area in square feet.
    pub overlap_area: f64,
    pub fraction_of_ceiling: f64,
    pub fraction_of_room: f64,
}

/// One ceiling as served.
#[derive(Debug, Clone, Serialize)]
pub struct CeilingResponse {
    #[serde(flatten)]
    pub ceiling: Ceiling,
    pub project_id: String,
    /// The model that owns the ceiling — which is not necessarily the model
    /// that owns the rooms in `rooms` below, though on every document measured
    /// so far it is.
    pub model_id: String,
    pub source: String,

    /// Every room this ceiling lies over, largest overlap first.
    ///
    /// **Empty means unattributed, and that is a reported state.** A ceiling
    /// over a stairwell, an external soffit, or one on a level that carries no
    /// rooms at all legitimately belongs to nothing — 6 of House A's 30, all of
    /// one type, 4 of them on a level with no rooms. Never an error, and not on
    /// its own a finding.
    pub rooms: Vec<CeilingRoom>,
}

/// What a `/ceilings` read is scoped to.
pub struct CeilingScope<'a> {
    pub project: Option<&'a str>,
    pub milestone: Option<&'a str>,
}

/// The whole `/ceilings` body.
#[derive(Debug, Clone, Serialize)]
pub struct CeilingsResult {
    pub revision: String,
    pub phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>>,
    pub levels_by_model: BTreeMap<String, Vec<Level>>,
    pub ceilings: Vec<CeilingResponse>,
}

/// A ceiling's plan footprint: the UNION of every piece it exports.
///
/// **The union, and not the first piece or the largest one** -- the rule House A
/// taught wrongly and RHH corrected. duHast exports a ceiling once per
/// horizontal face of its solid, so on House A a slab arrived as its top face,
/// its bottom face and some edge slivers, and the largest piece was the whole
/// ceiling. RHH's pieces are genuinely disjoint instead: against the union,
/// taking the first loses 44.0% of ceiling area on average (99.2% at worst) and
/// taking the largest still loses 25.7%, with 31 of 48 losing over 5%.
///
/// The union is the one operation correct on both. It collapses House A's
/// duplicated faces back to a single face -- `A ∪ A = A` -- and keeps RHH's
/// separate pieces, so neither document needs a special case. A SUM would have
/// been wrong on House A for exactly the reason the union is not.
fn ceiling_shape(ceiling: &Ceiling) -> Option<MultiPolygon<f64>> {
    let mut merged: Option<MultiPolygon<f64>> = None;
    for piece in &ceiling.polygons {
        let Some(outer) = piece.loops.first() else {
            continue;
        };
        if outer.points.len() < 3 {
            continue;
        }
        // Holes ARE carried here, unlike the room side. A room's hole is a
        // column or a shaft -- small, and `room_locator::outline_of` drops it so
        // a probe landing on a column still resolves. A ceiling's hole is a
        // light well or a void over an atrium and is routinely large: RHH
        // exports 173 holed polygons. Keeping them makes both the overlap and
        // the `fraction_of_ceiling` denominator measure the ceiling that is
        // actually there.
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

/// Which rooms one ceiling lies over, largest overlap first.
///
/// Same model only. CLAUDE.md's rule is that an element joining on a room id is
/// model-scoped everywhere, and spaces are the single exception because they
/// join on a project-unique *key* rather than an id. A ceiling joins on neither
/// — it joins on geometry — so the rule it follows is the conservative one until
/// a project is measured that needs otherwise. House A holds its ceilings and
/// its rooms in one model; RHH has not been probed.
fn attribute(ceiling: &Ceiling, elevation: f64, rooms: &[super::room_locator::Candidate]) -> Vec<CeilingRoom> {
    let Some(poly) = ceiling_shape(ceiling) else {
        return Vec::new(); // unmeasurable ceiling: exported, but nothing to place
    };
    let ceiling_area = poly.unsigned_area();
    if ceiling_area <= 0.0 {
        return Vec::new();
    }

    let mut out: Vec<CeilingRoom> = Vec::new();
    for room in rooms {
        if (room.elevation - elevation).abs() > super::room_locator::LEVEL_EPS_MM {
            continue;
        }
        let overlap = poly.intersection(&room.outline).unsigned_area();
        if overlap <= 0.0 {
            continue;
        }
        let room_area = room.outline.unsigned_area();
        let fraction_of_ceiling = overlap / ceiling_area;
        if overlap < MIN_OVERLAP_AREA || fraction_of_ceiling < MIN_FRACTION_OF_CEILING {
            continue;
        }
        out.push(CeilingRoom {
            room_id: room.reference.room_id.clone(),
            model_id: room.reference.model_id.clone(),
            overlap_area: overlap,
            fraction_of_ceiling,
            fraction_of_room: if room_area > 0.0 { overlap / room_area } else { 0.0 },
        });
    }
    // Largest first, so the room a ceiling mostly belongs to reads first and a
    // consumer wanting a single owner can take `rooms[0]` without inventing its
    // own rule.
    // Largest first, then by room id. **The tie-break is not decoration**: RHH
    // has ceilings lying over six rooms with byte-identical overlap areas (one
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

/// Assemble the `/ceilings` response, or `None` when no ceilings snapshot
/// exists at all — the same "nothing pushed" signal every other entity returns.
pub fn assemble_ceilings(state: &AppState, scope: &CeilingScope<'_>) -> Result<Option<CeilingsResult>, ServiceError> {
    if !state.has_any_snapshot(SnapshotKind::Ceilings).map_err(ServiceError::Internal)? {
        return Ok(None);
    }

    // Phase 1 -- scope, through the same function every entity uses.
    let scoped: Vec<(ModelKey, CeilingPayload)> =
        entity_scope::scope_snapshots(state, SnapshotKind::Ceilings, scope.project, scope.milestone, |ms| {
            &ms.ceiling_attachments
        })?;

    let revision = entity_scope::revision(&scoped);
    let phase_by_model = entity_scope::phase_by_model(&scoped);
    let levels_by_model = entity_scope::levels_by_model(&scoped);
    let placement = super::placement::from_index(state)?;

    // Phase 2 -- the room candidates, once per project.
    //
    // `SameModel` always, never the project's `room_resolution` setting: that
    // setting exists to decide whether geometry may *override an absent
    // authored reference*, and a ceiling has no authored reference to be absent.
    // Reading it here would let a project switch this entity off entirely and
    // report every ceiling as homeless, which is a wrong answer rather than a
    // disabled feature.
    let mut candidates_by_project: BTreeMap<String, entity_scope::Candidates> = BTreeMap::new();
    for (_, payload) in &scoped {
        if candidates_by_project.contains_key(&payload.project.id) {
            continue;
        }
        candidates_by_project.insert(
            payload.project.id.clone(),
            entity_scope::build_candidates(
                state,
                Some(&payload.project.id),
                scope.milestone,
                RoomResolution::SameModel,
                &scoped,
            )?,
        );
    }

    // Phase 3 -- derive.
    let mut ceilings: Vec<CeilingResponse> = Vec::new();
    for (_key, payload) in &scoped {
        let model_frame = placement.for_model(&payload.project.id, &payload.model.id);
        let candidates = candidates_by_project.get(&payload.project.id);

        for ceiling in &payload.ceilings {
            // Attribution runs in the MODEL's own frame, before placement --
            // the room candidates were built in that frame too under
            // `SameModel`, so placing the ceiling first would compare a placed
            // ceiling against unplaced rooms and miss every time.
            // The storey the ceiling is on, by the elevation its OWN model states
            // for its own level id -- a `Level.id` is per document, so the
            // elevation is what crosses. A ceiling naming a level its model does
            // not declare is attributed to nothing rather than to every room at
            // every height.
            let rooms = candidates
                .and_then(|c| {
                    let elevation = c.elevation_of(&payload.model.id, &ceiling.level_id)?;
                    Some(attribute(ceiling, elevation, c.rooms_in_model(&payload.model.id)))
                })
                .unwrap_or_default();

            let mut ceiling = ceiling.clone();
            if let Some(transform) = model_frame {
                // Every piece, not just the first: a ceiling is a list of
                // polygons now, and placing one of them would put the rest in
                // the wrong frame.
                for piece in &mut ceiling.polygons {
                    super::placement::place_loops(transform, &mut piece.loops);
                }
            }
            ceilings.push(CeilingResponse {
                ceiling,
                project_id: payload.project.id.clone(),
                model_id: payload.model.id.clone(),
                source: payload.model.source.clone(),
                rooms,
            });
        }
    }

    Ok(Some(CeilingsResult { revision, phase_by_model, levels_by_model, ceilings }))
}

/// Every ceiling attributed to one room, for the room-centric question the
/// entity was actually asked: "list ceilings by room".
///
/// A thin inversion rather than a second assembly, so both answers are derived
/// from one attribution pass and cannot disagree.
pub fn by_room(result: &CeilingsResult) -> BTreeMap<RoomRef, Vec<&CeilingResponse>> {
    let mut out: BTreeMap<RoomRef, Vec<&CeilingResponse>> = BTreeMap::new();
    for response in &result.ceilings {
        for room in &response.rooms {
            out.entry(RoomRef { model_id: room.model_id.clone(), room_id: room.room_id.clone() })
                .or_default()
                .push(response);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{CeilingPolygon, Point2D};
    use crate::service::room_locator::Candidate;

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

    /// A ceiling from its PIECES -- each piece an outer ring plus any holes.
    fn ceiling(id: &str, pieces: Vec<Vec<Loop>>) -> Ceiling {
        Ceiling {
            id: id.to_string(),
            level_id: "L1".to_string(),
            height_offset: None,
            polygons: pieces.into_iter().map(|loops| CeilingPolygon { loops }).collect(),
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
        let c = ceiling("c1", vec![vec![rect(1.0, 1.0, 11.0, 11.0)]]);
        let rooms = [room("r1", 0.0, 0.0, 12.0, 12.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].room_id, "r1");
        assert!((out[0].overlap_area - 100.0).abs() < 1e-6);
        assert!((out[0].fraction_of_ceiling - 1.0).abs() < 1e-9, "wholly inside");
        // Coverage is the OTHER question and must not be confused with it.
        assert!((out[0].fraction_of_room - 100.0 / 144.0).abs() < 1e-9);
    }

    /// The measured false positive that `MIN_FRACTION_OF_CEILING` exists for:
    /// House A's ceiling 2563038 reaching 0.97 sqft into two neighbours, 0.414%
    /// of itself. A fraction-of-ROOM test at duHast's 0.1% keeps these.
    #[test]
    fn test_sliver_of_a_large_ceiling_is_not_attributed() {
        // 234 sqft ceiling, overlapping a big room by a 1 x 1.1 ft strip:
        // above MIN_OVERLAP_AREA, below MIN_FRACTION_OF_CEILING.
        let c = ceiling("c1", vec![vec![rect(0.0, 0.0, 15.6, 15.0)]]);
        let rooms = [room("r1", 15.5, 0.0, 40.0, 1.1, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        let overlap = 0.1 * 1.1;
        assert!(overlap < MIN_OVERLAP_AREA, "this case is the FRACTION guard, not the area one");
        assert!(out.is_empty(), "a sliver is not a ceiling being in a room");
    }

    /// The other measured false positive, and the reason one threshold cannot
    /// do both jobs: House A exported two ceilings of 0.33 and 0.18 sqft, and
    /// each lies 100% inside a room. No fraction test can reject them.
    #[test]
    fn test_degenerate_ceiling_is_rejected_on_absolute_area() {
        let c = ceiling("c1", vec![vec![rect(0.0, 0.0, 0.6, 0.55)]]); // 0.33 sqft
        let rooms = [room("r1", -10.0, -10.0, 10.0, 10.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        assert!(out.is_empty(), "wholly inside, so only the area guard can reject it");
    }

    #[test]
    fn test_ceiling_spanning_two_rooms_reports_both_largest_first() {
        // 20x10 ceiling over two rooms: 6ft of it in r_small, 14ft in r_big.
        let c = ceiling("c1", vec![vec![rect(0.0, 0.0, 20.0, 10.0)]]);
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

    /// A ceiling duHast could not measure is exported with an empty polygon
    /// rather than dropped. It must attribute to nothing and must not panic.
    #[test]
    fn test_unmeasurable_ceiling_is_attributed_to_nothing() {
        let c = ceiling("c1", vec![]);
        let rooms = [room("r1", 0.0, 0.0, 10.0, 10.0, 0.0)];
        assert!(attribute(&c, 0.0, &rooms).is_empty());
    }

    /// A degenerate ring -- fewer than three points -- is the other empty shape,
    /// and `ceiling_polygon` has to reject it before `geo` sees it.
    #[test]
    fn test_two_point_loop_is_not_a_polygon() {
        let c = ceiling(
            "c1",
            vec![vec![Loop {
                points: vec![Point2D { x: 0.0, y: 0.0 }, Point2D { x: 1.0, y: 1.0 }],
            }]],
        );
        let rooms = [room("r1", 0.0, 0.0, 10.0, 10.0, 0.0)];
        assert!(attribute(&c, 0.0, &rooms).is_empty());
    }

    /// Storeys are separated by elevation, not by level id -- a `Level.id` is
    /// per document. A room directly below a ceiling must not claim it.
    #[test]
    fn test_a_room_on_another_storey_does_not_claim_the_ceiling() {
        let c = ceiling("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0)]]);
        let rooms = [room("r_below", 0.0, 0.0, 10.0, 10.0, -3000.0)];
        assert!(attribute(&c, 0.0, &rooms).is_empty(), "same plan position, different storey");
    }

    /// Equal overlaps must order deterministically, or `rooms[0]` -- documented
    /// as a usable single owner -- is a different room between two reads of the
    /// same data. RHH has a ceiling over six rooms at identical area.
    #[test]
    fn test_equal_overlaps_break_the_tie_on_room_id() {
        // One 10x10 ceiling split exactly in half by two rooms: 50 sqft each.
        let c = ceiling("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0)]]);
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
    /// a ceiling covering twice the room it actually covers.
    #[test]
    fn test_duplicate_faces_are_not_double_counted() {
        let c = ceiling("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0)], vec![rect(0.0, 0.0, 10.0, 10.0)]]);
        let rooms = [room("r1", 0.0, 0.0, 10.0, 10.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 1);
        assert!((out[0].overlap_area - 100.0).abs() < 1e-6, "the duplicate face must not double the area");
        assert!((out[0].fraction_of_ceiling - 1.0).abs() < 1e-9, "and the ceiling is 100 sqft, not 200");
    }

    /// **RHH's shape, and the reason this field is a list.** Two disjoint pieces
    /// are one ceiling. Taking the first would lose the second entirely -- 44%
    /// of ceiling area on average across RHH's 48 multi-polygon ceilings, 99.2%
    /// at worst -- and taking the largest would still lose the smaller piece.
    #[test]
    fn test_disjoint_pieces_are_one_ceiling() {
        // 100 sqft at the origin and 25 sqft well away from it.
        let c = ceiling("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0)], vec![rect(50.0, 0.0, 55.0, 5.0)]]);
        let rooms = [
            room("r_big", -1.0, -1.0, 11.0, 11.0, 0.0),
            room("r_far", 49.0, -1.0, 56.0, 6.0, 0.0),
        ];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 2, "both pieces attribute, so the ceiling is in both rooms");
        assert_eq!(out[0].room_id, "r_big", "largest overlap first");
        assert!((out[0].overlap_area - 100.0).abs() < 1e-6);
        assert!((out[1].overlap_area - 25.0).abs() < 1e-6);
        // The denominator is the WHOLE ceiling, both pieces: 125 sqft.
        assert!((out[0].fraction_of_ceiling - 100.0 / 125.0).abs() < 1e-9);
    }

    /// A ceiling's hole is a light well or a void over an atrium, and RHH
    /// exports 173 holed polygons. It must come out of the ceiling's own area,
    /// or a ceiling with a large void reports covering ground it does not.
    #[test]
    fn test_a_hole_is_subtracted_from_the_ceiling() {
        // 10x10 outer with a 4x4 void punched out of the middle: 100 - 16 = 84.
        let c = ceiling("c1", vec![vec![rect(0.0, 0.0, 10.0, 10.0), rect(3.0, 3.0, 7.0, 7.0)]]);
        let rooms = [room("r1", 0.0, 0.0, 10.0, 10.0, 0.0)];
        let out = attribute(&c, 0.0, &rooms);
        assert_eq!(out.len(), 1);
        assert!((out[0].overlap_area - 84.0).abs() < 1e-6, "the void is not ceiling");
        assert!(
            (out[0].fraction_of_ceiling - 1.0).abs() < 1e-9,
            "all of the ceiling that exists is in the room"
        );
    }
}
