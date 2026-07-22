use super::tests_support;

#[test]
fn migration_creates_tables() {
    let conn = tests_support::open();
    let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('publication_log','target_registry')",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(n, 2);
}
