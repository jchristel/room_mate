//! The area hierarchy: rooms classified into tiers, grouped per level, sibling overlaps resolved and exclusions applied
//! (`hierarchy_area_groups`, `level_groups`). The footprint of each group is `footprint.rs`.

use super::*;

/// A room paired with its resolved classification path — the input to the tier
/// dissolve. In production this comes straight off a `RoomResponse` (which
/// already carries both `room` and `classification`); kept as a borrowed pair so
/// `areas` doesn't depend on the `rooms` service.
pub struct ClassifiedRoom<'a> {
    pub room: &'a Room,
    pub path: &'a [TierValue],
}

/// One hierarchy group's dissolved footprint at one tier, on one level.
#[derive(Debug)]
pub struct AreaGroup {
    pub level_id: String,
    /// The resolved path prefix identifying this group, outermost first. Its
    /// length is the tier depth: a top-tier group has one element, a bottom-tier
    /// group the full path. The last element is this group's own tier value.
    pub path: Vec<TierValue>,
    pub footprint: MultiPolygon<f64>,
    pub area: f64,
    /// `false` when a Case-A (`group`) exclusion withholds this group from its
    /// parent's dissolve — the group is still reported (its own area is real),
    /// but it does not contribute to any tier above it. Always `true` for a group
    /// no exclusion targets.
    pub counted_upward: bool,
}

/// The Phase-2 pipeline: per-level, per-tier footprints for a set of classified
/// rooms. Rooms are partitioned by level first (footprints never union across
/// floors — a per-level decision), then each level builds its wall zone once and
/// hands every tier its share of it. Every tier's area is measured from that
/// tier's own polygon, never summed from children. Returns bottom-tier groups
/// first, then each tier above.
///
/// `gaps` supplies the wall gap per level, which is where the declared boundary
/// regime enters the geometry — see [`LevelGaps`].
pub fn hierarchy_area_groups(
    rooms: &[ClassifiedRoom],
    exclusions: &[HierarchyExclusion],
    gaps: &LevelGaps,
) -> Vec<AreaGroup> {
    // Case-B exclusion: drop excluded rooms before they become geometry, so they
    // vanish from every tier including their own bottom group — and, since the
    // wall zone is built from the survivors, so do the walls only they bounded.
    // Partition the survivors by level, first-seen order for determinism.
    let mut by_level: Vec<(String, Vec<&ClassifiedRoom>)> = Vec::new();
    for cr in rooms {
        if is_room_excluded(&cr.room.id, exclusions) {
            continue;
        }
        match by_level.iter_mut().find(|(lid, _)| lid == &cr.room.level_id) {
            Some((_, v)) => v.push(cr),
            None => by_level.push((cr.room.level_id.clone(), vec![cr])),
        }
    }

    let mut out = Vec::new();
    for (level_id, level_rooms) in &by_level {
        out.extend(level_groups(level_id, level_rooms, exclusions, gaps.for_level(level_id)));
    }
    out
}

/// Withdraw from **both** claimants any area they each claim, at one tier.
///
/// This is not arbitration papering over an overlap the design did not expect —
/// it is the design's own rule applied to the one shape that still needs it. At a
/// wall junction where four rooms meet, the small square at the centre is bounded
/// by all four; if two belong to A and two to B, each pair's diagonal is short
/// enough to close and both groups fill the square. Bounded by two groups means
/// it belongs to neither, so it comes out of both, and their common ancestor —
/// whose close merges all four rooms — claims it exactly once. That is what keeps
/// `parent = Σ children + newly-enclosed bands` exact rather than approximate.
///
/// Everything else that used to land here is gone: a group's fill is a subset of
/// the wall zone, which contains no rooms, so a footprint can no longer reach
/// into a neighbour's room, and the close no longer bulges past the level's own
/// outer boundary. What remains is junction-sized.
///
/// Intersections are all computed *before* any subtraction, so the outcome does
/// not depend on group order.
pub(super) fn resolve_sibling_overlaps(groups: &mut [(Vec<TierValue>, MultiPolygon<f64>)]) {
    let n = groups.len();
    if n < 2 {
        return;
    }
    let boxes: Vec<_> = groups.iter().map(|(_, fp)| fp.bounding_rect()).collect();
    let mut contested: Vec<MultiPolygon<f64>> = vec![MultiPolygon::new(vec![]); n];
    for i in 0..n {
        for j in (i + 1)..n {
            // Bounding-box reject first: most sibling pairs are nowhere near each
            // other, and this keeps the pass off the expensive boolean path.
            match (boxes[i], boxes[j]) {
                (Some(a), Some(b)) => {
                    if a.max().x < b.min().x || b.max().x < a.min().x || a.max().y < b.min().y || b.max().y < a.min().y
                    {
                        continue;
                    }
                }
                _ => continue,
            }
            let inter = groups[i].1.intersection(&groups[j].1);
            if inter.0.is_empty() {
                continue;
            }
            contested[i] = contested[i].union(&inter);
            contested[j] = contested[j].union(&inter);
        }
    }
    for (g, c) in groups.iter_mut().zip(contested) {
        if !c.0.is_empty() {
            g.1 = g.1.difference(&c);
        }
    }
}

