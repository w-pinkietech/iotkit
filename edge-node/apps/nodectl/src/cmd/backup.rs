use clap::Subcommand;
use iotkit_core_recovery::{
    BackupConfig, BackupConfigReplace, BackupPairPhase, BackupPairRecord, BackupReadiness,
    RecoveryError, RestoreRequest, acquire_recovery_operation, backup_status,
    configure_backup_guarded_with_pre_publish, create_backup_from_files, inspect_backup,
    load_owner_only_handoff, load_owner_only_passphrase, restore_candidate,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "linux")]
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
};

#[cfg(target_os = "linux")]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[cfg(target_os = "linux")]
use sha2::{Digest, Sha256};

type AppResult<T> = Result<T, CliBackupError>;

#[derive(Subcommand)]
pub enum BackupCommand {
    Configure(ConfigureArgs),
    Create(CreateArgs),
    Inspect(InspectArgs),
    Status(StatusArgs),
    Restore(RestoreArgs),
}

#[derive(clap::Args)]
pub struct ConfigureArgs {
    #[arg(long)]
    pub config: PathBuf,
    #[arg(long)]
    pub db: PathBuf,
    #[arg(long)]
    pub destination: PathBuf,
    #[arg(long = "staging-directory")]
    pub staging_directory: PathBuf,
    #[arg(long = "passphrase-file")]
    pub passphrase_file: PathBuf,
    #[arg(long)]
    pub freshness_seconds: u64,
    #[arg(long)]
    pub retention_count: u32,
    #[arg(long = "systemd-drop-in")]
    pub systemd_drop_in: PathBuf,
    #[arg(long)]
    pub replace_existing: bool,
}

#[derive(clap::Args)]
pub struct CreateArgs {
    #[arg(long)]
    pub config: PathBuf,
}

#[derive(clap::Args)]
pub struct InspectArgs {
    #[arg(long)]
    pub input: PathBuf,
    #[arg(long = "passphrase-file")]
    pub passphrase_file: PathBuf,
}

#[derive(clap::Args)]
pub struct StatusArgs {
    #[arg(long)]
    pub config: PathBuf,
}

#[derive(clap::Args)]
pub struct RestoreArgs {
    #[arg(long)]
    pub input: PathBuf,
    #[arg(long = "candidate-db")]
    pub candidate_db: PathBuf,
    #[arg(long = "live-db")]
    pub live_db: PathBuf,
    #[arg(long = "passphrase-file")]
    pub passphrase_file: PathBuf,
    #[arg(long = "recovery-handoff")]
    pub recovery_handoff: PathBuf,
}

#[derive(Debug)]
pub struct CliBackupError {
    reason_code: &'static str,
    action: &'static str,
}

impl CliBackupError {
    fn recovery(error: RecoveryError, action: &'static str) -> Self {
        Self {
            reason_code: error.reason_code(),
            action,
        }
    }

    fn invalid(action: &'static str) -> Self {
        Self {
            reason_code: "invalid_configuration",
            action,
        }
    }
}

impl std::fmt::Display for CliBackupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{{\"error\":{{\"reason_code\":\"{}\",\"action\":\"{}\"}}}}",
            self.reason_code, self.action
        )
    }
}

impl std::error::Error for CliBackupError {}

#[derive(Serialize)]
struct ConfigureOutput {
    status: &'static str,
}

#[derive(Serialize)]
struct CreatedOutput<'a> {
    status: &'static str,
    backup_id: &'a str,
    edge_node_id: &'a str,
    ledger_epoch: &'a str,
    accepted_cursor: i64,
    allocation_high_water: i64,
    created_at_ms: i64,
}

#[derive(Serialize)]
struct InspectOutput<'a> {
    status: &'static str,
    artifact_kind: &'a str,
    format_version: u32,
    backup_id: &'a str,
    edge_node_id: &'a str,
    ledger_epoch: &'a str,
    created_at_ms: i64,
    accepted_cursor: i64,
    allocation_high_water: i64,
    snapshot_mode: &'static str,
    schema_version: u32,
    database_length: u64,
}

#[derive(Serialize)]
struct StatusOutput<'a> {
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_at_ms: Option<i64>,
    #[serde(flatten)]
    artifact: Option<ArtifactOutput>,
}

#[derive(Serialize)]
struct ArtifactOutput {
    backup_id: String,
    edge_node_id: String,
    ledger_epoch: String,
    created_at_ms: i64,
    ciphertext_size: u64,
    accepted_cursor: i64,
    allocation_high_water: i64,
}

pub fn run(command: BackupCommand) -> AppResult<()> {
    match command {
        BackupCommand::Configure(args) => configure(args),
        BackupCommand::Create(args) => create(args),
        BackupCommand::Inspect(args) => inspect(args),
        BackupCommand::Status(args) => status(args),
        BackupCommand::Restore(args) => restore(args),
    }
}

fn configure(args: ConfigureArgs) -> AppResult<()> {
    let config_path = absolute_path(&args.config, "configure")?;
    let drop_in_path = absolute_path(&args.systemd_drop_in, "configure")?;
    let config = BackupConfig {
        schema_version: 1,
        database: absolute_path(&args.db, "configure")?,
        destination: absolute_path(&args.destination, "configure")?,
        staging_directory: absolute_path(&args.staging_directory, "configure")?,
        passphrase_file: absolute_path(&args.passphrase_file, "configure")?,
        expected_mount: iotkit_core_recovery::MountIdentity {
            // configure_backup replaces these values from the held mount
            // record. They must be nonempty in the request so validation can
            // distinguish malformed callers from a missing mount.
            mount_point: absolute_path(&args.destination, "configure")?,
            source: "pending".into(),
            filesystem_type: "pending".into(),
            filesystem_id: "pending".into(),
        },
        freshness_seconds: args.freshness_seconds,
        retention_count: args.retention_count,
    };
    configure_backup_pair(
        &config_path,
        &config,
        &drop_in_path,
        if args.replace_existing {
            BackupConfigReplace::ReplaceExisting
        } else {
            BackupConfigReplace::Refuse
        },
    )
    .map_err(|error| CliBackupError::recovery(error, "check_backup_configuration"))?;
    write_json(&ConfigureOutput {
        status: "configured",
    })
}

#[cfg(target_os = "linux")]
const PAIR_MARKER_MAX_BYTES: u64 = 8 * 1024;
#[cfg(target_os = "linux")]
const PAIR_TARGET_MAX_BYTES: u64 = 64 * 1024;

#[cfg(target_os = "linux")]
type PairPhase = BackupPairPhase;

#[cfg(target_os = "linux")]
type PairMarker = BackupPairRecord;

#[cfg(target_os = "linux")]
trait PairMarkerValidation {
    fn validate_basic(&self, paths: &PairPaths) -> Result<(), RecoveryError>;
}

#[cfg(target_os = "linux")]
impl PairMarkerValidation for PairMarker {
    fn validate_basic(&self, paths: &PairPaths) -> Result<(), RecoveryError> {
        self.validate_for_paths(&paths.config_path, Some(&paths.drop_in_path_hash))
    }
}

