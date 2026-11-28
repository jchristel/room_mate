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
const MAX_OVERRIDES: usize = 2_000;
const MAX_ADDITIONS: usize = 200;
/// An addition's recipe is a distance along and into a room; past this it is not a place in any room (mm).
pub const MAX_ADDITION_MM: i32 = 100_000;
/// What every addition's id begins with. An element id is a number, so an addition can never collide with a
/// blueprint item.
pub const ADDITION_PREFIX: &str = "add-";
/// A nudge further than this is a different place, not a nudge (mm).
pub const MAX_NUDGE_MM: i32 = 20_000;

static SAVE_LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The room, qualified by its model because a room id is unique only within one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomKey {
    pub model_id: String,
    pub room_id: String,
}

/// A box a person laid over a room by hand: the blueprint's own outline, moved, turned and
/// stretched until it matches a room whose outline is not quite the blueprint's. **The box is the
/// room reference the recipes are resolved against** (plan decision 6): items keep their size, a
/// fixed distance keeps its millimetres, a percentage stretches with the box. The room's own outline
/// is then only what findings are judged against.
///
/// `world = centre + R(rotate_deg) * S(sx, sy) * (p - c0)`, where `c0` is the vertex mean of the
/// blueprint's coarse outline (the pivot, so a stretch and a turn do not move the box), `S` stretches
/// along the BLUEPRINT's own axes (so they follow the box round when it is turned), and
/// `centre = room vertex mean + (dx_mm, dy_mm)`. Held relative to the room, so a room that moves is
/// followed. No mirroring: both scales are positive (decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub dx_mm: f64,
    pub dy_mm: f64,
    /// Counter-clockwise, degrees.
    pub rotate_deg: f64,
    pub sx: f64,
    pub sy: f64,
}

/// A stretch outside this is a different room, not a stretch.
pub const MIN_SCALE: f64 = 0.2;
pub const MAX_SCALE: f64 = 5.0;
const MAX_FRAME_MOVE_MM: f64 = 200_000.0;

impl Frame {
    /// The box as it is kept: finite, in range, the turn in (-180, 180].
    pub fn checked(self) -> Result<Frame, String> {
        let all = [self.dx_mm, self.dy_mm, self.rotate_deg, self.sx, self.sy];
        if all.iter().any(|v| !v.is_finite()) {
            return Err("a box has finite numbers".to_string());
        }
        if self.dx_mm.abs() > MAX_FRAME_MOVE_MM || self.dy_mm.abs() > MAX_FRAME_MOVE_MM {
            return Err(format!("a box sits within {MAX_FRAME_MOVE_MM} mm of its room"));
        }
        if !(MIN_SCALE..=MAX_SCALE).contains(&self.sx) || !(MIN_SCALE..=MAX_SCALE).contains(&self.sy) {
            return Err(format!(
                "a box stretches between {MIN_SCALE} and {MAX_SCALE} times; that is a different room"
            ));
        }
        let mut r = self.rotate_deg % 360.0;
        if r > 180.0 {
            r -= 360.0;
        } else if r <= -180.0 {
            r += 360.0;
        }
        Ok(Frame { rotate_deg: r, ..self })
    }

    /// Where blueprint point `p` lands: `c0` is the blueprint outline's pivot and `centre` where the
    /// pivot goes.
    pub fn place(&self, p: (f64, f64), c0: (f64, f64), centre: (f64, f64)) -> (f64, f64) {
        let (x, y) = ((p.0 - c0.0) * self.sx, (p.1 - c0.1) * self.sy);
        let (s, c) = self.rotate_deg.to_radians().sin_cos();
        (centre.0 + x * c - y * s, centre.1 + x * s + y * c)
    }
}

/// How the blueprint's edges were put on the room's. At most one of the three is set; none
/// means "the best fit by shape and doors, worked out on every read".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Mapping {
    /// A box laid over the room by hand (see `Frame`); the blueprint's edges map to the box's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
    /// Blueprint edge `i` to room edge `(i + shift) % edges`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<usize>,
    /// The room edge for each blueprint edge, chosen by a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_map: Option<Vec<usize>>,
}

