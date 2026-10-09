//! **Stack candidates**: for one room, the rooms on the storey above and the
//! storey below that sit over or under it, ranked, for a person to confirm as a
//! vertical link. A suggestion, never a write -- like everything derived here.
//!
//! ## Why plan overlap, and what was measured
//!
//! Measured on RHH (2026-10-10) over its 104 lift and 63 stair rooms: with the next
//! storey chosen sensibly, the room whose name shares the source's stem was the top
//! overlap in 146 of 158 lift hops and 70 of 75 stair hops, and a stem-mate was
//! never missed when one existed. Lifts overlap almost entirely or not at all, so
//! the threshold hardly matters for them; a stair overlaps by the shaft it shares
//! and misses by its landings, which is why the score is relative to the SMALLER
//! room and why 0.9 loses real stairs where 0.7 does not.
//!
//! ## The next storey is the next one with something over the room
//!
//! **Not the next level in the project's list.** RHH's car park has half-levels
//! (`C 0.5`, `C 1.5`, ...) stacked between hospital floors, so the level above a
//! hospital lift room, in elevation order, is a car-park level with nothing over it:
//! only 51 of 198 lift hops found any overlap that way, against 158 once the
//! neighbour was "the next storey above that has an overlapping room". That also
//! needs no notion of a building, which a project need not configure. The cost is
//! that a shaft which stops can find something unrelated further up, so each side
//! says how many storeys it passed (`levels_passed`) for the reader to judge, as
//! `levels_between` does for a link already drawn.
//!
//! ## What is ranked, and what is not decided here
//!
//! A candidate is a room whose overlap is at least [`MIN_CANDIDATE_OVERLAP`] of the
//! smaller of the two rooms. They are ordered by whether the name shares the
//! source's **stem** (the name without its last word: `LIFT 3 TRA003` and
//! `LIFT 3 TRA032` share `LIFT 3`; `STAIR PRES ENG501` does not share `STAIR 3`),
//! then by overlap. The stem is a boost and never a requirement, so a stair whose
//! landing rooms are named differently is still offered. `confidence` says whether
//! the top candidate stands alone (`clear`) or the reader has to choose
//! (`ambiguous`): RHH's lift roofs, where several lifts share one plant room, and a
//! stair beside a pressurised stairwell, are the cases that measured ambiguous.
//! Nothing is attributed when no room overlaps: that stair is the manual one.

use geo::{Area, BooleanOps, BoundingRect, Euclidean, Intersects, Length, MultiPolygon, Polygon};
use serde::Serialize;

use crate::state::AppState;

use super::room_locator::{outline_of, LEVEL_EPS_MM};
use super::rooms::{assemble_rooms, RoomScope};
use super::ServiceError;

/// An overlap counts as a candidate from this fraction of the smaller room.
pub const MIN_CANDIDATE_OVERLAP: f64 = 0.3;

/// At or above this a single candidate is offered without a choice.
pub const CLEAR_OVERLAP: f64 = 0.7;

/// Narrower than this (mean width, feet) an overlap is rounding along a shared
/// wall, not one room over another. The same 10 mm `surface_attribution` uses.
const MIN_OVERLAP_MEAN_WIDTH_FT: f64 = 10.0 / 304.8;

