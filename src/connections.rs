//! Authored connections: the places a person has said rooms are joined that no
//! door says. Today that is one thing, an **open zone**.
//!
//! **An open zone is a set of rooms the user declares to be one open space.**
//! Every wall two of its members share is treated as open, so the rooms connect
//! to each other as a door would connect them (`service::connectivity` derives
//! the edges at read time from the wall-sharing geometry; nothing derived is
//! stored). It is a set and not a list of pairs because what a person knows is
//! "this whole area is open", not which of 47 wall segments inside it are.
//!
//! **Beside the project settings, as one JSON document per project**, under
//! `<projects_dir>/connections/<project>.json`, written the way settings are:
//! temp then rename, a copy of the replaced file under `.backups/`, one lock.
//! That is **provisional** — the storage type is decided after the whole
//! shortest-path plan is built (`docs/PLAN-connectivity.md`, "Where it lives") —
//! so the one thing that matters here is that this module is the only code that
//! reads or writes the file. The document carries a `schema_version` and a
//! `taken_at`, which is what makes a move to a versioned store a copy.
//!
//! **Replace-whole-list, with optimistic concurrency.** A save names the
//! `taken_at` of the document it read (`base`), and a mismatch is a conflict
//! rather than a silent overwrite: two people editing the same project is the
//! ordinary case for authored data, and the last writer winning unseen is the
//! failure that would make anyone distrust it.
//!
//! **A room is never validated against the model on save once it is a full
//! `RoomRef`**: authored data outlives the geometry it references, and a stale
//! member is a reported state at read time, never an error. A *bare* room id
//! (what the plan knows) is resolved here, and that is refused when it names no
//! room or names rooms in two models, because a silent guess would store a link
//! to the wrong room.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::service::room_locator::RoomRef;
use crate::service::rooms::{assemble_rooms, RoomScope};
use crate::service::ServiceError;
use crate::state::{is_path_safe_component, AppState};

pub const SCHEMA_VERSION: u32 = 1;

/// Hard limits, so a hostile or runaway save cannot make every later read
/// expensive. Far above any real project.
const MAX_ZONES: usize = 5_000;
const MAX_ROOMS_PER_ZONE: usize = 5_000;
const MAX_NOTE_CHARS: usize = 2_000;

/// What a zone means. One kind today; the lift and stair kind step 3 adds is a
/// second variant, not a second record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneKind {
    /// Shared walls between members are open.
    #[default]
    Open,
    /// A stack: members are open to each other across the walls they share on
    /// one storey (as `Open`), and joined between neighbouring storeys at a
    /// cost. A stair or a lift, with its landings or lobbies.
    Vertical,
}

/// What one storey change costs when a vertical zone does not say, in feet of
/// walking: a flight of stairs with its landing. A lift charged per storey
/// overstates its ride, which is why a zone can carry its own figure.
pub const DEFAULT_LEVEL_COST_FT: f64 = 40.0;

/// Upper bound on a stated level cost, so a typo cannot make a stair longer
/// than the building.
const MAX_LEVEL_COST_FT: f64 = 1_000.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenZone {
    /// Stable identity: survives a rename, and is what an edit addresses.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: ZoneKind,
    pub rooms: Vec<RoomRef>,
    /// Walking-equivalent feet per storey change, for a vertical zone. Absent
    /// means `DEFAULT_LEVEL_COST_FT`. Per zone and not a setting because a lift
    /// and a stair do not cost the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_cost_ft: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionsDocument {
    pub schema_version: u32,
    /// When this version was saved. Empty for a project nobody has authored
    /// anything for. It is the `base` the next save must name.
    #[serde(default)]
    pub taken_at: String,
    #[serde(default)]
    pub zones: Vec<OpenZone>,
}

impl ConnectionsDocument {
    pub fn empty() -> Self {
        Self { schema_version: SCHEMA_VERSION, taken_at: String::new(), zones: Vec::new() }
    }
}

/// A room as a caller names it: the model is optional, because the plan does
/// not know it.
#[derive(Debug, Clone, Deserialize)]
pub struct RoomInput {
    pub room_id: String,
    #[serde(default)]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ZoneInput {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: ZoneKind,
    pub rooms: Vec<RoomInput>,
    #[serde(default)]
    pub level_cost_ft: Option<f64>,
    #[serde(default)]
    pub note: Option<String>,
}

