//! The rules a project chooses: the measurement standard, how an opening or an item finds its room (`RoomResolution`, `RoomAttribution`),
//! and the routing, opening, FF&E and area policies. Each default is the one a project that says nothing runs on.
//!
//! Kept whole: the per-project policies as flat data with documented defaults and no behaviour. The crate also
//! generates the settings page's TypeScript types, so a split would add re-exports for no gain; it already moved out of
//! `settings/mod.rs` as the policy half of that file.

use super::*;

/// A recognised area-measurement standard. Closed enum on purpose: the whole
/// point of declaring the standard is that a reader knows what the number
/// means, and a free-text field that accepts `"IMPS3"` defeats that silently.
/// An unknown value therefore fails the boot at TOML parse time, with serde
/// naming the accepted spellings — the "loud startup over silent no-op" rule.
///
/// The server **computes nothing** from this: it is carried and echoed on
/// `/areas` so an area figure never travels without its definition, which is
/// precisely what measurement standards exist to prevent. Adding a standard is
/// one variant here; the list is short because these are the ones a UK/EU
/// healthcare job actually cites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub enum MeasurementStandard {
    /// IPMS 1 — the whole building envelope, external walls included.
    #[serde(rename = "IPMS1")]
    Ipms1,
    /// IPMS 2 — measured to the internal dominant face, by component.
    #[serde(rename = "IPMS2")]
    Ipms2,
    /// IPMS 3 — exclusive occupier area, to the internal dominant face.
    #[serde(rename = "IPMS3")]
    Ipms3,
    /// DIN 277 — the German gross/net floor-area standard.
    #[serde(rename = "DIN277")]
    Din277,
    /// SIA 416 — the Swiss equivalent.
    #[serde(rename = "SIA416")]
    Sia416,
    /// BOMA — the North American office standard.
    #[serde(rename = "BOMA")]
    Boma,
    /// RICS *Code of Measuring Practice* (GIA/NIA family).
    #[serde(rename = "RICS")]
    Rics,
}

/// Area-measurement **policy**: the two facts about area aggregation that Revit
/// does not know and should not be asked.
///
/// The split against `contract::RoomBoundary` is the load-bearing part. Which
/// boundary regime a model was drawn to is a *model fact* Revit already holds,
/// so it rides the upload envelope. Which measurement standard applies is
/// **contractual**, and the width above which a gap stops being a wall and
/// becomes a void is a **project judgement** — neither is discoverable from the
/// model, so both live here. What the resulting number may be *called* is
/// STRATEGY-AREA-CALCULATION.md, "Relationship to measurement standards".
///
/// `max_wall_thickness` is deliberately **one quantity with three consumers**:
/// `service::areas` sizes its wall zone by it, `service::adjacency` uses it as
/// the default gap tolerance, and `service::room_locator` sizes the step it
/// probes across a wall by it. The first two were previously separate constants
/// (`areas::MAX_WALL_FT` and `adjacency::WALL_MAX_FT`) holding the same physical
/// number in two modules — a live drift risk this type removes, and the reason
/// the third consumer reached for this rather than adding a fourth constant.
/// One project's **door** policy: how doors are matched and compared across
/// milestones.
///
/// Its own section rather than reusing the top-level `comparison_key` /
/// `comparison_properties`, because those name *room* properties. A door's
/// vocabulary is a different one that merely overlaps in spelling — `Mark` on a
/// door and `Mark` on a room are different properties that happen to share a
/// name — so one shared setting would silently mean two things. That is the
/// "which canonical property names exist at all" item on
/// `docs/STRATEGY-ENTITIES.md`'s "what does not generalize" list. Cited by name
/// rather than as a relative link: ts-rs copies this comment into the generated
/// TypeScript, and no single relative path is right from both places.
///
/// The settings that depend on *which* of a door's two rooms owns it —
/// `room_attribution` and `room_reference_property` — waited for that question
/// to be answered rather than shipping with a default that would have settled
/// it by accident. Both now live on `OpeningPolicy`.
/// How far the server may go to work out which rooms an element is between,
/// when the model does not say.
///
/// **Off by default, and that is a deliberate choice rather than caution.**
/// Turning it on changes `owner_rooms` for elements the model left unresolved —
/// homeless becomes attributed — so it must be something a project opts into,
/// not something an upgrade does on its behalf. `room_attribution` has a default
/// because the chain only ever uses what the model already states; this fills in
/// what the model does not.
///
/// **A derived answer never overrides an authored one**, at any setting. A
/// door's `to_room` is the modeller's assignment — what the door *serves*, which
/// is not always what it opens into — and geometry replacing it would be exactly
/// the reconciliation `CLAUDE.md` forbids. What the geometry does is fill an
/// absent side, and disagree audibly with a present one
/// (`OpeningReport::room_geometry_mismatches`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub enum RoomResolution {
    /// Never derive. Absent references stay absent, and the geometric drift
    /// check does not run.
    #[default]
    Off,
    /// Resolve against the element's **own model's** rooms only. The setting for
    /// a model that holds its own rooms and simply failed to populate a
    /// reference.
    SameModel,
    /// Resolve against **every model in the project**, placing each one through
    /// its `model_to_shared` transform first. The setting for a split model —
    /// doors or windows in one file, rooms in another.
    ///
    /// **This is the first thing in the server to depend on `model_to_shared`
    /// being numerically correct.** The transform has been carried on the
    /// envelope since before any consumer existed, and nothing has ever checked
    /// it against a real survey: a model whose shared coordinates were never set
    /// up emits an identity transform, and two such models will be silently
    /// stacked on top of each other. Verify a project's placement before
    /// trusting this mode, and prefer `same_model` where it suffices.
    Project,
}

