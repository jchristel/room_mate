//! Reshaping a blueprint's reference outline: the box a person edits in the blueprint viewer.
//!
//! **Items do not move when the box does.** An item is a recipe against an edge, so changing the
//! outline would change where every item is. What a person means is the opposite: the items are
//! where they are, and the box is being fitted to them. So each item's centre and facing are read
//! out of the OLD outline (they are exact, `Placement::centre_in`), and written again as recipes
//! against the NEW one, the way capture writes them (`place_item`). The same goes for doors.
//!
//! **An item on a wall that did not change keeps its recipe whole**, modes included: a person who
//! chose "fixed from the end" on a wardrobe should not lose that because another wall moved.
//! Everything else takes capture's modes, because a mode is a choice about one particular edge
//! and there is nothing to carry it to.
//!
//! The fine outline is the room as it really is and is not touched here: it is what findings are
//! judged against, so reshaping the box never makes an item "outside".
//!
//! Pure: no store. The page asks, shows the answer as its draft, and a save is the ordinary new
//! revision.

use super::blueprint_capture::{find_follows, place_item, segment_distance, Measured, SourceItem};
use super::blueprint_geometry::{bounding_rect, coarse_outline, dist, dot, edges, is_simple, signed_area, Pt};
use super::blueprint_model::{BlueprintDoor, BlueprintItem, Outline, Point, RevisionContent};
use super::ServiceError;

/// A box is at least a rectangle's four corners.
pub const MIN_NODES: usize = 4;
const MAX_NODES: usize = 64;
/// A side shorter than this is two nodes on top of each other.
const MIN_SIDE_MM: f64 = 10.0;
/// A wall counts as unchanged when both its ends are within this of where they were.
const SAME_WALL_MM: f64 = 0.5;

/// What the new reference outline is.
pub enum Target {
    /// These nodes, as the person placed them.
    Nodes(Vec<Pt>),
    /// The rectangle round the room's real outline, in the blueprint's orientation.
    Rectangle,
    /// The room's own coarse outline (what a blueprint was made against before it started from a rectangle).
    RoomOutline,
}

fn invalid<T>(msg: impl Into<String>) -> Result<T, ServiceError> {
    Err(ServiceError::Invalid(msg.into()))
}

/// The box as it will be kept: enough nodes, finite, no side collapsed, not crossing itself, and
/// counter-clockwise (a clockwise box is turned round, because every edge's inward side is its left).
pub fn checked_ring(mut ring: Vec<Pt>) -> Result<Vec<Pt>, ServiceError> {
    if ring.len() < MIN_NODES {
        return invalid(format!("a box has at least {MIN_NODES} nodes"));
    }
    if ring.len() > MAX_NODES {
        return invalid(format!("a box has at most {MAX_NODES} nodes"));
    }
    if ring.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return invalid("a box has finite coordinates");
    }
    if edges(&ring).iter().any(|e| e.len < MIN_SIDE_MM) {
        return invalid(format!(
            "a side of the box is shorter than {MIN_SIDE_MM} mm: two nodes are on top of each other"
        ));
    }
    if !is_simple(&ring) {
        return invalid("the box crosses itself");
    }
    if signed_area(&ring) < 0.0 {
        ring.reverse();
    }
    Ok(ring)
}

