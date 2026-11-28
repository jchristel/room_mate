use super::*;

fn policy() -> BlueprintAuditPolicy {
    BlueprintAuditPolicy::default()
}

fn level_hosting() -> Hosting {
    Hosting {
        placement_type: Some("OneLevelBased".into()),
        host_kind: None,
        face_hosted: false,
    }
}

fn item(id: &str, code: Option<&str>, at: Pt) -> Layout {
    Layout {
        id: id.to_string(),
        code: code.map(str::to_string),
        type_name: format!("type-{id}"),
        family_name: None,
        centre: at,
        facing: (1.0, 0.0),
        hosting: Some(level_hosting()),
        class: ItemClass::Free,
        kit: None,
    }
}

fn element(id: &str, code: Option<&str>, at: Pt) -> Element {
    Element {
        id: id.to_string(),
        unique_id: Some(format!("u-{id}")),
        code: code.map(str::to_string),
        type_name: format!("type-{id}"),
        family_name: None,
        centre: Some(at),
        facing: Some((1.0, 0.0)),
        hosting: level_hosting(),
        kit: None,
    }
}

fn kinds(a: &Audit) -> Vec<(VerdictKind, Option<&str>, Option<&str>)> {
    a.verdicts
        .iter()
        .map(|v| (v.kind, v.item_id.as_deref(), v.element_id.as_deref()))
        .collect()
}

fn count(a: &Audit, kind: VerdictKind) -> usize {
    a.counts.get(&kind).copied().unwrap_or(0)
}

#[test]
fn an_empty_room_misses_every_item_and_says_so() {
    let layout = [
        item("a", Some("BED"), (0.0, 0.0)),
        item("b", Some("DESK"), (1000.0, 0.0)),
    ];
    let a = audit(&layout, &[], true, &policy());
    assert_eq!(RoomState::Empty, a.state);
    assert_eq!(2, count(&a, VerdictKind::Missing));
    assert_eq!(2, a.layout_items);
    assert_eq!(0, a.elements);
}

#[test]
fn an_element_on_its_item_is_in_place_and_one_near_it_is_moved() {
    let layout = [
        item("a", Some("BED"), (0.0, 0.0)),
        item("b", Some("DESK"), (5000.0, 0.0)),
    ];
    let found = [
        element("e1", Some("BED"), (3.0, 4.0)),
        element("e2", Some("DESK"), (5300.0, 0.0)),
    ];
    let a = audit(&layout, &found, true, &policy());
    assert_eq!(RoomState::Untracked, a.state);
    assert_eq!(
        vec![
            (VerdictKind::InPlace, Some("a"), Some("e1")),
            (VerdictKind::Moved, Some("b"), Some("e2")),
        ],
        kinds(&a)
    );
    let moved = a.verdicts.iter().find(|v| v.kind == VerdictKind::Moved).unwrap();
    assert_eq!(Some(300.0), moved.distance_mm);
    assert_eq!(Some("u-e2".to_string()), moved.unique_id);
}

#[test]
fn the_tolerances_are_the_projects_not_constants() {
    let layout = [item("a", Some("BED"), (0.0, 0.0))];
    let found = [element("e", Some("BED"), (40.0, 0.0))];
    assert_eq!(
        VerdictKind::Moved,
        audit(&layout, &found, true, &policy()).verdicts[0].kind,
        "40 mm is beyond 5"
    );
    let loose = BlueprintAuditPolicy { place_tolerance_mm: 50.0, ..BlueprintAuditPolicy::default() };
    assert_eq!(VerdictKind::InPlace, audit(&layout, &found, true, &loose).verdicts[0].kind);
    let tight = BlueprintAuditPolicy { near_tolerance_mm: 30.0, ..BlueprintAuditPolicy::default() };
    assert_eq!(
        2,
        kinds(&audit(&layout, &found, true, &tight)).len(),
        "beyond near: the element and the item are separate"
    );
}

#[test]
fn facing_the_wrong_way_in_the_right_place_is_moved_with_the_turn() {
    let layout = [item("a", Some("BED"), (0.0, 0.0))];
    let mut turned = element("e", Some("BED"), (0.0, 0.0));
    turned.facing = Some((0.0, 1.0));
    let a = audit(&layout, &[turned], true, &policy());
    let v = &a.verdicts[0];
    assert_eq!(VerdictKind::Moved, v.kind);
    assert!((v.turn_deg.unwrap() - 90.0).abs() < 1e-9);
    // Within the turn tolerance it agrees.
    let mut nearly = element("e", Some("BED"), (0.0, 0.0));
    nearly.facing = Some((4.0_f64.to_radians().cos(), 4.0_f64.to_radians().sin()));
    assert_eq!(VerdictKind::InPlace, audit(&layout, &[nearly], true, &policy()).verdicts[0].kind);
    // An element that states no facing is not failed on it: nothing to disagree with.
    let mut silent = element("e", Some("BED"), (0.0, 0.0));
    silent.facing = None;
    assert_eq!(VerdictKind::InPlace, audit(&layout, &[silent], true, &policy()).verdicts[0].kind);
}

