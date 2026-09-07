//! Putting a project's linked models into ONE coordinate frame, without ever
//! putting survey coordinates on the wire.
//!
//! **The bug this exists for.** `model_to_shared` has always ridden the
//! envelope, and nothing applied it to the geometry a consumer draws. That is
//! invisible while a project's models are exported from one origin — RHH's four
//! architectural models agree to six decimal places, so they line up by luck —
//! and it is glaring the moment they are not: RHH's five services models sit
//! ~250 ft east of the architectural ones, and up to 16 ft from each other, so
//! the spaces layer drew a whole hospital's worth of outlines beside the plan
//! instead of over it.
//!
//! **Why not simply place everything into shared space.** Because shared space
//! IS the survey grid, and RHH's is MGA Sydney: `(1_008_718, 20_572_195)` in
//! feet. The renderer uploads world coordinates as `Float32Array` vertex
//! attributes (`gl/lines.ts`, `gl/fills.ts`), and f32 near 2.06e7 has a ULP of
//! **1.06 ft — 324 mm**. Every wall in the building would snap to a foot-scale
//! grid in the northing direction. Measured, not reasoned: 27 mm of
//! quantisation in easting and 324 mm in northing, against 0.005 mm for the
//! model-space coordinates the viewer draws today. f64 would be fine; the
//! renderer is not f64, and it cannot be — the attribute format is float32.
//!
//! **So: a project-LOCAL frame.** One model is the anchor, and every model's
//! geometry is mapped into the anchor's own model space by composing
//! `anchor⁻¹ ∘ model` (in f64, where the near-cancelling survey translations
//! carry ~9 significant digits and leave sub-micron residue). The models align
//! exactly as they would in shared space, and the numbers stay in the ±400 ft
//! range they already occupy. The anchor's own geometry is byte-identical to
//! what it was before, which is why House A and the renderer's golden SVGs do
//! not move.

use std::collections::BTreeMap;

use crate::contract::ModelToShared;
use crate::state::AppState;

/// Per-model transforms into one project-local frame, keyed by
/// `(project id, model id)`.
///
/// A model absent from the map needs no transform: either it declared no
/// `model_to_shared`, or it *is* the anchor. Both are "draw it as it came", and
/// collapsing them is deliberate — an absent entry is the common case and
/// looking it up must not allocate an identity.
#[derive(Debug, Default, Clone)]
pub struct Placement {
    by_model: BTreeMap<(String, String), ModelToShared>,
}

impl Placement {
    /// This model's transform, or `None` when its geometry is already in the
    /// project's frame.
    pub fn for_model(&self, project_id: &str, model_id: &str) -> Option<&ModelToShared> {
        self.by_model.get(&(project_id.to_string(), model_id.to_string()))
    }

    /// Whether anything at all needs moving. A project whose models share an
    /// origin — and every single-model project — answers `false`, so the whole
    /// geometry walk is skipped.
    pub fn is_empty(&self) -> bool {
        self.by_model.is_empty()
    }
}

