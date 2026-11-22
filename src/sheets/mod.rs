//! The room layout sheet library: sheets a person chose to export from Revit, kept as
//! exported, so the server can offer the one closest to a room (`docs/PLAN-blueprints.md`,
//! track B, decisions 19, 20 and 27).
//!
//! **Their own endpoint and their own files, apart from blueprints.** A sheet in the
//! library is stored as exported and is **generic or bound to one room blueprint** by id
//! (`blueprint`, plan decision 27); a blueprint holds no copy. One JSON file per sheet, under
//! `<projects_dir>/sheets/<project>/<id>.json`, written the way blueprints are (temp then
//! rename, a copy of the replaced file under `.backups/`, one lock), and this module is the
//! only code that reads or writes them.
//!
//! **A sheet is identified by its Revit sheet id plus the document's title** (decided
//! 2026-10-19). A sheet id is unique only within a document, and the title says which. A
//! push that matches on both is the same sheet; one that matches on neither is a new sheet,
//! *even if its number matches an existing one*. A match whose **sheet number has changed
//! is stashed as pending**, and a person accepts or rejects it, because a renumbering is
//! exactly when "same sheet" is a claim someone should confirm. A match with the same
//! number just updates the sheet.
//!
//! **The sheet itself is opaque JSON, as duHast exports it** (`DataSheet`: properties,
//! viewports with their sheet positions, crop boxes, schedules). Typing it here would pin
//! the server to a rough export that is still changing; the figures the server does use
//! (identity, number, name, room) are separate fields.
//!
//! **A sheet's room link is a reference that is never validated on save.** A sheet pushed with a
//! blueprint is made for the blueprint's room; a sheet from the Sheets button has no link, only the
//! figures of the rooms the person said it covers. Authored data outlives the geometry, so a room
//! that has gone is a reported state at read time, not an error.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::is_path_safe_component;

mod push;
mod store;
pub use push::*;
pub use store::*;

pub const SCHEMA_VERSION: u32 = 1;

const MAX_SHEETS_PER_PROJECT: usize = 20_000;
const MAX_SHEETS_PER_PUSH: usize = 500;
const MAX_PENDING: usize = 20;
const MAX_SHEET_BYTES: usize = 5_000_000;
const MAX_TEXT_CHARS: usize = 500;
const MAX_ROOM_LABELS: usize = 200;
const MAX_OUTLINE_POINTS: usize = 5_000;

/// The room a sheet was made for. A reference, not validated against the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetRoom {
    pub model_id: String,
    pub room_id: String,
}

/// The room a sheet was made for, as it was when the sheet was pushed (plan decision 25): so
/// the sheet can be compared with a task's room without that room being on the server, or
/// unchanged. **No geometry**, and worked out by the server from the rooms and FF&E it holds,
/// never sent by the extractor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RoomSummary {
    /// The long side of the room's smallest enclosing rectangle, mm.
    pub x_mm: f64,
    /// The short side of that rectangle, mm.
    pub y_mm: f64,
    /// The room's area, square metres.
    pub area_m2: f64,
    /// FF&E items attributed to the room.
    pub items: usize,
}

/// The paper size of a sheet, measured in Revit from its title block's bounding box. Millimetres,
/// as measured: never snapped to A0..A3 (a sheet that fits none is still a size).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SheetSize {
    pub width_mm: f64,
    pub height_mm: f64,
}

