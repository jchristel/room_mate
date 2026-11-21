//! Room tasks: work the server holds for a Revit session to do (`docs/PLAN-blueprints.md`,
//! decisions 21 to 23 and 26).
//!
//! **A task is a record, unlike an assignment.** An assignment is a request worked out again on
//! every read; a task holds what Revit is told, as it was when the task was made -- the resolver's
//! placements (in the target MODEL's own frame, mm), the sheet layouts and the findings -- so
//! what a Revit session writes cannot change underneath it, and a later look says what was asked.
//! The plugin validates and writes; it never resolves (decision 2).
//!
//! **Four kinds.** `populate_room` places a blueprint's items in a room (and, if asked, its
//! sheets); `create_rls` makes room layout sheets from a library sheet and places nothing;
//! `update_room` brings a room populated earlier back in line with its blueprint (decision 21:
//! items, or only their locations, after a room-size or design change). An update is a DRY RUN
//! unless the person turns that off (decision 13): the session reports the diff and writes nothing.
//! `create_rls_set` makes the sheets of a SET of adjacent rooms as a person laid them out
//! (`service::rls_set_task`); its `room` is the set's lead room and `rls_set.rooms` names them all.
//!
//! **States, and who moves them.** A task is `queued` when made or resent. A Revit session asks
//! for a model's tasks (`claim`) and every queued one becomes `in_progress`, so another session
//! is not handed it. The session then reports each as `completed`, `completed_with_warnings`,
//! `failed` or `aborted` (an abort found by validating the whole list is reported before any
//! work). A person can `dismiss` a task, or `resend` one (back to `queued`), which is also the
//! way out of a session that died holding tasks `in_progress`. Nothing is ever deleted: a
//! task's attempts are kept, each with what the session said.
//!
//! One JSON file per task under `<projects_dir>/tasks/<project>/<id>.json`, written the way
//! assignments are (temp, then rename, a copy of the file replaced under `.backups/`, one lock).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::service::blueprint_model::{BlueprintSheet, ItemClass, Property};
use crate::service::rls_plan::{RlsPlan, ViewportMove};
use crate::service::rls_set_task::RlsSetPlan;
use crate::state::is_path_safe_component;
use crate::tag_rules::TaskRule;

mod store;
mod edits;
pub use store::*;
pub use edits::*;

pub const SCHEMA_VERSION: u32 = 1;
const MAX_TASKS_PER_PROJECT: usize = 50_000;
const MAX_MESSAGES: usize = 200;
const MAX_MESSAGE_CHARS: usize = 2_000;

static SAVE_LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Place a blueprint's items in a room, and its sheets if `create_rls`.
    PopulateRoom,
    /// Make room layout sheets from a library sheet; nothing is placed.
    CreateRls,
    /// Update a room populated earlier from its blueprint: the task holds the layout as the
    /// resolver now places it, and the session matches it to what is in the room by the
    /// marker each placed item carries (blueprint id, revision, item id).
    UpdateRoom,
    /// Make the sheets of a set of adjacent rooms, laid out in the composer (`rls_set`).
    CreateRlsSet,
}

/// What an update changes in a room that already holds the blueprint's items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// Items: add the ones the layout has and the room lacks, remove the ones it no longer has,
    /// change type and properties, and move.
    Items,
    /// Locations: move what is there; add and remove nothing.
    Locations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    InProgress,
    Completed,
    CompletedWithWarnings,
    Failed,
    Aborted,
    Dismissed,
}

impl TaskState {
    /// What a Revit session may report a task as.
    pub fn is_report(self) -> bool {
        matches!(self, Self::Completed | Self::CompletedWithWarnings | Self::Failed | Self::Aborted)
    }
}

/// The room, as the task names it. The id is the server's reference; the number is what the Revit
/// side finds the room by (decision 22), and is absent when the room has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRoom {
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    pub name: String,
}

/// Which blueprint revision a populate task was made from: a record, not a reference that must resolve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlueprintRef {
    pub id: String,
    pub name: String,
    pub revision: u32,
}

