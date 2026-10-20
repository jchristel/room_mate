//! Capture: a room and the FF&E in it, written as a blueprint revision's recipes.
//!
//! The rules are the ones measured on House A and RHH in stage 0
//! (`docs/PLAN-blueprints-stage0.md`), and each is here for a failure that run found:
//!
//! - **An item belongs to the edge it lies *beside*,** the nearest edge whose span
//!   contains it -- not the nearest segment. Nearest-segment put 6 items in notched rooms
//!   at an edge's end, where `along` clamps and the position cannot be reproduced.
//! - **Nested components are left out** (they move with their parent), and an item with
//!   no footprint has no position to write a recipe against. Both are counted, never silent.
//! - **A wall device** (under 100 mm thick) is its own class: 43% of House A's usable items.
//! - **Fixed or percentage by a rule a person can read:** an item within 300 mm of a corner
//!   keeps that gap, one within 150 mm of its wall keeps that gap, anything else is a
//!   percentage. Both thresholds are the plan's defaults; the mode is just data afterwards.
//! - **Runs:** an item abutting the previous one along its edge, or lying wholly inside
//!   another, follows it.
//!
//! Pure over millimetres. The adapter below reads the model and converts from feet.

use crate::contract::{DoorPayload, PropertyMap};
use crate::state::AppState;

use super::blueprint_geometry::{
    box_corners, centroid, cross, depth_at, dist, dot, edges, inside, simplify, sub, unit, Edge, Pt,
    COARSE_TOLERANCE_MM, FINE_TOLERANCE_MM,
};
use super::blueprint_model::{
    AlongMode, BlueprintDoor, BlueprintItem, Follow, ItemClass, OffMode, Outline, Placement, Point, Property,
    RevisionContent, Skipped,
};
use super::items::{assemble_items, ItemScope};
use super::openings::{assemble_openings, OpeningKind, OpeningScope};
use super::room_locator::RoomRef;
use super::rooms::{assemble_rooms, RoomScope};
use super::ServiceError;

/// Feet to millimetres: the wire's plan geometry is decimal feet.
pub const FT_TO_MM: f64 = 304.8;

/// An item within this of a corner keeps the gap to it.
pub const CORNER_GAP_MM: f64 = 300.0;
/// An item within this of its wall keeps the gap to it.
pub const WALL_GAP_MM: f64 = 150.0;
/// Thinner than this, in plan, is a wall device.
pub const WALL_DEVICE_MM: f64 = 100.0;
/// Two items closer than this along an edge are a run.
pub const RUN_GAP_MM: f64 = 50.0;

/// An FF&E item as capture reads it. Millimetres, in the project-local frame.
#[derive(Debug, Clone)]
pub struct SourceItem {
    pub id: String,
    pub family_name: Option<String>,
    pub type_name: String,
    pub category: String,
    pub footprint: Vec<Pt>,
    pub facing: Option<Pt>,
    pub nested: bool,
    pub properties: Vec<Property>,
    pub type_properties: Vec<Property>,
}

/// A door of the room, as capture reads it: only where it is. Millimetres, project-local frame.
#[derive(Debug, Clone)]
pub struct SourceDoor {
    pub id: String,
    pub footprint: Vec<Pt>,
}

/// Where a door lies against an outline: its edge, its distance along it and its width along it.
/// The nearest edge by segment distance, not by span: a door is IN the wall, so it is never
/// "beside" an edge in the sense an item is, and a door in a corner jog still has a nearest wall.
pub fn door_on_edges(footprint: &[Pt], coarse: &[Pt]) -> Option<(usize, f64, f64)> {
    if footprint.len() < 3 || coarse.len() < 3 {
        return None;
    }
    let c = centroid(footprint);
    let es = edges(coarse);
    let (i, _) = es
        .iter()
        .enumerate()
        .map(|(i, e)| (i, segment_distance(c, e)))
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    let e = es[i];
    let along = (c.0 - e.a.0) * e.t.0 + (c.1 - e.a.1) * e.t.1;
    let (lo, hi) = footprint.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
        let a = (p.0 - e.a.0) * e.t.0 + (p.1 - e.a.1) * e.t.1;
        (lo.min(a), hi.max(a))
    });
    Some((i, along, hi - lo))
}

/// A captured revision and what capture left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Captured {
    pub content: RevisionContent,
    pub skipped: Skipped,
}

