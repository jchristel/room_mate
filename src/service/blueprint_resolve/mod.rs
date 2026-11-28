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
//! recognised and *said*; the other hand is a blueprint a person authors (a reflection would
//! place the wrong families), so the resolver never mirrors and nothing generates a copy.
//!
//! Kept whole: the resolver proper: `Context` places each item from its recipe, and `resolve_with_overrides` applies
//! a person's nudges before judging the findings. The two share one placement state and splitting them would put it
//! across a module boundary. Doors and the outline fit already moved out (`doors`, `fit`).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::assignments::{Frame, ItemAddition, ItemOverride};

use super::blueprint_capture::{door_on_edges, room_doors, room_ring, SourceDoor};
use super::blueprint_geometry::{
    box_corners, centroid, coarse_outline, cross, depth_at, dot, edges, inside, overlaps, simplify, sub, Edge, Pt,
    FINE_TOLERANCE_MM,
};
use super::blueprint_model::{
    AlongMode, BlueprintDoor, BlueprintItem, ItemClass, OffMode, Outline, Point, RevisionContent,
};
use super::room_locator::RoomRef;
use super::ServiceError;
use crate::state::AppState;

mod doors;
mod fit;
pub use doors::*;
pub(in crate::service) use fit::*;

/// An item's corner this near the boundary of the fine outline is still inside.
pub const INSIDE_TOL_MM: f64 = 20.0;
/// Two items interpenetrating by more than this overlap.
pub const OVERLAP_PEN_MM: f64 = 5.0;
/// The shape-fit cost at or under which two outlines count as the same shape: a sum of
/// absolute log edge-length ratios, so 1.0 is about 10% per edge over nine edges.
pub const FIT_OK: f64 = 1.0;
/// Two rotations whose shape costs differ by less than this are a tie that doors may break.
pub(super) const SHAPE_TIE: f64 = 0.15;
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
    /// The person laid a box over the room by hand (`EdgeMap::Frame`): the recipes are resolved
    /// against the box, so nothing about the room's own shape was judged.
    Framed,
}

/// How the blueprint's edges are mapped onto the room's.
#[derive(Debug, Clone, PartialEq)]
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
    /// A box the person moved, turned and stretched over the room (`assignments::Frame`). **The box
    /// is the reference the recipes resolve against; the room's own outline only judges the
    /// findings** (`outside`), and its doors are not compared (the box has none of its own).
    Frame(Frame),
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
    /// For a room's ADDITION (`item_id` begins `add-`): the blueprint item it is a copy of, which is where its
    /// family, properties and kit are read from. Absent for a blueprint item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clone_of: Option<String>,
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
    /// The room is the blueprint's shape reflected: it needs the other hand's blueprint.
    MirrorImage { message: String },
    /// The outlines do not agree in shape.
    ShapeMismatch { message: String },
    /// The blueprint's doors are not where the room's are, at this mapping.
    DoorsDisagree { message: String },
    /// An override names an item this revision does not place (it was deleted, excluded, or the
    /// assignment now pins another revision). Reported, never an error: authored data outlives
    /// what it names.
    OverrideUnused { item: String },
    /// A nudge or turn was made against a recipe the item no longer has (a newer revision moved it), so it is HELD
    /// and not applied until a person says whether it still fits. Left out of an exclude, which does not depend on
    /// where the item is.
    OverrideStale { item: String },
    /// A room's addition that could not be placed: the item it clones is gone from the pinned revision (or excluded
    /// in it), or the room has no such edge. Reported, never an error: authored data outlives what it names.
    AdditionUnused { item: String, reason: String },
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
    /// The room's additions that could be placed: a copy of the item each clones, under the addition's own id.
    added: Vec<Added<'a>>,
}

