//! Tag rules: where a tag goes relative to the element it tags, learned from tags in Revit views
//! (plan decision 24, and its "tag data" paragraph).
//!
//! **Observations in, one rule per key out.** The extractor sends the tags it read from a sheet's
//! viewports (duHast's `DataTag`, as exported) as observations. Each tagged element of each tag
//! becomes a *sample* of the rule for its key: the tagged element's category, its **type code** (the
//! value of the project's `tag_type_code` type parameter, read by the extractor) and that
//! parameter's name, the tag's category, family name and type name, and the viewport type. **Never
//! family and type names of the tagged item, and never Revit ids**: a blueprint places new elements
//! with new ids, and names differ between models and get renamed (plan decision 27). Each
//! rule stores its samples and the figures worked out from them, so the same blueprint always tags
//! the same way: the median of the samples, recomputed whenever a sample is added or replaced.
//!
//! **A sample is a placement relative to the element, on paper.** The tag head, elbow and leader
//! end are taken from the element's location point into the element's own frame (x along its
//! facing, y to its left), and divided by the view scale, so millimetres on the sheet. A rule
//! therefore holds at any scale and turns with the item. The tag's rotation is stored relative to
//! the element's facing. An element with no plan facing gives no sample: a guessed direction is
//! worse than none.
//!
//! **Room, space and area tags are rules too, for the room.** The tagged element is the room (or space
//! or area), which has no type code and no facing, so such a rule is keyed by the category
//! alone (`Rooms`, code and parameter empty). Its place is written against the room's BOUNDARY
//! (`Boundary`, worked out by `service::room_tag_place` from the room the tag sat in); a tag whose room
//! was not on the server keeps only its offset in the plan's own axes, +x then +y. A task carries
//! it for every viewport of its type, whatever items it places, because it is about the room and
//! not about an item.
//!
//! **A rule is generic or bound to a room blueprint** (`RuleKey::blueprint`, and for an item's tag the
//! blueprint item, `instance`; plan decision 27). `ObserveContext` carries what the store cannot know
//! (the blueprint's items, the rooms) and the API fills it in (`tag_rules_api::observe_for`). What
//! moves rules and samples between the two scopes (a sample's move, export, import, unlinking) is
//! `tag_rule_binding`, split from here to keep this module to the rule itself.
//!
//! **A sample is identified by where it was read** (document, sheet, tag, tagged element), so
//! pushing the same sheet again replaces its samples and never counts them twice, **wherever they now
//! live**: a sample is in exactly one rule.
//!
//! One JSON file per rule under `<projects_dir>/tagrules/<project>/<id>.json`, written the way
//! sheets are (temp, rename, a copy under `.backups/`, one lock). This module and `tag_rule_binding`
//! are the only code that reads or writes them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::state::is_path_safe_component;

/// 2: rules are keyed by the item's type code (plan decision 27). A version-1 file, keyed by family
/// and type names, is not loaded: those rules come back by pushing the sheets again.
pub const SCHEMA_VERSION: u32 = 2;

pub(crate) const MAX_RULES_PER_PROJECT: usize = 20_000;
const MAX_OBSERVATIONS_PER_PUSH: usize = 5_000;
pub(crate) const MAX_SAMPLES_PER_RULE: usize = 500;
const MAX_TEXT_CHARS: usize = 300;

// ---- what the extractor sends -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct P3 {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub z: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Matrix {
    #[serde(default)]
    pub data: Vec<Vec<f64>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Location {
    #[serde(default)]
    pub translation_coord: P3,
    #[serde(default)]
    pub rotation_coord: Option<Matrix>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TypeNames {
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub family_name: String,
    #[serde(default)]
    pub type_name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TaggedElement {
    #[serde(default)]
    pub host_element_id: i64,
    #[serde(default)]
    pub link_element_id: i64,
    #[serde(flatten)]
    pub names: TypeNames,
    /// The value of the project's type-code parameter on the element's type, read by the
    /// extractor. Absent or blank: the element cannot be keyed and its tag is skipped.
    #[serde(default)]
    pub type_code: Option<String>,
    #[serde(default)]
    pub location_point: Option<Location>,
    /// The middle of the element's bounding box, model mm, read by the extractor. An elevation sample is measured
    /// from it when present: a family that sets its height with a parameter (a towel rail at 900, 1200 and 1500 mm)
    /// keeps one insertion point at the level for every height, so offsets from that point put every tag in one place.
    #[serde(default)]
    pub centre: Option<P3>,
    #[serde(default)]
    pub elbow_location: Option<P3>,
    #[serde(default)]
    pub leader_end: Option<P3>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ObservedTag {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub tag_kind: String,
    #[serde(default)]
    pub tag_type: TypeNames,
    #[serde(default)]
    pub rotation: f64,
    #[serde(default)]
    pub orientation: String,
    #[serde(default)]
    pub has_leader: bool,
    #[serde(default)]
    pub leader_end_condition: String,
    /// The tag head, mm, in the model's coordinates.
    #[serde(default)]
    pub point: P3,
    #[serde(default)]
    pub tagged_elements: Vec<TaggedElement>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceSheet {
    #[serde(default)]
    pub document_title: String,
    #[serde(default)]
    pub revit_sheet_id: String,
    #[serde(default)]
    pub sheet_number: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Observation {
    #[serde(default)]
    pub viewport_type: Option<String>,
    #[serde(default)]
    pub view_id: Option<i64>,
    #[serde(default)]
    pub view_scale: Option<f64>,
    /// The direction an ELEVATION looks along (model frame, a unit vector or any length), which is what its
    /// sample is measured against. Absent for a plan; an elevation without it gives no sample.
    #[serde(default)]
    pub view_direction: Option<P3>,
    #[serde(default)]
    pub sheet: SourceSheet,
    pub tag: ObservedTag,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObservationsRequest {
    /// The room blueprint these tags belong to (the id the blueprint push answered with), or absent for
    /// a generic push. A tag on an item of that blueprint, and a room tag, is then bound to it
    /// (plan decision 27); a tag on any other item is generic.
    #[serde(default)]
    pub blueprint: Option<String>,
    /// The type parameter `type_code` values were read from (the project's `tag_type_code`). A
    /// rule is keyed by it too, so codes from two parameters never meet. Blank: no element of an
    /// item can be keyed, and only room, space and area tags are taken in.
    #[serde(default)]
    pub type_code_parameter: String,
    pub observations: Vec<Observation>,
}

// ---- what is stored ---------------------------------------------------------------------------

/// What a rule is keyed by (plan decision 27). An item's tag is found by the **type code** of the
/// item (the value of the project's type-code parameter, shared by every instance of the type),
/// never by family and type names, which differ between models and get renamed. A room, space or
/// area tag has no code and no parameter: its key is the category alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuleKey {
    pub tagged_category: String,
    /// The type parameter the code was read from; empty for a room, space or area tag.
    #[serde(default)]
    pub code_parameter: String,
    /// The tagged item's type code; empty for a room, space or area tag.
    #[serde(default)]
    pub type_code: String,
    pub tag_category: String,
    pub tag_family: String,
    pub tag_type: String,
    pub viewport_type: String,
    /// The room blueprint the rule belongs to; empty for a generic rule (plan decision 27).
    #[serde(default)]
    pub blueprint: String,
    /// The blueprint item the rule is for (its id, the Revit element id of the item it was captured
    /// from), so a rule holds one sample; empty for a generic rule and for a room tag.
    #[serde(default)]
    pub instance: String,
    /// Elevations only: whether the item was seen `front` on (its facing parallel to the view direction) or from the
    /// `side`. A plan has none (empty). It keys the rule because the tag sits somewhere else on an item seen front
    /// on than on one seen edge on, and it is kept out of the id while empty so older rules keep theirs.
    #[serde(default)]
    pub view_side: String,
}

impl RuleKey {
    fn canonical(&self) -> String {
        [
            &self.tagged_category,
            &self.code_parameter,
            &self.type_code,
            &self.tag_category,
            &self.tag_family,
            &self.tag_type,
            &self.viewport_type,
            &self.blueprint,
            &self.instance,
        ]
        .iter()
        .map(|s| s.trim().to_lowercase())
        .collect::<Vec<_>>()
        .join("\u{1f}")
            + &if self.view_side.trim().is_empty() {
                String::new()
            } else {
                format!("\u{1f}{}", self.view_side.trim().to_lowercase())
            }
    }

    /// A file name for the key: stable (FNV-1a, never the std hasher, which may change between
    /// releases) and safe.
    pub fn id(&self) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in self.canonical().bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("tr-{h:016x}")
    }
}

/// What a room tag is written against (see `service::room_tag_place`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryOf {
    /// The coarse outline of the room the sheet was made for (a rule that belongs to a blueprint).
    Wall,
    /// The smallest rectangle round the room (a generic rule).
    Rectangle,
}

/// A room tag head written against the room's boundary: unitless along, paper mm off.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Boundary {
    pub of: BoundaryOf,
    /// Which edge of the outline or side of the rectangle (see `of`).
    pub edge: usize,
    /// How far along that edge, as a fraction of its length.
    pub along: f64,
    /// How far into the room, paper millimetres.
    pub off: f64,
}

/// Where one sample was read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleSource {
    pub sheet: SourceSheet,
    pub tag_id: i64,
    /// The tagged element's id in the source model: provenance and replacement only.
    pub element_id: i64,
}

/// Paper millimetres in the element's frame: x along its facing, y to its left.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Offset {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub source: SampleSource,
    pub view_scale: f64,
    pub head: Offset,
    /// Radians, relative to the element's facing, in (-pi, pi].
    pub rotation: f64,
    pub orientation: String,
    pub has_leader: bool,
    pub leader_end_condition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elbow: Option<Offset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Offset>,
    /// For a room tag whose room was on the server: the head written against the room's boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary: Option<Boundary>,
    /// Elevations: the offsets are from the element's bounding-box centre, not its insertion point. A sample stored
    /// before the centre was read is from the point, and the two cannot be converted into each other.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_centre: bool,
}