#[test]
fn the_same_code_far_away_is_not_the_item_it_is_extra_and_the_item_is_missing() {
    let layout = [item("a", Some("BED"), (0.0, 0.0))];
    let found = [element("far", Some("BED"), (4000.0, 0.0))];
    let a = audit(&layout, &found, true, &policy());
    assert_eq!(
        vec![
            (VerdictKind::Missing, Some("a"), None),
            (VerdictKind::Extra, None, Some("far"))
        ],
        kinds(&a),
        "pairing them would call a different bed the same bed"
    );
}

#[test]
fn more_elements_than_items_of_a_code_is_surplus_and_a_code_the_layout_lacks_is_foreign() {
    let layout = [item("a", Some("GUARD"), (0.0, 0.0))];
    let found = [
        element("g1", Some("GUARD"), (0.0, 0.0)),
        element("g2", Some("GUARD"), (1000.0, 0.0)),
        element("chair", Some("CHAIR"), (2000.0, 0.0)),
    ];
    let a = audit(&layout, &found, true, &policy());
    assert_eq!(1, count(&a, VerdictKind::InPlace));
    assert_eq!(1, count(&a, VerdictKind::Surplus), "two guards where the layout has one");
    assert_eq!(1, count(&a, VerdictKind::Foreign), "not in code: not the blueprint's");
    assert_eq!(0, count(&a, VerdictKind::Missing));
}

#[test]
fn another_code_standing_where_an_item_should_is_one_substitution_not_a_missing_and_a_foreign() {
    let layout = [item("a", Some("BED"), (0.0, 0.0))];
    let found = [element("e", Some("SOFA"), (2.0, 0.0))];
    let a = audit(&layout, &found, true, &policy());
    assert_eq!(vec![(VerdictKind::Substituted, Some("a"), Some("e"))], kinds(&a));
    // Not within the place tolerance: two separate findings.
    let a = audit(&layout, &[element("e", Some("SOFA"), (300.0, 0.0))], true, &policy());
    assert_eq!(1, count(&a, VerdictKind::Missing));
    assert_eq!(1, count(&a, VerdictKind::Foreign));
}

#[test]
fn two_items_of_one_code_pair_by_smallest_total_distance_not_greedy_nearest() {
    // Items at 0 and 100; elements at 60 and 160. The single nearest pair is (100, 60) at 40, which would
    // leave (0, 160) at 160 -- a total of 200. The best pairing is (0, 60) and (100, 160): 120.
    let layout = [item("a", Some("C"), (0.0, 0.0)), item("b", Some("C"), (100.0, 0.0))];
    let found = [
        element("e60", Some("C"), (60.0, 0.0)),
        element("e160", Some("C"), (160.0, 0.0)),
    ];
    let a = audit(&layout, &found, true, &policy());
    let pair = |item: &str| a.verdicts.iter().find(|v| v.item_id.as_deref() == Some(item)).unwrap().element_id.clone();
    assert_eq!(Some("e60".to_string()), pair("a"));
    assert_eq!(Some("e160".to_string()), pair("b"));
}

#[test]
fn a_swap_is_found_when_the_elements_are_the_items_the_other_way_round() {
    let layout = [
        item("left", Some("C"), (0.0, 0.0)),
        item("right", Some("C"), (1000.0, 0.0)),
    ];
    let found = [
        element("p", Some("C"), (1000.0, 0.0)),
        element("q", Some("C"), (0.0, 0.0)),
    ];
    let a = audit(&layout, &found, true, &policy());
    assert_eq!(2, count(&a, VerdictKind::InPlace), "same code, so which is which does not matter");
}

#[test]
fn more_items_than_elements_leaves_the_far_item_missing() {
    let layout = [item("a", Some("C"), (0.0, 0.0)), item("b", Some("C"), (3000.0, 0.0))];
    let a = audit(&layout, &[element("e", Some("C"), (3010.0, 0.0))], true, &policy());
    assert_eq!(
        vec![
            (VerdictKind::Moved, Some("b"), Some("e")),
            (VerdictKind::Missing, Some("a"), None)
        ],
        kinds(&a),
        "the element belongs to the item it is near, so the other is the one missing"
    );
}

