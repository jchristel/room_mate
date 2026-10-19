//! Which blueprint is placed in which room: the record behind the assign view.
//!
//! **A reference and a choice, never a result.** An assignment names the blueprint, the
//! revision it pins, the room, and the edge mapping a person settled on (a cyclic shift, or
//! their own list). The placements are NOT stored: they are the resolver's answer from the
//! pinned revision and the room as it stands today, worked out on every read, so a room that
//! moves is followed and nothing stored can disagree with the model
//! (`docs/PLAN-blueprints.md`, decision 1).
//!
//! **One assignment per room.** A room has one layout at a time, so assigning a room again
//! replaces its assignment (same id, new blueprint, revision or mapping) rather than adding a
//! second that would have to be argued with.
//!
//! **The revision is pinned**, so a blueprint edited later changes nothing here until a person
//! re-assigns; a read says `newer_revision` when the blueprint has moved on. Nothing is
//! validated against the model on save: authored data outlives the geometry it names, and a
//! room that has gone is a reported state on the read.
//!
//! One JSON file per assignment under `<projects_dir>/assignments/<project>/<id>.json`, apart
//! from `blueprints/` because the blueprint listing reads every `*.json` it finds there.
//! Written as the others are: temp, then rename, a copy of the file replaced under `.backups/`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::blueprints::{self, BlueprintError};
use crate::state::is_path_safe_component;

pub const SCHEMA_VERSION: u32 = 1;
const MAX_ASSIGNMENTS_PER_PROJECT: usize = 20_000;
const MAX_EDGES: usize = 2_000;
const MAX_NOTE_CHARS: usize = 2_000;

static SAVE_LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The room, qualified by its model because a room id is unique only within one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomKey {
    pub model_id: String,
    pub room_id: String,
}

/// How the blueprint's edges were put on the room's. At most one of the two is set; neither
/// means "the best fit by shape and doors, worked out on every read".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapping {
    /// Blueprint edge `i` to room edge `(i + shift) % edges`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<usize>,
    /// The room edge for each blueprint edge, chosen by a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_map: Option<Vec<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub schema_version: u32,
    pub id: String,
    pub blueprint_id: String,
    /// The revision pinned. New assignments take the blueprint's current one.
    pub revision: u32,
    pub room: RoomKey,
    /// The room's name when it was assigned: for display when the room has since gone.
    pub room_name: String,
    #[serde(default)]
    pub mapping: Mapping,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub taken_at: String,
}

/// What a person sends: the blueprint, optionally the revision (the current one if left out),
/// the room, and the mapping.
#[derive(Debug, Clone, Deserialize)]
pub struct AssignRequest {
    pub blueprint_id: String,
    #[serde(default)]
    pub revision: Option<u32>,
    pub room: RoomKey,
    #[serde(default)]
    pub room_name: String,
    #[serde(default)]
    pub mapping: Mapping,
    #[serde(default)]
    pub note: Option<String>,
}

/// An assignment and how it stands against its blueprint today.
#[derive(Debug, Clone, Serialize)]
pub struct AssignmentRow {
    #[serde(flatten)]
    pub assignment: Assignment,
    /// Absent when the blueprint has been deleted: a reported state.
    pub blueprint_name: Option<String>,
    pub current_revision: Option<u32>,
    /// The blueprint's current revision is later than the one pinned.
    pub newer_revision: bool,
    pub blueprint_missing: bool,
}

fn bad<T>(msg: String) -> Result<T, BlueprintError> {
    Err(BlueprintError::Invalid(msg))
}

fn internal<E: Into<anyhow::Error>>(e: E) -> BlueprintError {
    BlueprintError::Internal(e.into())
}

fn project_dir(projects_dir: &Path, project: &str) -> Result<PathBuf, BlueprintError> {
    if !is_path_safe_component(project) {
        return bad(format!("project id {project:?} cannot be a file name"));
    }
    Ok(projects_dir.join("assignments").join(project))
}

fn path_of(projects_dir: &Path, project: &str, id: &str) -> Result<PathBuf, BlueprintError> {
    if !is_path_safe_component(id) {
        return bad(format!("assignment id {id:?} cannot be a file name"));
    }
    Ok(project_dir(projects_dir, project)?.join(format!("{id}.json")))
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

fn read(path: &Path) -> Result<Assignment, BlueprintError> {
    let bytes = std::fs::read(path).map_err(internal)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| BlueprintError::Internal(anyhow::anyhow!("{} is not an assignment: {e}", path.display())))
}

