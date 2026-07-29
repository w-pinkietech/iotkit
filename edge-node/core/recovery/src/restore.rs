//! Restore an authenticated Node backup into a new, durably fenced candidate.
//!
//! This module deliberately keeps the live database out of the write path.  All
//! candidate writes are made through a held parent descriptor and a generated
//! owner-only temporary name, then published with `RENAME_NOREPLACE`.

#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use std::{
    ffi::CString,
    fs::File,
    os::fd::{AsRawFd, FromRawFd},
    os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
};

#[cfg(target_os = "linux")]
use iotkit_core_ops::{Actor, ActorKind, DispatchRequest, dispatch};
use iotkit_core_ops::{OpContext, OpDescriptor, OpError, Tier};
#[cfg(target_os = "linux")]
use rusqlite::{Connection, OpenFlags};
use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(target_os = "linux")]
use crate::container::{authenticate_container_file, decrypt_container_file_to_staging_file};
use crate::{BackupPassphrase, RecoveryError, RestoreReceipt, RestoreRequest};
#[cfg(target_os = "linux")]
use crate::{
    DirectoryCapability, NodeBackupManifest, RecoveryHandoff, RecoveryStartupMode, RestoreStatus,
    required_capacity, startup_mode, validate_restored_candidate, validate_snapshot,
};

pub const INSTALL_CANDIDATE_OP: &str = "recovery.restore.install_candidate";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallCandidateState {
    recovery_id: String,
    candidate_instance_id: String,
    backup_id: String,
    source_database_length: u64,
    source_database_sha256: String,
    edge_id: String,
    edge_node_id: String,
    old_ledger_epoch: String,
    proposed_new_epoch: String,
    credential_generation: i64,
    installed_at_ms: i64,
}