/// Which of a door's two room references attributes it to a room, for area
/// rollups, hierarchy scoping and door schedules. The attribution rule itself
/// is in `CLAUDE.md`.
///
/// **The default is a precedence chain, not a single pick**, and that is the
/// decision rather than a convenience. Each single pick leaves external doors
/// either mis-attributed or unattributed; the chain attributes every door that
/// has any room at all and leaves the rest *named* rather than silently absent.
///
/// **Trust it exactly as far as the model was drawn consistently.** Revit's
/// `FromRoom`/`ToRoom` follow the door instance's *orientation*, not the leaf
/// swing — flipping a door swaps them. So "the room it opens into" is a
/// modelling convention, which is why this is project policy with an override
/// rather than a rule in code, the same stance `measurement_standard` takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub enum RoomAttribution {
    /// The room the door opens *into*, falling back to the one it opens *from*.
    /// A door with neither is **homeless** — a reported state, not an error.
    #[default]
    ToRoomThenFromRoom,
    /// Strictly the room it opens into. A door with only a `from_room` is
    /// homeless under this policy even though it has a reference.
    ToRoom,
    /// Strictly the room it opens from.
    FromRoom,
    /// Both, so a door between two rooms is attributed twice. The reason
    /// `owner_rooms` is a list rather than an `Option`.
    Both,
    /// Never attribute a door to a room — every door is homeless, by choice.
    None,
}

impl RoomAttribution {
    /// The rooms this policy attributes one door to, in order, skipping absent
    /// references. Empty means homeless.
    ///
    /// The single place the chain is expressed, so the read path, the QA report
    /// and any future rollup cannot disagree about what a door belongs to.
    ///
    /// Generic over what a reference *is*, because it is now two things: a bare
    /// room id where the model stated one, and a model-qualified reference where
    /// the geometry derived one (a probe can reach a room in a linked model, and
    /// a bare id would be resolved against the wrong model's rooms). The policy
    /// is about presence and order, never about the payload, so making it
    /// generic keeps one chain rather than a second copy that could drift.
    pub fn owners<'a, T: ?Sized>(&self, from_room: Option<&'a T>, to_room: Option<&'a T>) -> Vec<&'a T> {
        match self {
            RoomAttribution::ToRoomThenFromRoom => to_room.or(from_room).into_iter().collect(),
            RoomAttribution::ToRoom => to_room.into_iter().collect(),
            RoomAttribution::FromRoom => from_room.into_iter().collect(),
            RoomAttribution::Both => [to_room, from_room].into_iter().flatten().collect(),
            RoomAttribution::None => vec![],
        }
    }
}

