//! What would change in every room a blueprint is assigned to, if its assignments moved to another revision: a
//! DRY RUN, per room, and nothing is written, queued or stored.
//!
//! Per assignment it resolves the target revision with the room's own mapping, overrides and additions, audits
//! the room against that layout (`blueprint_audit::audit_content`, the same comparison the audit read makes) and
//! reduces the verdicts to what a person would approve (`Action`). Beside it rides the revision diff from the
//! revision the assignment pins to the target (`blueprint_diff`), so a person reads WHY a room is on the list.
//!
//! **What is planned is only what the audit can prove without a ledger.** Nothing records which element Revit
//! placed (plan step 4), so an element is never planned for removal, retype or replacement here: `extra`, `surplus`,
//! `foreign`, `substituted` and `kit_differs` are `report`, which says "look at this" and does nothing. A `move` is
//! a right element a little out of place; an `add` is a layout item nothing in the room is. A family reload is
//! refused (plan 5.4), so a kit that differs is a report whatever the revisions did.
//!
//! **A held override is a decision, not an action.** A nudge made against a recipe the target moved is held by the
//! resolver (`override_stale`) and listed in `held`; the room's status is `needs_decision` until a person says whether
//! it still fits.

use std::collections::BTreeMap;

use serde::Serialize;

use super::blueprint_audit::{audit_basis, audit_content, RoomState, Verdict, VerdictKind};
use super::blueprint_diff::{diff, ItemDiff, OutlineDiff, RevisionDiff};
use super::blueprint_model::RevisionContent;
use super::blueprint_resolve::{Finding, FitKind};
use super::ServiceError;
use crate::assignments::{self, RoomKey};
use crate::blueprints::{self, BlueprintError};
use crate::state::AppState;

/// Which content the assignments would move to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// An accepted revision.
    Revision(u32),
    /// A pushed revision still waiting for review: the question a person asks BEFORE accepting it.
    Pending(u32),
}

/// What a person would approve for one host or item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// A layout item nothing in the room is: place it.
    Add,
    /// The right element, near its place: move it.
    Move,
    /// Something to look at that nothing is done about without a ledger.
    Report,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlannedAction {
    pub action: Action,
    /// The audit's verdict this came from, whole: the ids, distances and kit diff a person needs.
    pub verdict: Verdict,
}

