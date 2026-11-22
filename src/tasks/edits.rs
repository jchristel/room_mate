//! What a person changes about a QUEUED task: where its viewports go on the sheet (`move_viewports`) and which way its 3D
//! views look (`replace_three_ds`). Each leaves everything else about the task as it was and the task queued.

use super::*;

/// What a person changed about where a task's viewports go: the tasks page's drag, saved.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ViewportMoves {
    /// `create_rls`: the moved viewports by the template viewport's id. REPLACES the list the task holds, so `[]`
    /// puts every viewport back where the template has it; absent leaves it.
    #[serde(default)]
    pub moves: Option<Vec<ViewportMove>>,
    /// `create_rls_set`: the centre of sheet `sheet`'s view number `view`. The set's layout IS the task, so these
    /// are written into it.
    #[serde(default)]
    pub set_moves: Vec<SetMove>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SetMove {
    pub sheet: usize,
    pub view: usize,
    pub centre_mm: [f64; 2],
}

/// Far beyond any sheet: a centre outside it is a client fault, not a layout.
pub(super) const MOVE_LIMIT_MM: f64 = 20_000.0;

pub(super) fn check_centre(centre: [f64; 2]) -> Result<(), TaskError> {
    if centre.iter().all(|v| v.is_finite() && v.abs() <= MOVE_LIMIT_MM) {
        Ok(())
    } else {
        bad(format!("a viewport centre of {centre:?} is not a place on a sheet"))
    }
}

/// A person moves viewports on a queued task's sheet, to fix one that reaches past the paper. Only a queued task: a
/// session holds the others, and a finished one is history. Where a viewport goes is a decision about THIS task, so
/// nothing else changes and the task stays queued.
pub fn move_viewports(projects_dir: &Path, project: &str, id: &str, request: ViewportMoves) -> Result<Task, TaskError> {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut task = load(projects_dir, project, id)?;
    if task.state != TaskState::Queued {
        return Err(TaskError::Conflict("only a queued task's viewports can be moved".to_string()));
    }
    match task.kind {
        TaskKind::CreateRls => {
            if !request.set_moves.is_empty() {
                return bad("a create-RLS task has no set sheets: send `moves`".to_string());
            }
            let Some(moves) = request.moves else {
                return Ok(task);
            };
            let mut seen = std::collections::HashSet::new();
            for m in &moves {
                check_centre(m.centre_mm)?;
                if !seen.insert(m.view_id) {
                    return bad(format!("viewport {} is moved twice", m.view_id));
                }
            }
            if !task.sheets.iter().any(|s| s.sheet.get("view_ports").is_some_and(|p| p.is_array())) && !moves.is_empty()
            {
                return bad("the task's sheet has no viewports to move".to_string());
            }
            task.rls.get_or_insert_with(RlsPlan::default).viewport_moves = moves;
        }
        TaskKind::CreateRlsSet => {
            if request.moves.is_some() {
                return bad("a set task's viewports are moved by sheet and view: send `set_moves`".to_string());
            }
            let set = task
                .rls_set
                .as_mut()
                .ok_or_else(|| TaskError::Invalid("the task holds no set layout".to_string()))?;
            for m in &request.set_moves {
                check_centre(m.centre_mm)?;
                let view = set.sheets.get_mut(m.sheet).and_then(|s| s.views.get_mut(m.view));
                match view {
                    Some(v) => v.centre_mm = m.centre_mm,
                    None => return bad(format!("the set has no view {} on sheet {}", m.view, m.sheet)),
                }
            }
        }
        _ => return bad("this kind of task has no sheet to move viewports on".to_string()),
    }
    task.updated_at = now();
    write(projects_dir, project, &task)?;
    Ok(task)
}

/// A direction a person chose for one of a task's 3D views, in plan, in the target model's frame (see `aim_cameras`).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct CameraAim {
    /// The 3D view, by its place among the template's 3D viewports, from 1.
    pub index: usize,
    pub direction: [f64; 2],
}

/// The body of the aim request: EVERY aim the task should keep. A view not named looks along the template's direction.
#[derive(Debug, Clone, Deserialize)]
pub struct CameraAims {
    pub cameras: Vec<CameraAim>,
}

/// A queued create-RLS task may be aimed: a session holds the others, and a finished one is history.
pub fn check_aimable(task: &Task) -> Result<(), TaskError> {
    if task.state != TaskState::Queued {
        return Err(TaskError::Conflict("only a queued task's 3D views can be aimed".to_string()));
    }
    if task.kind != TaskKind::CreateRls {
        return bad("only a create-RLS task has 3D views to aim".to_string());
    }
    Ok(())
}

/// Store the 3D views worked out again for the directions a person chose (`task_build::aim_three_ds`). Nothing else
/// about the task changes and it stays queued.
pub fn replace_three_ds(
    projects_dir: &Path,
    project: &str,
    id: &str,
    plans: Vec<crate::service::rls_three_d::ThreeDPlan>,
) -> Result<Task, TaskError> {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut task = load(projects_dir, project, id)?;
    check_aimable(&task)?;
    task.rls.get_or_insert_with(RlsPlan::default).three_ds = plans;
    task.updated_at = now();
    write(projects_dir, project, &task)?;
    Ok(task)
}
