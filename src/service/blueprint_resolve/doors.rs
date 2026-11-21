//! A blueprint's doors against the target room's: how far apart they stand on the mapped edges (`door_gap`), which door matches which, and
//! the findings when they disagree. Doors break a tie of shape and never override a pinned mapping.

use super::*;

/// What a blueprint door scores when the room has no door on the edge it lands on.
pub const DOOR_UNMATCHED_MM: f64 = 3_000.0;

/// A chosen mapping whose doors are further apart than this is reported.
pub const DOOR_DISAGREE_MM: f64 = 600.0;

/// Mean distance from each blueprint door to the nearest door of the room on the edge it maps to,
/// taking a position as the same fraction along the edge. `None` when there is nothing to compare.
pub(super) fn door_gap(
    bp: &[BlueprintDoor],
    room: &[BlueprintDoor],
    edge_map: &[usize],
    t_edges: &[Edge],
) -> Option<f64> {
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
pub(super) fn doors_on(doors: &[SourceDoor], coarse: &[Pt]) -> Vec<BlueprintDoor> {
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
pub(super) fn map_doors(doors: &[BlueprintDoor], edge_map: &[usize], t_edges: &[Edge]) -> Vec<BlueprintDoor> {
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
pub(super) fn door_findings(
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