#[cfg(target_os = "linux")]
struct PairPaths {
    config_parent: File,
    drop_in_parent: File,
    config_name: CString,
    drop_in_name: CString,
    marker_name: CString,
    completion_name: CString,
    config_path: PathBuf,
    config_path_hash: String,
    drop_in_path_hash: String,
}

#[cfg(target_os = "linux")]
fn configure_backup_pair(
    config_path: &Path,
    config: &BackupConfig,
    drop_in_path: &Path,
    replacement: BackupConfigReplace,
) -> Result<(), RecoveryError> {
    let request_config_hash = canonical_config_hash(config)?;
    let request_drop_in = systemd_drop_in_bytes(&config.expected_mount.mount_point)?;
    let request_drop_in_hash = hex_digest(&request_drop_in);
    let guard = acquire_recovery_operation(config_path)?;
    let paths = PairPaths::open(config_path, drop_in_path)?;
    let marker_exists = paths.marker_exists()?;
    let completion_exists = paths.completion_exists()?;
    if marker_exists && paths.resume_pending(&request_config_hash, &request_drop_in_hash)? {
        return Ok(());
    }
    if completion_exists && paths.resume_completion(&request_config_hash, &request_drop_in_hash)? {
        return Ok(());
    }
    let (old_config_hash, old_drop_in_hash) = paths.preflight(replacement)?;
    let mut marker = PairMarker {
        schema_version: 3,
        txid: pair_txid()?,
        config_path_hash: paths.config_path_hash.clone(),
        drop_in_path_hash: paths.drop_in_path_hash.clone(),
        phase: PairPhase::Prepared,
        request_config_hash,
        request_drop_in_hash,
        config_hash: None,
        drop_in_hash: None,
        config_existed: old_config_hash.is_some(),
        drop_in_existed: old_drop_in_hash.is_some(),
        old_config_hash,
        old_drop_in_hash,
        config_temp_name: None,
        drop_in_temp_name: None,
    };
    paths.validate_phase_state(&marker)?;
    paths.write_marker(&marker, false)?;

    let mut published = false;
    let result = (|| {
        paths.backup_existing(&marker)?;
        pair_fault("after_backup")?;

        configure_backup_guarded_with_pre_publish(
            &guard,
            config_path,
            config,
            BackupConfigReplace::Refuse,
            |temp_name, config_bytes| {
                let persisted: BackupConfig = serde_json::from_slice(config_bytes)
                    .map_err(|_| RecoveryError::InvalidConfiguration)?;
                let drop_in_bytes = systemd_drop_in_bytes(&persisted.expected_mount.mount_point)?;
                marker.config_hash = Some(hex_digest(config_bytes));
                marker.drop_in_hash = Some(hex_digest(&drop_in_bytes));
                marker.config_temp_name = Some(
                    temp_name
                        .to_str()
                        .map_err(|_| RecoveryError::CleanupRequired)?
                        .to_owned(),
                );
                // Retain the exact drop-in bytes before publishing the config.
                // If the process dies after the config rename, recovery can
                // finish using this descriptor-bound temp file without
                // inventing a new artifact or deleting an unproven target.
                let drop_in_temp = paths.prepare_drop_in(&marker.txid, &drop_in_bytes)?;
                marker.drop_in_temp_name = Some(
                    drop_in_temp
                        .to_str()
                        .map_err(|_| RecoveryError::CleanupRequired)?
                        .to_owned(),
                );
                marker.phase = PairPhase::ConfigPublishing;
                paths.validate_phase_state(&marker)?;
                paths.write_marker(&marker, true)?;
                paths.sync_parents()?;
                pair_fault("before_config_rename")
            },
        )?;
        pair_fault("after_config_rename_before_marker")?;
        let config_bytes = paths.read_target(&paths.config_parent, &paths.config_name)?;
        let persisted: BackupConfig = serde_json::from_slice(&config_bytes)
            .map_err(|_| RecoveryError::InvalidConfiguration)?;
        let drop_in_bytes = systemd_drop_in_bytes(&persisted.expected_mount.mount_point)?;
        if marker.config_hash.as_deref() != Some(hex_digest(&config_bytes).as_str())
            || marker.drop_in_hash.as_deref() != Some(hex_digest(&drop_in_bytes).as_str())
        {
            return Err(RecoveryError::CleanupRequired);
        }
        marker.phase = PairPhase::ConfigPublished;
        paths.validate_phase_state(&marker)?;
        paths.write_marker(&marker, true)?;
        pair_fault("after_config_publish")?;

        let drop_in_temp = paths.drop_in_temp_name(&marker.txid);
        if paths
            .target_hash(&paths.drop_in_parent, &drop_in_temp)?
            .as_deref()
            != marker.drop_in_hash.as_deref()
        {
            return Err(RecoveryError::CleanupRequired);
        }
        marker.phase = PairPhase::DropInPublishing;
        paths.validate_phase_state(&marker)?;
        paths.write_marker(&marker, true)?;
        paths.sync_parents()?;
        pair_fault("before_drop_in_rename")?;
        paths.publish_drop_in_temp(&drop_in_temp)?;
        pair_fault("after_drop_in_rename_before_marker")?;
        pair_fault("after_drop_in_publish")?;
        marker.phase = PairPhase::DropInPublished;
        paths.validate_phase_state(&marker)?;
        paths.write_marker(&marker, true)?;
        paths.sync_parents()?;
        pair_fault("after_parent_sync")?;
        marker.phase = PairPhase::Published;
        paths.validate_phase_state(&marker)?;
        paths.write_marker(&marker, true)?;
        published = true;
        pair_fault("after_published_marker")?;
        paths.finalize(&marker)
    })();
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            if published {
                return Err(RecoveryError::CleanupRequired);
            }
            if paths.rollback(&marker).is_ok() {
                Err(error)
            } else {
                Err(RecoveryError::CleanupRequired)
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn configure_backup_pair(
    _config_path: &Path,
    _config: &BackupConfig,
    _drop_in_path: &Path,
    _replacement: BackupConfigReplace,
) -> Result<(), RecoveryError> {
    Err(RecoveryError::PlatformUnsupported)
}

#[cfg(target_os = "linux")]
impl PairPaths {
    fn open(config_path: &Path, drop_in_path: &Path) -> Result<Self, RecoveryError> {
        if !is_absolute_normalized(config_path) || !is_absolute_normalized(drop_in_path) {
            return Err(RecoveryError::InvalidConfiguration);
        }
        let config_parent_path = config_path
            .parent()
            .ok_or(RecoveryError::InvalidConfiguration)?;
        let drop_in_parent_path = drop_in_path
            .parent()
            .ok_or(RecoveryError::InvalidConfiguration)?;
        let config_parent = open_owner_directory(config_parent_path)?;
        let drop_in_parent = open_owner_directory(drop_in_parent_path)?;
        let config_name = c_name(
            config_path
                .file_name()
                .ok_or(RecoveryError::InvalidConfiguration)?,
        )?;
        let drop_in_name = c_name(
            drop_in_path
                .file_name()
                .ok_or(RecoveryError::InvalidConfiguration)?,
        )?;
        let marker_name = CString::new(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME)
            .map_err(|_| RecoveryError::InvalidConfiguration)?;
        let completion_name = CString::new(iotkit_core_recovery::BACKUP_PAIR_COMPLETION_NAME)
            .map_err(|_| RecoveryError::InvalidConfiguration)?;
        Ok(Self {
            config_parent,
            drop_in_parent,
            config_name,
            drop_in_name,
            marker_name,
            completion_name,
            config_path: config_path.to_path_buf(),
            config_path_hash: path_hash(config_path),
            drop_in_path_hash: path_hash(drop_in_path),
        })
    }

    fn preflight(
        &self,
        replacement: BackupConfigReplace,
    ) -> Result<(Option<String>, Option<String>), RecoveryError> {
        let config_hash = self.target_hash(&self.config_parent, &self.config_name)?;
        let drop_in_hash = self.target_hash(&self.drop_in_parent, &self.drop_in_name)?;
        if replacement == BackupConfigReplace::Refuse
            && (config_hash.is_some() || drop_in_hash.is_some())
        {
            return Err(RecoveryError::DestinationExists);
        }
        Ok((config_hash, drop_in_hash))
    }

    fn marker_exists(&self) -> Result<bool, RecoveryError> {
        match self.open_target(&self.config_parent, &self.marker_name)? {
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }

    fn completion_exists(&self) -> Result<bool, RecoveryError> {
        match self.open_target(&self.config_parent, &self.completion_name)? {
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }

    fn write_marker(
        &self,
        marker: &PairMarker,
        replace_existing: bool,
    ) -> Result<(), RecoveryError> {
        marker.validate_basic(self)?;
        let bytes = serde_json::to_vec(marker).map_err(|_| RecoveryError::InvalidConfiguration)?;
        if bytes.len() as u64 > PAIR_MARKER_MAX_BYTES {
            return Err(RecoveryError::InvalidConfiguration);
        }
        let temp_name = pair_name(&marker.txid, "marker.tmp")?;
        self.write_temp(&self.config_parent, &temp_name, &bytes)?;
        let result = if replace_existing {
            rename_exchange(
                self.config_parent.as_raw_fd(),
                &temp_name,
                self.config_parent.as_raw_fd(),
                &self.marker_name,
            )
        } else {
            rename_noreplace(
                self.config_parent.as_raw_fd(),
                &temp_name,
                self.config_parent.as_raw_fd(),
                &self.marker_name,
            )
        };
        if result.is_err() {
            let _ = unlink_name(&self.config_parent, &temp_name);
            return Err(RecoveryError::Storage);
        }
        if replace_existing {
            unlink_name(&self.config_parent, &temp_name)?;
        }
        self.config_parent
            .sync_all()
            .map_err(|_| RecoveryError::Storage)
    }

    fn read_marker(&self) -> Result<PairMarker, RecoveryError> {
        let file = self
            .open_target(&self.config_parent, &self.marker_name)?
            .ok_or(RecoveryError::CleanupRequired)?;
        let mut bytes = Vec::new();
        file.take(PAIR_MARKER_MAX_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| RecoveryError::CleanupRequired)?;
        if bytes.len() as u64 > PAIR_MARKER_MAX_BYTES {
            return Err(RecoveryError::CleanupRequired);
        }
        let marker: PairMarker =
            serde_json::from_slice(&bytes).map_err(|_| RecoveryError::CleanupRequired)?;
        marker.validate_basic(self)?;
        Ok(marker)
    }

    fn read_completion(&self) -> Result<PairMarker, RecoveryError> {
        let file = self
            .open_target(&self.config_parent, &self.completion_name)?
            .ok_or(RecoveryError::CleanupRequired)?;
        let mut bytes = Vec::new();
        file.take(PAIR_MARKER_MAX_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| RecoveryError::CleanupRequired)?;
        if bytes.len() as u64 > PAIR_MARKER_MAX_BYTES {
            return Err(RecoveryError::CleanupRequired);
        }
        let marker: PairMarker =
            serde_json::from_slice(&bytes).map_err(|_| RecoveryError::CleanupRequired)?;
        marker.validate_basic(self)?;
        if marker.phase != PairPhase::Published {
            return Err(RecoveryError::CleanupRequired);
        }
        Ok(marker)
    }

    fn resume_pending(
        &self,
        request_config_hash: &str,
        request_drop_in_hash: &str,
    ) -> Result<bool, RecoveryError> {
        let marker = self.read_marker()?;
        let completion = if self.completion_exists()? {
            Some(self.read_completion()?)
        } else {
            None
        };
        if let Some(completion) = completion.as_ref() {
            if self.is_post_commit_cleanup(&marker, completion)? {
                unlink_name(&self.config_parent, &self.marker_name)?;
                self.config_parent
                    .sync_all()
                    .map_err(|_| RecoveryError::CleanupRequired)?;
                return Ok(completion.request_config_hash == request_config_hash
                    && completion.request_drop_in_hash == request_drop_in_hash);
            }
            if !prior_completion_matches_marker(completion, &marker) {
                return Err(RecoveryError::CleanupRequired);
            }
        }
        let same_request = marker.request_config_hash == request_config_hash
            && marker.request_drop_in_hash == request_drop_in_hash;
        match marker.phase {
            PairPhase::Published => {
                self.validate_phase_state(&marker)?;
                self.finalize(&marker)?;
                if same_request {
                    return Ok(true);
                }
            }
            _ => return self.recover_pending(marker, same_request),
        }
        Ok(false)
    }

    fn resume_completion(
        &self,
        request_config_hash: &str,
        request_drop_in_hash: &str,
    ) -> Result<bool, RecoveryError> {
        let receipt = self.read_completion()?;
        let same_request = receipt.request_config_hash == request_config_hash
            && receipt.request_drop_in_hash == request_drop_in_hash;
        self.validate_phase_state(&receipt)?;
        Ok(same_request)
    }

    fn is_post_commit_cleanup(
        &self,
        marker: &PairMarker,
        completion: &PairMarker,
    ) -> Result<bool, RecoveryError> {
        if marker.phase != PairPhase::Published
            || completion.phase != PairPhase::Published
            || !prior_completion_matches_marker(marker, completion)
        {
            return Ok(false);
        }
        self.validate_phase_state(completion)?;
        Ok(true)
    }

    fn recover_pending(
        &self,
        marker: PairMarker,
        same_request: bool,
    ) -> Result<bool, RecoveryError> {
        self.validate_phase_state(&marker)?;
        match marker.phase {
            PairPhase::ConfigPublishing => {
                self.recover_config_publishing(&marker)?;
                self.advance_after_config(&marker)?;
            }
            PairPhase::ConfigPublished => self.advance_after_config(&marker)?,
            PairPhase::DropInPublishing => {
                self.recover_drop_in_publishing(&marker)?;
                self.advance_after_drop_in(&marker)?;
            }
            PairPhase::DropInPublished => self.advance_after_drop_in(&marker)?,
            PairPhase::Prepared => {
                // No new target has crossed its publication boundary. Restore
                // the receipt-proven old pair, remove the pending marker, and
                // let the caller re-enter preflight with its current explicit
                // replacement policy.
                self.rollback(&marker)?;
                return Ok(false);
            }
            PairPhase::Published => unreachable!(),
        }
        let updated = self.read_marker()?;
        if updated.phase != PairPhase::Published {
            return Err(RecoveryError::CleanupRequired);
        }
        self.finalize(&updated)?;
        Ok(same_request)
    }

    fn recover_config_publishing(&self, marker: &PairMarker) -> Result<(), RecoveryError> {
        let temp_name = c_name(std::ffi::OsStr::new(
            marker
                .config_temp_name
                .as_deref()
                .ok_or(RecoveryError::CleanupRequired)?,
        ))?;
        let target_hash = self.target_hash(&self.config_parent, &self.config_name)?;
        let temp_hash = self.target_hash(&self.config_parent, &temp_name)?;
        match (
            target_hash.as_deref(),
            temp_hash.as_deref(),
            marker.config_hash.as_deref(),
        ) {
            (Some(actual), None, Some(expected)) if actual == expected => Ok(()),
            (None, Some(actual), Some(expected)) if actual == expected => {
                rename_noreplace(
                    self.config_parent.as_raw_fd(),
                    &temp_name,
                    self.config_parent.as_raw_fd(),
                    &self.config_name,
                )
                .map_err(|_| RecoveryError::CleanupRequired)?;
                self.config_parent
                    .sync_all()
                    .map_err(|_| RecoveryError::CleanupRequired)
            }
            _ => Err(RecoveryError::CleanupRequired),
        }
    }

    fn recover_drop_in_publishing(&self, marker: &PairMarker) -> Result<(), RecoveryError> {
        let temp_name = c_name(std::ffi::OsStr::new(
            marker
                .drop_in_temp_name
                .as_deref()
                .ok_or(RecoveryError::CleanupRequired)?,
        ))?;
        let target_hash = self.target_hash(&self.drop_in_parent, &self.drop_in_name)?;
        let temp_hash = self.target_hash(&self.drop_in_parent, &temp_name)?;
        match (
            target_hash.as_deref(),
            temp_hash.as_deref(),
            marker.drop_in_hash.as_deref(),
        ) {
            (Some(actual), None, Some(expected)) if actual == expected => Ok(()),
            (None, Some(actual), Some(expected)) if actual == expected => {
                rename_noreplace(
                    self.drop_in_parent.as_raw_fd(),
                    &temp_name,
                    self.drop_in_parent.as_raw_fd(),
                    &self.drop_in_name,
                )
                .map_err(|_| RecoveryError::CleanupRequired)?;
                self.drop_in_parent
                    .sync_all()
                    .map_err(|_| RecoveryError::CleanupRequired)
            }
            _ => Err(RecoveryError::CleanupRequired),
        }
    }

    fn advance_after_config(&self, _marker: &PairMarker) -> Result<(), RecoveryError> {
        let mut current = self.read_marker()?;
        if current.phase == PairPhase::ConfigPublishing {
            self.validate_phase_state(&current)?;
            current.phase = PairPhase::ConfigPublished;
            self.validate_phase_state(&current)?;
            self.write_marker(&current, true)?;
            self.sync_parents()?;
        }
        if current.phase != PairPhase::ConfigPublished {
            return Ok(());
        }
        // The drop-in temp was prepared before the config rename.  Recovery
        // may only publish that exact retained artifact (or accept an already
        // exact target); it must not synthesize a fresh file from marker data.
        current.phase = PairPhase::DropInPublishing;
        self.validate_phase_state(&current)?;
        self.write_marker(&current, true)?;
        self.sync_parents()?;
        self.recover_drop_in_publishing(&current)?;
        current.phase = PairPhase::DropInPublished;
        self.validate_phase_state(&current)?;
        self.write_marker(&current, true)?;
        self.sync_parents()?;
        current.phase = PairPhase::Published;
        self.validate_phase_state(&current)?;
        self.write_marker(&current, true)?;
        self.sync_parents()
    }

    fn advance_after_drop_in(&self, _marker: &PairMarker) -> Result<(), RecoveryError> {
        let mut current = self.read_marker()?;
        if current.phase == PairPhase::DropInPublishing {
            self.validate_phase_state(&current)?;
            current.phase = PairPhase::DropInPublished;
            self.validate_phase_state(&current)?;
            self.write_marker(&current, true)?;
            self.sync_parents()?;
        }
        if current.phase == PairPhase::DropInPublished {
            self.validate_phase_state(&current)?;
            current.phase = PairPhase::Published;
            self.validate_phase_state(&current)?;
            self.write_marker(&current, true)?;
            self.sync_parents()?;
        }
        Ok(())
    }

    fn backup_existing(&self, marker: &PairMarker) -> Result<(), RecoveryError> {
        self.validate_phase_state(marker)?;
        if marker.old_config_hash.is_some() {
            let backup = pair_name(&marker.txid, "config.old")?;
            rename_noreplace(
                self.config_parent.as_raw_fd(),
                &self.config_name,
                self.config_parent.as_raw_fd(),
                &backup,
            )
            .map_err(|_| RecoveryError::Storage)?;
        }
        if marker.old_drop_in_hash.is_some() {
            let backup = pair_name(&marker.txid, "drop-in.old")?;
            rename_noreplace(
                self.drop_in_parent.as_raw_fd(),
                &self.drop_in_name,
                self.drop_in_parent.as_raw_fd(),
                &backup,
            )
            .map_err(|_| RecoveryError::Storage)?;
        }
        self.sync_parents()
    }

    fn prepare_drop_in(&self, txid: &str, bytes: &[u8]) -> Result<CString, RecoveryError> {
        let temp_name = pair_name(txid, "drop-in.tmp")?;
        self.write_temp(&self.drop_in_parent, &temp_name, bytes)?;
        Ok(temp_name)
    }

    fn publish_drop_in_temp(&self, temp_name: &CString) -> Result<(), RecoveryError> {
        rename_noreplace(
            self.drop_in_parent.as_raw_fd(),
            temp_name,
            self.drop_in_parent.as_raw_fd(),
            &self.drop_in_name,
        )
        .map_err(|_| {
            let _ = unlink_name(&self.drop_in_parent, temp_name);
            RecoveryError::Storage
        })?;
        self.drop_in_parent
            .sync_all()
            .map_err(|_| RecoveryError::Storage)
    }

    fn drop_in_temp_name(&self, txid: &str) -> CString {
        pair_name(txid, "drop-in.tmp").expect("static pair suffix cannot contain NUL")
    }

    fn write_temp(&self, parent: &File, name: &CString, bytes: &[u8]) -> Result<(), RecoveryError> {
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if fd < 0 {
            return Err(RecoveryError::Storage);
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let result = file
            .write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| RecoveryError::Storage);
        if result.is_err() {
            let _ = unlink_name(parent, name);
        }
        result
    }

    fn sync_parents(&self) -> Result<(), RecoveryError> {
        self.config_parent
            .sync_all()
            .map_err(|_| RecoveryError::Storage)?;
        if self.config_parent.as_raw_fd() != self.drop_in_parent.as_raw_fd() {
            self.drop_in_parent
                .sync_all()
                .map_err(|_| RecoveryError::Storage)?;
        }
        Ok(())
    }

    fn finalize(&self, marker: &PairMarker) -> Result<(), RecoveryError> {
        if marker.phase != PairPhase::Published {
            return Err(RecoveryError::CleanupRequired);
        }
        self.validate_phase_state(marker)?;
        let config_backup = pair_name(&marker.txid, "config.old")?;
        let drop_in_backup = pair_name(&marker.txid, "drop-in.old")?;
        unlink_name(&self.config_parent, &config_backup)?;
        pair_fault("after_config_backup_unlink")?;
        unlink_name(&self.drop_in_parent, &drop_in_backup)?;
        self.sync_parents()?;
        pair_fault("after_finalize_parent_sync")?;
        if self.completion_exists()? {
            let previous = self.read_completion()?;
            if !prior_completion_matches_marker(&previous, marker) {
                return Err(RecoveryError::CleanupRequired);
            }
            rename_exchange(
                self.config_parent.as_raw_fd(),
                &self.marker_name,
                self.config_parent.as_raw_fd(),
                &self.completion_name,
            )
            .map_err(|_| RecoveryError::CleanupRequired)?;
        } else {
            rename_noreplace(
                self.config_parent.as_raw_fd(),
                &self.marker_name,
                self.config_parent.as_raw_fd(),
                &self.completion_name,
            )
            .map_err(|_| RecoveryError::CleanupRequired)?;
        }
        pair_fault("after_completion_receipt")?;
        self.config_parent
            .sync_all()
            .map_err(|_| RecoveryError::CleanupRequired)?;
        pair_fault("after_completion_receipt_sync")?;
        if self.marker_exists()? {
            unlink_name(&self.config_parent, &self.marker_name)?;
            pair_fault("after_completion_marker_unlink")?;
            self.config_parent
                .sync_all()
                .map_err(|_| RecoveryError::CleanupRequired)?;
            pair_fault("after_completion_marker_unlink_sync")?;
        }
        Ok(())
    }

    fn rollback(&self, marker: &PairMarker) -> Result<(), RecoveryError> {
        if marker.phase == PairPhase::Published {
            return Err(RecoveryError::CleanupRequired);
        }
        self.validate_phase_state(marker)?;
        let config_backup = pair_name(&marker.txid, "config.old")?;
        let drop_in_backup = pair_name(&marker.txid, "drop-in.old")?;
        rollback_target(
            &self.config_parent,
            &self.config_name,
            &config_backup,
            marker.old_config_hash.as_deref(),
            marker.config_hash.as_deref(),
        )?;
        rollback_target(
            &self.drop_in_parent,
            &self.drop_in_name,
            &drop_in_backup,
            marker.old_drop_in_hash.as_deref(),
            marker.drop_in_hash.as_deref(),
        )?;
        if let Some(temp_name) = marker.config_temp_name.as_deref() {
            let temp_name = c_name(Path::new(temp_name).as_os_str())?;
            let _ = unlink_name(&self.config_parent, &temp_name);
        }
        if let Some(temp_name) = marker.drop_in_temp_name.as_deref() {
            let temp_name = c_name(Path::new(temp_name).as_os_str())?;
            let _ = unlink_name(&self.drop_in_parent, &temp_name);
        }
        self.sync_parents()?;
        unlink_name(&self.config_parent, &self.marker_name)?;
        self.config_parent
            .sync_all()
            .map_err(|_| RecoveryError::CleanupRequired)
    }

    fn validate_phase_state(&self, marker: &PairMarker) -> Result<(), RecoveryError> {
        marker.validate_basic(self)?;
        let config_backup = pair_name(&marker.txid, "config.old")?;
        let drop_in_backup = pair_name(&marker.txid, "drop-in.old")?;
        match marker.phase {
            PairPhase::Prepared => {
                self.validate_old_transition(
                    &self.config_parent,
                    &self.config_name,
                    &config_backup,
                    marker.old_config_hash.as_deref(),
                )?;
                self.validate_old_transition(
                    &self.drop_in_parent,
                    &self.drop_in_name,
                    &drop_in_backup,
                    marker.old_drop_in_hash.as_deref(),
                )?;
            }
            PairPhase::ConfigPublishing => {
                self.validate_publishing_transition(
                    &self.config_parent,
                    &self.config_name,
                    &config_backup,
                    marker.config_temp_name.as_deref(),
                    marker.old_config_hash.as_deref(),
                    marker.config_hash.as_deref(),
                )?;
                self.validate_publishing_transition(
                    &self.drop_in_parent,
                    &self.drop_in_name,
                    &drop_in_backup,
                    marker.drop_in_temp_name.as_deref(),
                    marker.old_drop_in_hash.as_deref(),
                    marker.drop_in_hash.as_deref(),
                )?;
            }
            PairPhase::ConfigPublished => {
                self.expect_exact(
                    &self.config_parent,
                    &self.config_name,
                    marker.config_hash.as_deref(),
                )?;
                self.expect_temp_absent(&self.config_parent, marker.config_temp_name.as_deref())?;
                self.validate_backup_presence(
                    &self.config_parent,
                    &config_backup,
                    marker.old_config_hash.as_deref(),
                    true,
                )?;
                self.validate_publishing_transition(
                    &self.drop_in_parent,
                    &self.drop_in_name,
                    &drop_in_backup,
                    marker.drop_in_temp_name.as_deref(),
                    marker.old_drop_in_hash.as_deref(),
                    marker.drop_in_hash.as_deref(),
                )?;
            }
            PairPhase::DropInPublishing => {
                self.expect_exact(
                    &self.config_parent,
                    &self.config_name,
                    marker.config_hash.as_deref(),
                )?;
                self.expect_temp_absent(&self.config_parent, marker.config_temp_name.as_deref())?;
                self.validate_backup_presence(
                    &self.config_parent,
                    &config_backup,
                    marker.old_config_hash.as_deref(),
                    true,
                )?;
                self.validate_publishing_transition(
                    &self.drop_in_parent,
                    &self.drop_in_name,
                    &drop_in_backup,
                    marker.drop_in_temp_name.as_deref(),
                    marker.old_drop_in_hash.as_deref(),
                    marker.drop_in_hash.as_deref(),
                )?;
            }
            PairPhase::DropInPublished => {
                self.expect_exact(
                    &self.config_parent,
                    &self.config_name,
                    marker.config_hash.as_deref(),
                )?;
                self.expect_exact(
                    &self.drop_in_parent,
                    &self.drop_in_name,
                    marker.drop_in_hash.as_deref(),
                )?;
                self.expect_temp_absent(&self.config_parent, marker.config_temp_name.as_deref())?;
                self.expect_temp_absent(&self.drop_in_parent, marker.drop_in_temp_name.as_deref())?;
                self.validate_backup_presence(
                    &self.config_parent,
                    &config_backup,
                    marker.old_config_hash.as_deref(),
                    true,
                )?;
                self.validate_backup_presence(
                    &self.drop_in_parent,
                    &drop_in_backup,
                    marker.old_drop_in_hash.as_deref(),
                    true,
                )?;
            }
            PairPhase::Published => {
                self.expect_exact(
                    &self.config_parent,
                    &self.config_name,
                    marker.config_hash.as_deref(),
                )?;
                self.expect_exact(
                    &self.drop_in_parent,
                    &self.drop_in_name,
                    marker.drop_in_hash.as_deref(),
                )?;
                self.expect_temp_absent(&self.config_parent, marker.config_temp_name.as_deref())?;
                self.expect_temp_absent(&self.drop_in_parent, marker.drop_in_temp_name.as_deref())?;
                self.validate_backup_presence(
                    &self.config_parent,
                    &config_backup,
                    marker.old_config_hash.as_deref(),
                    false,
                )?;
                self.validate_backup_presence(
                    &self.drop_in_parent,
                    &drop_in_backup,
                    marker.old_drop_in_hash.as_deref(),
                    false,
                )?;
            }
        }
        Ok(())
    }

    fn validate_publishing_transition(
        &self,
        parent: &File,
        target: &CString,
        backup: &CString,
        temp_name: Option<&str>,
        old_hash: Option<&str>,
        desired_hash: Option<&str>,
    ) -> Result<(), RecoveryError> {
        let Some(desired_hash) = desired_hash else {
            return Err(RecoveryError::CleanupRequired);
        };
        let Some(temp_name) = temp_name else {
            return Err(RecoveryError::CleanupRequired);
        };
        let temp_name = c_name(Path::new(temp_name).as_os_str())?;
        let target_hash = self.target_hash(parent, target)?;
        let backup_hash = self.target_hash(parent, backup)?;
        let temp_hash = self.target_hash(parent, &temp_name)?;
        match (
            old_hash,
            target_hash.as_deref(),
            backup_hash.as_deref(),
            temp_hash.as_deref(),
        ) {
            (Some(old), None, Some(actual_old), Some(actual_new))
                if actual_old == old && actual_new == desired_hash =>
            {
                Ok(())
            }
            (Some(old), Some(actual_new), Some(actual_old), None)
                if actual_old == old && actual_new == desired_hash =>
            {
                Ok(())
            }
            (None, None, None, Some(actual_new)) if actual_new == desired_hash => Ok(()),
            (None, Some(actual_new), None, None) if actual_new == desired_hash => Ok(()),
            _ => Err(RecoveryError::CleanupRequired),
        }
    }

    fn expect_temp_absent(
        &self,
        parent: &File,
        temp_name: Option<&str>,
    ) -> Result<(), RecoveryError> {
        let Some(temp_name) = temp_name else {
            return Err(RecoveryError::CleanupRequired);
        };
        let temp_name = c_name(Path::new(temp_name).as_os_str())?;
        if self.target_hash(parent, &temp_name)?.is_some() {
            return Err(RecoveryError::CleanupRequired);
        }
        Ok(())
    }

    fn validate_old_transition(
        &self,
        parent: &File,
        target: &CString,
        backup: &CString,
        old_hash: Option<&str>,
    ) -> Result<(), RecoveryError> {
        let target_hash = self
            .target_hash(parent, target)
            .map_err(|_| RecoveryError::CleanupRequired)?;
        let backup_hash = self
            .target_hash(parent, backup)
            .map_err(|_| RecoveryError::CleanupRequired)?;
        match old_hash {
            Some(expected)
                if (target_hash.as_deref() == Some(expected) && backup_hash.is_none())
                    || (target_hash.is_none() && backup_hash.as_deref() == Some(expected)) =>
            {
                Ok(())
            }
            None if target_hash.is_none() && backup_hash.is_none() => Ok(()),
            _ => Err(RecoveryError::CleanupRequired),
        }
    }

    fn validate_backup_presence(
        &self,
        parent: &File,
        backup: &CString,
        old_hash: Option<&str>,
        required: bool,
    ) -> Result<(), RecoveryError> {
        let actual = self
            .target_hash(parent, backup)
            .map_err(|_| RecoveryError::CleanupRequired)?;
        match (old_hash, actual, required) {
            (Some(expected), Some(actual), _) if expected == actual => Ok(()),
            (Some(_), None, false) => Ok(()),
            (None, None, _) => Ok(()),
            _ => Err(RecoveryError::CleanupRequired),
        }
    }

    fn expect_exact(
        &self,
        parent: &File,
        name: &CString,
        expected: Option<&str>,
    ) -> Result<(), RecoveryError> {
        let Some(expected) = expected else {
            return Err(RecoveryError::CleanupRequired);
        };
        let actual = self
            .target_hash(parent, name)
            .map_err(|_| RecoveryError::CleanupRequired)?;
        if actual.as_deref() != Some(expected) {
            return Err(RecoveryError::CleanupRequired);
        }
        Ok(())
    }

    fn open_target(&self, parent: &File, name: &CString) -> Result<Option<File>, RecoveryError> {
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            return if error.kind() == std::io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(RecoveryError::InvalidConfiguration)
            };
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = file
            .metadata()
            .map_err(|_| RecoveryError::InvalidConfiguration)?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(RecoveryError::InvalidConfiguration);
        }
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK) }
                < 0
        {
            return Err(RecoveryError::Storage);
        }
        Ok(Some(file))
    }

    fn read_target(&self, parent: &File, name: &CString) -> Result<Vec<u8>, RecoveryError> {
        let file = self
            .open_target(parent, name)?
            .ok_or(RecoveryError::Storage)?;
        let mut bytes = Vec::new();
        file.take(PAIR_TARGET_MAX_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| RecoveryError::Storage)?;
        if bytes.len() as u64 > PAIR_TARGET_MAX_BYTES {
            return Err(RecoveryError::InvalidConfiguration);
        }
        Ok(bytes)
    }

    fn target_hash(&self, parent: &File, name: &CString) -> Result<Option<String>, RecoveryError> {
        let Some(file) = self.open_target(parent, name)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        file.take(PAIR_TARGET_MAX_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| RecoveryError::InvalidConfiguration)?;
        if bytes.len() as u64 > PAIR_TARGET_MAX_BYTES {
            return Err(RecoveryError::InvalidConfiguration);
        }
        Ok(Some(hex_digest(&bytes)))
    }
}

