use clap::Subcommand;
use iotkit_core_recovery::{
    BackupConfig, BackupConfigReplace, BackupReadiness, RecoveryError, RestoreRequest,
    backup_status, configure_backup, create_backup, inspect_backup, load_owner_only_config,
    load_owner_only_handoff, load_owner_only_passphrase, restore_candidate,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
    configure_backup(
        &absolute_path(&args.config, "configure")?,
        &config,
        if args.replace_existing {
            BackupConfigReplace::ReplaceExisting
        } else {
            BackupConfigReplace::Refuse
        },
    )
    .map_err(|error| CliBackupError::recovery(error, "check_backup_configuration"))?;
    let persisted = load_owner_only_config(&absolute_path(&args.config, "configure")?)
        .map_err(|error| CliBackupError::recovery(error, "check_backup_configuration"))?;
    write_systemd_drop_in(
        &args.systemd_drop_in,
        &persisted.expected_mount.mount_point,
        args.replace_existing,
    )
    .map_err(|error| CliBackupError::recovery(error, "check_systemd_drop_in"))?;
    write_json(&ConfigureOutput {
        status: "configured",
    })
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
    use std::os::unix::fs::PermissionsExt;

    let base = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RecoveryError::InvalidConfiguration)?
        .as_nanos();
    let path = base.join(format!(
        ".iotkit-edge-restore-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir(&path).map_err(|_| RecoveryError::Storage)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| RecoveryError::Storage)?;
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

#[cfg(target_os = "linux")]
fn write_systemd_drop_in(
    path: &Path,
    mount_point: &Path,
    replace_existing: bool,
) -> Result<(), RecoveryError> {
    use std::ffi::CString;
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
        || mount_point.as_os_str().as_bytes().contains(&b'\n')
        || mount_point.as_os_str().as_bytes().contains(&b'\r')
    {
        return Err(RecoveryError::InvalidConfiguration);
    }
    let parent_path = path.parent().ok_or(RecoveryError::InvalidConfiguration)?;
    let parent = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(parent_path)
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    let parent_metadata = parent
        .metadata()
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    if !parent_metadata.is_dir()
        || parent_metadata.uid() != unsafe { libc::geteuid() }
        || parent_metadata.permissions().mode() & 0o077 != 0
    {
        return Err(RecoveryError::InvalidConfiguration);
    }
    let target_name = path
        .file_name()
        .ok_or(RecoveryError::InvalidConfiguration)?;
    let target_name =
        CString::new(target_name.as_bytes()).map_err(|_| RecoveryError::InvalidConfiguration)?;
    let temp_name = CString::new(format!(
        ".{}.{}.tmp",
        target_name.to_string_lossy(),
        std::process::id()
    ))
    .map_err(|_| RecoveryError::InvalidConfiguration)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            temp_name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(RecoveryError::Storage);
    }
    let mut output = unsafe { File::from_raw_fd(fd) };
    let contents = format!("[Unit]\nRequiresMountsFor={}\n", mount_point.display());
    let result = (|| {
        output
            .write_all(contents.as_bytes())
            .map_err(|_| RecoveryError::Storage)?;
        output.sync_all().map_err(|_| RecoveryError::Storage)?;
        let existing = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                target_name.as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if existing >= 0 {
            let existing = unsafe { File::from_raw_fd(existing) };
            let metadata = existing
                .metadata()
                .map_err(|_| RecoveryError::InvalidConfiguration)?;
            if !metadata.is_file()
                || metadata.nlink() != 1
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.permissions().mode() & 0o077 != 0
            {
                return Err(RecoveryError::InvalidConfiguration);
            }
            if !replace_existing {
                return Err(RecoveryError::DestinationExists);
            }
            let exchange = unsafe {
                libc::syscall(
                    libc::SYS_renameat2,
                    parent.as_raw_fd(),
                    temp_name.as_ptr(),
                    parent.as_raw_fd(),
                    target_name.as_ptr(),
                    libc::RENAME_EXCHANGE,
                )
            };
            if exchange != 0 {
                return Err(RecoveryError::Storage);
            }
            if unsafe { libc::unlinkat(parent.as_raw_fd(), temp_name.as_ptr(), 0) } != 0 {
                return Err(RecoveryError::ArtifactCleanupFailed);
            }
            return parent.sync_all().map_err(|_| RecoveryError::Storage);
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound {
            return Err(RecoveryError::InvalidConfiguration);
        }
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                parent.as_raw_fd(),
                temp_name.as_ptr(),
                parent.as_raw_fd(),
                target_name.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::AlreadyExists {
                    RecoveryError::DestinationExists
                } else {
                    RecoveryError::Storage
                },
            );
        }
        parent.sync_all().map_err(|_| RecoveryError::Storage)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(
            path.parent()
                .unwrap()
                .join(temp_name.to_string_lossy().as_ref()),
        );
    }
    result
}

#[cfg(not(target_os = "linux"))]
fn write_systemd_drop_in(
    _path: &Path,
    _mount_point: &Path,
    _replace_existing: bool,
) -> Result<(), RecoveryError> {
    Err(RecoveryError::PlatformUnsupported)
}