/// A person's change to ONE blueprint item in ONE room, kept with the assignment: leave it out, or
/// move and turn it from where the resolver put it.
///
/// **Applied to the resolver's answer, never to the blueprint**, so the blueprint stays the intent
/// and the change is this room's alone. **Whole millimetres and degrees**, which is what a person
/// types and what lets an assignment stay `Eq`. **One per item** (a second would have to be argued
/// with), and an item that is not in the pinned revision is reported by the resolver, not refused
/// here: authored data outlives what it names. **Affects its own item only**: a follower keeps the
/// place the resolver gave it, and can be overridden itself.
///
/// **A nudge has two spellings, and the item-frame one is the one to write.** `along_mm` and `off_mm` are in the
/// item's own frame: along the edge it sits against and into the room from it, so "away from the wall" stays
/// "away from the wall" when the room is turned or the mapping changes. `dx_mm` and `dy_mm` are in the project's
/// plan frame (where the placement is drawn), which is what a drag on the plan gives and what every override stored
/// before the item frame existed holds; they do NOT follow a rotation, and changing the mapping afterwards can
/// leave one that no longer fits. Both may be set and both apply.
///
/// **`base` is what a nudge or turn was made against**: the hash of the item's placement recipe in the revision
/// the override was saved under (`BlueprintItem::recipe_hash`), stamped by the server on save when absent. When a
/// new revision moves the item the hash differs and the nudge is HELD and reported (`override_stale`) rather than
/// riding silently on a place it was never made for. An exclude does not depend on where the item is, so it applies
/// regardless. An override with no `base` (made before it existed) is not checked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemOverride {
    pub item_id: String,
    /// Do not place this item in this room.
    #[serde(default, skip_serializing_if = "is_false")]
    pub exclude: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dx_mm: i32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dy_mm: i32,
    /// Along the edge the item sits against, from the edge's start towards its end (item frame).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub along_mm: i32,
    /// Into the room from that edge, along its inward normal (item frame).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub off_mm: i32,
    /// The recipe this was made against; see above. Left out by a client, filled in by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Counter-clockwise, about the item's own centre.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotate_deg: i32,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

impl ItemOverride {
    /// Whether it changes anything.
    pub fn is_noop(&self) -> bool {
        !self.exclude
            && self.dx_mm == 0
            && self.dy_mm == 0
            && self.along_mm == 0
            && self.off_mm == 0
            && self.rotate_deg == 0
    }

    /// Whether it moves or turns the item (as opposed to leaving it out), which is what a changed recipe strands.
    pub fn moves(&self) -> bool {
        self.dx_mm != 0 || self.dy_mm != 0 || self.along_mm != 0 || self.off_mm != 0 || self.rotate_deg != 0
    }
}

/// One more item in THIS room only: a clone of a blueprint item at a place the person chose on the room's own edge.
/// The corner-guard case: the blueprint has two, this room has a column corner and needs a third, and everything else
/// is fine.
///
/// **Room-local and never written back to the blueprint**: the blueprint stays the standard and the room owns its
/// exception. **A clone by reference** (`clone_of`), so when the blueprint changes the guard's type the extra guard
/// changes with it; an addition that stood alone would have to be edited in every room. **The recipe is on the ROOM's
/// coarse outline** (`edge` is the room's own edge, not the blueprint's), in fixed millimetres, because a column corner
/// is a jog the coarse outline removed and the person points at the wall that is there. Whole numbers, so an
/// assignment stays `Eq`. `id` begins with `add-`, which an element id (a number) never does, so it cannot collide with
/// a blueprint item; the resolver reports an addition whose item has gone (`addition_unused`), never an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemAddition {
    pub id: String,
    /// The blueprint item it is a copy of: its type, size, class and properties.
    pub clone_of: String,
    /// The room's coarse edge it stands against.
    pub edge: usize,
    /// From the edge's start along it (mm).
    pub along_mm: i32,
    /// Into the room from the edge (mm).
    pub off_mm: i32,
    /// From the edge's inward normal, counter-clockwise (degrees).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub facing_deg: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// This room's changes to single items (see `ItemOverride`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<ItemOverride>,
    /// Items this room has that the blueprint does not (see `ItemAddition`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additions: Vec<ItemAddition>,
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
    /// `None` keeps the room's existing overrides (a client written before they existed must not
    /// clear them by assigning again); `Some(vec![])` clears them.
    #[serde(default)]
    pub overrides: Option<Vec<ItemOverride>>,
    /// Same rule as `overrides`: `None` keeps the room's, `Some(vec![])` clears them.
    #[serde(default)]
    pub additions: Option<Vec<ItemAddition>>,
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
    if [
        mapping.shift.is_some(),
        mapping.edge_map.is_some(),
        mapping.frame.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count()
        > 1
    {
        return bad("a mapping is a shift, an edge map or a box, not more than one".to_string());
    }
    let mapping = match mapping.frame {
        Some(f) => Mapping { frame: Some(f.checked().map_err(BlueprintError::Invalid)?), ..mapping },
        None => mapping,
    };
    if mapping.edge_map.as_ref().is_some_and(|m| m.len() > MAX_EDGES) {
        return bad(format!("an edge map names at most {MAX_EDGES} edges"));
    }
    Ok(mapping)
}