#[cfg(target_os = "linux")]
fn prior_completion_matches_marker(prior: &PairMarker, marker: &PairMarker) -> bool {
    prior.phase == PairPhase::Published
        && prior.config_path_hash == marker.config_path_hash
        && prior.drop_in_path_hash == marker.drop_in_path_hash
        && marker.old_config_hash.as_deref() == prior.config_hash.as_deref()
        && marker.old_drop_in_hash.as_deref() == prior.drop_in_hash.as_deref()
}

#[cfg(target_os = "linux")]
fn rollback_target(
    parent: &File,
    target: &CString,
    backup: &CString,
    old_hash: Option<&str>,
    new_hash: Option<&str>,
) -> Result<(), RecoveryError> {
    let backup_hash = file_hash_in(parent, backup)?;
    let target_hash = file_hash_in(parent, target)?;
    match old_hash {
        Some(expected_old) => {
            if let Some(actual_backup) = backup_hash.as_deref() {
                if actual_backup != expected_old {
                    return Err(RecoveryError::CleanupRequired);
                }
                if let Some(actual_target) = target_hash.as_deref() {
                    if Some(actual_target) != new_hash {
                        return Err(RecoveryError::CleanupRequired);
                    }
                    unlink_name(parent, target)?;
                }
                rename_noreplace(parent.as_raw_fd(), backup, parent.as_raw_fd(), target)
                    .map_err(|_| RecoveryError::CleanupRequired)?;
            } else if target_hash.as_deref() != Some(expected_old) {
                return Err(RecoveryError::CleanupRequired);
            }
        }
        None => {
            if backup_hash.is_some() {
                return Err(RecoveryError::CleanupRequired);
            }
            if let Some(actual_target) = target_hash.as_deref() {
                if Some(actual_target) != new_hash {
                    return Err(RecoveryError::CleanupRequired);
                }
                unlink_name(parent, target)?;
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn file_hash_in(parent: &File, name: &CString) -> Result<Option<String>, RecoveryError> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(RecoveryError::CleanupRequired)
        };
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryError::CleanupRequired)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(RecoveryError::CleanupRequired);
    }
    let mut bytes = Vec::new();
    file.take(PAIR_TARGET_MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| RecoveryError::CleanupRequired)?;
    if bytes.len() as u64 > PAIR_TARGET_MAX_BYTES {
        return Err(RecoveryError::CleanupRequired);
    }
    Ok(Some(hex_digest(&bytes)))
}

