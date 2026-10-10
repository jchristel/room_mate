//! Authored connections: the places a person has said rooms are joined that no
//! door says. Two things, and they are deliberately different records.
//!
//! **An open zone is a set of rooms the user declares to be one open space.**
//! Every wall two of its members share is treated as open, so the rooms connect
//! to each other as a door would connect them (`service::connectivity` derives
//! the edges at read time from the wall-sharing geometry; nothing derived is
//! stored). It is a set and not a list of pairs because what a person knows is
//! "this whole area is open", not which of 47 wall segments inside it are.
//!
//! **A vertical link joins exactly two rooms on different storeys, and a person
//! makes each one by hand.** A lift or stair is a run of floor-to-floor hops, one
//! link per hop (level 1 to 2, 2 to 3, ...), never a jump from level 1 to 8. This
//! replaced a "vertical zone" (a flat list of rooms joined across storeys), which
//! could not tell the lift rooms from the lobbies around them and so joined a
//! corridor on one level straight to the next level. Two rooms per link has no
//! such ambiguity, and nothing here guesses what a stack is.
//!
//! **Beside the project settings, as one JSON document per project**, under
//! `<projects_dir>/connections/<project>.json`, written the way settings are:
//! temp then rename, a copy of the replaced file under `.backups/`, one lock.
//! That is **provisional** — the storage type is decided after the whole
//! shortest-path plan is built (`docs/PLAN-connectivity.md`, "Where it lives") —
//! so the one thing that matters here is that this module is the only code that
//! reads or writes the file. The document carries a `schema_version` and a
//! `taken_at`, which is what makes a move to a versioned store a copy.
//!
//! **Replace-whole-list, with optimistic concurrency.** A save names the
//! `taken_at` of the document it read (`base`), and a mismatch is a conflict
//! rather than a silent overwrite: two people editing the same project is the
//! ordinary case for authored data, and the last writer winning unseen is the
//! failure that would make anyone distrust it.
//!
//! **A room is never validated against the model on save once it is a full
//! `RoomRef`**: authored data outlives the geometry it references, and a stale
//! member is a reported state at read time, never an error. A *bare* room id
//! (what the plan knows) is resolved here, and that is refused when it names no
//! room or names rooms in two models, because a silent guess would store a link
//! to the wrong room. Whether a link's two rooms really are on different storeys
//! is likewise a read-time finding, because it needs the model.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::service::room_locator::RoomRef;
use crate::service::rooms::{assemble_rooms, RoomScope};
use crate::service::ServiceError;
use crate::state::{is_path_safe_component, AppState};

pub const SCHEMA_VERSION: u32 = 1;

/// Hard limits, so a hostile or runaway save cannot make every later read
/// expensive. Far above any real project.
const MAX_ZONES: usize = 5_000;
const MAX_LINKS: usize = 20_000;
const MAX_ROOMS_PER_ZONE: usize = 5_000;
const MAX_ROUTES: usize = 5_000;
const MAX_NOTE_CHARS: usize = 2_000;
const MAX_DISCONNECTS_PER_ZONE: usize = 50_000;
const MAX_NAME_CHARS: usize = 120;

/// What one storey change costs when a link does not say, in feet of walking: a
/// flight of stairs with its landing. A link can carry its own figure, because a
/// lift hop and a stair hop do not cost the same.
pub const DEFAULT_LEVEL_COST_FT: f64 = 40.0;

/// Upper bound on a stated cost, so a typo cannot make a stair longer than the
/// building.
const MAX_LEVEL_COST_FT: f64 = 1_000.0;

/// What a zone means. One kind, kept as an enum so a document written by a
/// version that knew another fails to load LOUDLY rather than being quietly read
/// as this one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneKind {
    /// Shared walls between members are open.
    #[default]
    Open,
}