/// The one answer for a key: what a task applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub head: Offset,
    pub rotation: f64,
    pub orientation: String,
    pub has_leader: bool,
    pub leader_end_condition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elbow: Option<Offset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Offset>,
    /// A room tag's head against the room's boundary, when its samples had one: that, and not
    /// `head`, is what is applied, so the tag follows the room's walls and not the plan's axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary: Option<Boundary>,
    /// Elevations: the offsets are from the item's bounding-box centre (see [`Sample::from_centre`]), so the
    /// plug-in must add them to the centre of the item it tags and not to its insertion point.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_centre: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagRule {
    pub schema_version: u32,
    pub id: String,
    pub key: RuleKey,
    /// `independent`, `room`, `space` or `area`: which kind of Revit tag to create.
    pub tag_kind: String,
    pub placement: Placement,
    pub sample_count: usize,
    pub updated_at: String,
    #[serde(default)]
    pub samples: Vec<Sample>,
}

/// A rule without its samples: what a list shows and a task carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSummary {
    pub id: String,
    pub key: RuleKey,
    pub tag_kind: String,
    pub placement: Placement,
    pub sample_count: usize,
    pub updated_at: String,
}

impl From<&TagRule> for RuleSummary {
    fn from(r: &TagRule) -> Self {
        RuleSummary {
            id: r.id.clone(),
            key: r.key.clone(),
            tag_kind: r.tag_kind.clone(),
            placement: r.placement.clone(),
            sample_count: r.sample_count,
            updated_at: r.updated_at.clone(),
        }
    }
}

/// A rule as a TASK carries it: where the tag goes, never which tag. The model's RLS settings name the room tag and
/// the independent tag the plug-in uses, so a tag family in the task would be a second answer to that question, and
/// one that is wrong whenever the settings name another tag. The rule store still keys by the tag it learned from
/// (that is what the samples measured); the task drops it. A task stored with the whole key still reads: serde
/// ignores the fields this leaves out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRule {
    pub id: String,
    pub key: TaskRuleKey,
    pub tag_kind: String,
    pub placement: Placement,
    pub sample_count: usize,
    #[serde(default)]
    pub updated_at: String,
}

/// [`RuleKey`] without the tag's category, family and type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRuleKey {
    pub tagged_category: String,
    #[serde(default)]
    pub code_parameter: String,
    #[serde(default)]
    pub type_code: String,
    pub viewport_type: String,
    #[serde(default)]
    pub blueprint: String,
    #[serde(default)]
    pub view_side: String,
}

impl From<&RuleSummary> for TaskRule {
    fn from(r: &RuleSummary) -> Self {
        TaskRule {
            id: r.id.clone(),
            key: TaskRuleKey {
                tagged_category: r.key.tagged_category.clone(),
                code_parameter: r.key.code_parameter.clone(),
                type_code: r.key.type_code.clone(),
                viewport_type: r.key.viewport_type.clone(),
                blueprint: r.key.blueprint.clone(),
                view_side: r.key.view_side.clone(),
            },
            tag_kind: r.tag_kind.clone(),
            placement: r.placement.clone(),
            sample_count: r.sample_count,
            updated_at: r.updated_at.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Skipped {
    pub tag_id: i64,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ObservationOutcome {
    /// Samples taken in (new or replacing).
    pub samples: usize,
    pub rules_changed: usize,
    /// The rules this push wrote (not serialised): what a blueprint's reconcile keeps.
    #[serde(skip)]
    pub rule_ids: Vec<String>,
    pub skipped: Vec<Skipped>,
}

#[derive(Debug)]
pub enum TagRuleError {
    NotFileBacked,
    Invalid(String),
    NotFound(String),
    Internal(anyhow::Error),
}

pub(crate) static SAVE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn bad<T>(msg: String) -> Result<T, TagRuleError> {
    Err(TagRuleError::Invalid(msg))
}

fn internal<E: Into<anyhow::Error>>(e: E) -> TagRuleError {
    TagRuleError::Internal(e.into())
}

fn project_dir(projects_dir: &Path, project: &str) -> Result<PathBuf, TagRuleError> {
    if !is_path_safe_component(project) {
        return bad(format!("project id {project:?} cannot be a file name"));
    }
    Ok(projects_dir.join("tagrules").join(project))
}

fn rule_path(projects_dir: &Path, project: &str, id: &str) -> Result<PathBuf, TagRuleError> {
    if !is_path_safe_component(id) {
        return bad(format!("rule id {id:?} cannot be a file name"));
    }
    Ok(project_dir(projects_dir, project)?.join(format!("{id}.json")))
}

fn read(path: &Path) -> Result<TagRule, TagRuleError> {
    let bytes = std::fs::read(path).map_err(internal)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| TagRuleError::Internal(anyhow::anyhow!("{} is not a tag rule: {e}", path.display())))
}

/// Every rule of the project, by key. A file that cannot be read is an error, not a gap; a file of
/// an older schema is left on disk and not loaded, and said once in the log.
pub fn load_all(projects_dir: &Path, project: &str) -> Result<Vec<TagRule>, TagRuleError> {
    let dir = project_dir(projects_dir, project)?;
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(internal(e)),
    };
    let mut out = Vec::new();
    let mut old = 0usize;
    for entry in entries {
        let path = entry.map_err(internal)?.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let rule = read(&path)?;
        if rule.schema_version < SCHEMA_VERSION {
            old += 1;
        } else {
            out.push(rule);
        }
    }
    if old > 0 {
        tracing::warn!("{old} tag rule(s) of project {project:?} are keyed by family and type names and are not used; push the sheets again to learn them by type code");
    }
    out.sort_by_key(|r| r.key.canonical());
    Ok(out)
}

pub fn list(projects_dir: &Path, project: &str) -> Result<Vec<RuleSummary>, TagRuleError> {
    Ok(load_all(projects_dir, project)?.iter().map(RuleSummary::from).collect())
}

pub fn load(projects_dir: &Path, project: &str, id: &str) -> Result<TagRule, TagRuleError> {
    let path = rule_path(projects_dir, project, id)?;
    match std::fs::read(&path) {
        Ok(_) => read(&path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(TagRuleError::NotFound(format!("no tag rule {id:?} in project {project:?}")))
        }
        Err(e) => Err(internal(e)),
    }
}

pub(crate) fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

pub(crate) fn write(projects_dir: &Path, project: &str, rule: &TagRule) -> Result<(), TagRuleError> {
    let path = rule_path(projects_dir, project, &rule.id)?;
    let dir = path.parent().expect("a rule path always has a parent");
    std::fs::create_dir_all(dir).map_err(internal)?;
    let json = serde_json::to_vec_pretty(rule).map_err(internal)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(internal)?;
    let stem = format!("tagrule-{project}-{}", rule.id);
    if path.exists()
        && let Err(e) = crate::backups::back_up(projects_dir, &stem, "json", &path)
    {
        std::fs::remove_file(&temp).ok();
        return Err(TagRuleError::Internal(e));
    }
    std::fs::rename(&temp, &path).map_err(internal)?;
    crate::backups::prune(projects_dir, &stem, "json");
    Ok(())
}

pub fn delete(projects_dir: &Path, project: &str, id: &str) -> Result<(), TagRuleError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    remove(projects_dir, project, id)
}

/// `delete` for a caller already holding the lock.
pub(crate) fn remove(projects_dir: &Path, project: &str, id: &str) -> Result<(), TagRuleError> {
    let path = rule_path(projects_dir, project, id)?;
    if !path.exists() {
        return Err(TagRuleError::NotFound(format!("no tag rule {id:?} in project {project:?}")));
    }
    let stem = format!("tagrule-{project}-{id}");
    crate::backups::back_up(projects_dir, &stem, "json", &path).map_err(TagRuleError::Internal)?;
    std::fs::remove_file(&path).map_err(internal)
}

// ---- from an observation to samples -----------------------------------------------------------

fn clean(s: &str) -> String {
    s.trim().chars().take(MAX_TEXT_CHARS).collect()
}

/// Whether a tag kind tags a room, space or area: the tagged element then has no family, type or facing.
pub fn is_spatial(tag_kind: &str) -> bool {
    matches!(tag_kind.trim().to_lowercase().as_str(), "room" | "space" | "area")
}

/// The element's facing in plan: its local X, projected. `None` when it has no plan component.
fn facing_of(loc: &Location) -> Option<(f64, f64)> {
    let row = loc.rotation_coord.as_ref()?.data.first()?;
    let (x, y) = (*row.first()?, *row.get(1)?);
    let len = x.hypot(y);
    (len > 1e-6).then(|| (x / len, y / len))
}

/// A model-space point as an offset from the element, in its frame, in paper mm.
fn to_offset(p: P3, from: P3, facing: (f64, f64), scale: f64) -> Offset {
    let (dx, dy) = (p.x - from.x, p.y - from.y);
    Offset {
        x: (dx * facing.0 + dy * facing.1) / scale,
        y: (-dx * facing.1 + dy * facing.0) / scale,
    }
}

/// How square-on an item's facing must be to the direction a view looks for the item to count as seen front on:
/// the cosine of about 25 degrees. Anything less is `side`, so an item at 45 degrees is read as seen from the side.
pub const FRONT_ON: f64 = 0.9;

/// Whether a viewport type is an elevation (an elevation's offsets are along the view, not the plan).
pub fn is_elevation(viewport_type: &str) -> bool {
    viewport_type.trim().eq_ignore_ascii_case("elevation")
}

/// A unit horizontal direction, or `None` for a vector with no plan part (a view looking straight down).
pub fn plan_direction(x: f64, y: f64) -> Option<(f64, f64)> {
    let len = x.hypot(y);
    (len > 1e-6).then(|| (x / len, y / len))
}

/// `front` when the item's plan facing is parallel (or opposite) to the direction the view looks, else `side`.
pub fn view_side(facing: (f64, f64), looks: (f64, f64)) -> &'static str {
    if (facing.0 * looks.0 + facing.1 * looks.1).abs() >= FRONT_ON {
        "front"
    } else {
        "side"
    }
}

