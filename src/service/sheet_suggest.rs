//! Which library sheet is closest to a room: the read behind "sheet creation mode".
//!
//! **Three figures, weighed equally** (decided 2026-10-19): the room's area, its
//! proportions and the number of FF&E items in it -- the three duHast's `RoomLayoutSheet`
//! sketch named. Each is compared as a **ratio** (the absolute value of its natural log), so
//! a room twice the area is as far from a sheet's room as one half the area, and 5 items
//! against 10 is not the same distance as 105 against 110. The distance is the plain mean
//! of the three, and the figures ride the answer so a person sees *why* a sheet was offered.
//!
//! **A sheet is ranked on the summary of its room stored at push** (`RoomSummary`, plan decision
//! 25), so it still ranks when that room has since been redrawn or is not on the server. A sheet
//! with no stored summary (pushed before it existed) is read from its room as it is today.
//! The target room is always read as it is today.
//!
//! **A suggestion is offered, never applied.** The ranking is the whole product: it chooses
//! nothing, and it is only asked for in sheet creation mode. A sheet with no room cannot be
//! ranked, and a sheet whose room is no longer in the model is reported with the reason --
//! "signal, not error".
//!
//! *Proportions* is the long side over the short side of the room's **smallest enclosing
//! rectangle** (tried along each edge direction), so an L or a rotated room is read by the
//! rectangle it fits, not by an axis-aligned box that flatters or punishes its angle.

use serde::Serialize;

use crate::sheets::{RoomSummary, SheetRecord, SheetRoom};
use crate::state::AppState;

use super::blueprint_capture::loop_mm;
use super::blueprint_geometry::{coarse_outline, dot, edges, signed_area, Pt};
use super::items::{assemble_items, ItemScope};
use super::room_locator::RoomRef;
use super::rooms::{assemble_rooms, RoomScope};
use super::ServiceError;

/// The three figures of a room.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Signature {
    /// Square metres, from the room's outer loop.
    pub area_m2: f64,
    /// Long side over short side of the smallest enclosing rectangle; 1 for a square.
    pub proportion: f64,
    /// FF&E items attributed to the room.
    pub items: usize,
}