/// The working figures for one item while placements are being worked out.
struct Measured {
    ht: f64,
    hn: f64,
    corners: [Pt; 4],
    area: f64,
}

fn translate(p: Pt, origin: Pt) -> Pt {
    (p.0 - origin.0, p.1 - origin.1)
}

/// Write `sources` as recipes against `ring`, the room's outer loop.
pub fn capture(ring: &[Pt], sources: Vec<SourceItem>) -> Result<Captured, ServiceError> {
    capture_with_doors(ring, sources, Vec::new())
}

/// `capture`, and the room's doors written against the same outline.
pub fn capture_with_doors(
    ring: &[Pt],
    sources: Vec<SourceItem>,
    doors: Vec<SourceDoor>,
) -> Result<Captured, ServiceError> {
    let fine = simplify(ring, FINE_TOLERANCE_MM);
    let coarse = simplify(ring, COARSE_TOLERANCE_MM);
    if coarse.len() < 3 {
        return Err(ServiceError::Invalid("the room's outline has fewer than three corners".to_string()));
    }
    // The room's centroid is the blueprint's origin, so a blueprint does not carry the
    // project's survey offset around with it.
    let origin = centroid(&fine);
    let fine: Vec<Pt> = fine.iter().map(|&p| translate(p, origin)).collect();
    let coarse: Vec<Pt> = coarse.iter().map(|&p| translate(p, origin)).collect();
    let es = edges(&coarse);

    let mut skipped = Skipped::default();
    let mut items: Vec<(BlueprintItem, Measured)> = Vec::new();
    for s in sources {
        if s.nested {
            skipped.nested += 1;
            continue;
        }
        if s.footprint.len() < 3 {
            skipped.no_footprint += 1;
            continue;
        }
        items.push(place_item(s, origin, &coarse, &es));
    }

    // A stable order, so two captures of one room are equal: edge, then along, then id.
    items.sort_by(|a, b| {
        (a.0.placement.edge, a.0.placement.along_mm, &a.0.id)
            .partial_cmp(&(b.0.placement.edge, b.0.placement.along_mm, &b.0.id))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    find_follows(&mut items);

    Ok(Captured {
        content: RevisionContent {
            outline: Outline {
                coarse: coarse.iter().map(|&p| Point::from(p)).collect(),
                fine: fine.iter().map(|&p| Point::from(p)).collect(),
            },
            items: items.into_iter().map(|(item, _)| item).collect(),
            doors: blueprint_doors(doors, origin, &coarse, &es),
            sheets: Vec::new(),
        },
        skipped,
    })
}

/// The doors that have a footprint, in a stable order (edge, along, id).
fn blueprint_doors(doors: Vec<SourceDoor>, origin: Pt, coarse: &[Pt], es: &[Edge]) -> Vec<BlueprintDoor> {
    let mut out: Vec<BlueprintDoor> = doors
        .into_iter()
        .filter_map(|d| {
            let local: Vec<Pt> = d.footprint.iter().map(|&p| translate(p, origin)).collect();
            let (edge, along_mm, width_mm) = door_on_edges(&local, coarse)?;
            Some(BlueprintDoor { id: d.id, edge, along_mm, edge_len_mm: es[edge].len, width_mm })
        })
        .collect();
    out.sort_by(|a, b| {
        (a.edge, a.along_mm, &a.id)
            .partial_cmp(&(b.edge, b.along_mm, &b.id))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// One item as a recipe against `coarse`'s edges, plus the figures `find_follows` needs.
fn place_item(s: SourceItem, origin: Pt, coarse: &[Pt], es: &[Edge]) -> (BlueprintItem, Measured) {
    let pts: Vec<Pt> = s.footprint.iter().map(|&p| translate(p, origin)).collect();
    let c = centroid(&pts);
    let f = s.facing.map_or((1.0, 0.0), unit);
    let v = (-f.1, f.0);
    let hu = pts.iter().map(|&p| dot(sub(p, c), f).abs()).fold(0.0, f64::max);
    let hv = pts.iter().map(|&p| dot(sub(p, c), v).abs()).fold(0.0, f64::max);

    // The edge it lies beside: the projection falls within the edge and the item is on
    // the room side of it (a little slack for a device sitting on the wall line).
    let beside = |i: usize| {
        let r = sub(c, es[i].a);
        let along = dot(r, es[i].t);
        (0.0..=es[i].len).contains(&along) && dot(r, es[i].n) >= -50.0
    };
    let valid: Vec<usize> = (0..es.len()).filter(|&i| beside(i)).collect();
    let off_of = |i: usize| dot(sub(c, es[i].a), es[i].n).abs();
    let corner_anchored = valid.is_empty();
    let edge = if corner_anchored {
        (0..es.len())
            .min_by(|&a, &b| segment_distance(c, &es[a]).total_cmp(&segment_distance(c, &es[b])))
            .unwrap_or(0)
    } else {
        *valid.iter().min_by(|&&a, &&b| off_of(a).total_cmp(&off_of(b))).unwrap_or(&0)
    };
    let e = es[edge];
    let r = sub(c, e.a);
    let (along, off) = (dot(r, e.t), dot(r, e.n));
    let facing_deg = cross(e.n, f).atan2(dot(e.n, f)).to_degrees();
    let depth = depth_at(coarse, edge, e.point(along, 0.0));

    // The box's half-extent along the edge and across it, exactly (a rectangle's support).
    let ht = dot(f, e.t).abs() * hu + dot(v, e.t).abs() * hv;
    let hn = dot(f, e.n).abs() * hu + dot(v, e.n).abs() * hv;
    let (gap_start, gap_end) = (along - ht, e.len - along - ht);
    let along_mode = if gap_start <= CORNER_GAP_MM && gap_start <= gap_end {
        AlongMode::FixedFromStart
    } else if gap_end <= CORNER_GAP_MM {
        AlongMode::FixedFromEnd
    } else {
        AlongMode::Percent
    };
    let off_mode = if off - hn <= WALL_GAP_MM { OffMode::Fixed } else { OffMode::Percent };
    let class = if 2.0 * hu.min(hv) < WALL_DEVICE_MM {
        ItemClass::WallDevice
    } else {
        ItemClass::Free
    };

    (
        BlueprintItem {
            id: s.id,
            family_name: s.family_name,
            type_name: s.type_name,
            category: s.category,
            class,
            excluded: false,
            placement: Placement {
                edge,
                along_mm: along,
                along_mode,
                off_mm: off,
                off_mode,
                edge_len_mm: e.len,
                depth_mm: depth,
                facing_deg,
                corner_anchored,
            },
            size_mm: [2.0 * hu, 2.0 * hv],
            follows: None,
            properties: s.properties,
            type_properties: s.type_properties,
        },
        Measured { ht, hn, corners: box_corners(c, f, hu, hv), area: 4.0 * hu * hv },
    )
}

fn segment_distance(c: Pt, e: &Edge) -> f64 {
    let t = dot(sub(c, e.a), e.t).clamp(0.0, e.len);
    dist(c, e.point(t, 0.0))
}

/// Mark each free item that sits inside another, or abuts the previous one on its edge.
/// Containment wins: a shelf in a wardrobe is the wardrobe's, not its neighbour's.
fn find_follows(items: &mut [(BlueprintItem, Measured)]) {
    let n = items.len();
    let mut follows: Vec<Option<Follow>> = vec![None; n];
    // First containment, for every item: a run is then made only of items that are not
    // inside another, or a shelf standing between two cabinets would be taken for the
    // cabinet before the next one.
    for i in 0..n {
        if items[i].0.class != ItemClass::Free {
            continue;
        }
        // The smallest free item that wholly contains this one.
        let parent = (0..n)
            .filter(|&j| {
                j != i
                    && items[j].0.class == ItemClass::Free
                    && items[j].1.area > items[i].1.area
                    && items[i].1.corners.iter().all(|&p| inside(p, &items[j].1.corners, 5.0))
            })
            .min_by(|&a, &b| items[a].1.area.total_cmp(&items[b].1.area));
        if let Some(j) = parent {
            follows[i] = Some(Follow { item: items[j].0.id.clone(), gap_mm: 0.0, inside: true });
        }
    }
    // Then runs: the previous free, not-contained item on the same edge, if it abuts.
    for i in 0..n {
        if items[i].0.class != ItemClass::Free || follows[i].is_some() {
            continue;
        }
        let previous = (0..i).rev().find(|&j| {
            items[j].0.class == ItemClass::Free
                && follows[j].as_ref().is_none_or(|f| !f.inside)
                && items[j].0.placement.edge == items[i].0.placement.edge
        });
        if let Some(j) = previous {
            let (a, b) = (&items[j], &items[i]);
            let gap = (b.0.placement.along_mm - b.1.ht) - (a.0.placement.along_mm + a.1.ht);
            let side_by_side = (b.0.placement.off_mm - a.0.placement.off_mm).abs() < a.1.hn + b.1.hn;
            if gap.abs() <= RUN_GAP_MM && side_by_side {
                follows[i] = Some(Follow { item: a.0.id.clone(), gap_mm: gap, inside: false });
            }
        }
    }
    for (item, f) in items.iter_mut().zip(follows) {
        item.0.follows = f;
    }
}

fn properties_of(map: &PropertyMap) -> Vec<Property> {
    map.iter()
        .map(|(k, v)| Property { name: k.to_string(), value: v.value.clone() })
        .collect()
}

/// A room reference from a request that may leave the model out, because the plan's room rows
/// do not name theirs. With a model it is taken as given; without one the room id is looked up
/// among the project's rooms: `None` when no room has it, an error naming the models when
/// several do, because a silent guess would act on the wrong room (a room id is unique only
/// within its model). The rule `connections::Resolver` applies to a saved link.
pub fn qualify_room(
    state: &AppState,
    project: &str,
    room_id: &str,
    model_id: Option<&str>,
) -> Result<Option<RoomRef>, ServiceError> {
    if let Some(model_id) = model_id.map(str::trim).filter(|m| !m.is_empty()) {
        return Ok(Some(RoomRef { model_id: model_id.to_string(), room_id: room_id.to_string() }));
    }
    let Some(rooms) = assemble_rooms(state, &RoomScope { project: Some(project), ..Default::default() })? else {
        return Ok(None);
    };
    let mut models: Vec<&str> =
        rooms.rooms.iter().filter(|r| r.room.id == room_id).map(|r| r.model_id.as_str()).collect();
    models.sort_unstable();
    models.dedup();
    match models.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some(RoomRef { model_id: (*only).to_string(), room_id: room_id.to_string() })),
        many => Err(ServiceError::Invalid(format!(
            "room id {room_id:?} exists in {} models ({}); name the model",
            many.len(),
            many.join(", ")
        ))),
    }
}

/// The room's name and outer loop in millimetres, in the project's plan frame, or `None` when the
/// project has no such room. Shared by capture and the resolver so both read the same outline.
pub fn room_ring(state: &AppState, project: &str, room: &RoomRef) -> Result<Option<(String, Vec<Pt>)>, ServiceError> {
    let Some(rooms) = assemble_rooms(state, &RoomScope { project: Some(project), ..Default::default() })? else {
        return Ok(None);
    };
    let Some(found) = rooms.rooms.iter().find(|r| r.model_id == room.model_id && r.room.id == room.room_id) else {
        return Ok(None);
    };
    let Some(outer) = found.room.loops.first() else {
        return Err(ServiceError::Invalid(format!("room {:?} has no outline", room.room_id)));
    };
    let ring = outer.points.iter().map(|p| (p.x * FT_TO_MM, p.y * FT_TO_MM)).collect();
    Ok(Some((found.room.name.clone(), ring)))
}

/// The doors that open onto or out of `room` (either side names it), as footprints in
/// millimetres. **Not the doors attributed to it**: a door belongs to the room it opens into
/// (`room_attribution`), but a door to the corridor is still a door of this room's wall, and
/// that is the question orientation asks. A door with no footprint is left out.
pub fn room_doors(state: &AppState, project: &str, room: &RoomRef) -> Result<Vec<SourceDoor>, ServiceError> {
    Ok(project_doors(state, project)?
        .into_iter()
        .filter(|d| d.opens_onto(room))
        .map(|d| d.door)
        .collect())
}

/// A door with the rooms it names, for a caller asking about several rooms from one read.
pub struct ProjectDoor {
    pub model_id: String,
    pub from_room: Option<String>,
    pub to_room: Option<String>,
    pub door: SourceDoor,
}

impl ProjectDoor {
    /// Either side names `room`, in the room's own model (a room id is unique only within one).
    pub fn opens_onto(&self, room: &RoomRef) -> bool {
        let names = |side: &Option<String>| side.as_deref() == Some(room.room_id.as_str());
        self.model_id == room.model_id && (names(&self.from_room) || names(&self.to_room))
    }
}

/// Every door of the project that has a footprint, in millimetres. One read, so assigning many
/// rooms costs one assembly of the doors and not one per room.
pub fn project_doors(state: &AppState, project: &str) -> Result<Vec<ProjectDoor>, ServiceError> {
    let scope = OpeningScope { project: Some(project), ..Default::default() };
    let Some(assembled) = assemble_openings::<DoorPayload>(state, OpeningKind::Doors, &scope)? else {
        return Ok(Vec::new());
    };
    Ok(assembled
        .openings
        .iter()
        .filter_map(|d| {
            let outer = d.door.loops.first()?;
            let footprint: Vec<Pt> = outer.points.iter().map(|p| (p.x * FT_TO_MM, p.y * FT_TO_MM)).collect();
            (footprint.len() >= 3).then(|| ProjectDoor {
                model_id: d.model_id.clone(),
                from_room: d.door.from_room.clone(),
                to_room: d.door.to_room.clone(),
                door: SourceDoor { id: d.door.id.clone(), footprint },
            })
        })
        .collect())
}

/// Read one room and the FF&E attributed to it, and capture them.
///
/// Returns `Ok(None)` when the project has no room by that reference. The FF&E is the
/// project's, scoped by the room it is attributed to (`owner_rooms_qualified`), so the
/// attribution policy the project runs on is the one a blueprint is captured under.
pub fn capture_room(
    state: &AppState,
    project: &str,
    room: &RoomRef,
) -> Result<Option<(Captured, String)>, ServiceError> {
    let Some((room_name, ring)) = room_ring(state, project, room)? else {
        return Ok(None);
    };

    let mut sources = Vec::new();
    if let Some(ffe) = assemble_items(state, &ItemScope { project: Some(project), ..Default::default() })? {
        let no_tier = PropertyMap::new();
        for it in ffe.ffe.iter().filter(|it| it.owner_rooms_qualified.contains(room)) {
            let type_props: &PropertyMap = it
                .type_properties_ref
                .and_then(|r| ffe.type_property_sets.get(r as usize))
                .map_or(&no_tier, |a| a);
            sources.push(SourceItem {
                id: it.item.id.clone(),
                family_name: it.item.family_name.clone(),
                type_name: it.item.type_name.clone(),
                category: it.item.category.clone(),
                footprint: it
                    .item
                    .loops
                    .first()
                    .map(|l| l.points.iter().map(|p| (p.x * FT_TO_MM, p.y * FT_TO_MM)).collect())
                    .unwrap_or_default(),
                facing: it.item.facing.as_ref().map(|f| (f.x, f.y)),
                nested: it.item.super_component_id.is_some(),
                properties: properties_of(&it.item.properties),
                type_properties: properties_of(type_props),
            });
        }
    }
    let captured = capture_with_doors(&ring, sources, room_doors(state, project, room)?)?;
    Ok(Some((captured, room_name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f64, h: f64) -> Vec<Pt> {
        vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    }

    /// A `w` x `d` item centred at `c`, facing `f`.
    fn item(id: &str, c: Pt, w: f64, d: f64, f: Pt) -> SourceItem {
        let g = unit(f);
        let v = (-g.1, g.0);
        let footprint = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)]
            .iter()
            .map(|(a, b)| (c.0 + a * w / 2.0 * g.0 + b * d / 2.0 * v.0, c.1 + a * w / 2.0 * g.1 + b * d / 2.0 * v.1))
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

    fn captured(ring: &[Pt], sources: Vec<SourceItem>) -> Captured {
        capture(ring, sources).unwrap()
    }

    fn by_id<'a>(c: &'a Captured, id: &str) -> &'a BlueprintItem {
        c.content.items.iter().find(|i| i.id == id).unwrap()
    }

    #[test]
    fn test_a_central_item_against_a_wall_is_a_percentage_along_it_and_keeps_its_gap_to_the_wall() {
        // 1000 deep, centred 500 off the wall: no gap to the wall at all.
        let c = captured(&rect(4000.0, 3000.0), vec![item("bed", (2000.0, 500.0), 1600.0, 1000.0, (0.0, 1.0))]);
        let p = &by_id(&c, "bed").placement;
        assert_eq!(p.edge, 0, "the bottom wall, the nearest the item lies beside");
        assert_eq!(p.along_mode, AlongMode::Percent);
        assert_eq!(p.off_mode, OffMode::Fixed);
    }

    #[test]
    fn test_an_item_out_in_the_room_is_a_percentage_of_its_depth() {
        let c = captured(&rect(4000.0, 3000.0), vec![item("table", (2000.0, 1500.0), 800.0, 800.0, (0.0, 1.0))]);
        let p = &by_id(&c, "table").placement;
        assert_eq!(p.off_mode, OffMode::Percent, "1100 mm from every wall");
        assert!((p.depth_mm - 3000.0).abs() < 1.0 || (p.depth_mm - 4000.0).abs() < 1.0);
    }

    #[test]
    fn test_an_item_in_a_corner_keeps_its_distance_to_it() {
        let c = captured(&rect(4000.0, 3000.0), vec![item("wardrobe", (400.0, 300.0), 800.0, 600.0, (0.0, 1.0))]);
        let p = &by_id(&c, "wardrobe").placement;
        assert_eq!(p.along_mode, AlongMode::FixedFromStart);
    }

    #[test]
    fn test_an_item_at_the_far_end_keeps_its_distance_from_the_end() {
        let c = captured(&rect(4000.0, 3000.0), vec![item("w", (3600.0, 300.0), 800.0, 600.0, (0.0, 1.0))]);
        assert_eq!(by_id(&c, "w").placement.along_mode, AlongMode::FixedFromEnd);
    }

    #[test]
    fn test_facing_is_relative_to_the_edges_inward_normal() {
        // Edge 0 runs along +x, so its normal is +y: an item facing +y is at 0, one facing +x at -90.
        let c = captured(
            &rect(4000.0, 3000.0),
            vec![
                item("a", (2000.0, 500.0), 800.0, 600.0, (0.0, 1.0)),
                item("b", (1000.0, 500.0), 800.0, 600.0, (1.0, 0.0)),
            ],
        );
        assert!(by_id(&c, "a").placement.facing_deg.abs() < 1e-6);
        assert!((by_id(&c, "b").placement.facing_deg + 90.0).abs() < 1e-6);
    }

    /// Capture loses nothing: the recipe puts every centre back where it was, on a
    /// room that is not axis aligned and has a notch.
    #[test]
    fn test_every_centre_comes_back_to_where_it_was() {
        let ring = vec![
            (0.0, 0.0),
            (5000.0, 0.0),
            (5000.0, 2000.0),
            (3000.0, 2000.0),
            (3000.0, 4000.0),
            (0.0, 4000.0),
        ];
        // Rotate the L by 30 degrees and shift it, so nothing is axis aligned.
        let (s, k) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
        let turn = |p: Pt| (p.0 * k - p.1 * s + 1.0e5, p.0 * s + p.1 * k - 2.0e4);
        let ring: Vec<Pt> = ring.into_iter().map(turn).collect();
        let want = [
            (500.0, 400.0),
            (2500.0, 300.0),
            (4500.0, 1000.0),
            (1500.0, 3500.0),
            (2000.0, 2000.0),
        ];
        let sources: Vec<SourceItem> = want
            .iter()
            .enumerate()
            .map(|(i, &c)| item(&format!("i{i}"), turn(c), 600.0, 400.0, (k, s)))
            .collect();
        let c = captured(&ring, sources);
        let coarse: Vec<Pt> = c.content.outline.coarse.iter().map(|&p| p.into()).collect();
        let origin = centroid(&simplify(&ring, FINE_TOLERANCE_MM));
        for (i, &w) in want.iter().enumerate() {
            let got = by_id(&c, &format!("i{i}")).placement.centre_in(&coarse).unwrap();
            let expected = translate(turn(w), origin);
            assert!(dist(got, expected) < 1e-6, "item {i}: {got:?} vs {expected:?}");
        }
    }

    #[test]
    fn test_nested_components_and_footprintless_items_are_counted_not_captured() {
        let mut nested = item("handle", (500.0, 500.0), 100.0, 100.0, (1.0, 0.0));
        nested.nested = true;
        let mut bare = item("marker", (900.0, 900.0), 100.0, 100.0, (1.0, 0.0));
        bare.footprint.clear();
        let c = captured(
            &rect(4000.0, 3000.0),
            vec![nested, bare, item("chair", (2000.0, 1500.0), 500.0, 500.0, (0.0, 1.0))],
        );
        assert_eq!(c.content.items.len(), 1);
        assert_eq!(c.skipped, Skipped { nested: 1, no_footprint: 1 });
    }

    #[test]
    fn test_a_socket_is_a_wall_device() {
        let c = captured(&rect(4000.0, 3000.0), vec![item("gpo", (2000.0, 5.0), 115.0, 8.0, (0.0, 1.0))]);
        assert_eq!(by_id(&c, "gpo").class, ItemClass::WallDevice);
        assert_eq!(by_id(&c, "gpo").placement.off_mode, OffMode::Fixed);
    }

    #[test]
    fn test_a_run_of_cabinets_follows_and_a_shelf_inside_a_wardrobe_is_the_wardrobes() {
        let c = captured(
            &rect(6000.0, 3000.0),
            vec![
                item("c1", (1000.0, 300.0), 600.0, 600.0, (0.0, 1.0)),
                item("c2", (1600.0, 300.0), 600.0, 600.0, (0.0, 1.0)),
                item("c3", (2200.0, 300.0), 600.0, 600.0, (0.0, 1.0)),
                item("lone", (4500.0, 300.0), 600.0, 600.0, (0.0, 1.0)),
                item("shelf", (1600.0, 300.0), 400.0, 300.0, (0.0, 1.0)),
            ],
        );
        assert!(by_id(&c, "c1").follows.is_none());
        let f2 = by_id(&c, "c2").follows.as_ref().unwrap();
        assert_eq!((f2.item.as_str(), f2.inside), ("c1", false));
        assert!(f2.gap_mm.abs() < 1.0);
        assert_eq!(by_id(&c, "c3").follows.as_ref().unwrap().item, "c2");
        assert!(by_id(&c, "lone").follows.is_none(), "1700 mm of floor is not a run");
        let shelf = by_id(&c, "shelf").follows.as_ref().unwrap();
        assert!(shelf.inside, "wholly within c2's footprint");
    }

    #[test]
    fn test_two_captures_of_one_room_are_equal() {
        let sources = || {
            vec![
                item("b", (3000.0, 400.0), 500.0, 500.0, (0.0, 1.0)),
                item("a", (1000.0, 400.0), 500.0, 500.0, (0.0, 1.0)),
            ]
        };
        assert_eq!(captured(&rect(4000.0, 3000.0), sources()), captured(&rect(4000.0, 3000.0), sources()));
    }

    #[test]
    fn test_the_blueprint_is_in_the_rooms_own_frame_not_the_projects() {
        let far: Vec<Pt> = rect(4000.0, 3000.0).into_iter().map(|p| (p.0 + 2.0e7, p.1 + 1.0e6)).collect();
        let c = captured(&far, vec![]);
        let xs: Vec<f64> = c.content.outline.fine.iter().map(|p| p.x).collect();
        assert!(xs.iter().all(|x| x.abs() <= 2000.0 + 1e-6), "centred on the origin, got {xs:?}");
    }

    #[test]
    fn test_a_room_with_fewer_than_three_corners_is_refused() {
        assert!(capture(&[(0.0, 0.0), (1000.0, 0.0)], vec![]).is_err());
    }

    #[test]
    fn test_a_door_in_the_wall_is_on_that_edge_at_that_distance_and_a_footprintless_one_is_left_out() {
        let ring = rect(6000.0, 4000.0);
        // 900 wide, in the south wall (edge 0), centred 1000 from its start.
        let door = SourceDoor {
            id: "d".to_string(),
            footprint: vec![(550.0, -50.0), (1450.0, -50.0), (1450.0, 50.0), (550.0, 50.0)],
        };
        let none = SourceDoor { id: "x".to_string(), footprint: vec![] };
        let c = capture_with_doors(&ring, vec![], vec![door, none]).unwrap().content;
        assert_eq!(c.doors.len(), 1);
        let d = &c.doors[0];
        assert_eq!(d.edge, 0);
        assert!((d.along_mm - 1000.0).abs() < 1.0, "{}", d.along_mm);
        assert!((d.width_mm - 900.0).abs() < 1.0, "{}", d.width_mm);
        assert!((d.edge_len_mm - 6000.0).abs() < 1.0);
    }
}
