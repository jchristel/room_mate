//! What a saved connections document may hold: zones and links, routes, reaches and passages each checked before anything is written
//! (`validate*`), so a person's save is refused with the reason rather than stored half-valid.

use super::*;

/// The one rule set for what a saved document may hold. Pure, so it is testable
/// without a file.
pub fn validate(zones: &[OpenZone], links: &[VerticalLink]) -> Result<(), ConnectionsError> {
    if zones.len() > MAX_ZONES {
        return bad(format!("{} zones is more than the {MAX_ZONES} allowed", zones.len()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for zone in zones {
        if !is_path_safe_component(&zone.id) {
            return bad(format!("zone id {:?} must be a short name that is safe as a file name", zone.id));
        }
        if !seen.insert(zone.id.as_str()) {
            return bad(format!("zone id {:?} appears twice", zone.id));
        }
        if zone.name.trim().is_empty() {
            return bad(format!("zone {:?} needs a name", zone.id));
        }
        if zone.rooms.len() < 2 {
            return bad(format!("zone {:?} needs at least two rooms: one room alone connects nothing", zone.name));
        }
        if zone.rooms.len() > MAX_ROOMS_PER_ZONE {
            return bad(format!(
                "zone {:?} has {} rooms; the limit is {MAX_ROOMS_PER_ZONE}",
                zone.name,
                zone.rooms.len()
            ));
        }
        let mut members = std::collections::BTreeSet::new();
        for r in &zone.rooms {
            if r.model_id.trim().is_empty() || r.room_id.trim().is_empty() {
                return bad(format!("zone {:?} names a room with no model or no id", zone.name));
            }
            if !members.insert(r) {
                return bad(format!("zone {:?} lists room {} of model {} twice", zone.name, r.room_id, r.model_id));
            }
        }
        if zone.disconnects.len() > MAX_DISCONNECTS_PER_ZONE {
            return bad(format!(
                "zone {:?} has {} disconnects; the limit is {MAX_DISCONNECTS_PER_ZONE}",
                zone.name,
                zone.disconnects.len()
            ));
        }
        let mut cut = std::collections::BTreeSet::new();
        for d in &zone.disconnects {
            // Held to the zone's own rooms: a cut names a wall the zone would
            // otherwise open, so one naming a room outside it cuts nothing, and the
            // editor drops a cut when its room leaves.
            if !members.contains(&d.a) || !members.contains(&d.b) {
                return bad(format!("zone {:?} disconnects a room that is not one of its members", zone.name));
            }
            if d.a == d.b {
                return bad(format!("zone {:?} disconnects room {} from itself", zone.name, d.a.room_id));
            }
            let pair = if d.a <= d.b { (&d.a, &d.b) } else { (&d.b, &d.a) };
            if !cut.insert(pair) {
                return bad(format!(
                    "zone {:?} disconnects rooms {} and {} twice",
                    zone.name, d.a.room_id, d.b.room_id
                ));
            }
        }
        if zone.hub.as_ref().is_some_and(|h| !members.contains(h)) {
            return bad(format!("the hub of zone {:?} is not one of its members", zone.name));
        }
        if zone.note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
            return bad(format!("the note on zone {:?} is longer than {MAX_NOTE_CHARS} characters", zone.name));
        }
    }

    if links.len() > MAX_LINKS {
        return bad(format!("{} links is more than the {MAX_LINKS} allowed", links.len()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for link in links {
        if !is_path_safe_component(&link.id) {
            return bad(format!("link id {:?} must be a short name that is safe as a file name", link.id));
        }
        if !seen.insert(link.id.as_str()) {
            return bad(format!("link id {:?} appears twice", link.id));
        }
        for r in [&link.a, &link.b] {
            if r.model_id.trim().is_empty() || r.room_id.trim().is_empty() {
                return bad(format!("link {:?} names a room with no model or no id", link.id));
            }
        }
        if link.a == link.b {
            return bad(format!("link {:?} joins a room to itself", link.id));
        }
        if let Some(cost) = link.cost_ft
            && (!cost.is_finite() || cost <= 0.0 || cost > MAX_LEVEL_COST_FT)
        {
            return bad(format!(
                "link {:?} cost {cost} must be above 0 and at most {MAX_LEVEL_COST_FT} ft",
                link.id
            ));
        }
        if link.note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
            return bad(format!("the note on link {:?} is longer than {MAX_NOTE_CHARS} characters", link.id));
        }
    }
    Ok(())
}

/// The rules for saved routes, apart from `validate` so the two older lists keep
/// their signature.
pub fn validate_routes(routes: &[SavedRoute]) -> Result<(), ConnectionsError> {
    if routes.len() > MAX_ROUTES {
        return bad(format!("{} routes is more than the {MAX_ROUTES} allowed", routes.len()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for route in routes {
        if !is_path_safe_component(&route.id) {
            return bad(format!("route id {:?} must be a short name that is safe as a file name", route.id));
        }
        if !seen.insert(route.id.as_str()) {
            return bad(format!("route id {:?} appears twice", route.id));
        }
        let name = route.name.trim();
        if name.is_empty() {
            return bad(format!("route {:?} needs a name", route.id));
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return bad(format!("the name of route {:?} is longer than {MAX_NAME_CHARS} characters", route.id));
        }
        for r in [&route.from, &route.to] {
            if r.model_id.trim().is_empty() || r.room_id.trim().is_empty() {
                return bad(format!("route {name:?} names a room with no model or no id"));
            }
        }
        if route.from == route.to {
            return bad(format!("route {name:?} starts and ends in the same room"));
        }
        if !is_hex_colour(&route.colour) {
            return bad(format!("route {name:?} colour {:?} must look like #1a2b3c", route.colour));
        }
        if route.width_mm.is_some_and(|w| !w.is_finite() || w <= 0.0 || w > 100_000.0) {
            return bad(format!("route {name:?} width must be above 0 and at most 100000 mm"));
        }
        if route.height_mm.is_some_and(|h| !h.is_finite() || h <= 0.0 || h > 50_000.0) {
            return bad(format!("route {name:?} height must be above 0 and at most 50000 mm"));
        }
        for p in [route.from_at, route.to_at].into_iter().flatten() {
            if !p.x.is_finite() || !p.y.is_finite() {
                return bad(format!("route {name:?} has a start or end point that is not a number"));
            }
        }
        if route.note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
            return bad(format!("the note on route {name:?} is longer than {MAX_NOTE_CHARS} characters"));
        }
    }
    Ok(())
}

/// The rules for saved reaches, apart from `validate` for the reason `validate_routes`
/// is. The walk and the grid go through `reach::params` and the method through
/// `Method::parse`, so a saved reach is exactly a request the read accepts.
pub fn validate_reaches(reaches: &[SavedReach]) -> Result<(), ConnectionsError> {
    use crate::service::reach;
    if reaches.len() > MAX_REACHES {
        return bad(format!("{} reaches is more than the {MAX_REACHES} allowed", reaches.len()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for reach in reaches {
        if !is_path_safe_component(&reach.id) {
            return bad(format!("reach id {:?} must be a short name that is safe as a file name", reach.id));
        }
        if !seen.insert(reach.id.as_str()) {
            return bad(format!("reach id {:?} appears twice", reach.id));
        }
        let name = reach.name.trim();
        if name.is_empty() {
            return bad(format!("reach {:?} needs a name", reach.id));
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return bad(format!("the name of reach {:?} is longer than {MAX_NAME_CHARS} characters", reach.id));
        }
        if reach.sources.is_empty() {
            return bad(format!("reach {name:?} needs at least one start room"));
        }
        if reach.sources.len() > reach::MAX_SOURCES {
            return bad(format!(
                "reach {name:?} has {} start rooms; the limit is {}",
                reach.sources.len(),
                reach::MAX_SOURCES
            ));
        }
        let mut rooms = std::collections::BTreeSet::new();
        for s in &reach.sources {
            if s.room.model_id.trim().is_empty() || s.room.room_id.trim().is_empty() {
                return bad(format!("reach {name:?} names a room with no model or no id"));
            }
            if !rooms.insert(&s.room) {
                return bad(format!("reach {name:?} lists room {} of model {} twice", s.room.room_id, s.room.model_id));
            }
            if s.at.is_some_and(|p| !p.x.is_finite() || !p.y.is_finite()) {
                return bad(format!("reach {name:?} has a start point that is not a number"));
            }
        }
        reach::params(Some(reach.limit_mm), Some(reach.spacing_mm)).map_err(|e| match e {
            ServiceError::Invalid(m) => ConnectionsError::Invalid(format!("reach {name:?}: {m}")),
            ServiceError::Internal(e) => ConnectionsError::Internal(e),
        })?;
        if let Some(m) = &reach.method {
            reach::Method::parse(Some(m))?;
        }
        if reach.note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
            return bad(format!("the note on reach {name:?} is longer than {MAX_NOTE_CHARS} characters"));
        }
    }
    Ok(())
}

/// The rules for passages, apart from `validate` for the reason `validate_routes` is.
/// Whether the two rooms share a storey, and whether each point lies in its room, are
/// read-time findings: they need the model.
pub fn validate_passages(passages: &[Passage]) -> Result<(), ConnectionsError> {
    if passages.len() > MAX_PASSAGES {
        return bad(format!("{} passages is more than the {MAX_PASSAGES} allowed", passages.len()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for p in passages {
        if !is_path_safe_component(&p.id) {
            return bad(format!("passage id {:?} must be a short name that is safe as a file name", p.id));
        }
        if !seen.insert(p.id.as_str()) {
            return bad(format!("passage id {:?} appears twice", p.id));
        }
        for r in [&p.a, &p.b] {
            if r.model_id.trim().is_empty() || r.room_id.trim().is_empty() {
                return bad(format!("passage {:?} names a room with no model or no id", p.id));
            }
        }
        if p.a == p.b {
            return bad(format!("passage {:?} joins a room to itself", p.id));
        }
        for pt in [p.from, p.to] {
            if !pt.x.is_finite() || !pt.y.is_finite() {
                return bad(format!("passage {:?} has an end that is not a number", p.id));
            }
        }
        if p.from == p.to {
            return bad(format!("passage {:?} starts and ends at the same point", p.id));
        }
        if p.width_mm.is_some_and(|w| !w.is_finite() || w <= 0.0 || w > 100_000.0) {
            return bad(format!("passage {:?} width must be above 0 and at most 100000 mm", p.id));
        }
        if p.height_mm.is_some_and(|h| !h.is_finite() || h <= 0.0 || h > 50_000.0) {
            return bad(format!("passage {:?} height must be above 0 and at most 50000 mm", p.id));
        }
        if p.note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
            return bad(format!("the note on passage {:?} is longer than {MAX_NOTE_CHARS} characters", p.id));
        }
    }
    Ok(())
}

/// `#rrggbb` and nothing else: the colour is drawn straight into an SVG stroke, so
/// accepting any CSS value would let a saved record carry a `url(...)`.
pub(super) fn is_hex_colour(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}
