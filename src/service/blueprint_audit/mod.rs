//! The room-state audit: which FF&E in a room is the assigned blueprint's, by code and by place.
//!
//! The resolver answers "where would this blueprint put its items in this room" and reads no existing FF&E.
//! This answers the question beside it: set that layout against what the room actually holds, and say, per
//! element and per item, whether it is **in place**, **moved**, **missing**, **extra**, **surplus**,
//! **foreign**, **substituted**, or one of the two things nothing is done about (`no_code`,
//! `unsupported_host`). Everything here is a read: it moves, adds and removes nothing.
//!
//! Two axes, judged separately because they fail separately. **Code** (the project's type-code parameter, the
//! same identity the tag rules use) answers "is this the right thing"; **location** answers "is it in the right
//! place". There is deliberately no family-and-type-name fallback: a wrong match here would move or remove a
//! real element.
//!
//! **What this does NOT judge yet, and the audit says so rather than staying silent:** nothing here knows which
//! elements Revit placed from a task, because nothing records that (the ledger, build step 4), so every room is
//! `Empty`, `Untracked` or `CannotJudge` and a verdict about a found element is an inference from code and
//! place, never a proof. The ledger-only verdicts (`drifted`, `retired`, `removed_by_hand`, `moved_out`), the
//! `component_edited` (a nested instance's own properties differ: the blueprint stores no nested instances) and the
//! height check (`height_differs`) are not here either.
//!
//! The pure part is [`audit`] over [`Layout`] and [`Element`], so it is tested without a server; `reads` makes
//! those from an assignment and the project's FF&E.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::blueprint_geometry::Pt;
use super::blueprint_model::{Hosting, ItemClass, KitPart};
use roommate_shared::settings::BlueprintAuditPolicy;

mod reads;
pub use reads::{audit_assignment, audit_basis, audit_content, AssignmentAudit};

/// The family placement types an item can have and still be judged by a point in plan: sitting on a level, or
/// on a work plane that is a level. Anything else hangs on something the plan point cannot judge.
const LEVEL_PLACEMENTS: [&str; 3] = ["OneLevelBased", "TwoLevelsBased", "WorkPlaneBased"];

/// Why an item is not judged (`unsupported_host`). A reason a person can act on, never a bare "no".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unsupported {
    /// The model did not say how the family is placed (an older snapshot, or an older duHast): cannot tell,
    /// which is not the same as level-based.
    PlacementUnknown,
    /// Hosted on a wall, a line or something else the plan point cannot judge.
    Placement,
    /// Hosted on a face of another element (a ceiling, a roof, stairs, another family).
    FaceHosted,
    /// A work-plane family that names no level.
    WorkPlaneNotOnLevel,
    /// A blueprint item with no hosting recorded that is a wall device by its size (an older blueprint).
    WallDevice,
}

/// Whether an item is judged by the audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Yes,
    /// A blueprint item captured before hosting was recorded, taken as level-based because nothing says
    /// otherwise. Counted, so the answer says how much of it rests on an assumption.
    Assumed,
    No(Unsupported),
}

/// The one rule that separates what the first iteration acts on from what it only flags, applied to a layout
/// item and a room's element alike.
///
/// **Measured on a real model** (2026-11-28, 822 FF&E instances): the family placement type alone would accept
/// about 90 face-hosted work-plane items, and `host` alone would accept the wall-hosted and line-based ones,
/// whose `Host` is empty. So both are needed:
/// supported = a placement type of `OneLevelBased`, `TwoLevelsBased` or `WorkPlaneBased`, **and** not
/// face-hosted, **and** a host that is a Level (a work-plane family MUST name one; the other two may have none).
///
/// A category is read by its English name (`Levels`), which is how the extractor sends it; a Revit run in
/// another language would flag every work-plane item, which is the safe side and visible in the counts.
pub fn supported(hosting: Option<&Hosting>) -> Support {
    let Some(h) = hosting else {
        return Support::No(Unsupported::PlacementUnknown);
    };
    let Some(placement) = h.placement_type.as_deref() else {
        return Support::No(Unsupported::PlacementUnknown);
    };
    if !LEVEL_PLACEMENTS.contains(&placement) {
        return Support::No(Unsupported::Placement);
    }
    if h.face_hosted {
        return Support::No(Unsupported::FaceHosted);
    }
    let on_level = h.host_kind.as_deref().is_some_and(|k| k.trim().eq_ignore_ascii_case("levels"));
    match (placement, h.host_kind.as_deref()) {
        ("WorkPlaneBased", _) if !on_level => Support::No(Unsupported::WorkPlaneNotOnLevel),
        (_, Some(_)) if !on_level => Support::No(Unsupported::Placement),
        _ => Support::Yes,
    }
}

