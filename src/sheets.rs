//! The room layout sheet library: sheets a person chose to export from Revit, kept as
//! exported, so the server can offer the one closest to a room (`docs/PLAN-blueprints.md`,
//! track B, decisions 18 to 20).
//!
//! **Their own endpoint and their own files, apart from blueprints.** A sheet in the
//! library is stored as exported and belongs to no blueprint; a blueprint revision holds its
//! own copy of its sheets (plan decision 18), a different storage method. One JSON file per sheet, under
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
//! **A sheet's room is a reference that is never validated on save.** It is chosen at export
//! (through the blueprint route) or afterwards in the viewer; authored data outlives the
//! geometry, so a room that has gone is a reported state at read time, not an error.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::is_path_safe_component;

pub const SCHEMA_VERSION: u32 = 1;

const MAX_SHEETS_PER_PROJECT: usize = 20_000;
const MAX_SHEETS_PER_PUSH: usize = 500;
const MAX_PENDING: usize = 20;
const MAX_SHEET_BYTES: usize = 5_000_000;
const MAX_TEXT_CHARS: usize = 500;

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

/// A re-push that matched an existing sheet but changed its number, waiting for a person.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingSheet {
    pub pending_id: u32,
    pub received_at: String,
    pub sheet_number: String,
    pub sheet_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<SheetRoom>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_summary: Option<RoomSummary>,
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
    /// The Revit phase the export was run under, when it was run through the blueprint route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// The room as it was when the sheet's room was set (decision 25). Absent for a sheet pushed
    /// before this was stored, or whose room was not on the server then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_summary: Option<RoomSummary>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_summary: Option<RoomSummary>,
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
            room_summary: r.room_summary,
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
    #[serde(default)]
    pub phase: Option<String>,
    /// Filled in by the server from the rooms and FF&E it holds; whatever the extractor sends is ignored.
    #[serde(default, skip_deserializing)]
    pub room_summary: Option<RoomSummary>,
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
pub struct UpdateRequest {
    pub base: String,
    /// Set the sheet's room.
    #[serde(default)]
    pub room: Option<SheetRoom>,
    /// Remove it. Wins over `room`.
    #[serde(default)]
    pub clear_room: bool,
    /// Filled in by the server for the new room; never read from the request.
    #[serde(default, skip_deserializing)]
    pub room_summary: Option<RoomSummary>,
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

static SAVE_LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn bad<T>(msg: String) -> Result<T, SheetError> {
    Err(SheetError::Invalid(msg))
}

fn internal<E: Into<anyhow::Error>>(e: E) -> SheetError {
    SheetError::Internal(e.into())
}

fn project_dir(projects_dir: &Path, project: &str) -> Result<PathBuf, SheetError> {
    if !is_path_safe_component(project) {
        return bad(format!("project id {project:?} cannot be a file name"));
    }
    Ok(projects_dir.join("sheets").join(project))
}

fn sheet_path(projects_dir: &Path, project: &str, id: &str) -> Result<PathBuf, SheetError> {
    if !is_path_safe_component(id) {
        return bad(format!("sheet id {id:?} cannot be a file name"));
    }
    Ok(project_dir(projects_dir, project)?.join(format!("{id}.json")))
}

fn read(path: &Path) -> Result<SheetRecord, SheetError> {
    let bytes = std::fs::read(path).map_err(internal)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| SheetError::Internal(anyhow::anyhow!("{} is not a sheet record: {e}", path.display())))
}

