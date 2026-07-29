use std::{collections::BTreeSet, fs::File, path::PathBuf};

#[cfg(target_os = "linux")]
use std::{
    ffi::{CStr, CString},
    fs::OpenOptions,
    io::{self, Read, Write},
    path::Path,
};

#[cfg(target_os = "linux")]
use std::os::{
    fd::{AsRawFd, FromRawFd, RawFd},
    unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
};

#[cfg(target_os = "linux")]
use crate::MountIdentity;
use crate::{
    BackupConfig, BackupPassphrase, DirectoryCapability, NodeBackupManifest, RecoveryError,
};

const MIB: u64 = 1024 * 1024;

/// A decoded Linux mountinfo record, intentionally without a Debug implementation.
#[derive(Clone, PartialEq, Eq)]
pub struct MountInfoEntry {
    pub mount_point: PathBuf,
    pub source: String,
    pub filesystem_type: String,
    pub major: u32,
    pub minor: u32,
}

/// A destination verified once and held for all later descriptor-relative work.
pub struct VerifiedBackupDestination {
    pub(crate) directory: DirectoryCapability,
}

/// A tmpfs staging directory verified once and held for anonymous plaintext work.
pub struct VerifiedStagingDirectory {
    directory: DirectoryCapability,
}

impl VerifiedBackupDestination {
    pub fn capability(&self) -> &DirectoryCapability {
        &self.directory
    }
}