/// Where a route's size checks read their data from.
///
/// **A route can be asked whether an object of a given width and height can make the
/// trip, and the model does not state most of what that needs.** No door carries a
/// width or a height in a form this server can rely on, and a room's clear height is
/// whatever parameter a project's template calls it. So the sources are a project's to
/// name, and everything they yield is reported as an ESTIMATE:
///
/// - a **door's width** is its footprint (overall, frame included) less
///   `door_frame_allowance_mm`;
/// - a **door's height** is the first of `door_height_properties` it carries, else, when
///   `door_height_from_type_name` is on, the size in its type name ("970 x 2040");
/// - a **room's height** is the lowest ceiling over it (`room_height_from_ceilings`),
///   else its `room_height_property`, read as millimetres.
///
/// Nothing is guessed when a source is silent: a door or room it cannot read is let
/// through and counted as unchecked, which the answer says.
///
/// Scalars first and the list last, for the TOML ordering rule in
/// CODING-CONVENTIONS.md.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct RoutingPolicy {
    /// The room property holding a room's clear height, in millimetres. Matched
    /// ignoring case and runs of spaces, because Revit parameter names are typed by
    /// people (one real project's is `Ceiling  Height`, with two). `None` means
    /// `RoutingPolicy::DEFAULT_ROOM_HEIGHT_PROPERTY`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_height_property: Option<String>,

    /// Whether a room's height is read from the CEILINGS over it, with the property as
    /// the fallback for a room none covers. On by default: a ceiling's offset is where the
    /// ceiling really is (a drop ceiling, a bulkhead) and the property is whatever a
    /// template says. The lowest ceiling covering at least a quarter of the room wins, and
    /// an offset at or below zero (a roof, a ceiling below its level) is not a ceiling over a
    /// room and is ignored. Measured on a real project: ceilings and the property each cover most rooms,
    /// and using both reaches more than either.
    #[serde(default = "RoutingPolicy::yes")]
    pub room_height_from_ceilings: bool,

    /// Whether a door's height may be read from its type name when none of
    /// `door_height_properties` is present: the convention that puts the size in the
    /// name ("970 x 2040 TD01", "DWWH-003 820W x 2100H"). On by default, because on
    /// a real project nearly all door types follow it and no door has a height property at all. Off
    /// for a project whose names say something else that happens to look like a size.
    #[serde(default = "RoutingPolicy::yes")]
    pub door_height_from_type_name: bool,

    /// What a door's frame and stops take off the width its footprint measures, in
    /// millimetres, to give the clear opening. `None` means
    /// `RoutingPolicy::DEFAULT_DOOR_FRAME_ALLOWANCE_MM`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub door_frame_allowance_mm: Option<f64>,

    /// Door properties tried, in order, for a door's height in millimetres (instance
    /// before type). Empty means `RoutingPolicy::DEFAULT_DOOR_HEIGHT_PROPERTIES`.
    #[serde(default)]
    pub door_height_properties: Vec<String>,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            room_height_property: None,
            room_height_from_ceilings: true,
            door_height_from_type_name: true,
            door_frame_allowance_mm: None,
            door_height_properties: Vec::new(),
        }
    }
}

impl RoutingPolicy {
    pub const DEFAULT_ROOM_HEIGHT_PROPERTY: &'static str = "Ceiling Height";
    pub const DEFAULT_DOOR_HEIGHT_PROPERTIES: &'static [&'static str] = &["Height", "Door Height"];
    /// The footprint is overall width, frame included: on a real project a "970 x 2040" leaf
    /// measures about 1,055 mm and its clear opening is nearer 900. 150 is a rule of thumb
    /// for a standard single door, an estimate and not a measurement.
    pub const DEFAULT_DOOR_FRAME_ALLOWANCE_MM: f64 = 150.0;
    /// The most a frame allowance may be: beyond it no door has a clear opening.
    pub const MAX_DOOR_FRAME_ALLOWANCE_MM: f64 = 2_000.0;

    fn yes() -> bool {
        true
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The room height property in force.
    pub fn room_height_property(&self) -> &str {
        self.room_height_property.as_deref().unwrap_or(Self::DEFAULT_ROOM_HEIGHT_PROPERTY)
    }

    /// The door height properties in force, in the order they are tried.
    pub fn door_height_properties(&self) -> Vec<String> {
        if self.door_height_properties.is_empty() {
            Self::DEFAULT_DOOR_HEIGHT_PROPERTIES.iter().map(|s| s.to_string()).collect()
        } else {
            self.door_height_properties.clone()
        }
    }

    /// The frame allowance in force, in millimetres.
    pub fn frame_allowance_mm(&self) -> f64 {
        self.door_frame_allowance_mm.unwrap_or(Self::DEFAULT_DOOR_FRAME_ALLOWANCE_MM)
    }

    /// Startup-loud checks: a blank property name can never match, and an allowance
    /// that is not a number or is absurd would silently zero every door.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.room_height_property.as_deref().is_some_and(|p| p.trim().is_empty()) {
            anyhow::bail!("[routing] room_height_property must not be blank; leave it out for the default");
        }
        if self.door_height_properties.iter().any(|p| p.trim().is_empty()) {
            anyhow::bail!("[routing] door_height_properties must not hold a blank name");
        }
        if let Some(a) = self.door_frame_allowance_mm
            && (!a.is_finite() || !(0.0..=Self::MAX_DOOR_FRAME_ALLOWANCE_MM).contains(&a))
        {
            anyhow::bail!(
                "[routing] door_frame_allowance_mm must be between 0 and {} mm (got {a})",
                Self::MAX_DOOR_FRAME_ALLOWANCE_MM
            );
        }
        Ok(())
    }
}