/// Every assignment of the project, by room name. A file that cannot be read is an error and
/// not a gap, as for blueprints.
pub fn list_raw(projects_dir: &Path, project: &str) -> Result<Vec<Assignment>, BlueprintError> {
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
    out.sort_by(|a, b| a.room_name.to_lowercase().cmp(&b.room_name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    Ok(out)
}

/// The assignments with where each stands against its blueprint, for the list read.
pub fn list(projects_dir: &Path, project: &str) -> Result<Vec<AssignmentRow>, BlueprintError> {
    list_raw(projects_dir, project)?
        .into_iter()
        .map(|assignment| {
            let blueprint = match blueprints::load(projects_dir, project, &assignment.blueprint_id) {
                Ok(b) => Some(b),
                Err(BlueprintError::NotFound(_)) => None,
                Err(e) => return Err(e),
            };
            Ok(AssignmentRow {
                blueprint_name: blueprint.as_ref().map(|b| b.name.clone()),
                current_revision: blueprint.as_ref().map(|b| b.current_revision),
                newer_revision: blueprint.as_ref().is_some_and(|b| b.current_revision > assignment.revision),
                blueprint_missing: blueprint.is_none(),
                assignment,
            })
        })
        .collect()
}

fn clean_mapping(mapping: Mapping) -> Result<Mapping, BlueprintError> {
    if mapping.shift.is_some() && mapping.edge_map.is_some() {
        return bad("a mapping is a shift or an edge map, not both".to_string());
    }
    if mapping.edge_map.as_ref().is_some_and(|m| m.len() > MAX_EDGES) {
        return bad(format!("an edge map names at most {MAX_EDGES} edges"));
    }
    Ok(mapping)
}

/// Assign a blueprint to a room, replacing the room's existing assignment if it has one.
pub fn assign(projects_dir: &Path, project: &str, request: AssignRequest) -> Result<Assignment, BlueprintError> {
    if request.room.room_id.trim().is_empty() || request.room.model_id.trim().is_empty() {
        return bad("an assignment needs a room and its model".to_string());
    }
    let mapping = clean_mapping(request.mapping)?;
    let note = request.note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    if note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
        return bad(format!("a note is at most {MAX_NOTE_CHARS} characters"));
    }
    let blueprint = blueprints::load(projects_dir, project, &request.blueprint_id)?;
    let revision = request.revision.unwrap_or(blueprint.current_revision);
    if blueprint.revision(revision).is_none() {
        return Err(BlueprintError::NotFound(format!("the blueprint has no revision {revision}")));
    }
    let _guard = SAVE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let existing = list_raw(projects_dir, project)?;
    let replaced = existing.iter().find(|a| a.room == request.room);
    if replaced.is_none() && existing.len() >= MAX_ASSIGNMENTS_PER_PROJECT {
        return bad(format!("a project holds at most {MAX_ASSIGNMENTS_PER_PROJECT} assignments"));
    }
    let id = match replaced {
        Some(a) => a.id.clone(),
        None => new_id(&project_dir(projects_dir, project)?),
    };
    let assignment = Assignment {
        schema_version: SCHEMA_VERSION,
        id,
        blueprint_id: blueprint.id.clone(),
        revision,
        room: request.room,
        room_name: request.room_name,
        mapping,
        note,
        taken_at: now(),
    };
    write(projects_dir, project, &assignment)?;
    Ok(assignment)
}

pub fn delete(projects_dir: &Path, project: &str, id: &str) -> Result<(), BlueprintError> {
    let path = path_of(projects_dir, project, id)?;
    let _guard = SAVE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !path.exists() {
        return Err(BlueprintError::NotFound(format!("no assignment {id:?} in project {project:?}")));
    }
    let stem = format!("assignment-{project}-{id}");
    crate::backups::back_up(projects_dir, &stem, "json", &path).map_err(BlueprintError::Internal)?;
    std::fs::remove_file(&path).map_err(internal)?;
    crate::backups::prune(projects_dir, &stem, "json");
    Ok(())
}

fn new_id(dir: &Path) -> String {
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let stamp = chrono::Utc::now().timestamp_micros();
        let id = format!("a{stamp:x}{n:x}");
        if !dir.join(format!("{id}.json")).exists() {
            return id;
        }
    }
}