impl VerifiedStagingDirectory {
    pub fn capability(&self) -> &DirectoryCapability {
        &self.directory
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxOperation {
    ProbeAfterCreate,
    ProbeFileSync,
    ProbeRename,
    ProbeReadback,
    ProbeParentSync,
    ProbeCleanupUnlink,
    ProbeCleanupSync,
    PublicationAfterLink,
    RetentionBeforeQuarantine,
    RetentionBeforeCleanup,
}

#[cfg(target_os = "linux")]
pub(crate) trait LinuxOperationHook {
    fn before(
        &self,
        _operation: LinuxOperation,
        _directory_fd: RawFd,
        _name: &CStr,
    ) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
struct SystemHook;

#[cfg(target_os = "linux")]
impl LinuxOperationHook for SystemHook {}

/// Parses Linux mountinfo records, decoding the octal escapes used by procfs.
pub fn parse_mountinfo(input: &str) -> Result<Vec<MountInfoEntry>, RecoveryError> {
    input
        .lines()
        .filter(|line| !line.is_empty())
        .map(parse_line)
        .collect()
}

fn parse_line(line: &str) -> Result<MountInfoEntry, RecoveryError> {
    let fields: Vec<_> = line.split(' ').collect();
    let dash = fields
        .iter()
        .position(|field| *field == "-")
        .ok_or(RecoveryError::MountMissing)?;
    if dash < 6 || fields.len() < dash + 3 {
        return Err(RecoveryError::MountMissing);
    }
    let (major, minor) = fields[2]
        .split_once(':')
        .ok_or(RecoveryError::MountMissing)?;
    Ok(MountInfoEntry {
        mount_point: PathBuf::from(decode_mount_field(fields[4])?),
        source: decode_mount_field(fields[dash + 2])?,
        filesystem_type: fields[dash + 1].to_owned(),
        major: major.parse().map_err(|_| RecoveryError::MountMissing)?,
        minor: minor.parse().map_err(|_| RecoveryError::MountMissing)?,
    })
}

fn decode_mount_field(value: &str) -> Result<String, RecoveryError> {
    let mut decoded = String::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'\\' {
            let digits = [
                bytes.next().ok_or(RecoveryError::MountMissing)?,
                bytes.next().ok_or(RecoveryError::MountMissing)?,
                bytes.next().ok_or(RecoveryError::MountMissing)?,
            ];
            let text = std::str::from_utf8(&digits).map_err(|_| RecoveryError::MountMissing)?;
            let decoded_byte =
                u8::from_str_radix(text, 8).map_err(|_| RecoveryError::MountMissing)?;
            decoded.push(decoded_byte as char);
        } else {
            decoded.push(byte as char);
        }
    }
    Ok(decoded)
}

/// Returns the required free space: input plus max(5%, 64 MiB), without overflow.
pub fn required_capacity(bytes: u64) -> Result<u64, RecoveryError> {
    let five_percent = bytes
        .checked_add(19)
        .ok_or(RecoveryError::CapacityOverflow)?
        / 20;
    bytes
        .checked_add(five_percent.max(64 * MIB))
        .ok_or(RecoveryError::CapacityOverflow)
}

/// Opens the configured destination once, verifies the held descriptor, then probes it.
pub fn verify_destination(
    config: &BackupConfig,
    bytes: u64,
) -> Result<VerifiedBackupDestination, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        crate::config::validate_config(config)?;
        let file = open_directory(&config.destination)?;
        verify_owned_directory(&file)?;
        let entries = parse_mountinfo(
            &std::fs::read_to_string("/proc/self/mountinfo")
                .map_err(|_| RecoveryError::MountMissing)?,
        )?;
        let mount =
            deepest_mount(&entries, &config.destination).ok_or(RecoveryError::MountMissing)?;
        if mount.mount_point != config.expected_mount.mount_point
            || mount.source != config.expected_mount.source
            || mount.filesystem_type != config.expected_mount.filesystem_type
        {
            return Err(RecoveryError::MountMissing);
        }
        let metadata = file.metadata().map_err(|_| RecoveryError::Storage)?;
        if libc::major(metadata.dev()) as u32 != mount.major
            || libc::minor(metadata.dev()) as u32 != mount.minor
        {
            return Err(RecoveryError::MountMissing);
        }
        if filesystem_identity(&file, &mount.source)? != config.expected_mount.filesystem_id {
            return Err(RecoveryError::MountIdentityUnavailable);
        }
        let database = open_regular_file(&config.database)?;
        if same_filesystem(&file, &database)? {
            return Err(RecoveryError::DestinationInvalid);
        }
        if free_bytes(&file)? < required_capacity(bytes)? {
            return Err(RecoveryError::StorageFull);
        }
        let directory = DirectoryCapability::from_open_file(file)?;
        probe_directory(&directory)?;
        Ok(VerifiedBackupDestination { directory })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (config, bytes);
        Err(RecoveryError::PlatformUnsupported)
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn derive_mount_identity(
    destination: &Path,
    mountinfo: &str,
) -> Result<MountIdentity, RecoveryError> {
    let file = open_directory(destination)?;
    verify_owned_directory(&file)?;
    let entries = parse_mountinfo(mountinfo)?;
    let mount = deepest_mount(&entries, destination).ok_or(RecoveryError::MountMissing)?;
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryError::DestinationInvalid)?;
    if libc::major(metadata.dev()) as u32 != mount.major
        || libc::minor(metadata.dev()) as u32 != mount.minor
    {
        return Err(RecoveryError::MountMissing);
    }
    Ok(MountIdentity {
        mount_point: mount.mount_point.clone(),
        source: mount.source.clone(),
        filesystem_type: mount.filesystem_type.clone(),
        filesystem_id: filesystem_identity(&file, &mount.source)?,
    })
}

/// Opens the configured staging directory once and requires a private tmpfs descriptor.
pub fn verify_staging_directory(
    config: &BackupConfig,
    bytes: u64,
) -> Result<VerifiedStagingDirectory, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        crate::config::validate_config(config)?;
        let file = open_directory(&config.staging_directory)?;
        verify_owned_directory(&file)?;
        let mut statfs = std::mem::MaybeUninit::<libc::statfs>::zeroed();
        if unsafe { libc::fstatfs(file.as_raw_fd(), statfs.as_mut_ptr()) } != 0 {
            return Err(RecoveryError::Storage);
        }
        if unsafe { statfs.assume_init().f_type } != 0x0102_1994 {
            return Err(RecoveryError::DestinationInvalid);
        }
        if free_bytes(&file)? < required_capacity(bytes)? {
            return Err(RecoveryError::StorageFull);
        }
        Ok(VerifiedStagingDirectory {
            directory: DirectoryCapability::from_open_file(file)?,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (config, bytes);
        Err(RecoveryError::PlatformUnsupported)
    }
}

#[cfg(target_os = "linux")]
fn deepest_mount<'a>(entries: &'a [MountInfoEntry], path: &Path) -> Option<&'a MountInfoEntry> {
    entries
        .iter()
        .filter(|entry| path.starts_with(&entry.mount_point))
        .max_by_key(|entry| entry.mount_point.as_os_str().len())
}

