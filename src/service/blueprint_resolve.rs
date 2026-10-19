//! The resolver: a blueprint revision and a target room, to where each item goes there.
//!
//! **One function for every consumer** (plan decision 2). The viewer's preview, the assign
//! UI and the Revit task all read this, because two copies of "where does the bed go" is how
//! a preview and a Revit result disagree. It is pure over millimetres: the adapter that reads
//! the model lives in `blueprint_resolve_room` below.
//!
//! **It always returns the placements it could make, and reports the rest** (decision 3). An
//! item that falls outside the room, overlaps another, or follows something that is gone is a
//! finding on that item, never a refusal. The one case it cannot place anything is an outline
//! whose edge count differs from the blueprint's, and that too is a finding that says so.
//!
//! **Scaling is the room's, never the item's** (decision 6). An item keeps its exported size.
//! A *fixed* distance keeps its millimetres; a *percentage* keeps its fraction of the edge, or
//! of the room's depth in front of it, **measured on the edge it is anchored to** and not on
//! any box's axes.
//!
//! **Runs move as one.** An item that follows another (an abutting cabinet, a shelf in a
//! wardrobe) is placed **rigidly relative to the item it follows**, in that item's own frame,
//! so a run is never squashed by independent percentages -- 183 of the 256 overlaps when a
//! room shrank 15% in stage 0 were between abutting items. The offset is worked out from the
//! blueprint's own placements, so nothing extra is stored.
//!
//! **No reflection.** Edges map by a cyclic shift only (decision 7). A mirror-image room is
//! recognised and *said*, so the person can make a mirrored copy; the resolver does not
//! quietly mirror, because the handed families would then be wrong.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::blueprint_capture::{door_on_edges, room_doors, room_ring, SourceDoor};
use super::blueprint_geometry::{
    box_corners, cross, depth_at, dot, edges, inside, overlaps, simplify, sub, Edge, Pt, COARSE_TOLERANCE_MM,
    FINE_TOLERANCE_MM,
};
use super::blueprint_model::{
    AlongMode, BlueprintDoor, BlueprintItem, ItemClass, OffMode, Outline, Point, RevisionContent,
};
use super::room_locator::RoomRef;
use super::ServiceError;
use crate::state::AppState;

/// An item's corner this near the boundary of the fine outline is still inside.
pub const INSIDE_TOL_MM: f64 = 20.0;
/// Two items interpenetrating by more than this overlap.
pub const OVERLAP_PEN_MM: f64 = 5.0;
/// The shape-fit cost at or under which two outlines count as the same shape: a sum of
/// absolute log edge-length ratios, so 1.0 is about 10% per edge over nine edges.
pub const FIT_OK: f64 = 1.0;
/// Two rotations whose shape costs differ by less than this are a tie that doors may break.
const SHAPE_TIE: f64 = 0.15;
/// A turn angle differing by more than this is a different corner.
const TURN_TOLERANCE_DEG: f64 = 25.0;
/// What a mismatched turn adds to the cost: enough to rule a shift out on its own.
const TURN_PENALTY: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FitKind {
    /// The target has the blueprint's shape, up to rotation.
    Plain,
    /// The target is the blueprint's shape **reflected**. Rotation cannot reach it.
    MirrorImage,
    /// The same number of edges, but not the same shape.
    NoFit,
    /// A different number of edges on the coarse outline: nothing is placed.
    EdgeCountDiffers,
    /// The person chose which room edge each blueprint edge goes to; no shape fit was judged.
    Manual,
}

/// How the blueprint's edges are mapped onto the room's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeMap {
    /// The best cyclic shift by shape.
    Auto,
    /// Blueprint edge `i` to room edge `(i + shift) % edges`.
    Shift(usize),
    /// Blueprint edge `i` to room edge `map[i]`, chosen by a person. **The way to place a blueprint
    /// in a room whose edge count differs, or whose mapping the shape cannot decide**: a lot of
    /// the resolving is manual anyway, so the person's choice is accepted as it stands, with no
    /// shape judgement and no finding about it. Two blueprint edges may share a room edge.
    Explicit(Vec<usize>),
}

#[derive(Debug, Clone, Serialize)]
pub struct Fit {
    pub kind: FitKind,
    /// The cyclic shift used: blueprint edge `i` maps to target edge `(i + shift) % edges`.
    pub shift: usize,
    /// Shape cost of the plain fit at `shift`; absent when the edge counts differ.
    pub cost: Option<f64>,
    /// Shape cost of the best fit of the reflected blueprint; absent when the edge counts differ.
    pub mirror_cost: Option<f64>,
}

