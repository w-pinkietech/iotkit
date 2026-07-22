use super::*;

#[test]
fn tier_order_matches_control_plane_escalation_order() {
    assert!(Tier::ReadOnly < Tier::Routine);
    assert!(Tier::Routine < Tier::Daily);
    assert!(Tier::Daily < Tier::Construction);
}

#[test]
fn tier_and_token_kind_round_trip_database_strings() {
    let cases = [
        (Tier::ReadOnly, "read_only"),
        (Tier::Routine, "routine"),
        (Tier::Daily, "daily"),
        (Tier::Construction, "construction"),
    ];
    for (tier, db) in cases {
        assert_eq!(tier.as_str(), db);
        assert_eq!(Tier::parse(db).unwrap(), tier);
    }
    assert!(Tier::parse("operator").is_err());

    assert_eq!(TokenKind::Human.as_str(), "human");
    assert_eq!(TokenKind::Ai.as_str(), "ai");
    assert_eq!(TokenKind::parse("human").unwrap(), TokenKind::Human);
    assert_eq!(TokenKind::parse("ai").unwrap(), TokenKind::Ai);
    assert!(TokenKind::parse("local_cli").is_err());
}
