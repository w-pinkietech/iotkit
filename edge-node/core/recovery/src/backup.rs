use std::path::Path;

#[cfg(target_os = "linux")]
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString},
    fs::{File, OpenOptions},
    os::{
        fd::FromRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
};

#[cfg(target_os = "linux")]
use iotkit_core_ops::{Actor, ActorKind, DispatchRequest, dispatch};
use iotkit_core_ops::{OpContext, OpDescriptor, OpError, Tier};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(target_os = "linux")]
use crate::destination::{
    verify_destination_for_reconciliation, verify_staging_directory_for_reconciliation,
};
use crate::{
    BackupConfig, BackupPassphrase, BackupReadiness, BackupStatusArtifact, NodeBackupManifest,
    RecoveryError, acquire_recovery_observation, load_owner_only_config, startup_mode,
};
#[cfg(target_os = "linux")]
use crate::{
    RecoveryStartupMode, VerifiedBackupDestination, VerifiedStagingDirectory,
    acquire_recovery_operation, create_consistent_snapshot, encrypt_container, verify_destination,
    verify_staging_directory,
};

type CompletionRow = (
    String,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    i64,
);

type LatestAttemptRow = (
    String,
    String,
    String,
    Option<String>,
    Option<i64>,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    i64,
    Option<i64>,
    Option<i64>,
);

type PreflightRow = (String, String, String, String, String, String, i64, i64);