#[test]
fn an_item_or_element_with_no_code_is_flagged_and_never_matched() {
    let layout = [item("a", None, (0.0, 0.0)), item("b", Some("BED"), (5000.0, 0.0))];
    let found = [
        element("nocode", None, (0.0, 0.0)),
        element("bed", Some("BED"), (5000.0, 0.0)),
    ];
    let a = audit(&layout, &found, true, &policy());
    assert_eq!(2, count(&a, VerdictKind::NoCode), "one on each side");
    assert_eq!(1, count(&a, VerdictKind::InPlace));
    assert_eq!(
        0,
        count(&a, VerdictKind::Substituted),
        "an element with no code is not a substitute either"
    );
    let flags: Vec<Side> = a
        .verdicts
        .iter()
        .filter(|v| v.kind == VerdictKind::NoCode)
        .filter_map(|v| v.side)
        .collect();
    assert!(flags.contains(&Side::Layout) && flags.contains(&Side::Room));
}

#[test]
fn supported_needs_the_placement_type_and_the_host_together() {
    let h = |p: Option<&str>, k: Option<&str>, face: bool| {
        supported(Some(&Hosting {
            placement_type: p.map(str::to_string),
            host_kind: k.map(str::to_string),
            face_hosted: face,
        }))
    };
    assert_eq!(Support::Yes, h(Some("OneLevelBased"), None, false));
    assert_eq!(Support::Yes, h(Some("TwoLevelsBased"), None, false));
    assert_eq!(
        Support::Yes,
        h(Some("WorkPlaneBased"), Some("Levels"), false),
        "a work plane that is a level"
    );
    assert_eq!(Support::Yes, h(Some("OneLevelBased"), Some("Levels"), false));
    // The cases measured on a real model that one fact alone gets wrong:
    assert_eq!(Support::No(Unsupported::FaceHosted), h(Some("WorkPlaneBased"), Some("Ceilings"), true));
    assert_eq!(Support::No(Unsupported::FaceHosted), h(Some("WorkPlaneBased"), None, true));
    assert_eq!(
        Support::No(Unsupported::Placement),
        h(Some("OneLevelBasedHosted"), None, false),
        "wall hosted, host empty"
    );
    assert_eq!(
        Support::No(Unsupported::Placement),
        h(Some("CurveBased"), None, false),
        "line based, host empty"
    );
    assert_eq!(
        Support::No(Unsupported::WorkPlaneNotOnLevel),
        h(Some("WorkPlaneBased"), None, false),
        "`<not associated>`"
    );
    assert_eq!(Support::No(Unsupported::Placement), h(Some("OneLevelBased"), Some("Floors"), false));
    // Cannot tell is not level-based:
    assert_eq!(Support::No(Unsupported::PlacementUnknown), h(None, None, false));
    assert_eq!(Support::No(Unsupported::PlacementUnknown), supported(None));
    assert_eq!(
        Support::Yes,
        h(Some("WorkPlaneBased"), Some(" levels "), false),
        "a category name is compared without case"
    );
}

#[test]
fn unsupported_items_are_flagged_on_either_side_and_never_compared_or_counted_missing() {
    let mut wall = element("socket", Some("GPO"), (0.0, 0.0));
    wall.hosting = Hosting {
        placement_type: Some("OneLevelBasedHosted".into()),
        host_kind: Some("Walls".into()),
        face_hosted: false,
    };
    let mut layout_socket = item("gpo", Some("GPO"), (0.0, 0.0));
    layout_socket.hosting = wall.hosting.clone().into();
    let a = audit(
        &[layout_socket, item("bed", Some("BED"), (3000.0, 0.0))],
        &[wall, element("b", Some("BED"), (3000.0, 0.0))],
        true,
        &policy(),
    );
    assert_eq!(2, count(&a, VerdictKind::UnsupportedHost));
    assert_eq!(0, count(&a, VerdictKind::Missing), "an item out of scope is not missing");
    assert_eq!(0, count(&a, VerdictKind::Foreign), "an element out of scope is not foreign");
    assert_eq!(1, count(&a, VerdictKind::InPlace));
    let why: Vec<Unsupported> = a.verdicts.iter().filter_map(|v| v.unsupported).collect();
    assert_eq!(vec![Unsupported::Placement, Unsupported::Placement], why);
}

