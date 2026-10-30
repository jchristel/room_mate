//! Solving one blueprint against many rooms, for the assign-many view.
//!
//! **The resolver, asked once per room, summarised.** Nothing here places an item or has a rule
//! of its own: each room goes through `resolve_with_doors` exactly as the single-room read does,
//! with `EdgeMap::Auto`, and what comes back is boiled down to a verdict a person can scan a list
//! by. The rooms and the doors are read ONCE for the whole batch, which is the only reason this
//! is not a loop over `resolve_in_room` (each of those assembles the project's doors).
//!
//! **A verdict is advice, never a gate on what a person may assign.** `clear` is a fit by shape
//! with nothing the room changes; `check` placed items but has findings (or was mapped by hand);
//! `cannot` placed nothing. The numbers behind it ride the row so the view can say why.

use serde::Serialize;

use super::blueprint_capture::{loop_mm, project_doors, SourceDoor};
use super::blueprint_geometry::Pt;
use super::blueprint_model::RevisionContent;
use super::blueprint_resolve::{resolve_with_doors, EdgeMap, Finding, FitKind};
use super::room_locator::RoomRef;
use super::rooms::{assemble_rooms, RoomScope};
use super::ServiceError;
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Fits by shape and the room changes nothing about the layout.
    Clear,
    /// Items were placed, and there is something to look at.
    Check,
    /// Nothing was placed: a shape that does not fit, a different edge count, or a refusal.
    Cannot,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoomSolve {
    pub room_id: String,
    /// The room's model, once found. Absent when the id names no room, or names several models.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub room_name: String,
    pub verdict: Verdict,
    /// Why the verdict is what it is, in a sentence.
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<FitKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shift: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    /// Mean mm between the blueprint's doors and the room's at the shift used; absent with no doors
    /// on either side (no evidence).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub door_gap_mm: Option<f64>,
    pub placed: usize,
    /// Items the blueprint places (excluded ones not counted).
    pub items: usize,
    pub outside: usize,
    pub overlap: usize,
    pub doors_disagree: bool,
}

fn refused(room_id: &str, model_id: Option<String>, room_name: String, reason: String) -> RoomSolve {
    RoomSolve {
        room_id: room_id.to_string(),
        model_id,
        room_name,
        verdict: Verdict::Cannot,
        reason,
        fit: None,
        shift: None,
        cost: None,
        door_gap_mm: None,
        placed: 0,
        items: 0,
        outside: 0,
        overlap: 0,
        doors_disagree: false,
    }
}

fn judge(content: &RevisionContent, ring: &[Pt], doors: &[SourceDoor], mut row: RoomSolve) -> RoomSolve {
    row.items = content.items.iter().filter(|i| !i.excluded).count();
    let r = match resolve_with_doors(content, ring, &EdgeMap::Auto, doors) {
        Ok(r) => r,
        Err(ServiceError::Invalid(msg)) => {
            row.reason = msg;
            return row;
        }
        Err(e) => {
            row.reason = format!("{e}");
            return row;
        }
    };
    row.fit = Some(r.fit.kind);
    row.shift = Some(r.fit.shift);
    row.cost = r.fit.cost;
    row.door_gap_mm = r.candidates.iter().find(|c| c.shift == r.fit.shift).and_then(|c| c.door_gap_mm);
    row.placed = r.placements.len();
    row.outside = r.findings.iter().filter(|f| matches!(f, Finding::Outside { .. })).count();
    row.overlap = r.findings.iter().filter(|f| matches!(f, Finding::Overlap { .. })).count();
    row.doors_disagree = r.findings.iter().any(|f| matches!(f, Finding::DoorsDisagree { .. }));
    let (verdict, reason) = match r.fit.kind {
        FitKind::EdgeCountDiffers => (Verdict::Cannot, "a different number of edges: map them by hand".to_string()),
        FitKind::NoFit => (Verdict::Cannot, "the room is not this shape".to_string()),
        FitKind::MirrorImage => (Verdict::Cannot, "the room is the other hand of this layout".to_string()),
        _ if r.placements.is_empty() && row.items > 0 => (Verdict::Cannot, "nothing was placed".to_string()),
        _ if r.findings.is_empty() => (Verdict::Clear, "fits, nothing the room changes".to_string()),
        _ => (Verdict::Check, format!("{} thing(s) to look at", r.findings.len())),
    };
    row.verdict = verdict;
    row.reason = reason;
    row
}

/// Solve `content` against each of `room_ids` as the project stands, in the order asked. A room
/// that cannot be solved is a row saying so and never fails the batch.
pub fn solve_rooms(
    state: &AppState,
    project: &str,
    content: &RevisionContent,
    room_ids: &[String],
) -> Result<Vec<RoomSolve>, ServiceError> {
    let Some(rooms) = assemble_rooms(state, &RoomScope { project: Some(project), ..Default::default() })? else {
        return Ok(room_ids
            .iter()
            .map(|id| refused(id, None, String::new(), "the project has no rooms".to_string()))
            .collect());
    };
    let doors = project_doors(state, project)?;
    Ok(room_ids
        .iter()
        .map(|id| {
            let mut found = rooms.rooms.iter().filter(|r| r.room.id == *id);
            let (Some(first), second) = (found.next(), found.next()) else {
                return refused(id, None, String::new(), "no such room in this project".to_string());
            };
            let name = first.room.name.clone();
            if second.is_some() {
                return refused(id, None, name, "the id exists in more than one model".to_string());
            }
            let model = first.model_id.clone();
            let Some(outer) = first.room.loops.first() else {
                return refused(id, Some(model), name, "the room has no outline".to_string());
            };
            let ring: Vec<Pt> = loop_mm(outer);
            let room = RoomRef { model_id: model.clone(), room_id: id.clone() };
            let mine: Vec<SourceDoor> = doors.iter().filter(|d| d.opens_onto(&room)).map(|d| d.door.clone()).collect();
            let row = refused(id, Some(model), name, String::new());
            judge(content, &ring, &mine, row)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::blueprint_capture::capture;

    fn rect(w: f64, h: f64) -> Vec<Pt> {
        vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    }

    fn row() -> RoomSolve {
        refused("r", Some("m".to_string()), "R".to_string(), String::new())
    }

    #[test]
    fn test_the_blueprints_own_shape_is_clear_and_another_edge_count_cannot() {
        let content = capture(&rect(4000.0, 3000.0), vec![]).unwrap().content;
        let same = judge(&content, &rect(4000.0, 3000.0), &[], row());
        assert_eq!((same.verdict, same.fit), (Verdict::Clear, Some(FitKind::Plain)));
        let l = vec![
            (0.0, 0.0),
            (4000.0, 0.0),
            (4000.0, 1500.0),
            (2000.0, 1500.0),
            (2000.0, 3000.0),
            (0.0, 3000.0),
        ];
        let other = judge(&content, &l, &[], row());
        assert_eq!((other.verdict, other.fit), (Verdict::Cannot, Some(FitKind::EdgeCountDiffers)));
    }
}