/// The viewer's right in plan for a view looking along `looks` (an elevation is vertical, so up is +z).
pub fn view_right(looks: (f64, f64)) -> (f64, f64) {
    (looks.1, -looks.0)
}

/// A model-space point as an offset from the element in an ELEVATION: x along the viewer's right, y up, paper mm.
fn to_view_offset(p: P3, from: P3, looks: (f64, f64), scale: f64) -> Offset {
    let right = view_right(looks);
    Offset {
        x: ((p.x - from.x) * right.0 + (p.y - from.y) * right.1) / scale,
        y: (p.z - from.z) / scale,
    }
}

fn wrap(angle: f64) -> f64 {
    let two_pi = std::f64::consts::TAU;
    let mut a = angle % two_pi;
    if a > std::f64::consts::PI {
        a -= two_pi;
    } else if a <= -std::f64::consts::PI {
        a += two_pi;
    }
    a
}

/// What the server knows that the observations do not say (plan decision 27). The store has none of
/// it, so the API layer fills it in; the default is "no blueprint, no rooms on hand".
#[derive(Default)]
pub struct ObserveContext<'a> {
    /// The room blueprint the push is for, with the ids of the items it holds: a tag on one of those
    /// is bound to it (and to that item), a tag on any other item is generic.
    pub blueprint: Option<(String, std::collections::HashSet<String>)>,
    /// For a push that names no blueprint (the Sheets button): the blueprint each SHEET is already bound to,
    /// by `(document title, Revit sheet id)`, and the items of each such blueprint. A sheet bound to a
    /// blueprint keeps its tags bound whichever button pushed them; without this the second push would
    /// move the sheet's tag samples out of the blueprint's rules into the generic ones while the sheet
    /// stayed bound. An explicit `blueprint` wins over this.
    pub sheet_blueprints: std::collections::HashMap<(String, String), String>,
    pub blueprint_items: std::collections::HashMap<String, std::collections::HashSet<String>>,
    /// A room tag head written against its room's boundary: `(model id, room id, head in the model's
    /// frame, mm, bound, view scale)`. `None`: the room is not on the server, and the sample keeps
    /// only its offset in the plan's axes.
    #[allow(clippy::type_complexity)]
    pub boundary: Option<&'a dyn Fn(&str, i64, P3, bool, f64) -> Option<Boundary>>,
}

impl ObserveContext<'_> {
    /// The blueprint a sheet's tags belong to and its items: the push's own blueprint if it names one,
    /// else the one the sheet is already bound to. `None`: the tags are generic.
    fn scope_of(&self, sheet: &SourceSheet) -> Option<(&str, &std::collections::HashSet<String>)> {
        if let Some((id, items)) = &self.blueprint {
            return Some((id, items));
        }
        let id = self.sheet_blueprints.get(&(sheet.document_title.clone(), sheet.revit_sheet_id.clone()))?;
        Some((id, self.blueprint_items.get(id)?))
    }
}

/// `(key, kind, sample)` for each tagged element of the observation that can be used, and a
/// reason for the ones that cannot.
fn samples_of(
    obs: &Observation,
    parameter: &str,
    ctx: &ObserveContext,
) -> Vec<Result<(RuleKey, String, Sample), String>> {
    let tag = &obs.tag;
    let Some(viewport_type) = obs.viewport_type.as_deref().map(clean).filter(|v| !v.is_empty()) else {
        return vec![Err("the observation names no viewport type".to_string())];
    };
    let Some(scale) = obs.view_scale.filter(|s| *s > 0.0) else {
        return vec![Err("the view's scale was not read".to_string())];
    };
    if tag.tag_type.family_name.trim().is_empty() {
        return vec![Err("the tag's family name was not read".to_string())];
    }
    if tag.tagged_elements.is_empty() {
        return vec![Err("the tag refers to no element".to_string())];
    }
    let spatial = is_spatial(&tag.tag_kind);
    tag.tagged_elements
        .iter()
        .map(|el| {
            if spatial {
                if el.names.category.trim().is_empty() {
                    return Err(format!("tagged element {}: its category was not read", el.host_element_id));
                }
            } else if parameter.is_empty() {
                return Err(format!(
                    "tagged element {}: the push names no type-code parameter (set `tag_type_code` in the project's settings)",
                    el.host_element_id
                ));
            } else if el.type_code.as_deref().map(clean).is_none_or(|c| c.is_empty()) {
                return Err(format!(
                    "tagged element {}: its type has no value for the type-code parameter {parameter:?}",
                    el.host_element_id
                ));
            }
            let loc = el
                .location_point
                .as_ref()
                .ok_or_else(|| format!("tagged element {}: it has no point location", el.host_element_id))?;
            // An item in an elevation is measured along the view (right, up) and keyed by how it is seen.
            let elevation = !spatial && is_elevation(&viewport_type);
            let looks = if elevation {
                let d = obs.view_direction.unwrap_or_default();
                Some(plan_direction(d.x, d.y).ok_or_else(|| {
                    format!("tagged element {}: the elevation's view direction was not read", el.host_element_id)
                })?)
            } else {
                None
            };
            // A room has no orientation of its own: its tag is placed in the plan's axes.
            let facing = if spatial {
                (1.0, 0.0)
            } else {
                facing_of(loc).ok_or_else(|| format!("tagged element {}: it has no plan facing", el.host_element_id))?
            };
            // An elevation is measured from the item's middle when the extractor read it, because the insertion point
            // need not be at the item's height (a plan keeps the point: its offsets turn with the item's facing).
            let centre = el.centre.filter(|_| looks.is_some());
            let from = centre.unwrap_or(loc.translation_coord);
            let element_id = if el.link_element_id > 0 { el.link_element_id } else { el.host_element_id };
            // Bound to the push's blueprint: a room tag always, an item's tag only when the item is one of
            // the blueprint's (its id is the Revit element id, which the blueprint kept).
            let (blueprint, instance) = match ctx.scope_of(&obs.sheet) {
                Some((id, _)) if spatial => (id.to_string(), String::new()),
                Some((id, items)) if items.contains(&element_id.to_string()) => (id.to_string(), element_id.to_string()),
                _ => (String::new(), String::new()),
            };
            let key = RuleKey {
                tagged_category: clean(&el.names.category),
                code_parameter: if spatial { String::new() } else { clean(parameter) },
                type_code: if spatial { String::new() } else { el.type_code.as_deref().map(clean).unwrap_or_default() },
                tag_category: clean(&tag.tag_type.category),
                tag_family: clean(&tag.tag_type.family_name),
                tag_type: clean(&tag.tag_type.type_name),
                viewport_type: viewport_type.clone(),
                blueprint: blueprint.clone(),
                instance,
                view_side: looks.map(|l| view_side(facing, l).to_string()).unwrap_or_default(),
            };
            let offset = |p: P3| match looks {
                Some(l) => to_view_offset(p, from, l, scale),
                None => to_offset(p, from, facing, scale),
            };
            let boundary = if spatial {
                ctx.boundary.and_then(|f| f(&obs.sheet.document_title, element_id, tag.point, !blueprint.is_empty(), scale))
            } else {
                None
            };
            let sample = Sample {
                source: SampleSource { sheet: obs.sheet.clone(), tag_id: tag.id, element_id },
                view_scale: scale,
                head: offset(tag.point),
                // in an elevation the tag's turn is about the view's normal, which the item's facing says nothing of
                rotation: if elevation { wrap(tag.rotation) } else { wrap(tag.rotation - facing.1.atan2(facing.0)) },
                orientation: clean(&tag.orientation),
                has_leader: tag.has_leader,
                leader_end_condition: clean(&tag.leader_end_condition),
                elbow: el.elbow_location.filter(|_| tag.has_leader).map(offset),
                end: el.leader_end.filter(|_| tag.has_leader).map(offset),
                boundary,
                from_centre: centre.is_some(),
            };
            let kind = if tag.tag_kind.trim().is_empty() {
                "independent".to_string()
            } else {
                clean(&tag.tag_kind)
            };
            Ok((key, kind, sample))
        })
        .collect()
}

