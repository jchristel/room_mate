use super::*;

/// Without a ledger nothing is known to be the blueprint's, so the only things planned are the two that need no
/// such knowledge: a layout item nothing is (`add`) and the right element a little out of place (`move`).
#[test]
fn test_only_a_missing_item_and_a_moved_element_are_planned_for_action() {
    use VerdictKind::*;
    assert_eq!(action_of(Missing), Some(Action::Add));
    assert_eq!(action_of(Moved), Some(Action::Move));
    for kind in [Extra, Surplus, Foreign, Substituted, KitDiffers] {
        assert_eq!(action_of(kind), Some(Action::Report), "{kind:?} is looked at, never acted on");
    }
    for kind in [InPlace, PresentByCode, NoCode, UnsupportedHost] {
        assert_eq!(action_of(kind), None, "{kind:?} asks for nothing");
    }
}

/// A room is listed worst first, so a person reads what needs them before what is done.
#[test]
fn test_rooms_sort_with_what_needs_a_person_first() {
    assert!(RoomStatus::Error < RoomStatus::CannotJudge);
    assert!(RoomStatus::CannotJudge < RoomStatus::NeedsDecision);
    assert!(RoomStatus::NeedsDecision < RoomStatus::Changes);
    assert!(RoomStatus::Changes < RoomStatus::NoChange);
}
