//! Which items of a room an RLS tags, in which view, and which it leaves for a person to decide.
//! **A prototype** (the RLS methodology tab): the shape of the answer is what is being tried.
//!
//! **The library tags what it can.** Every item attributed to the room is looked up in the tag
//! rules the library has learned (`tag_rules`) by its type code, for the plan viewport type: the generic
//! pool's rule if it has one, else the rule learned in room blueprints, pooled across them
//! (`tag_rules::rls_item_rules`, which `task_build` calls; `rule_for` only picks the one for a view). An
//! item with a rule is tagged in the
//! plan, at the rule's offset in the item's own frame (paper millimetres times the view's scale,
//! turned with the item's facing). An item whose type has no code is left out and says why. That is the plug-in's rule for placing tags
//! (decision 24) done on the server, so a person can see the result before a task is made.
//!
//! **Everything else is offered, per view, never guessed.** `ViewTags::candidates` lists the items
//! a view shows and does not tag, and `items_without_tags` those no view tags. A person answers
//! per item (`TagChoice`): tag it in a view they name, or dismiss it (a bracket, a switch, a
//! handle: there will be items nobody wants tagged). A chosen tag with no rule gets a default head
//! beside the item and a **rectangle** round it (the item's footprint in its own frame), with the
//! text of `tag_property` -- a setting of its own, not the plan viewer's FF&E tag.
//!
//! **Elevations have rules of their own.** A plan rule's offset is in the item's frame and drops z, which
//! says nothing about where a tag sits in an ELEVATION. An elevation rule (`RuleKey::viewport_type` =
//! `elevation`) is measured along the view's right and up, in paper mm, and keyed by how the item is seen:
//! **front on** (its facing parallel to the direction the view looks, `tag_rules::view_side`) or from the
//! **side**. The server turns it into `view_offset_mm` (model mm, right and up from the item's point); the
//! plug-in adds that to the item's own point along the new view's right and up, because the item's height
//! is Revit's to read. An item with no elevation rule for the way it is seen stays a candidate, and a chosen
//! tag there is placed along the elevation line only.
//!
//! **What an elevation shows** is what lies between the ends of its line and no further than it
//! looks (`rls_plan::ViewPlan`); occlusion is not judged.
//!
//! Positions are in the target model's frame, mm.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::blueprint_geometry::{centroid, dist, dot, sub, unit, Pt};
use super::blueprint_model::Property;
use super::rls_plan::{MarkerPlan, ViewPlan};
use crate::tag_rules::{is_spatial, plan_direction, view_side, RuleSummary};

/// The paper scale assumed when a viewport does not say.
const DEFAULT_SCALE: f64 = 100.0;
/// How far beside the footprint a default tag head sits, mm in the model.
const DEFAULT_GAP_MM: f64 = 150.0;

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

/// An item's type code: the project's type-code parameter, read from the type's properties and, for
/// a model that put it on the instance, from the instance's. A blank value is no value, and a
/// blank type value does not stop a real instance one (the tiering rule, as for doors).
pub fn type_code_of(parameter: &str, type_properties: &[Property], properties: &[Property]) -> Option<String> {
    let wanted = parameter.trim();
    if wanted.is_empty() {
        return None;
    }
    [type_properties, properties]
        .iter()
        .flat_map(|tier| tier.iter())
        .find(|p| p.name.trim().eq_ignore_ascii_case(wanted) && !p.value.trim().is_empty())
        .map(|p| p.value.trim().to_string())
}