/// One family instance to place, in the target model's own frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskItem {
    /// The blueprint item it came from.
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_name: Option<String>,
    pub type_name: String,
    pub category: String,
    /// `free`, or `wall_device` (placed by its wall and height, no footprint).
    pub placement_type: ItemClass,
    /// The instance's `Host` property as exported (for example `Level : LEVEL 1`), when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Centre in the target model's frame, millimetres.
    pub x_mm: f64,
    pub y_mm: f64,
    /// Unit vector of the item's facing, in the same frame.
    pub facing_x: f64,
    pub facing_y: f64,
    /// Footprint size in plan: along the facing, then across it.
    pub size_mm: [f64; 2],
    /// Instance and type properties as exported ("related data": elevation, mounting height ...).
    pub properties: Vec<Property>,
    pub type_properties: Vec<Property>,
}

/// One go at a task by a Revit session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub schema_version: u32,
    pub id: String,
    pub kind: TaskKind,
    pub state: TaskState,
    /// The model the task is for: what a Revit session asks for tasks by.
    pub model_id: String,
    pub room: TaskRoom,
    pub phase: String,
    #[serde(default)]
    pub overwrite_layout: bool,
    #[serde(default)]
    pub create_rls: bool,
    #[serde(default)]
    pub overwrite_rls: bool,
    /// `update_room` only: what the update changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_mode: Option<UpdateMode>,
    /// `update_room` only: report what would change and write nothing. Defaults to true.
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<BlueprintRef>,
    /// `create_rls`: the library sheet the layout was taken from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_sheet_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<TaskItem>,
    /// The sheet layouts to make (no tags in them: a tag comes from `rls.tags` and `tag_rules`, decision 24).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheets: Vec<BlueprintSheet>,
    /// `create_rls`: the sheet size asked for and where each elevation marker and its views go in
    /// this room (`service::rls_plan`). Absent on a task made before it existed, or for a sheet
    /// with nothing to place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rls: Option<RlsPlan>,
    /// `create_rls_set`: the rooms, the views and where each goes on which sheet. `room` is the lead room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rls_set: Option<RlsSetPlan>,
    /// How to tag what the sheets show, as the rules were when the task was made: one per key
    /// (`tag_rules`), for the viewport types of the sheets and, for a populate task, only the
    /// type codes it places. Where each tag goes, never which tag: that is the model's RLS settings
    /// (`tag_rules::TaskRule`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_rules: Vec<TaskRule>,
    /// What the resolver said the room changes, and any other warning from making the task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Value>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<Attempt>,
}

/// A task without its items and sheets: what a list shows.
#[derive(Debug, Clone, Serialize)]
pub struct TaskSummary {
    pub id: String,
    pub kind: TaskKind,
    pub state: TaskState,
    pub model_id: String,
    pub room: TaskRoom,
    pub phase: String,
    pub overwrite_layout: bool,
    pub create_rls: bool,
    pub overwrite_rls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_mode: Option<UpdateMode>,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<BlueprintRef>,
    pub items: usize,
    pub sheets: usize,
    pub tag_rules: usize,
    pub findings: usize,
    pub created_at: String,
    pub updated_at: String,
    pub attempts: usize,
    /// `create_rls_set`: every room of the set, the lead first, so a list can say which rooms a set covers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub set_rooms: Vec<TaskRoom>,
    /// What the last attempt said, for a glance at why a task failed or aborted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub last_messages: Vec<String>,
}

impl From<&Task> for TaskSummary {
    fn from(t: &Task) -> Self {
        TaskSummary {
            id: t.id.clone(),
            kind: t.kind,
            state: t.state,
            model_id: t.model_id.clone(),
            room: t.room.clone(),
            phase: t.phase.clone(),
            overwrite_layout: t.overwrite_layout,
            create_rls: t.create_rls,
            overwrite_rls: t.overwrite_rls,
            update_mode: t.update_mode,
            dry_run: t.dry_run,
            blueprint: t.blueprint.clone(),
            items: t.items.len(),
            // a set's own sheets, not the one template it carries
            sheets: t.rls_set.as_ref().map_or(t.sheets.len(), |s| s.sheets.len()),
            tag_rules: t.tag_rules.len(),
            findings: t.findings.len(),
            created_at: t.created_at.clone(),
            updated_at: t.updated_at.clone(),
            attempts: t.attempts.len(),
            set_rooms: t.rls_set.as_ref().map(|s| s.rooms.clone()).unwrap_or_default(),
            last_messages: t.attempts.last().map(|a| a.messages.clone()).unwrap_or_default(),
        }
    }
}

