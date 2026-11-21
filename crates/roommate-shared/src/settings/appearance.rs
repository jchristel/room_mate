//! How the viewer looks, from settings: colours per room state and element, the outline layers, hover properties and the element tags.

use super::*;

/// Per-entity colour overrides for the plan.
///
/// **Every field is optional, and absent means "whatever the theme says" --
/// which is the behaviour that existed before this type did.** The viewer's
/// palette comes from seven CSS custom properties read fresh on every paint, so
/// it follows the reader's light/dark theme; a project that pinned every colour
/// would look correct in one theme and unreadable in the other. Overriding only
/// what is named keeps the rest theme-aware, and makes the whole feature
/// additive: a project that sets nothing renders exactly as it did.
///
/// **Three shapes rather than one, because the entities genuinely differ.** A
/// single struct with four optional colours for all six would offer a ceiling
/// a hover FILL and a space a fill -- neither is drawn, and the setting would
/// do nothing. The settings page is generated from these types, so a field
/// here becomes a control there; giving each entity exactly the fields it can
/// express is what keeps a dead control impossible to build rather than merely
/// discouraged.
///
/// **What an entity can express changes as the viewer grows, and this type is
/// where that is recorded.** `OutlineAppearance` carried only `line` because
/// spaces, ceilings and floors had no pick index when it was written; the
/// selection filter gave them one, so `selection` is real now and the field
/// exists. The direction of the dependency is the point: a control is added
/// because something draws it, never the other way round.
///
/// Values are CSS colour strings, stored VERBATIM and never parsed here -- the
/// same treatment `Band::colour` gets, and for the same reason: the server
/// computes nothing about colour, so a spelling it accepted would prove
/// nothing about what a browser will render. The settings page writes these
/// with `<input type="color">`, which cannot produce anything but `#rrggbb`.
#[derive(Debug, Default, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct Appearance {
    /// Rooms: the only entity with all four. They are the base layer -- the
    /// only one that is filled, hovered and selected.
    #[serde(default, skip_serializing_if = "RoomAppearance::is_default")]
    pub rooms: RoomAppearance,

    /// Doors.
    #[serde(default, skip_serializing_if = "ElementAppearance::is_default")]
    pub doors: ElementAppearance,

    /// Windows.
    #[serde(default, skip_serializing_if = "ElementAppearance::is_default")]
    pub windows: ElementAppearance,

    /// FF&E.
    #[serde(default, skip_serializing_if = "ElementAppearance::is_default")]
    pub ffe: ElementAppearance,

    /// Spaces: an outline over the rooms, with no fill.
    #[serde(default, skip_serializing_if = "OutlineAppearance::is_default")]
    pub spaces: OutlineAppearance,

    /// Ceilings: as spaces. The dash is not settable here -- it is what tells a
    /// ceiling ring apart from the room outline directly beneath it, so it is a
    /// rule rather than a preference.
    #[serde(default, skip_serializing_if = "OutlineAppearance::is_default")]
    pub ceilings: OutlineAppearance,

    /// Floors: as ceilings, and the dash is not settable for the same reason.
    /// A floor, a ceiling and a room outline can all sit on one line of the
    /// plan, so the three are told apart by their stroke pattern -- solid,
    /// long dash, dot -- and a colour override cannot take that away.
    #[serde(default, skip_serializing_if = "OutlineAppearance::is_default")]
    pub floors: OutlineAppearance,

    /// The blueprint box: the outline a blueprint's items are written against, drawn in the
    /// blueprint viewer (where its nodes are edited) and over the plan by the Blueprint tool.
    /// Theme default: a blue, so it is never mistaken for the room outline it sits on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint_box: Option<String>,
}

impl Appearance {
    /// True when nothing is overridden, so an untouched block stays out of the
    /// written file.
    pub fn is_default(&self) -> bool {
        self.rooms.is_default()
            && self.doors.is_default()
            && self.windows.is_default()
            && self.ffe.is_default()
            && self.spaces.is_default()
            && self.ceilings.is_default()
            && self.floors.is_default()
            && self.blueprint_box.is_none()
    }
}

/// A room's four colours. The outline, the fill, the selection ring and the
/// hover fill.
#[derive(Debug, Default, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct RoomAppearance {
    /// Outline. Theme default: `--ink`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// Fill. Theme default: `--fill`. A room carrying a colour plan's fill is
    /// unaffected -- the plan is the more specific answer and already wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// Selection ring. Theme default: `--accent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    /// Hover fill. Theme default: `--fill-hover`. A room flagged by the QA
    /// report still hovers to the accent, which is a state beating a
    /// preference rather than an omission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover: Option<String>,
}

impl RoomAppearance {
    pub fn is_default(&self) -> bool {
        self.line.is_none() && self.fill.is_none() && self.selection.is_none() && self.hover.is_none()
    }
}

/// An element layer's three colours: doors, windows and FF&E each draw a
/// filled footprint, a glyph over it, and a selection ring. They share a shape
/// because they share a drawing convention, not by coincidence -- `doorGlyph`,
/// `windowGlyph` and `itemGlyph` were each written from the one before it.
///
/// No `hover`: hover is a room-only state today (`setHover` takes a room id).
/// Offering the colour would be offering a control with nothing behind it.
#[derive(Debug, Default, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct ElementAppearance {
    /// The glyph -- a door's arc, a window's panes, an item's orientation tick
    /// and marker. Theme default: `--ink`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// The footprint rectangle beneath the glyph, drawn semi-transparent.
    /// Theme default: `--ink` at the layer's own alpha, which an override
    /// keeps -- the transparency is what stops a footprint hiding the room
    /// under it, so it is structural rather than stylistic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// Selection ring. Theme default: `--accent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
}