/// How far each figure is from the target's, as the absolute natural log of the ratio.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Parts {
    pub area: f64,
    pub proportion: f64,
    pub items: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    pub sheet_id: String,
    pub sheet_number: String,
    pub sheet_name: String,
    pub document_title: String,
    /// The room the sheet is linked to, if any: a sheet pushed from the Sheets window has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room: Option<SheetRoom>,
    /// What the sheet covers, for a person to read: the rooms said at export, else its linked room's name.
    pub room_name: String,
    pub signature: Signature,
    pub parts: Parts,
    /// The mean of `parts`: smaller is closer.
    pub distance: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Unresolved {
    pub sheet_id: String,
    pub sheet_number: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Target {
    pub model_id: String,
    pub room_id: String,
    pub room_name: String,
    pub signature: Signature,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggestions {
    pub target: Target,
    /// Closest first.
    pub suggestions: Vec<Suggestion>,
    /// Sheets with no figures (no room figures were sent and no room is linked), which cannot be ranked.
    pub unassigned: usize,
    /// Sheets whose room is not in the model today.
    pub unresolved: Vec<Unresolved>,
}

const M2_PER_MM2: f64 = 1.0e-6;

/// The long side over the short side of the smallest rectangle (over the outline's own edge
/// directions) that encloses the outline.
pub fn proportion_of(ring: &[Pt]) -> f64 {
    enclosing_rectangle(ring).map_or(1.0, |(long, short)| long / short)
}

/// The long and short sides, mm, of the smallest rectangle (over the outline's own edge
/// directions) that encloses the outline. `None` for an outline with no area.
pub fn enclosing_rectangle(ring: &[Pt]) -> Option<(f64, f64)> {
    let ring = coarse_outline(ring);
    if ring.len() < 3 {
        return None;
    }
    let mut best: Option<(f64, f64, f64)> = None; // (area, long, short)
    for e in edges(&ring) {
        let n = (-e.t.1, e.t.0);
        let (mut lo_t, mut hi_t, mut lo_n, mut hi_n) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for &p in &ring {
            let (a, b) = (dot(p, e.t), dot(p, n));
            lo_t = lo_t.min(a);
            hi_t = hi_t.max(a);
            lo_n = lo_n.min(b);
            hi_n = hi_n.max(b);
        }
        let (w, h) = (hi_t - lo_t, hi_n - lo_n);
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let area = w * h;
        if best.is_none_or(|(a, _, _)| area < a) {
            best = Some((area, w.max(h), w.min(h)));
        }
    }
    best.map(|(_, long, short)| (long, short))
}

impl From<RoomSummary> for Signature {
    fn from(s: RoomSummary) -> Self {
        Signature {
            area_m2: s.area_m2,
            proportion: if s.y_mm > 0.0 { s.x_mm / s.y_mm } else { 1.0 },
            items: s.items,
        }
    }
}

/// What a library sheet stores about its room (decision 25), worked out now from the rooms and
/// FF&E the project holds. `Ok(None)` when the room is not among them.
pub fn room_summary(state: &AppState, project: &str, room: &RoomRef) -> Result<Option<RoomSummary>, ServiceError> {
    let Some(rooms) = assemble_rooms(state, &RoomScope { project: Some(project), ..Default::default() })? else {
        return Ok(None);
    };
    let Some(found) = rooms.rooms.iter().find(|r| r.model_id == room.model_id && r.room.id == room.room_id) else {
        return Ok(None);
    };
    let Some(outer) = found.room.loops.first() else {
        return Ok(None);
    };
    let ring: Vec<Pt> = loop_mm(outer);
    let Some((x_mm, y_mm)) = enclosing_rectangle(&ring) else {
        return Ok(None);
    };
    let mut items = 0;
    if let Some(ffe) = assemble_items(state, &ItemScope { project: Some(project), ..Default::default() })? {
        items = ffe.ffe.iter().filter(|it| it.owner_rooms_qualified.contains(room)).count();
    }
    Ok(Some(RoomSummary { x_mm, y_mm, area_m2: signed_area(&ring).abs() * M2_PER_MM2, items }))
}

fn ratio_distance(a: f64, b: f64) -> f64 {
    if a <= 0.0 || b <= 0.0 {
        return f64::INFINITY;
    }
    (a / b).ln().abs()
}

/// How far `other` is from `target`, figure by figure and as their mean.
pub fn distance(target: &Signature, other: &Signature) -> (Parts, f64) {
    let parts = Parts {
        area: ratio_distance(target.area_m2, other.area_m2),
        proportion: ratio_distance(target.proportion, other.proportion),
        // +1 so an empty room is a number, and 0 against 1 item is not infinitely far.
        items: ratio_distance(target.items as f64 + 1.0, other.items as f64 + 1.0),
    };
    (parts, (parts.area + parts.proportion + parts.items) / 3.0)
}

/// Rank `candidates` (a sheet, with its room's name and signature) against `target`.
/// Pure, so the ranking is testable without a model.
pub fn rank(
    target: &Signature,
    candidates: Vec<(&SheetRecord, Option<SheetRoom>, String, Signature)>,
) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = candidates
        .into_iter()
        .map(|(sheet, room, room_name, signature)| {
            let (parts, distance) = distance(target, &signature);
            Suggestion {
                sheet_id: sheet.id.clone(),
                sheet_number: sheet.sheet_number.clone(),
                sheet_name: sheet.sheet_name.clone(),
                document_title: sheet.document_title.clone(),
                room,
                room_name,
                signature,
                parts,
                distance,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        a.distance
            .total_cmp(&b.distance)
            .then_with(|| a.sheet_number.cmp(&b.sheet_number))
            .then_with(|| a.sheet_id.cmp(&b.sheet_id))
    });
    out
}

/// Rank the project's library sheets against one room. `Ok(None)` when the project has no
/// rooms, or the room is not among them.
pub fn suggest(
    state: &AppState,
    project: &str,
    target: &RoomRef,
    sheets: &[SheetRecord],
) -> Result<Option<Suggestions>, ServiceError> {
    let Some(rooms) = assemble_rooms(state, &RoomScope { project: Some(project), ..Default::default() })? else {
        return Ok(None);
    };
    let mut item_counts: std::collections::BTreeMap<RoomRef, usize> = std::collections::BTreeMap::new();
    if let Some(ffe) = assemble_items(state, &ItemScope { project: Some(project), ..Default::default() })? {
        for it in &ffe.ffe {
            for r in &it.owner_rooms_qualified {
                *item_counts.entry(r.clone()).or_default() += 1;
            }
        }
    }
    let signature_of = |room: &RoomRef| -> Option<(String, Signature)> {
        let found = rooms.rooms.iter().find(|r| r.model_id == room.model_id && r.room.id == room.room_id)?;
        let outer = found.room.loops.first()?;
        let ring: Vec<Pt> = loop_mm(outer);
        if ring.len() < 3 {
            return None;
        }
        Some((
            found.room.name.clone(),
            Signature {
                area_m2: signed_area(&ring).abs() * M2_PER_MM2,
                proportion: proportion_of(&ring),
                items: item_counts.get(room).copied().unwrap_or(0),
            },
        ))
    };
    let Some((room_name, signature)) = signature_of(target) else {
        return Ok(None);
    };

    let (mut candidates, mut unresolved, mut unassigned) = (Vec::new(), Vec::new(), 0);
    for sheet in sheets {
        // The figures stored with the sheet win: they are what it was made for, they are all a
        // sheet pushed without a linked room has, and they still rank when a room has moved on.
        if let Some(summary) = sheet.room_summary {
            let name = if !sheet.rooms.is_empty() {
                sheet.rooms.join("; ")
            } else {
                sheet
                    .room
                    .as_ref()
                    .and_then(|r| signature_of(&RoomRef { model_id: r.model_id.clone(), room_id: r.room_id.clone() }))
                    .map_or_else(String::new, |(name, _)| name)
            };
            candidates.push((sheet, sheet.room.clone(), name, Signature::from(summary)));
            continue;
        }
        // No figures: only a linked room can still be read as it is today.
        let Some(sheet_room) = &sheet.room else {
            unassigned += 1;
            continue;
        };
        let room_ref = RoomRef { model_id: sheet_room.model_id.clone(), room_id: sheet_room.room_id.clone() };
        match signature_of(&room_ref) {
            Some((name, sig)) => candidates.push((sheet, Some(sheet_room.clone()), name, sig)),
            None => unresolved.push(Unresolved {
                sheet_id: sheet.id.clone(),
                sheet_number: sheet.sheet_number.clone(),
                reason: format!(
                    "room {:?} of model {:?} is not in the model today",
                    sheet_room.room_id, sheet_room.model_id
                ),
            }),
        }
    }
    Ok(Some(Suggestions {
        target: Target {
            model_id: target.model_id.clone(),
            room_id: target.room_id.clone(),
            room_name,
            signature,
        },
        suggestions: rank(&signature, candidates),
        unassigned,
        unresolved,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rect(w: f64, h: f64) -> Vec<Pt> {
        vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    }

    fn sig(area: f64, proportion: f64, items: usize) -> Signature {
        Signature { area_m2: area, proportion, items }
    }

    fn record(id: &str, number: &str) -> SheetRecord {
        SheetRecord {
            schema_version: 1,
            id: id.into(),
            taken_at: String::new(),
            revit_sheet_id: id.into(),
            document_title: "D".into(),
            sheet_number: number.into(),
            sheet_name: String::new(),
            room: None,
            blueprint: None,
            phase: None,
            room_summary: None,
            size_mm: None,
            rooms: Vec::new(),
            received_at: String::new(),
            sheet: json!({}),
            pending: vec![],
            rejected: vec![],
            last_pending_id: 0,
        }
    }

    #[test]
    fn test_proportion_is_the_long_side_over_the_short_side() {
        assert!((proportion_of(&rect(4000.0, 2000.0)) - 2.0).abs() < 1e-9);
        assert!((proportion_of(&rect(3000.0, 3000.0)) - 1.0).abs() < 1e-9);
        // The same room turned 40 degrees reads the same: the rectangle is the room's own.
        let (s, k) = (40f64.to_radians().sin(), 40f64.to_radians().cos());
        let turned: Vec<Pt> =
            rect(4000.0, 2000.0).into_iter().map(|p| (p.0 * k - p.1 * s, p.0 * s + p.1 * k)).collect();
        assert!((proportion_of(&turned) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_the_enclosing_rectangle_gives_the_sides_and_a_stored_summary_reads_as_a_signature() {
        let (long, short) = enclosing_rectangle(&rect(4000.0, 2500.0)).unwrap();
        assert!((long - 4000.0).abs() < 1e-6 && (short - 2500.0).abs() < 1e-6);
        let sig = Signature::from(RoomSummary { x_mm: 4000.0, y_mm: 2000.0, area_m2: 7.5, items: 4 });
        assert_eq!((sig.area_m2, sig.proportion, sig.items), (7.5, 2.0, 4));
    }

    #[test]
    fn test_an_l_shaped_room_is_read_by_the_rectangle_that_encloses_it() {
        let l = vec![
            (0.0, 0.0),
            (6000.0, 0.0),
            (6000.0, 2000.0),
            (2000.0, 2000.0),
            (2000.0, 4000.0),
            (0.0, 4000.0),
        ];
        assert!((proportion_of(&l) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn test_distance_is_symmetric_in_the_ratio_and_zero_for_equal_rooms() {
        let a = sig(20.0, 1.5, 10);
        assert_eq!(distance(&a, &a).1, 0.0);
        let (double, _) = distance(&a, &sig(40.0, 1.5, 10));
        let (half, _) = distance(&a, &sig(10.0, 1.5, 10));
        assert!((double.area - half.area).abs() < 1e-12, "twice the area is as far as half the area");
    }

    #[test]
    fn test_the_three_figures_weigh_equally() {
        let target = sig(20.0, 1.5, 10);
        // One figure off by the same ratio each time: the distance is the same whichever figure it is.
        let by_area = distance(&target, &sig(40.0, 1.5, 10)).1;
        let by_proportion = distance(&target, &sig(20.0, 3.0, 10)).1;
        let by_items = distance(&target, &sig(20.0, 1.5, 21)).1; // (21 + 1) / (10 + 1) = 2
        assert!((by_area - by_proportion).abs() < 1e-12 && (by_area - by_items).abs() < 1e-12);
    }

    #[test]
    fn test_an_empty_room_is_a_number_not_infinity() {
        let d = distance(&sig(10.0, 1.0, 0), &sig(10.0, 1.0, 1));
        assert!(d.1.is_finite() && d.1 > 0.0);
    }

    #[test]
    fn test_the_closest_room_ranks_first_and_ties_break_by_sheet_number() {
        let target = sig(20.0, 1.5, 10);
        let (a, b, c, d) = (record("a", "RLS-03"), record("b", "RLS-01"), record("c", "RLS-02"), record("d", "RLS-04"));
        let room = |n: &str| Some(SheetRoom { model_id: "m".into(), room_id: n.into() });
        let ranked = rank(
            &target,
            vec![
                (&a, room("a"), "far".into(), sig(80.0, 4.0, 40)),
                (&b, room("b"), "twin 1".into(), sig(20.0, 1.5, 10)),
                (&c, room("c"), "twin 2".into(), sig(20.0, 1.5, 10)),
                (&d, room("d"), "near".into(), sig(22.0, 1.4, 11)),
            ],
        );
        let order: Vec<&str> = ranked.iter().map(|s| s.sheet_number.as_str()).collect();
        assert_eq!(order, ["RLS-01", "RLS-02", "RLS-04", "RLS-03"]);
        assert_eq!(ranked[0].distance, 0.0);
        assert!(ranked[3].parts.area > ranked[2].parts.area, "the figures ride the answer");
    }
}