/// One bottom-tier group on one level: its full classification path, the union
/// of its rooms, and the depth (if any) at which a Case-A exclusion withholds it.
pub(super) struct BottomGroup {
    path: Vec<TierValue>,
    rooms: MultiPolygon<f64>,
    /// `Some(k)` when an exclusion names the tier at index `k` of this path. The
    /// group at `path[..=k]` is still reported, but contributes to no tier above
    /// it — it participates at depth `d` only for `d >= k`.
    excluded_at: Option<usize>,
}

/// The depth at which a Case-A (`group`) exclusion withholds this path, if any —
/// the shallowest tier whose resolved value an exclusion names. Shallowest
/// because withholding at tier `k` already removes the group from every tier
/// above `k`, so a second, deeper match would change nothing.
pub(super) fn excluded_depth(path: &[TierValue], exclusions: &[HierarchyExclusion]) -> Option<usize> {
    (0..path.len()).find(|&k| is_group_excluded(&path[..=k], exclusions))
}

/// One level's whole pipeline. Kept in one function on purpose: the wall zone,
/// the per-depth fills and the roll-up are three views of one construction, and
/// the invariant that makes it correct — each depth's footprints are disjoint and
/// nested inside their parent's — is only checkable by reading them together.
pub(super) fn level_groups(
    level_id: &str,
    rooms: &[&ClassifiedRoom],
    exclusions: &[HierarchyExclusion],
    gap_ft: f64,
) -> Vec<AreaGroup> {
    let num_tiers = rooms.iter().map(|r| r.path.len()).max().unwrap_or(0);
    if num_tiers == 0 {
        return Vec::new(); // no hierarchy configured -> no groups
    }

    // Gather rooms into bottom-tier groups by full-path key (first-seen order).
    // `classify_room` guarantees a uniform-depth path, so `path.len() ==
    // num_tiers` for every room; anything shorter is not a group we can place.
    let mut grouped: Vec<(Vec<TierValue>, Vec<&Room>)> = Vec::new();
    for cr in rooms {
        if cr.path.len() != num_tiers {
            continue;
        }
        match grouped.iter_mut().find(|(p, _)| path_eq(p, cr.path)) {
            Some((_, v)) => v.push(cr.room),
            None => grouped.push((cr.path.to_vec(), vec![cr.room])),
        }
    }

    let bottom: Vec<BottomGroup> = grouped
        .into_iter()
        .map(|(path, rs)| BottomGroup {
            rooms: union_of_rooms(rs.into_iter()),
            excluded_at: excluded_depth(&path, exclusions),
            path,
        })
        .collect();

    // The level's own geometry: every surviving room, and the wall zone those
    // rooms enclose. Both computed exactly once — that is the whole point of the
    // reformulation. `dirs` comes from the rooms, never from the closed result,
    // because a direction the *rooms* never had is by definition one the close
    // invented.
    let mut all_rooms: MultiPolygon<f64> = MultiPolygon::new(vec![]);
    for g in &bottom {
        all_rooms = all_rooms.union(&g.rooms);
    }
    let dirs = edge_dirs(&all_rooms);
    let zone = wall_zone(&all_rooms, gap_ft);

    // Walk the tiers deepest-first so the output keeps its established order
    // (bottom-tier groups first, then each tier above). Each depth is computed
    // from the **bottom groups directly**, never from the tier below it.
    //
    // Not because closing is non-idempotent — it is idempotent, `φ(φ(X)) = φ(X)`
    // (see the module header). The reason is that `geo`'s bevel-join offset is
    // not a true closing: it is not even extensive, since a bevel only ever cuts
    // a corner. So `approx(approx(X))` really does drift where `φ(φ(X))` would
    // not, and re-closing an already-closed footprint compounds the corner error
    // once per tier. Closing the raw rooms once per prefix keeps every tier at
    // exactly one approximation deep.
    let mut results: Vec<AreaGroup> = Vec::new();
    for depth in (0..num_tiers).rev() {
        // Group the participating bottom groups by their prefix at this depth.
        let mut prefixes: Vec<(Vec<TierValue>, MultiPolygon<f64>)> = Vec::new();
        for g in &bottom {
            // A Case-A exclusion at tier k withholds this group from every tier
            // above k. It still appears at k itself — its own area is real.
            if g.excluded_at.is_some_and(|k| depth < k) {
                continue;
            }
            let prefix = &g.path[..=depth];
            match prefixes.iter_mut().find(|(p, _)| path_eq(p, prefix)) {
                Some((_, acc)) => *acc = acc.union(&g.rooms),
                None => prefixes.push((prefix.to_vec(), g.rooms.clone())),
            }
        }

        // Each prefix's share of the wall zone: the bands its own rooms close
        // over. A band with this prefix's rooms on one side only is not among
        // them — that band belongs to an ancestor, which is the ownership rule
        // falling out of the arithmetic rather than being imposed on it.
        let mut footprints: Vec<(Vec<TierValue>, MultiPolygon<f64>)> = prefixes
            .into_iter()
            .map(|(path, own_rooms)| {
                let fill = if gap_ft > 0.0 {
                    morphological_close(&own_rooms, gap_ft / 2.0).intersection(&zone)
                } else {
                    MultiPolygon::new(vec![])
                };
                (path, own_rooms.union(&fill))
            })
            .collect();
        resolve_sibling_overlaps(&mut footprints);

        results.extend(footprints.into_iter().map(|(path, fp)| {
            let cleaned = clean_rings(&fp, &dirs, gap_ft);
            emit(level_id, path, &cleaned, exclusions)
        }));
    }

    results
}