pub(crate) fn restore_descriptors() -> Vec<OpDescriptor> {
    vec![OpDescriptor {
        name: INSTALL_CANDIDATE_OP,
        tier: Tier::Construction,
        bulk_escalates: false,
        changes_state: true,
        params_schema: private_schema,
        targets: |_| Vec::new(),
        preconditions: install_preconditions,
        dry_run: |_, _| Ok(json!({"would": "install_fenced_candidate"})),
        execute: install_execute,
        secret_execute: None,
    }]
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

fn install_preconditions(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<(), OpError> {
    let state: InstallCandidateState = private_state(context)?;
    if !valid_identity(&state.recovery_id)
        || !valid_identity(&state.candidate_instance_id)
        || !valid_identity(&state.backup_id)
        || i64::try_from(state.source_database_length).is_err()
        || !valid_digest(&state.source_database_sha256)
        || !valid_identity(&state.edge_id)
        || !valid_identity(&state.edge_node_id)
        || !valid_identity(&state.old_ledger_epoch)
        || !valid_identity(&state.proposed_new_epoch)
        || state.old_ledger_epoch == state.proposed_new_epoch
        || state.credential_generation < 0
        || state.installed_at_ms < 0
    {
        return Err(OpError::Validation("restore_candidate".into()));
    }
    let candidate_rows: i64 = tx.query_row(
        "SELECT count(*) FROM edge_node_recovery_candidate",
        [],
        |row| row.get(0),
    )?;
    if candidate_rows != 0 {
        return Err(OpError::PreconditionFailed("candidate_conflict".into()));
    }
    let (node_id, epoch): (String, String) = tx.query_row(
        "SELECT
             (SELECT value FROM ledger_meta WHERE key='edge_node_id'),
             (SELECT value FROM ledger_meta WHERE key='epoch')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if node_id != state.edge_node_id || epoch != state.old_ledger_epoch {
        return Err(OpError::PreconditionFailed("candidate_identity".into()));
    }
    let (activation_state, edge_id, activation_epoch): (String, Option<String>, Option<String>) =
        tx.query_row(
            "SELECT state, edge_id, ledger_epoch FROM edge_node_activation WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    if activation_state != "active"
        || edge_id.as_deref() != Some(state.edge_id.as_str())
        || activation_epoch.as_deref() != Some(state.old_ledger_epoch.as_str())
    {
        return Err(OpError::PreconditionFailed("candidate_activation".into()));
    }
    Ok(())
}

fn install_execute(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<Value, OpError> {
    let state: InstallCandidateState = private_state(context)?;
    let source_database_length = i64::try_from(state.source_database_length)
        .map_err(|_| OpError::Validation("restore_candidate".into()))?;
    let new_auth_epoch = iotkit_core_ops::new_auth_epoch()?;
    iotkit_core_ops::enter_restored_local_recovery(tx, &new_auth_epoch)?;
    tx.execute(
        "INSERT INTO edge_node_recovery_candidate(
             singleton, state, recovery_id, candidate_instance_id, backup_id,
             source_database_length, source_database_sha256,
             edge_id, edge_node_id, old_ledger_epoch, proposed_new_epoch,
             credential_generation, handoff_schema_version, installed_at_ms
         ) VALUES(1, 'durably_fenced_candidate', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11)",
        params![
            state.recovery_id,
            state.candidate_instance_id,
            state.backup_id,
            source_database_length,
            state.source_database_sha256,
            state.edge_id,
            state.edge_node_id,
            state.old_ledger_epoch,
            state.proposed_new_epoch,
            state.credential_generation,
            state.installed_at_ms,
        ],
    )?;
    Ok(json!({"state": "durably_fenced_candidate"}))
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !value.contains(':')
        && !value.chars().any(char::is_control)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(target_os = "linux")]
pub(crate) fn validate_handoff(
    handoff: &RecoveryHandoff,
    manifest: &NodeBackupManifest,
) -> Result<(), RecoveryError> {
    validate_handoff_shape(handoff)?;
    if handoff.expected_backup_id.as_deref() != Some(manifest.backup_id.as_str())
        || handoff.edge_node_id != manifest.edge_node_id
        || handoff.old_ledger_epoch != manifest.ledger_epoch
    {
        return Err(RecoveryError::HandoffMismatch);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_handoff_shape(handoff: &RecoveryHandoff) -> Result<(), RecoveryError> {
    if handoff.schema_version != 1
        || !valid_identity(&handoff.recovery_id)
        || !valid_identity(&handoff.edge_id)
        || !valid_identity(&handoff.edge_node_id)
        || !valid_identity(&handoff.old_ledger_epoch)
        || !valid_identity(&handoff.proposed_new_epoch)
        || handoff.old_ledger_epoch == handoff.proposed_new_epoch
        || handoff.credential_generation < 0
    {
        return Err(RecoveryError::HandoffMismatch);
    }
    Ok(())
}

pub fn restore_candidate(
    request: &RestoreRequest,
    passphrase: &BackupPassphrase,
) -> Result<RestoreReceipt, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        restore_candidate_inner(request, passphrase, None)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (request, passphrase);
        Err(RecoveryError::PlatformUnsupported)
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestorePhase {
    Decrypted,
    Copied,
    FenceCommitted,
    Checkpointed,
    CandidateFileSynced,
    RenameSucceeded,
    ParentSynced,
    PublishedReadbackVerified,
}

#[cfg(target_os = "linux")]
pub(crate) fn restore_candidate_inner(
    request: &RestoreRequest,
    passphrase: &BackupPassphrase,
    hook: Option<&dyn Fn(RestorePhase, bool) -> Result<(), RecoveryError>>,
) -> Result<RestoreReceipt, RecoveryError> {
    let live = normalize_absolute(&request.live_database)?;
    let candidate = normalize_absolute(&request.candidate_database)?;
    let staging = normalize_absolute(&request.staging_directory)?;
    if live == candidate {
        return Err(RecoveryError::InvalidConfiguration);
    }

    // The live database's parent is the stable config-adjacent recovery lock.
    let _operation_guard = crate::acquire_recovery_operation(&live)?;
    let live_parent_path = live.parent().ok_or(RecoveryError::InvalidConfiguration)?;
    let candidate_parent_path = candidate
        .parent()
        .ok_or(RecoveryError::InvalidConfiguration)?;
    let live_parent = DirectoryCapability::open(live_parent_path)?;
    let candidate_parent = DirectoryCapability::open(candidate_parent_path)?;
    verify_owner_directory(&live_parent)?;
    verify_owner_directory(&candidate_parent)?;
    _operation_guard.ensure_parent(&live_parent)?;
    let live_parent_id = directory_identity(&live_parent)?;
    let candidate_parent_id = directory_identity(&candidate_parent)?;
    if live_parent_id == candidate_parent_id
        && file_name_bytes(&live)? == file_name_bytes(&candidate)?
    {
        return Err(RecoveryError::InvalidConfiguration);
    }

    let live_name = file_name_cstring(&live)?;
    let candidate_name = file_name_cstring(&candidate)?;
    let _candidate_lock = acquire_candidate_lock(&candidate_parent, &candidate_name)?;
    let live_identity = capture_optional_identity(&live_parent, &live_name)?;
    let (artifact_identity, manifest) = authenticate_input(&request.input, passphrase)?;
    if let Some(existing) = open_existing_candidate(&candidate_parent, &candidate_name)? {
        let receipt = replay_existing_candidate(
            existing,
            &candidate_parent,
            &candidate_name,
            &request.handoff,
            &manifest,
        )?;
        validate_live_identity(
            &live_parent,
            &live_name,
            live_identity,
            &manifest,
            &request.handoff.edge_id,
        )?;
        ensure_live_unchanged(&live_parent, &live_name, live_identity)?;
        return Ok(receipt);
    }
    validate_handoff(&request.handoff, &manifest)?;
    validate_live_identity(
        &live_parent,
        &live_name,
        live_identity,
        &manifest,
        &request.handoff.edge_id,
    )?;

    let staging_cap = DirectoryCapability::open(&staging)?;
    verify_owner_directory(&staging_cap)?;
    verify_staging_tmpfs(&staging_cap)?;
    let required = required_capacity(manifest.database_length)?;
    if free_bytes(&staging_cap)? < required || free_bytes(&candidate_parent)? < required {
        return Err(RecoveryError::StorageFull);
    }

    let decrypt_file = open_input(&request.input)?;
    if file_identity(&decrypt_file)? != artifact_identity {
        return Err(RecoveryError::CandidateConflict);
    }
    let (mut plaintext, decrypted_manifest) = decrypt_container_file_to_staging_file(
        decrypt_file,
        passphrase,
        &staging_cap,
        manifest.database_length,
    )?;
    if decrypted_manifest != manifest {
        return Err(RecoveryError::ManifestInvalid);
    }
    invoke_hook(hook, RestorePhase::Decrypted, false)?;

    let temporary_name = random_name("restore")?;
    let temporary_c =
        CString::new(temporary_name.as_bytes()).map_err(|_| RecoveryError::Storage)?;
    let temporary_fd = unsafe {
        libc::openat(
            candidate_parent.as_raw_fd(),
            temporary_c.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if temporary_fd < 0 {
        return Err(RecoveryError::Storage);
    }
    let mut temporary = unsafe { File::from_raw_fd(temporary_fd) };
    let temporary_path = candidate_parent_path.join(&temporary_name);
    let mut published = false;
    let result = (|| {
        plaintext.rewind().map_err(|_| RecoveryError::Storage)?;
        std::io::copy(&mut plaintext, &mut temporary).map_err(|_| RecoveryError::Storage)?;
        temporary.sync_all().map_err(|_| RecoveryError::Storage)?;
        invoke_hook(hook, RestorePhase::Copied, false)?;

        let facts = validate_snapshot(&temporary_path)?;
        if facts.edge_node_id != manifest.edge_node_id
            || facts.ledger_epoch != manifest.ledger_epoch
            || facts.database_length != manifest.database_length
            || facts.database_sha256 != manifest.database_sha256
        {
            return Err(RecoveryError::ManifestInvalid);
        }
        let edge_id = load_activation_edge_id(&temporary_path)?;
        if edge_id != request.handoff.edge_id {
            return Err(RecoveryError::HandoffMismatch);
        }
        dispatch_install_candidate(
            &temporary_path,
            InstallCandidateState {
                recovery_id: request.handoff.recovery_id.clone(),
                candidate_instance_id: random_id("candidate")?,
                backup_id: manifest.backup_id.clone(),
                source_database_length: manifest.database_length,
                source_database_sha256: manifest.database_sha256.clone(),
                edge_id,
                edge_node_id: manifest.edge_node_id.clone(),
                old_ledger_epoch: manifest.ledger_epoch.clone(),
                proposed_new_epoch: request.handoff.proposed_new_epoch.clone(),
                credential_generation: request.handoff.credential_generation,
                installed_at_ms: now_ms(),
            },
        )?;
        invoke_hook(hook, RestorePhase::FenceCommitted, false)?;

        checkpoint_without_wal(&temporary_path)?;
        invoke_hook(hook, RestorePhase::Checkpointed, false)?;
        ensure_no_sidecars(&temporary_path)?;
        temporary.sync_all().map_err(|_| RecoveryError::Storage)?;
        invoke_hook(hook, RestorePhase::CandidateFileSynced, false)?;
        publish_noreplace(&candidate_parent, &temporary_c, &candidate_name)?;
        invoke_hook(hook, RestorePhase::RenameSucceeded, true)?;
        candidate_parent
            .sync_directory()
            .map_err(|_| RecoveryError::CandidatePublicationUncertain)?;
        published = true;
        invoke_hook(hook, RestorePhase::ParentSynced, true)?;
        let published = open_existing_candidate(&candidate_parent, &candidate_name)?
            .ok_or(RecoveryError::CandidatePublicationUncertain)?;
        let receipt = read_and_verify_candidate(
            published,
            &candidate_parent,
            &candidate_name,
            &request.handoff,
            &manifest,
        )?;
        invoke_hook(hook, RestorePhase::PublishedReadbackVerified, true)?;
        ensure_live_unchanged(&live_parent, &live_name, live_identity)?;
        Ok(receipt)
    })();

    if result.is_err() {
        // Once rename succeeds the candidate is deliberately retained so the
        // next invocation can perform exact, non-mutating reconciliation.
        if !published {
            drop(temporary);
            let _ = remove_exact(&candidate_parent, &temporary_c);
            let _ = candidate_parent.sync_directory();
        }
    }
    result
}

#[cfg(target_os = "linux")]
fn invoke_hook(
    hook: Option<&dyn Fn(RestorePhase, bool) -> Result<(), RecoveryError>>,
    phase: RestorePhase,
    published: bool,
) -> Result<(), RecoveryError> {
    hook.map_or(Ok(()), |hook| hook(phase, published))
}

#[cfg(target_os = "linux")]
fn dispatch_install_candidate(
    path: &Path,
    state: InstallCandidateState,
) -> Result<(), RecoveryError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|_| RecoveryError::CandidateFenceInvalid)?;
    dispatch(
        &conn,
        crate::recovery_descriptors(),
        DispatchRequest {
            op: INSTALL_CANDIDATE_OP.into(),
            params: json!({"private_recovery_state": state}),
            dry_run: false,
            actor: Actor {
                actor_id: "recovery-restore".into(),
                actor_kind: ActorKind::LocalCli,
                tier_ceiling: Tier::Construction,
            },
            source: None,
            step_up_verified: true,
            clock_trust: None,
        },
    )
    .map_err(|_| RecoveryError::CandidateFenceInvalid)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn checkpoint_without_wal(path: &Path) -> Result<(), RecoveryError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|_| RecoveryError::CandidateFenceInvalid)?;
    let mode: String = conn
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .map_err(|_| RecoveryError::CandidateFenceInvalid)?;
    if !mode.eq_ignore_ascii_case("delete") {
        return Err(RecoveryError::CandidateFenceInvalid);
    }
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|_| RecoveryError::CandidateFenceInvalid)?;
    drop(conn);
    ensure_no_sidecars(path)
}

#[cfg(target_os = "linux")]
fn load_activation_edge_id(path: &Path) -> Result<String, RecoveryError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    conn.query_row(
        "SELECT edge_id FROM edge_node_activation WHERE singleton=1 AND state='active'",
        [],
        |row| row.get::<_, Option<String>>(0),
    )
    .map_err(|_| RecoveryError::InvalidSnapshot)?
    .ok_or(RecoveryError::InvalidSnapshot)
}

#[cfg(target_os = "linux")]
fn read_and_verify_candidate(
    file: File,
    parent: &DirectoryCapability,
    name: &CString,
    handoff: &RecoveryHandoff,
    manifest: &NodeBackupManifest,
) -> Result<RestoreReceipt, RecoveryError> {
    let path = file_path_from_fd(&file)?;
    ensure_no_sidecars_at(parent, name)?;
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RecoveryError::CandidateFenceInvalid)?;
    conn.pragma_update(None, "query_only", "ON")
        .map_err(|_| RecoveryError::CandidateFenceInvalid)?;
    let facts =
        validate_restored_candidate(&path).map_err(|_| RecoveryError::CandidateFenceInvalid)?;
    let mut expected_counts = manifest.counts.clone();
    expected_counts.ledger_events = expected_counts
        .ledger_events
        .checked_add(2)
        .ok_or(RecoveryError::CandidateFenceInvalid)?;
    expected_counts.audit_events = expected_counts
        .audit_events
        .checked_add(1)
        .ok_or(RecoveryError::CandidateFenceInvalid)?;
    if facts.edge_node_id != manifest.edge_node_id
        || facts.ledger_epoch != manifest.ledger_epoch
        || facts.accepted_cursor != manifest.accepted_cursor
        || facts.allocation_high_water != manifest.allocation_high_water
        || facts.schema_version != manifest.schema_version
        || facts.counts != expected_counts
    {
        return Err(RecoveryError::CandidateConflict);
    }
    verify_candidate_authority(&conn, handoff, manifest)?;
    let receipt = read_candidate_receipt(&conn)?;
    let provenance = read_candidate_provenance(&conn)?;
    if !matches!(
        startup_mode(&conn),
        Ok(RecoveryStartupMode::FencedCandidate { .. })
    ) || receipt.recovery_id != handoff.recovery_id
        || handoff.expected_backup_id.as_deref() != Some(receipt.backup_id.as_str())
        || receipt.backup_id != manifest.backup_id
        || receipt.edge_id != handoff.edge_id
        || receipt.edge_node_id != handoff.edge_node_id
        || receipt.edge_node_id != manifest.edge_node_id
        || handoff.schema_version != 1
        || receipt.old_ledger_epoch != handoff.old_ledger_epoch
        || receipt.old_ledger_epoch != manifest.ledger_epoch
        || receipt.proposed_new_epoch != handoff.proposed_new_epoch
        || receipt.credential_generation != handoff.credential_generation
        || i64::try_from(manifest.database_length).ok() != Some(provenance.database_length)
        || provenance.database_sha256 != manifest.database_sha256
    {
        return Err(RecoveryError::CandidateConflict);
    }
    // The descriptor-backed checks above prove the contents of `file`.  Recheck
    // the configured candidate name immediately before returning so a concurrent
    // rename or hard-link cannot make the published name point at another inode.
    ensure_candidate_name_identity(parent, name, &file)?;
    Ok(receipt)
}

#[cfg(target_os = "linux")]
fn verify_candidate_authority(
    conn: &Connection,
    handoff: &RecoveryHandoff,
    manifest: &NodeBackupManifest,
) -> Result<(), RecoveryError> {
    if iotkit_core_ops::ownership_state(conn).map_err(|_| RecoveryError::CandidateFenceInvalid)?
        != iotkit_core_ops::OwnershipState::LocalRecoveryRequired
    {
        return Err(RecoveryError::CandidateConflict);
    }
    let identity = iotkit_core_ledger::load_edge_node_identity(conn)
        .map_err(|_| RecoveryError::CandidateFenceInvalid)?;
    if identity.edge_node_id != manifest.edge_node_id
        || identity.ledger_epoch != manifest.ledger_epoch
    {
        return Err(RecoveryError::CandidateConflict);
    }
    let (state, edge_id, activation_epoch): (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT state, edge_id, ledger_epoch
             FROM edge_node_activation WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| RecoveryError::CandidateFenceInvalid)?;
    if state != "active"
        || edge_id.as_deref() != Some(handoff.edge_id.as_str())
        || activation_epoch.as_deref() != Some(manifest.ledger_epoch.as_str())
    {
        return Err(RecoveryError::CandidateConflict);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_no_sidecars_at(
    parent: &DirectoryCapability,
    name: &CString,
) -> Result<(), RecoveryError> {
    for suffix in [b"-wal".as_slice(), b"-shm", b"-journal"] {
        let mut sidecar = name.as_bytes().to_vec();
        sidecar.extend_from_slice(suffix);
        let sidecar = CString::new(sidecar).map_err(|_| RecoveryError::CandidateFenceInvalid)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                sidecar.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd >= 0 {
            drop(unsafe { File::from_raw_fd(fd) });
            return Err(RecoveryError::CandidateFenceInvalid);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            continue;
        }
        if error.raw_os_error() == Some(libc::ELOOP) {
            return Err(RecoveryError::CandidateConflict);
        }
        return Err(RecoveryError::Storage);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
struct CandidateProvenance {
    database_length: i64,
    database_sha256: String,
}

#[cfg(target_os = "linux")]
fn read_candidate_provenance(conn: &Connection) -> Result<CandidateProvenance, RecoveryError> {
    let (database_length, database_sha256): (Option<i64>, Option<String>) = conn
        .query_row(
            "SELECT source_database_length, source_database_sha256
             FROM edge_node_recovery_candidate WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| RecoveryError::CandidateFenceInvalid)?;
    let (Some(database_length), Some(database_sha256)) = (database_length, database_sha256) else {
        return Err(RecoveryError::CandidateFenceInvalid);
    };
    if database_length < 0 || !valid_digest(&database_sha256) {
        return Err(RecoveryError::CandidateFenceInvalid);
    }
    Ok(CandidateProvenance {
        database_length,
        database_sha256,
    })
}

#[cfg(target_os = "linux")]
fn read_candidate_receipt(conn: &Connection) -> Result<RestoreReceipt, RecoveryError> {
    conn.query_row(
        "SELECT recovery_id, candidate_instance_id, backup_id, edge_id, edge_node_id,
                old_ledger_epoch, proposed_new_epoch, credential_generation
         FROM edge_node_recovery_candidate WHERE singleton=1",
        [],
        |row| {
            Ok(RestoreReceipt {
                schema_version: 1,
                status: RestoreStatus::DurablyFencedCandidate,
                recovery_id: row.get(0)?,
                candidate_instance_id: row.get(1)?,
                backup_id: row.get(2)?,
                edge_id: row.get(3)?,
                edge_node_id: row.get(4)?,
                old_ledger_epoch: row.get(5)?,
                proposed_new_epoch: row.get(6)?,
                credential_generation: row.get(7)?,
            })
        },
    )
    .map_err(|_| RecoveryError::CandidateFenceInvalid)
}

#[cfg(target_os = "linux")]
fn replay_existing_candidate(
    file: File,
    parent: &DirectoryCapability,
    name: &CString,
    handoff: &RecoveryHandoff,
    manifest: &NodeBackupManifest,
) -> Result<RestoreReceipt, RecoveryError> {
    // Re-sync before readback.  The final descriptor-relative name check in
    // `read_and_verify_candidate` must be the last filesystem observation
    // before a successful replay is returned.
    parent
        .sync_directory()
        .map_err(|_| RecoveryError::CandidatePublicationUncertain)?;
    let receipt = read_and_verify_candidate(file, parent, name, handoff, manifest)?;
    Ok(receipt)
}

#[cfg(target_os = "linux")]
fn open_existing_candidate(
    parent: &DirectoryCapability,
    name: &CString,
) -> Result<Option<File>, RecoveryError> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        return if error.kind() == std::io::ErrorKind::NotFound {
            Ok(None)
        } else if error.raw_os_error() == Some(libc::ELOOP) {
            Err(RecoveryError::CandidateConflict)
        } else {
            Err(RecoveryError::CandidateExists)
        };
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryError::CandidateConflict)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || (metadata.mode() & 0o077) != 0
    {
        return Err(RecoveryError::CandidateConflict);
    }
    Ok(Some(file))
}

#[cfg(target_os = "linux")]
fn ensure_candidate_name_identity(
    parent: &DirectoryCapability,
    name: &CString,
    expected: &File,
) -> Result<(), RecoveryError> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(RecoveryError::CandidateConflict);
    }
    let current = unsafe { File::from_raw_fd(fd) };
    let metadata = current
        .metadata()
        .map_err(|_| RecoveryError::CandidateConflict)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || (metadata.mode() & 0o077) != 0
        || file_identity(&current)? != file_identity(expected)?
    {
        return Err(RecoveryError::CandidateConflict);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn acquire_candidate_lock(
    parent: &DirectoryCapability,
    candidate_name: &CString,
) -> Result<File, RecoveryError> {
    let mut lock_name = candidate_name.as_bytes().to_vec();
    lock_name.extend_from_slice(b".restore.lock");
    let lock_name = CString::new(lock_name).map_err(|_| RecoveryError::InvalidConfiguration)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            lock_name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(RecoveryError::Storage);
    }
    let lock = unsafe { File::from_raw_fd(fd) };
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(RecoveryError::OperationBusy);
    }
    Ok(lock)
}

#[cfg(target_os = "linux")]
fn publish_noreplace(
    parent: &DirectoryCapability,
    temporary: &CString,
    candidate: &CString,
) -> Result<(), RecoveryError> {
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            parent.as_raw_fd(),
            temporary.as_ptr(),
            parent.as_raw_fd(),
            candidate.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            return Err(RecoveryError::CandidateExists);
        }
        if error.raw_os_error() == Some(libc::ENOSYS) {
            return Err(RecoveryError::PlatformUnsupported);
        }
        return Err(RecoveryError::Storage);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_exact(parent: &DirectoryCapability, name: &CString) -> Result<(), RecoveryError> {
    let result = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(());
        }
        return Err(RecoveryError::ArtifactCleanupFailed);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn normalize_absolute(path: &Path) -> Result<PathBuf, RecoveryError> {
    if !path.is_absolute() {
        return Err(RecoveryError::InvalidConfiguration);
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::RootDir => normalized.push("/"),
            std::path::Component::CurDir => {}
            std::path::Component::Normal(value) => normalized.push(value),
            std::path::Component::ParentDir | std::path::Component::Prefix(_) => {
                return Err(RecoveryError::InvalidConfiguration);
            }
        }
    }
    if normalized.file_name().is_none() || normalized.parent().is_none() {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(normalized)
}

#[cfg(target_os = "linux")]
fn file_name_bytes(path: &Path) -> Result<Vec<u8>, RecoveryError> {
    path.file_name()
        .map(|value| value.as_bytes().to_vec())
        .ok_or(RecoveryError::InvalidConfiguration)
}

#[cfg(target_os = "linux")]
fn file_name_cstring(path: &Path) -> Result<CString, RecoveryError> {
    CString::new(file_name_bytes(path)?).map_err(|_| RecoveryError::InvalidConfiguration)
}

#[cfg(target_os = "linux")]
fn random_name(prefix: &str) -> Result<String, RecoveryError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| RecoveryError::Random)?;
    Ok(format!(".{prefix}-{}", hex(&bytes)))
}