/// Where a room stands against the target, worst first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomStatus {
    /// The assignment could not be read against the target (the pinned revision is gone, or the audit refused).
    Error,
    /// The room is not in the model any more.
    RoomGone,
    /// The target does not fit the room (`edge_count_differs`, `mirror_image`, `no_fit`): nothing to place.
    CannotJudge,
    /// An override is held against a recipe the target moved: a person decides before anything is planned for it.
    NeedsDecision,
    /// There are actions to approve.
    Changes,
    /// The room already looks like the target, up to reports.
    NoChange,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoomImpact {
    pub assignment_id: String,
    pub room: RoomKey,
    pub room_name: String,
    pub pinned_revision: u32,
    pub status: RoomStatus,
    /// Why, for `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<FitKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<RoomState>,
    /// Items whose nudge or turn is held (`override_stale`).
    pub held: Vec<String>,
    /// What the resolver said about the target in this room, `override_stale` aside.
    pub findings: Vec<Finding>,
    /// The blueprint's changes between the pinned revision and the target, for the items this room places (and the
    /// ones it no longer does).
    pub item_changes: Vec<ItemDiff>,
    pub plan: Vec<PlannedAction>,
    pub plan_counts: BTreeMap<Action, usize>,
    /// The audit's verdicts by kind, `in_place` and the flags included: what the plan leaves alone.
    pub verdict_counts: BTreeMap<VerdictKind, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetInfo {
    /// The revision number the target is, or would become if accepted.
    pub revision: u32,
    /// Set when the target is a pending revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlueprintImpact {
    pub blueprint_id: String,
    pub blueprint_name: String,
    pub current_revision: u32,
    pub target: TargetInfo,
    pub outline: OutlineDiff,
    pub rooms: Vec<RoomImpact>,
    /// Rooms by status.
    pub status_counts: BTreeMap<RoomStatus, usize>,
}

fn refuse<T>(reason: impl Into<String>) -> Result<T, ServiceError> {
    Err(ServiceError::Invalid(reason.into()))
}

/// The blueprint and the content of `target`, with the number the target is or would be.
fn load_target(
    projects_dir: &std::path::Path,
    project: &str,
    blueprint_id: &str,
    target: Target,
) -> Result<(blueprints::Blueprint, RevisionContent, TargetInfo), ServiceError> {
    let blueprint = match blueprints::load(projects_dir, project, blueprint_id) {
        Ok(b) => b,
        Err(BlueprintError::NotFound(_)) => return refuse("no such blueprint"),
        Err(e) => return Err(super::task_build::blueprint_error(e)),
    };
    let (content, info) = match target {
        Target::Revision(n) => match blueprint.revision(n) {
            Some(r) => (r.content.clone(), TargetInfo { revision: n, pending_id: None }),
            None => return refuse(format!("the blueprint has no revision {n}")),
        },
        Target::Pending(id) => match blueprint.pending.iter().find(|p| p.pending_id == id) {
            // Accepting makes it the next revision, which is the number a person will see.
            Some(p) => {
                let next = blueprint.revisions.iter().map(|r| r.number).max().unwrap_or(0) + 1;
                (p.content.clone(), TargetInfo { revision: next, pending_id: Some(id) })
            }
            None => return refuse(format!("the blueprint has no pending revision {id}")),
        },
    };
    Ok((blueprint, content, info))
}

/// The revision diff from `from` to `to`. Left out, `from` is the revision before `to`, or the current one when `to`
/// is pending: the question a person asks of a revision, "what does this change".
pub fn blueprint_diff(
    state: &AppState,
    projects_dir: &std::path::Path,
    project: &str,
    blueprint_id: &str,
    from: Option<u32>,
    to: Target,
) -> Result<RevisionDiff, ServiceError> {
    let (blueprint, content, info) = load_target(projects_dir, project, blueprint_id, to)?;
    let from = match from {
        Some(n) => n,
        None if info.pending_id.is_some() => blueprint.current_revision,
        None => match blueprint.revisions.iter().map(|r| r.number).filter(|n| *n < info.revision).max() {
            Some(n) => n,
            None => {
                return refuse(format!(
                    "revision {} is the first: there is nothing before it to compare with",
                    info.revision
                ))
            }
        },
    };
    let Some(old) = blueprint.revision(from) else {
        return refuse(format!("the blueprint has no revision {from}"));
    };
    let near = state
        .settings()
        .settings_for(project)
        .map(|b| b.blueprint_audit.near_tolerance_mm)
        .unwrap_or_default();
    let near = if near > 0.0 { near } else { 600.0 };
    Ok(diff(&old.content, &content, near))
}

/// Every assignment of a blueprint, read against `target`.
pub fn impact(
    state: &AppState,
    projects_dir: &std::path::Path,
    project: &str,
    blueprint_id: &str,
    target: Target,
) -> Result<BlueprintImpact, ServiceError> {
    let (blueprint, content, info) = load_target(projects_dir, project, blueprint_id, target)?;
    let (parameter, policy) = audit_basis(state, project)?;
    let mine: Vec<assignments::Assignment> = match assignments::list_raw(projects_dir, project) {
        Ok(all) => all.into_iter().filter(|a| a.blueprint_id == blueprint.id).collect(),
        Err(e) => return Err(super::task_build::blueprint_error(e)),
    };

    // One diff per pinned revision, not per room.
    let mut diffs: BTreeMap<u32, Option<RevisionDiff>> = BTreeMap::new();
    let mut rooms = Vec::new();
    for assignment in &mine {
        let from = diffs.entry(assignment.revision).or_insert_with(|| {
            blueprint
                .revision(assignment.revision)
                .map(|r| diff(&r.content, &content, policy.near_tolerance_mm))
        });
        let room =
            match audit_content(state, project, assignment, &blueprint, info.revision, &content, &parameter, &policy) {
                Ok(Some((audit, resolution))) => room_impact(assignment, from.as_ref(), audit, resolution),
                Ok(None) => bare(assignment, RoomStatus::RoomGone, None),
                Err(ServiceError::Invalid(why)) => bare(assignment, RoomStatus::Error, Some(why)),
                Err(e) => return Err(e),
            };
        rooms.push(room);
    }
    rooms.sort_by(|a, b| (a.status, &a.room_name).cmp(&(b.status, &b.room_name)));

    let mut status_counts: BTreeMap<RoomStatus, usize> = BTreeMap::new();
    for r in &rooms {
        *status_counts.entry(r.status).or_default() += 1;
    }
    // The outline is a fact about the two revisions, so read it from the first diff that exists; with no
    // assignment there is nothing to compare against, and the diff read answers that question.
    let outline = diffs.values().flatten().next().map(|d| d.outline.clone()).unwrap_or(OutlineDiff {
        old_edges: 0,
        new_edges: content.outline.coarse.len(),
        changed: false,
    });
    Ok(BlueprintImpact {
        blueprint_id: blueprint.id.clone(),
        blueprint_name: blueprint.name.clone(),
        current_revision: blueprint.current_revision,
        target: info,
        outline,
        rooms,
        status_counts,
    })
}

fn bare(a: &assignments::Assignment, status: RoomStatus, reason: Option<String>) -> RoomImpact {
    RoomImpact {
        assignment_id: a.id.clone(),
        room: a.room.clone(),
        room_name: a.room_name.clone(),
        pinned_revision: a.revision,
        status,
        reason,
        fit: None,
        state: None,
        held: Vec::new(),
        findings: Vec::new(),
        item_changes: Vec::new(),
        plan: Vec::new(),
        plan_counts: BTreeMap::new(),
        verdict_counts: BTreeMap::new(),
    }
}

/// The action a verdict asks for, if it asks for one.
fn action_of(kind: VerdictKind) -> Option<Action> {
    match kind {
        VerdictKind::Missing => Some(Action::Add),
        VerdictKind::Moved => Some(Action::Move),
        // Without a ledger none of these may be acted on: they are not known to be the blueprint's.
        VerdictKind::Extra
        | VerdictKind::Surplus
        | VerdictKind::Foreign
        | VerdictKind::Substituted
        | VerdictKind::KitDiffers => Some(Action::Report),
        // Right as it is, a room the target does not fit (the status says so) or a flag nothing is done about.
        VerdictKind::InPlace | VerdictKind::PresentByCode | VerdictKind::NoCode | VerdictKind::UnsupportedHost => None,
    }
}

fn room_impact(
    assignment: &assignments::Assignment,
    from: Option<&RevisionDiff>,
    audit: super::blueprint_audit::AssignmentAudit,
    resolution: super::blueprint_resolve::Resolution,
) -> RoomImpact {
    let mut held = Vec::new();
    let mut findings = Vec::new();
    for f in resolution.findings {
        match f {
            Finding::OverrideStale { item } => held.push(item),
            other => findings.push(other),
        }
    }
    let placed: std::collections::HashSet<&str> = resolution.placements.iter().map(|p| p.item_id.as_str()).collect();
    let item_changes: Vec<ItemDiff> = from
        .map(|d| {
            d.changed()
                // An item this room places, or one the target retired (which it no longer places).
                .filter(|c| {
                    placed.contains(c.item_id.as_str())
                        || c.changes.contains(&super::blueprint_diff::ChangeKind::Retired)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let plan: Vec<PlannedAction> = audit
        .audit
        .verdicts
        .iter()
        .filter_map(|v| action_of(v.kind).map(|action| PlannedAction { action, verdict: v.clone() }))
        .collect();
    let mut plan_counts: BTreeMap<Action, usize> = BTreeMap::new();
    for p in &plan {
        *plan_counts.entry(p.action).or_default() += 1;
    }
    let acts = plan.iter().any(|p| p.action != Action::Report);
    let status = if matches!(audit.fit, FitKind::EdgeCountDiffers | FitKind::MirrorImage | FitKind::NoFit) {
        RoomStatus::CannotJudge
    } else if !held.is_empty() {
        RoomStatus::NeedsDecision
    } else if acts {
        RoomStatus::Changes
    } else {
        RoomStatus::NoChange
    };
    RoomImpact {
        assignment_id: assignment.id.clone(),
        room: assignment.room.clone(),
        room_name: assignment.room_name.clone(),
        pinned_revision: assignment.revision,
        status,
        reason: None,
        fit: Some(audit.fit),
        state: Some(audit.audit.state),
        held,
        findings,
        item_changes,
        plan,
        plan_counts,
        verdict_counts: audit.audit.counts.clone(),
    }
}

#[cfg(test)]
mod tests;
