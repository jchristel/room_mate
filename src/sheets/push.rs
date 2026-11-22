//! A push from Revit and what a person decides about it: checking each pushed sheet, matching it to the library by identity
//! (update, new, or a renumbered sheet stashed pending), and `accept` / `reject` / `delete`.

use super::*;

pub(super) fn check_text(label: &str, s: &str, required: bool) -> Result<String, SheetError> {
    let s = s.trim();
    if required && s.is_empty() {
        return bad(format!("a sheet needs a {label}"));
    }
    if s.chars().count() > MAX_TEXT_CHARS {
        return bad(format!("a sheet's {label} is at most {MAX_TEXT_CHARS} characters"));
    }
    Ok(s.to_string())
}

pub(super) fn check_room(room: &Option<SheetRoom>) -> Result<(), SheetError> {
    if let Some(r) = room
        && (r.model_id.trim().is_empty() || r.room_id.trim().is_empty())
    {
        return bad("a sheet's room needs a model id and a room id".to_string());
    }
    Ok(())
}

/// A room outline is a ring or nothing: fewer than three points, a coordinate that is not a number, or more points
/// than any room has is a broken export, refused rather than stored to fail a task later.
pub(super) fn check_outline(number: &str, outline: &[[f64; 2]]) -> Result<(), SheetError> {
    if outline.is_empty() {
        return Ok(());
    }
    if outline.len() < 3 || outline.len() > MAX_OUTLINE_POINTS {
        return bad(format!(
            "sheet {number:?}: a room outline has 3 to {MAX_OUTLINE_POINTS} points, not {}",
            outline.len()
        ));
    }
    if outline.iter().flatten().any(|v| !v.is_finite()) {
        return bad(format!("sheet {number:?}: a room outline coordinate is not a number"));
    }
    Ok(())
}

pub(super) fn check_pushed(p: &PushedSheet) -> Result<PushedSheet, SheetError> {
    check_room(&p.room)?;
    check_outline(&p.sheet_number, &p.room_outline)?;
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
        room_outline: p.room_outline.clone(),
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
pub(super) fn settle_room_key(record: &mut SheetRecord) {
    record.room_key = match (&record.blueprint, record.room_summary) {
        (None, Some(summary)) => Some(room_key(&summary)),
        _ => None,
    };
}

/// A re-push with the same sheet number: the sheet's data replaces the record's. What the
/// extractor measured (room figures, size) and the phase replace what was there only when this
/// push sent them; a room link, an explicit choice made in Revit, sets only when named.
pub(super) fn refresh_record(record: &mut SheetRecord, p: PushedSheet, received_at: String) {
    record.taken_at = next_stamp(&record.taken_at);
    record.sheet_name = p.sheet_name;
    record.sheet = p.sheet;
    record.received_at = received_at;
    // The outline goes with the figures: a push that measured the room(s) again says what they are now, and a sheet
    // that went from one room to several loses its outline rather than keeping the old room's.
    if p.room.is_some() {
        record.room = p.room;
        record.room_summary = p.room_summary;
        record.room_outline = p.room_outline.clone();
    }
    if p.blueprint.is_some() {
        record.blueprint = p.blueprint;
    }
    if p.phase.is_some() {
        record.phase = p.phase;
    }
    if p.room_summary.is_some() {
        record.room_summary = p.room_summary;
        record.room_outline = p.room_outline;
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
                    room_outline: p.room_outline,
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
                    room_outline: p.room_outline,
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

pub(super) fn outcome_of(r: &SheetRecord, outcome: Outcome) -> PushOutcome {
    PushOutcome {
        id: r.id.clone(),
        revit_sheet_id: r.revit_sheet_id.clone(),
        document_title: r.document_title.clone(),
        sheet_number: r.sheet_number.clone(),
        outcome,
    }
}

pub(super) fn take_pending(record: &mut SheetRecord, pending_id: u32) -> Result<PendingSheet, SheetError> {
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
        record.room_outline = accepted.room_outline.clone();
    }
    if accepted.blueprint.is_some() {
        record.blueprint = accepted.blueprint;
    }
    if accepted.phase.is_some() {
        record.phase = accepted.phase;
    }
    if accepted.room_summary.is_some() {
        record.room_summary = accepted.room_summary;
        record.room_outline = accepted.room_outline;
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