/// A blueprint item as the audit reads it: where the resolver puts it in this room, with the room's own
/// overrides already applied (an excluded item is simply not here).
#[derive(Debug, Clone)]
pub struct Layout {
    pub id: String,
    pub code: Option<String>,
    pub type_name: String,
    pub family_name: Option<String>,
    /// Centre in the project's plan frame, millimetres.
    pub centre: Pt,
    /// Unit vector of the facing, same frame.
    pub facing: Pt,
    /// How it was hosted when captured; `None` on a blueprint captured before that was recorded.
    pub hosting: Option<Hosting>,
    pub class: ItemClass,
    /// What its family brings with it; `None` when the blueprint was captured before that was recorded.
    pub kit: Option<Vec<KitPart>>,
}

impl Layout {
    fn support(&self) -> Support {
        match (&self.hosting, self.class) {
            (Some(_), _) => supported(self.hosting.as_ref()),
            (None, ItemClass::WallDevice) => Support::No(Unsupported::WallDevice),
            (None, ItemClass::Free) => Support::Assumed,
        }
    }
}

/// An element of the room as the audit reads it: a HOST (a nested component is a part of its host's family and is
/// never judged on its own).
#[derive(Debug, Clone)]
pub struct Element {
    pub id: String,
    pub unique_id: Option<String>,
    pub code: Option<String>,
    pub type_name: String,
    pub family_name: Option<String>,
    /// The middle of its footprint, else its insertion point; project plan frame, millimetres. `None` when the
    /// model gave it neither.
    pub centre: Option<Pt>,
    pub facing: Option<Pt>,
    pub hosting: Hosting,
    /// What its family brings with it; `None` when the snapshot cannot say (see `blueprint_capture::kit_of`).
    pub kit: Option<Vec<KitPart>>,
}

/// What a verdict says. Ordered so a summary reads from fine to needs-a-person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    /// Right thing, right place and facing.
    InPlace,
    /// Right host, right place, different parts inside: a nested family changed, or the family was reloaded
    /// with other parts. **Never `in_place`**, because a person cannot see it on the plan. Reported only: a
    /// nested instance can be changed through its host alone, and a family reload is refused.
    KitDiffers,
    /// Right thing, near its place but not in it (or facing the wrong way).
    Moved,
    /// Right code, found in the room, but with no location to judge (the room's fit is unusable).
    PresentByCode,
    /// A layout item with nothing in the room that is it.
    Missing,
    /// Right code, in the room, but too far from any item of that code to be one of them.
    Extra,
    /// More elements of a code the layout has than the layout has items.
    Surplus,
    /// An element whose code the layout does not have at all: not the blueprint's.
    Foreign,
    /// Something else stands where a layout item should.
    Substituted,
    /// No type code, so nothing can be matched. Flagged and nothing else.
    NoCode,
    /// Out of scope in this iteration. Flagged and nothing else.
    UnsupportedHost,
}

/// Which side of the comparison a flag is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// A blueprint item.
    Layout,
    /// An element of the room.
    Room,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Verdict {
    pub kind: VerdictKind,
    /// The blueprint item, when the verdict is about one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    /// The element of the room, when the verdict is about one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The type of the element, or of the item when there is no element.
    pub type_name: String,
    /// Centre to centre, millimetres, for a pair.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance_mm: Option<f64>,
    /// Facing difference in degrees, for a pair whose facing both sides state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_deg: Option<f64>,
    /// For `no_code` and `unsupported_host`: which side, and for the latter why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsupported: Option<Unsupported>,
    /// For a matched pair: whether the two hosts carry the same parts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kit: Option<KitState>,
    /// For a pair whose kits differ: the parts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kit_diff: Option<KitDiff>,
}

