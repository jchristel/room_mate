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

pub const SCHEMA_VERSION: u32 = 1;

const MAX_SHEETS_PER_PROJECT: usize = 20_000;
const MAX_SHEETS_PER_PUSH: usize = 500;
const MAX_PENDING: usize = 20;
const MAX_SHEET_BYTES: usize = 5_000_000;
const MAX_TEXT_CHARS: usize = 500;
const MAX_ROOM_LABELS: usize = 200;

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

/// Unbind the sheets bound to `blueprint` that a re-push of it no longer carries: they stay in the
/// library as generic sheets (and get their `room_key`), nothing is deleted. `keep` is the identity
/// (Revit sheet id, document title) of every sheet the push carried. Returns how many were unbound.
pub fn unbind_missing(
    projects_dir: &Path,
    project: &str,
    blueprint: &str,
    keep: &[(String, String)],
) -> Result<usize, SheetError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut changed = 0;
    for mut record in load_all(projects_dir, project)? {
        let carried = keep.iter().any(|(id, doc)| *id == record.revit_sheet_id && *doc == record.document_title);
        if record.blueprint.as_deref() == Some(blueprint) && !carried {
            record.blueprint = None;
            settle_room_key(&mut record);
            record.taken_at = next_stamp(&record.taken_at);
            write(projects_dir, project, &record)?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// The sheets bound to a room blueprint (plan decision 27), in number order.
pub fn bound_to(projects_dir: &Path, project: &str, blueprint: &str) -> Result<Vec<SheetRecord>, SheetError> {
    let mut out: Vec<SheetRecord> = load_all(projects_dir, project)?
        .into_iter()
        .filter(|r| r.blueprint.as_deref() == Some(blueprint))
        .collect();
    out.sort_by(|a, b| a.sheet_number.cmp(&b.sheet_number));
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
        blueprint: p.blueprint.clone().map(|b| b.trim().to_string()).filter(|b| !b.is_empty()),
        phase: p.phase.clone().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        room_summary: p.room_summary,
        size_mm: p.size_mm,
        rooms: p
            .rooms
            .iter()
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .take(MAX_ROOM_LABELS)
            .collect(),
        sheet: p.sheet.clone(),
    })
}

/// A generic sheet's identifier from its room's figures: the bounding rectangle's long and short
/// sides to the nearest 10 mm, then the item count (`4000x3000-12`). The figures are the extractor's
/// (the smallest enclosing rectangle, so a rotated room reads the same).
pub fn room_key(summary: &RoomSummary) -> String {
    let tens = |mm: f64| ((mm / 10.0).round() * 10.0) as i64;
    format!("{}x{}-{}", tens(summary.x_mm), tens(summary.y_mm), summary.items)
}

/// Keep `room_key` in step with what the record now holds: set for a generic sheet with room
/// figures, absent for a bound one or one without.
fn settle_room_key(record: &mut SheetRecord) {
    record.room_key = match (&record.blueprint, record.room_summary) {
        (None, Some(summary)) => Some(room_key(&summary)),
        _ => None,
    };
}

/// A re-push with the same sheet number: the sheet's data replaces the record's. What the
/// extractor measured (room figures, size) and the phase replace what was there only when this
/// push sent them; a room link, an explicit choice made in Revit, sets only when named.
fn refresh_record(record: &mut SheetRecord, p: PushedSheet, received_at: String) {
    record.taken_at = next_stamp(&record.taken_at);
    record.sheet_name = p.sheet_name;
    record.sheet = p.sheet;
    record.received_at = received_at;
    if p.room.is_some() {
        record.room = p.room;
        record.room_summary = p.room_summary;
    }
    if p.blueprint.is_some() {
        record.blueprint = p.blueprint;
    }
    if p.phase.is_some() {
        record.phase = p.phase;
    }
    if p.room_summary.is_some() {
        record.room_summary = p.room_summary;
        record.rooms = p.rooms;
    }
    if p.size_mm.is_some() {
        record.size_mm = p.size_mm;
    }
    settle_room_key(record);
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
                let mut record = SheetRecord {
                    schema_version: SCHEMA_VERSION,
                    id: new_id(&dir),
                    taken_at: received_at.clone(),
                    revit_sheet_id: p.revit_sheet_id.clone(),
                    document_title: p.document_title.clone(),
                    sheet_number: p.sheet_number.clone(),
                    sheet_name: p.sheet_name,
                    room: p.room,
                    blueprint: p.blueprint,
                    phase: p.phase,
                    room_summary: p.room_summary,
                    size_mm: p.size_mm,
                    rooms: p.rooms,
                    room_key: None,
                    received_at,
                    sheet: p.sheet,
                    pending: Vec::new(),
                    rejected: Vec::new(),
                    last_pending_id: 0,
                };
                settle_room_key(&mut record);
                write(projects_dir, project, &record)?;
                outcomes.push(outcome_of(&record, Outcome::Created));
                existing.push(record);
            }
            Some(i) if existing[i].sheet_number == p.sheet_number => {
                let record = &mut existing[i];
                refresh_record(record, p, received_at);
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
                    blueprint: p.blueprint,
                    phase: p.phase,
                    room_summary: p.room_summary,
                    size_mm: p.size_mm,
                    rooms: p.rooms,
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
    if accepted.blueprint.is_some() {
        record.blueprint = accepted.blueprint;
    }
    if accepted.phase.is_some() {
        record.phase = accepted.phase;
    }
    if accepted.room_summary.is_some() {
        record.room_summary = accepted.room_summary;
        record.rooms = accepted.rooms;
    }
    if accepted.size_mm.is_some() {
        record.size_mm = accepted.size_mm;
    }
    settle_room_key(&mut record);
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
            blueprint: None,
            phase: None,
            room_summary: None,
            size_mm: None,
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