#[derive(Debug, Clone, Serialize)]
pub struct StackRoom {
    pub model_id: String,
    pub room_id: String,
    pub name: String,
    pub level_id: String,
    pub level_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StackCandidate {
    pub model_id: String,
    pub room_id: String,
    pub name: String,
    /// Overlap area as a fraction of the SMALLER of the two rooms.
    pub overlap: f64,
    /// Overlap as a fraction of the source room.
    pub fraction_of_room: f64,
    /// The name shares the source's stem.
    pub same_stem: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// One candidate stands alone, so it can be offered as the answer.
    Clear,
    /// Several fit; the reader chooses.
    Ambiguous,
}

#[derive(Debug, Clone, Serialize)]
pub struct StackSide {
    pub level_id: String,
    pub level_name: String,
    pub elevation: f64,
    /// Storeys with rooms that lie between the source and this one and had nothing
    /// over or under it. Above 0 means the shaft may stop short of here.
    pub levels_passed: usize,
    pub confidence: Confidence,
    pub candidates: Vec<StackCandidate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StackResult {
    pub revision: String,
    pub room: StackRoom,
    /// `None` when no storey above has a room over this one (the top of a shaft).
    pub up: Option<StackSide>,
    pub down: Option<StackSide>,
}

/// The name without its last word, lower-cased, or empty when it has one word.
fn stem_of(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.len() < 2 {
        return String::new();
    }
    words[..words.len() - 1].join(" ").to_lowercase()
}

fn mean_width(shape: &MultiPolygon<f64>) -> f64 {
    let perimeter: f64 = shape
        .iter()
        .map(|p| Euclidean.length(p.exterior()) + p.interiors().iter().map(|r| Euclidean.length(r)).sum::<f64>())
        .sum();
    if perimeter <= 0.0 {
        0.0
    } else {
        2.0 * shape.unsigned_area() / perimeter
    }
}

struct Placed<'a> {
    model_id: &'a str,
    room: &'a crate::contract::Room,
    outline: Polygon<f64>,
    area: f64,
}

/// The candidates for `source` among the rooms of one storey.
fn on_storey(source: &Placed<'_>, rooms: &[&Placed<'_>]) -> Vec<StackCandidate> {
    let Some(frame) = source.outline.bounding_rect() else {
        return Vec::new();
    };
    let stem = stem_of(&source.room.name);
    let mut out: Vec<StackCandidate> = rooms
        .iter()
        .filter_map(|other| {
            let other_frame = other.outline.bounding_rect()?;
            if !other_frame.intersects(&frame) {
                return None;
            }
            let shared = source.outline.intersection(&other.outline);
            let area = shared.unsigned_area();
            let smaller = source.area.min(other.area);
            if area <= 0.0 || smaller <= 0.0 || mean_width(&shared) < MIN_OVERLAP_MEAN_WIDTH_FT {
                return None;
            }
            let overlap = area / smaller;
            (overlap >= MIN_CANDIDATE_OVERLAP).then(|| StackCandidate {
                model_id: other.model_id.to_string(),
                room_id: other.room.id.clone(),
                name: other.room.name.clone(),
                overlap,
                fraction_of_room: area / source.area,
                same_stem: !stem.is_empty() && stem_of(&other.room.name) == stem,
            })
        })
        .collect();
    out.sort_by(|a, b| b.same_stem.cmp(&a.same_stem).then(b.overlap.total_cmp(&a.overlap)));
    out
}

/// One candidate stands alone when it is the only strong overlap, or the only
/// strong overlap sharing the stem.
fn confidence_of(candidates: &[StackCandidate]) -> Confidence {
    let strong = |c: &&StackCandidate| c.overlap >= CLEAR_OVERLAP;
    let strong_count = candidates.iter().filter(strong).count();
    let strong_stem = candidates.iter().filter(strong).filter(|c| c.same_stem).count();
    match (strong_count, strong_stem) {
        (1, _) | (_, 1) => Confidence::Clear,
        _ => Confidence::Ambiguous,
    }
}

