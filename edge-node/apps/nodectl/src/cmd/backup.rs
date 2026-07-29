use clap::Subcommand;
use iotkit_core_recovery::{
    BackupConfig, BackupConfigReplace, BackupReadiness, RecoveryError, RestoreRequest,
    acquire_recovery_operation, backup_status, configure_backup_guarded, create_backup,
    inspect_backup, load_owner_only_config, load_owner_only_handoff, load_owner_only_passphrase,
    restore_candidate,
};
use serde::{Deserialize, Serialize};
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
#[derive(Clone, Debug, Deserialize, Serialize)]
struct PairMarker {
    schema_version: u32,
    txid: String,
    config_path_hash: String,
    drop_in_path_hash: String,
    phase: String,
    config_hash: Option<String>,
    drop_in_hash: String,
    config_existed: bool,
    drop_in_existed: bool,
}

#[cfg(target_os = "linux")]
struct PairPaths {
    config_parent: File,
    drop_in_parent: File,
    config_name: CString,
    drop_in_name: CString,
    marker_name: CString,
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
    let guard = acquire_recovery_operation(config_path)?;
    let paths = PairPaths::open(config_path, drop_in_path)?;
    if paths.marker_exists()? && paths.resume_pending()? {
        return Ok(());
    }
    let (config_existed, drop_in_existed) = paths.preflight(replacement)?;
    let mut marker = PairMarker {
        schema_version: 1,
        txid: pair_txid()?,
        config_path_hash: paths.config_path_hash.clone(),
        drop_in_path_hash: paths.drop_in_path_hash.clone(),
        phase: "prepared".into(),
        config_hash: None,
        drop_in_hash: String::new(),
        config_existed,
        drop_in_existed,
    };
    paths.write_marker(&marker, false)?;