/// How the room-state audit judges an element against a blueprint's layout (`service::blueprint_audit`):
/// when an element is "in place", when it is merely "moved", and when it is not the layout's item at all.
///
/// **Settings, not constants, because the right numbers belong to the office and the model.** The place
/// tolerance is small on purpose: a level-based item is put at the recipe's point and facing with no
/// snapping, and a real Revit run (2026-11-28) read nine placed elements back at 0.000 mm, so anything a
/// person did by hand shows up as drift. The near tolerance is the line between "the same bed, moved" and
/// "a different bed": beyond it the element is reported `extra` and the item `missing`, never paired,
/// because pairing them would call one bed another.
///
/// Plain numbers with defaults rather than `Option`s: there is nothing for "unset" to mean that the default
/// does not, and a written settings file lists what the project runs on (`Settings::fill_defaults`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct BlueprintAuditPolicy {
    /// Within this distance (mm) an element is `in_place`.
    #[serde(default = "BlueprintAuditPolicy::default_place")]
    pub place_tolerance_mm: f64,

    /// Within this distance (mm), and beyond the place tolerance, an element of the right code is `moved`;
    /// beyond it the element and the item are not a pair.
    #[serde(default = "BlueprintAuditPolicy::default_near")]
    pub near_tolerance_mm: f64,

    /// Facing within this many degrees agrees with the layout's.
    #[serde(default = "BlueprintAuditPolicy::default_turn")]
    pub turn_tolerance_deg: f64,
}

impl Default for BlueprintAuditPolicy {
    fn default() -> Self {
        Self {
            place_tolerance_mm: Self::DEFAULT_PLACE_TOLERANCE_MM,
            near_tolerance_mm: Self::DEFAULT_NEAR_TOLERANCE_MM,
            turn_tolerance_deg: Self::DEFAULT_TURN_TOLERANCE_DEG,
        }
    }
}

impl BlueprintAuditPolicy {
    pub const DEFAULT_PLACE_TOLERANCE_MM: f64 = 5.0;
    pub const DEFAULT_NEAR_TOLERANCE_MM: f64 = 600.0;
    pub const DEFAULT_TURN_TOLERANCE_DEG: f64 = 5.0;
    /// The most a near tolerance may be: past a room's width every element would "be" the same item.
    pub const MAX_NEAR_TOLERANCE_MM: f64 = 5_000.0;
    /// The most a turn tolerance may be: past a quarter turn "facing" stops meaning anything.
    pub const MAX_TURN_TOLERANCE_DEG: f64 = 45.0;