/// The overrides as they are kept: one per item, none that change nothing, none out of range.
pub fn clean_overrides(overrides: Vec<ItemOverride>) -> Result<Vec<ItemOverride>, BlueprintError> {
    if overrides.len() > MAX_OVERRIDES {
        return bad(format!("an assignment holds at most {MAX_OVERRIDES} item overrides"));
    }
    let mut out: Vec<ItemOverride> = Vec::new();
    for o in overrides {
        if o.item_id.trim().is_empty() {
            return bad("an item override names an item".to_string());
        }
        if o.dx_mm.abs() > MAX_NUDGE_MM || o.dy_mm.abs() > MAX_NUDGE_MM {
            return bad(format!(
                "item {:?}: a nudge is at most {MAX_NUDGE_MM} mm; that is a different place",
                o.item_id
            ));
        }
        if o.rotate_deg.abs() > 360 {
            return bad(format!("item {:?}: a rotation is at most 360 degrees either way", o.item_id));
        }
        if out.iter().any(|x| x.item_id == o.item_id) {
            return bad(format!("item {:?} has more than one override; an item has one", o.item_id));
        }
        if !o.is_noop() {
            out.push(o);
        }
    }
    Ok(out)
}

/// The additions as they are kept: unique ids that begin `add-`, each cloning something, none out of range.
pub fn clean_additions(additions: Vec<ItemAddition>) -> Result<Vec<ItemAddition>, BlueprintError> {
    if additions.len() > MAX_ADDITIONS {
        return bad(format!("an assignment holds at most {MAX_ADDITIONS} additions"));
    }
    let mut out: Vec<ItemAddition> = Vec::new();
    for a in additions {
        if !a.id.starts_with(ADDITION_PREFIX) || a.id.len() == ADDITION_PREFIX.len() {
            return bad(format!("an addition's id begins {ADDITION_PREFIX:?} and goes on (got {:?})", a.id));
        }
        if a.clone_of.trim().is_empty() {
            return bad(format!("addition {:?} says which blueprint item it is a copy of", a.id));
        }
        if a.along_mm.abs() > MAX_ADDITION_MM || a.off_mm.abs() > MAX_ADDITION_MM {
            return bad(format!("addition {:?}: a distance is at most {MAX_ADDITION_MM} mm", a.id));
        }
        if a.facing_deg.abs() > 360 {
            return bad(format!("addition {:?}: a facing is at most 360 degrees either way", a.id));
        }
        if out.iter().any(|x| x.id == a.id) {
            return bad(format!("two additions are called {:?}", a.id));
        }
        out.push(a);
    }
    Ok(out)
}

/// Stamp each override that has no `base` with the recipe hash of its item in the revision being pinned. One whose
/// item is not in that revision keeps none: there is nothing to have been made against.
fn stamp_bases(
    overrides: Vec<ItemOverride>,
    items: &[crate::service::blueprint_model::BlueprintItem],
) -> Vec<ItemOverride> {
    overrides
        .into_iter()
        .map(|mut o| {
            if o.base.is_none() {
                o.base = items.iter().find(|i| i.id == o.item_id).map(|i| i.recipe_hash());
            }
            o
        })
        .collect()
}