/// What a `create_rls` task's phase says: the room's own phase, read in Revit when the sheet is made. A
/// person never types one: the views of a room layout sheet belong to the phase the room does.
pub const ROOM_OWN_PHASE: &str = "(the room's own phase)";

/// What a person's request makes of a task, before it has an id or a state.
#[derive(Debug, Clone)]
pub struct NewTask {
    pub kind: TaskKind,
    pub model_id: String,
    pub room: TaskRoom,
    pub phase: String,
    pub overwrite_layout: bool,
    pub create_rls: bool,
    pub overwrite_rls: bool,
    pub update_mode: Option<UpdateMode>,
    pub dry_run: bool,
    pub blueprint: Option<BlueprintRef>,
    pub library_sheet_id: Option<String>,
    pub items: Vec<TaskItem>,
    pub sheets: Vec<BlueprintSheet>,
    pub rls: Option<RlsPlan>,
    pub rls_set: Option<RlsSetPlan>,
    pub tag_rules: Vec<TaskRule>,
    pub findings: Vec<Value>,
}

/// One report from a Revit session about a task it was handed.
#[derive(Debug, Clone, Deserialize)]
pub struct Report {
    pub task_id: String,
    pub status: TaskState,
    #[serde(default)]
    pub messages: Vec<String>,
}

/// What reporting did with one task.
#[derive(Debug, Clone, Serialize)]
pub struct ReportOutcome {
    pub task_id: String,
    pub recorded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug)]