/// Every record of the project, by sheet number then document. A file that cannot be read is
/// an error, not a gap: skipping it would make a corrupt sheet look like a deleted one.
pub fn load_all(projects_dir: &Path, project: &str) -> Result<Vec<SheetRecord>, SheetError> {
    let dir = project_dir(projects_dir, project)?;
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(internal(e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.map_err(internal)?.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        out.push(read(&path)?);
    }
    out.sort_by(|a, b| {
        (a.sheet_number.to_lowercase(), &a.document_title, &a.id).cmp(&(
            b.sheet_number.to_lowercase(),
            &b.document_title,
            &b.id,
        ))
    });
    Ok(out)
}

pub fn list(projects_dir: &Path, project: &str) -> Result<Vec<SheetSummary>, SheetError> {
    Ok(load_all(projects_dir, project)?.iter().map(SheetSummary::from).collect())
}

pub fn load(projects_dir: &Path, project: &str, id: &str) -> Result<SheetRecord, SheetError> {
    let path = sheet_path(projects_dir, project, id)?;
    match std::fs::read(&path) {
        Ok(_) => read(&path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(SheetError::NotFound(format!("no sheet {id:?} in project {project:?}")))
        }
        Err(e) => Err(internal(e)),
    }
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

fn next_stamp(previous: &str) -> String {
    let stamp = now();
    if stamp.as_str() <= previous {
        format!("{previous}~")
    } else {
        stamp
    }
}

fn new_id(dir: &Path) -> String {
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let id = format!("sh-{:x}-{n:x}", chrono::Utc::now().timestamp_micros());
        if !dir.join(format!("{id}.json")).exists() {
            return id;
        }
    }
}

fn conflict(base: &str, current: &str) -> SheetError {
    SheetError::Conflict(format!(
        "the sheet changed since you read it (yours is from {base:?}, the server has {current:?}); reload and try again"
    ))
}

/// Same order as `blueprints::write`: everything that can refuse runs before this, the old file
/// is copied aside while it is still in place, and the new one is installed by rename.
fn write(projects_dir: &Path, project: &str, record: &SheetRecord) -> Result<(), SheetError> {
    let path = sheet_path(projects_dir, project, &record.id)?;
    let dir = path.parent().expect("a sheet path always has a parent");
    std::fs::create_dir_all(dir).map_err(internal)?;
    let json = serde_json::to_vec_pretty(record).map_err(internal)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(internal)?;
    let stem = format!("sheet-{project}-{}", record.id);
    if path.exists()
        && let Err(e) = crate::backups::back_up(projects_dir, &stem, "json", &path)
    {
        std::fs::remove_file(&temp).ok();
        return Err(SheetError::Internal(e));
    }
    std::fs::rename(&temp, &path).map_err(internal)?;
    crate::backups::prune(projects_dir, &stem, "json");
    Ok(())
}

fn check_text(label: &str, s: &str, required: bool) -> Result<String, SheetError> {
    let s = s.trim();
    if required && s.is_empty() {
        return bad(format!("a sheet needs a {label}"));
    }
    if s.chars().count() > MAX_TEXT_CHARS {
        return bad(format!("a sheet's {label} is at most {MAX_TEXT_CHARS} characters"));
    }
    Ok(s.to_string())
}

fn check_room(room: &Option<SheetRoom>) -> Result<(), SheetError> {
    if let Some(r) = room
        && (r.model_id.trim().is_empty() || r.room_id.trim().is_empty())
    {
        return bad("a sheet's room needs a model id and a room id".to_string());
    }
    Ok(())
}

fn check_pushed(p: &PushedSheet) -> Result<PushedSheet, SheetError> {
    check_room(&p.room)?;
    if !p.sheet.is_object() {
        return bad(format!("sheet {:?}: the exported data must be an object", p.sheet_number));
    }
    let size = serde_json::to_vec(&p.sheet).map_err(internal)?.len();
    if size > MAX_SHEET_BYTES {
        return bad(format!("sheet {:?} is {size} bytes; at most {MAX_SHEET_BYTES}", p.sheet_number));
    }
    Ok(PushedSheet {
        revit_sheet_id: check_text("Revit sheet id", &p.revit_sheet_id, true)?,
        document_title: check_text("document title", &p.document_title, true)?,
        sheet_number: check_text("sheet number", &p.sheet_number, true)?,
        sheet_name: check_text("sheet name", &p.sheet_name, false)?,
        room: p.room.clone(),
        phase: p.phase.clone().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        room_summary: p.room_summary,
        sheet: p.sheet.clone(),
    })
}

/// Store sheets that arrived from the extractor, by the matching rule in the module header.
/// All-or-nothing on validation: nothing is written unless every sheet in the push is valid.
pub fn push(projects_dir: &Path, project: &str, request: PushRequest) -> Result<Vec<PushOutcome>, SheetError> {
    if request.sheets.is_empty() {
        return bad("a push needs at least one sheet".to_string());
    }
    if request.sheets.len() > MAX_SHEETS_PER_PUSH {
        return bad(format!("a push carries at most {MAX_SHEETS_PER_PUSH} sheets"));
    }
    let pushed: Vec<PushedSheet> = request.sheets.iter().map(check_pushed).collect::<Result<_, _>>()?;
    let mut keys = std::collections::HashSet::new();
    for p in &pushed {
        if !keys.insert((p.revit_sheet_id.as_str(), p.document_title.as_str())) {
            return bad(format!(
                "sheet {:?} of {:?} appears twice in the push",
                p.revit_sheet_id, p.document_title
            ));
        }
    }

    let _guard = SAVE_LOCK.lock().unwrap();
    let mut existing = load_all(projects_dir, project)?;
    let dir = project_dir(projects_dir, project)?;
    let mut outcomes = Vec::new();
    for p in pushed {
        let at = existing
            .iter()
            .position(|r| r.revit_sheet_id == p.revit_sheet_id && r.document_title == p.document_title);
        let received_at = now();
        match at {
            None => {
                if existing.len() >= MAX_SHEETS_PER_PROJECT {
                    return bad(format!("a project holds at most {MAX_SHEETS_PER_PROJECT} sheets"));
                }
                let record = SheetRecord {
                    schema_version: SCHEMA_VERSION,
                    id: new_id(&dir),
                    taken_at: received_at.clone(),
                    revit_sheet_id: p.revit_sheet_id.clone(),
                    document_title: p.document_title.clone(),
                    sheet_number: p.sheet_number.clone(),
                    sheet_name: p.sheet_name,
                    room: p.room,
                    phase: p.phase,
                    room_summary: p.room_summary,
                    received_at,
                    sheet: p.sheet,
                    pending: Vec::new(),
                    rejected: Vec::new(),
                    last_pending_id: 0,
                };
                write(projects_dir, project, &record)?;
                outcomes.push(outcome_of(&record, Outcome::Created));
                existing.push(record);
            }
            Some(i) if existing[i].sheet_number == p.sheet_number => {
                let record = &mut existing[i];
                record.taken_at = next_stamp(&record.taken_at);
                record.sheet_name = p.sheet_name;
                record.sheet = p.sheet;
                record.received_at = received_at;
                // The room chosen in Revit is an explicit choice, so it sets; no room keeps what is there.
                if p.room.is_some() {
                    record.room = p.room;
                    record.room_summary = p.room_summary;
                }
                if p.phase.is_some() {
                    record.phase = p.phase;
                }
                write(projects_dir, project, record)?;
                outcomes.push(outcome_of(record, Outcome::Updated));
            }
            Some(i) => {
                let record = &mut existing[i];
                if record.pending.len() >= MAX_PENDING {
                    return bad(format!(
                        "sheet {:?} holds {MAX_PENDING} pending pushes already; review them first",
                        record.sheet_number
                    ));
                }
                record.last_pending_id += 1;
                let pending_id = record.last_pending_id;
                record.taken_at = next_stamp(&record.taken_at);
                record.pending.push(PendingSheet {
                    pending_id,
                    received_at,
                    sheet_number: p.sheet_number,
                    sheet_name: p.sheet_name,
                    room: p.room,
                    phase: p.phase,
                    room_summary: p.room_summary,
                    sheet: p.sheet,
                });
                write(projects_dir, project, record)?;
                outcomes.push(outcome_of(record, Outcome::Pending { pending_id }));
            }
        }
    }
    tracing::info!("sheets push to project {project}: {} sheet(s)", outcomes.len());
    Ok(outcomes)
}

fn outcome_of(r: &SheetRecord, outcome: Outcome) -> PushOutcome {
    PushOutcome {
        id: r.id.clone(),
        revit_sheet_id: r.revit_sheet_id.clone(),
        document_title: r.document_title.clone(),
        sheet_number: r.sheet_number.clone(),
        outcome,
    }
}

/// Set or clear the sheet's room (the viewer's step for rooms no blueprint covers).
pub fn update(projects_dir: &Path, project: &str, id: &str, request: UpdateRequest) -> Result<SheetRecord, SheetError> {
    check_room(&request.room)?;
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut record = load(projects_dir, project, id)?;
    if record.taken_at != request.base {
        return Err(conflict(&request.base, &record.taken_at));
    }
    if request.clear_room {
        record.room = None;
        record.room_summary = None;
    } else if request.room.is_some() {
        record.room = request.room;
        record.room_summary = request.room_summary;
    }
    record.taken_at = next_stamp(&record.taken_at);
    write(projects_dir, project, &record)?;
    Ok(record)
}

fn take_pending(record: &mut SheetRecord, pending_id: u32) -> Result<PendingSheet, SheetError> {
    match record.pending.iter().position(|p| p.pending_id == pending_id) {
        Some(i) => Ok(record.pending.remove(i)),
        None => bad(format!("the sheet has no pending push {pending_id}")),
    }
}

/// Accept a stashed push: its number, name, data and room replace the record's.
pub fn accept(
    projects_dir: &Path,
    project: &str,
    id: &str,
    pending_id: u32,
    request: DecideRequest,
) -> Result<SheetRecord, SheetError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut record = load(projects_dir, project, id)?;
    if record.taken_at != request.base {
        return Err(conflict(&request.base, &record.taken_at));
    }
    let accepted = take_pending(&mut record, pending_id)?;
    record.sheet_number = accepted.sheet_number;
    record.sheet_name = accepted.sheet_name;
    record.sheet = accepted.sheet;
    record.received_at = accepted.received_at;
    if accepted.room.is_some() {
        record.room = accepted.room;
        record.room_summary = accepted.room_summary;
    }
    if accepted.phase.is_some() {
        record.phase = accepted.phase;
    }
    record.taken_at = next_stamp(&record.taken_at);
    write(projects_dir, project, &record)?;
    Ok(record)
}