#[test]
fn a_blueprint_captured_before_hosting_was_recorded_is_judged_by_size_and_says_how_much_is_assumed() {
    let mut old_free = item("bed", Some("BED"), (0.0, 0.0));
    old_free.hosting = None;
    let mut old_device = item("gpo", Some("GPO"), (500.0, 0.0));
    old_device.hosting = None;
    old_device.class = ItemClass::WallDevice;
    let a = audit(&[old_free, old_device], &[element("b", Some("BED"), (0.0, 0.0))], true, &policy());
    assert_eq!(1, a.placement_assumed, "the free item rests on an assumption, and that is counted");
    assert_eq!(1, count(&a, VerdictKind::InPlace));
    let flagged = a.verdicts.iter().find(|v| v.kind == VerdictKind::UnsupportedHost).unwrap();
    assert_eq!(Some(Unsupported::WallDevice), flagged.unsupported);
}

#[test]
fn a_room_the_blueprint_does_not_fit_is_judged_by_code_alone() {
    let layout = [
        item("a", Some("BED"), (0.0, 0.0)),
        item("b", Some("BED"), (900.0, 0.0)),
        item("c", Some("DESK"), (0.0, 0.0)),
    ];
    let found = [
        element("e1", Some("BED"), (9999.0, 9999.0)),
        element("e2", Some("CHAIR"), (0.0, 0.0)),
    ];
    let a = audit(&layout, &found, false, &policy());
    assert_eq!(RoomState::CannotJudge, a.state);
    assert_eq!(1, count(&a, VerdictKind::PresentByCode));
    assert_eq!(2, count(&a, VerdictKind::Missing), "a second bed and the desk");
    assert_eq!(1, count(&a, VerdictKind::Foreign));
    assert!(a.verdicts.iter().all(|v| v.distance_mm.is_none()), "no positions, so no distances");
}

#[test]
fn the_same_room_gives_the_same_answer_in_the_same_order() {
    let layout = [item("b", Some("C"), (1000.0, 0.0)), item("a", Some("C"), (0.0, 0.0))];
    let found = [
        element("y", Some("C"), (1000.0, 0.0)),
        element("x", Some("C"), (0.0, 0.0)),
    ];
    let first = serde_json::to_string(&audit(&layout, &found, true, &policy())).unwrap();
    let again = serde_json::to_string(&audit(&layout, &found, true, &policy())).unwrap();
    assert_eq!(first, again);
}

#[test]
fn an_element_with_no_position_is_never_paired() {
    let layout = [item("a", Some("C"), (0.0, 0.0))];
    let mut lost = element("e", Some("C"), (0.0, 0.0));
    lost.centre = None;
    let a = audit(&layout, &[lost], true, &policy());
    assert_eq!(1, count(&a, VerdictKind::Missing));
    assert_eq!(1, count(&a, VerdictKind::Extra));
}

fn part(depth: u32, family: &str, type_name: &str) -> KitPart {
    KitPart {
        depth,
        family_name: Some(family.to_string()),
        type_name: Some(type_name.to_string()),
    }
}

fn with_kit(mut l: Layout, kit: Option<Vec<KitPart>>) -> Layout {
    l.kit = kit;
    l
}

fn with_element_kit(mut e: Element, kit: Option<Vec<KitPart>>) -> Element {
    e.kit = kit;
    e
}

#[test]
fn the_same_parts_are_the_same_kit_and_the_verdict_stays_in_place() {
    let kit = vec![part(1, "Handle", "H1"), part(1, "Handle", "H1"), part(2, "Screw", "S")];
    let a = audit(
        &[with_kit(item("a", Some("C"), (0.0, 0.0)), Some(kit.clone()))],
        &[with_element_kit(element("e", Some("C"), (0.0, 0.0)), Some(kit))],
        true,
        &policy(),
    );
    assert_eq!(VerdictKind::InPlace, a.verdicts[0].kind);
    assert_eq!(Some(KitState::Same), a.verdicts[0].kit);
    assert_eq!(0, a.kit_unknown);
}

#[test]
fn right_place_different_parts_is_kit_differs_never_in_place_and_names_the_parts() {
    // Two handles against one, and a part one level deeper than it should be.
    let want = vec![part(1, "Handle", "H1"), part(1, "Handle", "H1")];
    let have = vec![part(1, "Handle", "H1"), part(2, "Handle", "H1")];
    let a = audit(
        &[with_kit(item("a", Some("C"), (0.0, 0.0)), Some(want))],
        &[with_element_kit(element("e", Some("C"), (0.0, 0.0)), Some(have))],
        true,
        &policy(),
    );
    let v = &a.verdicts[0];
    assert_eq!(VerdictKind::KitDiffers, v.kind);
    let diff = v.kit_diff.as_ref().unwrap();
    assert_eq!(vec![part(1, "Handle", "H1")], diff.missing, "one handle short at depth 1");
    assert_eq!(
        vec![part(2, "Handle", "H1")],
        diff.extra,
        "and one at depth 2 that the blueprint does not have"
    );
    assert_eq!(1, count(&a, VerdictKind::KitDiffers));
    assert_eq!(0, count(&a, VerdictKind::InPlace));
}