#[cfg(target_os = "linux")]
fn open_directory(path: &Path) -> Result<File, RecoveryError> {
    let path =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| RecoveryError::DestinationInvalid)?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(RecoveryError::DestinationInvalid);
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(target_os = "linux")]
fn open_regular_file(path: &Path) -> Result<File, RecoveryError> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| RecoveryError::Storage)
}

#[cfg(target_os = "linux")]
fn verify_owned_directory(file: &File) -> Result<(), RecoveryError> {
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryError::DestinationInvalid)?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(RecoveryError::DestinationInvalid);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn same_filesystem(left: &File, right: &File) -> Result<bool, RecoveryError> {
    let l = left.metadata().map_err(|_| RecoveryError::Storage)?;
    let r = right.metadata().map_err(|_| RecoveryError::Storage)?;
    Ok(l.dev() == r.dev())
}

#[cfg(target_os = "linux")]
fn free_bytes(file: &File) -> Result<u64, RecoveryError> {
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    if unsafe { libc::fstatvfs(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(RecoveryError::Storage);
    }
    let stat = unsafe { stat.assume_init() };
    if stat.f_flag & libc::ST_RDONLY != 0 {
        return Err(RecoveryError::DestinationInvalid);
    }
    stat.f_bavail
        .checked_mul(stat.f_frsize)
        .ok_or(RecoveryError::CapacityOverflow)
}

#[cfg(target_os = "linux")]
pub(crate) fn filesystem_identity(file: &File, source: &str) -> Result<String, RecoveryError> {
    if source.starts_with("/dev/") {
        let source =
            std::fs::canonicalize(source).map_err(|_| RecoveryError::MountIdentityUnavailable)?;
        let entries = std::fs::read_dir("/dev/disk/by-uuid")
            .map_err(|_| RecoveryError::MountIdentityUnavailable)?;
        for entry in entries.flatten() {
            if std::fs::canonicalize(entry.path()).ok().as_ref() == Some(&source) {
                return Ok(format!("uuid:{}", entry.file_name().to_string_lossy()));
            }
        }
        return Err(RecoveryError::MountIdentityUnavailable);
    }
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(RecoveryError::Storage);
    }
    let fsid = unsafe { stat.assume_init().f_fsid };
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&fsid as *const libc::fsid_t).cast::<u8>(),
            std::mem::size_of::<libc::fsid_t>(),
        )
    };
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").map_err(|_| RecoveryError::Storage)?;
    }
    Ok(format!("fsid:{encoded}|{source}"))
}

#[cfg(target_os = "linux")]
fn probe_directory(directory: &DirectoryCapability) -> Result<(), RecoveryError> {
    probe_directory_with_hook(directory, &SystemHook)
}

