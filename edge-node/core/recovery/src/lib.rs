//! Durable recovery state and shared migration set for IoTKit Edge Node.

use iotkit_core_storage::Migration;

mod backup;
mod config;
mod container;
mod destination;
mod model;
mod restore;
mod snapshot;
mod state;

pub use backup::{
    BEGIN_BACKUP_ATTEMPT_OP, COMPLETE_BACKUP_ATTEMPT_OP, RECORD_BACKUP_PREFLIGHT_FAILURE_OP,
    backup_status, create_backup, create_backup_from_files, inspect_backup,
};
pub use config::{
    BACKUP_PAIR_COMPLETION_NAME, BACKUP_PAIR_MARKER_NAME, BackupConfigReplace, BackupPairPhase,
    BackupPairRecord, RecoveryObservationGuard, RecoveryOperationGuard,
    acquire_recovery_observation, acquire_recovery_operation, configure_backup,
    configure_backup_guarded, configure_backup_guarded_with_pre_publish, load_owner_only_config,
    load_owner_only_handoff, load_owner_only_passphrase,
};
pub use container::{
    DecryptedStage, DirectoryCapability, authenticate_container, decrypt_container_to_staging_file,
    encrypt_container,
};
pub use destination::{
    MountInfoEntry, VerifiedBackupDestination, VerifiedStagingDirectory, apply_retention,
    parse_mountinfo, publish_verified_artifact, required_capacity, verify_destination,
    verify_staging_directory,
};
pub use model::{
    BackupConfig, BackupCounts, BackupPassphrase, BackupReadiness, BackupStatusArtifact,
    MountIdentity, NODE_BACKUP_FORMAT_VERSION, NODE_BACKUP_SUFFIX, NodeBackupManifest,
    RecoveryError, RecoveryHandoff, RecoveryStartupMode, RestoreReceipt, RestoreRequest,
    RestoreStatus, SnapshotMode,
};
pub use restore::{INSTALL_CANDIDATE_OP, restore_candidate};
#[cfg(target_os = "linux")]
pub(crate) use snapshot::validate_restored_candidate;
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

#[cfg(test)]
#[path = "../tests/unit/config_tests.rs"]
mod config_tests;

#[cfg(test)]
#[path = "../tests/unit/destination_tests.rs"]
mod destination_tests;

#[cfg(test)]
#[path = "../tests/unit/backup_tests.rs"]
mod backup_tests;

#[cfg(test)]
#[path = "../tests/unit/restore_tests.rs"]
mod restore_tests;
