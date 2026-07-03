//! iotkit-core-registry: D6測定レジストリ(標準語彙カタログ+現場レジストリ)。
//! 正本文書: docs/redesign/decisions/D6-measurement-registry.md
pub mod catalog;

pub use catalog::{Catalog, CatalogEntry, ChannelMode, Range, ValueType, standard_catalog};

use iotkit_core_storage::Migration;

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 6,
    label: "registry",
    sql: include_str!("../migrations/0006_registry.sql"),
}];

#[cfg(test)]
mod migration_tests {
    #[test]
    fn ledger_and_registry_migrations_apply() {
        // ledger+registry連結(1,3,5,6——集合差ベースのrunnerは番号の飛びを許容する)。
        // timeseriesを含むゲートウェイ完全連結(1..6)の検証はTask 6のE2Eが担う。
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS); // 3, 5
        all.extend_from_slice(crate::MIGRATIONS); // 6
        all.sort_by_key(|m| m.version);
        let db = iotkit_core_storage::init_db_memory(&all).unwrap();
        db.with_conn_sync(|conn| {
            for t in ["registry_entries", "registry_aliases", "legacy_sensor_type_map"] {
                let exists: bool = conn.query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
                    [t], |r| r.get(0),
                ).unwrap();
                assert!(exists, "{t} must exist");
            }
            // series.quarantine_reason列(v5)
            let has_col: bool = conn.query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('series') WHERE name='quarantine_reason'",
                [], |r| r.get(0),
            ).unwrap();
            assert!(has_col, "series.quarantine_reason must exist");
            Ok(())
        }).unwrap();
    }
}