#[cfg(target_os = "linux")]
fn is_absolute_normalized(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|component| {
            !matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
}

#[cfg(target_os = "linux")]
fn c_name(name: &std::ffi::OsStr) -> Result<CString, RecoveryError> {
    CString::new(name.as_bytes()).map_err(|_| RecoveryError::InvalidConfiguration)
}

#[cfg(target_os = "linux")]
fn open_owner_directory(path: &Path) -> Result<File, RecoveryError> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(RecoveryError::InvalidConfiguration);
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(file)
}

#[cfg(target_os = "linux")]
fn pair_name(txid: &str, suffix: &str) -> Result<CString, RecoveryError> {
    CString::new(format!(".iotkit-backup-pair.{txid}.{suffix}"))
        .map_err(|_| RecoveryError::InvalidConfiguration)
}

#[cfg(target_os = "linux")]
fn pair_txid() -> Result<String, RecoveryError> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RecoveryError::Random)?
        .as_nanos();
    Ok(format!("{}-{stamp}", std::process::id()))
}

#[cfg(target_os = "linux")]
fn path_hash(path: &Path) -> String {
    hex_digest(path.as_os_str().as_bytes())
}

#[cfg(target_os = "linux")]
fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(target_os = "linux")]
fn unlink_name(parent: &File, name: &CString) -> Result<(), RecoveryError> {
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } == 0 {
        return Ok(());
    }
    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(RecoveryError::CleanupRequired)
    }
}