/// A re-push that matched an existing sheet but changed its number, waiting for a person.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingSheet {
    pub pending_id: u32,
    pub received_at: String,
    pub sheet_number: String,
    pub sheet_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<SheetRoom>,
    /// The room blueprint this sheet belongs to (plan decision 27), or `None` for a generic
    /// sheet. Set by a blueprint push and kept by a bare re-push; one blueprint at most.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_summary: Option<RoomSummary>,
    /// The sheet's paper size, when the extractor measured one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_mm: Option<SheetSize>,
    /// The outer outline of the ONE room the sheet was made for, as it was when the sheet was pushed: mm, in the
    /// sheet's own model frame (the frame its views are in). What carries the sheet's views to another room (the
    /// elevations, the plan crop and its turn) needs only this, so a sheet with no room link, one whose room was
    /// redrawn since, or one copied into another project still transfers. Empty for a sheet covering several rooms
    /// (their views are not against one room's walls) and for one pushed before it was stored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub room_outline: Vec<[f64; 2]>,
    /// The rooms the person said the sheet covers when they exported it ("number name"), a label
    /// only: the figures in `room_summary` already cover all of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rooms: Vec<String>,
    pub sheet: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedSheet {
    pub rejected_at: String,
    #[serde(flatten)]
    pub pending: PendingSheet,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetRecord {
    pub schema_version: u32,
    pub id: String,
    /// When this record was last saved: its version, and the `base` the next write names.
    pub taken_at: String,
    /// The sheet's `ElementId` in its document. With `document_title`, its identity.
    pub revit_sheet_id: String,
    pub document_title: String,
    pub sheet_number: String,
    pub sheet_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<SheetRoom>,
    /// The room blueprint this sheet belongs to (plan decision 27), or `None` for a generic
    /// sheet. Set by a blueprint push and kept by a bare re-push; one blueprint at most.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<String>,
    /// The Revit phase the export was run under, when it was run through the blueprint route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// The figures of the room(s) the sheet covers as they were when it was pushed (decision 25). Absent for a sheet pushed
    /// before this was stored, or whose room was not on the server then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_summary: Option<RoomSummary>,
    /// The sheet's paper size, when the extractor measured one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_mm: Option<SheetSize>,
    /// The outer outline of the ONE room the sheet was made for, as it was when the sheet was pushed: mm, in the
    /// sheet's own model frame (the frame its views are in). What carries the sheet's views to another room (the
    /// elevations, the plan crop and its turn) needs only this, so a sheet with no room link, one whose room was
    /// redrawn since, or one copied into another project still transfers. Empty for a sheet covering several rooms
    /// (their views are not against one room's walls) and for one pushed before it was stored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub room_outline: Vec<[f64; 2]>,
    /// The rooms the person said the sheet covers when they exported it ("number name"), a label
    /// only: the figures in `room_summary` already cover all of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rooms: Vec<String>,
    /// A GENERIC sheet's identifier: the size of its room's bounding rectangle and the number of
    /// items in it (`room_key`), so sheets made for alike rooms share one. Absent for a sheet
    /// bound to a blueprint (the blueprint is its identity) and for one with no room figures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_key: Option<String>,
    /// When the sheet's data last arrived.
    pub received_at: String,
    /// The sheet as duHast exports it.
    pub sheet: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<PendingSheet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<RejectedSheet>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub last_pending_id: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A record without the sheet body: what a list shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetSummary {
    pub id: String,
    pub taken_at: String,
    pub revit_sheet_id: String,
    pub document_title: String,
    pub sheet_number: String,
    pub sheet_name: String,
    pub room: Option<SheetRoom>,
    /// The room blueprint the sheet belongs to; absent for a generic sheet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_summary: Option<RoomSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_mm: Option<SheetSize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rooms: Vec<String>,
    pub viewports: usize,
    pub pending: usize,
}

impl From<&SheetRecord> for SheetSummary {
    fn from(r: &SheetRecord) -> Self {
        SheetSummary {
            id: r.id.clone(),
            taken_at: r.taken_at.clone(),
            revit_sheet_id: r.revit_sheet_id.clone(),
            document_title: r.document_title.clone(),
            sheet_number: r.sheet_number.clone(),
            sheet_name: r.sheet_name.clone(),
            room: r.room.clone(),
            blueprint: r.blueprint.clone(),
            room_key: r.room_key.clone(),
            room_summary: r.room_summary,
            size_mm: r.size_mm,
            rooms: r.rooms.clone(),
            viewports: r.sheet.get("view_ports").and_then(Value::as_array).map_or(0, Vec::len),
            pending: r.pending.len(),
        }
    }
}

/// One sheet as the extractor sends it.
#[derive(Debug, Clone, Deserialize)]
pub struct PushedSheet {
    pub revit_sheet_id: String,
    pub document_title: String,
    pub sheet_number: String,
    #[serde(default)]
    pub sheet_name: String,
    #[serde(default)]
    pub room: Option<SheetRoom>,
    /// Bind the sheet to this room blueprint (the blueprint push does). Absent: a re-push leaves
    /// the binding as it is, so the Sheets button never unbinds a sheet.
    #[serde(default)]
    pub blueprint: Option<String>,
    #[serde(default)]
    pub phase: Option<String>,
    /// The figures of the room (or rooms) the sheet covers, measured by the extractor. When it sends
    /// a `room` and no summary, the server reads them from the rooms it holds instead.
    #[serde(default)]
    pub room_summary: Option<RoomSummary>,
    #[serde(default)]
    pub size_mm: Option<SheetSize>,
    /// The outline of the one room the sheet covers, measured by the extractor (mm, the sheet's model frame). When
    /// it sends a `room` and no outline, the server reads it from the rooms it holds instead.
    #[serde(default)]
    pub room_outline: Vec<[f64; 2]>,
    #[serde(default)]
    pub rooms: Vec<String>,
    pub sheet: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PushRequest {
    pub sheets: Vec<PushedSheet>,
}

/// What a push did with one sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// No sheet matched on id and document: a new record.
    Created,
    /// Matched, same number: updated in place.
    Updated,
    /// Matched, but the number changed: stashed, waiting for a person.
    Pending { pending_id: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushOutcome {
    pub id: String,
    pub revit_sheet_id: String,
    pub document_title: String,
    pub sheet_number: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DecideRequest {
    pub base: String,
}

#[derive(Debug)]
pub enum SheetError {
    NotFileBacked,
    Invalid(String),
    NotFound(String),
    Conflict(String),
    Internal(anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-sheets-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok(); // a stale one from a run with the same process id would read as saved data
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sheet(id: &str, doc: &str, number: &str) -> PushedSheet {
        PushedSheet {
            revit_sheet_id: id.into(),
            document_title: doc.into(),
            sheet_number: number.into(),
            sheet_name: format!("Layout {number}"),
            room: None,
            blueprint: None,
            phase: None,
            room_summary: None,
            size_mm: None,
            room_outline: Vec::new(),
            rooms: Vec::new(),
            sheet: json!({ "view_ports": [{}, {}, {}] }),
        }
    }

    fn summary(items: usize) -> RoomSummary {
        RoomSummary { x_mm: 4000.0, y_mm: 3000.0, area_m2: 12.0, items }
    }

    /// A sheet needs no room: the figures and size the extractor measured are stored with it, a
    /// re-push replaces them, and a re-push that sends none keeps what is there.
    #[test]
    fn test_measured_figures_and_size_are_stored_without_a_room_and_kept_by_a_bare_re_push() {
        let dir = temp_dir("measured");
        let mut s = sheet("1", "Doc", "RLS-01");
        s.room_summary = Some(summary(5));
        s.size_mm = Some(SheetSize { width_mm: 841.0, height_mm: 594.0 });
        s.rooms = vec!["1.01 Bedroom".into(), "1.02 Ensuite".into()];
        let pushed = push_one(&dir, s);
        let rec = load(&dir, "p1", &pushed.id).unwrap();
        assert!(rec.room.is_none());
        assert_eq!(rec.room_summary, Some(summary(5)));
        assert_eq!(rec.size_mm, Some(SheetSize { width_mm: 841.0, height_mm: 594.0 }));
        assert_eq!(rec.rooms.len(), 2);

        push_one(&dir, sheet("1", "Doc", "RLS-01"));
        let kept = load(&dir, "p1", &pushed.id).unwrap();
        assert_eq!(kept.room_summary, Some(summary(5)));
        assert_eq!(kept.size_mm.map(|z| z.width_mm), Some(841.0));

        let mut again = sheet("1", "Doc", "RLS-01");
        again.room_summary = Some(summary(8));
        push_one(&dir, again);
        assert_eq!(load(&dir, "p1", &pushed.id).unwrap().room_summary, Some(summary(8)));
    }

    /// The outline of a one-room sheet is stored with its figures, kept by a bare re-push, and goes when a re-push
    /// measures the sheet again without one (it now covers several rooms).
    #[test]
    fn test_a_room_outline_travels_with_the_figures() {
        let dir = temp_dir("outline");
        let ring = vec![[0.0, 0.0], [4000.0, 0.0], [4000.0, 3000.0], [0.0, 3000.0]];
        let mut s = sheet("1", "Doc", "RLS-01");
        s.room_summary = Some(summary(5));
        s.room_outline = ring.clone();
        let pushed = push_one(&dir, s);
        assert_eq!(load(&dir, "p1", &pushed.id).unwrap().room_outline, ring);

        push_one(&dir, sheet("1", "Doc", "RLS-01"));
        assert_eq!(load(&dir, "p1", &pushed.id).unwrap().room_outline, ring);

        let mut several = sheet("1", "Doc", "RLS-01");
        several.room_summary = Some(summary(9));
        push_one(&dir, several);
        assert!(load(&dir, "p1", &pushed.id).unwrap().room_outline.is_empty());
    }

    #[test]
    fn test_a_broken_outline_is_refused() {
        let dir = temp_dir("outline-bad");
        let mut short = sheet("1", "Doc", "RLS-01");
        short.room_outline = vec![[0.0, 0.0], [1.0, 1.0]];
        assert!(matches!(
            push(&dir, "p1", PushRequest { sheets: vec![short] }),
            Err(SheetError::Invalid(_))
        ));
        let mut nan = sheet("1", "Doc", "RLS-01");
        nan.room_outline = vec![[0.0, 0.0], [1.0, f64::NAN], [1.0, 1.0]];
        assert!(matches!(push(&dir, "p1", PushRequest { sheets: vec![nan] }), Err(SheetError::Invalid(_))));
    }

    fn push_one(dir: &Path, s: PushedSheet) -> PushOutcome {
        push(dir, "p1", PushRequest { sheets: vec![s] }).unwrap().remove(0)
    }

    fn room(m: &str, r: &str) -> Option<SheetRoom> {
        Some(SheetRoom { model_id: m.into(), room_id: r.into() })
    }

    #[test]
    fn test_a_new_sheet_is_created_and_listed() {
        let dir = temp_dir("create");
        let out = push_one(&dir, sheet("100", "House A", "RLS-01"));
        assert_eq!(out.outcome, Outcome::Created);
        let rows = list(&dir, "p1").unwrap();
        assert_eq!((rows.len(), rows[0].viewports, rows[0].sheet_number.as_str()), (1, 3, "RLS-01"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_the_same_id_and_document_with_the_same_number_updates_in_place() {
        let dir = temp_dir("update");
        let first = push_one(&dir, sheet("100", "House A", "RLS-01"));
        let mut again = sheet("100", "House A", "RLS-01");
        again.sheet = json!({ "view_ports": [{}] });
        again.sheet_name = "Renamed".into();
        let out = push_one(&dir, again);
        assert_eq!(out.outcome, Outcome::Updated);
        assert_eq!(out.id, first.id, "the same record");
        let rec = load(&dir, "p1", &first.id).unwrap();
        assert_eq!((rec.sheet_name.as_str(), list(&dir, "p1").unwrap()[0].viewports), ("Renamed", 1));
        assert_eq!(list(&dir, "p1").unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Plan decision 27: a sheet is generic or bound to one room blueprint. A push that names a
    /// blueprint binds it; a bare re-push (the Sheets button) never unbinds it; a renumbered push
    /// that names one binds it only when accepted.
    #[test]
    fn test_a_blueprint_push_binds_a_sheet_and_a_bare_re_push_keeps_the_binding() {
        let dir = temp_dir("bound");
        let mut bound = sheet("100", "House A", "RLS-01");
        bound.blueprint = Some("bp-1".into());
        let first = push_one(&dir, bound);
        push_one(&dir, sheet("200", "House A", "RLS-02"));
        assert_eq!(
            bound_to(&dir, "p1", "bp-1").unwrap().iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            [first.id.as_str()]
        );
        assert!(bound_to(&dir, "p1", "bp-2").unwrap().is_empty());

        let mut again = sheet("100", "House A", "RLS-01");
        again.sheet_name = "Renamed".into();
        push_one(&dir, again);
        let rec = load(&dir, "p1", &first.id).unwrap();
        assert_eq!((rec.sheet_name.as_str(), rec.blueprint.as_deref()), ("Renamed", Some("bp-1")));

        let mut moved = sheet("100", "House A", "RLS-09");
        moved.blueprint = Some("bp-2".into());
        let out = push_one(&dir, moved);
        assert_eq!(out.outcome, Outcome::Pending { pending_id: 1 });
        assert_eq!(
            load(&dir, "p1", &first.id).unwrap().blueprint.as_deref(),
            Some("bp-1"),
            "pending changes nothing"
        );
        let rec = load(&dir, "p1", &first.id).unwrap();
        accept(&dir, "p1", &first.id, 1, DecideRequest { base: rec.taken_at }).unwrap();
        assert_eq!(load(&dir, "p1", &first.id).unwrap().blueprint.as_deref(), Some("bp-2"));
        assert_eq!(list(&dir, "p1").unwrap().iter().filter(|r| r.blueprint.is_some()).count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A generic sheet is identified by its room's bounding rectangle and item count; binding it to a
    /// blueprint takes the identifier away (the blueprint is its identity), and a sheet with no room
    /// figures has none.
    #[test]
    fn test_a_generic_sheet_gets_a_key_from_its_rooms_size_and_items() {
        let dir = temp_dir("room-key");
        let mut generic = sheet("100", "House A", "RLS-01");
        generic.room_summary = Some(RoomSummary { x_mm: 4003.0, y_mm: 2998.0, area_m2: 12.0, items: 12 });
        let first = push_one(&dir, generic.clone());
        assert_eq!(load(&dir, "p1", &first.id).unwrap().room_key.as_deref(), Some("4000x3000-12"));
        assert_eq!(list(&dir, "p1").unwrap()[0].room_key.as_deref(), Some("4000x3000-12"));
        push_one(&dir, sheet("200", "House A", "RLS-02"));
        let none = list(&dir, "p1").unwrap().into_iter().find(|s| s.sheet_number == "RLS-02").unwrap();
        assert_eq!(none.room_key, None, "no room figures, no key");

        let mut bound = generic;
        bound.blueprint = Some("bp-1".into());
        push_one(&dir, bound);
        assert_eq!(
            load(&dir, "p1", &first.id).unwrap().room_key,
            None,
            "a bound sheet is identified by its blueprint"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_sheet_the_blueprint_no_longer_carries_is_unbound_and_becomes_generic() {
        let dir = temp_dir("unbind");
        let summary = Some(RoomSummary { x_mm: 4000.0, y_mm: 3000.0, area_m2: 12.0, items: 5 });
        for (id, number) in [("100", "RLS-01"), ("200", "RLS-02")] {
            let mut s = sheet(id, "House A", number);
            s.blueprint = Some("bp".into());
            s.room_summary = summary;
            push_one(&dir, s);
        }
        assert!(
            list(&dir, "p1").unwrap().iter().all(|s| s.room_key.is_none()),
            "bound: identified by the blueprint"
        );
        let n = unbind_missing(&dir, "p1", "bp", &[("100".into(), "House A".into())]).unwrap();
        assert_eq!(n, 1);
        let rows = list(&dir, "p1").unwrap();
        let kept = rows.iter().find(|s| s.sheet_number == "RLS-01").unwrap();
        let dropped = rows.iter().find(|s| s.sheet_number == "RLS-02").unwrap();
        assert_eq!((kept.blueprint.as_deref(), dropped.blueprint.as_deref()), (Some("bp"), None));
        assert_eq!(dropped.room_key.as_deref(), Some("4000x3000-5"), "generic now, so it has its key");
        assert_eq!(unbind_missing(&dir, "p1", "bp", &[("100".into(), "House A".into())]).unwrap(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Matching is on the Revit id AND the document: the same id in another document is a
    /// different sheet, and so is a new id with a number an existing sheet already has.
    #[test]
    fn test_a_match_needs_both_the_sheet_id_and_the_document() {
        let dir = temp_dir("match");
        let a = push_one(&dir, sheet("100", "House A", "RLS-01"));
        let other_doc = push_one(&dir, sheet("100", "House B", "RLS-01"));
        let same_number = push_one(&dir, sheet("200", "House A", "RLS-01"));
        assert_eq!((other_doc.outcome, same_number.outcome), (Outcome::Created, Outcome::Created));
        assert_ne!(a.id, other_doc.id);
        assert_ne!(a.id, same_number.id);
        assert_eq!(list(&dir, "p1").unwrap().len(), 3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_changed_sheet_number_is_stashed_and_the_sheet_is_untouched() {
        let dir = temp_dir("pending");
        let first = push_one(&dir, sheet("100", "House A", "RLS-01"));
        let mut renumbered = sheet("100", "House A", "RLS-09");
        renumbered.sheet = json!({ "view_ports": [] });
        let out = push_one(&dir, renumbered);
        assert_eq!(out.outcome, Outcome::Pending { pending_id: 1 });
        let rec = load(&dir, "p1", &first.id).unwrap();
        assert_eq!(rec.sheet_number, "RLS-01", "not changed until a person says so");
        assert_eq!(rec.sheet["view_ports"].as_array().unwrap().len(), 3);
        assert_eq!((rec.pending.len(), rec.pending[0].sheet_number.as_str()), (1, "RLS-09"));
        assert_eq!(list(&dir, "p1").unwrap()[0].pending, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_accepting_applies_the_stashed_push_and_rejecting_keeps_it_aside() {
        let dir = temp_dir("decide");
        let first = push_one(&dir, sheet("100", "House A", "RLS-01"));
        let stashed = push_one(&dir, sheet("100", "House A", "RLS-09"));
        assert!(matches!(stashed.outcome, Outcome::Pending { .. }));
        let rec = load(&dir, "p1", &first.id).unwrap();
        let accepted = accept(&dir, "p1", &first.id, 1, DecideRequest { base: rec.taken_at.clone() }).unwrap();
        assert_eq!((accepted.sheet_number.as_str(), accepted.pending.len()), ("RLS-09", 0));

        push_one(&dir, sheet("100", "House A", "RLS-12"));
        let rec = load(&dir, "p1", &first.id).unwrap();
        assert_eq!(rec.pending[0].pending_id, 2, "an accepted push's id is not handed out again");
        let rejected = reject(&dir, "p1", &first.id, 2, DecideRequest { base: rec.taken_at.clone() }).unwrap();
        assert_eq!(
            (rejected.sheet_number.as_str(), rejected.pending.len(), rejected.rejected.len()),
            ("RLS-09", 0, 1)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_push_with_a_room_sets_it_and_one_without_keeps_it() {
        let dir = temp_dir("room");
        let mut s = sheet("100", "House A", "RLS-01");
        s.room = room("m", "r1");
        let first = push_one(&dir, s);
        push_one(&dir, sheet("100", "House A", "RLS-01"));
        assert_eq!(
            load(&dir, "p1", &first.id).unwrap().room,
            room("m", "r1"),
            "no room in the push keeps the room"
        );
        let mut moved = sheet("100", "House A", "RLS-01");
        moved.room = room("m", "r2");
        push_one(&dir, moved);
        assert_eq!(load(&dir, "p1", &first.id).unwrap().room, room("m", "r2"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_bad_push_writes_nothing() {
        let dir = temp_dir("bad");
        let mut blank = sheet("", "House A", "RLS-02");
        blank.revit_sheet_id = " ".into();
        let r = push(&dir, "p1", PushRequest { sheets: vec![sheet("100", "House A", "RLS-01"), blank] });
        assert!(matches!(r, Err(SheetError::Invalid(_))));
        assert!(list(&dir, "p1").unwrap().is_empty(), "the valid sheet was not written either");

        let twice = push(
            &dir,
            "p1",
            PushRequest { sheets: vec![sheet("100", "House A", "A"), sheet("100", "House A", "B")] },
        );
        assert!(twice.is_err());
        let mut not_object = sheet("1", "H", "X");
        not_object.sheet = json!([1, 2]);
        assert!(push(&dir, "p1", PushRequest { sheets: vec![not_object] }).is_err());
        assert!(push(&dir, "p1", PushRequest { sheets: vec![] }).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_delete_needs_the_current_version_and_keeps_a_backup() {
        let dir = temp_dir("delete");
        let first = push_one(&dir, sheet("100", "House A", "RLS-01"));
        let rec = load(&dir, "p1", &first.id).unwrap();
        assert!(matches!(delete(&dir, "p1", &first.id, "old"), Err(SheetError::Conflict(_))));
        delete(&dir, "p1", &first.id, &rec.taken_at).unwrap();
        assert!(matches!(load(&dir, "p1", &first.id), Err(SheetError::NotFound(_))));
        assert_eq!(std::fs::read_dir(dir.join(".backups")).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_projects_are_separate_and_unsafe_ids_refused() {
        let dir = temp_dir("scope");
        let first = push_one(&dir, sheet("100", "House A", "RLS-01"));
        assert!(list(&dir, "p2").unwrap().is_empty());
        assert!(matches!(load(&dir, "p2", &first.id), Err(SheetError::NotFound(_))));
        assert!(matches!(load(&dir, "p1", "../x"), Err(SheetError::Invalid(_))));
        std::fs::remove_dir_all(&dir).ok();
    }
}
