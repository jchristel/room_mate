use super::*;
use crate::service::blueprint_capture::{capture, SourceItem};
use crate::service::blueprint_geometry::Pt;
use crate::service::blueprint_model::KitPart;

fn rect(w: f64, h: f64) -> Vec<Pt> {
    vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
}

fn src(id: &str, type_name: &str, c: Pt) -> SourceItem {
    let (w, d) = (800.0, 800.0);
    SourceItem {
        id: id.to_string(),
        family_name: Some("Chair".to_string()),
        type_name: type_name.to_string(),
        category: "Furniture".to_string(),
        footprint: vec![
            (c.0 + d / 2.0, c.1 + w / 2.0),
            (c.0 - d / 2.0, c.1 + w / 2.0),
            (c.0 - d / 2.0, c.1 - w / 2.0),
            (c.0 + d / 2.0, c.1 - w / 2.0),
        ],
        facing: Some((0.0, 1.0)),
        nested: false,
        hosting: None,
        kit: None,
        properties: vec![],
        type_properties: vec![],
    }
}

fn content(items: Vec<SourceItem>) -> RevisionContent {
    capture(&rect(6000.0, 4000.0), items).unwrap().content
}

fn find<'a>(d: &'a RevisionDiff, id: &str) -> &'a ItemDiff {
    d.items
        .iter()
        .find(|i| i.item_id == id)
        .unwrap_or_else(|| panic!("no {id} in {:?}", d.items))
}

#[test]
fn test_two_identical_revisions_change_nothing() {
    let a = content(vec![src("1", "A", (1000.0, 1000.0)), src("2", "A", (3000.0, 2000.0))]);
    let d = diff(&a, &a.clone(), 600.0);
    assert_eq!(d.changed().count(), 0);
    assert_eq!(d.counts.get("unchanged"), Some(&2));
    assert!(!d.outline.changed);
    assert_eq!(d.matched_by_position, 0);
}

#[test]
fn test_a_recipe_that_moved_is_moved_and_says_how_far() {
    let old = content(vec![src("1", "A", (1000.0, 1000.0))]);
    let mut new = old.clone();
    new.items[0].placement.along_mm += 250.0;
    let d = diff(&old, &new, 600.0);
    let item = find(&d, "1");
    assert_eq!(item.changes, vec![ChangeKind::Moved]);
    assert!((item.moved_mm.unwrap() - 250.0).abs() < 1.0, "{:?}", item.moved_mm);
    assert_eq!(item.matched, Some(Matched::ById));
}

#[test]
fn test_a_type_family_or_property_change_is_named_and_a_move_beside_it_too() {
    let old = content(vec![src("1", "A", (1000.0, 1000.0)), src("2", "A", (3000.0, 2000.0))]);
    let mut new = old.clone();
    new.items[0].type_name = "B".to_string();
    new.items[1].family_name = Some("Stool".to_string());
    new.items[1].placement.off_mm += 100.0;
    new.items[1]
        .properties
        .push(crate::service::blueprint_model::Property { name: "Finish".into(), value: "Oak".into() });
    let d = diff(&old, &new, 600.0);
    assert_eq!(find(&d, "1").changes, vec![ChangeKind::TypeChanged]);
    assert_eq!(
        find(&d, "2").changes,
        vec![
            ChangeKind::Moved,
            ChangeKind::TypeChanged,
            ChangeKind::PropertiesChanged
        ]
    );
}

#[test]
fn test_an_item_new_to_the_revision_is_added_and_one_gone_is_retired() {
    let old = content(vec![src("1", "A", (1000.0, 1000.0)), src("2", "A", (5000.0, 3000.0))]);
    let new = content(vec![src("1", "A", (1000.0, 1000.0)), src("3", "Z", (3000.0, 2000.0))]);
    let d = diff(&old, &new, 600.0);
    assert_eq!(find(&d, "2").changes, vec![ChangeKind::Retired]);
    assert_eq!(find(&d, "3").changes, vec![ChangeKind::Added]);
    assert!(find(&d, "1").changes.is_empty());
}

#[test]
fn test_excluding_an_item_retires_it_and_bringing_it_back_adds_it() {
    let old = content(vec![src("1", "A", (1000.0, 1000.0)), src("2", "A", (3000.0, 2000.0))]);
    let mut new = old.clone();
    new.items[0].excluded = true;
    let d = diff(&old, &new, 600.0);
    assert_eq!(find(&d, "1").changes, vec![ChangeKind::Retired]);

    let back = diff(&new, &old, 600.0);
    assert_eq!(find(&back, "1").changes, vec![ChangeKind::Added]);

    // excluded in both is not in the blueprint in either
    let mut both = new.clone();
    both.items[0].excluded = true;
    assert!(diff(&new, &both, 600.0).items.iter().all(|i| i.item_id != "1"));
}

#[test]
fn test_a_recapture_with_new_ids_is_paired_by_place_and_says_so() {
    // Three chairs of one type in a row: each must pair with the one beside it, not all with the first.
    let old = content(vec![
        src("1", "A", (1000.0, 1000.0)),
        src("2", "A", (2000.0, 1000.0)),
        src("3", "A", (3000.0, 1000.0)),
    ]);
    let new = content(vec![
        src("11", "A", (1000.0, 1000.0)),
        src("12", "A", (2000.0, 1000.0)),
        src("13", "A", (3000.0, 1000.0)),
    ]);
    let d = diff(&old, &new, 600.0);
    assert_eq!(d.matched_by_position, 3);
    assert_eq!(d.changed().count(), 0, "{:?}", d.items);
    let item = find(&d, "12");
    assert_eq!((item.old_item_id.as_deref(), item.matched), (Some("2"), Some(Matched::ByPosition)));
}

#[test]
fn test_an_item_too_far_or_of_another_type_is_not_paired_by_place() {
    let old = content(vec![src("1", "A", (1000.0, 1000.0))]);
    let far = content(vec![src("9", "A", (4000.0, 3000.0))]);
    let d = diff(&old, &far, 600.0);
    assert_eq!(find(&d, "1").changes, vec![ChangeKind::Retired]);
    assert_eq!(find(&d, "9").changes, vec![ChangeKind::Added]);
    let other = content(vec![src("9", "B", (1000.0, 1000.0))]);
    assert_eq!(diff(&old, &other, 600.0).matched_by_position, 0);
}

fn kit(family: &str) -> Option<Vec<KitPart>> {
    Some(vec![KitPart { depth: 1, family_name: Some(family.to_string()), type_name: None }])
}

#[test]
fn test_a_kit_changes_only_when_both_revisions_recorded_one() {
    let old = content(vec![src("1", "A", (1000.0, 1000.0))]);
    let mut recorded = old.clone();
    recorded.items[0].kit = kit("Bracket");
    // an older revision that could not say, against a newer one that can: nothing changed on the item
    assert!(diff(&old, &recorded, 600.0).changed().next().is_none());
    let mut other = recorded.clone();
    other.items[0].kit = kit("Hinge");
    assert_eq!(find(&diff(&recorded, &other, 600.0), "1").changes, vec![ChangeKind::KitChanged]);
}

#[test]
fn test_a_redrawn_outline_is_reported_beside_the_items() {
    let old = content(vec![src("1", "A", (1000.0, 1000.0))]);
    let mut new = old.clone();
    new.outline.coarse[2].x += 50.0;
    let d = diff(&old, &new, 600.0);
    assert!(d.outline.changed);
    assert_eq!((d.outline.old_edges, d.outline.new_edges), (4, 4));
    new.outline.coarse.pop();
    assert_eq!(diff(&old, &new, 600.0).outline.new_edges, 3);
}
