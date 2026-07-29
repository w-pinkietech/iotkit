use std::path::{Component, Path};

#[cfg(target_os = "linux")]
use std::{
    ffi::{CStr, CString},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd, RawFd},
};

#[cfg(target_os = "linux")]
use std::os::unix::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
};

#[cfg(target_os = "linux")]
use crate::DirectoryCapability;
use crate::{BackupConfig, BackupPassphrase, RecoveryError, RecoveryHandoff};

#[cfg(target_os = "linux")]
const CONFIG_MAX_BYTES: u64 = 64 * 1024;

/// Name of the owner-only marker used while configuration and its systemd
/// drop-in are being published as one recovery operation.
pub const BACKUP_PAIR_MARKER_NAME: &str = ".iotkit-backup-pair.txn";

/// Explicit configuration replacement policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupConfigReplace {
    Refuse,
    ReplaceExisting,
}

/// Held nonblocking lease for one supported recovery operation.
pub struct RecoveryOperationGuard {
    #[cfg(target_os = "linux")]
    _lock: File,
    #[cfg(target_os = "linux")]
    parent_device: u64,
    #[cfg(target_os = "linux")]
    parent_inode: u64,
}

/// Held nonblocking shared observation lease for read-only recovery status.
pub struct RecoveryObservationGuard {
    #[cfg(target_os = "linux")]
    _lock: Option<File>,
}

