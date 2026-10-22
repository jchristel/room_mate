//! What a blueprint revision holds: the room's outline, and each item as a recipe
//! against the outline's edges.
//!
//! **A recipe, never coordinates.** An item is stored as *which edge, how far along
//! it, how far into the room, which way it faces*. Nothing derived is stored: the
//! placement in a particular room is the outline plus the recipe, worked out at read
//! time (`docs/PLAN-blueprints.md`, decision 1). Both the distance as captured **and**
//! the mode that says how it is interpreted ride the record, so switching a dimension
//! between fixed and percentage is a change of mode, never a recomputation that could
//! drift from what the author saw.
//!
//! **Placements are written against the coarse outline** (see `blueprint_geometry`), and
//! the fine outline rides beside it for checking findings. An item in a column nib is
//! outside the coarse outline and inside the fine one.
//!
//! **An item is the FF&E record as exported** (decision 5): family, type name,
//! category, and both property tiers verbatim. Sizes, heights and elevation are in the
//! properties and are read by name when needed, not lifted into typed fields here.

use serde::{Deserialize, Serialize};

use super::blueprint_geometry::{edges, Pt};

/// A plan point, millimetres, in the blueprint's own frame (the room's centroid is the
/// origin). On the wire as an object, like every other point here.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl From<Pt> for Point {
    fn from(p: Pt) -> Self {
        Point { x: p.0, y: p.1 }
    }
}

impl From<Point> for Pt {
    fn from(p: Point) -> Self {
        (p.x, p.y)
    }
}

/// The room's outline at the two tolerances. See `blueprint_geometry`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    /// Jogs under about 350 mm removed. **Edge indices in a placement refer to this
    /// outline**, counter-clockwise, edge `i` running from vertex `i` to vertex `i + 1`.
    pub coarse: Vec<Point>,
    /// Corners only (about 30 mm). What "is this item inside the room" is checked against.
    pub fine: Vec<Point>,
}

/// How `Placement::along_mm` is read in a room of another size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlongMode {
    /// Keep this many mm from the edge's start.
    FixedFromStart,
    /// Keep this many mm from the edge's end (`edge_len_mm - along_mm` as captured).
    FixedFromEnd,
    /// Keep the same fraction of the edge.
    Percent,
}

/// How `Placement::off_mm` is read in a room of another size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OffMode {
    /// Keep this many mm from the edge.
    Fixed,
    /// Keep the same fraction of the room's depth in front of the edge.
    Percent,
}

/// A free-standing item is checked for overlaps and sits where its footprint is. A wall
/// device (a switch, a socket, a rail: under 100 mm thick) is located by its wall and
/// height, carries no footprint check, and is left out of overlap findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemClass {
    Free,
    WallDevice,
}

/// An item's place in the room, in the room's own frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// Index into `Outline::coarse`'s edges.
    pub edge: usize,
    /// Distance of the item's centre along the edge from its start, as captured. May lie
    /// outside `0..=edge_len_mm` only for an item with no edge beside it (`corner_anchored`).
    pub along_mm: f64,
    pub along_mode: AlongMode,
    /// Distance of the centre into the room from the edge, as captured.
    pub off_mm: f64,
    pub off_mode: OffMode,
    /// The edge's length at capture: what `Percent` is a fraction of, and what
    /// `FixedFromEnd` counts back from.
    pub edge_len_mm: f64,
    /// The room's depth in front of the edge at the item, at capture: what an `off_mm`
    /// `Percent` is a fraction of.
    pub depth_mm: f64,
    /// Rotation of the item's facing from the edge's inward normal, degrees, counter-clockwise.
    pub facing_deg: f64,
    /// The item had no edge whose span contains it (a corner of a notched room), so its
    /// nearest edge was used and `along_mm` falls outside the edge. Reported, not hidden.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub corner_anchored: bool,
}