#[cfg(target_os = "linux")]
fn random_id(prefix: &str) -> Result<String, RecoveryError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| RecoveryError::Random)?;
    Ok(format!("{prefix}-{}", hex(&bytes)))
}

#[cfg(target_os = "linux")]
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(target_os = "linux")]
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn directory_identity(directory: &DirectoryCapability) -> Result<(u64, u64), RecoveryError> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::zeroed();
    if unsafe { libc::fstat(directory.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(RecoveryError::Storage);
    }
    let stat = unsafe { stat.assume_init() };
    Ok((stat.st_dev, stat.st_ino))
}

#[cfg(target_os = "linux")]
fn verify_owner_directory(directory: &DirectoryCapability) -> Result<(), RecoveryError> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::zeroed();
    if unsafe { libc::fstat(directory.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(RecoveryError::Storage);
    }
    let stat = unsafe { stat.assume_init() };
    if stat.st_uid != unsafe { libc::geteuid() }
        || (stat.st_mode & libc::S_IFMT) != libc::S_IFDIR
        || (stat.st_mode & 0o077) != 0
    {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn free_bytes(directory: &DirectoryCapability) -> Result<u64, RecoveryError> {
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    if unsafe { libc::fstatvfs(directory.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(RecoveryError::Storage);
    }
    let stat = unsafe { stat.assume_init() };
    stat.f_bavail
        .checked_mul(stat.f_frsize)
        .ok_or(RecoveryError::CapacityOverflow)
}

#[cfg(target_os = "linux")]
fn open_input(path: &Path) -> Result<File, RecoveryError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| RecoveryError::Storage)?;
    let metadata = file.metadata().map_err(|_| RecoveryError::Storage)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || (metadata.mode() & 0o077) != 0
    {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(file)
}

#[cfg(target_os = "linux")]
fn file_identity(file: &File) -> Result<(u64, u64), RecoveryError> {
    let metadata = file.metadata().map_err(|_| RecoveryError::Storage)?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(target_os = "linux")]
fn authenticate_input(
    path: &Path,
    passphrase: &BackupPassphrase,
) -> Result<((u64, u64), NodeBackupManifest), RecoveryError> {
    let file = open_input(path)?;
    let identity = file_identity(&file)?;
    let manifest = authenticate_container_file(file, passphrase)?;
    Ok((identity, manifest))
}

#[cfg(target_os = "linux")]
fn verify_staging_tmpfs(directory: &DirectoryCapability) -> Result<(), RecoveryError> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(directory.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(RecoveryError::Storage);
    }
    let stat = unsafe { stat.assume_init() };
    if stat.f_type as u64 != 0x0102_1994 {
        return Err(RecoveryError::DestinationInvalid);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn capture_optional_identity(
    parent: &DirectoryCapability,
    name: &CString,
) -> Result<Option<(u64, u64)>, RecoveryError> {
    let Some(file) = open_live_file(parent, name)? else {
        return Ok(None);
    };
    let metadata = file.metadata().map_err(|_| RecoveryError::Storage)?;
    if !metadata.is_file() {
        return Err(RecoveryError::HandoffMismatch);
    }
    Ok(Some((metadata.dev(), metadata.ino())))
}

#[cfg(target_os = "linux")]
fn open_live_file(
    parent: &DirectoryCapability,
    name: &CString,
) -> Result<Option<File>, RecoveryError> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        return if error.kind() == std::io::ErrorKind::NotFound {
            Ok(None)
        } else if error.raw_os_error() == Some(libc::ELOOP) {
            Err(RecoveryError::HandoffMismatch)
        } else {
            Err(RecoveryError::Storage)
        };
    }
    Ok(Some(unsafe { File::from_raw_fd(fd) }))
}

#[cfg(target_os = "linux")]
fn validate_live_identity(
    parent: &DirectoryCapability,
    name: &CString,
    expected_file: Option<(u64, u64)>,
    manifest: &NodeBackupManifest,
    expected_edge_id: &str,
) -> Result<(), RecoveryError> {
    let actual_file = capture_optional_identity(parent, name)?;
    if actual_file != expected_file {
        return Err(RecoveryError::HandoffMismatch);
    }
    let Some(expected_file) = expected_file else {
        return Ok(());
    };
    let source = open_live_file(parent, name)?.ok_or(RecoveryError::HandoffMismatch)?;
    if file_identity(&source)? != expected_file {
        return Err(RecoveryError::HandoffMismatch);
    }
    let conn = Connection::open_with_flags(
        file_path_from_fd(&source)?,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| RecoveryError::HandoffMismatch)?;
    let identity = iotkit_core_ledger::load_edge_node_identity(&conn)
        .map_err(|_| RecoveryError::HandoffMismatch)?;
    if identity.edge_node_id != manifest.edge_node_id
        || identity.ledger_epoch != manifest.ledger_epoch
    {
        return Err(RecoveryError::HandoffMismatch);
    }
    let active_edge_id: Option<String> = conn
        .query_row(
            "SELECT edge_id FROM edge_node_activation
             WHERE singleton=1 AND state='active'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::HandoffMismatch)?;
    if active_edge_id.as_deref() != Some(expected_edge_id) {
        return Err(RecoveryError::HandoffMismatch);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_live_unchanged(
    parent: &DirectoryCapability,
    name: &CString,
    expected: Option<(u64, u64)>,
) -> Result<(), RecoveryError> {
    if capture_optional_identity(parent, name)? != expected {
        return Err(RecoveryError::CandidatePublicationUncertain);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_no_sidecars(path: &Path) -> Result<(), RecoveryError> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        if PathBuf::from(sidecar).exists() {
            return Err(RecoveryError::CandidateFenceInvalid);
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn file_path_from_fd(file: &File) -> Result<PathBuf, RecoveryError> {
    // Keep the descriptor as the identity anchor.  Resolving the procfs link
    // to its pathname would reopen a potentially substituted inode; SQLite
    // follows this stable fd link directly while the caller holds `file`.
    Ok(PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd())))
}

#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