/// The candidates above and below one room.
///
/// `Ok(None)` when no rooms have ever been pushed (the adapter's 204). The room is
/// named by id and, when two models share the id, by model too; a bare id two
/// models share is refused with the candidates listed, never guessed.
pub fn stack_candidates(
    state: &AppState,
    project: &str,
    room_id: &str,
    model_id: Option<&str>,
    milestone: Option<&str>,
) -> Result<Option<StackResult>, ServiceError> {
    let scope = RoomScope { project: Some(project), milestone, ..Default::default() };
    let Some(assembled) = assemble_rooms(state, &scope)? else {
        return Ok(None);
    };

    let placed: Vec<Placed<'_>> = assembled
        .rooms
        .iter()
        .filter_map(|r| {
            let outline = outline_of(&r.room)?;
            let area = outline.unsigned_area();
            Some(Placed { model_id: &r.model_id, room: &r.room, outline, area })
        })
        .collect();

    let hits: Vec<&Placed<'_>> = placed
        .iter()
        .filter(|p| p.room.id == room_id && model_id.is_none_or(|m| m == p.model_id))
        .collect();
    let source = match hits.as_slice() {
        [one] => *one,
        [] => {
            return Err(ServiceError::Invalid(format!(
                "room {room_id:?}{} is not among the rooms in scope, or has no outline",
                model_id.map(|m| format!(" in model {m:?}")).unwrap_or_default()
            )))
        }
        many => {
            return Err(ServiceError::Invalid(format!(
                "room id {room_id:?} exists in {} models ({}); name the model",
                many.len(),
                many.iter().map(|p| p.model_id).collect::<Vec<_>>().join(", ")
            )))
        }
    };

    // Storeys that hold rooms, by elevation; a level with none is not a storey.
    let elevation_of = |id: &str| assembled.levels.iter().find(|l| l.id == id).map(|l| l.elevation);
    let name_of = |id: &str| assembled.levels.iter().find(|l| l.id == id).map(|l| l.name.clone()).unwrap_or_default();
    let mut storeys: Vec<(String, f64)> = Vec::new();
    for p in &placed {
        let id = &p.room.level_id;
        if let (Some(e), false) = (elevation_of(id), storeys.iter().any(|s| s.0 == *id)) {
            storeys.push((id.clone(), e));
        }
    }
    storeys.sort_by(|a, b| a.1.total_cmp(&b.1));
    let here = elevation_of(&source.room.level_id).unwrap_or(0.0);

    let side = |up: bool| -> Option<StackSide> {
        let mut order: Vec<&(String, f64)> = storeys
            .iter()
            .filter(|s| if up { s.1 > here + LEVEL_EPS_MM } else { s.1 < here - LEVEL_EPS_MM })
            .collect();
        if !up {
            order.reverse();
        }
        for (passed, (level_id, elevation)) in order.into_iter().enumerate() {
            let there: Vec<&Placed<'_>> = placed.iter().filter(|p| p.room.level_id == *level_id).collect();
            let candidates = on_storey(source, &there);
            if !candidates.is_empty() {
                return Some(StackSide {
                    level_id: level_id.clone(),
                    level_name: name_of(level_id),
                    elevation: *elevation,
                    levels_passed: passed,
                    confidence: confidence_of(&candidates),
                    candidates,
                });
            }
        }
        None
    };

    Ok(Some(StackResult {
        revision: assembled.revision.clone(),
        room: StackRoom {
            model_id: source.model_id.to_string(),
            room_id: source.room.id.clone(),
            name: source.room.name.clone(),
            level_id: source.room.level_id.clone(),
            level_name: name_of(&source.room.level_id),
        },
        up: side(true),
        down: side(false),
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use super::*;
    use crate::contract::{Level, Loop, Model, Point2D, Project, Room, RoomPayload, Snapshot, SUPPORTED_SCHEMA};
    use crate::state::ProjectSettings;
    use crate::storage::MemStore;

    fn bundle() -> ProjectSettings {
        ProjectSettings {
            spaces: Default::default(),
            reference: BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            anchor_model: None,
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            ffe: Default::default(),
            hierarchy_exclusions: vec![],
        }
    }

    /// A room `name` on `level`, spanning `x0..x1` by `0..10`.
    fn room(id: &str, name: &str, level: &str, x0: f64, x1: f64) -> Room {
        Room {
            enclosure: None,
            id: id.to_string(),
            name: name.to_string(),
            level_id: level.to_string(),
            loops: vec![Loop {
                points: vec![
                    Point2D { x: x0, y: 0.0 },
                    Point2D { x: x1, y: 0.0 },
                    Point2D { x: x1, y: 10.0 },
                    Point2D { x: x0, y: 10.0 },
                ],
            }],
            properties: BTreeMap::new(),
        }
    }

    /// Levels 3 m apart, with `lvl_empty` between `lvl1` and `lvl2` holding no rooms
    /// over the lift (it is the car park's half level).
    fn state(rooms: Vec<Room>) -> AppState {
        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle())]), None);
        state
            .set_snapshot(RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".into(), name: "P".into() },
                model: Model { id: "m1".into(), name: "M".into(), source: "revit".into() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                phase: Some("New Construction".into()),
                model_to_shared: None,
                room_boundary: Some(crate::contract::RoomBoundary::Centreline),
                levels: vec![
                    Level { id: "lvl1".into(), name: "Level 1".into(), elevation: 0.0 },
                    Level { id: "half".into(), name: "Half".into(), elevation: 1500.0 },
                    Level { id: "lvl2".into(), name: "Level 2".into(), elevation: 3000.0 },
                    Level { id: "lvl3".into(), name: "Level 3".into(), elevation: 6000.0 },
                ],
                rooms,
            })
            .unwrap();
        state
    }

    fn read(s: &AppState, id: &str) -> StackResult {
        stack_candidates(s, "p1", id, None, None).unwrap().unwrap()
    }

    #[test]
    fn test_the_stem_is_the_name_without_its_last_word() {
        assert_eq!(stem_of("LIFT 3 TRA003"), "lift 3");
        assert_eq!(stem_of("STAIR PRES ENG501"), "stair pres");
        assert_eq!(stem_of("LIFT"), "");
    }

    /// The case that made "next level in the list" wrong: a storey with rooms but
    /// nothing over the lift sits between, and the neighbour is the next storey
    /// that HAS something over it, with the storey passed counted.
    #[test]
    fn test_the_neighbour_is_the_next_storey_with_something_over_the_room() {
        let s = state(vec![
            room("l1", "LIFT 3 TRA001", "lvl1", 0.0, 10.0),
            room("elsewhere", "STORE", "half", 50.0, 60.0),
            room("l2", "LIFT 3 TRA002", "lvl2", 0.0, 10.0),
        ]);
        let up = read(&s, "l1").up.unwrap();
        assert_eq!(up.level_id, "lvl2");
        assert_eq!(up.levels_passed, 1, "the half level was passed over");
        assert_eq!(up.candidates[0].room_id, "l2");
        assert_eq!(up.confidence, Confidence::Clear);
        assert!(read(&s, "l1").down.is_none(), "nothing below the lowest storey");
    }

    /// The stem is a boost: the real stack-mate outranks a plant room that
    /// overlaps just as much, and with both strong the reader is not told it is
    /// clear unless the stem settles it.
    #[test]
    fn test_a_stem_mate_outranks_an_equal_overlap() {
        let s = state(vec![
            room("l1", "LIFT 3 TRA001", "lvl1", 0.0, 10.0),
            room("plant", "LIFT PLANT ENG912", "lvl2", 0.0, 10.0),
            room("l2", "LIFT 3 TRA002", "lvl2", 0.0, 10.0),
        ]);
        let up = read(&s, "l1").up.unwrap();
        assert_eq!(up.candidates.iter().map(|c| c.room_id.as_str()).collect::<Vec<_>>(), vec!["l2", "plant"]);
        assert!(up.candidates[0].same_stem && !up.candidates[1].same_stem);
        assert_eq!(up.confidence, Confidence::Clear, "only one strong overlap shares the stem");
    }

    /// Two equally good rooms and no stem to tell them apart is a choice, said so.
    #[test]
    fn test_two_equal_candidates_with_no_stem_are_ambiguous() {
        let s = state(vec![
            room("a", "SHAFT 1 AAA", "lvl1", 0.0, 10.0),
            room("b", "ROOM BBB", "lvl2", 0.0, 10.0),
            room("c", "OTHER CCC", "lvl2", 0.0, 10.0),
        ]);
        assert_eq!(read(&s, "a").up.unwrap().confidence, Confidence::Ambiguous);
    }

    /// A stair is measured against the SMALLER room, so a landing that only
    /// partly overlaps still scores, and a thin strip along a wall does not.
    #[test]
    fn test_overlap_is_relative_to_the_smaller_room_and_a_strip_is_not_one() {
        let s = state(vec![
            room("small", "STAIR 1 AAA", "lvl1", 0.0, 4.0),
            room("big", "HALL BBB", "lvl2", 0.0, 40.0),
            room("beside", "NEXT CCC", "lvl2", 4.0, 14.0),
        ]);
        let up = read(&s, "small").up.unwrap();
        let ids: Vec<&str> = up.candidates.iter().map(|c| c.room_id.as_str()).collect();
        assert_eq!(ids, vec!["big"], "the room that only touches along an edge is no candidate");
        assert!((up.candidates[0].overlap - 1.0).abs() < 1e-9);
        assert!((up.candidates[0].fraction_of_room - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_a_room_with_nothing_over_or_under_it_has_no_sides() {
        let s = state(vec![
            room("a", "STAIR 1 AAA", "lvl1", 0.0, 10.0),
            room("far", "FAR", "lvl2", 100.0, 110.0),
        ]);
        let r = read(&s, "a");
        assert!(r.up.is_none() && r.down.is_none());
    }

    #[test]
    fn test_an_unknown_room_is_refused_not_guessed() {
        let s = state(vec![room("a", "A BBB", "lvl1", 0.0, 10.0)]);
        assert!(stack_candidates(&s, "p1", "nope", None, None).is_err());
    }
}