/// Two rooms of one open zone whose shared wall stays CLOSED. A zone opens every
/// wall two members share, which is wrong where members touch but do not connect:
/// three bays along a corridor each open onto the corridor, and also onto each
/// other, so a route may take the bays and not the corridor. Undirected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disconnect {
    pub a: RoomRef,
    pub b: RoomRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenZone {
    /// Stable identity: survives a rename, and is what an edit addresses.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: ZoneKind,
    pub rooms: Vec<RoomRef>,
    /// Pairs of members whose shared wall is NOT open. See [`Disconnect`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disconnects: Vec<Disconnect>,
    /// When set, members connect ONLY through this room: a wall two other members
    /// share stays closed. A shortcut for the corridor-and-bays layout, stored as a
    /// room and not as the pairs it implies, so a member added later is held to it
    /// and a zone of a hundred bays is not a list of five thousand cuts. A single
    /// [`Disconnect`] can still close a wall between the hub and a member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub: Option<RoomRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One floor-to-floor hop a person drew: from a room on one storey to a room on
/// another. Undirected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerticalLink {
    pub id: String,
    pub a: RoomRef,
    pub b: RoomRef,
    /// Walking-equivalent feet for this hop. Absent means `DEFAULT_LEVEL_COST_FT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_ft: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A position in the plan, in the frame `/connectivity` answers in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RoutePoint {
    pub x: f64,
    pub y: f64,
}

/// A route somebody saved: the REQUEST, never the answer. It names the two rooms,
/// where in them the route starts and ends, and the method, and nothing about the
/// path, which is worked out at read time like every other derived thing here. So
/// a saved route follows the doors, zones and links as they change, and one whose
/// room has left the model reports that instead of drawing a stale line.
///
/// It lives beside the zones and links because it is the same kind of thing: a
/// project-wide, shared, authored record. `colour` is part of the record, not of
/// a viewer, so everyone sees a route in the colour it was saved with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedRoute {
    pub id: String,
    pub name: String,
    pub from: RoomRef,
    pub to: RoomRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_at: Option<RoutePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_at: Option<RoutePoint>,
    /// A routing method id; absent means the server's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// The width in mm of the object this route was saved for; absent checks nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_mm: Option<f64>,
    /// Its height in mm; absent checks nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_mm: Option<f64>,
    /// `#rrggbb`.
    pub colour: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionsDocument {
    pub schema_version: u32,
    /// When this version was saved. Empty for a project nobody has authored
    /// anything for. It is the `base` the next save must name.
    #[serde(default)]
    pub taken_at: String,
    #[serde(default)]
    pub zones: Vec<OpenZone>,
    #[serde(default)]
    pub links: Vec<VerticalLink>,
    /// Absent in a file written before routes could be saved, which reads as none.
    #[serde(default)]
    pub routes: Vec<SavedRoute>,
}

impl ConnectionsDocument {
    pub fn empty() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            taken_at: String::new(),
            zones: Vec::new(),
            links: Vec::new(),
            routes: Vec::new(),
        }
    }
}