pub const BEGIN_BACKUP_ATTEMPT_OP: &str = "recovery.backup.begin";
pub const COMPLETE_BACKUP_ATTEMPT_OP: &str = "recovery.backup.complete";
pub const RECORD_BACKUP_PREFLIGHT_FAILURE_OP: &str = "recovery.backup.record_preflight_failure";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BeginState {
    attempt_id: String,
    backup_id: String,
    artifact_name: String,
    edge_node_id: String,
    started_at_ms: i64,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteState {
    attempt_id: String,
    outcome: String,
    reason_code: String,
    artifact_length: Option<i64>,
    ledger_epoch: Option<String>,
    accepted_cursor: Option<i64>,
    allocation_high_water: Option<i64>,
    artifact_created_at_ms: Option<i64>,
    completed_at_ms: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreflightState {
    attempt_id: String,
    backup_id: String,
    artifact_name: String,
    edge_node_id: String,
    reason_code: String,
    started_at_ms: i64,
    completed_at_ms: i64,
}

pub(crate) fn backup_descriptors() -> Vec<OpDescriptor> {
    vec![
        descriptor(BEGIN_BACKUP_ATTEMPT_OP, begin_preconditions, begin_execute),
        descriptor(
            COMPLETE_BACKUP_ATTEMPT_OP,
            complete_preconditions,
            complete_execute,
        ),
        descriptor(
            RECORD_BACKUP_PREFLIGHT_FAILURE_OP,
            preflight_preconditions,
            preflight_execute,
        ),
    ]
}

fn descriptor(
    name: &'static str,
    preconditions: fn(&Transaction<'_>, &OpContext<'_>) -> Result<(), OpError>,
    execute: fn(&Transaction<'_>, &OpContext<'_>) -> Result<Value, OpError>,
) -> OpDescriptor {
    OpDescriptor {
        name,
        tier: Tier::Construction,
        bulk_escalates: false,
        changes_state: true,
        params_schema: private_schema,
        targets: |_| Vec::new(),
        preconditions,
        dry_run: |_, _| Ok(json!({"would": "update_backup_receipt"})),
        execute,
        secret_execute: None,
    }
}

fn private_schema() -> Value {
    json!({"required": ["private_recovery_state"]})
}

fn private_state<T: for<'de> Deserialize<'de>>(context: &OpContext<'_>) -> Result<T, OpError> {
    serde_json::from_value(
        context
            .params
            .get("private_recovery_state")
            .cloned()
            .ok_or_else(|| OpError::Validation("private_recovery_state".into()))?,
    )
    .map_err(|_| OpError::Validation("private_recovery_state".into()))
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 255 && !value.chars().any(char::is_control)
}

fn valid_artifact_name(value: &str) -> bool {
    valid_identity(value)
        && value.ends_with(crate::NODE_BACKUP_SUFFIX)
        && !value.contains('/')
        && !value.contains('\\')
        && value != "."
        && value != ".."
}

pub(crate) fn valid_backup_failure_reason(value: &str) -> bool {
    matches!(
        value,
        "invalid_startup_state"
            | "snapshot_invalid"
            | "storage"
            | "container_invalid"
            | "authentication_failed"
            | "manifest_invalid"
            | "destination_exists"
            | "cryptography"
            | "random"
            | "passphrase_invalid"
            | "storage_full"
            | "platform_unsupported"
            | "artifact_publication_uncertain"
            | "artifact_cleanup_failed"
            | "invalid_configuration"
            | "mount_missing"
            | "mount_identity_unavailable"
            | "destination_invalid"
            | "capacity_overflow"
            | "operation_busy"
            | "cleanup_required"
            | "interrupted"
    )
}

fn begin_preconditions(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<(), OpError> {
    let state: BeginState = private_state(context)?;
    if !valid_identity(&state.attempt_id)
        || !valid_identity(&state.backup_id)
        || !valid_artifact_name(&state.artifact_name)
        || !valid_identity(&state.edge_node_id)
        || state.started_at_ms < 0
    {
        return Err(OpError::Validation("backup_attempt".into()));
    }
    let existing: Option<(String, String, String, String, String, i64)> = tx
        .query_row(
            "SELECT attempt_id, backup_id, state, artifact_name, edge_node_id, started_at_ms
             FROM edge_node_backup_attempts
             WHERE attempt_id=?1 OR backup_id=?2 OR artifact_name=?3",
            params![state.attempt_id, state.backup_id, state.artifact_name],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    if existing.as_ref().is_some_and(|existing| {
        existing
            != &(
                state.attempt_id,
                state.backup_id,
                "started".into(),
                state.artifact_name,
                state.edge_node_id,
                state.started_at_ms,
            )
    }) {
        return Err(OpError::PreconditionFailed(
            "backup_attempt_conflict".into(),
        ));
    }
    Ok(())
}

fn begin_execute(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<Value, OpError> {
    let state: BeginState = private_state(context)?;
    tx.execute(
        "INSERT OR IGNORE INTO edge_node_backup_attempts(
             attempt_id, backup_id, state, artifact_name, edge_node_id, started_at_ms
         ) VALUES(?1, ?2, 'started', ?3, ?4, ?5)",
        params![
            state.attempt_id,
            state.backup_id,
            state.artifact_name,
            state.edge_node_id,
            state.started_at_ms
        ],
    )?;
    Ok(json!({"state": "started"}))
}

fn complete_preconditions(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<(), OpError> {
    let state: CompleteState = private_state(context)?;
    if !valid_identity(&state.attempt_id)
        || !valid_identity(&state.reason_code)
        || state.completed_at_ms < 0
        || !matches!(state.outcome.as_str(), "success" | "failed")
    {
        return Err(OpError::Validation("backup_completion".into()));
    }
    let existing: Option<CompletionRow> = tx
        .query_row(
            "SELECT state, reason_code, artifact_length, ledger_epoch, accepted_cursor,
                    allocation_high_water, artifact_created_at_ms, completed_at_ms,
                    started_at_ms
             FROM edge_node_backup_attempts WHERE attempt_id=?1",
            [state.attempt_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .optional()?;
    let Some(existing) = existing else {
        return Err(OpError::PreconditionFailed("backup_attempt_missing".into()));
    };
    if state.completed_at_ms < existing.8 {
        return Err(OpError::PreconditionFailed("backup_attempt_missing".into()));
    }
    let success = state.outcome == "success";
    let success_fields = state.artifact_length.is_some_and(|value| value >= 0)
        && state.ledger_epoch.as_deref().is_some_and(valid_identity)
        && state.accepted_cursor.is_some_and(|value| value >= 0)
        && state.allocation_high_water.is_some_and(|value| value >= 0)
        && state.artifact_created_at_ms.is_some_and(|value| value >= 0);
    if (success && (state.reason_code != "ok" || !success_fields))
        || (!success
            && (!valid_backup_failure_reason(&state.reason_code)
                || state.artifact_length.is_some()
                || state.ledger_epoch.is_some()
                || state.accepted_cursor.is_some()
                || state.allocation_high_water.is_some()
                || state.artifact_created_at_ms.is_some()))
    {
        return Err(OpError::Validation("backup_completion".into()));
    }
    if existing.0 != "started"
        && (existing.0 != state.outcome
            || existing.1.as_deref() != Some(state.reason_code.as_str())
            || existing.2 != state.artifact_length
            || existing.3 != state.ledger_epoch
            || existing.4 != state.accepted_cursor
            || existing.5 != state.allocation_high_water
            || existing.6 != state.artifact_created_at_ms
            || existing.7 != Some(state.completed_at_ms))
    {
        return Err(OpError::PreconditionFailed(
            "backup_attempt_terminal".into(),
        ));
    }
    Ok(())
}

fn complete_execute(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<Value, OpError> {
    let state: CompleteState = private_state(context)?;
    let changed = tx.execute(
        "UPDATE edge_node_backup_attempts
         SET state=?2, reason_code=?3, artifact_length=?4, ledger_epoch=?5,
             accepted_cursor=?6, allocation_high_water=?7, artifact_created_at_ms=?8,
             completed_at_ms=?9
         WHERE attempt_id=?1 AND state='started'",
        params![
            state.attempt_id,
            state.outcome,
            state.reason_code,
            state.artifact_length,
            state.ledger_epoch,
            state.accepted_cursor,
            state.allocation_high_water,
            state.artifact_created_at_ms,
            state.completed_at_ms
        ],
    )?;
    if changed == 0 {
        let exact: bool = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM edge_node_backup_attempts
                WHERE attempt_id=?1 AND state=?2 AND reason_code=?3
                  AND artifact_length IS ?4 AND ledger_epoch IS ?5
                  AND accepted_cursor IS ?6 AND allocation_high_water IS ?7
                  AND artifact_created_at_ms IS ?8 AND completed_at_ms=?9
            )",
            params![
                state.attempt_id,
                state.outcome,
                state.reason_code,
                state.artifact_length,
                state.ledger_epoch,
                state.accepted_cursor,
                state.allocation_high_water,
                state.artifact_created_at_ms,
                state.completed_at_ms
            ],
            |row| row.get(0),
        )?;
        if !exact {
            return Err(OpError::PreconditionFailed(
                "backup_attempt_terminal".into(),
            ));
        }
    }
    Ok(json!({"state": state.outcome}))
}

fn preflight_preconditions(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<(), OpError> {
    let state: PreflightState = private_state(context)?;
    if !valid_identity(&state.attempt_id)
        || !valid_identity(&state.backup_id)
        || !valid_artifact_name(&state.artifact_name)
        || !valid_identity(&state.edge_node_id)
        || !valid_backup_failure_reason(&state.reason_code)
        || state.started_at_ms < 0
        || state.completed_at_ms < state.started_at_ms
    {
        return Err(OpError::Validation("backup_preflight".into()));
    }
    let existing: Option<PreflightRow> = tx
        .query_row(
            "SELECT attempt_id, backup_id, state, artifact_name, edge_node_id, reason_code,
                started_at_ms, completed_at_ms
         FROM edge_node_backup_attempts
         WHERE attempt_id=?1 OR backup_id=?2 OR artifact_name=?3",
            params![state.attempt_id, state.backup_id, state.artifact_name],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()?;
    if existing.as_ref().is_some_and(|existing| {
        existing
            != &(
                state.attempt_id,
                state.backup_id,
                "failed".into(),
                state.artifact_name,
                state.edge_node_id,
                state.reason_code,
                state.started_at_ms,
                state.completed_at_ms,
            )
    }) {
        return Err(OpError::PreconditionFailed(
            "backup_attempt_conflict".into(),
        ));
    }
    Ok(())
}

fn preflight_execute(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<Value, OpError> {
    let state: PreflightState = private_state(context)?;
    tx.execute(
        "INSERT OR IGNORE INTO edge_node_backup_attempts(
             attempt_id, backup_id, state, reason_code, artifact_name, edge_node_id,
             started_at_ms, completed_at_ms
         ) VALUES(?1, ?2, 'failed', ?3, ?4, ?5, ?6, ?7)",
        params![
            state.attempt_id,
            state.backup_id,
            state.reason_code,
            state.artifact_name,
            state.edge_node_id,
            state.started_at_ms,
            state.completed_at_ms
        ],
    )?;
    Ok(json!({"state": "failed"}))
}

/// Creates one encrypted Edge Node backup.
pub fn create_backup(
    config_path: &Path,
    passphrase: &BackupPassphrase,
    now_ms: i64,
) -> Result<NodeBackupManifest, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        create_backup_with_hook(config_path, passphrase, now_ms, &SystemBackupHook)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (config_path, passphrase, now_ms);
        Err(RecoveryError::PlatformUnsupported)
    }
}

/// Creates one encrypted backup after atomically selecting the owner-only
/// configuration and its configured passphrase under the config operation
/// lease. Callers must not load either file before invoking this API.
pub fn create_backup_from_files(
    config_path: &Path,
    now_ms: i64,
) -> Result<NodeBackupManifest, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        create_backup_from_files_with_hook(config_path, now_ms, &SystemBackupHook)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (config_path, now_ms);
        Err(RecoveryError::PlatformUnsupported)
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackupHookPoint {
    BeforeSnapshot,
    AfterBegin,
    AfterPublication,
    AfterReadback,
    BeforeReceipt,
    AfterReceipt,
    BeforeReconciliationParentSync,
}

#[cfg(target_os = "linux")]
pub(crate) trait BackupHook {
    fn at(&self, _point: BackupHookPoint, _config: &BackupConfig) -> Result<(), RecoveryError> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
struct SystemBackupHook;

#[cfg(target_os = "linux")]
impl BackupHook for SystemBackupHook {}

#[cfg(target_os = "linux")]
pub(crate) fn create_backup_with_hook(
    config_path: &Path,
    passphrase: &BackupPassphrase,
    now_ms: i64,
    hook: &impl BackupHook,
) -> Result<NodeBackupManifest, RecoveryError> {
    let guard = acquire_recovery_operation(config_path)?;
    let config = load_owner_only_config(config_path)?;
    create_backup_guarded(&guard, &config, passphrase, now_ms, hook)
}

#[cfg(target_os = "linux")]
pub(crate) fn create_backup_from_files_with_hook(
    config_path: &Path,
    now_ms: i64,
    hook: &impl BackupHook,
) -> Result<NodeBackupManifest, RecoveryError> {
    let guard = acquire_recovery_operation(config_path)?;
    let config = load_owner_only_config(config_path)?;
    let passphrase = crate::config::load_owner_only_passphrase(&config.passphrase_file)?;
    pause_after_create_selection(config_path)?;
    create_backup_guarded(&guard, &config, &passphrase, now_ms, hook)
}

#[cfg(target_os = "linux")]
fn pause_after_create_selection(config_path: &Path) -> Result<(), RecoveryError> {
    let Some(expected) = std::env::var_os("IOTKIT_TEST_BACKUP_CREATE_PAUSE_PATH") else {
        return Ok(());
    };
    if std::path::Path::new(&expected) != config_path
        || std::env::var_os("IOTKIT_TEST_BACKUP_CREATE_PAUSE_AFTER_SELECTION").is_none()
    {
        return Ok(());
    }
    let ready = std::env::var_os("IOTKIT_TEST_BACKUP_CREATE_READY_FILE")
        .ok_or(RecoveryError::InvalidConfiguration)?;
    let proceed = std::env::var_os("IOTKIT_TEST_BACKUP_CREATE_CONTINUE_FILE")
        .ok_or(RecoveryError::InvalidConfiguration)?;
    std::fs::write(&ready, b"ready").map_err(|_| RecoveryError::Storage)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !std::path::Path::new(&proceed).exists() {
        if std::time::Instant::now() >= deadline {
            return Err(RecoveryError::Storage);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn create_backup_guarded(
    guard: &crate::RecoveryOperationGuard,
    config: &BackupConfig,
    passphrase: &BackupPassphrase,
    now_ms: i64,
    hook: &impl BackupHook,
) -> Result<NodeBackupManifest, RecoveryError> {
    crate::config::validate_config(config)?;
    if now_ms < 0 || !(12..=1024).contains(&passphrase.char_count()) {
        return Err(if now_ms < 0 {
            RecoveryError::InvalidConfiguration
        } else {
            RecoveryError::InvalidPassphrase
        });
    }

    let source_metadata =
        std::fs::symlink_metadata(&config.database).map_err(|_| RecoveryError::Storage)?;
    if !source_metadata.file_type().is_file() {
        return Err(RecoveryError::Storage);
    }
    let source = Connection::open_with_flags(&config.database, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|_| RecoveryError::Storage)?;
    if !matches!(startup_mode(&source)?, RecoveryStartupMode::Normal) {
        return Err(RecoveryError::InvalidStartupState);
    }
    let edge_node_id: String = source
        .query_row(
            "SELECT value FROM ledger_meta WHERE key='edge_node_id'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    if !valid_identity(&edge_node_id) {
        return Err(RecoveryError::InvalidStartupState);
    }
    let ledger_epoch: String = source
        .query_row(
            "SELECT value FROM ledger_meta WHERE key='epoch'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    if !valid_identity(&ledger_epoch) {
        return Err(RecoveryError::InvalidStartupState);
    }
    let source_length = source_metadata.len();
    let has_started: bool = source
        .query_row(
            "SELECT EXISTS(
                    SELECT 1 FROM edge_node_backup_attempts WHERE state='started'
                )",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::Storage)?;
    if has_started {
        let reconciliation_staging = verify_staging_directory_for_reconciliation(guard, config)?;
        cleanup_prior_plaintext(&reconciliation_staging)?;
        let reconciliation_destination = verify_destination_for_reconciliation(guard, config)?;
        if let Some(manifest) = reconcile_started(
            &source,
            &reconciliation_destination,
            passphrase,
            now_ms,
            config,
            guard,
            hook,
        )? {
            return Ok(manifest);
        }
    }
    let destination = match verify_destination(guard, config, source_length) {
        Ok(destination) => destination,
        Err(error) => {
            record_preflight(&source, &edge_node_id, now_ms, error)?;
            return Err(error);
        }
    };
    let staging = match verify_staging_directory(guard, config, source_length) {
        Ok(staging) => staging,
        Err(error) => {
            record_preflight(&source, &edge_node_id, now_ms, error)?;
            return Err(error);
        }
    };
    cleanup_prior_plaintext(&staging)?;

    let attempt_id = random_identity("attempt-")?;
    let backup_id = random_identity("backup-")?;
    let artifact_name = format!("{backup_id}{}", crate::NODE_BACKUP_SUFFIX);
    let stage_name = format!(".iotkit-backup-stage-{}.sqlite", random_identity("")?);
    let stage_path = staging_path(&staging, &stage_name)?;
    if let Err(error) = hook.at(BackupHookPoint::BeforeSnapshot, config) {
        return cleanup_stage(&staging, &stage_name).and(Err(error));
    }
    let snapshot =
        match create_consistent_snapshot(&config.database, &stage_path, &backup_id, now_ms) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let cleanup = cleanup_stage(&staging, &stage_name);
                let receipt = dispatch_state(
                    &source,
                    RECORD_BACKUP_PREFLIGHT_FAILURE_OP,
                    &PreflightState {
                        attempt_id,
                        backup_id,
                        artifact_name,
                        edge_node_id,
                        reason_code: error.reason_code().into(),
                        started_at_ms: now_ms,
                        completed_at_ms: now_ms,
                    },
                );
                receipt?;
                return cleanup.and(Err(error));
            }
        };
    if !source_path_still_identical(&source_metadata, &config.database) {
        return cleanup_stage(&staging, &stage_name).and(Err(RecoveryError::InvalidSnapshot));
    }
    if snapshot.manifest.edge_node_id != edge_node_id
        || snapshot.manifest.ledger_epoch != ledger_epoch
    {
        let cleanup = cleanup_stage(&staging, &stage_name);
        dispatch_state(
            &source,
            RECORD_BACKUP_PREFLIGHT_FAILURE_OP,
            &PreflightState {
                attempt_id,
                backup_id,
                artifact_name,
                edge_node_id,
                reason_code: RecoveryError::InvalidSnapshot.reason_code().into(),
                started_at_ms: now_ms,
                completed_at_ms: now_ms,
            },
        )?;
        return cleanup.and(Err(RecoveryError::InvalidSnapshot));
    }
    if let Err(error) = dispatch_state(
        &source,
        BEGIN_BACKUP_ATTEMPT_OP,
        &BeginState {
            attempt_id: attempt_id.clone(),
            backup_id: backup_id.clone(),
            artifact_name: artifact_name.clone(),
            edge_node_id: edge_node_id.clone(),
            started_at_ms: now_ms,
        },
    ) {
        return cleanup_stage(&staging, &stage_name).and(Err(error));
    }
    hook.at(BackupHookPoint::AfterBegin, config)?;

    let publication = encrypt_container(
        &snapshot.path,
        &snapshot.manifest,
        passphrase,
        destination.capability(),
        &artifact_name,
    );
    if let Err(error) = publication {
        let cleanup = cleanup_stage(&staging, &stage_name);
        if error == RecoveryError::ArtifactPublicationUncertain {
            return cleanup.and(Err(error));
        }
        complete_failure(&source, &attempt_id, now_ms, error)?;
        return cleanup.and(Err(error));
    }
    if let Err(error) = hook.at(BackupHookPoint::AfterPublication, config) {
        return cleanup_stage(&staging, &stage_name).and(Err(error));
    }

    let completion = (|| {
        let (published_file, artifact_length) =
            open_published_artifact(&destination, &artifact_name)
                .map_err(|_| RecoveryError::ArtifactPublicationUncertain)?;
        let inspected = crate::container::authenticate_container_file(published_file, passphrase)
            .map_err(|_| RecoveryError::ArtifactPublicationUncertain)?;
        if inspected != snapshot.manifest {
            return Err(RecoveryError::ArtifactPublicationUncertain);
        }
        hook.at(BackupHookPoint::AfterReadback, config)?;
        hook.at(BackupHookPoint::BeforeReceipt, config)?;
        dispatch_state(
            &source,
            COMPLETE_BACKUP_ATTEMPT_OP,
            &CompleteState {
                attempt_id,
                outcome: "success".into(),
                reason_code: "ok".into(),
                artifact_length: Some(
                    i64::try_from(artifact_length)
                        .map_err(|_| RecoveryError::ArtifactPublicationUncertain)?,
                ),
                ledger_epoch: Some(inspected.ledger_epoch.clone()),
                accepted_cursor: Some(inspected.accepted_cursor),
                allocation_high_water: Some(inspected.allocation_high_water),
                artifact_created_at_ms: Some(inspected.created_at_ms),
                completed_at_ms: now_ms,
            },
        )
        .map_err(|_| RecoveryError::ArtifactPublicationUncertain)?;
        hook.at(BackupHookPoint::AfterReceipt, config)?;
        Ok(inspected)
    })();
    let inspected = match completion {
        Ok(inspected) => inspected,
        Err(error) => return cleanup_stage(&staging, &stage_name).and(Err(error)),
    };
    let retention = apply_success_retention(
        &source,
        guard,
        &destination,
        passphrase,
        &edge_node_id,
        config.retention_count,
    );
    let cleanup = cleanup_stage(&staging, &stage_name);
    match (retention, cleanup) {
        (Err(error), _) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(inspected),
    }
}

#[cfg(target_os = "linux")]
fn source_path_still_identical(original: &std::fs::Metadata, path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|current| {
        current.file_type().is_file()
            && current.dev() == original.dev()
            && current.ino() == original.ino()
    })
}

/// Authenticates and returns the manifest of one backup without writing plaintext.
///
/// Inspection is deliberately configuration-free: unlike create/status/restore,
/// it does not acquire the config-adjacent operation lease or open the live
/// database. The artifact is authenticated through one held descriptor by the
/// container layer.
pub fn inspect_backup(
    input: &Path,
    passphrase: &BackupPassphrase,
) -> Result<NodeBackupManifest, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(input)
            .map_err(|_| RecoveryError::Storage)?;
        let metadata = file.metadata().map_err(|_| RecoveryError::Storage)?;
        if !metadata.file_type().is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(RecoveryError::InvalidConfiguration);
        }
        crate::config::clear_nonblock(&file)?;
        crate::container::authenticate_container_file(file, passphrase)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (input, passphrase);
        Err(RecoveryError::PlatformUnsupported)
    }
}

/// Reads durable backup readiness without mutating the configured database.
pub fn backup_status(config_path: &Path, now_ms: i64) -> Result<BackupReadiness, RecoveryError> {
    if now_ms < 0 {
        return Err(RecoveryError::InvalidConfiguration);
    }
    #[cfg(target_os = "linux")]
    let observation = match acquire_recovery_observation(config_path) {
        Ok(guard) => guard,
        Err(RecoveryError::OperationBusy) => return Ok(BackupReadiness::OperationBusy),
        Err(error) => return Err(error),
    };
    #[cfg(target_os = "linux")]
    if crate::config::pending_pair_marker(config_path)? {
        return Err(RecoveryError::CleanupRequired);
    }
    match std::fs::symlink_metadata(config_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BackupReadiness::NotConfigured);
        }
        Err(_) => return Err(RecoveryError::InvalidConfiguration),
        Ok(_) => {}
    }
    #[cfg(not(target_os = "linux"))]
    let observation = match acquire_recovery_observation(config_path) {
        Ok(guard) => guard,
        Err(RecoveryError::OperationBusy) => return Ok(BackupReadiness::OperationBusy),
        Err(error) => return Err(error),
    };
    let config = load_owner_only_config(config_path)?;
    if observation.coordinates_existing_lock() {
        return read_backup_status(&config, now_ms);
    }
    let first = read_backup_status(&config, now_ms)?;
    let second = match acquire_recovery_observation(config_path) {
        Ok(guard) => guard,
        Err(RecoveryError::OperationBusy) => return Ok(BackupReadiness::OperationBusy),
        Err(error) => return Err(error),
    };
    if second.coordinates_existing_lock() {
        let current = load_owner_only_config(config_path)?;
        read_backup_status(&current, now_ms)
    } else {
        Ok(first)
    }
}

fn read_backup_status(
    config: &BackupConfig,
    now_ms: i64,
) -> Result<BackupReadiness, RecoveryError> {
    let conn = Connection::open_with_flags(&config.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RecoveryError::Storage)?;
    let _ = startup_mode(&conn)?;
    let latest: Option<LatestAttemptRow> = conn
        .query_row(
            "SELECT attempt_id, backup_id, state, reason_code, artifact_length, edge_node_id,
                    ledger_epoch, accepted_cursor, allocation_high_water, started_at_ms,
                    artifact_created_at_ms, completed_at_ms
             FROM edge_node_backup_attempts
             ORDER BY started_at_ms DESC, rowid DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                ))
            },
        )
        .optional()
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    let Some((
        _attempt_id,
        backup_id,
        state,
        reason_code,
        artifact_length,
        edge_node_id,
        ledger_epoch,
        accepted_cursor,
        allocation_high_water,
        started_at_ms,
        artifact_created_at_ms,
        completed_at_ms,
    )) = latest
    else {
        return Ok(BackupReadiness::Failed {
            reason_code: "never_completed".into(),
            observed_at_ms: now_ms,
            last_verified: None,
        });
    };
    if state != "success" {
        return Ok(BackupReadiness::Failed {
            reason_code: if state == "started" {
                "interrupted".into()
            } else {
                reason_code.unwrap_or_else(|| "failed".into())
            },
            observed_at_ms: completed_at_ms.unwrap_or(started_at_ms),
            last_verified: last_success(&conn)?,
        });
    }
    let artifact = BackupStatusArtifact {
        backup_id,
        edge_node_id,
        ledger_epoch: ledger_epoch.ok_or(RecoveryError::InvalidStartupState)?,
        created_at_ms: artifact_created_at_ms.ok_or(RecoveryError::InvalidStartupState)?,
        artifact_length: u64::try_from(artifact_length.ok_or(RecoveryError::InvalidStartupState)?)
            .map_err(|_| RecoveryError::InvalidStartupState)?,
        accepted_cursor: accepted_cursor.ok_or(RecoveryError::InvalidStartupState)?,
        allocation_high_water: allocation_high_water.ok_or(RecoveryError::InvalidStartupState)?,
    };
    let freshness_ms = i64::try_from(config.freshness_seconds)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1000))
        .ok_or(RecoveryError::CapacityOverflow)?;
    if now_ms <= artifact.created_at_ms.saturating_add(freshness_ms) {
        Ok(BackupReadiness::Healthy { artifact })
    } else {
        Ok(BackupReadiness::Stale { artifact })
    }
}

