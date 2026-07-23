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
        label: "edge_node_activation",
        sql: include_str!("../migrations/0020_edge_node_activation.sql"),
    },
];

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
pub(crate) mod tests_support;

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod tests;