/// A room as a caller names it: the model is optional, because the plan does
/// not know it.
#[derive(Debug, Clone, Deserialize)]
pub struct RoomInput {
    pub room_id: String,
    #[serde(default)]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DisconnectInput {
    pub a: RoomInput,
    pub b: RoomInput,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ZoneInput {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: ZoneKind,
    pub rooms: Vec<RoomInput>,
    #[serde(default)]
    pub disconnects: Vec<DisconnectInput>,
    #[serde(default)]
    pub hub: Option<RoomInput>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinkInput {
    pub id: String,
    pub a: RoomInput,
    pub b: RoomInput,
    #[serde(default)]
    pub cost_ft: Option<f64>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteInput {
    pub id: String,
    pub name: String,
    pub from: RoomInput,
    pub to: RoomInput,
    #[serde(default)]
    pub from_at: Option<RoutePoint>,
    #[serde(default)]
    pub to_at: Option<RoutePoint>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub width_mm: Option<f64>,
    #[serde(default)]
    pub height_mm: Option<f64>,
    pub colour: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// The body of a save: the WHOLE document, every list.
#[derive(Debug, Clone, Deserialize)]
pub struct SaveRequest {
    /// The `taken_at` of the document the caller read; empty for "there was
    /// none".
    #[serde(default)]
    pub base: String,
    #[serde(default)]
    pub zones: Vec<ZoneInput>,
    #[serde(default)]
    pub links: Vec<LinkInput>,
    /// **Absent means "keep the saved routes", not "there are none".** The other
    /// two lists predate this one, so a caller written before routes existed sends
    /// no `routes` at all, and reading that as an empty list would have it silently
    /// delete every saved route on its next save. An explicit empty list clears.
    #[serde(default)]
    pub routes: Option<Vec<RouteInput>>,
}

#[derive(Debug)]
pub enum ConnectionsError {
    /// The state wasn't built from a settings directory (in-memory tests).
    NotFileBacked,
    Invalid(String),
    /// The document moved since the caller read it.
    Conflict(String),
    Internal(anyhow::Error),
}

impl From<ServiceError> for ConnectionsError {
    fn from(e: ServiceError) -> Self {
        match e {
            ServiceError::Invalid(msg) => ConnectionsError::Invalid(msg),
            ServiceError::Internal(e) => ConnectionsError::Internal(e),
        }
    }
}

/// Serialises every save, so the read-compare-write below cannot interleave.
static SAVE_LOCK: Mutex<()> = Mutex::new(());

fn document_path(projects_dir: &Path, project: &str) -> Result<PathBuf, ConnectionsError> {
    if !is_path_safe_component(project) {
        return Err(ConnectionsError::Invalid(format!("project id {project:?} cannot be a file name")));
    }
    Ok(projects_dir.join("connections").join(format!("{project}.json")))
}

/// The project's connections, or the empty document when nobody has authored
/// any. A file that exists and cannot be read is an error, not "empty": silently
/// serving none would make a corrupt file look like a deletion.
pub fn load(projects_dir: &Path, project: &str) -> Result<ConnectionsDocument, ConnectionsError> {
    let path = document_path(projects_dir, project)?;
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            ConnectionsError::Internal(anyhow::anyhow!("{} is not a connections document: {e}", path.display()))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConnectionsDocument::empty()),
        Err(e) => Err(ConnectionsError::Internal(e.into())),
    }
}

fn bad<T>(msg: String) -> Result<T, ConnectionsError> {
    Err(ConnectionsError::Invalid(msg))
}

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

/// `#rrggbb` and nothing else: the colour is drawn straight into an SVG stroke, so
/// accepting any CSS value would let a saved record carry a `url(...)`.
fn is_hex_colour(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Resolves the rooms a caller named into full references. A room with a model
/// is taken as given; a bare id is looked up among the project's rooms and
/// refused when it names none or several.
struct Resolver {
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

/// Replace the project's document with `request`, if it still is the one the
/// caller read.
///
/// Order is load-bearing, as in `settings_api::save_project`: everything that
/// can refuse (conflict, validation) runs before anything is written, the old
/// file is copied aside while it is still in place, and the new one is installed
/// by rename so a reader never sees half a document.
pub fn save(
    state: &AppState,
    projects_dir: &Path,
    project: &str,
    request: SaveRequest,
) -> Result<ConnectionsDocument, ConnectionsError> {
    let path = document_path(projects_dir, project)?;
    // Resolve before the lock: it reads the model and can take seconds on a big
    // project, and nothing about it needs the file.
    let (zones, links) = resolve(state, project, request.zones, request.links)?;
    validate(&zones, &links)?;
    let routes = resolve_routes(state, project, request.routes)?;
    if let Some(routes) = &routes {
        validate_routes(routes)?;
    }

    let _guard = SAVE_LOCK.lock().unwrap();
    let current = load(projects_dir, project)?;
    if current.taken_at != request.base {
        return Err(ConnectionsError::Conflict(format!(
            "the connections changed since you read them (yours is from {:?}, the server has {:?}); reload and try again",
            request.base, current.taken_at
        )));
    }

    let mut taken_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    if taken_at <= current.taken_at {
        // A clock that did not move forward must not produce an id that sorts
        // before the one it replaces: the id is the version.
        taken_at = format!("{}~", current.taken_at);
    }
    let routes = routes.unwrap_or(current.routes);
    let document = ConnectionsDocument { schema_version: SCHEMA_VERSION, taken_at, zones, links, routes };

    let dir = path.parent().expect("a connections path always has a parent");
    std::fs::create_dir_all(dir).map_err(|e| ConnectionsError::Internal(e.into()))?;
    let json = serde_json::to_vec_pretty(&document).map_err(|e| ConnectionsError::Internal(e.into()))?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(|e| ConnectionsError::Internal(e.into()))?;

    let stem = format!("connections-{project}");
    if path.exists()
        && let Err(e) = crate::backups::back_up(projects_dir, &stem, "json", &path)
    {
        std::fs::remove_file(&temp).ok();
        return Err(ConnectionsError::Internal(e));
    }
    std::fs::rename(&temp, &path).map_err(|e| ConnectionsError::Internal(e.into()))?;
    crate::backups::prune(projects_dir, &stem, "json");
    tracing::info!(
        "saved connections for project {project} ({} zones, {} links, {} routes)",
        document.zones.len(),
        document.links.len(),
        document.routes.len()
    );
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemStore;
    use std::collections::HashMap;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-connections-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok(); // a stale one from a run with the same process id would read as saved data
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn state() -> AppState {
        AppState::new(Box::new(MemStore::new()), HashMap::new(), None)
    }

    fn room(model: &str, id: &str) -> RoomInput {
        RoomInput { room_id: id.to_string(), model_id: Some(model.to_string()) }
    }

    fn zone(id: &str, rooms: Vec<RoomInput>) -> ZoneInput {
        ZoneInput {
            id: id.to_string(),
            name: format!("Zone {id}"),
            kind: ZoneKind::Open,
            rooms,
            disconnects: vec![],
            hub: None,
            note: None,
        }
    }

    fn link(id: &str, a: RoomInput, b: RoomInput) -> LinkInput {
        LinkInput { id: id.to_string(), a, b, cost_ft: None, note: None }
    }

    fn request(base: &str, zones: Vec<ZoneInput>) -> SaveRequest {
        SaveRequest { base: base.to_string(), zones, links: vec![], routes: None }
    }

    #[test]
    fn test_a_project_with_nothing_authored_reads_as_empty() {
        let dir = temp_dir("empty");
        let doc = load(&dir, "p1").unwrap();
        assert!(doc.zones.is_empty() && doc.links.is_empty());
        assert_eq!(doc.taken_at, "");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_save_then_load_round_trips_both_lists_and_stamps_a_version() {
        let dir = temp_dir("round-trip");
        let saved = save(
            &state(),
            &dir,
            "p1",
            SaveRequest {
                base: String::new(),
                zones: vec![zone("z1", vec![room("m", "a"), room("m", "b")])],
                links: vec![link("up", room("m", "b"), room("m", "e"))],
                routes: None,
            },
        )
        .unwrap();
        assert!(!saved.taken_at.is_empty());
        let read = load(&dir, "p1").unwrap();
        assert_eq!(read, saved);
        assert_eq!((read.zones.len(), read.links.len()), (1, 1));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two people editing from the same read: the second is told, and nothing of
    /// the first is lost.
    #[test]
    fn test_a_stale_base_is_a_conflict_not_an_overwrite() {
        let dir = temp_dir("conflict");
        let first =
            save(&state(), &dir, "p1", request("", vec![zone("z1", vec![room("m", "a"), room("m", "b")])])).unwrap();
        let stale = save(&state(), &dir, "p1", request("", vec![zone("z2", vec![room("m", "c"), room("m", "d")])]));
        assert!(matches!(stale, Err(ConnectionsError::Conflict(_))));
        assert_eq!(load(&dir, "p1").unwrap(), first, "the earlier save is untouched");
        let second = save(
            &state(),
            &dir,
            "p1",
            request(&first.taken_at, vec![zone("z2", vec![room("m", "c"), room("m", "d")])]),
        )
        .unwrap();
        assert!(second.taken_at > first.taken_at);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn cut(a: &str, b: &str) -> DisconnectInput {
        DisconnectInput { a: room("m", a), b: room("m", b) }
    }

    #[test]
    fn test_disconnects_and_a_hub_round_trip_and_an_old_zone_has_none() {
        let dir = temp_dir("cuts");
        let mut z = zone("z1", vec![room("m", "a"), room("m", "b"), room("m", "c")]);
        z.disconnects = vec![cut("a", "b")];
        z.hub = Some(room("m", "c"));
        let saved = save(&state(), &dir, "p1", request("", vec![z])).unwrap();
        assert_eq!(saved.zones[0].disconnects.len(), 1);
        assert_eq!(saved.zones[0].hub.as_ref().map(|h| h.room_id.as_str()), Some("c"));
        assert_eq!(load(&dir, "p1").unwrap(), saved);
        // A file from before cuts existed reads as a zone with none.
        std::fs::write(
            dir.join("connections").join("p1.json"),
            r#"{"schema_version":1,"taken_at":"t","zones":[{"id":"z","name":"Z","rooms":[{"model_id":"m","room_id":"a"},{"model_id":"m","room_id":"b"}]}],"links":[]}"#,
        )
        .unwrap();
        let old = load(&dir, "p1").unwrap();
        assert!(old.zones[0].disconnects.is_empty() && old.zones[0].hub.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_validation_refuses_a_cut_or_hub_that_is_not_the_zones_own() {
        let check = |f: &dyn Fn(&mut ZoneInput)| {
            let mut z = zone("z1", vec![room("m", "a"), room("m", "b"), room("m", "c")]);
            f(&mut z);
            let (zones, links) = resolve(&state(), "p1", vec![z], vec![]).unwrap();
            validate(&zones, &links)
        };
        assert!(check(&|z| z.disconnects = vec![cut("a", "b")]).is_ok());
        assert!(check(&|z| z.disconnects = vec![cut("a", "zz")]).is_err(), "not a member");
        assert!(check(&|z| z.disconnects = vec![cut("a", "a")]).is_err(), "from itself");
        assert!(
            check(&|z| z.disconnects = vec![cut("a", "b"), cut("b", "a")]).is_err(),
            "twice, either way round"
        );
        assert!(check(&|z| z.hub = Some(room("m", "zz"))).is_err(), "hub not a member");
        assert!(check(&|z| z.hub = Some(room("m", "a"))).is_ok());
    }

    fn route(id: &str, colour: &str) -> RouteInput {
        RouteInput {
            id: id.to_string(),
            name: format!("Route {id}"),
            from: room("m", "a"),
            to: room("m", "b"),
            from_at: Some(RoutePoint { x: 1.0, y: 2.0 }),
            to_at: None,
            method: Some(String::new()),
            width_mm: None,
            height_mm: None,
            colour: colour.to_string(),
            note: None,
        }
    }

    fn with_routes(base: &str, routes: Option<Vec<RouteInput>>) -> SaveRequest {
        SaveRequest { routes, ..request(base, vec![]) }
    }

    #[test]
    fn test_routes_round_trip_and_an_absent_list_keeps_them() {
        let dir = temp_dir("routes");
        let first = save(&state(), &dir, "p1", with_routes("", Some(vec![route("r1", "#AA00cc")]))).unwrap();
        assert_eq!(first.routes.len(), 1);
        assert_eq!(first.routes[0].colour, "#aa00cc", "stored lower-case");
        assert_eq!(first.routes[0].method, None, "a blank method is the default, not a method named \"\"");
        // A caller that predates routes sends none; that must not delete them.
        let second = save(&state(), &dir, "p1", with_routes(&first.taken_at, None)).unwrap();
        assert_eq!(second.routes, first.routes);
        // An explicit empty list is how they are cleared.
        let third = save(&state(), &dir, "p1", with_routes(&second.taken_at, Some(vec![]))).unwrap();
        assert!(third.routes.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_file_written_before_routes_existed_still_loads() {
        let dir = temp_dir("old-routes");
        std::fs::create_dir_all(dir.join("connections")).unwrap();
        std::fs::write(
            dir.join("connections").join("p1.json"),
            r#"{"schema_version":1,"taken_at":"t","zones":[],"links":[]}"#,
        )
        .unwrap();
        assert!(load(&dir, "p1").unwrap().routes.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_routes_width_round_trips_and_a_nonsense_width_is_refused() {
        let dir = temp_dir("route-width");
        let mut r = route("r1", "#aa00cc");
        r.width_mm = Some(1200.0);
        let saved = save(&state(), &dir, "p1", with_routes("", Some(vec![r]))).unwrap();
        assert_eq!(saved.routes[0].width_mm, Some(1200.0));
        assert_eq!(load(&dir, "p1").unwrap().routes[0].width_mm, Some(1200.0));
        // 0 is "no check", stored as absent rather than as a width of nothing.
        let mut zero = route("r2", "#aa00cc");
        zero.width_mm = Some(0.0);
        let routes = resolve_routes(&state(), "p1", Some(vec![zero])).unwrap().unwrap();
        assert_eq!(routes[0].width_mm, None);
        let mut tall = route("r4", "#aa00cc");
        tall.height_mm = Some(2100.0);
        let saved =
            save(&state(), &dir, "p1", with_routes(&load(&dir, "p1").unwrap().taken_at, Some(vec![tall]))).unwrap();
        assert_eq!(saved.routes[0].height_mm, Some(2100.0));
        for bad in [-5.0, f64::NAN, 200_000.0] {
            let mut r = route("r5", "#aa00cc");
            r.height_mm = Some(bad);
            let routes = resolve_routes(&state(), "p1", Some(vec![r])).unwrap().unwrap();
            assert!(validate_routes(&routes).is_err(), "height {bad}");
        }
        for bad in [-5.0, f64::NAN, 200_000.0] {
            let mut r = route("r3", "#aa00cc");
            r.width_mm = Some(bad);
            let routes = resolve_routes(&state(), "p1", Some(vec![r])).unwrap().unwrap();
            assert!(validate_routes(&routes).is_err(), "{bad}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_validation_refuses_what_could_not_be_a_route() {
        let check = |r: RouteInput| {
            let routes = resolve_routes(&state(), "p1", Some(vec![r])).unwrap().unwrap();
            validate_routes(&routes)
        };
        assert!(check(route("ok", "#00ff00")).is_ok());
        assert!(check(route("c", "red")).is_err(), "a name, not a hex colour");
        assert!(check(route("c", "#12345")).is_err());
        assert!(check(route("c", "#12345g")).is_err());
        let mut same = route("s", "#000000");
        same.to = room("m", "a");
        assert!(check(same).is_err(), "a route to itself");
        let mut nameless = route("n", "#000000");
        nameless.name = "  ".to_string();
        assert!(check(nameless).is_err());
        let mut nan = route("p", "#000000");
        nan.to_at = Some(RoutePoint { x: f64::NAN, y: 0.0 });
        assert!(check(nan).is_err());
        let twice = resolve_routes(&state(), "p1", Some(vec![route("d", "#000000"), route("d", "#111111")]))
            .unwrap()
            .unwrap();
        assert!(validate_routes(&twice).is_err(), "a duplicate id");
    }

    #[test]
    fn test_a_replaced_document_is_kept_under_backups() {
        let dir = temp_dir("backup");
        let first =
            save(&state(), &dir, "p1", request("", vec![zone("z1", vec![room("m", "a"), room("m", "b")])])).unwrap();
        assert!(!dir.join(crate::backups::BACKUP_DIR).exists(), "a first save has nothing to keep");
        save(&state(), &dir, "p1", request(&first.taken_at, vec![])).unwrap();
        let kept: Vec<_> = std::fs::read_dir(dir.join(crate::backups::BACKUP_DIR)).unwrap().collect();
        assert_eq!(kept.len(), 1);
        let name = kept[0].as_ref().unwrap().file_name().to_string_lossy().into_owned();
        assert!(name.starts_with("connections-p1.") && name.ends_with(".json"), "{name}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_validation_refuses_what_could_not_be_a_zone() {
        let ok = |z: ZoneInput| {
            let (zones, links) = resolve(&state(), "p1", vec![z], vec![]).unwrap();
            validate(&zones, &links)
        };
        assert!(ok(zone("z1", vec![room("m", "a"), room("m", "b")])).is_ok());
        assert!(ok(zone("z1", vec![room("m", "a")])).is_err(), "one room connects nothing");
        assert!(ok(zone("z1", vec![room("m", "a"), room("m", "a")])).is_err(), "a room twice");
        assert!(
            ok(zone("../x", vec![room("m", "a"), room("m", "b")])).is_err(),
            "an id that is not a name"
        );
        let mut unnamed = zone("z1", vec![room("m", "a"), room("m", "b")]);
        unnamed.name = "  ".to_string();
        assert!(ok(unnamed).is_err());
        let both = vec![
            zone("z1", vec![room("m", "a"), room("m", "b")]),
            zone("z1", vec![room("m", "c"), room("m", "d")]),
        ];
        let (zones, links) = resolve(&state(), "p1", both, vec![]).unwrap();
        assert!(validate(&zones, &links).is_err(), "a duplicate id");
    }

    #[test]
    fn test_validation_refuses_what_could_not_be_a_link() {
        let check = |l: LinkInput| {
            let (zones, links) = resolve(&state(), "p1", vec![], vec![l]).unwrap();
            validate(&zones, &links)
        };
        assert!(check(link("up", room("m", "b"), room("m", "e"))).is_ok());
        assert!(check(link("up", room("m", "b"), room("m", "b"))).is_err(), "a room to itself");
        assert!(check(link("../up", room("m", "b"), room("m", "e"))).is_err(), "an id that is not a name");
        for cost in [0.0, -5.0, f64::NAN, 5_000.0] {
            let mut l = link("up", room("m", "b"), room("m", "e"));
            l.cost_ft = Some(cost);
            assert!(check(l).is_err(), "cost {cost}");
        }
        let mut priced = link("up", room("m", "b"), room("m", "e"));
        priced.cost_ft = Some(25.0);
        assert!(check(priced).is_ok());

        let twice = vec![
            link("up", room("m", "a"), room("m", "b")),
            link("up", room("m", "c"), room("m", "d")),
        ];
        let (zones, links) = resolve(&state(), "p1", vec![], twice).unwrap();
        assert!(validate(&zones, &links).is_err(), "a duplicate link id");
    }

    /// A bare room id the project has never heard of is refused rather than
    /// stored as a link to nothing, in a zone and in a link.
    #[test]
    fn test_a_bare_id_naming_no_room_is_refused() {
        let bare = || RoomInput { room_id: "ghost".to_string(), model_id: None };
        let err = resolve(&state(), "p1", vec![zone("z1", vec![bare(), room("m", "b")])], vec![]).unwrap_err();
        assert!(matches!(err, ConnectionsError::Invalid(m) if m.contains("ghost")));
        let err = resolve(&state(), "p1", vec![], vec![link("up", bare(), room("m", "b"))]).unwrap_err();
        assert!(matches!(err, ConnectionsError::Invalid(m) if m.contains("ghost")));
    }

    /// A document written when a zone could be `vertical` must fail to load
    /// loudly, not be read as an open zone: that would silently turn a stack into
    /// an open area.
    #[test]
    fn test_an_old_vertical_zone_fails_loudly_instead_of_becoming_an_open_one() {
        let dir = temp_dir("old-vertical");
        std::fs::create_dir_all(dir.join("connections")).unwrap();
        std::fs::write(
            dir.join("connections").join("p1.json"),
            r#"{"schema_version":1,"taken_at":"t","zones":[{"id":"s","name":"S","kind":"vertical","rooms":[]}]}"#,
        )
        .unwrap();
        assert!(matches!(load(&dir, "p1"), Err(ConnectionsError::Internal(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A document with no `links` key, as every document saved before links
    /// existed, still loads.
    #[test]
    fn test_a_document_without_links_still_loads() {
        let dir = temp_dir("no-links");
        std::fs::create_dir_all(dir.join("connections")).unwrap();
        std::fs::write(
            dir.join("connections").join("p1.json"),
            r#"{"schema_version":1,"taken_at":"t","zones":[]}"#,
        )
        .unwrap();
        assert!(load(&dir, "p1").unwrap().links.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_path_unsafe_project_id_is_refused() {
        let dir = temp_dir("unsafe");
        assert!(matches!(load(&dir, "../etc"), Err(ConnectionsError::Invalid(_))));
        std::fs::remove_dir_all(&dir).ok();
    }
}