impl RecoveryObservationGuard {
    pub(crate) fn coordinates_existing_lock(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self._lock.is_some()
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
pub(crate) struct ConfigWriteOps {
    pub(crate) after_existing_open: fn(RawFd, &CStr) -> std::io::Result<()>,
    pub(crate) before_cleanup: fn(RawFd, &CStr) -> std::io::Result<()>,
}

#[cfg(target_os = "linux")]
impl ConfigWriteOps {
    pub(crate) fn system() -> Self {
        Self {
            after_existing_open: |_, _| Ok(()),
            before_cleanup: |_, _| Ok(()),
        }
    }
}

/// Writes schema-1 owner-only backup configuration without following its final path component.
pub fn configure_backup(
    path: &Path,
    config: &BackupConfig,
    replacement: BackupConfigReplace,
) -> Result<(), RecoveryError> {
    validate_config_request(config)?;
    let guard = acquire_recovery_operation(path)?;
    configure_backup_guarded(&guard, path, config, replacement)
}

/// Writes configuration while the caller holds the stable recovery operation lease.
pub fn configure_backup_guarded(
    guard: &RecoveryOperationGuard,
    path: &Path,
    config: &BackupConfig,
    replacement: BackupConfigReplace,
) -> Result<(), RecoveryError> {
    validate_config_request(config)?;
    #[cfg(target_os = "linux")]
    {
        let parent_path = path.parent().ok_or(RecoveryError::InvalidConfiguration)?;
        let parent = open_directory(parent_path)?;
        validate_owner_directory_open(&parent)?;
        let metadata = parent
            .metadata()
            .map_err(|_| RecoveryError::InvalidConfiguration)?;
        if metadata.dev() != guard.parent_device || metadata.ino() != guard.parent_inode {
            return Err(RecoveryError::InvalidConfiguration);
        }
        crate::destination::ensure_no_cleanup_leftovers(&parent)?;
        let mountinfo =
            fs::read_to_string("/proc/self/mountinfo").map_err(|_| RecoveryError::MountMissing)?;
        configure_backup_with(
            path,
            config,
            replacement,
            &mountinfo,
            ConfigWriteOps::system(),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (guard, path, replacement);
        Err(RecoveryError::PlatformUnsupported)
    }
}

/// Acquires the stable owner-only config-adjacent recovery operation lock.
pub fn acquire_recovery_operation(
    config_path: &Path,
) -> Result<RecoveryOperationGuard, RecoveryError> {
    if !is_absolute_normalized(config_path) {
        return Err(RecoveryError::InvalidConfiguration);
    }
    #[cfg(target_os = "linux")]
    {
        let parent_path = config_path
            .parent()
            .ok_or(RecoveryError::InvalidConfiguration)?;
        let parent = open_directory(parent_path)?;
        validate_owner_directory_open(&parent)?;
        let (lock, created) = open_lock_file(parent.as_raw_fd(), c".iotkit-recovery.lock")?;
        validate_lock_file(&lock)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            return if matches!(
                error.raw_os_error(),
                Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
            ) {
                Err(RecoveryError::OperationBusy)
            } else {
                Err(RecoveryError::Storage)
            };
        }
        if created {
            parent.sync_all().map_err(|_| RecoveryError::Storage)?;
        }
        let metadata = parent
            .metadata()
            .map_err(|_| RecoveryError::InvalidConfiguration)?;
        Ok(RecoveryOperationGuard {
            _lock: lock,
            parent_device: metadata.dev(),
            parent_inode: metadata.ino(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config_path;
        Err(RecoveryError::PlatformUnsupported)
    }
}

#[cfg(target_os = "linux")]
impl RecoveryOperationGuard {
    pub(crate) fn ensure_parent(&self, parent: &DirectoryCapability) -> Result<(), RecoveryError> {
        if parent.identity()? != (self.parent_device, self.parent_inode) {
            return Err(RecoveryError::InvalidConfiguration);
        }
        Ok(())
    }
}

/// Observes the stable recovery lease without creating any filesystem entry.
pub fn acquire_recovery_observation(
    config_path: &Path,
) -> Result<RecoveryObservationGuard, RecoveryError> {
    if !is_absolute_normalized(config_path) {
        return Err(RecoveryError::InvalidConfiguration);
    }
    #[cfg(target_os = "linux")]
    {
        let parent_path = config_path
            .parent()
            .ok_or(RecoveryError::InvalidConfiguration)?;
        let parent = open_directory(parent_path)?;
        validate_owner_directory_open(&parent)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                c".iotkit-recovery.lock".as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            return if error.kind() == std::io::ErrorKind::NotFound {
                Ok(RecoveryObservationGuard { _lock: None })
            } else {
                Err(RecoveryError::Storage)
            };
        }
        let lock = unsafe { File::from_raw_fd(fd) };
        validate_lock_file(&lock)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            return if matches!(
                error.raw_os_error(),
                Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
            ) {
                Err(RecoveryError::OperationBusy)
            } else {
                Err(RecoveryError::Storage)
            };
        }
        Ok(RecoveryObservationGuard { _lock: Some(lock) })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config_path;
        Err(RecoveryError::PlatformUnsupported)
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn configure_backup_with(
    path: &Path,
    config: &BackupConfig,
    replacement: BackupConfigReplace,
    mountinfo: &str,
    ops: ConfigWriteOps,
) -> Result<(), RecoveryError> {
    validate_config_request(config)?;
    if !is_absolute_normalized(path) {
        return Err(RecoveryError::InvalidConfiguration);
    }
    let mut persisted = config.clone();
    persisted.expected_mount =
        crate::destination::derive_mount_identity(&config.destination, mountinfo)?;
    validate_config(&persisted)?;
    let bytes = serde_json::to_vec(&persisted).map_err(|_| RecoveryError::InvalidConfiguration)?;
    if bytes.len() as u64 > CONFIG_MAX_BYTES {
        return Err(RecoveryError::InvalidConfiguration);
    }

    let parent_path = path.parent().ok_or(RecoveryError::InvalidConfiguration)?;
    let destination_name = c_name(file_name(path)?)?;
    let parent = open_directory(parent_path)?;
    validate_owner_directory_open(&parent)?;
    let temporary_name = random_sibling_name(&destination_name)?;
    let temporary_fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            temporary_name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if temporary_fd < 0 {
        return Err(RecoveryError::Storage);
    }
    let mut output = unsafe { File::from_raw_fd(temporary_fd) };
    let output_result = (|| {
        output
            .write_all(&bytes)
            .map_err(|_| RecoveryError::Storage)?;
        output.sync_all().map_err(|_| RecoveryError::Storage)?;
        match replacement {
            BackupConfigReplace::Refuse => {
                renameat2(
                    parent.as_raw_fd(),
                    &temporary_name,
                    parent.as_raw_fd(),
                    &destination_name,
                    libc::RENAME_NOREPLACE,
                )
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        RecoveryError::DestinationExists
                    } else {
                        RecoveryError::Storage
                    }
                })?;
            }
            BackupConfigReplace::ReplaceExisting => {
                replace_existing_config(&parent, &temporary_name, &destination_name, &output, ops)?;
            }
        }
        parent.sync_all().map_err(|_| RecoveryError::Storage)
    })();
    if let Err(error) = output_result {
        if output
            .metadata()
            .map_err(|_| RecoveryError::ArtifactCleanupFailed)?
            .nlink()
            == 0
        {
            return Err(error);
        }
        if (ops.before_cleanup)(parent.as_raw_fd(), &temporary_name).is_err()
            || crate::destination::remove_exact_file_at(
                parent.as_raw_fd(),
                &temporary_name,
                &output,
            )
            .is_err()
        {
            return Err(RecoveryError::ArtifactCleanupFailed);
        }
        parent
            .sync_all()
            .map_err(|_| RecoveryError::ArtifactCleanupFailed)?;
        return Err(error);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn replace_existing_config(
    parent: &File,
    temporary_name: &CStr,
    destination_name: &CStr,
    output: &File,
    ops: ConfigWriteOps,
) -> Result<(), RecoveryError> {
    let existing_fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            destination_name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if existing_fd < 0 {
        return Err(RecoveryError::InvalidConfiguration);
    }
    let existing = unsafe { File::from_raw_fd(existing_fd) };
    validate_owner_file_open(&existing, CONFIG_MAX_BYTES)?;
    (ops.after_existing_open)(parent.as_raw_fd(), destination_name)
        .map_err(|_| RecoveryError::Storage)?;

    renameat2(
        parent.as_raw_fd(),
        temporary_name,
        parent.as_raw_fd(),
        destination_name,
        libc::RENAME_EXCHANGE,
    )
    .map_err(|_| RecoveryError::Storage)?;

    (ops.before_cleanup)(parent.as_raw_fd(), temporary_name)
        .map_err(|_| RecoveryError::ArtifactCleanupFailed)?;
    match crate::destination::remove_exact_file_at(parent.as_raw_fd(), temporary_name, &existing) {
        Ok(()) => Ok(()),
        Err(crate::destination::ExactCleanupError::MismatchRestored) => {
            rollback_exchange(parent, temporary_name, destination_name, output)?;
            Err(RecoveryError::InvalidConfiguration)
        }
        Err(crate::destination::ExactCleanupError::Uncertain) => {
            Err(RecoveryError::ArtifactCleanupFailed)
        }
    }
}

#[cfg(target_os = "linux")]
fn rollback_exchange(
    parent: &File,
    temporary_name: &CStr,
    destination_name: &CStr,
    output: &File,
) -> Result<(), RecoveryError> {
    renameat2(
        parent.as_raw_fd(),
        temporary_name,
        parent.as_raw_fd(),
        destination_name,
        libc::RENAME_EXCHANGE,
    )
    .map_err(|_| RecoveryError::Storage)?;
    crate::destination::remove_exact_file_at(parent.as_raw_fd(), temporary_name, output)
        .map_err(|_| RecoveryError::ArtifactCleanupFailed)?;
    parent.sync_all().map_err(|_| RecoveryError::Storage)
}

/// Loads a bounded schema-1 configuration from an owner-only regular file.
pub fn load_owner_only_config(path: &Path) -> Result<BackupConfig, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        if pending_pair_marker(path)? {
            return Err(RecoveryError::CleanupRequired);
        }
        let bytes = read_owner_file(path, CONFIG_MAX_BYTES)?;
        let config =
            serde_json::from_slice(&bytes).map_err(|_| RecoveryError::InvalidConfiguration)?;
        validate_config(&config)?;
        Ok(config)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(RecoveryError::PlatformUnsupported)
    }
}

/// Loads a bounded UTF-8 passphrase from an owner-only regular file.
pub fn load_owner_only_passphrase(path: &Path) -> Result<BackupPassphrase, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        let bytes = read_owner_file(path, 4_098).map_err(|_| RecoveryError::InvalidPassphrase)?;
        let mut value = String::from_utf8(bytes).map_err(|_| RecoveryError::InvalidPassphrase)?;
        if value.ends_with('\n') {
            value.pop();
            if value.ends_with('\r') {
                value.pop();
            }
        }
        if value
            .chars()
            .any(|character| character == '\r' || character == '\n' || character == '\0')
        {
            return Err(RecoveryError::InvalidPassphrase);
        }
        let count = value.chars().count();
        if !(12..=1024).contains(&count) {
            return Err(RecoveryError::InvalidPassphrase);
        }
        Ok(BackupPassphrase::new(value))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(RecoveryError::PlatformUnsupported)
    }
}

