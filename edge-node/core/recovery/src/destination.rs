use std::{collections::BTreeSet, fs::File, path::PathBuf};

#[cfg(target_os = "linux")]
use std::{ffi::CString, fs::OpenOptions, path::Path};

#[cfg(target_os = "linux")]
use std::os::{
    fd::{AsRawFd, FromRawFd},
    unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
};

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
    let fd = directory.as_raw_fd();
    let name = CString::new(format!(".iotkit-probe-{}", std::process::id()))
        .map_err(|_| RecoveryError::Storage)?;
    let final_name = CString::new(format!(".iotkit-probe-final-{}", std::process::id()))
        .map_err(|_| RecoveryError::Storage)?;
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
    let result = (|| {
        use std::io::{Read, Write};
        probe.write_all(bytes).map_err(|_| RecoveryError::Storage)?;
        probe.sync_all().map_err(|_| RecoveryError::Storage)?;
        let renamed = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                fd,
                name.as_ptr(),
                fd,
                final_name.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if renamed != 0 {
            return Err(RecoveryError::PlatformUnsupported);
        }
        unsafe { libc::fsync(fd) };
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
        let mut read_back = Vec::new();
        unsafe { File::from_raw_fd(read_fd) }
            .read_to_end(&mut read_back)
            .map_err(|_| RecoveryError::Storage)?;
        if read_back != bytes {
            return Err(RecoveryError::DestinationInvalid);
        }
        Ok(())
    })();
    drop(probe);
    unsafe {
        libc::unlinkat(fd, name.as_ptr(), 0);
        libc::unlinkat(fd, final_name.as_ptr(), 0);
        libc::fsync(fd);
    }
    result
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
            return if unsafe { *libc::__errno_location() } == libc::EEXIST {
                Err(RecoveryError::DestinationExists)
            } else {
                Err(RecoveryError::Storage)
            };
        }
        if unsafe { libc::fsync(directory_fd) } != 0 {
            return Err(RecoveryError::ArtifactPublicationUncertain);
        }
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
        crate::container::authenticate_container_file(
            unsafe { File::from_raw_fd(read_fd) },
            passphrase,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (destination, artifact, output_name, passphrase);
        Err(RecoveryError::PlatformUnsupported)
    }
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
        let mut candidates: Vec<(String, NodeBackupManifest, u64, u64)> = Vec::new();
        loop {
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                break;
            }
            let name_bytes =
                unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name_bytes == b"." || name_bytes == b".." || !name_bytes.starts_with(b".iotkit-") {
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
                    metadata.dev(),
                    metadata.ino(),
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
        for (name, manifest, expected_dev, expected_ino) in candidates.into_iter().skip(keep) {
            if newest.is_none_or(|created_at| manifest.created_at_ms >= created_at) {
                continue;
            }
            let name = CString::new(name).map_err(|_| RecoveryError::Storage)?;
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
                continue;
            }
            let stat = unsafe { stat.assume_init() };
            if stat.st_dev as u64 != expected_dev || stat.st_ino != expected_ino {
                continue;
            }
            if unsafe { libc::unlinkat(directory_fd, name.as_ptr(), 0) } == 0 {
                removed = removed.checked_add(1).ok_or(RecoveryError::Storage)?;
            }
        }
        if removed > 0 && unsafe { libc::fsync(directory_fd) } != 0 {
            return Err(RecoveryError::ArtifactPublicationUncertain);
        }
        Ok(removed)
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