// ---- from samples to the one rule -------------------------------------------------------------

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn median_offset(it: impl Iterator<Item = Offset>) -> Option<Offset> {
    let all: Vec<Offset> = it.collect();
    (!all.is_empty()).then(|| Offset {
        x: median(all.iter().map(|o| o.x).collect()),
        y: median(all.iter().map(|o| o.y).collect()),
    })
}

/// The most common value, the first seen winning a tie, so the answer does not move with order.
fn mode(values: impl Iterator<Item = String>) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for v in values {
        match counts.iter_mut().find(|(s, _)| *s == v) {
            Some(c) => c.1 += 1,
            None => counts.push((v, 1)),
        }
    }
    let best = counts.iter().map(|c| c.1).max().unwrap_or(0);
    counts.into_iter().find(|c| c.1 == best).map(|c| c.0).unwrap_or_default()
}

/// The single placement a rule's samples agree on. Median per component; a rotation is the
/// median of angles taken around the first (so a spread across +-pi does not average to nothing).
/// Samples measured from an item's centre and from its insertion point are offsets from two different places, so
/// they are never mixed: when any sample is from the centre only those count, and the rule is from the centre. A
/// rule whose samples are all older stays from the point until its sheets are pushed again.
pub fn reduce(samples: &[Sample]) -> Placement {
    let centred: Vec<Sample> = samples.iter().filter(|s| s.from_centre).cloned().collect();
    let from_centre = !centred.is_empty();
    let samples = if from_centre { &centred[..] } else { samples };
    let anchor = samples.first().map_or(0.0, |s| s.rotation);
    let rotation = wrap(anchor + median(samples.iter().map(|s| wrap(s.rotation - anchor)).collect()));
    let leaders = samples.iter().filter(|s| s.has_leader).count();
    let has_leader = leaders * 2 > samples.len();
    let with_leader = || samples.iter().filter(|s| s.has_leader);
    Placement {
        head: median_offset(samples.iter().map(|s| s.head)).unwrap_or(Offset { x: 0.0, y: 0.0 }),
        rotation,
        orientation: mode(samples.iter().map(|s| s.orientation.clone())),
        has_leader,
        leader_end_condition: if has_leader {
            mode(with_leader().map(|s| s.leader_end_condition.clone()))
        } else {
            String::new()
        },
        elbow: if has_leader {
            median_offset(with_leader().filter_map(|s| s.elbow))
        } else {
            None
        },
        end: if has_leader {
            median_offset(with_leader().filter_map(|s| s.end))
        } else {
            None
        },
        boundary: reduce_boundary(samples),
        from_centre,
    }
}

/// The boundary recipe the samples agree on: the more common kind, then the more common edge of
/// that kind, then the median of how far along and how far in. `None` when no sample had one.
fn reduce_boundary(samples: &[Sample]) -> Option<Boundary> {
    let all: Vec<Boundary> = samples.iter().filter_map(|s| s.boundary).collect();
    let walls = all.iter().filter(|b| b.of == BoundaryOf::Wall).count();
    let of = if walls * 2 >= all.len() && walls > 0 {
        BoundaryOf::Wall
    } else {
        BoundaryOf::Rectangle
    };
    let same: Vec<Boundary> = all.into_iter().filter(|b| b.of == of).collect();
    if same.is_empty() {
        return None;
    }
    let edge = mode(same.iter().map(|b| b.edge.to_string())).parse().unwrap_or(0);
    let on: Vec<&Boundary> = same.iter().filter(|b| b.edge == edge).collect();
    Some(Boundary {
        of,
        edge,
        along: median(on.iter().map(|b| b.along).collect()),
        off: median(on.iter().map(|b| b.off).collect()),
    })
}

/// Takes a push of observations in. Each sample replaces the one read from the same place.
pub fn observe(
    projects_dir: &Path,
    project: &str,
    request: ObservationsRequest,
    ctx: &ObserveContext,
) -> Result<ObservationOutcome, TagRuleError> {
    if request.observations.len() > MAX_OBSERVATIONS_PER_PUSH {
        return bad(format!("at most {MAX_OBSERVATIONS_PER_PUSH} observations in one push"));
    }
    let parameter = clean(&request.type_code_parameter);
    let mut outcome = ObservationOutcome::default();
    let mut grouped: HashMap<String, (RuleKey, String, Vec<Sample>)> = HashMap::new();
    for obs in &request.observations {
        for result in samples_of(obs, &parameter, ctx) {
            match result {
                Ok((key, kind, sample)) => {
                    outcome.samples += 1;
                    grouped.entry(key.id()).or_insert_with(|| (key, kind, Vec::new())).2.push(sample);
                }
                Err(reason) => outcome.skipped.push(Skipped { tag_id: obs.tag.id, reason }),
            }
        }
    }
    let _guard = SAVE_LOCK.lock().unwrap();
    let existing = load_all(projects_dir, project)?;
    let by_id: HashMap<String, TagRule> = existing.into_iter().map(|r| (r.id.clone(), r)).collect();
    let mut ids: Vec<String> = grouped.keys().cloned().collect();
    ids.sort();
    let touched: std::collections::HashSet<String> = ids.iter().cloned().collect();
    let sources: Vec<SampleSource> = grouped.values().flat_map(|g| g.2.iter().map(|s| s.source.clone())).collect();
    for id in ids {
        let (key, kind, incoming) = grouped.remove(&id).expect("id came from the map");
        if !by_id.contains_key(&id) && by_id.len() >= MAX_RULES_PER_PROJECT {
            return bad(format!("a project holds at most {MAX_RULES_PER_PROJECT} tag rules"));
        }
        let mut samples = by_id.get(&id).map(|r| r.samples.clone()).unwrap_or_default();
        for s in incoming {
            samples.retain(|e| e.source != s.source);
            samples.push(s);
        }
        if samples.len() > MAX_SAMPLES_PER_RULE {
            samples.drain(..samples.len() - MAX_SAMPLES_PER_RULE);
        }
        let rule = TagRule {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            key,
            tag_kind: kind,
            placement: reduce(&samples),
            sample_count: samples.len(),
            updated_at: now(),
            samples,
        };
        write(projects_dir, project, &rule)?;
        outcome.rules_changed += 1;
        outcome.rule_ids.push(rule.id.clone());
    }
    outcome.rules_changed +=
        crate::tag_rule_binding::take_moved_samples_out(projects_dir, project, &by_id, &touched, &sources)?;
    Ok(outcome)
}

/// The rules a task carries: those for these viewport types and, when the items are known, only
/// for these type codes (a room, space or area tag rule is about the room, so it is carried
/// whatever the items are). `None` items means any element (a task that finds the items already
/// in the room). A rule keyed by another parameter than `parameter` is never carried: its codes
/// mean something else. **Generic rules, and those bound to `blueprint`** when the sheet is bound to
/// one (plan decision 27); a rule bound to another blueprint is not the sheet's. Compared
/// case-insensitively.
pub fn rules_for(
    projects_dir: &Path,
    project: &str,
    viewport_types: &[String],
    parameter: &str,
    codes: Option<&[String]>,
    blueprint: Option<&str>,
) -> Result<Vec<RuleSummary>, TagRuleError> {
    let wanted = |s: &str| s.trim().to_lowercase();
    let types: Vec<String> = viewport_types.iter().map(|v| wanted(v)).collect();
    let codes: Option<Vec<String>> = codes.map(|c| c.iter().map(|s| wanted(s)).collect());
    let parameter = wanted(parameter);
    Ok(load_all(projects_dir, project)?
        .iter()
        .filter(|r| types.contains(&wanted(&r.key.viewport_type)))
        .filter(|r| r.key.blueprint.is_empty() || blueprint.is_some_and(|b| r.key.blueprint == b))
        .filter(|r| {
            is_spatial(&r.tag_kind)
                || (!parameter.is_empty()
                    && wanted(&r.key.code_parameter) == parameter
                    && codes.as_ref().is_none_or(|c| c.contains(&wanted(&r.key.type_code))))
        })
        .map(RuleSummary::from)
        .collect())
}