/// One cyclic shift that fits the room's shape, and how well its doors agree.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub shift: usize,
    pub cost: f64,
    /// Mean distance, mm, from each blueprint door to the nearest door of the room on the edge it
    /// lands on (`DOOR_UNMATCHED_MM` where that edge has none). Absent when either side has no door:
    /// no evidence, which is not agreement.
    pub door_gap_mm: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Placed {
    pub item_id: String,
    pub type_name: String,
    pub category: String,
    pub class: ItemClass,
    /// Centre in the project's own plan frame, millimetres: it draws straight onto the plan.
    pub centre: Point,
    /// Unit vector of the item's facing, in the same frame.
    pub facing: Point,
    /// Footprint size in plan: along the facing, then across it.
    pub size_mm: [f64; 2],
    /// The target edge it was placed against.
    pub edge: usize,
    /// The rule that produced this placement, in words.
    pub rule: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Finding {
    /// A corner of the item is outside the room's true outline, where it was inside the blueprint's.
    Outside { item: String },
    /// The item overlaps another that it did not overlap in the blueprint.
    Overlap { a: String, b: String },
    /// A follower whose item is excluded, missing or part of a loop: placed by its own recipe.
    FollowBroken { item: String, reason: String },
    /// The item had no edge beside it when captured.
    CornerAnchored { item: String },
    /// The room is the blueprint's shape reflected: make a mirrored copy.
    MirrorImage { message: String },
    /// The outlines do not agree in shape.
    ShapeMismatch { message: String },
    /// The blueprint's doors are not where the room's are, at this mapping.
    DoorsDisagree { message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    pub fit: Fit,
    /// The target room's outline at the two tolerances, in the project frame.
    pub outline: Outline,
    pub placements: Vec<Placed>,
    pub findings: Vec<Finding>,
    /// Items the blueprint holds but excludes: not placed, counted.
    pub excluded: usize,
    /// The rotations that fit the room's shape, best first by door agreement and then by shape.
    /// Empty when the mapping was made by hand or the edge counts differ.
    pub candidates: Vec<Candidate>,
    /// The room's doors on its coarse outline: what the blueprint's doors are compared with.
    pub doors: Vec<BlueprintDoor>,
    /// The blueprint's doors as the chosen mapping lands them on the room (edge and distance along
    /// it, in the room): so a person sees whether they sit on the room's doors without working it out.
    pub mapped_doors: Vec<BlueprintDoor>,
}

/// What a blueprint door scores when the room has no door on the edge it lands on.
pub const DOOR_UNMATCHED_MM: f64 = 3_000.0;
/// A chosen mapping whose doors are further apart than this is reported.
pub const DOOR_DISAGREE_MM: f64 = 600.0;

/// Mean distance from each blueprint door to the nearest door of the room on the edge it maps to,
/// taking a position as the same fraction along the edge. `None` when there is nothing to compare.
fn door_gap(bp: &[BlueprintDoor], room: &[BlueprintDoor], edge_map: &[usize], t_edges: &[Edge]) -> Option<f64> {
    if bp.is_empty() || room.is_empty() {
        return None;
    }
    let gaps: Vec<f64> = bp
        .iter()
        .filter_map(|d| {
            let j = *edge_map.get(d.edge)?;
            let len = t_edges.get(j)?.len;
            let want = d.along_mm / d.edge_len_mm.max(1.0) * len;
            Some(
                room.iter()
                    .filter(|r| r.edge == j)
                    .map(|r| (r.along_mm - want).abs())
                    .min_by(f64::total_cmp)
                    .unwrap_or(DOOR_UNMATCHED_MM),
            )
        })
        .collect();
    (!gaps.is_empty()).then(|| gaps.iter().sum::<f64>() / gaps.len() as f64)
}

/// The room's doors against its coarse outline: edge, distance along it and width.
fn doors_on(doors: &[SourceDoor], coarse: &[Pt]) -> Vec<BlueprintDoor> {
    let es = edges(coarse);
    doors
        .iter()
        .filter_map(|d| {
            let (edge, along_mm, width_mm) = door_on_edges(&d.footprint, coarse)?;
            Some(BlueprintDoor { id: d.id.clone(), edge, along_mm, edge_len_mm: es[edge].len, width_mm })
        })
        .collect()
}

/// The blueprint's doors as `edge_map` lands them on the room: the same fraction along the edge
/// they were on, which is the position `door_gap` compares.
fn map_doors(doors: &[BlueprintDoor], edge_map: &[usize], t_edges: &[Edge]) -> Vec<BlueprintDoor> {
    doors
        .iter()
        .filter_map(|d| {
            let edge = *edge_map.get(d.edge)?;
            let len = t_edges.get(edge)?.len;
            Some(BlueprintDoor {
                id: d.id.clone(),
                edge,
                along_mm: d.along_mm / d.edge_len_mm.max(1.0) * len,
                edge_len_mm: len,
                width_mm: d.width_mm,
            })
        })
        .collect()
}

/// Say so when the chosen mapping puts the blueprint's doors well away from the room's.
fn door_findings(
    bp: &[BlueprintDoor],
    room: &[BlueprintDoor],
    edge_map: &[usize],
    t_edges: &[Edge],
    findings: &mut Vec<Finding>,
) {
    if let Some(gap) = door_gap(bp, room, edge_map, t_edges)
        && gap > DOOR_DISAGREE_MM
    {
        findings.push(Finding::DoorsDisagree {
            message: format!(
                "the blueprint's doors land {gap:.0} mm from the room's doors on average; check the rotation"
            ),
        });
    }
}

fn turns(ring: &[Pt]) -> Vec<f64> {
    let es = edges(ring);
    (0..es.len())
        .map(|i| {
            let (a, b) = (es[i].t, es[(i + 1) % es.len()].t);
            cross(a, b).atan2(dot(a, b)).to_degrees()
        })
        .collect()
}

fn shape_cost(a: &[Edge], ta: &[f64], b: &[Edge], tb: &[f64], shift: usize) -> f64 {
    let k = a.len();
    (0..k)
        .map(|j| {
            let m = (j + shift) % k;
            let len = (b[m].len / a[j].len).ln().abs();
            let turn = if (ta[j] - tb[m]).abs() > TURN_TOLERANCE_DEG { TURN_PENALTY } else { 0.0 };
            len + turn
        })
        .sum()
}

/// The shift with the lowest shape cost, and that cost.
fn best_shift(a: &[Pt], b: &[Pt]) -> (usize, f64) {
    let (ea, eb) = (edges(a), edges(b));
    let (ta, tb) = (turns(a), turns(b));
    (0..ea.len())
        .map(|s| (s, shape_cost(&ea, &ta, &eb, &tb, s)))
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .unwrap_or((0, f64::MAX))
}

/// The ring reflected across the x axis, still counter-clockwise.
fn reflected(ring: &[Pt]) -> Vec<Pt> {
    ring.iter().rev().map(|&(x, y)| (x, -y)).collect()
}

/// An item's centre and orientation (radians) in the frame of `ring`'s edges.
fn frame_in(item: &BlueprintItem, es: &[Edge]) -> Option<(Pt, f64)> {
    let e = es.get(item.placement.edge)?;
    let c = e.point(item.placement.along_mm, item.placement.off_mm);
    Some((c, e.n.1.atan2(e.n.0) + item.placement.facing_deg.to_radians()))
}

/// Where an item lands: its centre, its orientation in radians, the target edge it sits
/// against, and the rule that put it there.
struct Pose {
    c: Pt,
    angle: f64,
    edge: usize,
    rule: String,
}

fn rotate(v: Pt, angle: f64) -> Pt {
    let (s, c) = angle.sin_cos();
    (v.0 * c - v.1 * s, v.0 * s + v.1 * c)
}

fn corners_of(item: &BlueprintItem, c: Pt, angle: f64) -> [Pt; 4] {
    box_corners(c, (angle.cos(), angle.sin()), item.size_mm[0] / 2.0, item.size_mm[1] / 2.0)
}

fn rule_text(item: &BlueprintItem, edge: usize) -> String {
    let p = &item.placement;
    let along = match p.along_mode {
        AlongMode::FixedFromStart => format!("{:.0} mm from the start", p.along_mm),
        AlongMode::FixedFromEnd => format!("{:.0} mm from the end", p.edge_len_mm - p.along_mm),
        AlongMode::Percent => format!("{:.0}% along", p.along_mm / p.edge_len_mm * 100.0),
    };
    let off = match p.off_mode {
        OffMode::Fixed => format!("{:.0} mm from the wall", p.off_mm),
        OffMode::Percent => format!("{:.0}% of the room's depth", p.off_mm / p.depth_mm * 100.0),
    };
    format!("edge {edge} (blueprint edge {}): {along}, {off}", p.edge)
}

/// How the blueprint's coarse outline sits on the target's, and what to say about it.
fn fit_outlines(
    bp_ring: &[Pt],
    t_coarse: &[Pt],
    shift: Option<usize>,
    doors: (&[BlueprintDoor], &[BlueprintDoor]),
    findings: &mut Vec<Finding>,
) -> (Fit, Vec<Candidate>) {
    let (es_bp, es_t) = (edges(bp_ring), edges(t_coarse));
    let (ta, tb) = (turns(bp_ring), turns(t_coarse));
    let k = t_coarse.len();
    let mut candidates: Vec<Candidate> = (0..k)
        .map(|s| Candidate {
            shift: s,
            cost: shape_cost(&es_bp, &ta, &es_t, &tb, s),
            door_gap_mm: door_gap(doors.0, doors.1, &(0..k).map(|i| (i + s) % k).collect::<Vec<_>>(), &es_t),
        })
        .filter(|c| c.cost <= FIT_OK)
        .collect();
    // Doors choose between rotations the shape cannot tell apart: those within `SHAPE_TIE` of the best
    // cost. A rotation that fits clearly worse is not promoted because a door happens to line up.
    let best = candidates.iter().map(|c| c.cost).fold(f64::INFINITY, f64::min);
    candidates.sort_by(|a, b| {
        let (ta, tb) = (a.cost > best + SHAPE_TIE, b.cost > best + SHAPE_TIE);
        ta.cmp(&tb)
            .then(a.door_gap_mm.unwrap_or(0.0).total_cmp(&b.door_gap_mm.unwrap_or(0.0)))
            .then(a.cost.total_cmp(&b.cost))
            .then(a.shift.cmp(&b.shift))
    });
    let (auto_shift, auto_cost) = match candidates.first() {
        Some(c) => (c.shift, c.cost),
        None => best_shift(bp_ring, t_coarse),
    };
    let s = shift.unwrap_or(auto_shift);
    let cost = if shift.is_some() {
        shape_cost(&es_bp, &turns(bp_ring), &es_t, &turns(t_coarse), s)
    } else {
        auto_cost
    };
    let mirror_cost = best_shift(&reflected(bp_ring), t_coarse).1;
    // A mirror image only counts when NO rotation fits: a rectangle's reflection is itself a rotation,
    // so a badly pinned shift on a symmetric room is a wrong mapping, not a mirrored room.
    let kind = if cost <= FIT_OK {
        FitKind::Plain
    } else if auto_cost <= FIT_OK {
        FitKind::NoFit
    } else if mirror_cost <= FIT_OK {
        FitKind::MirrorImage
    } else {
        FitKind::NoFit
    };
    match kind {
        FitKind::MirrorImage => findings.push(Finding::MirrorImage {
            message: "this room is the blueprint's shape reflected; rotation cannot reach it, so make a mirrored copy of the blueprint".to_string(),
        }),
        FitKind::NoFit if auto_cost <= FIT_OK => findings.push(Finding::ShapeMismatch {
            message: format!("shift {s} does not fit the room's shape (cost {cost:.2}); shift {auto_shift} does"),
        }),
        FitKind::NoFit => findings.push(Finding::ShapeMismatch {
            message: format!("the outlines have the same number of edges but not the same shape (cost {cost:.2})"),
        }),
        _ => {}
    }
    (Fit { kind, shift: s, cost: Some(cost), mirror_cost: Some(mirror_cost) }, candidates)
}

/// Everything the placement step reads: the live items, their frames in the blueprint, and the
/// target's edges at the chosen mapping.
struct Context<'a> {
    live: Vec<&'a BlueprintItem>,
    bp_frame: BTreeMap<&'a str, (Pt, f64)>,
    es_t: Vec<Edge>,
    t_coarse: &'a [Pt],
    /// The blueprint's own fine outline: what "already outside" in the blueprint is judged against.
    bp_fine: Vec<Pt>,
    /// Room edge for each blueprint edge.
    edge_map: Vec<usize>,
}

impl Context<'_> {
    /// Placed against an edge by the item's own recipe.
    fn anchored(&self, item: &BlueprintItem) -> Pose {
        let p = &item.placement;
        let j = self.edge_map.get(p.edge).copied().unwrap_or(0);
        let e = self.es_t[j];
        let along = match p.along_mode {
            AlongMode::FixedFromStart => p.along_mm,
            AlongMode::FixedFromEnd => e.len - (p.edge_len_mm - p.along_mm),
            AlongMode::Percent => p.along_mm / p.edge_len_mm * e.len,
        };
        let off = match p.off_mode {
            OffMode::Fixed => p.off_mm,
            OffMode::Percent => p.off_mm / p.depth_mm * depth_at(self.t_coarse, j, e.point(along, 0.0)),
        };
        let angle = e.n.1.atan2(e.n.0) + p.facing_deg.to_radians();
        Pose { c: e.point(along, off), angle, edge: j, rule: rule_text(item, j) }
    }

    /// Where every live item lands: those with an edge first, followers once the item they follow is
    /// placed. A follower of something missing, or in a loop, falls back to its own recipe.
    fn place_all(&self, findings: &mut Vec<Finding>) -> BTreeMap<String, Pose> {
        let by_id: BTreeMap<&str, &BlueprintItem> = self.live.iter().map(|i| (i.id.as_str(), *i)).collect();
        let mut placed: BTreeMap<String, Pose> = BTreeMap::new();
        let mut pending: Vec<&BlueprintItem> = Vec::new();
        for item in &self.live {
            match &item.follows {
                Some(f) if by_id.contains_key(f.item.as_str()) => pending.push(item),
                Some(f) => {
                    findings.push(Finding::FollowBroken {
                        item: item.id.clone(),
                        reason: format!("it follows {:?}, which the blueprint has excluded or does not hold", f.item),
                    });
                    placed.insert(item.id.clone(), self.anchored(item));
                }
                None => {
                    placed.insert(item.id.clone(), self.anchored(item));
                }
            }
        }
        loop {
            let before = pending.len();
            pending.retain(|item| {
                let parent_id = item.follows.as_ref().map_or("", |f| f.item.as_str());
                let Some(parent) = placed.get(parent_id) else {
                    return true;
                };
                let (pc, pa, pj) = (parent.c, parent.angle, parent.edge);
                let (Some(&(bc, ba)), Some(&(ic, ia))) =
                    (self.bp_frame.get(parent_id), self.bp_frame.get(item.id.as_str()))
                else {
                    return true;
                };
                // The follower's offset in the followed item's own frame, then carried to where that
                // item now is.
                let at = rotate(rotate(sub(ic, bc), -ba), pa);
                placed.insert(
                    item.id.clone(),
                    Pose {
                        c: (pc.0 + at.0, pc.1 + at.1),
                        angle: pa + (ia - ba),
                        edge: pj,
                        rule: format!("follows {parent_id}, keeping its place against it"),
                    },
                );
                false
            });
            if pending.len() == before {
                break;
            }
        }
        for item in pending {
            findings.push(Finding::FollowBroken {
                item: item.id.clone(),
                reason: "it is part of a loop of followers, so it was placed by its own recipe".to_string(),
            });
            placed.insert(item.id.clone(), self.anchored(item));
        }
        placed
    }

    /// What the person is shown for each live item that was placed, and a finding for each that had no
    /// wall beside it when captured.
    fn placements(&self, placed: &BTreeMap<String, Pose>, findings: &mut Vec<Finding>) -> Vec<Placed> {
        let mut out = Vec::new();
        for item in &self.live {
            let Some(pose) = placed.get(&item.id) else { continue };
            if item.placement.corner_anchored {
                findings.push(Finding::CornerAnchored { item: item.id.clone() });
            }
            out.push(Placed {
                item_id: item.id.clone(),
                type_name: item.type_name.clone(),
                category: item.category.clone(),
                class: item.class,
                centre: pose.c.into(),
                facing: (pose.angle.cos(), pose.angle.sin()).into(),
                size_mm: item.size_mm,
                edge: pose.edge,
                rule: pose.rule.clone(),
            });
        }
        out
    }

    /// Outside the true outline, and new overlaps, for free items only: a wall device is located
    /// by its wall and carries no footprint check.
    fn footprint_findings(&self, placed: &BTreeMap<String, Pose>, t_fine: &[Pt], findings: &mut Vec<Finding>) {
        let free: Vec<&BlueprintItem> = self.live.iter().copied().filter(|i| i.class == ItemClass::Free).collect();
        let boxes_in_room: BTreeMap<&str, [Pt; 4]> = free
            .iter()
            .filter_map(|i| placed.get(&i.id).map(|p| (i.id.as_str(), corners_of(i, p.c, p.angle))))
            .collect();
        let boxes_in_blueprint: BTreeMap<&str, [Pt; 4]> = free
            .iter()
            .filter_map(|i| self.bp_frame.get(i.id.as_str()).map(|(c, a)| (i.id.as_str(), corners_of(i, *c, *a))))
            .collect();
        // Only what the room CHANGES: an item already outside its own source room (the model has
        // some, three of 66 on RHH's `1 BED B`) would otherwise be reported on every resolve.
        for i in &free {
            let (Some(now), Some(was)) = (boxes_in_room.get(i.id.as_str()), boxes_in_blueprint.get(i.id.as_str()))
            else {
                continue;
            };
            let inside_now = now.iter().all(|&p| inside(p, t_fine, INSIDE_TOL_MM));
            let inside_then = was.iter().all(|&p| inside(p, &self.bp_fine, INSIDE_TOL_MM));
            if inside_then && !inside_now {
                findings.push(Finding::Outside { item: i.id.clone() });
            }
        }
        let related: BTreeSet<(&str, &str)> = free
            .iter()
            .filter_map(|i| i.follows.as_ref().map(|f| (i.id.as_str(), f.item.as_str())))
            .flat_map(|(a, b)| [(a, b), (b, a)])
            .collect();
        for (n, a) in free.iter().enumerate() {
            for b in &free[n + 1..] {
                if related.contains(&(a.id.as_str(), b.id.as_str())) {
                    continue;
                }
                let (Some(now_a), Some(now_b)) = (boxes_in_room.get(a.id.as_str()), boxes_in_room.get(b.id.as_str()))
                else {
                    continue;
                };
                let was = match (boxes_in_blueprint.get(a.id.as_str()), boxes_in_blueprint.get(b.id.as_str())) {
                    (Some(x), Some(y)) => overlaps(x, y, OVERLAP_PEN_MM),
                    _ => false,
                };
                if !was && overlaps(now_a, now_b, OVERLAP_PEN_MM) {
                    findings.push(Finding::Overlap { a: a.id.clone(), b: b.id.clone() });
                }
            }
        }
    }
}

