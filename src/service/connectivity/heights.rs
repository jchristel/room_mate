//! How tall a room and a door are, since a height limits what can pass: the clear height from the ceilings over a room (`clear_heights`),
//! a door's own height and a plausible-height check. Absent is unknown, never zero.

use super::*;

/// A height in millimetres is never below this; a smaller number is another unit (metres,
/// feet) and is NOT guessed at, so the room or door goes unchecked and is counted.
pub(super) const MIN_PLAUSIBLE_HEIGHT_MM: f64 = 100.0;

/// The height asked for, from the optional number a caller sends. 0 is "no check".
pub fn height_param(height_mm: Option<f64>) -> Result<Option<f64>, ServiceError> {
    match height_mm {
        None => Ok(None),
        Some(h) if h.is_finite() && (0.0..=50_000.0).contains(&h) => Ok(Some(h).filter(|h| *h > 0.0)),
        Some(h) => Err(ServiceError::Invalid(format!("height_mm must be between 0 and 50000 mm, not {h}"))),
    }
}

/// A property name with case and runs of spaces ignored, for matching.
pub(super) fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A numeric property in millimetres, found by normalised name. `None` when absent,
/// not a number, or too small to be millimetres.
pub(super) fn property_mm(map: &crate::contract::PropertyMap, names: &[&str]) -> Option<f64> {
    map.iter()
        .find(|(k, _)| names.contains(&norm(k).as_str()))
        .and_then(|(_, v)| v.value.trim().parse::<f64>().ok())
        .filter(|h| h.is_finite() && *h >= MIN_PLAUSIBLE_HEIGHT_MM)
}

/// The height in a door's type name, for the convention that puts the size in it:
/// "970 x 2040 TD01", "DWWH-003 820W x 2100H". The first `A x B` wins, so a later size
/// inside the name (a vision panel, "200Wx800H") is not taken for the door. An
/// ESTIMATE, from a naming habit and not from the model's geometry; `None` for a name
/// with no size, and for a number no door could be.
pub(super) fn height_from_type_name(name: &str) -> Option<f64> {
    let cs: Vec<char> = name.chars().collect();
    for (p, &c) in cs.iter().enumerate() {
        if !matches!(c, 'x' | 'X' | '×') {
            continue;
        }
        let left = cs[..p].iter().rev().find(|ch| !ch.is_whitespace() && !matches!(ch, 'W' | 'w'));
        if !left.is_some_and(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let digits: String = cs[p + 1..]
            .iter()
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(h) = digits.parse::<f64>() {
            return (300.0..=10_000.0).contains(&h).then_some(h);
        }
    }
    None
}

/// A ceiling covering less than this fraction of a room is not what the room's height is.
/// The attribution lists every real overlap, including a strip of a neighbour's ceiling
/// reaching under a shared wall, and measured on a real project the answer hardly moves between 0 and
/// 0.5 (most rooms agree with the room property throughout), so the middle of the range, stated.
pub(super) const MIN_CEILING_COVERAGE_OF_ROOM: f64 = 0.25;

/// Each room's height from the CEILINGS over it, in mm: the lowest ceiling that covers at
/// least [`MIN_CEILING_COVERAGE_OF_ROOM`] of the room. A ceiling whose offset is not at
/// least a plausible millimetre height (a roof, a ceiling below its level) is not a
/// ceiling over a room and is ignored. Empty when no ceilings were ever pushed.
pub(super) fn ceiling_heights(
    state: &AppState,
    project: &str,
    milestone: Option<&str>,
) -> Result<BTreeMap<RoomRef, f64>, ServiceError> {
    use crate::service::surfaces::{assemble_surfaces, SurfaceScope};
    let scope = SurfaceScope { project: Some(project), milestone, storeys: None };
    let Some(assembled) = assemble_surfaces::<crate::contract::CeilingPayload>(state, &scope)? else {
        return Ok(BTreeMap::new());
    };
    let mut heights: BTreeMap<RoomRef, f64> = BTreeMap::new();
    for ceiling in &assembled.surfaces {
        let Some(mm) = ceiling.surface.height_offset.map(|ft| ft * MM_PER_FT) else {
            continue;
        };
        if !mm.is_finite() || mm < MIN_PLAUSIBLE_HEIGHT_MM {
            continue;
        }
        for r in ceiling.rooms.iter().filter(|r| r.fraction_of_room >= MIN_CEILING_COVERAGE_OF_ROOM) {
            let room = RoomRef { model_id: r.model_id.clone(), room_id: r.room_id.clone() };
            heights.entry(room).and_modify(|h| *h = h.min(mm)).or_insert(mm);
        }
    }
    Ok(heights)
}

/// Where a room's clear height came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HeightFrom {
    Ceiling,
    RoomProperty,
}

/// The clear height, mm, of each of `wanted` that has one, by the rule the route size checks use: the lowest ceiling
/// covering enough of the room ([`ceiling_heights`], when the project reads heights from ceilings), else the room's
/// height property (`[routing] room_height_property`). Shared so a room layout sheet set (`rls_set`) cuts its
/// elevations to the height a route through the room is checked against, never a second answer.
pub(in crate::service) fn clear_heights(
    state: &AppState,
    project: &str,
    rooms: &crate::service::rooms::RoomsResult,
    wanted: &[RoomRef],
) -> Result<BTreeMap<RoomRef, (f64, HeightFrom)>, ServiceError> {
    let policy = state.settings().settings_for(project).map(|b| b.routing.clone()).unwrap_or_default();
    let from_ceilings = if policy.room_height_from_ceilings {
        ceiling_heights(state, project, None)?
    } else {
        BTreeMap::new()
    };
    let key = norm(policy.room_height_property());
    let mut out = BTreeMap::new();
    for room in wanted {
        if let Some(&h) = from_ceilings.get(room) {
            out.insert(room.clone(), (h, HeightFrom::Ceiling));
            continue;
        }
        let found = rooms.rooms.iter().find(|r| r.model_id == room.model_id && r.room.id == room.room_id);
        if let Some(h) = found.and_then(|r| property_mm(&r.room.properties, &[key.as_str()])) {
            out.insert(room.clone(), (h, HeightFrom::RoomProperty));
        }
    }
    Ok(out)
}

/// A door's estimated height from the project's sources, in the order they name: its
/// own properties (instance, then type), then -- when the policy allows -- its type name.
pub(super) fn door_height_mm(
    door: &crate::contract::Opening,
    properties: &[String],
    from_type_name: bool,
) -> Option<f64> {
    let names: Vec<&str> = properties.iter().map(String::as_str).collect();
    property_mm(&door.properties, &names)
        .or_else(|| property_mm(&door.type_properties, &names))
        .or_else(|| from_type_name.then(|| height_from_type_name(&door.type_name)).flatten())
}