/// The body of a save.
#[derive(Debug, Clone, Deserialize)]
pub struct SaveRequest {
    /// The `taken_at` of the document the caller read; empty for "there was
    /// none".
    #[serde(default)]
    pub base: String,
    pub zones: Vec<ZoneInput>,
}

#[derive(Debug)]
pub enum ConnectionsError {
    /// The state wasn't built from a settings directory (in-memory tests).
    NotFileBacked,
    Invalid(String),
    /// The document moved since the caller read it.
    Conflict(String),
    Internal(anyhow::Error),
}

impl From<ServiceError> for ConnectionsError {
    fn from(e: ServiceError) -> Self {
        match e {
            ServiceError::Invalid(msg) => ConnectionsError::Invalid(msg),
            ServiceError::Internal(e) => ConnectionsError::Internal(e),
        }
    }
}

/// Serialises every save, so the read-compare-write below cannot interleave.
static SAVE_LOCK: Mutex<()> = Mutex::new(());

fn document_path(projects_dir: &Path, project: &str) -> Result<PathBuf, ConnectionsError> {
    if !is_path_safe_component(project) {
        return Err(ConnectionsError::Invalid(format!("project id {project:?} cannot be a file name")));
    }
    Ok(projects_dir.join("connections").join(format!("{project}.json")))
}

/// The project's connections, or the empty document when nobody has authored
/// any. A file that exists and cannot be read is an error, not "empty": silently
/// serving none would make a corrupt file look like a deletion.
pub fn load(projects_dir: &Path, project: &str) -> Result<ConnectionsDocument, ConnectionsError> {
    let path = document_path(projects_dir, project)?;
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            ConnectionsError::Internal(anyhow::anyhow!("{} is not a connections document: {e}", path.display()))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConnectionsDocument::empty()),
        Err(e) => Err(ConnectionsError::Internal(e.into())),
    }
}