/// Where each item of `content` goes in the room whose outer loop is `target_ring`.
///
/// `shift` pins the edge mapping (blueprint edge `i` to target edge `(i + shift) % edges`);
/// left out, the best fit by shape is used.
pub fn resolve(
    content: &RevisionContent,
    target_ring: &[Pt],
    shift: Option<usize>,
) -> Result<Resolution, ServiceError> {
    resolve_with(content, target_ring, &shift.map_or(EdgeMap::Auto, EdgeMap::Shift))
}

/// `resolve` with the edge mapping spelt out: automatic, a pinned shift, or a person's own.
pub fn resolve_with(
    content: &RevisionContent,
    target_ring: &[Pt],
    mapping: &EdgeMap,
) -> Result<Resolution, ServiceError> {
    resolve_with_doors(content, target_ring, mapping, &[])
}

/// `resolve_with`, and the room's doors (footprints in the project frame) to orient it by. Where
/// the shape fits in several rotations the one whose doors agree best is used; a mapping given
/// by hand is never overridden, only reported on.
pub fn resolve_with_doors(
    content: &RevisionContent,
    target_ring: &[Pt],
    mapping: &EdgeMap,
    target_doors: &[SourceDoor],
) -> Result<Resolution, ServiceError> {
    let t_coarse = simplify(target_ring, COARSE_TOLERANCE_MM);
    let t_fine = simplify(target_ring, FINE_TOLERANCE_MM);
    if t_coarse.len() < 3 {
        return Err(ServiceError::Invalid("the room's outline has fewer than three corners".to_string()));
    }
    let outline = Outline {
        coarse: t_coarse.iter().map(|&p| p.into()).collect(),
        fine: t_fine.iter().map(|&p| p.into()).collect(),
    };
    let bp_ring: Vec<Pt> = content.outline.coarse.iter().map(|&p| p.into()).collect();
    let t_doors = doors_on(target_doors, &t_coarse);
    let mut candidates = Vec::new();
    let mut findings = Vec::new();
    let excluded = content.items.iter().filter(|i| i.excluded).count();

    let (fit, edge_map) = if let EdgeMap::Explicit(map) = mapping {
        if map.len() != bp_ring.len() {
            return Err(ServiceError::Invalid(format!(
                "the edge map names {} edges and the blueprint has {}",
                map.len(),
                bp_ring.len()
            )));
        }
        if let Some(bad) = map.iter().find(|&&e| e >= t_coarse.len()) {
            return Err(ServiceError::Invalid(format!(
                "the edge map names room edge {bad}, but the room has {}",
                t_coarse.len()
            )));
        }
        (Fit { kind: FitKind::Manual, shift: 0, cost: None, mirror_cost: None }, map.clone())
    } else {
        if bp_ring.len() != t_coarse.len() {
            findings.push(Finding::ShapeMismatch {
                message: format!(
                    "the blueprint's outline has {} edges and the room's has {}, so nothing was placed; map the edges by hand",
                    bp_ring.len(),
                    t_coarse.len()
                ),
            });
            return Ok(Resolution {
                fit: Fit { kind: FitKind::EdgeCountDiffers, shift: 0, cost: None, mirror_cost: None },
                outline,
                placements: Vec::new(),
                findings,
                excluded,
                candidates: Vec::new(),
                doors: t_doors,
                mapped_doors: Vec::new(),
            });
        }
        let shift = if let EdgeMap::Shift(s) = mapping { Some(*s) } else { None };
        if let Some(s) = shift
            && s >= t_coarse.len()
        {
            return Err(ServiceError::Invalid(format!(
                "shift {s} is out of range: the room has {} edges",
                t_coarse.len()
            )));
        }
        let (fit, found) = fit_outlines(&bp_ring, &t_coarse, shift, (&content.doors, &t_doors), &mut findings);
        candidates = found;
        let k = t_coarse.len();
        let map = (0..k).map(|i| (i + fit.shift) % k).collect();
        (fit, map)
    };
    door_findings(&content.doors, &t_doors, &edge_map, &edges(&t_coarse), &mut findings);
    let es_bp = edges(&bp_ring);
    let live: Vec<&BlueprintItem> = content.items.iter().filter(|i| !i.excluded).collect();
    let bp_frame: BTreeMap<&str, (Pt, f64)> =
        live.iter().filter_map(|i| frame_in(i, &es_bp).map(|f| (i.id.as_str(), f))).collect();
    let bp_fine: Vec<Pt> = content.outline.fine.iter().map(|&p| p.into()).collect();
    let ctx = Context { live, bp_frame, es_t: edges(&t_coarse), t_coarse: &t_coarse, bp_fine, edge_map };

    let placed = ctx.place_all(&mut findings);
    let placements = ctx.placements(&placed, &mut findings);
    ctx.footprint_findings(&placed, &t_fine, &mut findings);

    let mapped_doors = map_doors(&content.doors, &ctx.edge_map, &ctx.es_t);
    Ok(Resolution {
        fit,
        outline,
        placements,
        findings,
        excluded,
        candidates,
        doors: t_doors,
        mapped_doors,
    })
}