fn last_success(conn: &Connection) -> Result<Option<BackupStatusArtifact>, RecoveryError> {
    conn.query_row(
        "SELECT backup_id, edge_node_id, ledger_epoch, artifact_created_at_ms,
                artifact_length, accepted_cursor, allocation_high_water
         FROM edge_node_backup_attempts
         WHERE state='success'
         ORDER BY completed_at_ms DESC, rowid DESC LIMIT 1",
        [],
        |row| {
            let length: i64 = row.get(4)?;
            Ok(BackupStatusArtifact {
                backup_id: row.get(0)?,
                edge_node_id: row.get(1)?,
                ledger_epoch: row.get(2)?,
                created_at_ms: row.get(3)?,
                artifact_length: u64::try_from(length)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, length))?,
                accepted_cursor: row.get(5)?,
                allocation_high_water: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(|_| RecoveryError::InvalidStartupState)
}

#[cfg(target_os = "linux")]
fn record_preflight(
    conn: &Connection,
    edge_node_id: &str,
    now_ms: i64,
    error: RecoveryError,
) -> Result<(), RecoveryError> {
    let backup_id = random_identity("backup-")?;
    dispatch_state(
        conn,
        RECORD_BACKUP_PREFLIGHT_FAILURE_OP,
        &PreflightState {
            attempt_id: random_identity("attempt-")?,
            artifact_name: format!("{backup_id}{}", crate::NODE_BACKUP_SUFFIX),
            backup_id,
            edge_node_id: edge_node_id.into(),
            reason_code: error.reason_code().into(),
            started_at_ms: now_ms,
            completed_at_ms: now_ms,
        },
    )
}

#[cfg(target_os = "linux")]
fn complete_failure(
    conn: &Connection,
    attempt_id: &str,
    now_ms: i64,
    error: RecoveryError,
) -> Result<(), RecoveryError> {
    complete_with_reason(conn, attempt_id, now_ms, error.reason_code())
}

#[cfg(target_os = "linux")]
fn complete_with_reason(
    conn: &Connection,
    attempt_id: &str,
    now_ms: i64,
    reason_code: &str,
) -> Result<(), RecoveryError> {
    dispatch_state(
        conn,
        COMPLETE_BACKUP_ATTEMPT_OP,
        &CompleteState {
            attempt_id: attempt_id.into(),
            outcome: "failed".into(),
            reason_code: reason_code.into(),
            artifact_length: None,
            ledger_epoch: None,
            accepted_cursor: None,
            allocation_high_water: None,
            artifact_created_at_ms: None,
            completed_at_ms: now_ms,
        },
    )
}

#[cfg(target_os = "linux")]
fn random_identity(prefix: &str) -> Result<String, RecoveryError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| RecoveryError::Random)?;
    let mut value = String::from(prefix);
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").map_err(|_| RecoveryError::Random)?;
    }
    Ok(value)
}