/// The item rules an RLS creation task applies (plan decision 27, reworked 2026-11-05), worked out from `rules`.
///
/// **Per type code, viewport type and view side, ONE answer, whichever room blueprint it was learned in.**
/// The generic pool (rules from the Sheets button) is used when it has the key; a rule learned on one item of
/// any room blueprint is then ignored. Only a key the generic pool lacks falls back to the blueprint rules, and
/// those are available to every instance of the type code, assigned or not.
///
/// **A pool is pooled by SAMPLES, not by rule.** Several rules can hold one key's samples (one per blueprint
/// item, since a bound item rule holds one sample), and the median of their medians is not the median of the
/// samples; `reduce` is applied to all of them, as it is for one rule. Rules learned with different tags are
/// pooled too: the task places the tag the model's RLS settings name, never the one a rule was learned with.
///
/// The summary returned is the pool's most recently updated rule with the pooled placement and sample count, so
/// `rule_id` still names a rule the library lists. Room, space and area rules are not item rules and
/// are not returned (`rules_for` carries them).
pub fn rls_item_rules(rules: &[TagRule], viewport_types: &[String], parameter: &str) -> Vec<RuleSummary> {
    let wanted = |s: &str| s.trim().to_lowercase();
    let types: Vec<String> = viewport_types.iter().map(|v| wanted(v)).collect();
    let parameter = wanted(parameter);
    if parameter.is_empty() {
        return Vec::new();
    }
    // (type code, viewport type, view side) -> the rules for that key
    type Group<'a> = ((String, String, String), Vec<&'a TagRule>);
    let mut groups: Vec<Group> = Vec::new();
    for r in rules {
        if is_spatial(&r.tag_kind)
            || wanted(&r.key.code_parameter) != parameter
            || wanted(&r.key.type_code).is_empty()
            || !types.contains(&wanted(&r.key.viewport_type))
        {
            continue;
        }
        let key = (wanted(&r.key.type_code), wanted(&r.key.viewport_type), wanted(&r.key.view_side));
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(r),
            None => groups.push((key, vec![r])),
        }
    }
    let mut out = Vec::new();
    for (_, members) in groups {
        let generic: Vec<&TagRule> = members.iter().copied().filter(|r| r.key.blueprint.is_empty()).collect();
        let pool = if generic.is_empty() { members } else { generic };
        if let Some(rule) = pooled(&pool) {
            out.push(rule);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// One pool of rules for a key, as one rule: every sample of every rule in it, reduced together. The rules may
/// have been learned with different tags (category, family, type), and that no longer divides them: the task places
/// the tag the model's RLS settings name, so each sample is just one more answer to where a tag of that item goes.
fn pooled(pool: &[&TagRule]) -> Option<RuleSummary> {
    let weight = |r: &TagRule| r.samples.len().max(r.sample_count);
    let newest = pool
        .iter()
        .copied()
        .max_by(|a, b| a.updated_at.cmp(&b.updated_at).then_with(|| b.id.cmp(&a.id)))?;
    let samples: Vec<Sample> = pool.iter().flat_map(|r| r.samples.iter().cloned()).collect();
    let mut summary = RuleSummary::from(newest);
    if !samples.is_empty() {
        summary.placement = reduce(&samples);
    }
    summary.sample_count = pool.iter().map(|r| weight(r)).sum();
    Some(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    // The blueprint-binding tests drive `observe` and the binding operations together, so they live here with the fixtures.
    use crate::tag_rule_binding::{bound_to, import_bound, unlink_blueprint};

    // ---- rls_item_rules: the effective item rules of a create-RLS task ------------------------------

    fn sample_at(x: f64, tag_id: i64) -> Sample {
        Sample {
            source: SampleSource {
                sheet: SourceSheet { document_title: "m".into(), revit_sheet_id: "1".into(), sheet_number: "1".into() },
                tag_id,
                element_id: tag_id,
            },
            view_scale: 50.0,
            head: Offset { x, y: 0.0 },
            rotation: 0.0,
            orientation: "Horizontal".into(),
            has_leader: false,
            leader_end_condition: String::new(),
            elbow: None,
            end: None,
            boundary: None,
            from_centre: false,
        }
    }

    fn item_rule(
        id: &str,
        code: &str,
        blueprint: &str,
        instance: &str,
        tag_type: &str,
        xs: &[f64],
        at: &str,
    ) -> TagRule {
        let samples: Vec<Sample> = xs.iter().enumerate().map(|(i, x)| sample_at(*x, i as i64)).collect();
        TagRule {
            schema_version: 2,
            id: id.into(),
            key: RuleKey {
                tagged_category: "Furniture".into(),
                code_parameter: "Assembly Code".into(),
                type_code: code.into(),
                tag_category: "Multi-Category Tags".into(),
                tag_family: "T".into(),
                tag_type: tag_type.into(),
                viewport_type: "floor plan".into(),
                blueprint: blueprint.into(),
                instance: instance.into(),
                view_side: String::new(),
            },
            tag_kind: "independent".into(),
            placement: reduce(&samples),
            sample_count: samples.len(),
            updated_at: at.into(),
            samples,
        }
    }

    fn plan_types() -> Vec<String> {
        vec!["floor plan".to_string()]
    }

    #[test]
    fn test_a_rule_learned_on_a_blueprint_item_is_available_for_any_instance_of_the_code() {
        let rules = [item_rule("a", "FQSN-113", "bp1", "i1", "Std", &[10.0], "1")];
        let got = rls_item_rules(&rules, &plan_types(), "Assembly Code");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].placement.head.x, 10.0);
    }

    #[test]
    fn test_blueprint_rules_for_one_code_are_pooled_by_sample_across_blueprints() {
        // Three blueprints, one sample each: the median of the samples is 20, not a median of medians of something else.
        let rules = [
            item_rule("a", "C", "bp1", "i1", "Std", &[10.0], "1"),
            item_rule("b", "C", "bp2", "i2", "Std", &[20.0], "2"),
            item_rule("c", "C", "bp3", "i3", "Std", &[90.0], "3"),
        ];
        let got = rls_item_rules(&rules, &plan_types(), "Assembly Code");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].placement.head.x, 20.0);
        assert_eq!(got[0].sample_count, 3);
        assert_eq!(got[0].id, "c", "the rule_id is the most recently updated rule of the pool");
    }

    #[test]
    fn test_the_generic_pool_wins_and_blueprint_rules_are_ignored_for_that_code() {
        let rules = [
            item_rule("g", "C", "", "", "Std", &[40.0], "1"),
            item_rule("b", "C", "bp1", "i1", "Std", &[10.0], "2"),
            item_rule("other", "D", "bp1", "i2", "Std", &[70.0], "2"),
        ];
        let got = rls_item_rules(&rules, &plan_types(), "Assembly Code");
        let of = |id: &str| got.iter().find(|r| r.key.type_code == id).map(|r| r.placement.head.x);
        assert_eq!(of("C"), Some(40.0), "generic wins for a code it has");
        assert_eq!(of("D"), Some(70.0), "a code the generic pool lacks falls back to the blueprint rule");
    }

    #[test]
    fn test_rules_learned_with_different_tags_are_pooled_because_the_settings_choose_the_tag() {
        let rules = [
            item_rule("a", "C", "bp1", "i1", "Small", &[10.0], "1"),
            item_rule("b", "C", "bp2", "i2", "Big", &[50.0, 60.0], "2"),
        ];
        let got = rls_item_rules(&rules, &plan_types(), "Assembly Code");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].placement.head.x, 50.0, "the median of 10, 50 and 60");
        assert_eq!(got[0].sample_count, 3);
    }

    #[test]
    fn test_a_task_rule_carries_no_tag() {
        let summary = RuleSummary::from(&item_rule("a", "C", "", "", "Std", &[10.0], "1"));
        let text = serde_json::to_string(&TaskRule::from(&summary)).unwrap();
        assert!(
            !text.contains("tag_family") && !text.contains("tag_type") && !text.contains("tag_category"),
            "{text}"
        );
        // A task stored with the whole key still reads.
        let old: TaskRule = serde_json::from_str(&serde_json::to_string(&summary).unwrap()).unwrap();
        assert_eq!(old.key.type_code, "C");
    }

    #[test]
    fn test_other_viewport_types_parameters_and_room_tags_are_not_item_rules() {
        let mut elevation = item_rule("e", "C", "bp1", "i1", "Std", &[1.0], "1");
        elevation.key.viewport_type = "elevation".into();
        let mut other_parameter = item_rule("p", "C", "bp1", "i1", "Std", &[1.0], "1");
        other_parameter.key.code_parameter = "Type Mark".into();
        let mut room = item_rule("r", "", "bp1", "", "Std", &[1.0], "1");
        room.tag_kind = "room".into();
        let rules = [elevation, other_parameter, room];
        assert!(rls_item_rules(&rules, &plan_types(), "Assembly Code").is_empty());
        assert!(
            rls_item_rules(&rules, &plan_types(), "").is_empty(),
            "a project with no code parameter has no item rules"
        );
    }

    /// A fresh directory per test, removed when dropped.
    struct TestDir(PathBuf);

    impl TestDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn dir() -> TestDir {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("roommate-tagrules-{}-{n}", std::process::id()));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).unwrap();
        TestDir(path)
    }

    fn location(x: f64, y: f64, facing: (f64, f64)) -> Location {
        Location {
            translation_coord: P3 { x, y, z: 0.0 },
            rotation_coord: Some(Matrix {
                data: vec![
                    vec![facing.0, facing.1, 0.0],
                    vec![-facing.1, facing.0, 0.0],
                    vec![0.0, 0.0, 1.0],
                ],
            }),
        }
    }

    /// A bed at (`x`, 0) facing `facing`, tagged by a head at `head`.
    fn obs(tag_id: i64, x: f64, facing: (f64, f64), head: (f64, f64)) -> Observation {
        Observation {
            viewport_type: Some("floor plan".to_string()),
            view_id: Some(7),
            view_scale: Some(50.0),
            view_direction: None,
            sheet: SourceSheet {
                document_title: "M".into(),
                revit_sheet_id: "1".into(),
                sheet_number: "RLS-1".into(),
            },
            tag: ObservedTag {
                id: tag_id,
                tag_kind: "independent".into(),
                tag_type: TypeNames {
                    category: "Furniture Tags".into(),
                    family_name: "FTag".into(),
                    type_name: "Std".into(),
                },
                rotation: 0.0,
                orientation: "Horizontal".into(),
                point: P3 { x: head.0, y: head.1, z: 0.0 },
                tagged_elements: vec![TaggedElement {
                    host_element_id: 100 + tag_id,
                    names: TypeNames {
                        category: "Furniture".into(),
                        family_name: "Bed".into(),
                        type_name: "Queen".into(),
                    },
                    type_code: Some("BED-01".into()),
                    location_point: Some(location(x, 0.0, facing)),
                    ..Default::default()
                }],
                ..Default::default()
            },
        }
    }

    fn push(d: &Path, o: Vec<Observation>) -> ObservationOutcome {
        push_for(d, o, None, &ObserveContext::default())
    }

    fn push_for(d: &Path, o: Vec<Observation>, blueprint: Option<&str>, ctx: &ObserveContext) -> ObservationOutcome {
        let request = ObservationsRequest {
            blueprint: blueprint.map(str::to_string),
            type_code_parameter: "Assembly Code".into(),
            observations: o,
        };
        observe(d, "p", request, ctx).unwrap()
    }

    fn bound_as(id: &str, items: &[&str]) -> ObserveContext<'static> {
        ObserveContext {
            blueprint: Some((id.to_string(), items.iter().map(|s| s.to_string()).collect())),
            ..Default::default()
        }
    }

    /// A push for blueprint `bp` whose items are the elements numbered `items` (the bed of `obs(n, ..)` is 100 + n).
    fn bound(items: &[&str]) -> ObserveContext<'static> {
        ObserveContext {
            blueprint: Some(("bp".to_string(), items.iter().map(|s| s.to_string()).collect())),
            ..Default::default()
        }
    }

    /// `obs` seen in an elevation looking along +y (so the viewer's right is +x and up is +z), the tag 200 mm
    /// to the right of the bed and 900 mm above its point; the bed faces `facing`.
    fn elevation_obs(tag_id: i64, facing: (f64, f64)) -> Observation {
        let mut o = obs(tag_id, 0.0, facing, (200.0, 0.0));
        o.viewport_type = Some("elevation".into());
        o.view_direction = Some(P3 { x: 0.0, y: 1.0, z: 0.0 });
        o.tag.point = P3 { x: 200.0, y: 0.0, z: 900.0 };
        o
    }

    #[test]
    fn test_an_elevation_tag_is_measured_along_the_views_right_and_up_and_keyed_by_how_the_item_is_seen() {
        let d = dir();
        // a bed facing the viewer (facing parallel to the view direction) and one seen from the side
        let out = push(d.path(), vec![elevation_obs(1, (0.0, -1.0)), elevation_obs(2, (1.0, 0.0))]);
        assert_eq!(out.samples, 2, "{:?}", out.skipped);
        let rules = load_all(d.path(), "p").unwrap();
        let mut sides: Vec<&str> = rules.iter().map(|r| r.key.view_side.as_str()).collect();
        sides.sort();
        assert_eq!(sides, ["front", "side"]);
        for r in &rules {
            // 200 mm right and 900 mm up at 1:50 is 4 and 18 mm on paper, whichever way the bed faces
            assert!(
                (r.placement.head.x - 4.0).abs() < 1e-9 && (r.placement.head.y - 18.0).abs() < 1e-9,
                "{:?}",
                r.placement.head
            );
        }
    }

    #[test]
    fn test_rails_at_three_heights_on_one_insertion_point_learn_one_offset_from_their_centres() {
        // House A's WC: three towel rails share an insertion point at the level, and the family lifts each to
        // 900, 1200 and 1500 mm. Their tags sit 100 mm below each rail. From the point the three samples are three
        // answers (the task took the median and stacked every tag); from the centres they are one.
        let d = dir();
        let rail = |tag_id: i64, height: f64| {
            let mut o = elevation_obs(tag_id, (1.0, 0.0));
            o.tag.point = P3 { x: 200.0, y: 0.0, z: height - 100.0 };
            o.tag.tagged_elements[0].centre = Some(P3 { x: 0.0, y: 0.0, z: height });
            o
        };
        let out = push(d.path(), vec![rail(1, 900.0), rail(2, 1200.0), rail(3, 1500.0)]);
        assert_eq!(out.samples, 3, "{:?}", out.skipped);
        let rules = load_all(d.path(), "p").unwrap();
        assert_eq!(rules.len(), 1);
        let r = &rules[0];
        assert!(r.samples.iter().all(|s| s.from_centre));
        assert!(r.placement.from_centre);
        // 200 mm right and 100 mm down at 1:50
        assert!(
            (r.placement.head.x - 4.0).abs() < 1e-9 && (r.placement.head.y + 2.0).abs() < 1e-9,
            "{:?}",
            r.placement
        );
    }

    #[test]
    fn test_a_plan_keeps_the_insertion_point_and_old_samples_are_not_mixed_with_centred_ones() {
        let d = dir();
        // a plan tag is measured from the point even when the centre was read
        let mut plan = obs(1, 0.0, (1.0, 0.0), (200.0, 0.0));
        plan.tag.tagged_elements[0].centre = Some(P3 { x: 50.0, y: 0.0, z: 400.0 });
        push(d.path(), vec![plan]);
        let rules = load_all(d.path(), "p").unwrap();
        assert!(!rules[0].placement.from_centre && !rules[0].samples[0].from_centre);
        assert!((rules[0].placement.head.x - 4.0).abs() < 1e-9);
        // two samples from the point and one from the centre: the centre's alone decides, and says so
        let mut old = sample_at(10.0, 1);
        let old2 = sample_at(12.0, 2);
        let mut new = sample_at(-3.0, 3);
        new.from_centre = true;
        let p = reduce(&[old.clone(), old2.clone(), new]);
        assert!(p.from_centre);
        assert_eq!(p.head.x, -3.0);
        // and with none from the centre, nothing changes
        old.head.x = 11.0;
        let p = reduce(&[old, old2]);
        assert!(!p.from_centre);
        assert_eq!(p.head.x, 11.5);
    }

    #[test]
    fn test_an_elevation_with_no_view_direction_gives_no_sample_and_a_plan_rule_keeps_its_id() {
        let d = dir();
        let mut o = elevation_obs(1, (0.0, 1.0));
        o.view_direction = None;
        let out = push(d.path(), vec![o]);
        assert_eq!(out.samples, 0);
        assert!(out.skipped[0].reason.contains("view direction"), "{:?}", out.skipped);
        // an empty side is left out of the id, so a rule written before the field existed is still found
        let mut key = RuleKey {
            tagged_category: "A".into(),
            code_parameter: "B".into(),
            type_code: "C".into(),
            tag_category: "D".into(),
            tag_family: "E".into(),
            tag_type: "F".into(),
            viewport_type: "floor plan".into(),
            blueprint: String::new(),
            instance: String::new(),
            view_side: String::new(),
        };
        let plain = key.id();
        key.view_side = "front".into();
        assert_ne!(plain, key.id());
    }

    #[test]
    fn test_a_tag_on_a_blueprint_item_is_bound_to_that_item_and_any_other_is_generic() {
        let d = dir();
        // beds 101 and 102 are in the blueprint, bed 103 is not.
        let out = push_for(
            d.path(),
            vec![
                obs(1, 0.0, (1.0, 0.0), (100.0, 0.0)),
                obs(2, 0.0, (1.0, 0.0), (300.0, 0.0)),
                obs(3, 0.0, (1.0, 0.0), (500.0, 0.0)),
            ],
            Some("bp"),
            &bound(&["101", "102"]),
        );
        assert_eq!(out.samples, 3);
        let rules = load_all(d.path(), "p").unwrap();
        let mut keys: Vec<(String, String)> =
            rules.iter().map(|r| (r.key.blueprint.clone(), r.key.instance.clone())).collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                ("".into(), "".into()),
                ("bp".into(), "101".into()),
                ("bp".into(), "102".into())
            ],
            "one rule per blueprint item, one holding the sample of the item that is not the blueprint's"
        );
        assert!(rules.iter().all(|r| r.sample_count == 1));
        // And the carried rules: a sheet of that blueprint gets all three, another gets only the generic one.
        let types = ["floor plan".to_string()];
        let of = |b: Option<&str>| rules_for(d.path(), "p", &types, "Assembly Code", None, b).unwrap().len();
        assert_eq!((of(Some("bp")), of(Some("other")), of(None)), (3, 1, 1));
    }

    #[test]
    fn test_a_sheet_pushed_again_under_another_scope_takes_its_samples_with_it() {
        let d = dir();
        let one = || vec![obs(1, 0.0, (1.0, 0.0), (100.0, 0.0))];
        push(d.path(), one());
        assert_eq!(load_all(d.path(), "p").unwrap().len(), 1);
        // The same sheet, now for a blueprint: the generic rule had only that sample, so it goes.
        push_for(d.path(), one(), Some("bp"), &bound(&["101"]));
        let rules = load_all(d.path(), "p").unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].key.blueprint, "bp");
        // A second generic sheet's sample stays in the generic rule when the first one moves.
        let mut other = obs(9, 0.0, (1.0, 0.0), (100.0, 0.0));
        other.sheet.revit_sheet_id = "2".into();
        push(d.path(), vec![other]);
        push(d.path(), one());
        let generic: Vec<_> = load_all(d.path(), "p")
            .unwrap()
            .into_iter()
            .filter(|r| r.key.blueprint.is_empty())
            .collect();
        assert_eq!((generic.len(), generic[0].sample_count), (1, 2), "both generic samples, once each");
        assert_eq!(load_all(d.path(), "p").unwrap().len(), 1, "the bound rule lost its sample and went");
    }

    #[test]
    fn test_a_room_tag_keeps_its_boundary_and_a_room_off_the_server_keeps_only_the_axes() {
        let d = dir();
        let seen = std::cell::RefCell::new(Vec::new());
        let f = |model: &str, room: i64, head: P3, bound: bool, scale: f64| -> Option<Boundary> {
            seen.borrow_mut().push((model.to_string(), room, head.x, bound, scale));
            Some(Boundary { of: BoundaryOf::Rectangle, edge: 0, along: 0.25, off: 20.0 })
        };
        let ctx = ObserveContext { boundary: Some(&f), ..Default::default() };
        push_for(d.path(), vec![room_obs(1, (1500.0, 250.0))], None, &ctx);
        let rule = &load_all(d.path(), "p").unwrap()[0];
        assert_eq!(
            rule.placement.boundary,
            Some(Boundary { of: BoundaryOf::Rectangle, edge: 0, along: 0.25, off: 20.0 })
        );
        assert_eq!(
            *seen.borrow(),
            [("M".to_string(), 101, 1500.0, false, 50.0)],
            "the room's id, the head, generic, the scale"
        );
        // No boundary function (the room is not on the server): the rule is the plan-axes offset as before.
        let d = dir();
        push(d.path(), vec![room_obs(1, (1500.0, 250.0))]);
        assert_eq!(load_all(d.path(), "p").unwrap()[0].placement.boundary, None);
    }

    #[test]
    fn test_samples_with_a_boundary_reduce_to_the_common_edge_and_the_median() {
        let at = |edge, along, off| Sample {
            source: SampleSource { sheet: SourceSheet::default(), tag_id: 0, element_id: 0 },
            view_scale: 50.0,
            head: Offset { x: 0.0, y: 0.0 },
            rotation: 0.0,
            orientation: String::new(),
            has_leader: false,
            leader_end_condition: String::new(),
            elbow: None,
            end: None,
            boundary: Some(Boundary { of: BoundaryOf::Wall, edge, along, off }),
            from_centre: false,
        };
        let p = reduce(&[at(2, 0.2, 10.0), at(2, 0.4, 14.0), at(1, 0.9, 99.0)]);
        let b = p.boundary.unwrap();
        assert_eq!((b.of, b.edge), (BoundaryOf::Wall, 2), "the edge most samples agree on");
        assert!((b.along - 0.3).abs() < 1e-9 && (b.off - 12.0).abs() < 1e-9, "{b:?}");
    }

    #[test]
    fn test_a_bound_rule_the_push_no_longer_writes_is_unlinked_into_the_generic_rule_of_its_key() {
        let d = dir();
        // Two beds of the blueprint are tagged, and another sheet has a generic tag for the same code.
        let mut generic = obs(9, 0.0, (1.0, 0.0), (100.0, 0.0));
        generic.sheet.revit_sheet_id = "2".into();
        push(d.path(), vec![generic]);
        let out = push_for(
            d.path(),
            vec![
                obs(1, 0.0, (1.0, 0.0), (300.0, 0.0)),
                obs(2, 0.0, (1.0, 0.0), (500.0, 0.0)),
            ],
            Some("bp"),
            &bound(&["101", "102"]),
        );
        assert_eq!(load_all(d.path(), "p").unwrap().len(), 3);
        // The re-push keeps bed 101 and drops bed 102.
        let keep: std::collections::HashSet<String> = load_all(d.path(), "p")
            .unwrap()
            .into_iter()
            .filter(|r| r.key.instance == "101")
            .map(|r| r.id)
            .collect();
        assert_eq!(out.rule_ids.len(), 2);
        let n = unlink_blueprint(d.path(), "p", "bp", &keep).unwrap();
        assert_eq!(n, 1);
        let rules = load_all(d.path(), "p").unwrap();
        assert_eq!(rules.len(), 2, "the bound rule of 101 and the generic one");
        let generic = rules.iter().find(|r| r.key.blueprint.is_empty()).unwrap();
        assert_eq!(generic.sample_count, 2, "its own sample and the unlinked one");
        assert!(generic.samples.iter().all(|s| s.boundary.is_none()));
        // Nothing to unlink the second time.
        assert_eq!(unlink_blueprint(d.path(), "p", "bp", &keep).unwrap(), 0);
    }

    #[test]
    fn test_an_unlinked_rule_with_no_generic_twin_becomes_the_generic_rule() {
        let d = dir();
        push_for(d.path(), vec![obs(1, 0.0, (1.0, 0.0), (300.0, 0.0))], Some("bp"), &bound(&["101"]));
        assert_eq!(unlink_blueprint(d.path(), "p", "bp", &Default::default()).unwrap(), 1);
        let rules = load_all(d.path(), "p").unwrap();
        assert_eq!(
            (rules.len(), rules[0].key.blueprint.as_str(), rules[0].key.instance.as_str()),
            (1, "", "")
        );
        assert_eq!(rules[0].sample_count, 1);
    }

    #[test]
    fn test_imported_rules_are_bound_to_the_new_blueprint_and_worked_out_again() {
        let d = dir();
        push_for(
            d.path(),
            vec![obs(1, 0.0, (1.0, 0.0), (300.0, 0.0))],
            Some("old"),
            &bound_as("old", &["101"]),
        );
        let exported = bound_to(d.path(), "p", "old").unwrap();
        assert_eq!(exported.len(), 1);
        let ids = import_bound(d.path(), "p", "new", exported).unwrap();
        assert_eq!(ids.len(), 1);
        let again = bound_to(d.path(), "p", "new").unwrap();
        assert_eq!((again.len(), again[0].key.instance.as_str(), again[0].sample_count), (1, "101", 1));
        assert_eq!(again[0].id, again[0].key.id());
    }

    #[test]
    fn test_a_rule_of_the_name_keyed_schema_is_not_loaded() {
        let d = dir();
        push(d.path(), vec![obs(1, 0.0, (1.0, 0.0), (100.0, 0.0))]);
        let rule = load_all(d.path(), "p").unwrap().remove(0);
        let old = TagRule { schema_version: 1, ..rule };
        write(d.path(), "p", &old).unwrap();
        assert!(load_all(d.path(), "p").unwrap().is_empty());
        // The sheet pushed again makes the rule afresh, under the same key.
        push(d.path(), vec![obs(1, 0.0, (1.0, 0.0), (100.0, 0.0))]);
        assert_eq!(load_all(d.path(), "p").unwrap()[0].schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn test_an_offset_is_in_the_elements_frame_and_in_paper_millimetres() {
        // Element facing +y; the head is 1000 mm along +x of the model, which is to the element's right.
        let d = dir();
        let out = push(d.path(), vec![obs(1, 0.0, (0.0, 1.0), (1000.0, 500.0))]);
        assert_eq!(out.samples, 1);
        let rule = &load_all(d.path(), "p").unwrap()[0];
        // along facing (+y) = 500 mm -> 10 on paper; to the left (-x) = -1000 mm -> -20.
        assert!((rule.placement.head.x - 10.0).abs() < 1e-9, "{:?}", rule.placement.head);
        assert!((rule.placement.head.y + 20.0).abs() < 1e-9, "{:?}", rule.placement.head);
    }

    #[test]
    fn test_one_rule_per_key_and_the_median_of_its_samples() {
        let d = dir();
        // three beds, the same tag type, three different head positions along x (paper mm 2, 4, 100)
        push(
            d.path(),
            vec![
                obs(1, 0.0, (1.0, 0.0), (100.0, 0.0)),
                obs(2, 5000.0, (1.0, 0.0), (5200.0, 0.0)),
                obs(3, 9000.0, (1.0, 0.0), (14000.0, 0.0)),
            ],
        );
        let rules = load_all(d.path(), "p").unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].sample_count, 3);
        assert!((rules[0].placement.head.x - 4.0).abs() < 1e-9);
    }

    #[test]
    fn test_pushing_the_same_sheet_again_replaces_its_samples() {
        let d = dir();
        push(d.path(), vec![obs(1, 0.0, (1.0, 0.0), (100.0, 0.0))]);
        push(d.path(), vec![obs(1, 0.0, (1.0, 0.0), (500.0, 0.0))]);
        let rule = &load_all(d.path(), "p").unwrap()[0];
        assert_eq!(rule.sample_count, 1);
        assert!((rule.placement.head.x - 10.0).abs() < 1e-9);
    }

    #[test]
    fn test_the_key_is_the_type_code_so_names_and_case_do_not_matter() {
        let a = obs(1, 0.0, (1.0, 0.0), (1.0, 0.0));
        // Another family and type name, the same code in another case: the same kind of item.
        let mut b = obs(2, 0.0, (1.0, 0.0), (1.0, 0.0));
        b.tag.tagged_elements[0].names.family_name = "Divan".into();
        b.tag.tagged_elements[0].names.type_name = "Double".into();
        b.tag.tagged_elements[0].type_code = Some("bed-01".into());
        let d = dir();
        push(d.path(), vec![a, b]);
        let rules = load_all(d.path(), "p").unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].sample_count, 2);
        assert_eq!(rules[0].key.code_parameter, "Assembly Code");
    }

    #[test]
    fn test_two_codes_are_two_rules_and_two_parameters_never_meet() {
        let a = obs(1, 0.0, (1.0, 0.0), (1.0, 0.0));
        let mut b = obs(2, 0.0, (1.0, 0.0), (1.0, 0.0));
        b.tag.tagged_elements[0].type_code = Some("DESK-02".into());
        let d = dir();
        push(d.path(), vec![a.clone(), b]);
        assert_eq!(load_all(d.path(), "p").unwrap().len(), 2);
        // Another tag with the same code, read under another parameter: a rule of its own.
        let mut c = a;
        c.tag.id = 7;
        observe(
            d.path(),
            "p",
            ObservationsRequest { blueprint: None, type_code_parameter: "Type Mark".into(), observations: vec![c] },
            &ObserveContext::default(),
        )
        .unwrap();
        assert_eq!(
            load_all(d.path(), "p").unwrap().len(),
            3,
            "the same code under another parameter is its own rule"
        );
    }

    #[test]
    fn test_an_item_with_no_code_is_skipped_and_so_is_a_push_naming_no_parameter() {
        let mut none = obs(1, 0.0, (1.0, 0.0), (1.0, 0.0));
        none.tag.tagged_elements[0].type_code = None;
        let mut blank = obs(2, 0.0, (1.0, 0.0), (1.0, 0.0));
        blank.tag.tagged_elements[0].type_code = Some("  ".into());
        let out = push(dir().path(), vec![none, blank]);
        assert_eq!((out.samples, out.skipped.len()), (0, 2));
        assert!(out.skipped[0].reason.contains("Assembly Code"), "{}", out.skipped[0].reason);

        let d = dir();
        let out = observe(
            d.path(),
            "p",
            ObservationsRequest {
                blueprint: None,
                type_code_parameter: String::new(),
                observations: vec![obs(3, 0.0, (1.0, 0.0), (1.0, 0.0))],
            },
            &ObserveContext::default(),
        )
        .unwrap();
        assert_eq!(out.samples, 0);
        assert!(out.skipped[0].reason.contains("tag_type_code"));
    }

    #[test]
    fn test_what_cannot_be_used_is_skipped_with_a_reason() {
        let mut no_scale = obs(1, 0.0, (1.0, 0.0), (1.0, 0.0));
        no_scale.view_scale = None;
        let mut no_facing = obs(2, 0.0, (0.0, 0.0), (1.0, 0.0));
        no_facing.tag.tagged_elements[0].location_point =
            Some(Location { translation_coord: P3::default(), rotation_coord: None });
        let mut no_element = obs(3, 0.0, (1.0, 0.0), (1.0, 0.0));
        no_element.tag.tagged_elements.clear();
        let out = push(dir().path(), vec![no_scale, no_facing, no_element]);
        assert_eq!(out.samples, 0);
        assert_eq!(out.skipped.len(), 3);
        assert!(out.skipped[1].reason.contains("facing"));
    }

    /// A room tag: the tagged element is the room, with a category and a point and nothing else.
    fn room_obs(tag_id: i64, head: (f64, f64)) -> Observation {
        let mut o = obs(tag_id, 1000.0, (1.0, 0.0), head);
        o.tag.tag_kind = "room".into();
        o.tag.tag_type = TypeNames {
            category: "Room Tags".into(),
            family_name: "Room Tag".into(),
            type_name: "Room Tag".into(),
        };
        o.tag.tagged_elements[0].names = TypeNames { category: "Rooms".into(), ..Default::default() };
        o.tag.tagged_elements[0].location_point =
            Some(Location { translation_coord: P3 { x: 1000.0, y: 0.0, z: 0.0 }, rotation_coord: None });
        o
    }

    #[test]
    fn test_a_room_tag_is_a_rule_keyed_by_the_category_in_the_plans_own_axes() {
        let d = dir();
        let out = push(d.path(), vec![room_obs(1, (1000.0 + 500.0, 250.0))]);
        assert_eq!((out.samples, out.skipped.len()), (1, 0), "{:?}", out.skipped);
        let rule = &load_all(d.path(), "p").unwrap()[0];
        assert_eq!(rule.tag_kind, "room");
        assert_eq!(
            (
                rule.key.tagged_category.as_str(),
                rule.key.code_parameter.as_str(),
                rule.key.type_code.as_str()
            ),
            ("Rooms", "", "")
        );
        // 500 mm along +x and 250 mm along +y at 1:50 is 10 and 5 on paper.
        assert!((rule.placement.head.x - 10.0).abs() < 1e-9 && (rule.placement.head.y - 5.0).abs() < 1e-9);
    }

    #[test]
    fn test_a_room_tag_with_no_category_is_skipped_with_a_reason() {
        let mut o = room_obs(1, (1.0, 1.0));
        o.tag.tagged_elements[0].names.category.clear();
        let out = push(dir().path(), vec![o]);
        assert_eq!(out.samples, 0);
        assert!(out.skipped[0].reason.contains("category"));
    }

    #[test]
    fn test_a_room_rule_rides_with_every_task_whatever_items_it_places() {
        let d = dir();
        push(d.path(), vec![obs(1, 0.0, (1.0, 0.0), (1.0, 0.0)), room_obs(2, (1001.0, 1.0))]);
        let other = ["CHAIR-09".to_string()];
        let carried =
            rules_for(d.path(), "p", &["floor plan".to_string()], "Assembly Code", Some(&other), None).unwrap();
        assert_eq!(carried.len(), 1);
        assert_eq!(carried[0].tag_kind, "room");
    }

    #[test]
    fn test_rules_for_filters_by_viewport_type_and_items() {
        let d = dir();
        push(d.path(), vec![obs(1, 0.0, (1.0, 0.0), (1.0, 0.0))]);
        let p = "Assembly Code";
        let all = rules_for(d.path(), "p", &["Floor Plan".to_string()], p, None, None).unwrap();
        assert_eq!(all.len(), 1);
        assert!(rules_for(d.path(), "p", &["elevation".to_string()], p, None, None).unwrap().is_empty());
        let items = ["bed-01".to_string()];
        assert_eq!(
            rules_for(d.path(), "p", &["floor plan".to_string()], p, Some(&items), None)
                .unwrap()
                .len(),
            1
        );
        let other = ["DESK-02".to_string()];
        assert!(rules_for(d.path(), "p", &["floor plan".to_string()], p, Some(&other), None)
            .unwrap()
            .is_empty());
        // Another parameter's codes mean something else, and none named means no item rules at all.
        assert!(rules_for(d.path(), "p", &["floor plan".to_string()], "Type Mark", None, None)
            .unwrap()
            .is_empty());
        assert!(rules_for(d.path(), "p", &["floor plan".to_string()], "", None, None).unwrap().is_empty());
    }

    #[test]
    fn test_a_rotation_spread_across_pi_does_not_average_to_nothing() {
        let s = |r: f64| Sample {
            source: SampleSource { sheet: SourceSheet::default(), tag_id: 0, element_id: 0 },
            view_scale: 50.0,
            head: Offset { x: 0.0, y: 0.0 },
            rotation: r,
            orientation: String::new(),
            has_leader: false,
            leader_end_condition: String::new(),
            elbow: None,
            end: None,
            boundary: None,
            from_centre: false,
        };
        let p = reduce(&[s(3.1), s(-3.1), s(3.12)]);
        assert!(p.rotation.abs() > 3.0, "{}", p.rotation);
    }

    #[test]
    fn test_delete_removes_the_rule_and_a_missing_one_is_not_found() {
        let d = dir();
        push(d.path(), vec![obs(1, 0.0, (1.0, 0.0), (1.0, 0.0))]);
        let id = load_all(d.path(), "p").unwrap()[0].id.clone();
        delete(d.path(), "p", &id).unwrap();
        assert!(matches!(delete(d.path(), "p", &id), Err(TagRuleError::NotFound(_))));
        assert!(load_all(d.path(), "p").unwrap().is_empty());
    }
}