/// Resolve `content` onto one room of the project, read as it stands today. `Ok(None)` when the
/// project has no such room. The result is in the project's plan frame, so it draws onto the plan.
pub fn resolve_in_room(
    state: &AppState,
    project: &str,
    content: &RevisionContent,
    room: &RoomRef,
    mapping: &EdgeMap,
) -> Result<Option<(String, Resolution)>, ServiceError> {
    let Some((room_name, ring)) = room_ring(state, project, room)? else {
        return Ok(None);
    };
    let doors = room_doors(state, project, room)?;
    Ok(Some((room_name, resolve_with_doors(content, &ring, mapping, &doors)?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::blueprint_capture::{capture, SourceItem};
    use crate::service::blueprint_geometry::{centroid, unit};

    fn rect(w: f64, h: f64) -> Vec<Pt> {
        vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    }

    /// An item `w` wide (across its facing) and `d` deep (along it), centred at `c` facing `f`.
    fn src(id: &str, c: Pt, w: f64, d: f64, f: Pt) -> SourceItem {
        let g = unit(f);
        let v = (-g.1, g.0);
        let footprint = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)]
            .iter()
            .map(|(a, b)| (c.0 + a * d / 2.0 * g.0 + b * w / 2.0 * v.0, c.1 + a * d / 2.0 * g.1 + b * w / 2.0 * v.1))
            .collect();
        SourceItem {
            id: id.to_string(),
            family_name: None,
            type_name: "T".to_string(),
            category: "Furniture".to_string(),
            footprint,
            facing: Some(f),
            nested: false,
            properties: vec![],
            type_properties: vec![],
        }
    }

    fn content(ring: &[Pt], items: Vec<SourceItem>) -> RevisionContent {
        capture(ring, items).unwrap().content
    }

    fn placed<'a>(r: &'a Resolution, id: &str) -> &'a Placed {
        r.placements.iter().find(|p| p.item_id == id).unwrap()
    }

    fn at(r: &Resolution, id: &str) -> Pt {
        let c = placed(r, id).centre;
        (c.x, c.y)
    }

    fn near(a: Pt, b: Pt, tol: f64) -> bool {
        (a.0 - b.0).hypot(a.1 - b.1) <= tol
    }

    /// The blueprint's own room: every item comes back exactly where it was.
    #[test]
    fn test_a_blueprint_resolved_onto_its_own_room_puts_every_item_back() {
        let ring = rect(6000.0, 4000.0);
        let items = vec![
            src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0)),
            src("wardrobe", (500.0, 1500.0), 1200.0, 600.0, (1.0, 0.0)),
            src("table", (3000.0, 2200.0), 800.0, 800.0, (0.0, 1.0)),
        ];
        let want: Vec<(&str, Pt)> = vec![
            ("bed", (3000.0, 600.0)),
            ("wardrobe", (500.0, 1500.0)),
            ("table", (3000.0, 2200.0)),
        ];
        let r = resolve(&content(&ring, items), &ring, None).unwrap();
        assert_eq!((r.fit.kind, r.fit.shift), (FitKind::Plain, 0));
        for (id, c) in want {
            assert!(near(at(&r, id), c, 1e-6), "{id}: {:?} vs {c:?}", at(&r, id));
        }
        assert!(r.findings.is_empty(), "{:?}", r.findings);
    }

    /// A larger room: a corner item keeps its distance to the corner, a centred one stays centred.
    #[test]
    fn test_fixed_distances_stay_fixed_and_percentages_scale_with_the_room() {
        let ring = rect(6000.0, 4000.0);
        let c = content(
            &ring,
            vec![
                src("corner", (400.0, 300.0), 800.0, 600.0, (0.0, 1.0)),
                src("centred", (3000.0, 300.0), 800.0, 600.0, (0.0, 1.0)),
            ],
        );
        let bigger = rect(9000.0, 4000.0);
        let r = resolve(&c, &bigger, None).unwrap();
        assert!(near(at(&r, "corner"), (400.0, 300.0), 1e-6), "keeps 400 mm from the corner");
        assert!(near(at(&r, "centred"), (4500.0, 300.0), 1e-6), "still half way along the wall");
        assert!(r.findings.is_empty(), "{:?}", r.findings);
    }

    /// A far end keeps its distance from the END, not the start.
    #[test]
    fn test_an_item_at_the_far_end_keeps_its_distance_from_that_end() {
        let ring = rect(6000.0, 4000.0);
        let c = content(&ring, vec![src("far", (5600.0, 300.0), 800.0, 600.0, (0.0, 1.0))]);
        let r = resolve(&c, &rect(9000.0, 4000.0), None).unwrap();
        assert!(near(at(&r, "far"), (8600.0, 300.0), 1e-6));
    }

    /// The same room turned and moved: the shape fit finds the rotation by itself.
    #[test]
    fn test_a_rotated_and_moved_room_is_found_and_the_items_turn_with_it() {
        let ring = rect(6000.0, 4000.0);
        let c = content(
            &ring,
            vec![
                src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0)),
                src("desk", (5500.0, 3000.0), 1200.0, 600.0, (-1.0, 0.0)),
            ],
        );
        let (s, k) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
        let turn = |p: Pt| (p.0 * k - p.1 * s + 2.0e6, p.0 * s + p.1 * k - 5.0e5);
        let target: Vec<Pt> = ring.iter().map(|&p| turn(p)).collect();
        let r = resolve(&c, &target, None).unwrap();
        assert_eq!(r.fit.kind, FitKind::Plain);
        assert!(near(at(&r, "bed"), turn((3000.0, 600.0)), 1e-6));
        assert!(near(at(&r, "desk"), turn((5500.0, 3000.0)), 1e-6));
        let f = placed(&r, "bed").facing;
        assert!(near((f.x, f.y), (-s, k), 1e-9), "facing turned with the room");
        assert!(r.findings.is_empty(), "{:?}", r.findings);
    }

    /// The same room, edges numbered from a different corner. A rectangle has two mappings that fit
    /// equally (a half turn apart), which is exactly why the blueprint keeps its door: the shape alone
    /// cannot choose between them. Pinning the shift chooses.
    #[test]
    fn test_the_shift_is_found_from_the_shape_and_can_be_pinned() {
        let ring = rect(6000.0, 4000.0);
        let c = content(&ring, vec![src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0))]);
        // The same rectangle listed from its second corner: blueprint edge 0 is target edge 3 (or, a
        // half turn on, edge 1).
        let shifted = vec![(6000.0, 0.0), (6000.0, 4000.0), (0.0, 4000.0), (0.0, 0.0)];
        let auto = resolve(&c, &shifted, None).unwrap();
        assert_eq!(auto.fit.kind, FitKind::Plain);
        assert!(auto.fit.cost.unwrap() < 1e-9);
        let same_place = resolve(&c, &shifted, Some(3)).unwrap();
        assert!(
            near(at(&same_place, "bed"), (3000.0, 600.0), 1e-6),
            "shift 3 puts the bed where it was on the plan"
        );
        let half_turn = resolve(&c, &shifted, Some(1)).unwrap();
        assert!(
            near(at(&half_turn, "bed"), (3000.0, 3400.0), 1e-6),
            "the other fit is a half turn about the centre"
        );
        let wrong = resolve(&c, &shifted, Some(0)).unwrap();
        assert_eq!(wrong.fit.kind, FitKind::NoFit, "a quarter turn is not this room's shape");
        assert!(resolve(&c, &shifted, Some(9)).is_err());
    }

    /// A smaller room: the fixed-distance items stay clear of the wall, and what no longer fits is a finding.
    #[test]
    fn test_what_no_longer_fits_is_reported_and_still_placed() {
        let ring = rect(6000.0, 4000.0);
        let c = content(
            &ring,
            vec![
                src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0)),
                src("wardrobe", (5600.0, 3500.0), 800.0, 600.0, (0.0, -1.0)),
            ],
        );
        let r = resolve(&c, &rect(2000.0, 1200.0), None).unwrap();
        assert_eq!(r.placements.len(), 2, "always the placements it could make");
        assert!(r.findings.iter().any(|f| matches!(f, Finding::Outside { .. })), "{:?}", r.findings);
    }

    /// Two items that did not overlap in the blueprint, squeezed together by a smaller room.
    /// An item that was already outside its own source room is not a finding of the new room.
    #[test]
    fn test_an_item_already_outside_in_the_blueprint_is_not_reported_again() {
        let ring = rect(6000.0, 4000.0);
        let mut c = content(
            &ring,
            vec![
                src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0)),
                src("ghost", (3000.0, 3000.0), 400.0, 400.0, (0.0, 1.0)),
            ],
        );
        // Put the ghost's recipe a metre beyond the top wall, so it is outside in the blueprint itself.
        let g = c.items.iter_mut().find(|i| i.id == "ghost").unwrap();
        g.placement.off_mm = -1000.0;
        g.placement.off_mode = OffMode::Fixed;
        let r = resolve(&c, &ring, None).unwrap();
        assert!(!r.findings.iter().any(|f| matches!(f, Finding::Outside { .. })), "{:?}", r.findings);
    }

    #[test]
    fn test_a_new_overlap_is_a_finding_and_an_old_one_is_not() {
        let ring = rect(8000.0, 4000.0);
        let c = content(
            &ring,
            vec![
                src("a", (2000.0, 1500.0), 800.0, 800.0, (0.0, 1.0)),
                src("b", (6000.0, 1500.0), 800.0, 800.0, (0.0, 1.0)),
            ],
        );
        let squeezed = resolve(&c, &rect(1400.0, 4000.0), None).unwrap();
        assert!(
            squeezed.findings.iter().any(|f| matches!(f, Finding::Overlap { .. })),
            "{:?}",
            squeezed.findings
        );
        let same = resolve(&c, &ring, None).unwrap();
        assert!(!same.findings.iter().any(|f| matches!(f, Finding::Overlap { .. })));
    }

    /// The room is the blueprint's shape reflected: said, not silently mirrored.
    #[test]
    fn test_a_mirror_image_room_is_recognised_and_said() {
        let l = vec![
            (0.0, 0.0),
            (6000.0, 0.0),
            (6000.0, 2000.0),
            (2500.0, 2000.0),
            (2500.0, 4500.0),
            (0.0, 4500.0),
        ];
        let c = content(&l, vec![src("bed", (1000.0, 800.0), 800.0, 800.0, (0.0, 1.0))]);
        let mirrored: Vec<Pt> = l.iter().rev().map(|&(x, y)| (x, -y)).collect();
        let r = resolve(&c, &mirrored, None).unwrap();
        assert_eq!(r.fit.kind, FitKind::MirrorImage);
        assert!(r.findings.iter().any(|f| matches!(f, Finding::MirrorImage { .. })), "{:?}", r.findings);
        assert!(r.fit.mirror_cost.unwrap() < r.fit.cost.unwrap());
        assert_eq!(r.placements.len(), 1, "still returns what it could place");
    }

    /// A room with a different edge count, placed by a person's own mapping.
    #[test]
    fn test_a_manual_edge_map_places_items_in_a_room_with_a_different_edge_count() {
        let c = content(&rect(6000.0, 4000.0), vec![src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0))]);
        let l = vec![
            (0.0, 0.0),
            (6000.0, 0.0),
            (6000.0, 2000.0),
            (2500.0, 2000.0),
            (2500.0, 4500.0),
            (0.0, 4500.0),
        ];
        assert_eq!(resolve(&c, &l, None).unwrap().placements.len(), 0, "automatic gives up");
        // Blueprint edges 0..3 (bottom, right, top, left) to the L's bottom, upper right, notch, left.
        let r = resolve_with(&c, &l, &EdgeMap::Explicit(vec![0, 1, 4, 5])).unwrap();
        assert_eq!(r.fit.kind, FitKind::Manual);
        assert_eq!(r.placements.len(), 1);
        assert!(
            near(at(&r, "bed"), (3000.0, 600.0), 1e-6),
            "the bottom wall is the bottom wall: {:?}",
            at(&r, "bed")
        );
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        // Moving the bed's wall to another room edge moves the bed there.
        let moved = resolve_with(&c, &l, &EdgeMap::Explicit(vec![4, 1, 4, 5])).unwrap();
        assert!(!near(at(&moved, "bed"), (3000.0, 600.0), 1.0));
    }

    #[test]
    fn test_a_manual_edge_map_that_cannot_be_applied_is_refused() {
        let c = content(&rect(6000.0, 4000.0), vec![src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0))]);
        let ring = rect(6000.0, 4000.0);
        assert!(resolve_with(&c, &ring, &EdgeMap::Explicit(vec![0, 1, 2])).is_err(), "too few edges named");
        assert!(
            resolve_with(&c, &ring, &EdgeMap::Explicit(vec![0, 1, 2, 9])).is_err(),
            "a room edge that is not there"
        );
        assert!(resolve_with(&c, &ring, &EdgeMap::Explicit(vec![0, 1, 2, 3])).is_ok());
    }

    #[test]
    fn test_a_room_with_a_different_edge_count_places_nothing_and_says_why() {
        let c = content(&rect(6000.0, 4000.0), vec![src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0))]);
        let l = vec![
            (0.0, 0.0),
            (6000.0, 0.0),
            (6000.0, 2000.0),
            (2500.0, 2000.0),
            (2500.0, 4500.0),
            (0.0, 4500.0),
        ];
        let r = resolve(&c, &l, None).unwrap();
        assert_eq!(r.fit.kind, FitKind::EdgeCountDiffers);
        assert!(r.placements.is_empty());
        assert!(matches!(r.findings[0], Finding::ShapeMismatch { .. }));
    }

    /// A run keeps its gaps when the room changes, where independent percentages would squash it.
    #[test]
    fn test_a_run_moves_as_one_and_keeps_its_gaps() {
        let ring = rect(8000.0, 4000.0);
        let c = content(
            &ring,
            vec![
                src("c1", (3000.0, 300.0), 600.0, 600.0, (0.0, 1.0)),
                src("c2", (3600.0, 300.0), 600.0, 600.0, (0.0, 1.0)),
                src("c3", (4200.0, 300.0), 600.0, 600.0, (0.0, 1.0)),
            ],
        );
        assert!(c.items.iter().filter(|i| i.follows.is_some()).count() == 2, "c2 and c3 follow");
        let r = resolve(&c, &rect(5000.0, 4000.0), None).unwrap();
        let (x1, x2, x3) = (at(&r, "c1").0, at(&r, "c2").0, at(&r, "c3").0);
        assert!(
            ((x2 - x1) - 600.0).abs() < 1e-6 && ((x3 - x2) - 600.0).abs() < 1e-6,
            "still abutting: {x1} {x2} {x3}"
        );
        assert!(!r.findings.iter().any(|f| matches!(f, Finding::Overlap { .. })), "{:?}", r.findings);
    }

    /// A shelf in a wardrobe goes where the wardrobe goes, at the same place inside it.
    #[test]
    fn test_an_item_inside_another_stays_inside_it() {
        let ring = rect(6000.0, 4000.0);
        let c = content(
            &ring,
            vec![
                src("wardrobe", (3000.0, 300.0), 1200.0, 600.0, (0.0, 1.0)),
                src("shelf", (3200.0, 300.0), 400.0, 300.0, (0.0, 1.0)),
            ],
        );
        let r = resolve(&c, &rect(9000.0, 4000.0), None).unwrap();
        let (w, s) = (at(&r, "wardrobe"), at(&r, "shelf"));
        assert!(
            near(s, (w.0 + 200.0, w.1), 1e-6),
            "200 mm to the right of the wardrobe's centre, as captured"
        );
        assert!(r.findings.is_empty(), "{:?}", r.findings);
    }

    #[test]
    fn test_a_loop_of_followers_is_placed_by_their_own_recipes_and_reported() {
        let ring = rect(6000.0, 4000.0);
        let mut c = content(
            &ring,
            vec![
                src("a", (1000.0, 300.0), 600.0, 600.0, (0.0, 1.0)),
                src("b", (3000.0, 300.0), 600.0, 600.0, (0.0, 1.0)),
            ],
        );
        use crate::service::blueprint_model::Follow;
        c.items[0].follows = Some(Follow { item: "b".into(), gap_mm: 0.0, inside: false });
        c.items[1].follows = Some(Follow { item: "a".into(), gap_mm: 0.0, inside: false });
        let r = resolve(&c, &ring, None).unwrap();
        assert_eq!(r.placements.len(), 2);
        assert_eq!(r.findings.iter().filter(|f| matches!(f, Finding::FollowBroken { .. })).count(), 2);
        assert!(near(at(&r, "a"), (1000.0, 300.0), 1e-6));
    }

    #[test]
    fn test_a_follower_of_an_excluded_item_is_placed_alone_and_reported() {
        let ring = rect(8000.0, 4000.0);
        let mut c = content(
            &ring,
            vec![
                src("c1", (3000.0, 300.0), 600.0, 600.0, (0.0, 1.0)),
                src("c2", (3600.0, 300.0), 600.0, 600.0, (0.0, 1.0)),
            ],
        );
        c.items.iter_mut().find(|i| i.id == "c1").unwrap().excluded = true;
        let r = resolve(&c, &ring, None).unwrap();
        assert_eq!((r.placements.len(), r.excluded), (1, 1), "the excluded item is counted, not placed");
        assert!(
            r.findings.iter().any(|f| matches!(f, Finding::FollowBroken { item, .. } if item == "c2")),
            "{:?}",
            r.findings
        );
    }

    /// A socket sits on the wall line: it is never "outside", and never an overlap.
    #[test]
    fn test_a_wall_device_is_left_out_of_the_footprint_findings() {
        let ring = rect(6000.0, 4000.0);
        let c = content(
            &ring,
            vec![
                src("gpo", (3000.0, 4.0), 115.0, 8.0, (0.0, 1.0)),
                src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0)),
            ],
        );
        assert_eq!(c.items.iter().find(|i| i.id == "gpo").unwrap().class, ItemClass::WallDevice);
        let r = resolve(&c, &rect(2500.0, 1500.0), None).unwrap();
        assert!(
            !r.findings
                .iter()
                .any(|f| matches!(f, Finding::Outside { item } | Finding::Overlap { a: item, .. } if item == "gpo")),
            "{:?}",
            r.findings
        );
    }

    #[test]
    fn test_every_placement_says_which_rule_produced_it() {
        let ring = rect(6000.0, 4000.0);
        let c = content(
            &ring,
            vec![
                src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0)),
                src("c2", (3000.0, 2000.0), 600.0, 600.0, (0.0, 1.0)),
            ],
        );
        let r = resolve(&c, &ring, None).unwrap();
        assert!(
            placed(&r, "bed").rule.contains("edge 0") && placed(&r, "bed").rule.contains("% along"),
            "{}",
            placed(&r, "bed").rule
        );
    }

    #[test]
    fn test_the_result_is_in_the_target_rooms_own_frame_so_it_draws_on_the_plan() {
        let far: Vec<Pt> = rect(6000.0, 4000.0).into_iter().map(|p| (p.0 + 2.0e7, p.1 + 1.0e6)).collect();
        let c = content(&rect(6000.0, 4000.0), vec![src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0))]);
        let r = resolve(&c, &far, None).unwrap();
        let ct = centroid(&r.outline.fine.iter().map(|&p| p.into()).collect::<Vec<Pt>>());
        assert!(near(ct, (2.0e7 + 3000.0, 1.0e6 + 2000.0), 1e-3));
        assert!(near(at(&r, "bed"), (2.0e7 + 3000.0, 1.0e6 + 600.0), 1e-3));
    }

    /// A door as capture reads it: a 900 x 100 footprint centred at `c`, lying along x.
    fn door_at(id: &str, c: Pt) -> SourceDoor {
        SourceDoor {
            id: id.to_string(),
            footprint: vec![
                (c.0 - 450.0, c.1 - 50.0),
                (c.0 + 450.0, c.1 - 50.0),
                (c.0 + 450.0, c.1 + 50.0),
                (c.0 - 450.0, c.1 + 50.0),
            ],
        }
    }

    #[test]
    fn test_a_doors_position_chooses_between_the_two_rotations_a_rectangle_fits() {
        use crate::service::blueprint_capture::capture_with_doors;
        let ring = rect(6000.0, 4000.0);
        let c = capture_with_doors(
            &ring,
            vec![src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0))],
            vec![door_at("d", (1000.0, 0.0))],
        )
        .unwrap()
        .content;
        assert_eq!(c.doors.len(), 1);
        assert_eq!(c.doors[0].edge, 0);

        // The room's door is on the opposite wall (edge 2), at the matching end after a half turn.
        let with_door_on_top = [door_at("t", (5000.0, 4000.0))];
        let r = resolve_with_doors(&c, &ring, &EdgeMap::Auto, &with_door_on_top).unwrap();
        assert_eq!(r.fit.shift, 2, "the half turn puts the blueprint's door on the room's");
        assert_eq!(r.candidates.len(), 2, "both rotations fit the shape");
        assert!(r.candidates[0].door_gap_mm.unwrap() < r.candidates[1].door_gap_mm.unwrap());
        assert!(r.findings.iter().all(|f| !matches!(f, Finding::DoorsDisagree { .. })));

        // With the door on the same wall, no half turn.
        let same_wall = [door_at("t", (1000.0, 0.0))];
        assert_eq!(resolve_with_doors(&c, &ring, &EdgeMap::Auto, &same_wall).unwrap().fit.shift, 0);
    }

    #[test]
    fn test_no_doors_on_either_side_is_no_evidence_and_changes_nothing() {
        let ring = rect(6000.0, 4000.0);
        let c = content(&ring, vec![src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0))]);
        let r = resolve(&c, &ring, None).unwrap();
        assert_eq!(r.fit.shift, 0);
        assert!(r.candidates.iter().all(|c| c.door_gap_mm.is_none()));
        assert!(r.doors.is_empty());
    }

    #[test]
    fn test_a_manual_mapping_is_kept_and_a_wrong_door_is_only_reported() {
        use crate::service::blueprint_capture::capture_with_doors;
        let ring = rect(6000.0, 4000.0);
        let c = capture_with_doors(&ring, vec![], vec![door_at("d", (1000.0, 0.0))]).unwrap().content;
        let top = [door_at("t", (5000.0, 4000.0))];
        let r = resolve_with_doors(&c, &ring, &EdgeMap::Shift(0), &top).unwrap();
        assert_eq!(r.fit.shift, 0, "a pinned shift is never overridden");
        assert!(r.findings.iter().any(|f| matches!(f, Finding::DoorsDisagree { .. })));
    }
}