/// A room's addition, resolved against the blueprint: `item` is a copy of the item it clones, carrying the addition's id.
struct Added<'a> {
    item: &'a BlueprintItem,
    clone_of: &'a str,
    edge: usize,
    along_mm: f64,
    off_mm: f64,
    facing_deg: f64,
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
                clone_of: None,
            });
        }
        for a in &self.added {
            let Some(pose) = placed.get(&a.item.id) else { continue };
            out.push(Placed {
                item_id: a.item.id.clone(),
                type_name: a.item.type_name.clone(),
                category: a.item.category.clone(),
                class: a.item.class,
                centre: pose.c.into(),
                facing: (pose.angle.cos(), pose.angle.sin()).into(),
                size_mm: a.item.size_mm,
                edge: pose.edge,
                rule: pose.rule.clone(),
                clone_of: Some(a.clone_of.to_string()),
            });
        }
        out
    }

    /// The room's whole layout: the recipes place the items, the overrides change them (an exclude removes one, a nudge
    /// or turn moves one), the additions are put on the room's own edges, and only then are `outside` and `overlap`
    /// judged, so they are judged where everything now is.
    fn lay_out(&self, overrides: &[ItemOverride], t_fine: &[Pt], findings: &mut Vec<Finding>) -> Vec<Placed> {
        let mut placed = self.place_all(findings);
        apply_overrides(self, &mut placed, overrides, findings);
        for a in &self.added {
            placed.insert(a.item.id.clone(), self.added_pose(a));
        }
        let placements = self.placements(&placed, findings);
        self.footprint_findings(&placed, t_fine, findings);
        placements
    }

    /// Where a room's addition stands: against the room's OWN edge, by the distances the person gave.
    fn added_pose(&self, a: &Added<'_>) -> Pose {
        let e = self.es_t[a.edge];
        Pose {
            c: e.point(a.along_mm, a.off_mm),
            angle: e.n.1.atan2(e.n.0) + a.facing_deg.to_radians(),
            edge: a.edge,
            rule: format!(
                "added for this room: a copy of {}, edge {}, {:.0} mm along, {:.0} mm from the wall",
                a.clone_of, a.edge, a.along_mm, a.off_mm
            ),
        }
    }

    /// Outside the true outline, and new overlaps, for free items only: a wall device is located
    /// by its wall and carries no footprint check.
    fn footprint_findings(&self, placed: &BTreeMap<String, Pose>, t_fine: &[Pt], findings: &mut Vec<Finding>) {
        // A room's additions are judged like any item: against the room's outline, and for new overlaps.
        let free: Vec<&BlueprintItem> = self
            .live
            .iter()
            .copied()
            .chain(self.added.iter().map(|a| a.item))
            .filter(|i| i.class == ItemClass::Free)
            .collect();
        let boxes_in_room: BTreeMap<&str, [Pt; 4]> = free
            .iter()
            .filter_map(|i| placed.get(&i.id).map(|p| (i.id.as_str(), corners_of(i, p.c, p.angle))))
            .collect();
        let boxes_in_blueprint: BTreeMap<&str, [Pt; 4]> = free
            .iter()
            .filter_map(|i| self.bp_frame.get(i.id.as_str()).map(|(c, a)| (i.id.as_str(), corners_of(i, *c, *a))))
            .collect();
        // Only what the room CHANGES: an item already outside its own source room (the model has
        // some, a few in a typical bedroom) would otherwise be reported on every resolve.
        for i in &free {
            let Some(now) = boxes_in_room.get(i.id.as_str()) else {
                continue;
            };
            let inside_now = now.iter().all(|&p| inside(p, t_fine, INSIDE_TOL_MM));
            // An addition has no place in the blueprint to have been outside, so it is judged where it stands.
            let inside_then = boxes_in_blueprint
                .get(i.id.as_str())
                .is_none_or(|was| was.iter().all(|&p| inside(p, &self.bp_fine, INSIDE_TOL_MM)));
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
    resolve_with_overrides(content, target_ring, mapping, target_doors, &[])
}

/// A person's changes to single items, applied to the poses the recipes gave (see
/// `ItemOverride`). **After placement and before the findings**, so `outside` and `overlap` are
/// judged on where the item now is. It touches its own item only: a follower keeps the place the
/// resolver gave it.
fn apply_overrides(
    ctx: &Context<'_>,
    placed: &mut BTreeMap<String, Pose>,
    overrides: &[ItemOverride],
    findings: &mut Vec<Finding>,
) {
    for o in overrides {
        if o.exclude {
            if placed.remove(&o.item_id).is_none() {
                findings.push(Finding::OverrideUnused { item: o.item_id.clone() });
            }
            continue;
        }
        let Some(pose) = placed.get_mut(&o.item_id) else {
            findings.push(Finding::OverrideUnused { item: o.item_id.clone() });
            continue;
        };
        // Made against a recipe the item no longer has: held, not applied. An override with no `base` (made before
        // it existed) is not checked.
        if let (Some(base), Some(item)) = (&o.base, ctx.live.iter().find(|i| i.id == o.item_id))
            && o.moves()
            && *base != item.recipe_hash()
        {
            findings.push(Finding::OverrideStale { item: o.item_id.clone() });
            continue;
        }
        // In the item's own frame first (along the edge it stands against, then into the room), so "away from the
        // wall" survives a turn of the room; then the plan-frame nudge a drag on the plan gives.
        if (o.along_mm != 0 || o.off_mm != 0)
            && let Some(e) = ctx.es_t.get(pose.edge)
        {
            pose.c = (
                pose.c.0 + e.t.0 * f64::from(o.along_mm) + e.n.0 * f64::from(o.off_mm),
                pose.c.1 + e.t.1 * f64::from(o.along_mm) + e.n.1 * f64::from(o.off_mm),
            );
        }
        pose.c = (pose.c.0 + f64::from(o.dx_mm), pose.c.1 + f64::from(o.dy_mm));
        pose.angle += f64::from(o.rotate_deg).to_radians();
        pose.rule = format!("{} (changed for this room)", pose.rule);
    }
}

/// The blueprint's coarse outline as a person laid it over `room` (`Frame`): the reference the
/// recipes resolve against. Same edges as the blueprint, so the mapping is the identity.
fn box_over(bp_ring: &[Pt], room: &[Pt], f: &Frame) -> Result<Vec<Pt>, ServiceError> {
    if bp_ring.len() < 3 {
        return Err(ServiceError::Invalid("the blueprint's outline has fewer than three corners".to_string()));
    }
    let (c0, room_at) = (centroid(bp_ring), centroid(room));
    let centre = (room_at.0 + f.dx_mm, room_at.1 + f.dy_mm);
    Ok(bp_ring.iter().map(|&p| f.place(p, c0, centre)).collect())
}

/// A copy of the item each addition clones, under the addition's own id and standing alone (a follower is placed where
/// its recipe says, and an addition has its own). One whose item is gone from the revision, or whose edge the room
/// does not have, is reported and left out.
fn addition_copies<'a>(
    additions: &'a [ItemAddition],
    live: &[&BlueprintItem],
    edge_count: usize,
    findings: &mut Vec<Finding>,
) -> Vec<(&'a ItemAddition, BlueprintItem)> {
    let mut copies = Vec::new();
    for a in additions {
        match live.iter().find(|i| i.id == a.clone_of) {
            None => findings.push(Finding::AdditionUnused {
                item: a.id.clone(),
                reason: format!(
                    "the blueprint item {:?} it copies is not in this revision, or is excluded",
                    a.clone_of
                ),
            }),
            Some(_) if a.edge >= edge_count => findings.push(Finding::AdditionUnused {
                item: a.id.clone(),
                reason: format!("the room has no edge {} (it has {edge_count})", a.edge),
            }),
            Some(source) => {
                let mut copy = (*source).clone();
                copy.id = a.id.clone();
                copy.follows = None;
                copies.push((a, copy));
            }
        }
    }
    copies
}

