//! A blueprint's history: revisions are append-only, a push from Revit waits as PENDING until a person accepts or rejects it, and an
//! edit is always a new revision. `create`, `add_revision`, `push`, `accept`, `reject`, `update`.

use super::*;

pub fn create(projects_dir: &Path, project: &str, request: CreateRequest) -> Result<Blueprint, BlueprintError> {
    let name = clean_name(&request.name)?;
    let note = clean_note(request.note)?;
    validate_content(&request.content)?;
    let _guard = SAVE_LOCK.lock().unwrap();
    let dir = project_dir(projects_dir, project)?;
    if list(projects_dir, project)?.len() >= MAX_BLUEPRINTS_PER_PROJECT {
        return bad(format!("a project holds at most {MAX_BLUEPRINTS_PER_PROJECT} blueprints"));
    }
    let taken_at = now();
    let blueprint = Blueprint {
        schema_version: SCHEMA_VERSION,
        id: new_id(&dir),
        name,
        taken_at: taken_at.clone(),
        source: request.source,
        current_revision: 1,
        in_use: true,
        revisions: vec![Revision { number: 1, created_at: taken_at, note, content: request.content }],
        pending: Vec::new(),
        rejected: Vec::new(),
        last_pending_id: 0,
    };
    write(projects_dir, project, &blueprint)?;
    tracing::info!("created blueprint {} ({:?}) in project {project}", blueprint.id, blueprint.name);
    Ok(blueprint)
}

/// Append a revision. Revisions are never edited in place.
pub fn add_revision(
    projects_dir: &Path,
    project: &str,
    id: &str,
    request: AddRevisionRequest,
) -> Result<Blueprint, BlueprintError> {
    let note = clean_note(request.note)?;
    validate_content(&request.content)?;
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut blueprint = load(projects_dir, project, id)?;
    if blueprint.taken_at != request.base {
        return Err(conflict(&request.base, &blueprint.taken_at));
    }
    if blueprint.revisions.len() >= MAX_REVISIONS {
        return bad(format!("a blueprint holds at most {MAX_REVISIONS} revisions"));
    }
    let number = blueprint.revisions.iter().map(|r| r.number).max().unwrap_or(0) + 1;
    blueprint.taken_at = next_stamp(&blueprint.taken_at);
    let content = request.content;
    blueprint
        .revisions
        .push(Revision { number, created_at: blueprint.taken_at.clone(), note, content });
    if request.make_current {
        blueprint.current_revision = number;
    }
    write(projects_dir, project, &blueprint)?;
    Ok(blueprint)
}

/// Store a revision that arrived from elsewhere as **pending**. It is not a revision, is not
/// current, and no assignment can pin it; it waits for `accept` or `reject`.
pub fn push(projects_dir: &Path, project: &str, id: &str, request: PushRequest) -> Result<Blueprint, BlueprintError> {
    let note = clean_note(request.note)?;
    let origin = clean_note(request.origin)?;
    validate_content(&request.content)?;
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut blueprint = load(projects_dir, project, id)?;
    if blueprint.pending.len() >= MAX_PENDING {
        return bad(format!("a blueprint holds at most {MAX_PENDING} pending revisions; review some first"));
    }
    let pending_id = blueprint.last_pending_id + 1;
    blueprint.last_pending_id = pending_id;
    blueprint.taken_at = next_stamp(&blueprint.taken_at);
    blueprint.pending.push(PendingRevision {
        pending_id,
        received_at: blueprint.taken_at.clone(),
        origin,
        note,
        bundle: request.bundle,
        content: request.content,
    });
    write(projects_dir, project, &blueprint)?;
    tracing::info!("blueprint {id} in project {project} received pending revision {pending_id}");
    Ok(blueprint)
}

pub(super) fn take_pending(blueprint: &mut Blueprint, pending_id: u32) -> Result<PendingRevision, BlueprintError> {
    match blueprint.pending.iter().position(|p| p.pending_id == pending_id) {
        Some(i) => Ok(blueprint.pending.remove(i)),
        None => bad(format!("the blueprint has no pending revision {pending_id}")),
    }
}

/// Accept a pending revision: it becomes the next revision and the current one.
pub fn accept(
    projects_dir: &Path,
    project: &str,
    id: &str,
    pending_id: u32,
    request: DecideRequest,
) -> Result<Blueprint, BlueprintError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut blueprint = load(projects_dir, project, id)?;
    if blueprint.taken_at != request.base {
        return Err(conflict(&request.base, &blueprint.taken_at));
    }
    if blueprint.revisions.len() >= MAX_REVISIONS {
        return bad(format!("a blueprint holds at most {MAX_REVISIONS} revisions"));
    }
    let accepted = take_pending(&mut blueprint, pending_id)?;
    let number = blueprint.revisions.iter().map(|r| r.number).max().unwrap_or(0) + 1;
    blueprint.taken_at = next_stamp(&blueprint.taken_at);
    blueprint.revisions.push(Revision {
        number,
        created_at: blueprint.taken_at.clone(),
        note: accepted.note,
        content: accepted.content,
    });
    blueprint.current_revision = number;
    write(projects_dir, project, &blueprint)?;
    Ok(blueprint)
}

/// Reject a pending revision. It is kept, marked rejected, and the history is untouched.
pub fn reject(
    projects_dir: &Path,
    project: &str,
    id: &str,
    pending_id: u32,
    request: DecideRequest,
) -> Result<Blueprint, BlueprintError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut blueprint = load(projects_dir, project, id)?;
    if blueprint.taken_at != request.base {
        return Err(conflict(&request.base, &blueprint.taken_at));
    }
    let mut declined = take_pending(&mut blueprint, pending_id)?;
    declined.bundle = None; // a rejected revision's sheets and tags are never applied, so are not kept
    blueprint.taken_at = next_stamp(&blueprint.taken_at);
    blueprint
        .rejected
        .push(RejectedRevision { rejected_at: blueprint.taken_at.clone(), revision: declined });
    write(projects_dir, project, &blueprint)?;
    Ok(blueprint)
}

/// Rename, and/or move which revision is current.
pub fn update(
    projects_dir: &Path,
    project: &str,
    id: &str,
    request: UpdateRequest,
) -> Result<Blueprint, BlueprintError> {
    let name = request.name.as_deref().map(clean_name).transpose()?;
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut blueprint = load(projects_dir, project, id)?;
    if blueprint.taken_at != request.base {
        return Err(conflict(&request.base, &blueprint.taken_at));
    }
    if let Some(n) = request.current_revision {
        if blueprint.revision(n).is_none() {
            return bad(format!("the blueprint has no revision {n}"));
        }
        blueprint.current_revision = n;
    }
    if let Some(name) = name {
        blueprint.name = name;
    }
    if let Some(in_use) = request.in_use {
        blueprint.in_use = in_use;
    }
    blueprint.taken_at = next_stamp(&blueprint.taken_at);
    write(projects_dir, project, &blueprint)?;
    Ok(blueprint)
}