    let result = (|| {
        paths.backup_existing(&marker)?;
        pair_fault("after_backup")?;

        configure_backup_guarded(&guard, config_path, config, BackupConfigReplace::Refuse)?;
        pair_fault("after_config_publish")?;
        let config_bytes = paths.read_target(&paths.config_name, PAIR_MARKER_MAX_BYTES)?;
        let persisted: BackupConfig = serde_json::from_slice(&config_bytes)
            .map_err(|_| RecoveryError::InvalidConfiguration)?;
        marker.config_hash = Some(hex_digest(&config_bytes));
        let drop_in_bytes = systemd_drop_in_bytes(&persisted.expected_mount.mount_point)?;
        marker.drop_in_hash = hex_digest(&drop_in_bytes);
        marker.phase = "config_published".into();
        paths.write_marker(&marker, true)?;

        paths.publish_drop_in(&marker.txid, &drop_in_bytes)?;
        pair_fault("after_drop_in_publish")?;
        marker.phase = "drop_in_published".into();
        paths.write_marker(&marker, true)?;
        paths.sync_parents()?;
        pair_fault("after_parent_sync")?;
        marker.phase = "published".into();
        paths.write_marker(&marker, true)?;
        paths.finalize(&marker)
    })();
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
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
        Ok(Self {
            config_parent,
            drop_in_parent,
            config_name,
            drop_in_name,
            marker_name,
            config_path_hash: path_hash(config_path),
            drop_in_path_hash: path_hash(drop_in_path),
        })
    }

    fn preflight(&self, replacement: BackupConfigReplace) -> Result<(bool, bool), RecoveryError> {
        let config_existed = self.target_exists(&self.config_parent, &self.config_name)?;
        let drop_in_existed = self.target_exists(&self.drop_in_parent, &self.drop_in_name)?;
        if replacement == BackupConfigReplace::Refuse && (config_existed || drop_in_existed) {
            return Err(RecoveryError::DestinationExists);
        }
        Ok((config_existed, drop_in_existed))
    }

    fn marker_exists(&self) -> Result<bool, RecoveryError> {
        match self.open_target(&self.config_parent, &self.marker_name)? {
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }

    fn write_marker(
        &self,
        marker: &PairMarker,
        replace_existing: bool,
    ) -> Result<(), RecoveryError> {
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
        if marker.schema_version != 1
            || marker.txid.is_empty()
            || marker.config_path_hash != self.config_path_hash
            || marker.drop_in_path_hash != self.drop_in_path_hash
            || marker.phase.is_empty()
        {
            return Err(RecoveryError::CleanupRequired);
        }
        Ok(marker)
    }

    fn resume_pending(&self) -> Result<bool, RecoveryError> {
        let marker = self.read_marker()?;
        if marker.phase == "published"
            && marker.config_hash.is_some()
            && !marker.drop_in_hash.is_empty()
            && self.hash_matches(
                &self.config_parent,
                &self.config_name,
                marker.config_hash.as_deref(),
            )?
            && self.hash_matches(
                &self.drop_in_parent,
                &self.drop_in_name,
                Some(&marker.drop_in_hash),
            )?
        {
            self.finalize(&marker)?;
            return Ok(true);
        }
        self.rollback(&marker)?;
        Ok(false)
    }

    fn backup_existing(&self, marker: &PairMarker) -> Result<(), RecoveryError> {
        if marker.config_existed {
            let backup = pair_name(&marker.txid, "config.old")?;
            rename_noreplace(
                self.config_parent.as_raw_fd(),
                &self.config_name,
                self.config_parent.as_raw_fd(),
                &backup,
            )
            .map_err(|_| RecoveryError::Storage)?;
        }
        if marker.drop_in_existed {
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

    fn publish_drop_in(&self, txid: &str, bytes: &[u8]) -> Result<(), RecoveryError> {
        let temp_name = pair_name(txid, "drop-in.tmp")?;
        self.write_temp(&self.drop_in_parent, &temp_name, bytes)?;
        rename_noreplace(
            self.drop_in_parent.as_raw_fd(),
            &temp_name,
            self.drop_in_parent.as_raw_fd(),
            &self.drop_in_name,
        )
        .map_err(|_| {
            let _ = unlink_name(&self.drop_in_parent, &temp_name);
            RecoveryError::Storage
        })?;
        self.drop_in_parent
            .sync_all()
            .map_err(|_| RecoveryError::Storage)
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
        let config_backup = pair_name(&marker.txid, "config.old")?;
        let drop_in_backup = pair_name(&marker.txid, "drop-in.old")?;
        unlink_name(&self.config_parent, &config_backup)?;
        unlink_name(&self.drop_in_parent, &drop_in_backup)?;
        self.sync_parents()?;
        unlink_name(&self.config_parent, &self.marker_name)?;
        self.config_parent
            .sync_all()
            .map_err(|_| RecoveryError::Storage)
    }

    fn rollback(&self, marker: &PairMarker) -> Result<(), RecoveryError> {
        let config_backup = pair_name(&marker.txid, "config.old")?;
        let drop_in_backup = pair_name(&marker.txid, "drop-in.old")?;
        rollback_target(
            &self.config_parent,
            &self.config_name,
            &config_backup,
            marker.config_existed,
        )?;
        rollback_target(
            &self.drop_in_parent,
            &self.drop_in_name,
            &drop_in_backup,
            marker.drop_in_existed,
        )?;
        let _ = unlink_name(&self.config_parent, &pair_name(&marker.txid, "config.tmp")?);
        let _ = unlink_name(
            &self.drop_in_parent,
            &pair_name(&marker.txid, "drop-in.tmp")?,
        );
        self.sync_parents()?;
        unlink_name(&self.config_parent, &self.marker_name)?;
        self.config_parent
            .sync_all()
            .map_err(|_| RecoveryError::CleanupRequired)
    }

    fn target_exists(&self, parent: &File, name: &CString) -> Result<bool, RecoveryError> {
        Ok(self.open_target(parent, name)?.is_some())
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

    fn read_target(&self, name: &CString, limit: u64) -> Result<Vec<u8>, RecoveryError> {
        let file = self
            .open_target(&self.config_parent, name)?
            .ok_or(RecoveryError::Storage)?;
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| RecoveryError::Storage)?;
        if bytes.len() as u64 > limit {
            return Err(RecoveryError::InvalidConfiguration);
        }
        Ok(bytes)
    }

    fn hash_matches(
        &self,
        parent: &File,
        name: &CString,
        expected: Option<&str>,
    ) -> Result<bool, RecoveryError> {
        let Some(expected) = expected else {
            return Ok(false);
        };
        let Some(file) = self.open_target(parent, name)? else {
            return Ok(false);
        };
        let mut bytes = Vec::new();
        file.take(PAIR_MARKER_MAX_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| RecoveryError::Storage)?;
        Ok(bytes.len() as u64 <= PAIR_MARKER_MAX_BYTES && hex_digest(&bytes) == expected)
    }
}

#[cfg(target_os = "linux")]
fn rollback_target(
    parent: &File,
    target: &CString,
    backup: &CString,
    originally_existed: bool,
) -> Result<(), RecoveryError> {
    let backup_exists = target_exists_in(parent, backup)?;
    if backup_exists {
        if target_exists_in(parent, target)? {
            unlink_name(parent, target)?;
        }
        rename_noreplace(parent.as_raw_fd(), backup, parent.as_raw_fd(), target)
            .map_err(|_| RecoveryError::CleanupRequired)?;
    } else if originally_existed {
        if !target_exists_in(parent, target)? {
            return Err(RecoveryError::CleanupRequired);
        }
    } else if target_exists_in(parent, target)? {
        unlink_name(parent, target)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn target_exists_in(parent: &File, name: &CString) -> Result<bool, RecoveryError> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            Ok(false)
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
    Ok(true)
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
    let config = load_owner_only_config(&config_path)
        .map_err(|error| CliBackupError::recovery(error, "check_backup_configuration"))?;
    let passphrase = load_owner_only_passphrase(&config.passphrase_file)
        .map_err(|error| CliBackupError::recovery(error, "read_owner_only_passphrase"))?;
    let manifest = create_backup(&config_path, &passphrase, now_ms()?)
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