/// Each addition with the copy it stands as, ready to place.
fn resolved_additions<'a>(copies: &'a [(&'a ItemAddition, BlueprintItem)]) -> Vec<Added<'a>> {
    copies
        .iter()
        .map(|(a, copy)| Added {
            item: copy,
            clone_of: &a.clone_of,
            edge: a.edge,
            along_mm: f64::from(a.along_mm),
            off_mm: f64::from(a.off_mm),
            facing_deg: f64::from(a.facing_deg),
        })
        .collect()
}

/// `resolve_with_doors` with a room's own item overrides.
pub fn resolve_with_overrides(
    content: &RevisionContent,
    target_ring: &[Pt],
    mapping: &EdgeMap,
    target_doors: &[SourceDoor],
    overrides: &[ItemOverride],
) -> Result<Resolution, ServiceError> {
    resolve_with_changes(content, target_ring, mapping, target_doors, overrides, &[])
}

/// `resolve_with_overrides` and the items this room has that the blueprint does not (`ItemAddition`).
///
/// The room's whole difference from the blueprint, in the order it is applied: the recipes place the items, the
/// overrides change them (an exclude removes one, a nudge or turn moves one), then the additions are put on the
/// room's own edges, and only then are `outside` and `overlap` judged, so they are judged where everything now is.
pub fn resolve_with_changes(
    content: &RevisionContent,
    target_ring: &[Pt],
    mapping: &EdgeMap,
    target_doors: &[SourceDoor],
    overrides: &[ItemOverride],
    additions: &[ItemAddition],
) -> Result<Resolution, ServiceError> {
    let bp_ring: Vec<Pt> = content.outline.coarse.iter().map(|&p| p.into()).collect();
    let framed = if let EdgeMap::Frame(f) = mapping {
        Some(f.checked().map_err(ServiceError::Invalid)?)
    } else {
        None
    };
    let (t_coarse, t_fine) = match &framed {
        Some(f) => (box_over(&bp_ring, target_ring, f)?, simplify(target_ring, FINE_TOLERANCE_MM)),
        None => (coarse_outline(target_ring), simplify(target_ring, FINE_TOLERANCE_MM)),
    };
    if t_coarse.len() < 3 {
        return Err(ServiceError::Invalid("the room's outline has fewer than three corners".to_string()));
    }
    let outline = Outline {
        coarse: t_coarse.iter().map(|&p| p.into()).collect(),
        fine: t_fine.iter().map(|&p| p.into()).collect(),
    };
    let t_doors = if framed.is_some() { Vec::new() } else { doors_on(target_doors, &t_coarse) };
    let mut candidates = Vec::new();
    let mut findings = Vec::new();
    let excluded = content.items.iter().filter(|i| i.excluded).count();

    let (fit, edge_map) = if framed.is_some() {
        (
            Fit { kind: FitKind::Framed, shift: 0, cost: None, mirror_cost: None },
            (0..bp_ring.len()).collect(),
        )
    } else if let EdgeMap::Explicit(map) = mapping {
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
    if framed.is_none() {
        door_findings(&content.doors, &t_doors, &edge_map, &edges(&t_coarse), &mut findings);
    }
    let es_bp = edges(&bp_ring);
    let live: Vec<&BlueprintItem> = content.items.iter().filter(|i| !i.excluded).collect();
    let bp_frame: BTreeMap<&str, (Pt, f64)> =
        live.iter().filter_map(|i| frame_in(i, &es_bp).map(|f| (i.id.as_str(), f))).collect();
    let bp_fine: Vec<Pt> = content.outline.fine.iter().map(|&p| p.into()).collect();
    let es_t = edges(&t_coarse);
    let copies = addition_copies(additions, &live, es_t.len(), &mut findings);
    let added = resolved_additions(&copies);
    let ctx = Context { live, bp_frame, es_t, t_coarse: &t_coarse, bp_fine, edge_map, added };

    let placements = ctx.lay_out(overrides, &t_fine, &mut findings);

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
    resolve_in_room_with(state, project, content, room, mapping, &[])
}

/// `resolve_in_room` with the room's own item overrides (an assignment's, or a preview's).
pub fn resolve_in_room_with(
    state: &AppState,
    project: &str,
    content: &RevisionContent,
    room: &RoomRef,
    mapping: &EdgeMap,
    overrides: &[ItemOverride],
) -> Result<Option<(String, Resolution)>, ServiceError> {
    resolve_in_room_changes(state, project, content, room, mapping, overrides, &[])
}

/// `resolve_in_room_with` and the room's additions: the whole of an assignment's difference from its blueprint.
pub fn resolve_in_room_changes(
    state: &AppState,
    project: &str,
    content: &RevisionContent,
    room: &RoomRef,
    mapping: &EdgeMap,
    overrides: &[ItemOverride],
    additions: &[ItemAddition],
) -> Result<Option<(String, Resolution)>, ServiceError> {
    let Some((room_name, ring)) = room_ring(state, project, room)? else {
        return Ok(None);
    };
    let doors = room_doors(state, project, room)?;
    Ok(Some((
        room_name,
        resolve_with_changes(content, &ring, mapping, &doors, overrides, additions)?,
    )))
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
            hosting: None,
            kit: None,
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

    fn nudge(id: &str, dx: i32, dy: i32, rot: i32) -> ItemOverride {
        ItemOverride {
            item_id: id.to_string(),
            dx_mm: dx,
            dy_mm: dy,
            rotate_deg: rot,
            ..Default::default()
        }
    }

    fn two_items() -> (Vec<Pt>, RevisionContent) {
        let ring = rect(6000.0, 4000.0);
        let c = content(
            &ring,
            vec![
                src("bed", (3000.0, 600.0), 1600.0, 1000.0, (0.0, 1.0)),
                src("table", (3000.0, 2200.0), 800.0, 800.0, (0.0, 1.0)),
            ],
        );
        (ring, c)
    }

    #[test]
    fn test_an_override_moves_and_turns_its_own_item_only() {
        let (ring, c) = two_items();
        let plain = resolve_with_doors(&c, &ring, &EdgeMap::Auto, &[]).unwrap();
        let o = resolve_with_overrides(&c, &ring, &EdgeMap::Auto, &[], &[nudge("table", 150, -50, 90)]).unwrap();
        assert!(near(at(&o, "table"), (at(&plain, "table").0 + 150.0, at(&plain, "table").1 - 50.0), 1e-6));
        assert!(near(at(&o, "bed"), at(&plain, "bed"), 1e-9), "the other item is where it was");
        let (a, b) = (placed(&plain, "table").facing, placed(&o, "table").facing);
        // A quarter turn counter-clockwise: (x, y) -> (-y, x).
        assert!((b.x + a.y).abs() < 1e-9 && (b.y - a.x).abs() < 1e-9, "{a:?} -> {b:?}");
        assert!(placed(&o, "table").rule.contains("changed for this room"));
    }

    #[test]
    fn test_an_excluded_override_leaves_the_item_out_and_an_unknown_one_is_reported() {
        let (ring, c) = two_items();
        let leave_out = ItemOverride { item_id: "bed".to_string(), exclude: true, ..Default::default() };
        let r = resolve_with_overrides(&c, &ring, &EdgeMap::Auto, &[], &[leave_out, nudge("ghost", 10, 0, 0)]).unwrap();
        assert!(r.placements.iter().all(|p| p.item_id != "bed"));
        assert_eq!(r.placements.len(), 1);
        assert!(
            r.findings.contains(&Finding::OverrideUnused { item: "ghost".to_string() }),
            "{:?}",
            r.findings
        );
    }

    #[test]
    fn test_findings_are_judged_where_the_override_put_the_item() {
        let (ring, c) = two_items();
        let plain = resolve_with_doors(&c, &ring, &EdgeMap::Auto, &[]).unwrap();
        assert!(!plain.findings.iter().any(|f| matches!(f, Finding::Outside { .. })));
        let out = resolve_with_overrides(&c, &ring, &EdgeMap::Auto, &[], &[nudge("table", 9000, 0, 0)]);
        // 9000 mm is a legal nudge for the resolver (the store bounds it); the table is now outside.
        let out = out.unwrap();
        assert!(
            out.findings.iter().any(|f| matches!(f, Finding::Outside { item } if item == "table")),
            "{:?}",
            out.findings
        );
    }

    fn addition(id: &str, clone_of: &str, edge: usize, along: i32, off: i32, facing: i32) -> ItemAddition {
        ItemAddition {
            id: id.to_string(),
            clone_of: clone_of.to_string(),
            edge,
            along_mm: along,
            off_mm: off,
            facing_deg: facing,
        }
    }

    #[test]
    fn test_an_addition_is_a_copy_of_its_item_standing_where_the_person_put_it_and_nothing_else_moves() {
        let (ring, c) = two_items();
        let plain = resolve_with_doors(&c, &ring, &EdgeMap::Shift(0), &[]).unwrap();
        // The room's edge 1 is the right-hand wall (x = 6000, running up), whose inward normal points at -x.
        let r =
            resolve_with_changes(&c, &ring, &EdgeMap::Shift(0), &[], &[], &[addition("add-1", "bed", 1, 500, 100, 0)])
                .unwrap();
        assert_eq!(r.placements.len(), 3, "the blueprint's two items and the room's one more");
        assert!(near(at(&r, "add-1"), (5900.0, 500.0), 1e-6), "{:?}", at(&r, "add-1"));
        let (added, bed) = (placed(&r, "add-1"), placed(&r, "bed"));
        assert_eq!(added.clone_of.as_deref(), Some("bed"), "its family and properties are the bed's");
        assert_eq!((&added.type_name, added.size_mm, added.class), (&bed.type_name, bed.size_mm, bed.class));
        assert!(bed.clone_of.is_none(), "a blueprint item clones nothing");
        assert!(near(at(&r, "bed"), at(&plain, "bed"), 1e-9) && near(at(&r, "table"), at(&plain, "table"), 1e-9));
        assert!(added.rule.contains("added for this room"), "{}", added.rule);
        // Facing is from the edge's inward normal.
        let turned =
            resolve_with_changes(&c, &ring, &EdgeMap::Shift(0), &[], &[], &[addition("add-1", "bed", 1, 500, 100, 90)])
                .unwrap();
        let f = placed(&turned, "add-1").facing;
        assert!(
            (f.x - 0.0).abs() < 1e-9 && (f.y + 1.0).abs() < 1e-9,
            "-x turned a quarter counter-clockwise is -y: {f:?}"
        );
    }

    #[test]
    fn test_an_addition_whose_item_or_edge_is_gone_is_reported_and_the_rest_still_resolves() {
        let (ring, c) = two_items();
        let r = resolve_with_changes(
            &c,
            &ring,
            &EdgeMap::Auto,
            &[],
            &[],
            &[
                addition("add-1", "ghost", 0, 0, 100, 0),
                addition("add-2", "bed", 9, 0, 100, 0),
            ],
        )
        .unwrap();
        assert_eq!(r.placements.len(), 2, "nothing was added");
        let unused: Vec<&str> = r
            .findings
            .iter()
            .filter_map(|f| match f {
                Finding::AdditionUnused { item, .. } => Some(item.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(vec!["add-1", "add-2"], unused);
    }

    #[test]
    fn test_an_addition_is_judged_like_any_item_outside_the_room_and_on_top_of_another() {
        let (ring, c) = two_items();
        let outside = resolve_with_changes(
            &c,
            &ring,
            &EdgeMap::Shift(0),
            &[],
            &[],
            &[addition("add-1", "bed", 0, 3000, -900, 0)],
        )
        .unwrap();
        assert!(
            outside.findings.iter().any(|f| matches!(f, Finding::Outside { item } if item == "add-1")),
            "{:?}",
            outside.findings
        );
        // On the table: edge 0 is the bottom wall, so 3000 along and 2200 in is the table's own centre.
        let on_top = resolve_with_changes(
            &c,
            &ring,
            &EdgeMap::Shift(0),
            &[],
            &[],
            &[addition("add-1", "bed", 0, 3000, 2200, 0)],
        )
        .unwrap();
        assert!(
            on_top
                .findings
                .iter()
                .any(|f| matches!(f, Finding::Overlap { a, b } if (a == "add-1" || b == "add-1"))),
            "{:?}",
            on_top.findings
        );
        let clear =
            resolve_with_changes(&c, &ring, &EdgeMap::Shift(0), &[], &[], &[addition("add-1", "bed", 1, 1500, 600, 0)])
                .unwrap();
        assert!(!clear
            .findings
            .iter()
            .any(|f| matches!(f, Finding::Outside { .. } | Finding::Overlap { .. })));
    }

    fn based(mut o: ItemOverride, base: &str) -> ItemOverride {
        o.base = Some(base.to_string());
        o
    }

    #[test]
    fn test_a_nudge_made_against_another_recipe_is_held_and_reported_but_an_exclude_and_an_unbased_nudge_apply() {
        let (ring, c) = two_items();
        let plain = resolve_with_doors(&c, &ring, &EdgeMap::Auto, &[]).unwrap();
        let table = c.items.iter().find(|i| i.id == "table").unwrap();
        // Made against this very recipe: applies.
        let fresh = resolve_with_overrides(
            &c,
            &ring,
            &EdgeMap::Auto,
            &[],
            &[based(nudge("table", 150, 0, 0), &table.recipe_hash())],
        )
        .unwrap();
        assert!(near(at(&fresh, "table"), (at(&plain, "table").0 + 150.0, at(&plain, "table").1), 1e-6));
        assert!(!fresh.findings.iter().any(|f| matches!(f, Finding::OverrideStale { .. })));
        // Made against a recipe the item no longer has: held, and said.
        let stale = resolve_with_overrides(
            &c,
            &ring,
            &EdgeMap::Auto,
            &[],
            &[based(nudge("table", 150, 0, 0), "0000000000000000")],
        )
        .unwrap();
        assert!(near(at(&stale, "table"), at(&plain, "table"), 1e-9), "held, not applied");
        assert!(
            stale.findings.contains(&Finding::OverrideStale { item: "table".to_string() }),
            "{:?}",
            stale.findings
        );
        // Made before `base` existed: not checked.
        let old = resolve_with_overrides(&c, &ring, &EdgeMap::Auto, &[], &[nudge("table", 150, 0, 0)]).unwrap();
        assert!(near(at(&old, "table"), (at(&plain, "table").0 + 150.0, at(&plain, "table").1), 1e-6));
        // Leaving an item out does not depend on where it is, so a stale base does not stop it.
        let gone = ItemOverride {
            item_id: "bed".to_string(),
            exclude: true,
            base: Some("0000000000000000".into()),
            ..Default::default()
        };
        let r = resolve_with_overrides(&c, &ring, &EdgeMap::Auto, &[], &[gone]).unwrap();
        assert!(r.placements.iter().all(|p| p.item_id != "bed"));
        assert!(!r.findings.iter().any(|f| matches!(f, Finding::OverrideStale { .. })));
    }

    #[test]
    fn test_a_recipe_hash_changes_when_the_item_is_put_somewhere_else_and_not_when_it_is_only_retyped() {
        let (_, c) = two_items();
        let bed = c.items.iter().find(|i| i.id == "bed").unwrap().clone();
        let mut retyped = bed.clone();
        retyped.type_name = "Another bed".to_string();
        retyped.size_mm = [2000.0, 1200.0];
        assert_eq!(
            bed.recipe_hash(),
            retyped.recipe_hash(),
            "a re-specified item that stays put keeps its room's nudges"
        );
        let mut moved = bed.clone();
        moved.placement.along_mm += 50.0;
        assert_ne!(bed.recipe_hash(), moved.recipe_hash());
        let mut turned = bed.clone();
        turned.placement.facing_deg += 90.0;
        assert_ne!(bed.recipe_hash(), turned.recipe_hash());
        assert_eq!(16, bed.recipe_hash().len());
    }

    fn turned_ring(ring: &[Pt]) -> Vec<Pt> {
        ring.iter().map(|&(x, y)| (-y, x)).collect()
    }

    #[test]
    fn test_a_nudge_in_the_items_own_frame_follows_the_room_when_it_is_turned() {
        let (ring, c) = two_items();
        let turned = turned_ring(&ring);
        let item_frame = |o: ItemOverride, r: &[Pt]| {
            let plain = resolve_with_doors(&c, r, &EdgeMap::Shift(0), &[]).unwrap();
            let moved = resolve_with_overrides(&c, r, &EdgeMap::Shift(0), &[], &[o]).unwrap();
            let (a, b) = (at(&plain, "bed"), at(&moved, "bed"));
            (b.0 - a.0, b.1 - a.1)
        };
        let along = ItemOverride { item_id: "bed".to_string(), along_mm: 100, ..Default::default() };
        let off = ItemOverride { item_id: "bed".to_string(), off_mm: 50, ..Default::default() };
        // The bed stands against edge 0. Upright, that wall runs +x and the room is at +y.
        assert!(near(item_frame(along.clone(), &ring), (100.0, 0.0), 1e-6));
        assert!(near(item_frame(off.clone(), &ring), (0.0, 50.0), 1e-6));
        // Turned a quarter, the same two nudges are the same two things: along the wall, and away from it.
        assert!(near(item_frame(along, &turned), (0.0, 100.0), 1e-6), "along the wall, which now runs +y");
        assert!(near(item_frame(off, &turned), (-50.0, 0.0), 1e-6), "into the room, which is now at -x");
        // The plan-frame nudge is what it always was, and does not turn.
        let plan = nudge("bed", 100, 0, 0);
        assert!(near(item_frame(plan, &turned), (100.0, 0.0), 1e-6));
    }

    fn frame(dx: f64, dy: f64, rot: f64, sx: f64, sy: f64) -> Frame {
        Frame { dx_mm: dx, dy_mm: dy, rotate_deg: rot, sx, sy }
    }

    #[test]
    fn test_a_box_turns_stretches_and_moves_about_the_blueprints_pivot() {
        let f = frame(100.0, -50.0, 90.0, 2.0, 0.5);
        let c0 = (1000.0, 1000.0);
        // The pivot goes to the centre; a point 10 right of it is stretched to 20, then turned to point up.
        let at = f.place(c0, c0, (0.0, 0.0));
        assert!(near(at, (0.0, 0.0), 1e-9));
        let p = f.place((1010.0, 1000.0), c0, (0.0, 0.0));
        assert!(near(p, (0.0, 20.0), 1e-9), "{p:?}");
        let q = f.place((1000.0, 1010.0), c0, (0.0, 0.0));
        assert!(near(q, (-5.0, 0.0), 1e-9), "{q:?}");
    }

    #[test]
    fn test_a_box_outside_what_a_stretch_can_be_is_refused_and_a_turn_is_wrapped() {
        assert!(frame(0.0, 0.0, 0.0, 0.1, 1.0).checked().is_err());
        assert!(frame(0.0, 0.0, 0.0, 1.0, 9.0).checked().is_err());
        assert!(frame(0.0, 0.0, f64::NAN, 1.0, 1.0).checked().is_err());
        assert!((frame(0.0, 0.0, 270.0, 1.0, 1.0).checked().unwrap().rotate_deg + 90.0).abs() < 1e-9);
    }

    #[test]
    fn test_an_identity_box_on_the_blueprints_own_room_puts_every_item_back() {
        let (ring, c) = two_items();
        let plain = resolve_with_doors(&c, &ring, &EdgeMap::Auto, &[]).unwrap();
        let r = resolve_with_overrides(&c, &ring, &EdgeMap::Frame(frame(0.0, 0.0, 0.0, 1.0, 1.0)), &[], &[]).unwrap();
        assert_eq!(r.fit.kind, FitKind::Framed);
        for id in ["bed", "table"] {
            assert!(near(at(&r, id), at(&plain, id), 1e-6), "{id}: {:?} vs {:?}", at(&r, id), at(&plain, id));
        }
        assert_eq!(r.outline.coarse.len(), c.outline.coarse.len(), "the box has the blueprint's edges");
    }

    #[test]
    fn test_a_stretched_box_keeps_item_sizes_and_moves_the_items_with_it() {
        let (ring, c) = two_items();
        let plain = resolve_with_doors(&c, &ring, &EdgeMap::Auto, &[]).unwrap();
        let r = resolve_with_overrides(&c, &ring, &EdgeMap::Frame(frame(0.0, 0.0, 0.0, 1.5, 1.0)), &[], &[]).unwrap();
        for id in ["bed", "table"] {
            assert_eq!(placed(&r, id).size_mm, placed(&plain, id).size_mm, "items keep their size");
        }
        // The box is 1.5 times as wide, about the room's centre: its outline spans 9000 mm in x.
        let xs: Vec<f64> = r.outline.coarse.iter().map(|p| p.x).collect();
        let width = xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
        assert!((width - 9000.0).abs() < 1e-6, "{width}");
        // The table sits mid-room, so it stays on the room's centre line.
        assert!((at(&r, "table").0 - at(&plain, "table").0).abs() < 1e-3);
    }

    #[test]
    fn test_a_turned_box_turns_the_items_and_judges_findings_against_the_real_room() {
        let (ring, c) = two_items();
        let plain = resolve_with_doors(&c, &ring, &EdgeMap::Auto, &[]).unwrap();
        let r = resolve_with_overrides(&c, &ring, &EdgeMap::Frame(frame(0.0, 0.0, 90.0, 1.0, 1.0)), &[], &[]).unwrap();
        let (a, b) = (placed(&plain, "bed").facing, placed(&r, "bed").facing);
        assert!((b.x + a.y).abs() < 1e-9 && (b.y - a.x).abs() < 1e-9, "{a:?} -> {b:?}");
        // A quarter turn of a 6000 x 4000 box puts it 4000 wide in a 6000 x 4000 room: some of it is outside.
        let wide =
            resolve_with_overrides(&c, &ring, &EdgeMap::Frame(frame(0.0, 0.0, 0.0, 1.0, 1.0)), &[], &[]).unwrap();
        assert!(!wide.findings.iter().any(|f| matches!(f, Finding::Outside { .. })));
        let off =
            resolve_with_overrides(&c, &ring, &EdgeMap::Frame(frame(5000.0, 0.0, 0.0, 1.0, 1.0)), &[], &[]).unwrap();
        assert!(off.findings.iter().any(|f| matches!(f, Finding::Outside { .. })), "{:?}", off.findings);
    }
}