impl Placement {
    /// The item's centre as captured, in the frame of `coarse`. Used to prove capture
    /// is lossless; the resolver for another room is a separate thing.
    pub fn centre_in(&self, coarse: &[Pt]) -> Option<Pt> {
        let e = *edges(coarse).get(self.edge)?;
        Some(e.point(self.along_mm, self.off_mm))
    }
}

/// An item that sits against, or inside, another: a run of casework, or a shelf in a
/// wardrobe. Independent percentages would squash a run (183 of 256 overlaps when a room
/// shrank 15%, in stage 0), so a follower keeps its gap to the item it follows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Follow {
    /// The followed item's `BlueprintItem::id`.
    pub item: String,
    /// Along the edge, from the followed item's far side to this one's near side, mm.
    /// Negative when they overlap. Meaningless when `inside`.
    pub gap_mm: f64,
    /// Whether this item sits wholly within the followed one's footprint, rather than
    /// beside it.
    pub inside: bool,
}

/// One property as exported: the source's own name and the raw string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Property {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlueprintItem {
    /// The Revit element id in the model it was captured from. Identifies the item within
    /// the blueprint; it names nothing in another model.
    pub id: String,
    /// The family. Absent until the extractor that pushed this item carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_name: Option<String>,
    pub type_name: String,
    pub category: String,
    pub class: ItemClass,
    /// Removed from the blueprint but kept: it can be added back, and an earlier
    /// revision's settings imported onto it. Never deleted.
    #[serde(default)]
    pub excluded: bool,
    pub placement: Placement,
    /// Footprint size in plan, mm: along the facing direction, then across it.
    pub size_mm: [f64; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follows: Option<Follow>,
    /// Instance properties as exported, in name order.
    #[serde(default)]
    pub properties: Vec<Property>,
    /// Type properties as exported, in name order.
    #[serde(default)]
    pub type_properties: Vec<Property>,
}

/// A door of the room the blueprint was captured from. **Orientation only**: doors are already
/// placed in the model, and a blueprint never creates one. They exist so that two rotations
/// that fit a room's shape equally well (a rectangle fits twice, half a turn apart) can be told
/// apart by where the doors are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlueprintDoor {
    /// The Revit element id, in the model it was captured from.
    pub id: String,
    /// Index into `Outline::coarse`'s edges: the edge the door lies on.
    pub edge: usize,
    /// Distance of the door's centre along that edge from its start, mm.
    pub along_mm: f64,
    /// The edge's length at capture.
    pub edge_len_mm: f64,
    /// The door's extent along the edge, mm.
    pub width_mm: f64,
}

/// A room layout sheet a blueprint carries: the layout of the sheets to make for a room that is
/// assigned the blueprint. **A copy, independent of the sheet library** (plan decision 18), and
/// **layout only**: no tags (decision 24), and the sheet's number, name and id are kept to
/// recognise it on a re-push and are never used on assignment (the Revit side generates those
/// from the target room).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlueprintSheet {
    /// The sheet's `ElementId` in its document. With `document_title`, its identity on a re-push.
    pub revit_sheet_id: String,
    pub document_title: String,
    /// As in the source model: informational.
    pub sheet_number: String,
    #[serde(default)]
    pub sheet_name: String,
    /// The sheet as duHast exports it (viewports with sheet positions and types, schedules),
    /// opaque to the server like a library sheet's body.
    pub sheet: serde_json::Value,
}

/// Everything one revision of a blueprint holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevisionContent {
    pub outline: Outline,
    pub items: Vec<BlueprintItem>,
    /// Absent in a revision written before doors were captured, which is not "no doors".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doors: Vec<BlueprintDoor>,
    /// The room layout sheets the Revit push carried. Empty in a revision made before sheets
    /// were held, which is not "no sheets wanted": a task from it cannot create sheets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheets: Vec<BlueprintSheet>,
}

/// What capture left out, as numbers: a silence would read as "there was nothing there".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    /// Components of another instance; they move with their parent.
    pub nested: usize,
    /// No footprint, so no position the recipe could be written against.
    pub no_footprint: usize,
}