pub(super) fn emit(
    level_id: &str,
    path: Vec<TierValue>,
    footprint: &MultiPolygon<f64>,
    exclusions: &[HierarchyExclusion],
) -> AreaGroup {
    let counted_upward = !is_group_excluded(&path, exclusions);
    AreaGroup {
        level_id: level_id.to_string(),
        area: footprint_area(footprint),
        footprint: footprint.clone(),
        path,
        counted_upward,
    }
}

/// Case B — is this room id withheld before geometry (stage 1)?
pub(super) fn is_room_excluded(id: &str, exclusions: &[HierarchyExclusion]) -> bool {
    exclusions.iter().any(|e| match e {
        HierarchyExclusion::Rooms { ids } => ids.iter().any(|x| x == id),
        HierarchyExclusion::Group { .. } => false,
    })
}

/// Case A — is this group (identified by its own, last tier value) withheld from
/// its parent's dissolve (stage 2)? Matches an exclusion whose `tier` names this
/// group's tier and whose `value` equals the resolved code or name.
pub(super) fn is_group_excluded(path: &[TierValue], exclusions: &[HierarchyExclusion]) -> bool {
    let Some(last) = path.last() else { return false };
    exclusions.iter().any(|e| match e {
        HierarchyExclusion::Group { tier, value } => {
            tier == &last.tier
                && (last.code.as_deref() == Some(value.as_str()) || last.name.as_deref() == Some(value.as_str()))
        }
        HierarchyExclusion::Rooms { .. } => false,
    })
}

// ============================ endpoint / wire shape ============================

/// Two classification prefixes name the same group when their resolved values
/// match tier-for-tier (`tier` label is positional and always agrees, so only
/// code/name/undefined need comparing).
pub(super) fn path_eq(a: &[TierValue], b: &[TierValue]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.code == y.code && x.name == y.name && x.undefined == y.undefined)
}