/// The one rule set for what a saved document may hold. Pure, so it is testable
/// without a file.
pub fn validate(zones: &[OpenZone]) -> Result<(), ConnectionsError> {
    let bad = |msg: String| Err(ConnectionsError::Invalid(msg));
    if zones.len() > MAX_ZONES {
        return bad(format!("{} zones is more than the {MAX_ZONES} allowed", zones.len()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for zone in zones {
        if !is_path_safe_component(&zone.id) {
            return bad(format!("zone id {:?} must be a short name that is safe as a file name", zone.id));
        }
        if !seen.insert(zone.id.as_str()) {
            return bad(format!("zone id {:?} appears twice", zone.id));
        }
        if zone.name.trim().is_empty() {
            return bad(format!("zone {:?} needs a name", zone.id));
        }
        if zone.rooms.len() < 2 {
            return bad(format!("zone {:?} needs at least two rooms: one room alone connects nothing", zone.name));
        }
        if zone.rooms.len() > MAX_ROOMS_PER_ZONE {
            return bad(format!(
                "zone {:?} has {} rooms; the limit is {MAX_ROOMS_PER_ZONE}",
                zone.name,
                zone.rooms.len()
            ));
        }
        let mut members = std::collections::BTreeSet::new();
        for r in &zone.rooms {
            if r.model_id.trim().is_empty() || r.room_id.trim().is_empty() {
                return bad(format!("zone {:?} names a room with no model or no id", zone.name));
            }
            if !members.insert(r) {
                return bad(format!("zone {:?} lists room {} of model {} twice", zone.name, r.room_id, r.model_id));
            }
        }
        if let Some(cost) = zone.level_cost_ft {
            if zone.kind != ZoneKind::Vertical {
                return bad(format!("zone {:?} states a level cost but is not vertical", zone.name));
            }
            if !cost.is_finite() || cost <= 0.0 || cost > MAX_LEVEL_COST_FT {
                return bad(format!(
                    "zone {:?} level cost {cost} must be above 0 and at most {MAX_LEVEL_COST_FT} ft",
                    zone.name
                ));
            }
        }
        if zone.note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
            return bad(format!("the note on zone {:?} is longer than {MAX_NOTE_CHARS} characters", zone.name));
        }
    }
    Ok(())
}

/// Turn the rooms a caller named into full references. A room with a model is
/// taken as given; a bare id is looked up among the project's rooms and refused
/// when it names none or several.
pub fn resolve(state: &AppState, project: &str, input: Vec<ZoneInput>) -> Result<Vec<OpenZone>, ConnectionsError> {
    let needs_lookup = input.iter().any(|z| z.rooms.iter().any(|r| r.model_id.is_none()));
    let mut models_of: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    if needs_lookup
        && let Some(rooms) = assemble_rooms(state, &RoomScope { project: Some(project), ..Default::default() })?
    {
        for r in &rooms.rooms {
            models_of.entry(r.room.id.clone()).or_default().push(r.model_id.clone());
        }
    }
    input
        .into_iter()
        .map(|z| {
            let rooms = z
                .rooms
                .into_iter()
                .map(|r| match r.model_id.filter(|m| !m.trim().is_empty()) {
                    Some(model_id) => Ok(RoomRef { model_id, room_id: r.room_id }),
                    None => match models_of.get(&r.room_id).map(Vec::as_slice) {
                        Some([only]) => Ok(RoomRef { model_id: only.clone(), room_id: r.room_id }),
                        Some(many) if many.len() > 1 => Err(ConnectionsError::Invalid(format!(
                            "room id {:?} exists in {} models ({}); name the model",
                            r.room_id,
                            many.len(),
                            many.join(", ")
                        ))),
                        _ => Err(ConnectionsError::Invalid(format!(
                            "room {:?} is not among the project's rooms",
                            r.room_id
                        ))),
                    },
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(OpenZone {
                id: z.id,
                name: z.name.trim().to_string(),
                kind: z.kind,
                rooms,
                level_cost_ft: z.level_cost_ft,
                note: z.note,
            })
        })
        .collect()
}

/// Replace the project's document with `request`, if it still is the one the
/// caller read.
///
/// Order is load-bearing, as in `settings_api::save_project`: everything that
/// can refuse (conflict, validation) runs before anything is written, the old
/// file is copied aside while it is still in place, and the new one is installed
/// by rename so a reader never sees half a document.
pub fn save(
    state: &AppState,
    projects_dir: &Path,
    project: &str,
    request: SaveRequest,
) -> Result<ConnectionsDocument, ConnectionsError> {
    let path = document_path(projects_dir, project)?;
    // Resolve before the lock: it reads the model and can take seconds on a big
    // project, and nothing about it needs the file.
    let zones = resolve(state, project, request.zones)?;
    validate(&zones)?;

    let _guard = SAVE_LOCK.lock().unwrap();
    let current = load(projects_dir, project)?;
    if current.taken_at != request.base {
        return Err(ConnectionsError::Conflict(format!(
            "the connections changed since you read them (yours is from {:?}, the server has {:?}); reload and try again",
            request.base, current.taken_at
        )));
    }

    let mut taken_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    if taken_at <= current.taken_at {
        // A clock that did not move forward must not produce an id that sorts
        // before the one it replaces: the id is the version.
        taken_at = format!("{}~", current.taken_at);
    }
    let document = ConnectionsDocument { schema_version: SCHEMA_VERSION, taken_at, zones };

    let dir = path.parent().expect("a connections path always has a parent");
    std::fs::create_dir_all(dir).map_err(|e| ConnectionsError::Internal(e.into()))?;
    let json = serde_json::to_vec_pretty(&document).map_err(|e| ConnectionsError::Internal(e.into()))?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(|e| ConnectionsError::Internal(e.into()))?;

    let stem = format!("connections-{project}");
    if path.exists()
        && let Err(e) = crate::backups::back_up(projects_dir, &stem, "json", &path)
    {
        std::fs::remove_file(&temp).ok();
        return Err(ConnectionsError::Internal(e));
    }
    std::fs::rename(&temp, &path).map_err(|e| ConnectionsError::Internal(e.into()))?;
    crate::backups::prune(projects_dir, &stem, "json");
    tracing::info!("saved connections for project {project} ({} zones)", document.zones.len());
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemStore;
    use std::collections::HashMap;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-connections-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn state() -> AppState {
        AppState::new(Box::new(MemStore::new()), HashMap::new(), None)
    }

    fn room(model: &str, id: &str) -> RoomInput {
        RoomInput { room_id: id.to_string(), model_id: Some(model.to_string()) }
    }

    fn zone(id: &str, rooms: Vec<RoomInput>) -> ZoneInput {
        ZoneInput {
            id: id.to_string(),
            name: format!("Zone {id}"),
            kind: ZoneKind::Open,
            rooms,
            level_cost_ft: None,
            note: None,
        }
    }

    fn request(base: &str, zones: Vec<ZoneInput>) -> SaveRequest {
        SaveRequest { base: base.to_string(), zones }
    }

    #[test]
    fn test_a_project_with_nothing_authored_reads_as_empty() {
        let dir = temp_dir("empty");
        let doc = load(&dir, "p1").unwrap();
        assert!(doc.zones.is_empty());
        assert_eq!(doc.taken_at, "");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_save_then_load_round_trips_and_stamps_a_version() {
        let dir = temp_dir("round-trip");
        let saved =
            save(&state(), &dir, "p1", request("", vec![zone("z1", vec![room("m", "a"), room("m", "b")])])).unwrap();
        assert!(!saved.taken_at.is_empty());
        let read = load(&dir, "p1").unwrap();
        assert_eq!(read, saved);
        assert_eq!(read.zones[0].rooms.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two people editing from the same read: the second is told, and nothing of
    /// the first is lost.
    #[test]
    fn test_a_stale_base_is_a_conflict_not_an_overwrite() {
        let dir = temp_dir("conflict");
        let first =
            save(&state(), &dir, "p1", request("", vec![zone("z1", vec![room("m", "a"), room("m", "b")])])).unwrap();
        let stale = save(&state(), &dir, "p1", request("", vec![zone("z2", vec![room("m", "c"), room("m", "d")])]));
        assert!(matches!(stale, Err(ConnectionsError::Conflict(_))));
        assert_eq!(load(&dir, "p1").unwrap(), first, "the earlier save is untouched");
        // And naming the version it read works.
        let second = save(
            &state(),
            &dir,
            "p1",
            request(&first.taken_at, vec![zone("z2", vec![room("m", "c"), room("m", "d")])]),
        )
        .unwrap();
        assert!(second.taken_at > first.taken_at);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_replaced_document_is_kept_under_backups() {
        let dir = temp_dir("backup");
        let first =
            save(&state(), &dir, "p1", request("", vec![zone("z1", vec![room("m", "a"), room("m", "b")])])).unwrap();
        assert!(!dir.join(crate::backups::BACKUP_DIR).exists(), "a first save has nothing to keep");
        save(&state(), &dir, "p1", request(&first.taken_at, vec![])).unwrap();
        let kept: Vec<_> = std::fs::read_dir(dir.join(crate::backups::BACKUP_DIR)).unwrap().collect();
        assert_eq!(kept.len(), 1);
        let name = kept[0].as_ref().unwrap().file_name().to_string_lossy().into_owned();
        assert!(name.starts_with("connections-p1.") && name.ends_with(".json"), "{name}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_validation_refuses_what_could_not_be_a_zone() {
        let ok = |z: ZoneInput| resolve(&state(), "p1", vec![z]).and_then(|zs| validate(&zs));
        assert!(ok(zone("z1", vec![room("m", "a"), room("m", "b")])).is_ok());
        assert!(ok(zone("z1", vec![room("m", "a")])).is_err(), "one room connects nothing");
        assert!(ok(zone("z1", vec![room("m", "a"), room("m", "a")])).is_err(), "a room twice");
        assert!(
            ok(zone("../x", vec![room("m", "a"), room("m", "b")])).is_err(),
            "an id that is not a name"
        );
        let mut unnamed = zone("z1", vec![room("m", "a"), room("m", "b")]);
        unnamed.name = "  ".to_string();
        assert!(ok(unnamed).is_err());
        let both = vec![
            zone("z1", vec![room("m", "a"), room("m", "b")]),
            zone("z1", vec![room("m", "c"), room("m", "d")]),
        ];
        assert!(validate(&resolve(&state(), "p1", both).unwrap()).is_err(), "a duplicate id");
    }

    /// A bare room id the project has never heard of is refused rather than
    /// stored as a link to nothing.
    #[test]
    fn test_a_bare_id_naming_no_room_is_refused() {
        let bare = RoomInput { room_id: "ghost".to_string(), model_id: None };
        let err = resolve(&state(), "p1", vec![zone("z1", vec![bare, room("m", "b")])]).unwrap_err();
        assert!(matches!(err, ConnectionsError::Invalid(m) if m.contains("ghost")));
    }

    #[test]
    fn test_a_path_unsafe_project_id_is_refused() {
        let dir = temp_dir("unsafe");
        assert!(matches!(load(&dir, "../etc"), Err(ConnectionsError::Invalid(_))));
        std::fs::remove_dir_all(&dir).ok();
    }
}
