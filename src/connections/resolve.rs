//! A saved document's rooms read against the project's own rooms (`Resolver`): each zone, route, reach and passage resolved to the room
//! it names now, with a reported reason where it cannot be. Nothing resolved is stored.

use super::*;

/// Resolves the rooms a caller named into full references. A room with a model
/// is taken as given; a bare id is looked up among the project's rooms and
/// refused when it names none or several.
pub(super) struct Resolver {
    models_of: std::collections::BTreeMap<String, Vec<String>>,
}

impl Resolver {
    /// Reads the project's rooms only when a bare id needs looking up.
    fn for_project(state: &AppState, project: &str, needs_lookup: bool) -> Result<Self, ConnectionsError> {
        let mut models_of: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        if needs_lookup
            && let Some(rooms) = assemble_rooms(state, &RoomScope { project: Some(project), ..Default::default() })?
        {
            for r in &rooms.rooms {
                models_of.entry(r.room.id.clone()).or_default().push(r.model_id.clone());
            }
        }
        Ok(Self { models_of })
    }

    fn room(&self, r: RoomInput) -> Result<RoomRef, ConnectionsError> {
        match r.model_id.filter(|m| !m.trim().is_empty()) {
            Some(model_id) => Ok(RoomRef { model_id, room_id: r.room_id }),
            None => match self.models_of.get(&r.room_id).map(Vec::as_slice) {
                Some([only]) => Ok(RoomRef { model_id: only.clone(), room_id: r.room_id }),
                Some(many) if many.len() > 1 => bad(format!(
                    "room id {:?} exists in {} models ({}); name the model",
                    r.room_id,
                    many.len(),
                    many.join(", ")
                )),
                _ => bad(format!("room {:?} is not among the project's rooms", r.room_id)),
            },
        }
    }
}

/// Turn the callers' rooms into full references, for both lists at once so the
/// project's rooms are read once.
pub fn resolve(
    state: &AppState,
    project: &str,
    zones: Vec<ZoneInput>,
    links: Vec<LinkInput>,
) -> Result<(Vec<OpenZone>, Vec<VerticalLink>), ConnectionsError> {
    let unnamed = |r: &RoomInput| r.model_id.is_none();
    let needs_lookup = zones.iter().any(|z| {
        z.rooms.iter().any(unnamed)
            || z.disconnects.iter().any(|d| unnamed(&d.a) || unnamed(&d.b))
            || z.hub.as_ref().is_some_and(unnamed)
    }) || links.iter().any(|l| l.a.model_id.is_none() || l.b.model_id.is_none());
    let resolver = Resolver::for_project(state, project, needs_lookup)?;

    let zones = zones
        .into_iter()
        .map(|z| {
            let rooms = z.rooms.into_iter().map(|r| resolver.room(r)).collect::<Result<Vec<_>, _>>()?;
            let disconnects = z
                .disconnects
                .into_iter()
                .map(|d| Ok(Disconnect { a: resolver.room(d.a)?, b: resolver.room(d.b)? }))
                .collect::<Result<Vec<_>, ConnectionsError>>()?;
            let hub = z.hub.map(|h| resolver.room(h)).transpose()?;
            Ok(OpenZone {
                id: z.id,
                name: z.name.trim().to_string(),
                kind: z.kind,
                rooms,
                disconnects,
                hub,
                note: z.note,
            })
        })
        .collect::<Result<Vec<_>, ConnectionsError>>()?;
    let links = links
        .into_iter()
        .map(|l| {
            Ok(VerticalLink {
                id: l.id,
                a: resolver.room(l.a)?,
                b: resolver.room(l.b)?,
                cost_ft: l.cost_ft,
                note: l.note,
            })
        })
        .collect::<Result<Vec<_>, ConnectionsError>>()?;
    Ok((zones, links))
}

