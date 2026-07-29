//! Durable recovery state and shared migration set for IoTKit Edge Node.

use iotkit_core_storage::Migration;

mod container;
mod model;
mod snapshot;
mod state;

pub use container::{
    DecryptedStage, authenticate_container, decrypt_container_to_staging_file, encrypt_container,
};
pub use model::{
    BackupConfig, BackupCounts, BackupPassphrase, BackupReadiness, BackupStatusArtifact,
    MountIdentity, NODE_BACKUP_FORMAT_VERSION, NODE_BACKUP_SUFFIX, NodeBackupManifest,
    RecoveryError, RecoveryHandoff, RecoveryStartupMode, RestoreReceipt, RestoreRequest,
    RestoreStatus, SnapshotMode,
};
pub use snapshot::{
    SnapshotArtifact, SnapshotFacts, create_consistent_snapshot, validate_snapshot,
};
pub use state::{probe_startup_path, startup_mode};

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 23,
    label: "edge_node_recovery",
    sql: include_str!("../migrations/0023_edge_node_recovery.sql"),
}];

/// Returns the complete Edge Node migration set in version order.
pub fn all_edge_node_migrations() -> Vec<Migration> {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    migrations.extend_from_slice(MIGRATIONS);
    migrations.sort_by_key(|migration| migration.version);
    debug_assert!(
        migrations
            .windows(2)
            .all(|pair| pair[0].version != pair[1].version)
    );
    migrations
}

pub use snapshot::recovery_descriptors;

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
pub(crate) mod tests_support;