pub enum TaskError {
    NotFileBacked,
    Invalid(String),
    NotFound(String),
    /// The task is not in a state that allows this (for example resending one already queued).
    Conflict(String),
    Internal(anyhow::Error),
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFileBacked => write!(f, "this server was not started from a settings directory"),
            Self::Invalid(m) | Self::NotFound(m) | Self::Conflict(m) => write!(f, "{m}"),
            Self::Internal(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TaskError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-tasks-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn new_task(model: &str, room: &str) -> NewTask {
        NewTask {
            kind: TaskKind::PopulateRoom,
            model_id: model.to_string(),
            room: TaskRoom {
                room_id: room.to_string(),
                number: Some("1.01".to_string()),
                name: format!("Room {room}"),
            },
            phase: "New Construction".to_string(),
            overwrite_layout: false,
            create_rls: false,
            overwrite_rls: false,
            update_mode: None,
            dry_run: false,
            blueprint: None,
            library_sheet_id: None,
            items: vec![],
            sheets: vec![],
            rls: None,
            rls_set: None,
            tag_rules: vec![],
            findings: vec![],
        }
    }

    fn report_of(id: &str, status: TaskState) -> Report {
        Report { task_id: id.to_string(), status, messages: vec!["done".to_string()] }
    }

    #[test]
    fn test_a_session_is_handed_its_models_queued_tasks_once() {
        let dir = temp_dir("claim");
        let a = create(&dir, "p", new_task("m1", "r1")).unwrap();
        create(&dir, "p", new_task("m2", "r2")).unwrap();
        let got = claim(&dir, "p", "m1", &[]).unwrap();
        assert_eq!(got.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec![a.id.as_str()]);
        assert_eq!(got[0].state, TaskState::InProgress);
        assert!(claim(&dir, "p", "m1", &[]).unwrap().is_empty(), "a second session is not handed it again");
        assert_eq!(claim(&dir, "p", "m2", &[]).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn claim_can_be_narrowed_to_a_kind_and_leaves_the_rest_queued() {
        let dir = temp_dir("claim-kind");
        let mut rls = new_task("m1", "r1");
        rls.kind = TaskKind::CreateRls;
        rls.create_rls = true;
        create(&dir, "p", rls).unwrap();
        create(&dir, "p", new_task("m1", "r2")).unwrap();
        let got = claim(&dir, "p", "m1", &[TaskKind::CreateRls]).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, TaskKind::CreateRls);
        let rest = claim(&dir, "p", "m1", &[]).unwrap();
        assert_eq!(rest.len(), 1, "the populate task was not handed over to the RLS script");
        assert_eq!(rest[0].kind, TaskKind::PopulateRoom);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn set_task(model: &str, rooms: &[&str]) -> NewTask {
        let room = |id: &str| TaskRoom { room_id: id.to_string(), number: None, name: format!("Room {id}") };
        let mut t = new_task(model, rooms[0]);
        t.kind = TaskKind::CreateRlsSet;
        t.rls_set = Some(RlsSetPlan {
            sheet_size: "a1".to_string(),
            paper_mm: [841.0, 594.0],
            lead_room: rooms[0].to_string(),
            rooms: rooms.iter().map(|r| room(r)).collect(),
            plan: None,
            markers: vec![],
            schedules: vec![],
            three_d: false,
            sheets: vec![],
        });
        t
    }

    #[test]
    fn two_live_sets_may_not_share_a_room_and_resend_says_so_too() {
        let dir = temp_dir("sets");
        let first = create(&dir, "p", set_task("m1", &["a", "b"])).unwrap();
        // b is not the lead of either set: it is the overlap that counts, not the lead room
        let clash = create(&dir, "p", set_task("m1", &["c", "b"]));
        assert!(
            matches!(&clash, Err(TaskError::Conflict(m)) if m.contains("Room b")),
            "names the shared room: {clash:?}"
        );
        create(&dir, "p", set_task("m1", &["c", "d"])).expect("no room in common");
        create(&dir, "p", set_task("m2", &["a", "b"])).expect("another model's rooms");
        let mut single = new_task("m1", "b");
        single.kind = TaskKind::CreateRls;
        create(&dir, "p", single).expect("a one-room sheet of a room in a set is another kind of task");
        assert_eq!(TaskSummary::from(&first).set_rooms.len(), 2);

        dismiss(&dir, "p", &first.id).unwrap();
        let again = create(&dir, "p", set_task("m1", &["e", "a"])).unwrap();
        assert!(
            matches!(resend(&dir, "p", &first.id), Err(TaskError::Conflict(_))),
            "a is in a live set again"
        );
        dismiss(&dir, "p", &again.id).unwrap();
        resend(&dir, "p", &first.id).expect("nothing live holds a or b now");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn claim_can_be_narrowed_to_the_tasks_a_person_ticked() {
        let dir = temp_dir("claim-ids");
        let a = create(&dir, "p", new_task("m1", "r1")).unwrap();
        let b = create(&dir, "p", new_task("m1", "r2")).unwrap();
        let got = claim_chosen(&dir, "p", "m1", &[], &[b.id.clone(), "gone".to_string()]).unwrap();
        assert_eq!(got.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec![b.id.as_str()]);
        let rest = claim(&dir, "p", "m1", &[]).unwrap();
        assert_eq!(
            rest.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec![a.id.as_str()],
            "the unticked one stays queued"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_report_records_the_result_and_only_for_a_task_in_progress() {
        let dir = temp_dir("report");
        let t = create(&dir, "p", new_task("m1", "r1")).unwrap();
        let early = report(&dir, "p", vec![report_of(&t.id, TaskState::Completed)]).unwrap();
        assert!(!early[0].recorded, "not handed out yet");
        claim(&dir, "p", "m1", &[]).unwrap();
        let bad_status = report(&dir, "p", vec![report_of(&t.id, TaskState::Queued)]).unwrap();
        assert!(!bad_status[0].recorded);
        let ok = report(
            &dir,
            "p",
            vec![
                report_of(&t.id, TaskState::CompletedWithWarnings),
                report_of("nope", TaskState::Failed),
            ],
        )
        .unwrap();
        assert!(ok[0].recorded && !ok[1].recorded, "one bad report does not cost the others");
        let after = load(&dir, "p", &t.id).unwrap();
        assert_eq!(after.state, TaskState::CompletedWithWarnings);
        assert_eq!(after.attempts.len(), 1);
        assert_eq!(after.attempts[0].messages, vec!["done".to_string()]);
        assert!(after.attempts[0].finished_at.is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_dismiss_and_resend_move_a_task_and_keep_its_attempts() {
        let dir = temp_dir("resend");
        let t = create(&dir, "p", new_task("m1", "r1")).unwrap();
        claim(&dir, "p", "m1", &[]).unwrap();
        // A session that died holding it: a person puts it back.
        assert_eq!(resend(&dir, "p", &t.id).unwrap().state, TaskState::Queued);
        assert!(matches!(resend(&dir, "p", &t.id), Err(TaskError::Conflict(_))), "already queued");
        claim(&dir, "p", "m1", &[]).unwrap();
        report(&dir, "p", vec![report_of(&t.id, TaskState::Failed)]).unwrap();
        assert_eq!(dismiss(&dir, "p", &t.id).unwrap().state, TaskState::Dismissed);
        assert!(matches!(dismiss(&dir, "p", &t.id), Err(TaskError::Conflict(_))));
        assert_eq!(load(&dir, "p", &t.id).unwrap().attempts.len(), 2, "attempts are kept");
        // A late report for a dismissed task is refused.
        assert!(!report(&dir, "p", vec![report_of(&t.id, TaskState::Completed)]).unwrap()[0].recorded);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_second_live_task_for_the_same_room_is_refused() {
        let dir = temp_dir("clash");
        let first = create(&dir, "p", new_task("m1", "r1")).unwrap();
        assert!(matches!(create(&dir, "p", new_task("m1", "r1")), Err(TaskError::Conflict(_))));
        create(&dir, "p", new_task("m1", "r2")).unwrap();
        dismiss(&dir, "p", &first.id).unwrap();
        let again = create(&dir, "p", new_task("m1", "r1")).unwrap();
        // The dismissed one cannot be resent while a live one exists for the room.
        assert!(matches!(resend(&dir, "p", &first.id), Err(TaskError::Conflict(_))));
        assert_ne!(first.id, again.id);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_the_list_narrows_by_model_and_state_and_a_task_needs_a_phase() {
        let dir = temp_dir("list");
        create(&dir, "p", new_task("m1", "r1")).unwrap();
        create(&dir, "p", new_task("m2", "r2")).unwrap();
        claim(&dir, "p", "m2", &[]).unwrap();
        assert_eq!(list(&dir, "p", Some("m1"), None).unwrap().len(), 1);
        assert_eq!(list(&dir, "p", None, Some(TaskState::InProgress)).unwrap().len(), 1);
        assert_eq!(list(&dir, "p", None, None).unwrap().len(), 2);
        let mut blank = new_task("m1", "r3");
        blank.phase = "  ".to_string();
        assert!(matches!(create(&dir, "p", blank), Err(TaskError::Invalid(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_an_update_task_keeps_its_mode_and_is_a_separate_live_task_from_a_populate() {
        let dir = temp_dir("update");
        create(&dir, "p", new_task("m1", "r1")).unwrap();
        let mut update = new_task("m1", "r1");
        update.kind = TaskKind::UpdateRoom;
        update.update_mode = Some(UpdateMode::Locations);
        update.dry_run = true;
        let made = create(&dir, "p", update.clone()).expect("a different kind for the same room is allowed");
        assert_eq!(made.update_mode, Some(UpdateMode::Locations));
        assert!(made.dry_run);
        let again = create(&dir, "p", update);
        assert!(matches!(again, Err(TaskError::Conflict(_))), "{again:?}");
        let read = load(&dir, "p", &made.id).unwrap();
        assert_eq!(read.update_mode, Some(UpdateMode::Locations));
        assert_eq!(TaskSummary::from(&read).update_mode, Some(UpdateMode::Locations));
    }

    fn rls_task_with_viewports() -> NewTask {
        let mut t = new_task("m1", "r1");
        t.kind = TaskKind::CreateRls;
        t.create_rls = true;
        t.sheets = vec![BlueprintSheet {
            revit_sheet_id: "1".to_string(),
            document_title: "D".to_string(),
            sheet_number: "S".to_string(),
            sheet_name: String::new(),
            size_mm: None,
            room_summary: None,
            room_outline: Vec::new(),
            sheet: serde_json::json!({"view_ports": [{"view_id": 7}]}),
        }];
        t
    }

    #[test]
    fn test_moving_a_viewport_is_saved_replaced_and_only_for_a_queued_task() {
        let dir = temp_dir("move");
        let made = create(&dir, "p", rls_task_with_viewports()).unwrap();
        let moved = move_viewports(
            &dir,
            "p",
            &made.id,
            ViewportMoves {
                moves: Some(vec![ViewportMove { view_id: 7, centre_mm: [100.0, 50.0] }]),
                set_moves: vec![],
            },
        )
        .unwrap();
        assert_eq!(moved.state, TaskState::Queued);
        assert_eq!(
            moved.rls.as_ref().unwrap().viewport_moves,
            vec![ViewportMove { view_id: 7, centre_mm: [100.0, 50.0] }]
        );
        assert_eq!(load(&dir, "p", &made.id).unwrap().rls.unwrap().viewport_moves.len(), 1, "it is stored");
        // a request that leaves `moves` out keeps them, and an empty list puts every viewport back
        let kept = move_viewports(&dir, "p", &made.id, ViewportMoves::default()).unwrap();
        assert_eq!(kept.rls.unwrap().viewport_moves.len(), 1);
        let cleared =
            move_viewports(&dir, "p", &made.id, ViewportMoves { moves: Some(vec![]), set_moves: vec![] }).unwrap();
        assert!(cleared.rls.unwrap().viewport_moves.is_empty());
        // not a place on a sheet, and the same viewport twice
        for bad_moves in [
            vec![ViewportMove { view_id: 7, centre_mm: [f64::NAN, 0.0] }],
            vec![ViewportMove { view_id: 7, centre_mm: [1e9, 0.0] }],
            vec![
                ViewportMove { view_id: 7, centre_mm: [1.0, 1.0] },
                ViewportMove { view_id: 7, centre_mm: [2.0, 2.0] },
            ],
        ] {
            let r = move_viewports(&dir, "p", &made.id, ViewportMoves { moves: Some(bad_moves), set_moves: vec![] });
            assert!(matches!(r, Err(TaskError::Invalid(_))));
        }
        // a task a session holds is not edited under it
        claim(&dir, "p", "m1", &[]).unwrap();
        let r = move_viewports(&dir, "p", &made.id, ViewportMoves { moves: Some(vec![]), set_moves: vec![] });
        assert!(matches!(r, Err(TaskError::Conflict(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_kind_with_no_sheet_refuses_viewport_moves() {
        let dir = temp_dir("move-kind");
        let made = create(&dir, "p", new_task("m1", "r1")).unwrap();
        let r = move_viewports(&dir, "p", &made.id, ViewportMoves { moves: Some(vec![]), set_moves: vec![] });
        assert!(matches!(r, Err(TaskError::Invalid(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Only a queued create-RLS task's 3D views can be aimed; the stored plan keeps everything else.
    #[test]
    fn test_only_a_queued_create_rls_task_can_be_aimed() {
        let dir = temp_dir("aim");
        let mut new = new_task("m", "1");
        new.kind = TaskKind::CreateRls;
        new.rls = Some(RlsPlan { room_outline: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], ..Default::default() });
        let task = create(&dir, "p", new).unwrap();
        check_aimable(&task).unwrap();
        let aimed = replace_three_ds(&dir, "p", &task.id, vec![]).unwrap();
        assert_eq!(3, aimed.rls.unwrap().room_outline.len(), "the outline is kept");

        let populate = create(&dir, "p", new_task("m", "2")).unwrap();
        assert!(matches!(check_aimable(&populate), Err(TaskError::Invalid(_))));
        dismiss(&dir, "p", &task.id).unwrap();
        assert!(matches!(replace_three_ds(&dir, "p", &task.id, vec![]), Err(TaskError::Conflict(_))));
    }
}