#[cfg(target_os = "linux")]
fn staging_path(
    staging: &VerifiedStagingDirectory,
    name: &str,
) -> Result<std::path::PathBuf, RecoveryError> {
    if !is_stage_name(name) {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(std::path::PathBuf::from(format!(
        "/proc/self/fd/{}/{}",
        staging.capability().as_raw_fd(),
        name
    )))
}

#[cfg(target_os = "linux")]
fn is_stage_name(name: &str) -> bool {
    name.strip_prefix(".iotkit-backup-stage-")
        .and_then(|name| name.strip_suffix(".sqlite"))
        .is_some_and(|hex| hex.len() == 32 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(target_os = "linux")]
pub(crate) fn cleanup_prior_plaintext(
    staging: &VerifiedStagingDirectory,
) -> Result<(), RecoveryError> {
    let directory_fd = staging.capability().as_raw_fd();
    let duplicated = unsafe {
        libc::openat(
            directory_fd,
            c".".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if duplicated < 0 {
        return Err(RecoveryError::Storage);
    }
    let stream = unsafe { libc::fdopendir(duplicated) };
    if stream.is_null() {
        unsafe {
            libc::close(duplicated);
        }
        return Err(RecoveryError::Storage);
    }
    let mut names = Vec::new();
    let mut actionable_unknown = false;
    loop {
        unsafe {
            *libc::__errno_location() = 0;
        }
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            let errno = unsafe { *libc::__errno_location() };
            unsafe {
                libc::closedir(stream);
            }
            if errno != 0 {
                return Err(RecoveryError::Storage);
            }
            break;
        }
        let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes.starts_with(b".iotkit-backup-stage-") {
            match std::str::from_utf8(bytes) {
                Ok(name) if is_stage_name(name) => names.push(name.to_string()),
                _ => actionable_unknown = true,
            }
        }
    }
    for name in names {
        cleanup_stage(staging, &name)?;
    }
    if actionable_unknown {
        Err(RecoveryError::CleanupRequired)
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn cleanup_stage(staging: &VerifiedStagingDirectory, name: &str) -> Result<(), RecoveryError> {
    if !is_stage_name(name) {
        return Err(RecoveryError::CleanupRequired);
    }
    let name = CString::new(name).map_err(|_| RecoveryError::CleanupRequired)?;
    let fd = unsafe {
        libc::openat(
            staging.capability().as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(RecoveryError::CleanupRequired)
        };
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryError::CleanupRequired)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(RecoveryError::CleanupRequired);
    }
    crate::destination::remove_exact_file_at(staging.capability().as_raw_fd(), &name, &file)
        .map_err(|_| RecoveryError::ArtifactCleanupFailed)
}

#[cfg(target_os = "linux")]
fn open_published_artifact(
    destination: &VerifiedBackupDestination,
    name: &str,
) -> Result<(File, u64), RecoveryError> {
    if !valid_artifact_name(name) {
        return Err(RecoveryError::DestinationInvalid);
    }
    let name = CString::new(name).map_err(|_| RecoveryError::DestinationInvalid)?;
    let fd = unsafe {
        libc::openat(
            destination.capability().as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(RecoveryError::ArtifactPublicationUncertain);
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryError::ArtifactPublicationUncertain)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(RecoveryError::ArtifactPublicationUncertain);
    }
    Ok((file, metadata.len()))
}

#[cfg(target_os = "linux")]
fn reconcile_started(
    conn: &Connection,
    destination: &VerifiedBackupDestination,
    passphrase: &BackupPassphrase,
    now_ms: i64,
    config: &BackupConfig,
    guard: &crate::RecoveryOperationGuard,
    hook: &impl BackupHook,
) -> Result<Option<NodeBackupManifest>, RecoveryError> {
    let started: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT attempt_id, backup_id, artifact_name, edge_node_id
             FROM edge_node_backup_attempts
             WHERE state='started'
             ORDER BY started_at_ms DESC, attempt_id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|_| RecoveryError::Storage)?;
    let Some((attempt_id, backup_id, artifact_name, edge_node_id)) = started else {
        return Ok(None);
    };
    let authenticated =
        open_published_artifact(destination, &artifact_name).and_then(|(file, length)| {
            crate::container::authenticate_container_file(file, passphrase)
                .map(|manifest| (manifest, length))
        });
    let Ok((manifest, length)) = authenticated else {
        complete_with_reason(conn, &attempt_id, now_ms, "interrupted")?;
        return Ok(None);
    };
    if manifest.backup_id != backup_id || manifest.edge_node_id != edge_node_id {
        complete_with_reason(conn, &attempt_id, now_ms, "interrupted")?;
        return Ok(None);
    }
    hook.at(BackupHookPoint::BeforeReconciliationParentSync, config)?;
    destination.capability().sync_directory()?;
    dispatch_state(
        conn,
        COMPLETE_BACKUP_ATTEMPT_OP,
        &CompleteState {
            attempt_id,
            outcome: "success".into(),
            reason_code: "ok".into(),
            artifact_length: Some(i64::try_from(length).map_err(|_| RecoveryError::Storage)?),
            ledger_epoch: Some(manifest.ledger_epoch.clone()),
            accepted_cursor: Some(manifest.accepted_cursor),
            allocation_high_water: Some(manifest.allocation_high_water),
            artifact_created_at_ms: Some(manifest.created_at_ms),
            completed_at_ms: now_ms,
        },
    )?;
    apply_success_retention(
        conn,
        guard,
        destination,
        passphrase,
        &edge_node_id,
        config.retention_count,
    )?;
    Ok(Some(manifest))
}

#[cfg(target_os = "linux")]
fn apply_success_retention(
    conn: &Connection,
    guard: &crate::RecoveryOperationGuard,
    destination: &VerifiedBackupDestination,
    passphrase: &BackupPassphrase,
    edge_node_id: &str,
    retention_count: u32,
) -> Result<(), RecoveryError> {
    let mut statement = conn
        .prepare(
            "SELECT backup_id, artifact_name
             FROM edge_node_backup_attempts WHERE state='success'",
        )
        .map_err(|_| RecoveryError::Storage)?;
    let artifacts = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| RecoveryError::Storage)?
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(|_| RecoveryError::Storage)?;
    crate::destination::apply_recorded_retention(
        guard,
        destination,
        passphrase,
        edge_node_id,
        &artifacts,
        retention_count,
    )?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn dispatch_state<T: Serialize>(
    conn: &Connection,
    op: &str,
    state: &T,
) -> Result<(), RecoveryError> {
    dispatch(
        conn,
        crate::recovery_descriptors(),
        DispatchRequest {
            op: op.into(),
            params: json!({"private_recovery_state": state}),
            dry_run: false,
            actor: Actor {
                actor_id: "recovery-backup".into(),
                actor_kind: ActorKind::LocalCli,
                tier_ceiling: Tier::Construction,
            },
            source: None,
            step_up_verified: true,
            clock_trust: None,
        },
    )
    .map(|_| ())
    .map_err(|_| RecoveryError::Storage)
}