#[test]
fn a_kit_that_one_side_cannot_state_is_unknown_never_agreement() {
    for (want, have) in [
        (None, Some(vec![])),
        (Some(vec![]), None),
        (None, None),
        (Some(vec![part(1, "H", "1")]), None),
    ] {
        let a = audit(
            &[with_kit(item("a", Some("C"), (0.0, 0.0)), want)],
            &[with_element_kit(element("e", Some("C"), (0.0, 0.0)), have)],
            true,
            &policy(),
        );
        assert_eq!(VerdictKind::InPlace, a.verdicts[0].kind, "no evidence is not a difference");
        assert_eq!(Some(KitState::Unknown), a.verdicts[0].kit);
        assert_eq!(1, a.kit_unknown);
    }
}

#[test]
fn a_family_with_no_parts_on_both_sides_is_the_same_kit() {
    let a = audit(
        &[with_kit(item("a", Some("C"), (0.0, 0.0)), Some(vec![]))],
        &[with_element_kit(element("e", Some("C"), (0.0, 0.0)), Some(vec![]))],
        true,
        &policy(),
    );
    assert_eq!(Some(KitState::Same), a.verdicts[0].kit);
}

#[test]
fn a_moved_host_keeps_its_verdict_and_carries_the_kit_difference_beside_it() {
    let a = audit(
        &[with_kit(
            item("a", Some("C"), (0.0, 0.0)),
            Some(vec![part(1, "H", "1")]),
        )],
        &[with_element_kit(element("e", Some("C"), (300.0, 0.0)), Some(vec![]))],
        true,
        &policy(),
    );
    assert_eq!(VerdictKind::Moved, a.verdicts[0].kind, "where it is is the bigger finding");
    assert_eq!(Some(KitState::Differs), a.verdicts[0].kit);
}

fn host_item(parts: &[(&str, &str, &str, &str)], placement: Option<&str>) -> crate::contract::Item {
    let sub: Vec<serde_json::Value> = parts
        .iter()
        .map(|(id, parent, family, type_name)| {
            serde_json::json!({ "id": id, "parent_id": parent, "family_name": family, "type_name": type_name })
        })
        .collect();
    serde_json::from_value(serde_json::json!({
        "id": "host", "level_id": "1", "category": "Casework", "type_id": "t", "type_name": "Cupboard",
        "placement_type": placement, "sub_components": sub,
    }))
    .unwrap()
}

#[test]
fn a_kit_is_read_by_depth_from_the_parent_chain_and_sorted() {
    use crate::service::blueprint_capture::kit_of;
    let item = host_item(
        &[
            ("3", "2", "Screw", "S"),
            ("1", "host", "Handle", "H1"),
            ("2", "1", "Cap", "C"),
            ("4", "host", "Handle", "H1"),
        ],
        Some("OneLevelBased"),
    );
    assert_eq!(
        Some(vec![
            part(1, "Handle", "H1"),
            part(1, "Handle", "H1"),
            part(2, "Cap", "C"),
            part(3, "Screw", "S")
        ]),
        kit_of(&item),
        "a direct child is depth 1, a part nested in it 2, and so on; element ids are not part of the kit"
    );
}

#[test]
fn an_item_from_a_snapshot_that_cannot_say_has_no_recorded_kit_not_an_empty_one() {
    use crate::service::blueprint_capture::kit_of;
    assert_eq!(
        None,
        kit_of(&host_item(&[], None)),
        "no placement type: exported before nesting was read, so unknown"
    );
    assert_eq!(
        Some(vec![]),
        kit_of(&host_item(&[], Some("OneLevelBased"))),
        "a newer export with no parts has none"
    );
}

#[test]
fn a_cycle_in_a_malformed_kit_does_not_spin() {
    use crate::service::blueprint_capture::kit_of;
    let item = host_item(&[("1", "2", "A", "a"), ("2", "1", "B", "b")], Some("OneLevelBased"));
    assert_eq!(2, kit_of(&item).unwrap().len(), "read, bounded, not hung");
}