/// Whether a matched host carries the blueprint's parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KitState {
    Same,
    Differs,
    /// One side cannot say (an older snapshot or blueprint): no evidence, **never agreement**, as a door gap
    /// with no door on either side is not agreement.
    Unknown,
}

/// What differs between a layout item's parts and an element's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KitDiff {
    /// In the blueprint's kit and not the element's.
    pub missing: Vec<KitPart>,
    /// In the element's and not the blueprint's.
    pub extra: Vec<KitPart>,
}

/// The parts one list has and the other lacks, as multisets: two of the same part against one is one part short.
fn multiset_difference(a: &[KitPart], b: &[KitPart]) -> Vec<KitPart> {
    let mut left: Vec<&KitPart> = b.iter().collect();
    let mut out = Vec::new();
    for part in a {
        if let Some(at) = left.iter().position(|q| *q == part) {
            left.swap_remove(at);
        } else {
            out.push(part.clone());
        }
    }
    out
}

fn compare_kit(layout: Option<&[KitPart]>, element: Option<&[KitPart]>) -> (KitState, Option<KitDiff>) {
    let (Some(want), Some(have)) = (layout, element) else {
        return (KitState::Unknown, None);
    };
    let (missing, extra) = (multiset_difference(want, have), multiset_difference(have, want));
    if missing.is_empty() && extra.is_empty() {
        (KitState::Same, None)
    } else {
        (KitState::Differs, Some(KitDiff { missing, extra }))
    }
}

/// How much the audit can say about the room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomState {
    /// Nothing in the room: every layout item is missing and a populate fills it.
    Empty,
    /// Furnished by hand or by another process, and nothing records what is the blueprint's: matched by code
    /// and place, an inference.
    Untracked,
    /// The blueprint does not fit the room (`edge_count_differs`, `mirror_image`, `no_fit`), so there are no
    /// expected positions: presence by code only.
    CannotJudge,
}

#[derive(Debug, Clone, Serialize)]
pub struct Audit {
    pub state: RoomState,
    /// The tolerances the verdicts were judged by, so an answer carries what it was measured against.
    pub place_tolerance_mm: f64,
    pub near_tolerance_mm: f64,
    pub turn_tolerance_deg: f64,
    /// How many layout items and room elements were looked at.
    pub layout_items: usize,
    pub elements: usize,
    /// Layout items judged on a hosting the blueprint did not record (an older blueprint): an assumption,
    /// counted so it is visible.
    pub placement_assumed: usize,
    /// Matched hosts whose kit could not be compared because one side cannot say.
    pub kit_unknown: usize,
    pub counts: BTreeMap<VerdictKind, usize>,
    pub verdicts: Vec<Verdict>,
}

fn dist(a: Pt, b: Pt) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The signed difference of two unit vectors' headings, absolute, degrees in `0..=180`.
fn turn_between(a: Pt, b: Pt) -> f64 {
    (a.0 * b.1 - a.1 * b.0).atan2(a.0 * b.0 + a.1 * b.1).to_degrees().abs()
}

