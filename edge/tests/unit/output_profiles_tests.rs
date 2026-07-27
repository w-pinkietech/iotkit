use super::delivery_state;

#[test]
fn pending_for_299_999_milliseconds_remains_neutral_delivery() {
    assert_eq!(delivery_state(1, 0, Some(700_001), 1_000_000), "delivering");
}

#[test]
fn pending_for_300_000_milliseconds_is_a_possible_delivery_stall() {
    assert_eq!(
        delivery_state(1, 0, Some(700_000), 1_000_000),
        "possible_delivery_stall"
    );
}