/// Reject a stashed push. It is kept, marked rejected, and the sheet is untouched.
pub fn reject(
    projects_dir: &Path,
    project: &str,
    id: &str,
    pending_id: u32,
    request: DecideRequest,
) -> Result<SheetRecord, SheetError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut record = load(projects_dir, project, id)?;
    if record.taken_at != request.base {
        return Err(conflict(&request.base, &record.taken_at));
    }
    let declined = take_pending(&mut record, pending_id)?;
    record.taken_at = next_stamp(&record.taken_at);
    record
        .rejected
        .push(RejectedSheet { rejected_at: record.taken_at.clone(), pending: declined });
    write(projects_dir, project, &record)?;
    Ok(record)
}

pub fn delete(projects_dir: &Path, project: &str, id: &str, base: &str) -> Result<(), SheetError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    let record = load(projects_dir, project, id)?;
    if record.taken_at != base {
        return Err(conflict(base, &record.taken_at));
    }
    let path = sheet_path(projects_dir, project, id)?;
    let stem = format!("sheet-{project}-{id}");
    crate::backups::back_up(projects_dir, &stem, "json", &path).map_err(SheetError::Internal)?;
    std::fs::remove_file(&path).map_err(internal)?;
    tracing::info!("deleted sheet {id} from project {project}");
    Ok(())
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
            phase: None,
            room_summary: None,
            sheet: json!({ "view_ports": [{}, {}, {}] }),
        }
    }

    fn summary(items: usize) -> RoomSummary {
        RoomSummary { x_mm: 4000.0, y_mm: 3000.0, area_m2: 12.0, items }
    }

    /// The room's summary is stored with the sheet, follows its room, and is dropped with it.
    #[test]
    fn test_the_room_summary_is_stored_replaced_with_the_room_and_cleared_with_it() {
        let dir = temp_dir("summary");
        let mut s = sheet("1", "Doc", "RLS-01");
        s.room = room("m", "r1");
        s.room_summary = Some(summary(7));
        let pushed = push_one(&dir, s);
        let rec = load(&dir, "p1", &pushed.id).unwrap();
        assert_eq!(rec.room_summary, Some(summary(7)));
        let moved = update(
            &dir,
            "p1",
            &rec.id,
            UpdateRequest {
                base: rec.taken_at.clone(),
                room: room("m", "r2"),
                clear_room: false,
                room_summary: Some(summary(9)),
            },
        )
        .unwrap();
        assert_eq!(moved.room_summary, Some(summary(9)));
        let cleared = update(
            &dir,
            "p1",
            &rec.id,
            UpdateRequest { base: moved.taken_at, room: None, clear_room: true, room_summary: None },
        )
        .unwrap();
        assert_eq!((cleared.room, cleared.room_summary), (None, None));
        std::fs::remove_dir_all(&dir).ok();
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
    fn test_a_room_can_be_set_and_cleared_afterwards() {
        let dir = temp_dir("set-room");
        let first = push_one(&dir, sheet("100", "House A", "RLS-01"));
        let rec = load(&dir, "p1", &first.id).unwrap();
        let set = update(
            &dir,
            "p1",
            &first.id,
            UpdateRequest {
                base: rec.taken_at.clone(),
                room: room("m", "r1"),
                clear_room: false,
                room_summary: None,
            },
        )
        .unwrap();
        assert_eq!(set.room, room("m", "r1"));
        let stale = update(
            &dir,
            "p1",
            &first.id,
            UpdateRequest { base: rec.taken_at, room: room("m", "r2"), clear_room: false, room_summary: None },
        );
        assert!(matches!(stale, Err(SheetError::Conflict(_))));
        let cleared = update(
            &dir,
            "p1",
            &first.id,
            UpdateRequest { base: set.taken_at, room: None, clear_room: true, room_summary: None },
        )
        .unwrap();
        assert_eq!(cleared.room, None);
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