/// Turns a save's routes into full records. `None` is the caller's "keep what is
/// saved" and stays `None`, so the caller can tell it from an empty list.
pub fn resolve_routes(
    state: &AppState,
    project: &str,
    routes: Option<Vec<RouteInput>>,
) -> Result<Option<Vec<SavedRoute>>, ConnectionsError> {
    let Some(routes) = routes else { return Ok(None) };
    let unnamed = |r: &RoomInput| r.model_id.as_deref().is_none_or(|m| m.trim().is_empty());
    let needs_lookup = routes.iter().any(|r| unnamed(&r.from) || unnamed(&r.to));
    let resolver = Resolver::for_project(state, project, needs_lookup)?;
    routes
        .into_iter()
        .map(|r| {
            Ok(SavedRoute {
                id: r.id,
                name: r.name.trim().to_string(),
                from: resolver.room(r.from)?,
                to: resolver.room(r.to)?,
                from_at: r.from_at,
                to_at: r.to_at,
                method: r.method.filter(|m| !m.trim().is_empty()),
                width_mm: r.width_mm.filter(|w| *w != 0.0),
                height_mm: r.height_mm.filter(|h| *h != 0.0),
                colour: r.colour.to_ascii_lowercase(),
                note: r.note,
            })
        })
        .collect::<Result<Vec<_>, ConnectionsError>>()
        .map(Some)
}

/// Turns a save's reaches into full records. `None` is "keep what is saved" and stays
/// `None`, so the caller can tell it from an empty list.
pub fn resolve_reaches(
    state: &AppState,
    project: &str,
    reaches: Option<Vec<ReachInput>>,
) -> Result<Option<Vec<SavedReach>>, ConnectionsError> {
    let Some(reaches) = reaches else { return Ok(None) };
    let unnamed = |r: &RoomInput| r.model_id.as_deref().is_none_or(|m| m.trim().is_empty());
    let needs_lookup = reaches.iter().any(|r| r.sources.iter().any(|s| unnamed(&s.room)));
    let resolver = Resolver::for_project(state, project, needs_lookup)?;
    reaches
        .into_iter()
        .map(|r| {
            Ok(SavedReach {
                id: r.id,
                name: r.name.trim().to_string(),
                sources: r
                    .sources
                    .into_iter()
                    .map(|s| Ok(ReachStart { room: resolver.room(s.room)?, at: s.at }))
                    .collect::<Result<Vec<_>, ConnectionsError>>()?,
                limit_mm: r.limit_mm,
                spacing_mm: r.spacing_mm,
                method: r.method.filter(|m| !m.trim().is_empty()),
                ignore_cuts: r.ignore_cuts,
                note: r.note,
            })
        })
        .collect::<Result<Vec<_>, ConnectionsError>>()
        .map(Some)
}

/// Turns a save's passages into full records. `None` is "keep what is saved" and
/// stays `None`. A zero width or height is the form's blank and reads as unknown.
pub fn resolve_passages(
    state: &AppState,
    project: &str,
    passages: Option<Vec<PassageInput>>,
) -> Result<Option<Vec<Passage>>, ConnectionsError> {
    let Some(passages) = passages else { return Ok(None) };
    let unnamed = |r: &RoomInput| r.model_id.as_deref().is_none_or(|m| m.trim().is_empty());
    let needs_lookup = passages.iter().any(|p| unnamed(&p.a) || unnamed(&p.b));
    let resolver = Resolver::for_project(state, project, needs_lookup)?;
    passages
        .into_iter()
        .map(|p| {
            Ok(Passage {
                id: p.id,
                a: resolver.room(p.a)?,
                b: resolver.room(p.b)?,
                from: p.from,
                to: p.to,
                width_mm: p.width_mm.filter(|w| *w != 0.0),
                height_mm: p.height_mm.filter(|h| *h != 0.0),
                note: p.note,
            })
        })
        .collect::<Result<Vec<_>, ConnectionsError>>()
        .map(Some)
}