#[cfg(target_os = "linux")]
fn rename_noreplace(
    old_parent: libc::c_int,
    old_name: &CString,
    new_parent: libc::c_int,
    new_name: &CString,
) -> std::io::Result<()> {
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            old_parent,
            old_name.as_ptr(),
            new_parent,
            new_name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn rename_exchange(
    old_parent: libc::c_int,
    old_name: &CString,
    new_parent: libc::c_int,
    new_name: &CString,
) -> std::io::Result<()> {
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            old_parent,
            old_name.as_ptr(),
            new_parent,
            new_name.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn pair_fault(phase: &str) -> Result<(), RecoveryError> {
    if std::env::var_os("IOTKIT_TEST_BACKUP_PAIR_PAUSE_PHASE")
        .and_then(|value| value.into_string().ok())
        .as_deref()
        == Some(phase)
    {
        let ready = std::env::var_os("IOTKIT_TEST_BACKUP_PAIR_READY_FILE")
            .ok_or(RecoveryError::InvalidConfiguration)?;
        let continue_path = std::env::var_os("IOTKIT_TEST_BACKUP_PAIR_CONTINUE_FILE")
            .ok_or(RecoveryError::InvalidConfiguration)?;
        std::fs::write(&ready, b"ready").map_err(|_| RecoveryError::Storage)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !std::path::Path::new(&continue_path).exists() {
            if std::time::Instant::now() >= deadline {
                return Err(RecoveryError::Storage);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    if std::env::var_os("IOTKIT_TEST_BACKUP_PAIR_FAIL_PHASE")
        .and_then(|value| value.into_string().ok())
        .as_deref()
        == Some(phase)
    {
        return Err(RecoveryError::Storage);
    }
    if std::env::var_os("IOTKIT_TEST_BACKUP_PAIR_CRASH_PHASE")
        .and_then(|value| value.into_string().ok())
        .as_deref()
        == Some(phase)
    {
        std::process::abort();
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn systemd_drop_in_bytes(mount_point: &Path) -> Result<Vec<u8>, RecoveryError> {
    let encoded = systemd_mount_path(mount_point)?;
    Ok(format!("[Unit]\nRequiresMountsFor={encoded}\n").into_bytes())
}

#[cfg(target_os = "linux")]
fn canonical_config_hash(config: &BackupConfig) -> Result<String, RecoveryError> {
    let bytes = serde_json::to_vec(config).map_err(|_| RecoveryError::InvalidConfiguration)?;
    if bytes.len() as u64 > PAIR_TARGET_MAX_BYTES {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(hex_digest(&bytes))
}

#[cfg(target_os = "linux")]
fn systemd_mount_path(path: &Path) -> Result<String, RecoveryError> {
    let bytes = path.as_os_str().as_bytes();
    if !path.is_absolute() || bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
        return Err(RecoveryError::InvalidConfiguration);
    }
    let mut encoded = String::with_capacity(bytes.len());
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'.') {
            encoded.push(*byte as char);
        } else {
            use std::fmt::Write as _;
            write!(&mut encoded, "\\x{byte:02x}")
                .map_err(|_| RecoveryError::InvalidConfiguration)?;
        }
    }
    Ok(encoded)
}

fn create(args: CreateArgs) -> AppResult<()> {
    let config_path = absolute_path(&args.config, "create")?;
    let manifest = create_backup_from_files(&config_path, now_ms()?)
        .map_err(|error| CliBackupError::recovery(error, "retry_backup"))?;
    write_json(&CreatedOutput {
        status: "created",
        backup_id: &manifest.backup_id,
        edge_node_id: &manifest.edge_node_id,
        ledger_epoch: &manifest.ledger_epoch,
        accepted_cursor: manifest.accepted_cursor,
        allocation_high_water: manifest.allocation_high_water,
        created_at_ms: manifest.created_at_ms,
    })
}

fn inspect(args: InspectArgs) -> AppResult<()> {
    let input = absolute_path(&args.input, "inspect")?;
    let passphrase_path = absolute_path(&args.passphrase_file, "inspect")?;
    let passphrase = load_owner_only_passphrase(&passphrase_path)
        .map_err(|error| CliBackupError::recovery(error, "read_owner_only_passphrase"))?;
    let manifest = inspect_backup(&input, &passphrase)
        .map_err(|error| CliBackupError::recovery(error, "check_backup_artifact"))?;
    let snapshot_mode = match manifest.snapshot_mode {
        iotkit_core_recovery::SnapshotMode::Online => "online",
    };
    write_json(&InspectOutput {
        status: "authenticated",
        artifact_kind: &manifest.artifact_kind,
        format_version: manifest.format_version,
        backup_id: &manifest.backup_id,
        edge_node_id: &manifest.edge_node_id,
        ledger_epoch: &manifest.ledger_epoch,
        created_at_ms: manifest.created_at_ms,
        accepted_cursor: manifest.accepted_cursor,
        allocation_high_water: manifest.allocation_high_water,
        snapshot_mode,
        schema_version: manifest.schema_version,
        database_length: manifest.database_length,
    })
}

fn status(args: StatusArgs) -> AppResult<()> {
    let config_path = absolute_path(&args.config, "status")?;
    let readiness = backup_status(&config_path, now_ms()?)
        .map_err(|error| CliBackupError::recovery(error, "check_backup_status"))?;
    let output = match readiness {
        BackupReadiness::NotConfigured => StatusOutput {
            status: "not_configured",
            reason_code: None,
            observed_at_ms: None,
            artifact: None,
        },
        BackupReadiness::OperationBusy => StatusOutput {
            status: "operation_busy",
            reason_code: None,
            observed_at_ms: None,
            artifact: None,
        },
        BackupReadiness::Healthy { artifact } => StatusOutput {
            status: "healthy",
            reason_code: None,
            observed_at_ms: None,
            artifact: Some(artifact_output(&artifact)),
        },
        BackupReadiness::Stale { artifact } => StatusOutput {
            status: "stale",
            reason_code: None,
            observed_at_ms: None,
            artifact: Some(artifact_output(&artifact)),
        },
        BackupReadiness::Failed {
            reason_code,
            observed_at_ms,
            last_verified,
        } => StatusOutput {
            status: "failed",
            reason_code: Some(reason_code),
            observed_at_ms: Some(observed_at_ms),
            artifact: last_verified.as_ref().map(artifact_output),
        },
    };
    write_json(&output)
}

fn restore(args: RestoreArgs) -> AppResult<()> {
    let passphrase_path = absolute_path(&args.passphrase_file, "restore")?;
    let handoff_path = absolute_path(&args.recovery_handoff, "restore")?;
    let input = absolute_path(&args.input, "restore")?;
    let candidate_database = absolute_path(&args.candidate_db, "restore")?;
    let live_database = absolute_path(&args.live_db, "restore")?;
    let handoff = load_owner_only_handoff(&handoff_path)
        .map_err(|error| CliBackupError::recovery(error, "read_owner_only_handoff"))?;
    let passphrase = load_owner_only_passphrase(&passphrase_path)
        .map_err(|error| CliBackupError::recovery(error, "read_owner_only_passphrase"))?;
    let staging = create_restore_staging()
        .map_err(|error| CliBackupError::recovery(error, "prepare_restore_staging"))?;
    let request = RestoreRequest {
        input,
        candidate_database,
        live_database,
        staging_directory: staging.clone(),
        handoff,
    };
    let result = restore_candidate(&request, &passphrase)
        .map_err(|error| CliBackupError::recovery(error, "resolve_restore_candidate"));
    let cleanup = std::fs::remove_dir(&staging);
    let receipt = match (result, cleanup) {
        (Ok(receipt), Ok(())) => receipt,
        (Ok(_), Err(_)) => {
            return Err(CliBackupError::recovery(
                RecoveryError::ArtifactCleanupFailed,
                "cleanup_restore_staging",
            ));
        }
        (Err(error), _) => return Err(error),
    };
    write_json(&receipt)
}

#[cfg(target_os = "linux")]
fn create_restore_staging() -> Result<PathBuf, RecoveryError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    let base = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RecoveryError::InvalidConfiguration)?
        .as_nanos();
    let path = base.join(format!(
        ".iotkit-edge-restore-{}-{stamp}",
        std::process::id()
    ));
    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(&path).map_err(|_| RecoveryError::Storage)?;
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(_) => {
            let _ = std::fs::remove_dir(&path);
            return Err(RecoveryError::Storage);
        }
    };
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        let _ = std::fs::remove_dir(&path);
        return Err(RecoveryError::InvalidConfiguration);
    }
    if std::env::var_os("IOTKIT_TEST_RESTORE_STAGING_FAIL_AFTER_CREATE").is_some() {
        return match std::fs::remove_dir(&path) {
            Ok(()) => Err(RecoveryError::Storage),
            Err(_) => Err(RecoveryError::ArtifactCleanupFailed),
        };
    }
    Ok(path)
}

#[cfg(not(target_os = "linux"))]
fn create_restore_staging() -> Result<PathBuf, RecoveryError> {
    Err(RecoveryError::PlatformUnsupported)
}

fn artifact_output(artifact: &iotkit_core_recovery::BackupStatusArtifact) -> ArtifactOutput {
    ArtifactOutput {
        backup_id: artifact.backup_id.clone(),
        edge_node_id: artifact.edge_node_id.clone(),
        ledger_epoch: artifact.ledger_epoch.clone(),
        created_at_ms: artifact.created_at_ms,
        ciphertext_size: artifact.artifact_length,
        accepted_cursor: artifact.accepted_cursor,
        allocation_high_water: artifact.allocation_high_water,
    }
}

fn absolute_path(path: &Path, action: &'static str) -> AppResult<PathBuf> {
    if path.is_absolute()
        && path.components().all(|component| {
            !matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        Ok(path.to_path_buf())
    } else {
        Err(CliBackupError::invalid(action))
    }
}

fn now_ms() -> AppResult<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CliBackupError::invalid("read_clock"))?;
    i64::try_from(elapsed.as_millis()).map_err(|_| CliBackupError::invalid("read_clock"))
}

fn write_json<T: Serialize>(value: &T) -> AppResult<()> {
    let output = serde_json::to_string(value).map_err(|_| CliBackupError::invalid("write_json"))?;
    println!("{output}");
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/cmd/backup_tests.rs"]
mod tests;