/// The content with its reference outline replaced and every item and door written against it.
pub fn reshape(content: &RevisionContent, target: Target) -> Result<RevisionContent, ServiceError> {
    let old: Vec<Pt> = content.outline.coarse.iter().map(|&p| p.into()).collect();
    let fine: Vec<Pt> = content.outline.fine.iter().map(|&p| p.into()).collect();
    let new = checked_ring(match target {
        Target::Nodes(nodes) => nodes,
        Target::Rectangle => bounding_rect(&fine),
        Target::RoomOutline => coarse_outline(&fine),
    })?;
    let (old_es, new_es) = (edges(&old), edges(&new));

    // New edge index of each old edge that is still there.
    let survivor: Vec<Option<usize>> = old_es
        .iter()
        .map(|o| {
            new_es
                .iter()
                .position(|n| dist(o.a, n.a) <= SAME_WALL_MM && dist(o.b, n.b) <= SAME_WALL_MM)
        })
        .collect();

    let mut placed: Vec<(BlueprintItem, Measured)> = Vec::with_capacity(content.items.len());
    for item in &content.items {
        let p = &item.placement;
        let Some(e) = old_es.get(p.edge) else {
            return invalid(format!("item {} is on edge {}, which the outline does not have", item.id, p.edge));
        };
        let c = e.point(p.along_mm, p.off_mm);
        let (s, k) = p.facing_deg.to_radians().sin_cos();
        let facing = (e.n.0 * k - e.n.1 * s, e.n.0 * s + e.n.1 * k);
        let [width, depth] = item.size_mm;
        let footprint = super::blueprint_geometry::box_corners(c, facing, width / 2.0, depth / 2.0).to_vec();
        let source = SourceItem {
            id: item.id.clone(),
            family_name: item.family_name.clone(),
            type_name: item.type_name.clone(),
            category: item.category.clone(),
            footprint,
            facing: Some(facing),
            nested: false,
            properties: item.properties.clone(),
            type_properties: item.type_properties.clone(),
        };
        let (mut again, measured) = place_item(source, (0.0, 0.0), &new, &new_es);
        // What the person decided, not what a rule would guess.
        again.excluded = item.excluded;
        again.class = item.class;
        again.size_mm = item.size_mm;
        if survivor[p.edge] == Some(again.placement.edge) {
            again.placement.along_mode = p.along_mode;
            again.placement.off_mode = p.off_mode;
        }
        placed.push((again, measured));
    }
    find_follows(&mut placed);

    let doors: Vec<BlueprintDoor> = content
        .doors
        .iter()
        .filter_map(|d| {
            let e = old_es.get(d.edge)?;
            let c = e.point(d.along_mm, 0.0);
            let (i, _) = new_es
                .iter()
                .enumerate()
                .map(|(i, n)| (i, segment_distance(c, n)))
                .min_by(|a, b| a.1.total_cmp(&b.1))?;
            let n = new_es[i];
            Some(BlueprintDoor {
                id: d.id.clone(),
                edge: i,
                along_mm: dot((c.0 - n.a.0, c.1 - n.a.1), n.t),
                edge_len_mm: n.len,
                width_mm: d.width_mm,
            })
        })
        .collect();

    Ok(RevisionContent {
        outline: Outline {
            coarse: new.iter().map(|&p| Point::from(p)).collect(),
            fine: content.outline.fine.clone(),
        },
        items: placed.into_iter().map(|(i, _)| i).collect(),
        doors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::blueprint_capture::{capture_shaped, Reference, SourceDoor};
    use crate::service::blueprint_geometry::rotate;

    fn room() -> Vec<Pt> {
        // An L: 6 m x 4 m with a 2 m x 2 m notch out of the top right.
        vec![
            (0.0, 0.0),
            (6000.0, 0.0),
            (6000.0, 2000.0),
            (4000.0, 2000.0),
            (4000.0, 4000.0),
            (0.0, 4000.0),
        ]
    }

    fn bed(id: &str, c: Pt) -> SourceItem {
        SourceItem {
            id: id.to_string(),
            family_name: None,
            type_name: "Bed".to_string(),
            category: "Furniture".to_string(),
            footprint: crate::service::blueprint_geometry::box_corners(c, (1.0, 0.0), 1000.0, 500.0).to_vec(),
            facing: Some((1.0, 0.0)),
            nested: false,
            properties: Vec::new(),
            type_properties: Vec::new(),
        }
    }

    fn centres(c: &RevisionContent) -> Vec<(String, Pt)> {
        let ring: Vec<Pt> = c.outline.coarse.iter().map(|&p| p.into()).collect();
        c.items.iter().map(|i| (i.id.clone(), i.placement.centre_in(&ring).unwrap())).collect()
    }

    fn captured(turn: f64) -> RevisionContent {
        let ring: Vec<Pt> = room().into_iter().map(|p| rotate(p, turn)).collect();
        let items = vec![
            bed("a", rotate((1500.0, 1000.0), turn)),
            bed("b", rotate((1200.0, 3000.0), turn)),
        ];
        let doors = vec![SourceDoor {
            id: "d".to_string(),
            footprint: crate::service::blueprint_geometry::box_corners(
                rotate((0.0, 2000.0), turn),
                (0.0, 1.0),
                450.0,
                60.0,
            )
            .to_vec(),
        }];
        capture_shaped(&ring, items, doors, Reference::Box).unwrap().content
    }

    #[test]
    fn test_a_capture_starts_as_a_rectangle_squared_up_however_the_room_is_turned() {
        for turn in [0.0, 31.0, 100.0, -17.5] {
            let c = captured(turn);
            assert_eq!(c.outline.coarse.len(), 4, "turn {turn}");
            let ring: Vec<Pt> = c.outline.coarse.iter().map(|&p| p.into()).collect();
            assert!(ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .all(|(a, b)| (a.0 - b.0).abs() < 1e-6 || (a.1 - b.1).abs() < 1e-6));
            // The real outline rides beside it and is the L, not the box.
            assert_eq!(c.outline.fine.len(), 6);
        }
    }

    #[test]
    fn test_a_turned_room_gives_the_same_recipes_as_a_square_one() {
        let (a, b) = (captured(0.0), captured(31.0));
        let strip = |c: &RevisionContent| -> Vec<(usize, i64, i64)> {
            c.items
                .iter()
                .map(|i| (i.placement.edge, i.placement.along_mm.round() as i64, i.placement.off_mm.round() as i64))
                .collect()
        };
        assert_eq!(strip(&a), strip(&b));
    }

    #[test]
    fn test_reshaping_to_the_room_keeps_every_item_where_it_was() {
        let before = captured(0.0);
        let after = reshape(&before, Target::RoomOutline).unwrap();
        assert_eq!(after.outline.coarse.len(), 6);
        let (x, y) = (centres(&before), centres(&after));
        for ((ida, pa), (idb, pb)) in x.iter().zip(&y) {
            assert_eq!(ida, idb);
            assert!(dist(*pa, *pb) < 1e-6, "{ida} moved {pa:?} -> {pb:?}");
        }
        assert_eq!(after.doors.len(), 1);
        let d = &after.doors[0];
        assert!(d.edge < 6 && d.along_mm >= 0.0 && d.along_mm <= d.edge_len_mm);
    }

    #[test]
    fn test_moving_one_node_leaves_items_on_other_walls_with_their_modes() {
        let mut before = captured(0.0);
        let ring: Vec<Pt> = before.outline.coarse.iter().map(|&p| p.into()).collect();
        let on = before.items[0].placement.edge;
        before.items[0].placement.along_mode = crate::service::blueprint_model::AlongMode::FixedFromEnd;
        // Pull a corner away from the wall the first item is NOT on.
        let far = (on + 2) % 4;
        let mut nodes = ring.clone();
        nodes[far] = (nodes[far].0 + 300.0, nodes[far].1 + 300.0);
        let after = reshape(&before, Target::Nodes(nodes)).unwrap();
        for ((_, pa), (_, pb)) in centres(&before).iter().zip(centres(&after).iter()) {
            assert!(dist(*pa, *pb) < 1e-6);
        }
        let kept = after.items.iter().find(|i| i.id == before.items[0].id).unwrap();
        assert_eq!(kept.placement.along_mode, crate::service::blueprint_model::AlongMode::FixedFromEnd);
    }

    #[test]
    fn test_a_box_with_three_nodes_or_a_crossing_is_refused_and_a_clockwise_one_is_turned_round() {
        let c = captured(0.0);
        let tri = vec![(0.0, 0.0), (6000.0, 0.0), (0.0, 4000.0)];
        assert!(reshape(&c, Target::Nodes(tri)).is_err());
        let bow = vec![(0.0, 0.0), (6000.0, 4000.0), (6000.0, 0.0), (0.0, 4000.0)];
        assert!(reshape(&c, Target::Nodes(bow)).is_err());
        let cw = vec![(0.0, 0.0), (0.0, 4000.0), (6000.0, 4000.0), (6000.0, 0.0)];
        let out = reshape(&c, Target::Nodes(cw)).unwrap();
        assert!(signed_area(&out.outline.coarse.iter().map(|&p| p.into()).collect::<Vec<Pt>>()) > 0.0);
    }
}