fn write(projects_dir: &Path, project: &str, assignment: &Assignment) -> Result<(), BlueprintError> {
    let path = path_of(projects_dir, project, &assignment.id)?;
    let dir = path.parent().expect("an assignment path always has a parent");
    std::fs::create_dir_all(dir).map_err(internal)?;
    let json = serde_json::to_vec_pretty(assignment).map_err(internal)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(internal)?;
    let stem = format!("assignment-{project}-{}", assignment.id);
    if path.exists()
        && let Err(e) = crate::backups::back_up(projects_dir, &stem, "json", &path)
    {
        std::fs::remove_file(&temp).ok();
        return Err(BlueprintError::Internal(e));
    }
    std::fs::rename(&temp, &path).map_err(internal)?;
    crate::backups::prune(projects_dir, &stem, "json");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blueprints::CreateRequest;
    use crate::service::blueprint_model::{Outline, Point, RevisionContent};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-assignments-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn content() -> RevisionContent {
        let p = |x, y| Point { x, y };
        let ring = vec![
            p(-1000.0, -1000.0),
            p(1000.0, -1000.0),
            p(1000.0, 1000.0),
            p(-1000.0, 1000.0),
        ];
        RevisionContent {
            outline: Outline { coarse: ring.clone(), fine: ring },
            items: vec![],
            doors: vec![],
        }
    }

    fn blueprint(dir: &Path, name: &str) -> String {
        blueprints::create(
            dir,
            "p",
            CreateRequest { name: name.to_string(), note: None, source: None, content: content() },
        )
        .unwrap()
        .id
    }

    fn request(blueprint_id: &str, room: &str) -> AssignRequest {
        AssignRequest {
            blueprint_id: blueprint_id.to_string(),
            revision: None,
            room: RoomKey { model_id: "m".to_string(), room_id: room.to_string() },
            room_name: format!("Room {room}"),
            mapping: Mapping::default(),
            note: None,
        }
    }

    #[test]
    fn test_assigning_a_room_again_replaces_its_assignment_and_keeps_its_id() {
        let tmp = temp_dir(&format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let (a, b) = (blueprint(&tmp, "A"), blueprint(&tmp, "B"));
        let first = assign(&tmp, "p", request(&a, "r1")).unwrap();
        let again = assign(&tmp, "p", request(&b, "r1")).unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(list_raw(&tmp, "p").unwrap().len(), 1);
        assert_eq!(list_raw(&tmp, "p").unwrap()[0].blueprint_id, b);
        assign(&tmp, "p", request(&a, "r2")).unwrap();
        assert_eq!(list_raw(&tmp, "p").unwrap().len(), 2);
    }

    #[test]
    fn test_the_pinned_revision_stays_put_and_the_read_says_a_newer_one_exists() {
        let tmp = temp_dir(&format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let a = blueprint(&tmp, "A");
        let made = assign(&tmp, "p", request(&a, "r1")).unwrap();
        assert_eq!(made.revision, 1);
        let loaded = blueprints::load(&tmp, "p", &a).unwrap();
        blueprints::add_revision(
            &tmp,
            "p",
            &a,
            blueprints::AddRevisionRequest {
                base: loaded.taken_at,
                note: None,
                content: content(),
                make_current: true,
            },
        )
        .unwrap();
        let rows = list(&tmp, "p").unwrap();
        assert_eq!(rows[0].assignment.revision, 1);
        assert!(rows[0].newer_revision);
        assert_eq!(rows[0].current_revision, Some(2));
    }

    #[test]
    fn test_a_deleted_blueprint_is_a_reported_state_not_a_failed_read() {
        let tmp = temp_dir(&format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let a = blueprint(&tmp, "A");
        assign(&tmp, "p", request(&a, "r1")).unwrap();
        let taken = blueprints::load(&tmp, "p", &a).unwrap().taken_at;
        blueprints::delete(&tmp, "p", &a, &taken).unwrap();
        let rows = list(&tmp, "p").unwrap();
        assert!(rows[0].blueprint_missing && rows[0].blueprint_name.is_none() && !rows[0].newer_revision);
    }

    #[test]
    fn test_a_mapping_is_a_shift_or_an_edge_map_and_an_unknown_blueprint_or_revision_is_refused() {
        let tmp = temp_dir(&format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let a = blueprint(&tmp, "A");
        let mut both = request(&a, "r1");
        both.mapping = Mapping { shift: Some(1), edge_map: Some(vec![0, 1, 2, 3]) };
        assert!(matches!(assign(&tmp, "p", both), Err(BlueprintError::Invalid(_))));
        assert!(matches!(assign(&tmp, "p", request("nope", "r1")), Err(BlueprintError::NotFound(_))));
        let mut late = request(&a, "r1");
        late.revision = Some(9);
        assert!(matches!(assign(&tmp, "p", late), Err(BlueprintError::NotFound(_))));
    }

    #[test]
    fn test_delete_removes_the_assignment_and_a_second_delete_is_not_found() {
        let tmp = temp_dir(&format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let a = blueprint(&tmp, "A");
        let made = assign(&tmp, "p", request(&a, "r1")).unwrap();
        delete(&tmp, "p", &made.id).unwrap();
        assert!(list_raw(&tmp, "p").unwrap().is_empty());
        assert!(matches!(delete(&tmp, "p", &made.id), Err(BlueprintError::NotFound(_))));
    }
}