/// Assign a blueprint to a room, replacing the room's existing assignment if it has one.
pub fn assign(projects_dir: &Path, project: &str, request: AssignRequest) -> Result<Assignment, BlueprintError> {
    if request.room.room_id.trim().is_empty() || request.room.model_id.trim().is_empty() {
        return bad("an assignment needs a room and its model".to_string());
    }
    let mapping = clean_mapping(request.mapping)?;
    let overrides = request.overrides.map(clean_overrides).transpose()?;
    let additions = request.additions.map(clean_additions).transpose()?;
    let note = request.note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    if note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
        return bad(format!("a note is at most {MAX_NOTE_CHARS} characters"));
    }
    let blueprint = blueprints::load(projects_dir, project, &request.blueprint_id)?;
    if !blueprint.in_use {
        return bad(format!("blueprint {:?} is not in use, so it takes no new assignment", blueprint.name));
    }
    let revision = request.revision.unwrap_or(blueprint.current_revision);
    let Some(pinned) = blueprint.revision(revision) else {
        return Err(BlueprintError::NotFound(format!("the blueprint has no revision {revision}")));
    };
    // Made against the revision being pinned now. The ones KEPT from the room's existing assignment (`None` below)
    // keep the base they were made with, which is how a new revision's moved recipe shows up as stale.
    let overrides = overrides.map(|o| stamp_bases(o, &pinned.content.items));
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
        overrides: match overrides {
            Some(o) => o,
            None => replaced.map(|a| a.overrides.clone()).unwrap_or_default(),
        },
        additions: match additions {
            Some(a) => a,
            None => replaced.map(|a| a.additions.clone()).unwrap_or_default(),
        },
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
            CreateRequest {
                name: name.to_string(),
                note: None,
                source: None,
                bundle: None,
                content: content(),
            },
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
            overrides: None,
            additions: None,
            note: None,
        }
    }

    /// A blueprint holding one item, so an override has something to have been made against.
    fn blueprint_with_a_guard(dir: &Path) -> (String, String) {
        use crate::service::blueprint_capture::{capture, SourceItem};
        let ring = vec![(0.0, 0.0), (4000.0, 0.0), (4000.0, 3000.0), (0.0, 3000.0)];
        let guard = SourceItem {
            id: "guard".to_string(),
            family_name: None,
            type_name: "Corner guard".to_string(),
            category: "Generic Models".to_string(),
            footprint: vec![(100.0, 100.0), (300.0, 100.0), (300.0, 300.0), (100.0, 300.0)],
            facing: Some((1.0, 0.0)),
            nested: false,
            hosting: None,
            kit: None,
            properties: vec![],
            type_properties: vec![],
        };
        let content = capture(&ring, vec![guard]).unwrap().content;
        let hash = content.items[0].recipe_hash();
        let id = blueprints::create(
            dir,
            "p",
            CreateRequest { name: "G".to_string(), note: None, source: None, bundle: None, content },
        )
        .unwrap()
        .id;
        (id, hash)
    }

    fn nudge_of(item: &str) -> ItemOverride {
        ItemOverride { item_id: item.to_string(), along_mm: 40, ..Default::default() }
    }

    #[test]
    fn test_an_override_is_stamped_with_the_recipe_it_was_made_against_and_keeps_that_when_kept() {
        let tmp = temp_dir(&format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let (bp, hash) = blueprint_with_a_guard(&tmp);
        let mut req = request(&bp, "r1");
        req.overrides = Some(vec![nudge_of("guard"), nudge_of("ghost")]);
        let made = assign(&tmp, "p", req).unwrap();
        let by = |id: &str| made.overrides.iter().find(|o| o.item_id == id).unwrap();
        assert_eq!(by("guard").base.as_deref(), Some(hash.as_str()), "stamped by the server, not the client");
        assert_eq!(
            by("ghost").base,
            None,
            "an item the revision does not have has nothing to have been made against"
        );

        // A client that sends it back with its base keeps it; one that sends none is stamped afresh.
        let mut again = request(&bp, "r1");
        again.overrides = Some(vec![ItemOverride { base: Some("old".into()), ..nudge_of("guard") }]);
        assert_eq!(
            assign(&tmp, "p", again).unwrap().overrides[0].base.as_deref(),
            Some("old"),
            "a base is never overwritten"
        );
        // Assigning again WITHOUT overrides keeps the room's, base and all: how a new revision shows a stale one.
        assert_eq!(assign(&tmp, "p", request(&bp, "r1")).unwrap().overrides[0].base.as_deref(), Some("old"));
    }

    #[test]
    fn test_additions_are_kept_cleaned_and_survive_assigning_again_without_them() {
        let tmp = temp_dir(&format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let (bp, _) = blueprint_with_a_guard(&tmp);
        let one = |id: &str| ItemAddition {
            id: id.to_string(),
            clone_of: "guard".to_string(),
            edge: 1,
            along_mm: 0,
            off_mm: 150,
            facing_deg: 0,
        };
        let mut req = request(&bp, "r1");
        req.additions = Some(vec![one("add-1")]);
        assert_eq!(assign(&tmp, "p", req).unwrap().additions, vec![one("add-1")]);
        assert_eq!(
            assign(&tmp, "p", request(&bp, "r1")).unwrap().additions.len(),
            1,
            "a client that knows nothing of them keeps them"
        );
        let mut clear = request(&bp, "r1");
        clear.additions = Some(vec![]);
        assert!(assign(&tmp, "p", clear).unwrap().additions.is_empty(), "an empty list clears them");

        for (bad, why) in [
            (vec![one("guard-2")], "an id that does not begin add-"),
            (vec![one("add-")], "an id that is only the prefix"),
            (vec![one("add-1"), one("add-1")], "two with one id"),
            (vec![ItemAddition { clone_of: " ".to_string(), ..one("add-1") }], "no item to copy"),
            (
                vec![ItemAddition { off_mm: 200_000, ..one("add-1") }],
                "a distance that is not a place in a room",
            ),
            (vec![ItemAddition { facing_deg: 400, ..one("add-1") }], "a facing past a full turn"),
        ] {
            let mut req = request(&bp, "r2");
            req.additions = Some(bad);
            assert!(assign(&tmp, "p", req).is_err(), "{why} is refused");
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
        blueprints::remove_for_test(&tmp, "p", &a);
        let rows = list(&tmp, "p").unwrap();
        assert!(rows[0].blueprint_missing && rows[0].blueprint_name.is_none() && !rows[0].newer_revision);
    }

    #[test]
    fn test_a_mapping_is_a_shift_or_an_edge_map_and_an_unknown_blueprint_or_revision_is_refused() {
        let tmp = temp_dir(&format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let a = blueprint(&tmp, "A");
        let mut both = request(&a, "r1");
        both.mapping = Mapping { shift: Some(1), edge_map: Some(vec![0, 1, 2, 3]), frame: None };
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

    fn nudge(id: &str, dx: i32) -> ItemOverride {
        ItemOverride { item_id: id.to_string(), dx_mm: dx, ..Default::default() }
    }

    #[test]
    fn test_overrides_are_kept_one_per_item_and_a_noop_is_dropped() {
        let tmp = temp_dir(&format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let a = blueprint(&tmp, "A");
        let mut req = request(&a, "r1");
        req.overrides = Some(vec![nudge("bed", 50), nudge("chair", 0)]);
        let made = assign(&tmp, "p", req).unwrap();
        assert_eq!(made.overrides, vec![nudge("bed", 50)], "a no-op change is not kept");

        let mut twice = request(&a, "r2");
        twice.overrides = Some(vec![nudge("bed", 1), nudge("bed", 2)]);
        assert!(
            matches!(assign(&tmp, "p", twice), Err(BlueprintError::Invalid(_))),
            "an item has one override"
        );

        let mut far = request(&a, "r3");
        far.overrides = Some(vec![nudge("bed", MAX_NUDGE_MM + 1)]);
        assert!(matches!(assign(&tmp, "p", far), Err(BlueprintError::Invalid(_))));
    }

    #[test]
    fn test_assigning_again_without_overrides_keeps_them_and_an_empty_list_clears_them() {
        let tmp = temp_dir(&format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let a = blueprint(&tmp, "A");
        let mut first = request(&a, "r1");
        first.overrides = Some(vec![nudge("bed", 50)]);
        assign(&tmp, "p", first).unwrap();

        let kept = assign(&tmp, "p", request(&a, "r1")).unwrap();
        assert_eq!(
            kept.overrides,
            vec![nudge("bed", 50)],
            "a client that does not send them must not clear them"
        );

        let mut clear = request(&a, "r1");
        clear.overrides = Some(Vec::new());
        assert!(assign(&tmp, "p", clear).unwrap().overrides.is_empty());
    }

    #[test]
    fn test_a_box_is_kept_checked_and_exclusive_of_the_other_mappings() {
        let tmp = temp_dir(&format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let a = blueprint(&tmp, "A");
        let boxed = Frame { dx_mm: 120.0, dy_mm: -40.0, rotate_deg: 270.0, sx: 1.2, sy: 0.9 };

        let mut req = request(&a, "r1");
        req.mapping = Mapping { frame: Some(boxed), ..Mapping::default() };
        let made = assign(&tmp, "p", req).unwrap();
        let kept = made.mapping.frame.expect("the box is kept");
        assert!((kept.rotate_deg + 90.0).abs() < 1e-9, "a turn is wrapped: {}", kept.rotate_deg);
        assert_eq!(list_raw(&tmp, "p").unwrap()[0].mapping.frame, Some(kept), "and it reads back");

        let mut both = request(&a, "r2");
        both.mapping = Mapping { frame: Some(boxed), shift: Some(1), edge_map: None };
        assert!(
            matches!(assign(&tmp, "p", both), Err(BlueprintError::Invalid(_))),
            "a box and a shift are two answers"
        );

        let mut wild = request(&a, "r3");
        wild.mapping = Mapping { frame: Some(Frame { sx: 9.0, ..boxed }), ..Mapping::default() };
        assert!(matches!(assign(&tmp, "p", wild), Err(BlueprintError::Invalid(_))));
    }
}