    fn default_place() -> f64 {
        Self::DEFAULT_PLACE_TOLERANCE_MM
    }
    fn default_near() -> f64 {
        Self::DEFAULT_NEAR_TOLERANCE_MM
    }
    fn default_turn() -> f64 {
        Self::DEFAULT_TURN_TOLERANCE_DEG
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Startup-loud: a tolerance that is not a number, is negative, or leaves no room between "in place" and
    /// "the same item" would silently change every verdict.
    pub fn validate(&self) -> anyhow::Result<()> {
        for (name, value) in [
            ("place_tolerance_mm", self.place_tolerance_mm),
            ("near_tolerance_mm", self.near_tolerance_mm),
            ("turn_tolerance_deg", self.turn_tolerance_deg),
        ] {
            if !value.is_finite() || value < 0.0 {
                anyhow::bail!("[blueprint_audit] {name} must be a number of at least 0 (got {value})");
            }
        }
        if self.near_tolerance_mm > Self::MAX_NEAR_TOLERANCE_MM {
            anyhow::bail!(
                "[blueprint_audit] near_tolerance_mm must be at most {} mm (got {})",
                Self::MAX_NEAR_TOLERANCE_MM,
                self.near_tolerance_mm
            );
        }
        if self.place_tolerance_mm > self.near_tolerance_mm {
            anyhow::bail!(
                "[blueprint_audit] place_tolerance_mm ({}) must not exceed near_tolerance_mm ({})",
                self.place_tolerance_mm,
                self.near_tolerance_mm
            );
        }
        if self.turn_tolerance_deg > Self::MAX_TURN_TOLERANCE_DEG {
            anyhow::bail!(
                "[blueprint_audit] turn_tolerance_deg must be at most {} degrees (got {})",
                Self::MAX_TURN_TOLERANCE_DEG,
                self.turn_tolerance_deg
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct OpeningPolicy {
    /// The door property whose value identifies "the same door" across
    /// milestones — the door counterpart of `Settings::comparison_key`, and
    /// `None` (the default) is a real, reachable state that reports "not
    /// configured" rather than falling back to anything.
    ///
    /// `"$id"` is usually the right answer, and is worth stating because it is
    /// *not* the right answer for rooms: a door's ElementId is stable across
    /// pushes of the same model, so it identifies the same physical leaf, while
    /// a room comparison keys on an authored Number precisely because room ids
    /// are not meaningful to the people reading the diff. There is still no
    /// default — a project that renumbers doors between milestones wants `Mark`,
    /// and guessing wrong produces a diff that looks authoritative and is wrong.
    ///
    /// A scalar declared before `comparison_properties` per the TOML ordering
    /// rule in CODING-CONVENTIONS.md.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_key: Option<String>,

    /// Which room a door belongs to (see `RoomAttribution`).
    ///
    /// **This has a default, reversing what the design originally proposed.**
    /// That sketch argued for no default, on the grounds that an unset value
    /// meaning "do not attribute" is honest rather than arbitrary. The argument
    /// was sound for a *single pick* — choosing `to_room` on a project's behalf
    /// would silently mis-attribute every external door. It stops holding for
    /// the chain, which attributes only what the model actually states and names
    /// the rest; and the attribution is derived at read time, never stored, so a
    /// project that disagrees changes one line and loses nothing.
    #[serde(default)]
    pub room_attribution: RoomAttribution,

    /// How far the server may go to work out a door's rooms from its geometry
    /// (see `RoomResolution`). Off by default.
    ///
    /// Declared after `room_attribution` because it feeds it: a derived
    /// reference fills an absent side, and attribution then runs over the
    /// result. It never replaces a reference the model states.
    #[serde(default)]
    pub room_resolution: RoomResolution,

    /// The door property carrying an **authored** room reference (in the sample
    /// export, `"Door Room Reference"`), reconciled against the room
    /// `room_attribution` actually picked. Absent — the default — disables the
    /// check.
    ///
    /// **A property name rather than the `reconcile_room_reference = true` the
    /// design first sketched.** A bool would have needed the property name
    /// hard-coded, and which parameter carries this is a family-and-office
    /// convention, not a fact about doors. One field states both that the check
    /// is wanted and what it reads.
    ///
    /// It is worth turning on: on the House A sample the authored value
    /// disagrees with the attributed room on 4 of 26 doors, three of them where
    /// the geometry picks an exterior or circulation space over the served room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_reference_property: Option<String>,

    /// Ordered door property names compared across milestones, resolved in the
    /// same vocabulary the `/doors` filter uses — so `$to_room` and `$from_room`
    /// are comparable, which is how "this door moved between rooms" becomes a
    /// reported difference with no machinery of its own.
    #[serde(default)]
    pub comparison_properties: Vec<String>,
}

impl OpeningPolicy {
    /// Whether this policy is entirely unset — used to keep an untouched
    /// `[doors]` section out of a written settings file, same as `AreaPolicy`.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Whether a family instance that is a *component of another instance* counts
/// as an item.
///
/// **The one place FF&E deliberately does not follow doors**, and the argument
/// is about what kind of question it is. `nested_opening_ids` drops a door leaf
/// at the producer and is right to: "is this door leaf a door" has one answer,
/// always, everywhere. "Is this component an item" does not -- a joinery handle
/// is not, a chair nested in a workstation group might be -- so it is a project
/// convention, and conventions live here and are applied at read time.
///
/// That also makes the exclusion *visible*. 2236 of 4134 exported "doors" on one
/// job were hardware, and they were invisible because the producer had already
/// dropped them by the time anyone could count them. `ItemReport` states how
/// many this policy excluded, on every read, from the first push.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub enum NestedComponents {
    /// A component is not an item. **The default, and it is a measurement
    /// rather than a preference**: on House A, 179 of 647 instances had a
    /// super-component and every population in that set was hardware or
    /// sub-assembly -- 87 joinery handles of one family, 70 generic models
    /// inside electrical fixtures, 3 generic models inside doors.
    #[default]
    Exclude,
    /// Every instance is an item, components included. For a project that nests
    /// genuinely schedulable FF&E -- a workstation group whose chair is a real
    /// line on the schedule.
    Include,
}

/// FF&E policy for one project.
///
/// Four of these fields are spelled exactly as `OpeningPolicy` spells them and
/// mean exactly what they mean there. They are not shared through a common
/// struct, because the two policies differ in both directions: an opening needs
/// `room_attribution` and an item has nothing to attribute, an item needs
/// `nested_components` and an opening has nothing to exclude. A shared struct
/// would carry both exceptions and force every reader to work out which half
/// applied.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct FfePolicy {
    /// The item property whose value identifies "the same item" across
    /// milestones, and `None` (the default) reports "not configured" rather
    /// than falling back to anything.
    ///
    /// **`"$id"` is a worse default here than it is for doors, which is why
    /// there is still no default at all.** A door's ElementId is stable across
    /// pushes and identifies the same physical leaf. FF&E is the thing that gets
    /// deleted and re-placed wholesale between issues -- a re-specified desk is
    /// a new ElementId and the same desk -- so a project comparing FF&E across
    /// milestones usually wants an authored `Mark` or an asset number, and
    /// guessing `$id` would produce a diff that looks authoritative and reports
    /// every replaced item as one deletion plus one addition.
    ///
    /// A scalar declared before `comparison_properties` per the TOML ordering
    /// rule in CODING-CONVENTIONS.md.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_key: Option<String>,

    /// Whether a component of another instance counts as an item (see
    /// `NestedComponents`). Defaults to excluding them.
    #[serde(default)]
    pub nested_components: NestedComponents,

    /// How far the server may go to work out an item's room from its geometry
    /// (see `RoomResolution`). Off by default.
    ///
    /// **Off is right here where it was arguable for windows**, and the reason
    /// is the premise of the entity rather than a preference. A facade model
    /// holds windows and no rooms, so `room_resolution` is the only mechanism by
    /// which any of its openings is ever attributed. FF&E lives in the same
    /// document as the rooms it serves -- that is *why* RoomMate performs this
    /// join, since Revit will not schedule it -- so authored references
    /// populate: 572 of 647 on House A. Geometry is a fallback for the
    /// remainder, taken deliberately, not the primary path.
    #[serde(default)]
    pub room_resolution: RoomResolution,

    /// The item property carrying an **authored** room reference, reconciled
    /// against the room the item actually names. Absent -- the default --
    /// disables the check.
    ///
    /// A property name rather than a bool, for the reason `[doors]` gives: which
    /// parameter carries this is a family-and-office convention, not a fact
    /// about furniture, and one field states both that the check is wanted and
    /// what it reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_reference_property: Option<String>,

    /// Ordered item property names compared across milestones, resolved in the
    /// same vocabulary the `/ffe` filter uses -- so `$room` and `$category` are
    /// comparable, which is how "this item moved between rooms" becomes a
    /// reported difference with no machinery of its own.
    #[serde(default)]
    pub comparison_properties: Vec<String>,
}

impl FfePolicy {
    /// Whether this policy is entirely unset -- used to keep an untouched
    /// `[ffe]` section out of a written settings file, same as `OpeningPolicy`.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct AreaPolicy {
    /// What the reported area figure *means*. `None` (the default) is an honest
    /// "undeclared" and is echoed as such — not silently presented as any
    /// particular standard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement_standard: Option<MeasurementStandard>,

    /// Feet. The widest gap between finish-face rooms still counted as a wall;
    /// anything wider is a genuine void (a courtyard, atrium, lightwell) and
    /// stays open. Must be positive and finite — it describes a real partition,
    /// and a zero-thickness wall is not a thing.
    ///
    /// **Zero is not how you say "centreline".** That is the *regime*, declared
    /// per model on the envelope, and it collapses the effective gap to zero
    /// (see `wall_gap_ft`). Conflating the two would make a project-wide policy
    /// value override a per-model fact — the exact mistake declaring the regime
    /// per model exists to prevent. `adjacency`'s `?wall_max=0` request parameter is a different
    /// thing again: a caller asking one question, not a project declaring policy.
    #[serde(default = "AreaPolicy::default_max_wall_thickness")]
    pub max_wall_thickness: f64,

    /// **Fallback only** for models whose extractor predates
    /// `contract::RoomBoundary` on the envelope. A declared envelope value
    /// always wins — this can never override a model that states its own regime,
    /// because the model is the authority. `None` (the default) falls through to
    /// finish face, which is the conservative reading: a close still runs, which
    /// is exactly today's behaviour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_location: Option<crate::contract::RoomBoundary>,
}

impl Default for AreaPolicy {
    fn default() -> Self {
        Self {
            measurement_standard: None,
            max_wall_thickness: Self::DEFAULT_MAX_WALL_THICKNESS_FT,
            boundary_location: None,
        }
    }
}

impl AreaPolicy {
    /// ~1.5 ft (~450mm): clears a thick partition without reaching across the
    /// narrowest real void. The value `areas::MAX_WALL_FT` and
    /// `adjacency::WALL_MAX_FT` both independently carried before they became
    /// one declared quantity, so an un-migrated project behaves exactly as it
    /// did.
    pub const DEFAULT_MAX_WALL_THICKNESS_FT: f64 = 1.5;

    /// Hard ceiling on the declared thickness, and on a requested
    /// `?wall_max=`. Beyond ~5 ft a "wall" spans a corridor, and the answer
    /// stops being about walls at all. Rejected loudly rather than clamped.
    pub const MAX_WALL_THICKNESS_LIMIT_FT: f64 = 5.0;

    fn default_max_wall_thickness() -> f64 {
        Self::DEFAULT_MAX_WALL_THICKNESS_FT
    }

    /// Whether this policy is entirely defaults — the `skip_serializing_if`
    /// predicate, so an untouched project file gains no `[areas]` block.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Startup-loud checks. A thickness that is non-finite, non-positive or
    /// absurd can never describe a wall, and left unchecked it would silently
    /// produce either no wall zone at all or one that swallows courtyards.
    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.max_wall_thickness.is_finite() {
            anyhow::bail!("[areas] max_wall_thickness must be a finite number");
        }
        if self.max_wall_thickness <= 0.0 {
            anyhow::bail!(
                "[areas] max_wall_thickness must be positive (got {}); a centreline model is \
                 declared by its boundary regime, not by a zero wall thickness",
                self.max_wall_thickness
            );
        }
        if self.max_wall_thickness > Self::MAX_WALL_THICKNESS_LIMIT_FT {
            anyhow::bail!(
                "[areas] max_wall_thickness {} ft exceeds the {} ft limit; a gap that wide \
                 bridges rooms, not walls",
                self.max_wall_thickness,
                Self::MAX_WALL_THICKNESS_LIMIT_FT
            );
        }
        Ok(())
    }

    /// The regime in force for one model: what the model declared, else this
    /// project's fallback, else finish face. Three levels, most authoritative
    /// first — the model knows, the project guesses, the server assumes the
    /// case that still needs work done.
    pub fn resolve_boundary(&self, declared: Option<crate::contract::RoomBoundary>) -> crate::contract::RoomBoundary {
        declared.or(self.boundary_location).unwrap_or(crate::contract::RoomBoundary::FinishFace)
    }

    /// The gap this regime implies, in feet — the one number `areas` closes by
    /// and `adjacency` defaults to. Centreline rooms already touch, so there is
    /// nothing to bridge and nothing to fill: zero, not a small number. That
    /// zero is the whole payoff of declaring the regime — at radius zero the morphological
    /// close is the identity, so bevels, chamfers, spikes and sibling overlaps
    /// cannot arise at all rather than merely being smaller.
    pub fn wall_gap_ft(&self, boundary: crate::contract::RoomBoundary) -> f64 {
        match boundary {
            crate::contract::RoomBoundary::Centreline => 0.0,
            crate::contract::RoomBoundary::FinishFace => self.max_wall_thickness,
        }
    }
}