impl ElementAppearance {
    pub fn is_default(&self) -> bool {
        self.line.is_none() && self.fill.is_none() && self.selection.is_none()
    }
}

/// An overlay drawn as an outline only: never filled, but selectable since the
/// per-zone selection filter gave these three a pick index.
///
/// No `fill`, and that one is structural rather than pending: a space, a
/// ceiling and the room beneath them occupy the same ground, so a second
/// filled polygon would either hide the room or be hidden by it. The layer is
/// an outline BECAUSE the comparison with the room is the question it exists
/// to answer.
#[derive(Debug, Default, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct OutlineAppearance {
    /// The ring. Theme default: `--accent` for spaces, `--ink` for ceilings and
    /// floors -- and that contrast is the point, so overriding one and not the
    /// other is a way to lose it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// Selection ring, and the pick list's hover preview of it. Theme default:
    /// `--accent`.
    ///
    /// **Worth setting precisely because the ring is thicker than the layer's
    /// own line.** A ceiling draws dashed in the room ink, so its selection
    /// ring -- solid, 4px, accent -- is what separates "this ceiling" from the
    /// ceiling line itself; a project that has already moved `line` off the
    /// ink will usually want to move this with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
}

impl OutlineAppearance {
    pub fn is_default(&self) -> bool {
        self.line.is_none() && self.selection.is_none()
    }
}

/// Which property a hover over the plan reads out, per entity.
///
/// **One flat struct where `Appearance` needed three**, and the difference is
/// real rather than a style choice: appearance is shaped by what each entity
/// DRAWS, which genuinely differs — a space has no fill, a ceiling has no
/// hover state. Every entity here carries properties, so every entity can
/// answer the same question, and splitting this would be shape for its own
/// sake.
///
/// **Absent is the viewer's existing behaviour, not an empty tooltip**: the
/// hover falls back to the element's name, then its id. So does a name that no
/// element carries — which is the ordinary failure here, since these are Revit
/// parameter names typed by hand and nothing on the server knows a door's
/// vocabulary. A typo costs a reader the feature, never the tooltip; the
/// alternative, a blank hover, reads as a broken plan.
///
/// The lookup is the contract's own tier rule: the INSTANCE parameter, then
/// the TYPE, and a tier only wins when it holds something. A blank instance
/// parameter does not shadow a real type value — `Door Leaf Thickness` is
/// blank on 22 of 26 sample doors while the type says 40.0, which is exactly
/// the property someone would put here.
#[derive(Debug, Default, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct HoverProperties {
    /// Rooms. Model properties only: a joined reference field is not on the
    /// wire for every read this has to serve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rooms: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doors: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ffe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spaces: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ceilings: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floors: Option<String>,
}

impl HoverProperties {
    /// True when no entity names one, so an untouched block stays out of the
    /// written file.
    pub fn is_default(&self) -> bool {
        self.rooms.is_none()
            && self.doors.is_none()
            && self.windows.is_none()
            && self.ffe.is_none()
            && self.spaces.is_none()
            && self.ceilings.is_none()
            && self.floors.is_none()
    }
}

/// What the tag written on each FF&E item, space, ceiling and floor says.
///
/// **Rooms keep `room_label`**, which the server resolves; an element's tag is
/// resolved by the page, from the same two property tiers the hover reads, because
/// an element response carries its properties and the page already does that lookup.
/// Every entity has the same two questions, so one shape serves all four.
#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct ElementTags {
    #[serde(default = "TagSettings::ffe")]
    pub ffe: TagSettings,
    #[serde(default = "TagSettings::spaces")]
    pub spaces: TagSettings,
    #[serde(default = "TagSettings::surface")]
    pub ceilings: TagSettings,
    #[serde(default = "TagSettings::surface")]
    pub floors: TagSettings,
}

impl Default for ElementTags {
    fn default() -> Self {
        Self {
            ffe: TagSettings::ffe(),
            spaces: TagSettings::spaces(),
            ceilings: TagSettings::surface(),
            floors: TagSettings::surface(),
        }
    }
}

/// One entity's tag: the ordered fields and the two colours.
///
/// **Fields** are `$id`, `$name` and `$type_name` (an element's own, which a property
/// lookup cannot reach) or a property name, read instance first, then type, a tier
/// winning only when it holds something. An element with none of them prints no tag,
/// and an empty list prints no tag on the entity: a tag the settings withheld is not
/// one to invent.
///
/// **Colours** are CSS strings stored verbatim, like `Appearance`. Absent follows the
/// layer's own line colour, so a tag reads as part of the layer it labels and follows
/// the theme; `secondary` is the stacked lines under the first and falls back to
/// `primary`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct TagSettings {
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<String>,
}

impl TagSettings {
    fn with(fields: &[&str]) -> Self {
        Self {
            fields: fields.iter().map(|f| f.to_string()).collect(),
            primary: None,
            secondary: None,
        }
    }
    fn ffe() -> Self {
        Self::with(&["$type_name"])
    }
    fn spaces() -> Self {
        Self::with(&["$name"])
    }
    fn surface() -> Self {
        Self::with(&["$type_name"])
    }
}
