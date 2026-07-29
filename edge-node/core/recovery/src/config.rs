use std::path::{Component, Path};

#[cfg(target_os = "linux")]
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

#[cfg(target_os = "linux")]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

use crate::{BackupConfig, BackupPassphrase, RecoveryError};

#[cfg(target_os = "linux")]
const CONFIG_MAX_BYTES: u64 = 64 * 1024;

/// Explicit configuration replacement policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupConfigReplace {
    Refuse,
    ReplaceExisting,
}

/// Writes schema-1 owner-only backup configuration without following its final path component.
pub fn configure_backup(
    path: &Path,
    config: &BackupConfig,
    replacement: BackupConfigReplace,
) -> Result<(), RecoveryError> {
    validate_config(config)?;
    #[cfg(target_os = "linux")]
    {
        let parent = path.parent().ok_or(RecoveryError::InvalidConfiguration)?;
        let name = file_name(path)?;
        let bytes = serde_json::to_vec(config).map_err(|_| RecoveryError::InvalidConfiguration)?;
        if bytes.len() as u64 > CONFIG_MAX_BYTES {
            return Err(RecoveryError::InvalidConfiguration);
        }
        let temporary = temporary_path(path)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)
            .map_err(|_| RecoveryError::Storage)?;
        let write_result = (|| {
            output
                .write_all(&bytes)
                .map_err(|_| RecoveryError::Storage)?;
            output.sync_all().map_err(|_| RecoveryError::Storage)?;
            drop(output);
            if path.exists() {
                if replacement != BackupConfigReplace::ReplaceExisting {
                    return Err(RecoveryError::DestinationExists);
                }
                validate_owner_file(path, CONFIG_MAX_BYTES)?;
            }
            rename_no_replace_or_replace(&temporary, path, replacement)?;
            File::open(parent)
                .and_then(|parent| parent.sync_all())
                .map_err(|_| RecoveryError::Storage)
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        let _ = name;
        write_result
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, replacement);
        Err(RecoveryError::PlatformUnsupported)
    }
}

/// Loads a bounded schema-1 configuration from an owner-only regular file.
pub fn load_owner_only_config(path: &Path) -> Result<BackupConfig, RecoveryError> {
    #[cfg(target_os = "linux")]
    {
        let mut file = open_owner_file(path, CONFIG_MAX_BYTES)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| RecoveryError::Storage)?;
        if bytes.len() as u64 > CONFIG_MAX_BYTES {
            return Err(RecoveryError::InvalidConfiguration);
        }
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
        let mut file =
            open_owner_file(path, 4_098).map_err(|_| RecoveryError::InvalidPassphrase)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| RecoveryError::Storage)?;
        if bytes.len() > 4_098 {
            return Err(RecoveryError::InvalidPassphrase);
        }
        let value = String::from_utf8(bytes).map_err(|_| RecoveryError::InvalidPassphrase)?;
        if value.chars().any(char::is_control) {
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

pub(crate) fn validate_config(config: &BackupConfig) -> Result<(), RecoveryError> {
    if config.schema_version != 1
        || config.freshness_seconds == 0
        || config.expected_mount.mount_point.as_os_str().is_empty()
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
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    validate_owner_file_open(&file, limit)?;
    Ok(file)
}

#[cfg(target_os = "linux")]
fn validate_owner_file(path: &Path, limit: u64) -> Result<(), RecoveryError> {
    let file = open_owner_file_unchecked(path)?;
    validate_owner_file_open(&file, limit)
}

#[cfg(target_os = "linux")]
fn open_owner_file_unchecked(path: &Path) -> Result<File, RecoveryError> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| RecoveryError::InvalidConfiguration)
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
fn file_name(path: &Path) -> Result<&std::ffi::OsStr, RecoveryError> {
    path.file_name()
        .filter(|name| !name.is_empty())
        .ok_or(RecoveryError::InvalidConfiguration)
}

#[cfg(target_os = "linux")]
fn temporary_path(path: &Path) -> Result<PathBuf, RecoveryError> {
    let name = file_name(path)?;
    let mut temporary = path.to_path_buf();
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|_| RecoveryError::Random)?;
    let suffix = format!(".iotkit-config-{}", u64::from_le_bytes(random));
    temporary.set_file_name(format!("{}{}", name.to_string_lossy(), suffix));
    Ok(temporary)
}

#[cfg(target_os = "linux")]
fn rename_no_replace_or_replace(
    temporary: &Path,
    destination: &Path,
    replacement: BackupConfigReplace,
) -> Result<(), RecoveryError> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let temporary = CString::new(temporary.as_os_str().as_bytes())
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    let flags = match replacement {
        BackupConfigReplace::Refuse => libc::RENAME_NOREPLACE,
        BackupConfigReplace::ReplaceExisting => 0,
    };
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            temporary.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            flags,
        )
    };
    if result == 0 {
        Ok(())
    } else if unsafe { *libc::__errno_location() } == libc::EEXIST {
        Err(RecoveryError::DestinationExists)
    } else {
        Err(RecoveryError::Storage)
    }
}