#[cfg(target_os = "linux")]
pub(crate) fn probe_directory_with_hook(
    directory: &DirectoryCapability,
    hook: &impl LinuxOperationHook,
) -> Result<(), RecoveryError> {
    let fd = directory.as_raw_fd();
    let name = CString::new(random_name(".iotkit-probe-")?).map_err(|_| RecoveryError::Storage)?;
    let final_name =
        CString::new(random_name(".iotkit-probe-final-")?).map_err(|_| RecoveryError::Storage)?;
    let probe_fd = unsafe {
        libc::openat(
            fd,
            name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if probe_fd < 0 {
        return Err(RecoveryError::DestinationInvalid);
    }
    let mut probe = unsafe { File::from_raw_fd(probe_fd) };
    let bytes = b"iotkit-probe-v1";
    let mut cleanup_name = &name;
    let result = (|| {
        hook.before(LinuxOperation::ProbeAfterCreate, fd, &name)
            .map_err(|_| RecoveryError::DestinationInvalid)?;
        validate_probe_file(&probe)?;
        probe.write_all(bytes).map_err(|_| RecoveryError::Storage)?;
        hook.before(LinuxOperation::ProbeFileSync, fd, &name)
            .map_err(|_| RecoveryError::Storage)?;
        probe.sync_all().map_err(|_| RecoveryError::Storage)?;
        hook.before(LinuxOperation::ProbeRename, fd, &name)
            .map_err(|_| RecoveryError::PlatformUnsupported)?;
        rename_noreplace(fd, &name, fd, &final_name)?;
        cleanup_name = &final_name;
        hook.before(LinuxOperation::ProbeParentSync, fd, &final_name)
            .map_err(|_| RecoveryError::Storage)?;
        sync_fd(fd)?;
        hook.before(LinuxOperation::ProbeReadback, fd, &final_name)
            .map_err(|_| RecoveryError::DestinationInvalid)?;
        let read_fd = unsafe {
            libc::openat(
                fd,
                final_name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if read_fd < 0 {
            return Err(RecoveryError::DestinationInvalid);
        }
        let mut read_file = unsafe { File::from_raw_fd(read_fd) };
        validate_probe_file(&read_file)?;
        if file_identity(&read_file)? != file_identity(&probe)? {
            return Err(RecoveryError::DestinationInvalid);
        }
        let mut read_back = Vec::new();
        read_file
            .read_to_end(&mut read_back)
            .map_err(|_| RecoveryError::Storage)?;
        if read_back != bytes {
            return Err(RecoveryError::DestinationInvalid);
        }
        Ok(())
    })();
    let cleanup = cleanup_probe(fd, cleanup_name, &probe, hook);
    drop(probe);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(target_os = "linux")]
fn validate_probe_file(file: &File) -> Result<(), RecoveryError> {
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryError::DestinationInvalid)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(RecoveryError::DestinationInvalid);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn cleanup_probe(
    directory_fd: RawFd,
    name: &CStr,
    expected: &File,
    hook: &impl LinuxOperationHook,
) -> Result<(), RecoveryError> {
    let mut first_error = None;
    if hook
        .before(LinuxOperation::ProbeCleanupUnlink, directory_fd, name)
        .is_err()
    {
        first_error = Some(RecoveryError::Storage);
        hook.before(LinuxOperation::ProbeCleanupUnlink, directory_fd, name)
            .map_err(|_| RecoveryError::Storage)?;
    }
    remove_exact_file_at(directory_fd, name, expected)
        .map_err(|_| RecoveryError::ArtifactCleanupFailed)?;
    if hook
        .before(LinuxOperation::ProbeCleanupSync, directory_fd, name)
        .and_then(|()| sync_fd_io(directory_fd))
        .is_err()
    {
        first_error = Some(RecoveryError::Storage);
        hook.before(LinuxOperation::ProbeCleanupSync, directory_fd, name)
            .map_err(|_| RecoveryError::Storage)?;
        sync_fd(directory_fd)?;
    }
    first_error.map_or(Ok(()), Err)
}

/// Publishes an already-open encrypted artifact through the held destination descriptor.
pub fn publish_verified_artifact(
    destination: &VerifiedBackupDestination,
    artifact: &mut File,
    output_name: &str,
    passphrase: &BackupPassphrase,
) -> Result<NodeBackupManifest, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        publish_verified_artifact_with_hook(
            destination,
            artifact,
            output_name,
            passphrase,
            &SystemHook,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (destination, artifact, output_name, passphrase);
        Err(RecoveryError::PlatformUnsupported)
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn publish_verified_artifact_with_hook(
    destination: &VerifiedBackupDestination,
    artifact: &mut File,
    output_name: &str,
    passphrase: &BackupPassphrase,
    hook: &impl LinuxOperationHook,
) -> Result<NodeBackupManifest, RecoveryError> {
    validate_entry_name(output_name)?;
    let directory_fd = destination.directory.as_raw_fd();
    let output_fd = unsafe {
        libc::openat(
            directory_fd,
            c".".as_ptr(),
            libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
            0o600,
        )
    };
    if output_fd < 0 {
        return Err(RecoveryError::PlatformUnsupported);
    }
    let mut output = unsafe { File::from_raw_fd(output_fd) };
    std::io::copy(artifact, &mut output).map_err(|_| RecoveryError::Storage)?;
    output.sync_all().map_err(|_| RecoveryError::Storage)?;
    let output_identity = file_identity(&output)?;
    let name = CString::new(output_name).map_err(|_| RecoveryError::DestinationInvalid)?;
    if unsafe {
        libc::linkat(
            output.as_raw_fd(),
            c"".as_ptr(),
            directory_fd,
            name.as_ptr(),
            libc::AT_EMPTY_PATH,
        )
    } != 0
    {
        return if io::Error::last_os_error().kind() == io::ErrorKind::AlreadyExists {
            Err(RecoveryError::DestinationExists)
        } else {
            Err(RecoveryError::Storage)
        };
    }
    hook.before(LinuxOperation::PublicationAfterLink, directory_fd, &name)
        .map_err(|_| RecoveryError::ArtifactPublicationUncertain)?;
    sync_fd(directory_fd).map_err(|_| RecoveryError::ArtifactPublicationUncertain)?;
    let read_fd = unsafe {
        libc::openat(
            directory_fd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if read_fd < 0 {
        return Err(RecoveryError::ArtifactPublicationUncertain);
    }
    let final_file = unsafe { File::from_raw_fd(read_fd) };
    if file_identity(&final_file).map_err(|_| RecoveryError::ArtifactPublicationUncertain)?
        != output_identity
    {
        return Err(RecoveryError::ArtifactPublicationUncertain);
    }
    let manifest = crate::container::authenticate_container_file(final_file, passphrase)?;
    if entry_identity(directory_fd, &name)? != output_identity {
        return Err(RecoveryError::ArtifactPublicationUncertain);
    }
    Ok(manifest)
}

/// Removes only authenticated, recorded artifacts older than the retained newer successes.
pub fn apply_retention(
    destination: &VerifiedBackupDestination,
    passphrase: &BackupPassphrase,
    edge_node_id: &str,
    successful_backup_ids: &BTreeSet<String>,
    retention_count: u32,
) -> Result<u32, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        apply_retention_with_hook(
            destination,
            passphrase,
            edge_node_id,
            successful_backup_ids,
            retention_count,
            &SystemHook,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            destination,
            passphrase,
            edge_node_id,
            successful_backup_ids,
            retention_count,
        );
        Err(RecoveryError::PlatformUnsupported)
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn apply_retention_with_hook(
    destination: &VerifiedBackupDestination,
    passphrase: &BackupPassphrase,
    edge_node_id: &str,
    successful_backup_ids: &BTreeSet<String>,
    retention_count: u32,
    hook: &impl LinuxOperationHook,
) -> Result<u32, RecoveryError> {
    let directory_fd = destination.directory.as_raw_fd();
    let duplicated = unsafe { libc::dup(directory_fd) };
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
    let mut candidates: Vec<(String, NodeBackupManifest, File)> = Vec::new();
    loop {
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        let name_bytes = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        let name = match CString::new(name_bytes) {
            Ok(name) => name,
            Err(_) => continue,
        };
        let fd = unsafe {
            libc::openat(
                directory_fd,
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            continue;
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !metadata.is_file() || metadata.nlink() != 1 {
            continue;
        }
        let authenticated_file = match file.try_clone() {
            Ok(file) => file,
            Err(_) => continue,
        };
        let manifest = match crate::container::authenticate_container_file(file, passphrase) {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        if manifest.edge_node_id == edge_node_id
            && successful_backup_ids.contains(&manifest.backup_id)
        {
            candidates.push((
                name.to_string_lossy().into_owned(),
                manifest,
                authenticated_file,
            ));
        }
    }
    unsafe {
        libc::closedir(stream);
    }
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.1.created_at_ms));
    let mut removed = 0_u32;
    let keep = usize::try_from(retention_count.max(1)).map_err(|_| RecoveryError::Storage)?;
    let newest = candidates
        .first()
        .map(|candidate| candidate.1.created_at_ms);
    for (name, manifest, authenticated_file) in candidates.into_iter().skip(keep) {
        if newest.is_none_or(|created_at| manifest.created_at_ms >= created_at) {
            continue;
        }
        delete_verified_candidate(directory_fd, &name, &authenticated_file, hook)?;
        removed = removed.checked_add(1).ok_or(RecoveryError::Storage)?;
    }
    if removed > 0 && unsafe { libc::fsync(directory_fd) } != 0 {
        return Err(RecoveryError::ArtifactPublicationUncertain);
    }
    Ok(removed)
}

#[cfg(target_os = "linux")]
fn delete_verified_candidate(
    destination_fd: libc::c_int,
    name: &str,
    authenticated_file: &File,
    hook: &impl LinuxOperationHook,
) -> Result<(), RecoveryError> {
    let nonce = random_name(".iotkit-retention-")?;
    let quarantine = CString::new(nonce).map_err(|_| RecoveryError::Storage)?;
    if unsafe { libc::mkdirat(destination_fd, quarantine.as_ptr(), 0o700) } != 0 {
        return Err(RecoveryError::Storage);
    }
    let quarantine_fd = unsafe {
        libc::openat(
            destination_fd,
            quarantine.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if quarantine_fd < 0 {
        return Err(RecoveryError::ArtifactCleanupFailed);
    }
    let quarantine_directory = unsafe { File::from_raw_fd(quarantine_fd) };
    let original = CString::new(name).map_err(|_| RecoveryError::Storage)?;
    let item = c"artifact";
    hook.before(
        LinuxOperation::RetentionBeforeQuarantine,
        destination_fd,
        &original,
    )
    .map_err(|_| RecoveryError::Storage)?;
    if rename_noreplace(
        destination_fd,
        &original,
        quarantine_directory.as_raw_fd(),
        item,
    )
    .is_err()
    {
        return cleanup_retention_quarantine(
            destination_fd,
            &quarantine,
            &quarantine_directory,
            hook,
        )
        .and(Err(RecoveryError::DestinationInvalid));
    }
    let file_fd = unsafe {
        libc::openat(
            quarantine_directory.as_raw_fd(),
            item.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if file_fd < 0 {
        return preserve_quarantined(
            destination_fd,
            &original,
            &quarantine,
            &quarantine_directory,
            hook,
            RecoveryError::DestinationInvalid,
        );
    }
    let file = unsafe { File::from_raw_fd(file_fd) };
    let quarantined_identity = match file_identity(&file) {
        Ok(identity) => identity,
        Err(_) => {
            return preserve_quarantined(
                destination_fd,
                &original,
                &quarantine,
                &quarantine_directory,
                hook,
                RecoveryError::DestinationInvalid,
            );
        }
    };
    let authenticated_identity = match file_identity(authenticated_file) {
        Ok(identity) => identity,
        Err(_) => {
            return preserve_quarantined(
                destination_fd,
                &original,
                &quarantine,
                &quarantine_directory,
                hook,
                RecoveryError::DestinationInvalid,
            );
        }
    };
    if quarantined_identity != authenticated_identity {
        return preserve_quarantined(
            destination_fd,
            &original,
            &quarantine,
            &quarantine_directory,
            hook,
            RecoveryError::DestinationInvalid,
        );
    }
    if unsafe { libc::unlinkat(quarantine_directory.as_raw_fd(), item.as_ptr(), 0) } != 0
        || quarantine_directory.sync_all().is_err()
    {
        return Err(RecoveryError::ArtifactPublicationUncertain);
    }
    cleanup_retention_quarantine(destination_fd, &quarantine, &quarantine_directory, hook)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn preserve_quarantined(
    destination_fd: RawFd,
    original: &CStr,
    quarantine_name: &CStr,
    quarantine_directory: &File,
    hook: &impl LinuxOperationHook,
    reason: RecoveryError,
) -> Result<(), RecoveryError> {
    if rename_noreplace(
        quarantine_directory.as_raw_fd(),
        c"artifact",
        destination_fd,
        original,
    )
    .is_err()
    {
        return Err(RecoveryError::ArtifactPublicationUncertain);
    }
    quarantine_directory
        .sync_all()
        .map_err(|_| RecoveryError::ArtifactPublicationUncertain)?;
    cleanup_retention_quarantine(destination_fd, quarantine_name, quarantine_directory, hook)?;
    Err(reason)
}

#[cfg(target_os = "linux")]
fn cleanup_retention_quarantine(
    destination_fd: RawFd,
    quarantine_name: &CStr,
    quarantine_directory: &File,
    hook: &impl LinuxOperationHook,
) -> Result<(), RecoveryError> {
    hook.before(
        LinuxOperation::RetentionBeforeCleanup,
        destination_fd,
        quarantine_name,
    )
    .map_err(|_| RecoveryError::ArtifactCleanupFailed)?;
    remove_exact_empty_directory_at(destination_fd, quarantine_name, quarantine_directory)
        .map_err(|_| RecoveryError::ArtifactCleanupFailed)
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExactCleanupError {
    MismatchRestored,
    Uncertain,
}

#[cfg(target_os = "linux")]
pub(crate) fn remove_exact_file_at(
    directory_fd: RawFd,
    name: &CStr,
    expected: &File,
) -> Result<(), ExactCleanupError> {
    let (quarantine_name, quarantine) = create_private_directory(directory_fd, ".iotkit-cleanup-")?;
    let item = c"entry";
    if rename_noreplace(directory_fd, name, quarantine.as_raw_fd(), item).is_err() {
        return Err(ExactCleanupError::Uncertain);
    }
    let moved_fd = unsafe {
        libc::openat(
            quarantine.as_raw_fd(),
            item.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if moved_fd < 0 {
        return Err(ExactCleanupError::Uncertain);
    }
    let moved = unsafe { File::from_raw_fd(moved_fd) };
    let matches = match (file_identity(&moved), file_identity(expected)) {
        (Ok(moved), Ok(expected)) => moved == expected,
        _ => return Err(ExactCleanupError::Uncertain),
    };
    if !matches {
        if rename_noreplace(quarantine.as_raw_fd(), item, directory_fd, name).is_err() {
            return Err(ExactCleanupError::Uncertain);
        }
        remove_exact_empty_directory_at(directory_fd, &quarantine_name, &quarantine)?;
        return Err(ExactCleanupError::MismatchRestored);
    }
    if unsafe { libc::unlinkat(quarantine.as_raw_fd(), item.as_ptr(), 0) } != 0 {
        return Err(ExactCleanupError::Uncertain);
    }
    sync_fd(quarantine.as_raw_fd()).map_err(|_| ExactCleanupError::Uncertain)?;
    remove_exact_empty_directory_at(directory_fd, &quarantine_name, &quarantine)
}

#[cfg(target_os = "linux")]
pub(crate) fn remove_exact_empty_directory_at(
    directory_fd: RawFd,
    name: &CStr,
    expected: &File,
) -> Result<(), ExactCleanupError> {
    let (placeholder_name, placeholder) =
        create_private_directory(directory_fd, ".iotkit-cleanup-dir-")?;
    if rename_exchange(directory_fd, name, directory_fd, &placeholder_name).is_err() {
        return Err(ExactCleanupError::Uncertain);
    }
    let matches = match (
        entry_identity(directory_fd, &placeholder_name),
        file_identity(expected),
        entry_identity(directory_fd, name),
        file_identity(&placeholder),
    ) {
        (Ok(moved), Ok(expected), Ok(replacement), Ok(placeholder)) => {
            moved == expected && replacement == placeholder
        }
        _ => false,
    };
    if !matches {
        if rename_exchange(directory_fd, name, directory_fd, &placeholder_name).is_err() {
            return Err(ExactCleanupError::Uncertain);
        }
        if unsafe { libc::unlinkat(directory_fd, placeholder_name.as_ptr(), libc::AT_REMOVEDIR) }
            != 0
        {
            return Err(ExactCleanupError::Uncertain);
        }
        sync_fd(directory_fd).map_err(|_| ExactCleanupError::Uncertain)?;
        return Err(ExactCleanupError::MismatchRestored);
    }
    if unsafe { libc::unlinkat(directory_fd, placeholder_name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(ExactCleanupError::Uncertain);
    }
    if unsafe { libc::unlinkat(directory_fd, name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(ExactCleanupError::Uncertain);
    }
    sync_fd(directory_fd).map_err(|_| ExactCleanupError::Uncertain)
}

#[cfg(target_os = "linux")]
fn create_private_directory(
    directory_fd: RawFd,
    prefix: &str,
) -> Result<(CString, File), ExactCleanupError> {
    let name = CString::new(random_name(prefix).map_err(|_| ExactCleanupError::Uncertain)?)
        .map_err(|_| ExactCleanupError::Uncertain)?;
    if unsafe { libc::mkdirat(directory_fd, name.as_ptr(), 0o700) } != 0 {
        return Err(ExactCleanupError::Uncertain);
    }
    let fd = unsafe {
        libc::openat(
            directory_fd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(ExactCleanupError::Uncertain);
    }
    Ok((name, unsafe { File::from_raw_fd(fd) }))
}

#[cfg(target_os = "linux")]
fn random_name(prefix: &str) -> Result<String, RecoveryError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| RecoveryError::Random)?;
    let mut name = String::from(prefix);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut name, "{byte:02x}").map_err(|_| RecoveryError::Storage)?;
    }
    Ok(name)
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(target_os = "linux")]
fn file_identity(file: &File) -> Result<FileIdentity, RecoveryError> {
    let metadata = file.metadata().map_err(|_| RecoveryError::Storage)?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(target_os = "linux")]
fn entry_identity(directory_fd: RawFd, name: &CStr) -> Result<FileIdentity, RecoveryError> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::zeroed();
    if unsafe {
        libc::fstatat(
            directory_fd,
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(RecoveryError::ArtifactPublicationUncertain);
    }
    let stat = unsafe { stat.assume_init() };
    Ok(FileIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
    })
}

#[cfg(target_os = "linux")]
fn rename_noreplace(
    old_directory: RawFd,
    old_name: &CStr,
    new_directory: RawFd,
    new_name: &CStr,
) -> Result<(), RecoveryError> {
    if unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            old_directory,
            old_name.as_ptr(),
            new_directory,
            new_name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } == 0
    {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        if matches!(
            error.raw_os_error(),
            Some(code) if code == libc::ENOSYS || code == libc::EINVAL || code == libc::ENOTSUP
        ) {
            Err(RecoveryError::PlatformUnsupported)
        } else {
            Err(RecoveryError::Storage)
        }
    }
}

#[cfg(target_os = "linux")]
fn rename_exchange(
    first_directory: RawFd,
    first_name: &CStr,
    second_directory: RawFd,
    second_name: &CStr,
) -> Result<(), RecoveryError> {
    if unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            first_directory,
            first_name.as_ptr(),
            second_directory,
            second_name.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(RecoveryError::Storage)
    }
}

#[cfg(target_os = "linux")]
fn sync_fd(fd: RawFd) -> Result<(), RecoveryError> {
    sync_fd_io(fd).map_err(|_| RecoveryError::Storage)
}

#[cfg(target_os = "linux")]
fn sync_fd_io(fd: RawFd) -> io::Result<()> {
    if unsafe { libc::fsync(fd) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn validate_entry_name(name: &str) -> Result<(), RecoveryError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.bytes().any(|byte| byte == 0)
    {
        Err(RecoveryError::DestinationInvalid)
    } else {
        Ok(())
    }
}