/// Choose each project's anchor and build the transforms into its frame.
///
/// `models` is `(project id, model id, its declared placement)` for everything
/// in scope, in any order.
///
/// **The anchor is `[project] anchor_model` where the project names one, and
/// otherwise the lexicographically smallest model id that declares a
/// placement.** It has to be deterministic either way, because it defines the
/// frame every coordinate in the answer is expressed in — a frame that moved
/// between two reads would make a milestone comparison diff geometry that never
/// changed. Lexicographic on the id is the only *derivation* available that does
/// not depend on the request: choosing "the model with the most rooms" would
/// move the frame when a model was re-pushed, and choosing the first *in scope*
/// would move it when a building filter changed.
///
/// **A configured name matching no model with a placement falls back to the
/// derivation, with a warning.** Signal, not error: such a project is still
/// perfectly drawable and every model still lands in one frame — just not the
/// one that was asked for. Refusing to serve it would not make the name any
/// more correct.
///
/// `preferred` is `project id -> anchor_model`.
///
/// **Computed over the models given, which callers must scope per project and
/// not per request.** The frame is a property of the project; deriving it from a
/// filtered subset is the failure mode above, wearing a different hat.
///
/// A model that declares no placement is left alone rather than assumed to be at
/// the anchor's origin. It cannot be positioned — there is nothing to position
/// it with — and drawing it where it was authored is the answer that at least
/// matches what the viewer did before. This is why the map's absent entry means
/// "as-is" for two different reasons at once.
pub fn resolve<'a>(
    models: impl IntoIterator<Item = (&'a str, &'a str, Option<&'a ModelToShared>)>,
    preferred: &BTreeMap<String, String>,
) -> Placement {
    // project -> (anchor model id, its placement). A configured name wins
    // outright; among the rest the lowest model id does.
    let mut anchors: BTreeMap<&str, (&str, &ModelToShared)> = BTreeMap::new();
    let mut declared: Vec<(&str, &str, &ModelToShared)> = Vec::new();

    for (project_id, model_id, transform) in models {
        let Some(transform) = transform else { continue };
        declared.push((project_id, model_id, transform));
        let wanted = preferred.get(project_id).map(String::as_str);
        anchors
            .entry(project_id)
            .and_modify(|current| {
                // Once the configured model has been seen, nothing displaces it.
                if Some(current.0) != wanted && (Some(model_id) == wanted || model_id < current.0) {
                    *current = (model_id, transform);
                }
            })
            .or_insert((model_id, transform));
    }

    for (project_id, wanted) in preferred {
        if let Some((resolved, _)) = anchors.get(project_id.as_str())
            && *resolved != wanted.as_str()
        {
            tracing::warn!(
                "project '{project_id}' sets anchor_model = '{wanted}', but no such model declares a                  placement — drawing in '{resolved}'s frame instead"
            );
        }
    }

    let mut by_model = BTreeMap::new();
    for (project_id, model_id, transform) in declared {
        let Some((anchor_id, anchor)) = anchors.get(project_id) else {
            continue;
        };
        if model_id == *anchor_id {
            continue; // the frame's own model: nothing to do, by construction
        }
        let Some(inverse) = anchor.inverse() else {
            // A degenerate anchor cannot define a frame. Leaving every model
            // as-is reproduces the pre-placement behaviour rather than
            // collapsing the project onto a point.
            continue;
        };
        let composed = inverse.then(transform);
        if composed.is_identity() {
            continue; // models exported from the same origin, which is the norm
        }
        by_model.insert((project_id.to_string(), model_id.to_string()), composed);
    }

    Placement { by_model }
}

/// The project frame for everything the store knows about, from the manifest
/// index.
///
/// **Read from the INDEX, not from the payloads a request happens to be
/// serving**, and that is the whole reason `ModelEntry::placement` exists. The
/// anchor has to be identical for `/rooms`, `/doors`, `/windows`, `/ffe` and
/// `/spaces` or the layers stop lining up with each other — and each of those
/// reads parses a different set of models, so deriving it from what a read has
/// in hand would give five different frames. `model_index` is the only source
/// all five share, and it is already read on every request (`has_any_snapshot`).
///
/// Unscoped by project on purpose: the index is cheap, and scoping it would put
/// the request back into the frame's derivation by the back door.
///
/// **A model whose manifest predates this field has no placement and is left
/// where it was authored.** Its geometry then draws exactly as it did before —
/// which is the honest answer, since nothing available says where it belongs.
/// One re-push per model fills it in.
pub fn from_index(state: &AppState) -> Result<Placement, super::ServiceError> {
    let index = state.model_index().map_err(super::ServiceError::Internal)?;
    let registry = state.settings();
    // Every project that names an anchor. Collected up front rather than looked
    // up per row, because the choice is per project and the rows are per model.
    let preferred: BTreeMap<String, String> = index
        .iter()
        .filter_map(|row| {
            let wanted = registry.settings_for(&row.key.project_id)?.anchor_model.clone()?;
            Some((row.key.project_id.clone(), wanted))
        })
        .collect();
    Ok(resolve(
        index
            .iter()
            .map(|row| (row.key.project_id.as_str(), row.key.model_id.as_str(), row.placement.as_ref())),
        &preferred,
    ))
}