/// Loads a bounded, closed recovery handoff from an owner-only regular file.
pub fn load_owner_only_handoff(path: &Path) -> Result<RecoveryHandoff, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        let bytes = read_owner_file(path, CONFIG_MAX_BYTES)?;
        let handoff: RecoveryHandoff =
            serde_json::from_slice(&bytes).map_err(|_| RecoveryError::InvalidConfiguration)?;
        if !crate::model::validate_recovery_handoff(&handoff) {
            return Err(RecoveryError::InvalidConfiguration);
        }
        Ok(handoff)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(RecoveryError::PlatformUnsupported)
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn validate_config(config: &BackupConfig) -> Result<(), RecoveryError> {
    validate_config_request(config)?;
    if config.expected_mount.mount_point.as_os_str().is_empty()
        || config.expected_mount.source.is_empty()
        || config.expected_mount.filesystem_type.is_empty()
        || config.expected_mount.filesystem_id.is_empty()
    {
        return Err(RecoveryError::InvalidConfiguration);
    }
    for path in [
        &config.database,
        &config.destination,
        &config.staging_directory,
        &config.passphrase_file,
        &config.expected_mount.mount_point,
    ] {
        if !is_absolute_normalized(path) {
            return Err(RecoveryError::InvalidConfiguration);
        }
    }
    if overlaps(&config.destination, &config.staging_directory)
        || overlaps(&config.destination, &config.database)
        || overlaps(&config.staging_directory, &config.database)
    {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(())
}

fn validate_config_request(config: &BackupConfig) -> Result<(), RecoveryError> {
    if config.schema_version != 1 || config.freshness_seconds == 0 || config.retention_count == 0 {
        return Err(RecoveryError::InvalidConfiguration);
    }
    for path in [
        &config.database,
        &config.destination,
        &config.staging_directory,
        &config.passphrase_file,
    ] {
        if !is_absolute_normalized(path) {
            return Err(RecoveryError::InvalidConfiguration);
        }
    }
    if overlaps(&config.destination, &config.staging_directory)
        || overlaps(&config.destination, &config.database)
        || overlaps(&config.staging_directory, &config.database)
    {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(())
}

fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn is_absolute_normalized(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
}

#[cfg(target_os = "linux")]
fn open_owner_file(path: &Path, limit: u64) -> Result<File, RecoveryError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    validate_owner_file_open(&file, limit)?;
    clear_nonblock(&file)?;
    Ok(file)
}

#[cfg(target_os = "linux")]
fn pending_pair_marker(path: &Path) -> Result<bool, RecoveryError> {
    let parent_path = path.parent().ok_or(RecoveryError::InvalidConfiguration)?;
    let parent = open_directory(parent_path)?;
    validate_owner_directory_open(&parent)?;
    let name =
        CString::new(BACKUP_PAIR_MARKER_NAME).map_err(|_| RecoveryError::InvalidConfiguration)?;
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
            Ok(false)
        } else {
            Err(RecoveryError::CleanupRequired)
        };
    }
    let marker = unsafe { File::from_raw_fd(fd) };
    let metadata = marker
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
fn read_owner_file(path: &Path, limit: u64) -> Result<Vec<u8>, RecoveryError> {
    let file = open_owner_file(path, limit)?;
    let pause_path = std::env::var_os("IOTKIT_TEST_OWNER_FILE_PAUSE_PATH");
    if std::env::var_os("IOTKIT_TEST_OWNER_FILE_PAUSE_AFTER_FSTAT").is_some()
        && pause_path
            .as_deref()
            .is_some_and(|value| Path::new(value) == path)
    {
        let ready = std::env::var_os("IOTKIT_TEST_OWNER_FILE_READY_FILE")
            .ok_or(RecoveryError::InvalidConfiguration)?;
        let proceed = std::env::var_os("IOTKIT_TEST_OWNER_FILE_CONTINUE_FILE")
            .ok_or(RecoveryError::InvalidConfiguration)?;
        std::fs::write(&ready, b"ready").map_err(|_| RecoveryError::Storage)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !std::path::Path::new(&proceed).exists() {
            if std::time::Instant::now() >= deadline {
                return Err(RecoveryError::Storage);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| RecoveryError::Storage)?;
    if bytes.len() as u64 > limit {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
pub(crate) fn clear_nonblock(file: &File) -> Result<(), RecoveryError> {
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(RecoveryError::Storage);
    }
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK) } < 0 {
        return Err(RecoveryError::Storage);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_owner_file_open(file: &File, limit: u64) -> Result<(), RecoveryError> {
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    if !metadata.is_file()
        || metadata.len() > limit
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_owner_directory_open(directory: &File) -> Result<(), RecoveryError> {
    let metadata = directory
        .metadata()
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn file_name(path: &Path) -> Result<&std::ffi::OsStr, RecoveryError> {
    path.file_name()
        .filter(|name| !name.is_empty())
        .ok_or(RecoveryError::InvalidConfiguration)
}

#[cfg(target_os = "linux")]
fn c_name(name: &std::ffi::OsStr) -> Result<CString, RecoveryError> {
    CString::new(name.as_bytes()).map_err(|_| RecoveryError::InvalidConfiguration)
}

#[cfg(target_os = "linux")]
fn open_lock_file(directory_fd: RawFd, name: &CStr) -> Result<(File, bool), RecoveryError> {
    let create_flags =
        libc::O_CREAT | libc::O_EXCL | libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    let created_fd = unsafe { libc::openat(directory_fd, name.as_ptr(), create_flags, 0o600) };
    if created_fd >= 0 {
        return Ok((unsafe { File::from_raw_fd(created_fd) }, true));
    }
    if std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists {
        return Err(RecoveryError::Storage);
    }
    let existing_fd = unsafe {
        libc::openat(
            directory_fd,
            name.as_ptr(),
            libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if existing_fd < 0 {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok((unsafe { File::from_raw_fd(existing_fd) }, false))
}

#[cfg(target_os = "linux")]
fn validate_lock_file(file: &File) -> Result<(), RecoveryError> {
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
    Ok(())
}

#[cfg(target_os = "linux")]
fn random_sibling_name(destination: &CStr) -> Result<CString, RecoveryError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| RecoveryError::Random)?;
    let mut name = format!(".{}.", destination.to_string_lossy());
    use std::fmt::Write as _;
    for byte in bytes {
        write!(&mut name, "{byte:02x}").map_err(|_| RecoveryError::Storage)?;
    }
    name.push_str(".iotkit-config");
    CString::new(name).map_err(|_| RecoveryError::InvalidConfiguration)
}

#[cfg(target_os = "linux")]
fn open_directory(path: &Path) -> Result<File, RecoveryError> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(RecoveryError::InvalidConfiguration)
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(target_os = "linux")]
fn renameat2(
    old_directory: RawFd,
    old_name: &CStr,
    new_directory: RawFd,
    new_name: &CStr,
    flags: u32,
) -> std::io::Result<()> {
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            old_directory,
            old_name.as_ptr(),
            new_directory,
            new_name.as_ptr(),
            flags,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
