use iotkit_core_storage::Migration;

pub mod activation;
pub mod descriptor;
pub mod mqtt;
pub mod store;
pub mod wire;

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("ledger: {0}")]
    Ledger(String),
    #[error("invalid: {0}")]
    Invalid(String),
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 10,
        label: "publish",
        sql: include_str!("../migrations/0010_publish.sql"),
    },
    Migration {
        version: 20,
        label: "site_activation",
        sql: include_str!("../migrations/0020_site_activation.sql"),
    },
];

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    pub fn open() -> rusqlite::Connection {
        let mut all: Vec<Migration> = Vec::new();
        all.extend_from_slice(iotkit_core_storage::MIGRATIONS);
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
        all.extend_from_slice(MIGRATIONS);
        all.sort_by_key(|m| m.version);
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        iotkit_core_storage::run_migrations(&conn, &all).unwrap();
        conn
    }
}

#[cfg(test)]
mod tests {
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
}