/// Move one model's plan geometry into the project frame, in place.
///
/// Positions and directions are transformed differently and the distinction is
/// not cosmetic — see `ModelToShared::place_direction`. A direction that
/// collapses under the transform is left as it was rather than zeroed: a normal
/// that cannot be rotated is a data problem to report elsewhere, not a reason to
/// hand a consumer `(0, 0)` and have it draw a glyph pointing nowhere.
pub fn place_loops(transform: &ModelToShared, loops: &mut [crate::contract::Loop]) {
    for ring in loops {
        for point in &mut ring.points {
            *point = transform.place(*point);
        }
    }
}

/// Move an optional plan point, leaving `None` alone.
pub fn place_point(transform: &ModelToShared, point: &mut Option<crate::contract::Point2D>) {
    if let Some(p) = point {
        *p = transform.place(*p);
    }
}

/// Rotate an optional plan direction, leaving it as-is if it degenerates.
pub fn place_normal(transform: &ModelToShared, normal: &mut Option<crate::contract::Point2D>) {
    if let Some(n) = normal
        && let Some(rotated) = transform.place_direction(*n)
    {
        *n = rotated;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Point2D;

    /// `resolve` with no project naming an anchor — the derived rule, which is
    /// what most of these tests exercise.
    fn resolve_derived<'a>(
        models: impl IntoIterator<Item = (&'a str, &'a str, Option<&'a ModelToShared>)>,
    ) -> Placement {
        resolve(models, &BTreeMap::new())
    }

    /// A rotation by `deg` about the origin, then a translation. The shape a
    /// Revit `ProjectLocation` produces.
    fn placement(deg: f64, e: f64, f: f64) -> ModelToShared {
        let (s, c) = deg.to_radians().sin_cos();
        ModelToShared { matrix: [c, s, -s, c, e, f] }
    }

    fn close(a: Point2D, b: Point2D) -> bool {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6
    }

    #[test]
    fn test_inverse_round_trips_a_survey_scale_placement() {
        // RHH's actual matrix. The translation is MGA Sydney in feet, which is
        // the whole reason the frame has to be project-local.
        let m = ModelToShared {
            matrix: [
                0.8007313709487278,
                0.5990235985155888,
                -0.5990235985155888,
                0.8007313709487278,
                1_008_718.0348887928,
                20_572_194.93756784,
            ],
        };
        let p = Point2D { x: -188.0, y: 416.5 };
        let back = m.inverse().expect("rigid").place(m.place(p));
        assert!(close(back, p), "round trip landed at {back:?}, not {p:?}");
    }

    #[test]
    fn test_two_models_on_different_origins_land_on_each_other() {
        // The bug, in miniature: one physical point, authored in two models
        // whose origins differ. After placement they must coincide.
        let a = placement(36.8, 1_008_718.03, 20_572_194.94);
        let b = placement(36.8, 1_008_514.36, 20_572_050.95);
        let shared = Point2D { x: 1_008_800.0, y: 20_572_300.0 };
        let in_a = a.inverse().expect("rigid").place(shared);
        let in_b = b.inverse().expect("rigid").place(shared);
        assert!(
            !close(in_a, in_b),
            "the two model spaces must actually differ, or the test proves nothing"
        );

        let placement = resolve_derived([("p", "a-model", Some(&a)), ("p", "b-model", Some(&b))]);
        let transform = placement.for_model("p", "b-model").expect("the non-anchor model moves");
        assert!(close(transform.place(in_b), in_a), "b's point must land on a's");
    }

    #[test]
    fn test_the_placed_point_stays_at_model_scale() {
        // The precision claim, asserted rather than argued: placement must not
        // leave survey-magnitude numbers behind for the f32 renderer.
        let a = placement(36.8, 1_008_718.03, 20_572_194.94);
        let b = placement(36.8, 1_008_514.36, 20_572_050.95);
        let placement = resolve_derived([("p", "a-model", Some(&a)), ("p", "b-model", Some(&b))]);
        let moved = placement.for_model("p", "b-model").expect("moves").place(Point2D { x: 100.0, y: 200.0 });
        assert!(moved.x.abs() < 10_000.0 && moved.y.abs() < 10_000.0, "escaped model scale: {moved:?}");
    }

    #[test]
    fn test_the_anchor_model_is_never_transformed() {
        // Byte-identical output for the anchor is what keeps the golden SVGs and
        // every single-model project exactly where they were.
        let a = placement(36.8, 1_008_718.03, 20_572_194.94);
        let b = placement(36.8, 1_008_514.36, 20_572_050.95);
        let placement = resolve_derived([("p", "b-model", Some(&b)), ("p", "a-model", Some(&a))]);
        assert!(placement.for_model("p", "a-model").is_none(), "the anchor is the lowest model id");
        assert!(placement.for_model("p", "b-model").is_some());
    }

    /// `[project] anchor_model` overrides the derived choice, so a project can
    /// say which model's coordinate space its exports come out in.
    #[test]
    fn test_a_configured_anchor_wins_over_the_derived_one() {
        let a = placement(10.0, 5.0, 7.0);
        let b = placement(20.0, 9.0, 3.0);
        let preferred = BTreeMap::from([("p".to_string(), "bbb".to_string())]);
        let placed = resolve([("p", "aaa", Some(&a)), ("p", "bbb", Some(&b))], &preferred);
        assert!(placed.for_model("p", "bbb").is_none(), "the configured model is the untransformed anchor");
        assert!(placed.for_model("p", "aaa").is_some(), "and the lowest id now moves instead");
    }

    /// Order-independent, like the derived rule: the configured model wins
    /// whether it is seen before or after a lower-sorting sibling.
    #[test]
    fn test_a_configured_anchor_wins_from_either_direction() {
        let a = placement(10.0, 5.0, 7.0);
        let b = placement(20.0, 9.0, 3.0);
        let preferred = BTreeMap::from([("p".to_string(), "bbb".to_string())]);
        for models in [
            vec![("p", "aaa", Some(&a)), ("p", "bbb", Some(&b))],
            vec![("p", "bbb", Some(&b)), ("p", "aaa", Some(&a))],
        ] {
            let placed = resolve(models, &preferred);
            assert!(placed.for_model("p", "bbb").is_none());
            assert!(placed.for_model("p", "aaa").is_some());
        }
    }

    /// A stale anchor name is a warning, not an error -- the project is still
    /// drawable, and every model still lands in one frame.
    #[test]
    fn test_an_unknown_configured_anchor_falls_back_to_the_derivation() {
        let a = placement(10.0, 5.0, 7.0);
        let b = placement(20.0, 9.0, 3.0);
        let preferred = BTreeMap::from([("p".to_string(), "deleted-model".to_string())]);
        let placed = resolve([("p", "aaa", Some(&a)), ("p", "bbb", Some(&b))], &preferred);
        assert!(placed.for_model("p", "aaa").is_none(), "the derived anchor answers instead");
        assert!(placed.for_model("p", "bbb").is_some());
    }

    /// One project naming an anchor must not choose the frame for another.
    #[test]
    fn test_a_configured_anchor_is_scoped_to_its_own_project() {
        let a = placement(10.0, 5.0, 7.0);
        let b = placement(20.0, 9.0, 3.0);
        let c = placement(30.0, 1.0, 2.0);
        let d = placement(40.0, 4.0, 8.0);
        let preferred = BTreeMap::from([("p1".to_string(), "zzz".to_string())]);
        let placed = resolve(
            [
                ("p1", "aaa", Some(&a)),
                ("p1", "zzz", Some(&b)),
                ("p2", "aaa", Some(&c)),
                ("p2", "zzz", Some(&d)),
            ],
            &preferred,
        );
        assert!(placed.for_model("p1", "zzz").is_none(), "p1 anchors where it was told");
        assert!(placed.for_model("p1", "aaa").is_some());
        assert!(placed.for_model("p2", "aaa").is_none(), "p2 still derives its own");
        assert!(placed.for_model("p2", "zzz").is_some());
    }

    #[test]
    fn test_the_anchor_does_not_depend_on_iteration_order() {
        let a = placement(10.0, 5.0, 7.0);
        let b = placement(20.0, 9.0, 3.0);
        let c = placement(30.0, 1.0, 2.0);
        let forward = resolve_derived([("p", "aaa", Some(&a)), ("p", "bbb", Some(&b)), ("p", "ccc", Some(&c))]);
        let reverse = resolve_derived([("p", "ccc", Some(&c)), ("p", "bbb", Some(&b)), ("p", "aaa", Some(&a))]);
        for model in ["aaa", "bbb", "ccc"] {
            assert_eq!(
                forward.for_model("p", model).map(|t| t.matrix),
                reverse.for_model("p", model).map(|t| t.matrix),
                "{model} moved when the input order changed"
            );
        }
    }

    #[test]
    fn test_models_sharing_an_origin_need_no_transform() {
        // RHH's four architectural models. Placing them would be a no-op walk
        // over every point of every polygon on the largest payloads served.
        let a = placement(36.8, 1_008_718.03, 20_572_194.94);
        let b = placement(36.8, 1_008_718.03, 20_572_194.94);
        let placement = resolve_derived([("p", "a", Some(&a)), ("p", "b", Some(&b))]);
        assert!(placement.is_empty(), "identical origins must produce no work");
    }

    #[test]
    fn test_projects_get_their_own_frames() {
        // Two unrelated jobs must not be anchored to each other; a project is
        // the largest thing a coordinate frame can span.
        let a = placement(0.0, 100.0, 100.0);
        let b = placement(0.0, 500.0, 500.0);
        let placement = resolve_derived([("p1", "m", Some(&a)), ("p2", "m", Some(&b))]);
        assert!(placement.for_model("p1", "m").is_none(), "each project anchors on its own only model");
        assert!(placement.for_model("p2", "m").is_none());
    }

    #[test]
    fn test_a_model_with_no_declared_placement_is_left_alone() {
        let a = placement(36.8, 1_008_718.03, 20_572_194.94);
        let placement = resolve_derived([("p", "anchor", Some(&a)), ("p", "unplaced", None)]);
        assert!(placement.for_model("p", "unplaced").is_none(), "nothing to position it with");
    }

    #[test]
    fn test_a_direction_is_rotated_but_not_translated() {
        let m = placement(90.0, 1000.0, 2000.0);
        let rotated = m.place_direction(Point2D { x: 1.0, y: 0.0 }).expect("non-degenerate");
        assert!(close(rotated, Point2D { x: 0.0, y: 1.0 }), "got {rotated:?}");
    }

    #[test]
    fn test_loops_and_points_move_together() {
        let m = placement(0.0, 10.0, 20.0);
        let mut loops =
            vec![crate::contract::Loop { points: vec![Point2D { x: 1.0, y: 2.0 }, Point2D { x: 3.0, y: 4.0 }] }];
        let mut point = Some(Point2D { x: 1.0, y: 2.0 });
        place_loops(&m, &mut loops);
        place_point(&m, &mut point);
        assert!(close(loops[0].points[0], Point2D { x: 11.0, y: 22.0 }));
        assert!(close(point.expect("present"), Point2D { x: 11.0, y: 22.0 }));
    }
}