/// Minimum-total-cost assignment of `rows` to `cols` (Hungarian, potentials form), for a cost matrix that is
/// not necessarily square. Returns, per row, the column it is paired with. Groups are the items of ONE code in
/// ONE room, so the cubic cost is a few elements.
fn assign(cost: &[Vec<f64>], rows: usize, cols: usize) -> Vec<Option<usize>> {
    if rows == 0 || cols == 0 {
        return vec![None; rows];
    }
    if rows > cols {
        // Solve the transpose and read the answer back.
        let t: Vec<Vec<f64>> = (0..cols).map(|c| (0..rows).map(|r| cost[r][c]).collect()).collect();
        let by_col = assign(&t, cols, rows);
        let mut out = vec![None; rows];
        for (c, r) in by_col.iter().enumerate() {
            if let Some(r) = r {
                out[*r] = Some(c);
            }
        }
        return out;
    }
    // rows <= cols: e-maxx's potentials form, 1-based.
    let (n, m) = (rows, cols);
    let mut u = vec![0.0_f64; n + 1];
    let mut v = vec![0.0_f64; m + 1];
    let mut p = vec![0_usize; m + 1];
    let mut way = vec![0_usize; m + 1];
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0_usize;
        let mut minv = vec![f64::INFINITY; m + 1];
        let mut used = vec![false; m + 1];
        loop {
            used[j0] = true;
            let (i0, mut delta, mut j1) = (p[j0], f64::INFINITY, 0_usize);
            for j in 1..=m {
                if !used[j] {
                    let cur = cost[i0 - 1][j - 1] - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=m {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut out = vec![None; n];
    for j in 1..=m {
        if p[j] != 0 {
            out[p[j] - 1] = Some(j - 1);
        }
    }
    out
}

fn verdict_for_element(kind: VerdictKind, e: &Element) -> Verdict {
    Verdict {
        kind,
        item_id: None,
        element_id: Some(e.id.clone()),
        unique_id: e.unique_id.clone(),
        code: e.code.clone(),
        type_name: e.type_name.clone(),
        distance_mm: None,
        turn_deg: None,
        side: None,
        unsupported: None,
        kit: None,
        kit_diff: None,
    }
}

fn verdict_for_item(kind: VerdictKind, l: &Layout) -> Verdict {
    Verdict {
        kind,
        item_id: Some(l.id.clone()),
        element_id: None,
        unique_id: None,
        code: l.code.clone(),
        type_name: l.type_name.clone(),
        distance_mm: None,
        turn_deg: None,
        side: None,
        unsupported: None,
        kit: None,
        kit_diff: None,
    }
}

fn flag(kind: VerdictKind, side: Side, why: Option<Unsupported>, mut v: Verdict) -> Verdict {
    v.kind = kind;
    v.side = Some(side);
    v.unsupported = why;
    v
}

/// Set a layout against a room's elements.
///
/// `judge_location` is false when the blueprint does not fit the room, so the layout has no positions: the
/// audit then compares by code only and says `cannot_judge`.
pub fn audit(layout: &[Layout], elements: &[Element], judge_location: bool, policy: &BlueprintAuditPolicy) -> Audit {
    let mut verdicts: Vec<Verdict> = Vec::new();
    let mut placement_assumed = 0_usize;
    let mut kit_unknown = 0_usize;

    // Partition both sides. Order matters: an unsupported item is not asked for its code, because its code says
    // nothing the audit may act on.
    let mut items: Vec<&Layout> = Vec::new();
    for l in layout {
        match l.support() {
            Support::No(why) => verdicts.push(flag(
                VerdictKind::UnsupportedHost,
                Side::Layout,
                Some(why),
                verdict_for_item(VerdictKind::UnsupportedHost, l),
            )),
            support => {
                if support == Support::Assumed {
                    placement_assumed += 1;
                }
                if l.code.is_none() {
                    verdicts.push(flag(
                        VerdictKind::NoCode,
                        Side::Layout,
                        None,
                        verdict_for_item(VerdictKind::NoCode, l),
                    ));
                } else {
                    items.push(l);
                }
            }
        }
    }
    let mut found: Vec<&Element> = Vec::new();
    for e in elements {
        match supported(Some(&e.hosting)) {
            Support::No(why) => verdicts.push(flag(
                VerdictKind::UnsupportedHost,
                Side::Room,
                Some(why),
                verdict_for_element(VerdictKind::UnsupportedHost, e),
            )),
            _ if e.code.is_none() => {
                verdicts.push(flag(VerdictKind::NoCode, Side::Room, None, verdict_for_element(VerdictKind::NoCode, e)));
            }
            _ => found.push(e),
        }
    }

    let state = if elements.is_empty() {
        RoomState::Empty
    } else if judge_location {
        RoomState::Untracked
    } else {
        RoomState::CannotJudge
    };

    if judge_location {
        match_by_place(&items, &found, policy, &mut verdicts);
        kit_unknown = verdicts.iter().filter(|v| v.kit == Some(KitState::Unknown)).count();
    } else {
        match_by_code(&items, &found, &mut verdicts);
    }

    let mut counts: BTreeMap<VerdictKind, usize> = BTreeMap::new();
    for v in &verdicts {
        *counts.entry(v.kind).or_default() += 1;
    }
    // A stable order, so the same room reads the same: by kind (fine to needs-a-person), then by what it names.
    verdicts.sort_by(|a, b| {
        (a.kind, a.item_id.as_deref(), a.element_id.as_deref()).cmp(&(
            b.kind,
            b.item_id.as_deref(),
            b.element_id.as_deref(),
        ))
    });
    Audit {
        state,
        place_tolerance_mm: policy.place_tolerance_mm,
        near_tolerance_mm: policy.near_tolerance_mm,
        turn_tolerance_deg: policy.turn_tolerance_deg,
        layout_items: layout.len(),
        elements: elements.len(),
        placement_assumed,
        kit_unknown,
        counts,
        verdicts,
    }
}

/// The by-place match: within one code, pair items and elements by smallest total distance (not greedy nearest,
/// so two same-code items swap correctly); a pair beyond the near tolerance is not a pair.
fn match_by_place(items: &[&Layout], found: &[&Element], policy: &BlueprintAuditPolicy, out: &mut Vec<Verdict>) {
    let codes_in_layout: BTreeSet<&str> = items.iter().filter_map(|l| l.code.as_deref()).collect();
    let codes: BTreeSet<&str> = codes_in_layout
        .iter()
        .copied()
        .chain(found.iter().filter_map(|e| e.code.as_deref()))
        .collect();
    // What no pairing took, kept for the substitution pass.
    let mut loose_items: Vec<&Layout> = Vec::new();
    let mut loose_elements: Vec<&Element> = Vec::new();
    // The cap that keeps one far pair from dragging the others: past it every pair costs the same, so the
    // optimum is chosen among the pairs that are in reach.
    let cap = 2.0 * policy.near_tolerance_mm.max(1.0);

    for code in &codes {
        let ls: Vec<&Layout> = items.iter().copied().filter(|l| l.code.as_deref() == Some(code)).collect();
        let es: Vec<&Element> = found.iter().copied().filter(|e| e.code.as_deref() == Some(code)).collect();
        let distance = |l: &Layout, e: &Element| e.centre.map_or(f64::INFINITY, |c| dist(l.centre, c));
        let cost: Vec<Vec<f64>> = ls.iter().map(|l| es.iter().map(|e| distance(l, e).min(cap)).collect()).collect();
        let pairing = assign(&cost, ls.len(), es.len());
        let mut taken = vec![false; es.len()];
        for (i, l) in ls.iter().enumerate() {
            let Some(j) = pairing[i] else {
                loose_items.push(l);
                continue;
            };
            let d = distance(l, es[j]);
            if d > policy.near_tolerance_mm {
                loose_items.push(l);
                continue;
            }
            taken[j] = true;
            let turn = es[j].facing.map(|f| turn_between(l.facing, f));
            let facing_agrees = turn.is_none_or(|t| t <= policy.turn_tolerance_deg);
            let kind = if d <= policy.place_tolerance_mm && facing_agrees {
                VerdictKind::InPlace
            } else {
                VerdictKind::Moved
            };
            let (kit, kit_diff) = compare_kit(l.kit.as_deref(), es[j].kit.as_deref());
            // In place and in the right place, but not the same thing inside: never `in_place`.
            let kind = if kind == VerdictKind::InPlace && kit == KitState::Differs {
                VerdictKind::KitDiffers
            } else {
                kind
            };
            let mut v = verdict_for_element(kind, es[j]);
            v.item_id = Some(l.id.clone());
            v.distance_mm = Some(d);
            v.turn_deg = turn;
            v.kit = Some(kit);
            v.kit_diff = kit_diff;
            out.push(v);
        }
        for (j, e) in es.iter().enumerate() {
            if !taken[j] {
                loose_elements.push(e);
            }
        }
    }

    let (item_done, element_done) = substitutions(&loose_items, &loose_elements, policy, out);

    // What is left, by code. Both sides left over: the same kind of thing is in the room but not where any item
    // is (extra) and the item is missing. Only elements left over: more than the layout wants (surplus), or a
    // code the layout never had (foreign). Only items left over: missing.
    for code in &codes {
        let ul: Vec<&Layout> = loose_items
            .iter()
            .enumerate()
            .filter(|(i, l)| !item_done[*i] && l.code.as_deref() == Some(code))
            .map(|(_, l)| *l)
            .collect();
        let ue: Vec<&Element> = loose_elements
            .iter()
            .enumerate()
            .filter(|(j, e)| !element_done[*j] && e.code.as_deref() == Some(code))
            .map(|(_, e)| *e)
            .collect();
        let extra_kind = if !ul.is_empty() {
            VerdictKind::Extra
        } else if codes_in_layout.contains(code) {
            VerdictKind::Surplus
        } else {
            VerdictKind::Foreign
        };
        out.extend(ul.iter().map(|l| verdict_for_item(VerdictKind::Missing, l)));
        out.extend(ue.iter().map(|e| verdict_for_element(extra_kind, e)));
    }
}

/// Substitution: something of ANOTHER code stands where an item should (within the place tolerance). It is one
/// finding, not a `foreign` plus a `missing`. Returns which loose items and elements it used.
fn substitutions(
    loose_items: &[&Layout],
    loose_elements: &[&Element],
    policy: &BlueprintAuditPolicy,
    out: &mut Vec<Verdict>,
) -> (Vec<bool>, Vec<bool>) {
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (i, l) in loose_items.iter().enumerate() {
        for (j, e) in loose_elements.iter().enumerate() {
            if e.code != l.code
                && let Some(c) = e.centre
            {
                let d = dist(l.centre, c);
                if d <= policy.place_tolerance_mm {
                    pairs.push((d, i, j));
                }
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut item_done, mut element_done) = (vec![false; loose_items.len()], vec![false; loose_elements.len()]);
    for (d, i, j) in pairs {
        if item_done[i] || element_done[j] {
            continue;
        }
        item_done[i] = true;
        element_done[j] = true;
        let mut v = verdict_for_element(VerdictKind::Substituted, loose_elements[j]);
        v.item_id = Some(loose_items[i].id.clone());
        v.distance_mm = Some(d);
        out.push(v);
    }
    (item_done, element_done)
}

/// Presence by code, for a room the blueprint does not fit: no expected positions, so no pairing by place and
/// no distance. Counts only, per code.
fn match_by_code(items: &[&Layout], found: &[&Element], out: &mut Vec<Verdict>) {
    let codes_in_layout: BTreeSet<&str> = items.iter().filter_map(|l| l.code.as_deref()).collect();
    let codes: BTreeSet<&str> = codes_in_layout
        .iter()
        .copied()
        .chain(found.iter().filter_map(|e| e.code.as_deref()))
        .collect();
    for code in &codes {
        let ls: Vec<&&Layout> = items.iter().filter(|l| l.code.as_deref() == Some(code)).collect();
        let es: Vec<&&Element> = found.iter().filter(|e| e.code.as_deref() == Some(code)).collect();
        let both = ls.len().min(es.len());
        for (l, e) in ls.iter().zip(es.iter()).take(both) {
            let mut v = verdict_for_element(VerdictKind::PresentByCode, e);
            v.item_id = Some(l.id.clone());
            out.push(v);
        }
        out.extend(ls.iter().skip(both).map(|l| verdict_for_item(VerdictKind::Missing, l)));
        let more = if codes_in_layout.contains(code) {
            VerdictKind::Surplus
        } else {
            VerdictKind::Foreign
        };
        out.extend(es.iter().skip(both).map(|e| verdict_for_element(more, e)));
    }
}

#[cfg(test)]
mod tests;
