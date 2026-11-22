//! What a task's tags are made of: the room's items (`RoomItem`), a person's choice per item, and the plan the server worked out
//! (`TagPlan`: tags placed, candidates, and items no rule tags). Wire types; the rules are applied in `mod.rs`.

use super::*;

/// One item of the room, in the target model's frame.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomItem {
    pub id: String,
    pub family: String,
    pub type_name: String,
    pub category: String,
    /// The value of the project's type-code parameter for this item's type (`type_code_of`).
    /// `None`: the type has none, so no rule can find the item and it is left untagged.
    pub type_code: Option<String>,
    pub footprint: Vec<Pt>,
    pub facing: Option<Pt>,
    /// A component of another family instance (a shower rail in its shower). Tagged only when the library has a
    /// rule for its type code, and never offered as a candidate: see `plan_tags`.
    pub nested: bool,
    pub properties: Vec<Property>,
    pub type_properties: Vec<Property>,
}

impl RoomItem {
    pub(in crate::service) fn centre(&self) -> Option<Pt> {
        (!self.footprint.is_empty()).then(|| centroid(&self.footprint))
    }
}

/// What a person decided about one item.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TagChoice {
    pub item_id: String,
    pub action: ChoiceAction,
    /// The view to tag it in (a `tag`).
    #[serde(default)]
    pub view_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceAction {
    Tag,
    Dismiss,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct TagRequest {
    #[serde(default)]
    pub choices: Vec<TagChoice>,
    /// What a tag without a rule says: `$id`, `$type_name`, `$family_name`, or a property name
    /// (instance, then type). Absent: the type name.
    #[serde(default)]
    pub tag_property: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// From a rule the library learned.
    Rule,
    /// A person chose it; the head is a default.
    Choice,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedTag {
    pub item_id: String,
    pub source: Source,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    /// Plan views: where the head goes. Absent in an elevation, which only knows `along_line_mm`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<[f64; 2]>,
    /// Radians, in the plan frame, for a rule's tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    /// Elevations: the distance along the elevation line, from its start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub along_line_mm: Option<f64>,
    /// Elevations, from a rule: where the head goes from the item's point, model mm, along the view's right
    /// then up. The plug-in adds it to the item's point (it reads the height).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_offset_mm: Option<[f64; 2]>,
    /// Elevations: the offsets are from the middle of the item's bounding box, not its insertion point (the rule's
    /// samples were measured that way). Absent: from the point, as every rule learned before the centre was read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_centre: bool,
    /// Elevations, from a rule: `Horizontal` or `Vertical` (an elevation tag's turn is its orientation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    /// Elevations, from a rule that has a leader: where it bends and ends, as `view_offset_mm`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_leader: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elbow_offset_mm: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_offset_mm: Option<[f64; 2]>,
    /// What a tag without a rule says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The rectangle drawn round the item, in the item's own frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<[[f64; 2]; 4]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemRef {
    pub item_id: String,
    pub family: String,
    pub type_name: String,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewTags {
    pub view_id: i64,
    /// `floor plan` or `elevation`.
    pub view_type: String,
    pub tagged: Vec<PlacedTag>,
    /// Items this view shows and does not tag: what a person can choose to tag here.
    pub candidates: Vec<ItemRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Untagged {
    #[serde(flatten)]
    pub item: ItemRef,
    /// The views that show it, so a person knows where it could be tagged. Empty: no view shows it.
    pub in_views: Vec<i64>,
    /// Why no rule tagged it, when that is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TagPlan {
    pub views: Vec<ViewTags>,
    pub items_without_tags: Vec<Untagged>,
    pub dismissed: Vec<ItemRef>,
}