impl RoomItem {
    fn centre(&self) -> Option<Pt> {
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

fn item_ref(i: &RoomItem) -> ItemRef {
    ItemRef {
        item_id: i.id.clone(),
        family: i.family.clone(),
        type_name: i.type_name.clone(),
        category: i.category.clone(),
    }
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Whether a rule is about this item's type code. The caller has already narrowed the rules to
/// the project's type-code parameter (`tag_rules::rules_for`), so the code alone decides.
fn has_code(rule: &RuleSummary, item: &RoomItem) -> bool {
    item.type_code.as_deref().is_some_and(|c| same(&rule.key.type_code, c))
}

/// The rule that tags `item` in a viewport of type `kind` seen from `side`. `rules` are the effective ones
/// (`tag_rules::rls_item_rules`: one per type code, viewport type and view side, the generic pool's or else the
/// blueprint rules pooled), so the first that fits is the answer; which blueprint item a rule was learned on
/// no longer matters.
fn rule_for<'a>(rules: &'a [RuleSummary], kind: &str, side: &str, item: &RoomItem) -> Option<&'a RuleSummary> {
    rules.iter().find(|r| {
        !is_spatial(&r.tag_kind)
            && same(&r.key.viewport_type, kind)
            && same(&r.key.view_side, side)
            && has_code(r, item)
    })
}

/// The footprint's rectangle in the item's own frame (x along its facing), as four corners.
fn outline_of(item: &RoomItem) -> Option<[[f64; 2]; 4]> {
    let f = unit(item.facing?);
    let l = (-f.1, f.0);
    let c = item.centre()?;
    let (mut ax, mut bx, mut al, mut bl) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in &item.footprint {
        let d = sub(*p, c);
        let (x, y) = (dot(d, f), dot(d, l));
        ax = ax.min(x);
        bx = bx.max(x);
        al = al.min(y);
        bl = bl.max(y);
    }
    let at = |x: f64, y: f64| [c.0 + f.0 * x + l.0 * y, c.1 + f.1 * x + l.1 * y];
    Some([at(ax, al), at(bx, al), at(bx, bl), at(ax, bl)])
}

fn text_of(item: &RoomItem, property: Option<&str>) -> String {
    match property.map(str::trim) {
        None | Some("") | Some("$type_name") => item.type_name.clone(),
        Some("$id") => item.id.clone(),
        Some("$family_name") => item.family.clone(),
        Some(name) => item
            .properties
            .iter()
            .chain(&item.type_properties)
            .find(|p| p.name == name && !p.value.trim().is_empty())
            .map_or_else(String::new, |p| p.value.clone()),
    }
}

/// A viewport of the sheet that can show items: `(view_id, vp_type, scale)`.
fn viewports(sheet: &Value) -> Vec<(i64, String, f64)> {
    let mut out = Vec::new();
    for port in sheet.get("view_ports").and_then(Value::as_array).into_iter().flatten() {
        let Some(kind) = port.get("vp_type").and_then(Value::as_str) else {
            continue;
        };
        if kind != "floor plan" && kind != "elevation" {
            continue;
        }
        let Some(id) = port.get("view_id").and_then(Value::as_i64) else {
            continue;
        };
        let scale = port
            .get("view")
            .and_then(|v| v.get("view_scale"))
            .and_then(Value::as_f64)
            .filter(|s| *s > 0.0)
            .unwrap_or(DEFAULT_SCALE);
        out.push((id, kind.to_string(), scale));
    }
    out
}

/// Where along `view`'s line `c` lies, when the view shows it.
fn shown_in(view: &ViewPlan, c: Pt) -> Option<f64> {
    let (s, e) = ((view.line_start[0], view.line_start[1]), (view.line_end[0], view.line_end[1]));
    let width = dist(s, e);
    if width <= 0.0 {
        return None;
    }
    let (r, looks) = (unit(sub(e, s)), (view.looks[0], view.looks[1]));
    let d = sub(c, s);
    let (u, t) = (dot(d, r), dot(d, looks));
    let limit = if view.depth_mm > 0.0 { view.depth_mm } else { f64::INFINITY };
    ((0.0..=width).contains(&u) && (0.0..=limit).contains(&t)).then_some(u)
}

/// The items no view tags, with where they could be and why no rule did. `views` carries each view's
/// kind (`ViewTags::view_type`) and `elevations` the elevations' directions, so an item seen only in
/// elevations is told which way it was seen and which tag the library lacks.
fn untagged(
    items: &[RoomItem],
    views: &[ViewTags],
    elevations: &[&ViewPlan],
    rules: &[RuleSummary],
    dismissed_ids: &[&str],
) -> Vec<Untagged> {
    let mut items_without_tags = Vec::new();
    for item in items.iter().filter(|i| !dismissed_ids.contains(&i.id.as_str())) {
        let tagged_somewhere = views.iter().any(|v| v.tagged.iter().any(|t| t.item_id == item.id));
        if tagged_somewhere {
            continue;
        }
        let in_views: Vec<i64> = views
            .iter()
            .filter(|v| v.candidates.iter().any(|c| c.item_id == item.id))
            .map(|v| v.view_id)
            .collect();
        let kinds: Vec<&str> = views
            .iter()
            .filter(|v| in_views.contains(&v.view_id))
            .map(|v| v.view_type.as_str())
            .collect();
        let coded: Vec<&RuleSummary> = rules.iter().filter(|r| !is_spatial(&r.tag_kind) && has_code(r, item)).collect();
        let reason = if item.centre().is_none() {
            Some("it has no footprint".to_string())
        } else if in_views.is_empty() {
            Some("no view of the sheet shows it".to_string())
        } else if item.facing.is_none() {
            Some("it has no facing, so a rule cannot place its tag".to_string())
        } else if item.type_code.is_none() {
            Some("its type has no value for the type-code parameter, so no rule can find it".to_string())
        } else if coded.is_empty() {
            Some("the library has no tag for this type code".to_string())
        } else {
            // One reason per kind of view that shows it and has no rule for it.
            let mut why = Vec::new();
            if kinds.contains(&"floor plan") && !coded.iter().any(|r| same(&r.key.viewport_type, "floor plan")) {
                why.push("the library has no plan tag for this type code".to_string());
            }
            if kinds.contains(&"elevation") {
                let elevation_rules: Vec<&&RuleSummary> =
                    coded.iter().filter(|r| same(&r.key.viewport_type, "elevation")).collect();
                let side = item
                    .facing
                    .zip(elevations.iter().find(|v| in_views.contains(&v.view_id)))
                    .and_then(|(f, v)| plan_direction(v.looks[0], v.looks[1]).map(|l| view_side(unit(f), l)));
                match side {
                    _ if elevation_rules.is_empty() => why.push("the library has no elevation tag for this type code".to_string()),
                    Some(side) if !elevation_rules.iter().any(|r| same(&r.key.view_side, side)) => why.push(format!(
                        "the library's elevation tag for this type code is for an item seen {}, and this one is seen {}",
                        if side == "front" { "from the side" } else { "front on" },
                        if side == "front" { "front on" } else { "from the side" },
                    )),
                    _ => {}
                }
            }
            (!why.is_empty()).then(|| why.join("; "))
        };
        items_without_tags.push(Untagged { item: item_ref(item), in_views, reason });
    }
    items_without_tags
}

/// An elevation tag from a rule: the rule's offsets (paper mm along the view's right and up) at the view's scale.
fn elevation_tag(item: &RoomItem, r: &RuleSummary, scale: f64, along: Option<f64>) -> PlacedTag {
    let out = |o: crate::tag_rules::Offset| [o.x * scale, o.y * scale];
    PlacedTag {
        item_id: item.id.clone(),
        source: Source::Rule,
        rule_id: Some(r.id.clone()),
        head: None,
        rotation: Some(r.placement.rotation),
        along_line_mm: along,
        view_offset_mm: Some(out(r.placement.head)),
        from_centre: r.placement.from_centre,
        orientation: Some(r.placement.orientation.clone()).filter(|o| !o.trim().is_empty()),
        has_leader: r.placement.has_leader,
        elbow_offset_mm: r.placement.elbow.filter(|_| r.placement.has_leader).map(out),
        end_offset_mm: r.placement.end.filter(|_| r.placement.has_leader).map(out),
        text: None,
        outline: None,
    }
}

/// Work out the tags of `sheet` for `items`. `rules` are the project's tag rules for the sheet's
/// viewport types; `markers` are where the elevations went in this room.
pub fn plan_tags(
    sheet: &Value,
    items: &[RoomItem],
    markers: &[MarkerPlan],
    rules: &[RuleSummary],
    request: &TagRequest,
) -> TagPlan {
    // A nested component is a part of a family, so it is not one of the room's items unless someone tagged its
    // type: then the library has a rule for its code, and the task applies it like any other. One nobody tagged (a
    // handle, a hinge) is left out entirely, never offered as a candidate nor listed as untagged.
    let kept: Vec<RoomItem> = items
        .iter()
        .filter(|i| !i.nested || rules.iter().any(|r| !is_spatial(&r.tag_kind) && has_code(r, i)))
        .cloned()
        .collect();
    let items = &kept[..];
    let elevations: Vec<&ViewPlan> = markers.iter().flat_map(|m| &m.views).collect();
    let dismissed_ids: Vec<&str> = request
        .choices
        .iter()
        .filter(|c| c.action == ChoiceAction::Dismiss)
        .map(|c| c.item_id.as_str())
        .collect();
    let chosen = |item: &RoomItem, view: i64| {
        request
            .choices
            .iter()
            .any(|c| c.action == ChoiceAction::Tag && c.item_id == item.id && c.view_id == Some(view))
    };

    let mut views = Vec::new();
    for (view_id, kind, scale) in viewports(sheet) {
        let mut tagged = Vec::new();
        let mut candidates = Vec::new();
        for item in items.iter().filter(|i| !dismissed_ids.contains(&i.id.as_str())) {
            let Some(c) = item.centre() else { continue };
            let (shown, along) = if kind == "elevation" {
                match elevations.iter().find(|v| v.view_id == view_id).and_then(|v| shown_in(v, c)) {
                    Some(u) => (true, Some(u)),
                    None => (false, None),
                }
            } else {
                (true, None)
            };
            if !shown {
                continue;
            }
            let view = elevations.iter().find(|v| v.view_id == view_id);
            let looks = view.and_then(|v| plan_direction(v.looks[0], v.looks[1]));
            let rule = match (kind.as_str(), looks, item.facing) {
                ("floor plan", _, _) => rule_for(rules, &kind, "", item),
                ("elevation", Some(l), Some(f)) => rule_for(rules, &kind, view_side(unit(f), l), item),
                _ => None,
            };
            match (rule, item.facing) {
                (Some(r), Some(_)) if kind == "elevation" => tagged.push(elevation_tag(item, r, scale, along)),
                (Some(r), Some(f)) => {
                    let (f, l) = (unit(f), (-unit(f).1, unit(f).0));
                    let (x, y) = (r.placement.head.x * scale, r.placement.head.y * scale);
                    tagged.push(PlacedTag {
                        item_id: item.id.clone(),
                        source: Source::Rule,
                        rule_id: Some(r.id.clone()),
                        head: Some([c.0 + f.0 * x + l.0 * y, c.1 + f.1 * x + l.1 * y]),
                        rotation: Some(f.1.atan2(f.0) + r.placement.rotation),
                        along_line_mm: None,
                        view_offset_mm: None,
                        from_centre: false,
                        orientation: None,
                        has_leader: false,
                        elbow_offset_mm: None,
                        end_offset_mm: None,
                        text: None,
                        outline: None,
                    });
                }
                _ if chosen(item, view_id) => {
                    let text = text_of(item, request.tag_property.as_deref());
                    let outline = outline_of(item);
                    let head = (kind == "floor plan").then(|| {
                        // Beside the footprint, on the model's +x side, a little clear of it.
                        let right = item.footprint.iter().map(|p| p.0).fold(f64::MIN, f64::max);
                        [right + DEFAULT_GAP_MM, c.1]
                    });
                    tagged.push(PlacedTag {
                        item_id: item.id.clone(),
                        source: Source::Choice,
                        rule_id: None,
                        head,
                        rotation: None,
                        along_line_mm: along,
                        view_offset_mm: None,
                        from_centre: false,
                        orientation: None,
                        has_leader: false,
                        elbow_offset_mm: None,
                        end_offset_mm: None,
                        text: Some(text),
                        outline,
                    });
                }
                _ => candidates.push(item_ref(item)),
            }
        }
        views.push(ViewTags { view_id, view_type: kind, tagged, candidates });
    }

    let items_without_tags = untagged(items, &views, &elevations, rules, &dismissed_ids);
    let dismissed = items.iter().filter(|i| dismissed_ids.contains(&i.id.as_str())).map(item_ref).collect();
    TagPlan { views, items_without_tags, dismissed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_rules::{Offset, Placement, RuleKey};
    use serde_json::json;

    fn item(id: &str, family: &str, c: Pt, facing: Option<Pt>) -> RoomItem {
        RoomItem {
            id: id.into(),
            family: family.into(),
            type_name: format!("{family} type"),
            category: "Furniture".into(),
            type_code: Some(format!("code-{family}")),
            footprint: vec![
                (c.0 - 400.0, c.1 - 300.0),
                (c.0 + 400.0, c.1 - 300.0),
                (c.0 + 400.0, c.1 + 300.0),
                (c.0 - 400.0, c.1 + 300.0),
            ],
            facing,
            nested: false,
            properties: vec![Property { name: "Mark".into(), value: format!("M-{id}") }],
            type_properties: vec![],
        }
    }

    fn rule(family: &str, x: f64, y: f64) -> RuleSummary {
        RuleSummary {
            id: format!("tr-{family}"),
            key: RuleKey {
                tagged_category: "Furniture".into(),
                code_parameter: "Assembly Code".into(),
                type_code: format!("CODE-{family}"),
                tag_category: "Tags".into(),
                tag_family: "T".into(),
                tag_type: "T".into(),
                viewport_type: "floor plan".into(),
                blueprint: String::new(),
                instance: String::new(),
                view_side: String::new(),
            },
            tag_kind: "independent".into(),
            placement: Placement {
                head: Offset { x, y },
                rotation: 0.0,
                orientation: "Horizontal".into(),
                has_leader: false,
                leader_end_condition: String::new(),
                elbow: None,
                end: None,
                boundary: None,
                from_centre: false,
            },
            sample_count: 3,
            updated_at: String::new(),
        }
    }

    fn sheet() -> Value {
        json!({ "view_ports": [
            { "vp_type": "floor plan", "view_id": 1, "view": { "view_scale": 50 } },
            { "vp_type": "elevation", "view_id": 2, "view": {} },
            { "vp_type": "schedule", "view_id": 3, "view": {} },
        ]})
    }

    /// One elevation looking at the bottom wall (towards -y), 2 m wide, 1.5 m deep, line at y = 1000.
    fn markers() -> Vec<MarkerPlan> {
        vec![MarkerPlan {
            marker_id: 1,
            location: [2000.0, 1500.0],
            views: vec![ViewPlan {
                view_id: 2,
                marker_index: 0,
                line_start: [1000.0, 1000.0],
                line_end: [3000.0, 1000.0],
                looks: [0.0, -1.0],
                depth_mm: 1500.0,
                depth_to_wall_mm: Some(1000.0),
                faces_edge: 0,
            }],
        }]
    }

    #[test]
    fn an_item_with_a_rule_is_tagged_in_the_plan_at_the_rules_offset_turned_with_it() {
        let items = vec![item("bed", "Bed", (2000.0, 500.0), Some((0.0, 1.0)))];
        // 10 mm on paper along the facing at 1:50 is 500 mm in the model; the facing is +y.
        let plan = plan_tags(&sheet(), &items, &markers(), &[rule("Bed", 10.0, 0.0)], &TagRequest::default());
        let tag = &plan.views[0].tagged[0];
        assert_eq!(tag.source as u8, Source::Rule as u8);
        let head = tag.head.unwrap();
        assert!((head[0] - 2000.0).abs() < 1e-6 && (head[1] - 1000.0).abs() < 1e-6, "{head:?}");
        assert!(plan.items_without_tags.is_empty() || plan.items_without_tags[0].in_views == vec![2]);
    }

    #[test]
    fn a_nested_item_is_tagged_only_when_the_library_has_a_rule_for_its_type_code() {
        // House A's WC: a shower rail nested in its shower carries a tag someone placed, so its code has a rule; a
        // handle nested in a cupboard has none and must not appear at all, not even as a candidate
        let mut rail = item("rail", "Rail", (2000.0, 500.0), Some((0.0, 1.0)));
        rail.nested = true;
        let mut handle = item("handle", "Handle", (1000.0, 500.0), Some((0.0, 1.0)));
        handle.nested = true;
        let plan = plan_tags(&sheet(), &[rail, handle], &markers(), &[rule("Rail", 10.0, 0.0)], &TagRequest::default());
        let ids: Vec<&str> = plan.views.iter().flat_map(|v| v.tagged.iter().map(|t| t.item_id.as_str())).collect();
        assert_eq!(ids, ["rail"]);
        assert!(plan.views.iter().all(|v| v.candidates.iter().all(|c| c.item_id != "handle")));
        assert!(plan.items_without_tags.iter().all(|u| u.item.item_id != "handle"));
    }

    fn elevation_rule(family: &str, side: &str, right: f64, up: f64) -> RuleSummary {
        let mut r = rule(family, right, up);
        r.key.viewport_type = "elevation".into();
        r.key.view_side = side.into();
        r
    }

    #[test]
    fn an_item_seen_front_on_in_an_elevation_is_tagged_by_the_front_rule_along_the_views_right_and_up() {
        // the elevation looks towards -y, so its right is -x; the bed faces the viewer (+y), seen front on
        let items = vec![item("bed", "Bed", (2000.0, 500.0), Some((0.0, 1.0)))];
        let rules = [
            elevation_rule("Bed", "front", 4.0, 18.0),
            elevation_rule("Bed", "side", 99.0, 99.0),
        ];
        let plan = plan_tags(&sheet(), &items, &markers(), &rules, &TagRequest::default());
        let elevation = plan.views.iter().find(|v| v.view_type == "elevation").unwrap();
        let tag = &elevation.tagged[0];
        assert_eq!(tag.source, Source::Rule);
        // 4 and 18 mm of paper at the default 1:100 are 400 and 1800 mm in the model
        assert_eq!(tag.view_offset_mm, Some([400.0, 1800.0]));
        assert_eq!(tag.orientation.as_deref(), Some("Horizontal"));
        assert!(!tag.has_leader && tag.end_offset_mm.is_none());
        assert!(tag.head.is_none() && tag.along_line_mm.is_some());
        assert!(elevation.candidates.is_empty());
        // a rule learned before the centre was read is from the point, and says nothing more
        assert!(!tag.from_centre);
        let mut centred = rules.clone();
        centred[0].placement.from_centre = true;
        let plan = plan_tags(&sheet(), &items, &markers(), &centred, &TagRequest::default());
        let elevation = plan.views.iter().find(|v| v.view_type == "elevation").unwrap();
        assert!(
            elevation.tagged[0].from_centre,
            "the plug-in must be told to add the offset to the item's centre"
        );
    }

    #[test]
    fn a_rule_with_a_leader_sends_its_elbow_and_end_scaled_and_a_vertical_tag_says_so() {
        let items = vec![item("bed", "Bed", (2000.0, 500.0), Some((0.0, 1.0)))];
        let mut r = elevation_rule("Bed", "front", 4.0, 18.0);
        r.placement.has_leader = true;
        r.placement.orientation = "Vertical".into();
        r.placement.elbow = Some(Offset { x: 2.0, y: 10.0 });
        r.placement.end = Some(Offset { x: 0.0, y: 0.0 });
        let plan = plan_tags(&sheet(), &items, &markers(), &[r], &TagRequest::default());
        let tag = &plan.views.iter().find(|v| v.view_type == "elevation").unwrap().tagged[0];
        assert!(tag.has_leader);
        assert_eq!(
            (tag.orientation.as_deref(), tag.elbow_offset_mm, tag.end_offset_mm),
            (Some("Vertical"), Some([200.0, 1000.0]), Some([0.0, 0.0]))
        );
    }

    #[test]
    fn an_item_seen_from_the_side_takes_the_side_rule_and_without_one_says_which_is_missing() {
        let items = vec![item("bed", "Bed", (2000.0, 500.0), Some((1.0, 0.0)))];
        let side = plan_tags(
            &sheet(),
            &items,
            &markers(),
            &[elevation_rule("Bed", "side", 1.0, 2.0)],
            &TagRequest::default(),
        );
        assert_eq!(side.views.iter().find(|v| v.view_type == "elevation").unwrap().tagged.len(), 1);

        let wrong = plan_tags(
            &sheet(),
            &items,
            &markers(),
            &[elevation_rule("Bed", "front", 1.0, 2.0)],
            &TagRequest::default(),
        );
        let why = wrong.items_without_tags[0].reason.clone().unwrap_or_default();
        assert!(why.contains("seen front on") && why.contains("this one is seen from the side"), "{why}");

        let plan_only = plan_tags(&sheet(), &items, &markers(), &[rule("Bed", 1.0, 0.0)], &TagRequest::default());
        // the plan rule tags it in the plan, so it is not "without tags" at all
        assert!(plan_only.items_without_tags.is_empty());
    }

    #[test]
    fn an_item_seen_only_in_elevations_with_just_a_plan_rule_is_told_so() {
        // a sheet with an elevation only
        let sheet = json!({ "view_ports": [ { "vp_type": "elevation", "view_id": 2, "view": {} } ]});
        let items = vec![item("bed", "Bed", (2000.0, 500.0), Some((0.0, 1.0)))];
        let plan = plan_tags(&sheet, &items, &markers(), &[rule("Bed", 1.0, 0.0)], &TagRequest::default());
        assert_eq!(
            plan.items_without_tags[0].reason.as_deref(),
            Some("the library has no elevation tag for this type code")
        );
    }

    #[test]
    fn an_item_with_no_rule_is_a_candidate_in_every_view_that_shows_it_and_is_listed_with_why() {
        let items = vec![item("shelf", "Shelf", (2000.0, 500.0), Some((1.0, 0.0)))];
        let plan = plan_tags(&sheet(), &items, &markers(), &[], &TagRequest::default());
        assert_eq!(plan.views[0].candidates.len(), 1, "the plan shows it");
        assert_eq!(
            plan.views[1].candidates.len(),
            1,
            "so does the elevation, which looks at 0 to 1500 mm of y below 1000"
        );
        let u = &plan.items_without_tags[0];
        assert_eq!(u.in_views, vec![1, 2]);
        assert!(u.reason.as_deref().unwrap().contains("no tag for this type code"));
    }

    #[test]
    fn an_elevation_shows_only_what_lies_in_its_footprint() {
        let items = vec![
            item("in", "A", (2000.0, 500.0), Some((1.0, 0.0))),
            item("beside", "B", (5000.0, 500.0), Some((1.0, 0.0))),
            item("behind", "C", (2000.0, 1800.0), Some((1.0, 0.0))),
        ];
        let plan = plan_tags(&sheet(), &items, &markers(), &[], &TagRequest::default());
        let ids: Vec<&str> = plan.views[1].candidates.iter().map(|c| c.item_id.as_str()).collect();
        assert_eq!(ids, vec!["in"], "beside is past the line's end and behind is on the viewer's side of it");
    }

    #[test]
    fn a_chosen_tag_gets_a_default_head_a_rectangle_and_the_property_text() {
        let items = vec![item("shelf", "Shelf", (2000.0, 500.0), Some((1.0, 0.0)))];
        let request = TagRequest {
            choices: vec![TagChoice { item_id: "shelf".into(), action: ChoiceAction::Tag, view_id: Some(1) }],
            tag_property: Some("Mark".into()),
        };
        let plan = plan_tags(&sheet(), &items, &markers(), &[], &request);
        let tag = &plan.views[0].tagged[0];
        assert_eq!(tag.text.as_deref(), Some("M-shelf"));
        let o = tag.outline.unwrap();
        assert!(
            (o[0][0] - 1600.0).abs() < 1e-6 && (o[2][0] - 2400.0).abs() < 1e-6 && (o[2][1] - 800.0).abs() < 1e-6,
            "{o:?}"
        );
        assert!(tag.head.unwrap()[0] > 2400.0, "beside the footprint");
        // Tagged in the plan, so no longer without tags; still a candidate in the elevation.
        assert!(plan.items_without_tags.is_empty());
        assert_eq!(plan.views[1].candidates.len(), 1);
    }

    #[test]
    fn a_chosen_tag_in_an_elevation_is_placed_along_its_line() {
        let items = vec![item("shelf", "Shelf", (2000.0, 500.0), Some((1.0, 0.0)))];
        let request = TagRequest {
            choices: vec![TagChoice { item_id: "shelf".into(), action: ChoiceAction::Tag, view_id: Some(2) }],
            tag_property: None,
        };
        let plan = plan_tags(&sheet(), &items, &markers(), &[], &request);
        let tag = &plan.views[1].tagged[0];
        assert_eq!(tag.along_line_mm, Some(1000.0));
        assert!(tag.head.is_none());
        assert_eq!(tag.text.as_deref(), Some("Shelf type"));
    }

    #[test]
    fn a_dismissed_item_is_listed_once_and_offered_nowhere() {
        let items = vec![item("clip", "Clip", (2000.0, 500.0), Some((1.0, 0.0)))];
        let request = TagRequest {
            choices: vec![TagChoice { item_id: "clip".into(), action: ChoiceAction::Dismiss, view_id: None }],
            tag_property: None,
        };
        let plan = plan_tags(&sheet(), &items, &markers(), &[], &request);
        assert_eq!(plan.dismissed.len(), 1);
        assert!(plan.items_without_tags.is_empty());
        assert!(plan.views.iter().all(|v| v.candidates.is_empty() && v.tagged.is_empty()));
    }

    #[test]
    fn an_item_without_a_facing_cannot_take_a_rules_tag_and_says_so() {
        let items = vec![item("pot", "Bed", (2000.0, 500.0), None)];
        let plan = plan_tags(&sheet(), &items, &markers(), &[rule("Bed", 10.0, 0.0)], &TagRequest::default());
        assert!(plan.views[0].tagged.is_empty());
        assert!(plan.items_without_tags[0].reason.as_deref().unwrap().contains("no facing"));
    }

    #[test]
    fn test_an_item_with_no_type_code_is_untagged_and_says_why() {
        let mut bed = item("1", "Bed", (0.0, 0.0), Some((1.0, 0.0)));
        bed.type_code = None;
        let plan = plan_tags(&sheet(), &[bed], &[], &[rule("Bed", 1.0, 1.0)], &TagRequest::default());
        assert!(plan.views.iter().all(|v| v.tagged.is_empty()));
        let why = plan.items_without_tags[0].reason.as_deref().unwrap();
        assert!(why.contains("type-code parameter"), "{why}");
    }

    #[test]
    fn test_type_code_of_reads_the_type_then_the_instance_and_ignores_blanks() {
        let p = |n: &str, v: &str| Property { name: n.into(), value: v.into() };
        let ty = [p("Assembly Code", "B-01")];
        let inst = [p("assembly code", "X")];
        assert_eq!(type_code_of("assembly code", &ty, &inst).as_deref(), Some("B-01"));
        assert_eq!(type_code_of("Assembly Code", &[p("Assembly Code", " ")], &inst).as_deref(), Some("X"));
        assert_eq!(type_code_of("", &ty, &inst), None);
        assert_eq!(type_code_of("Nope", &ty, &inst), None);
    }

    #[test]
    fn test_an_item_takes_the_rule_for_its_type_code_whatever_blueprint_item_it_was_learned_on() {
        let bed = item("1", "Bed", (0.0, 0.0), Some((1.0, 0.0)));
        let mut learned = rule("Bed", 30.0, 0.0);
        learned.key.blueprint = "bp".into();
        learned.key.instance = "b1".into();
        let pick = |rules: &[RuleSummary]| rule_for(rules, "floor plan", "", &bed).map(|r| r.placement.head.x);
        assert_eq!(
            pick(&[learned]),
            Some(30.0),
            "a rule learned on a blueprint item tags any item of that code"
        );
        assert_eq!(pick(&[rule("Desk", 10.0, 0.0)]), None, "another type code never matches");
    }
}
