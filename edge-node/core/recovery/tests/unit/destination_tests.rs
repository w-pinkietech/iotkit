use std::path::PathBuf;

#[cfg(target_os = "linux")]
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    fs, io,
    os::fd::{FromRawFd, RawFd},
};

use super::*;
#[cfg(target_os = "linux")]
use tempfile::TempDir;

use crate::tests_support::mountinfo;

#[cfg(target_os = "linux")]
type HookMutation =
    fn(crate::destination::LinuxOperation, RawFd, &std::ffi::CStr) -> io::Result<()>;

#[cfg(target_os = "linux")]
#[derive(Default)]
struct TestHook {
    fail_once: Cell<Option<crate::destination::LinuxOperation>>,
    mutate: Option<HookMutation>,
}

#[cfg(target_os = "linux")]
impl crate::destination::LinuxOperationHook for TestHook {
    fn before(
        &self,
        operation: crate::destination::LinuxOperation,
        directory_fd: RawFd,
        name: &std::ffi::CStr,
    ) -> io::Result<()> {
        if self.fail_once.get() == Some(operation) {
            self.fail_once.set(None);
            return Err(io::Error::from_raw_os_error(libc::EIO));
        }
        if let Some(mutate) = self.mutate {
            mutate(operation, directory_fd, name)?;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn held_destination(path: &std::path::Path) -> VerifiedBackupDestination {
    VerifiedBackupDestination {
        directory: DirectoryCapability::open(path).unwrap(),
    }
}

#[cfg(target_os = "linux")]
fn entry_names(path: &std::path::Path) -> BTreeSet<String> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn mountinfo_decodes_escaped_fields_and_selects_deepest_matching_mount() {
    let mountinfo = concat!(
        "24 1 8:1 / / rw,relatime - ext4 /dev/sda1 rw\n",
        "31 24 0:44 / /mnt/edge\\040backups rw - nfs4 server:/node\\040backups rw\n",
        "32 31 0:44 /daily /mnt/edge\\040backups/daily rw - nfs4 server:/node\\040backups rw\n",
    );
    let entries = parse_mountinfo(mountinfo).unwrap();
    let selected = entries
        .iter()
        .filter(|entry| PathBuf::from("/mnt/edge backups/daily/a").starts_with(&entry.mount_point))
        .max_by_key(|entry| entry.mount_point.as_os_str().len())
        .unwrap();

    assert_eq!(
        selected.mount_point,
        PathBuf::from("/mnt/edge backups/daily")
    );
    assert_eq!(selected.source, "server:/node backups");
    assert_eq!(selected.filesystem_type, "nfs4");
}

#[test]
fn mountinfo_parses_ext4_nfs_smb_and_bind_records() {
    let entries = parse_mountinfo(&format!(
        "{}{}{}{}",
        mountinfo::EXT4,
        mountinfo::NFS,
        mountinfo::SMB,
        mountinfo::BIND
    ))
    .unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].filesystem_type, "ext4");
    assert_eq!(entries[1].filesystem_type, "nfs4");
    assert_eq!(entries[2].filesystem_type, "cifs");
    assert_eq!(entries[3].mount_point, PathBuf::from("/mnt/bind"));
}

#[test]
fn capacity_reserves_five_percent_or_sixty_four_mib_with_checked_arithmetic() {
    const MIB: u64 = 1024 * 1024;
    assert_eq!(required_capacity(0), Ok(64 * MIB));
    assert_eq!(required_capacity(20 * 64 * MIB), Ok(21 * 64 * MIB));
    assert_eq!(
        required_capacity(u64::MAX),
        Err(RecoveryError::CapacityOverflow)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn verification_rejects_missing_expected_mount_before_touching_fallback_directory() {
    let root = TempDir::new().unwrap();
    let destination = root.path().join("fallback");
    fs::create_dir(&destination).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).unwrap();
    let config = BackupConfig {
        schema_version: 1,
        database: root.path().join("database.db"),
        destination: destination.clone(),
        staging_directory: root.path().join("stage"),
        passphrase_file: root.path().join("passphrase"),
        expected_mount: MountIdentity {
            mount_point: destination,
            source: "server:/not-present".into(),
            filesystem_type: "nfs4".into(),
            filesystem_id: "fsid:not-present".into(),
        },
        freshness_seconds: 60,
        retention_count: 1,
    };

    assert!(matches!(
        verify_destination(&config, 1),
        Err(RecoveryError::MountMissing)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn publication_after_path_replacement_uses_the_held_destination_descriptor() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let destination = root.path().join("destination");
    let replacement = root.path().join("replacement");
    let moved = root.path().join("moved");
    fs::create_dir(&destination).unwrap();
    fs::create_dir(&replacement).unwrap();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o700)).unwrap();
    let held = VerifiedBackupDestination {
        directory: DirectoryCapability::open(&destination).unwrap(),
    };
    fs::rename(&destination, &moved).unwrap();
    fs::rename(&replacement, &destination).unwrap();
    let source = root.path().join("ciphertext");
    fs::write(&source, b"not a container").unwrap();

    assert_eq!(
        publish_verified_artifact(
            &held,
            &mut fs::File::open(&source).unwrap(),
            ".iotkit-artifact",
            &BackupPassphrase::new("correct horse battery staple".into()),
        ),
        Err(RecoveryError::ContainerInvalid)
    );
    assert!(moved.join(".iotkit-artifact").exists());
    assert!(!destination.join(".iotkit-artifact").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn retention_never_removes_unknown_unverified_files() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let destination = root.path().join("destination");
    fs::create_dir(&destination).unwrap();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).unwrap();
    let unknown = destination.join(".iotkit-unknown");
    fs::write(&unknown, b"not a container").unwrap();

    assert_eq!(
        apply_retention(
            &VerifiedBackupDestination {
                directory: DirectoryCapability::open(&destination).unwrap(),
            },
            &BackupPassphrase::new("correct horse battery staple".into()),
            "node-a",
            &BTreeSet::new(),
            1,
        ),
        Ok(0)
    );
    assert!(unknown.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn stable_mount_identity_binds_fsid_to_the_decoded_source() {
    let root = TempDir::new().unwrap();
    let one = crate::destination::filesystem_identity(
        &fs::File::open(root.path()).unwrap(),
        "server:/one",
    )
    .unwrap();
    let two = crate::destination::filesystem_identity(
        &fs::File::open(root.path()).unwrap(),
        "server:/two",
    )
    .unwrap();
    assert_ne!(
        one, two,
        "a mutable source name without fstatfs identity is unsafe"
    );
    assert!(one.starts_with("fsid:"));
}

#[cfg(target_os = "linux")]
#[test]
fn capability_probe_faults_leave_no_entries_and_a_retry_succeeds() {
    use crate::destination::LinuxOperation::{
        ProbeCleanupSync, ProbeCleanupUnlink, ProbeFileSync, ProbeParentSync, ProbeReadback,
        ProbeRename,
    };
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let directory = DirectoryCapability::open(root.path()).unwrap();

    for operation in [
        ProbeFileSync,
        ProbeRename,
        ProbeReadback,
        ProbeParentSync,
        ProbeCleanupUnlink,
        ProbeCleanupSync,
    ] {
        let hook = TestHook {
            fail_once: Cell::new(Some(operation)),
            mutate: None,
        };
        assert!(crate::destination::probe_directory_with_hook(&directory, &hook).is_err());
        assert_eq!(entry_names(root.path()), BTreeSet::new(), "{operation:?}");
        crate::destination::probe_directory_with_hook(&directory, &TestHook::default()).unwrap();
        assert_eq!(entry_names(root.path()), BTreeSet::new(), "{operation:?}");
    }
}

#[cfg(target_os = "linux")]
fn broaden_probe_mode(
    operation: crate::destination::LinuxOperation,
    directory_fd: RawFd,
    name: &std::ffi::CStr,
) -> io::Result<()> {
    if operation == crate::destination::LinuxOperation::ProbeAfterCreate
        && unsafe { libc::fchmodat(directory_fd, name.as_ptr(), 0o640, 0) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn capability_probe_rejects_a_file_with_permissions_broader_than_0600() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let directory = DirectoryCapability::open(root.path()).unwrap();
    let hook = TestHook {
        fail_once: Cell::new(None),
        mutate: Some(broaden_probe_mode),
    };

    assert_eq!(
        crate::destination::probe_directory_with_hook(&directory, &hook),
        Err(RecoveryError::DestinationInvalid)
    );
    assert_eq!(entry_names(root.path()), BTreeSet::new());
}

#[cfg(target_os = "linux")]
fn add_probe_hardlink(
    operation: crate::destination::LinuxOperation,
    directory_fd: RawFd,
    name: &std::ffi::CStr,
) -> io::Result<()> {
    if operation == crate::destination::LinuxOperation::ProbeAfterCreate
        && unsafe {
            libc::linkat(
                directory_fd,
                name.as_ptr(),
                directory_fd,
                c".probe-extra-link".as_ptr(),
                0,
            )
        } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn capability_probe_rejects_a_multiple_link_file() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let directory = DirectoryCapability::open(root.path()).unwrap();
    let hook = TestHook {
        fail_once: Cell::new(None),
        mutate: Some(add_probe_hardlink),
    };

    assert_eq!(
        crate::destination::probe_directory_with_hook(&directory, &hook),
        Err(RecoveryError::DestinationInvalid)
    );
    fs::remove_file(root.path().join(".probe-extra-link")).unwrap();
    assert_eq!(entry_names(root.path()), BTreeSet::new());
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct NameRecordingHook {
    created_names: RefCell<Vec<String>>,
}

#[cfg(target_os = "linux")]
impl crate::destination::LinuxOperationHook for NameRecordingHook {
    fn before(
        &self,
        operation: crate::destination::LinuxOperation,
        _directory_fd: RawFd,
        name: &std::ffi::CStr,
    ) -> io::Result<()> {
        if operation == crate::destination::LinuxOperation::ProbeAfterCreate {
            self.created_names
                .borrow_mut()
                .push(name.to_string_lossy().into_owned());
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[test]
fn capability_probe_uses_a_fresh_cryptorandom_name_on_retry() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let directory = DirectoryCapability::open(root.path()).unwrap();
    let hook = NameRecordingHook::default();

    crate::destination::probe_directory_with_hook(&directory, &hook).unwrap();
    crate::destination::probe_directory_with_hook(&directory, &hook).unwrap();

    let names = hook.created_names.borrow();
    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1]);
    for name in names.iter() {
        let random = name.strip_prefix(".iotkit-probe-").unwrap();
        assert_eq!(random.len(), 32);
        assert!(random.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}

#[cfg(target_os = "linux")]
fn swap_published_entry(
    operation: crate::destination::LinuxOperation,
    directory_fd: RawFd,
    name: &std::ffi::CStr,
) -> io::Result<()> {
    if operation != crate::destination::LinuxOperation::PublicationAfterLink {
        return Ok(());
    }
    if unsafe {
        libc::renameat(
            directory_fd,
            name.as_ptr(),
            directory_fd,
            c".preserved-intended".as_ptr(),
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe {
        libc::openat(
            directory_fd,
            name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut replacement = unsafe { fs::File::from_raw_fd(fd) };
    use std::io::Write as _;
    replacement.write_all(b"replacement")
}

#[cfg(target_os = "linux")]
#[test]
fn publication_swap_after_link_never_authenticates_or_deletes_the_replacement() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let destination_path = root.path().join("destination");
    fs::create_dir(&destination_path).unwrap();
    fs::set_permissions(&destination_path, fs::Permissions::from_mode(0o700)).unwrap();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/node-backup-v1.bin");
    let mut artifact = fs::File::open(&fixture).unwrap();
    let destination = held_destination(&destination_path);
    let passphrase = BackupPassphrase::new("public-format-passphrase".into());
    let hook = TestHook {
        fail_once: Cell::new(None),
        mutate: Some(swap_published_entry),
    };

    assert_eq!(
        crate::destination::publish_verified_artifact_with_hook(
            &destination,
            &mut artifact,
            ".iotkit-final",
            &passphrase,
            &hook,
        ),
        Err(RecoveryError::ArtifactPublicationUncertain)
    );
    assert_eq!(
        fs::read(destination_path.join(".iotkit-final")).unwrap(),
        b"replacement"
    );
    assert!(
        authenticate_container(&destination_path.join(".preserved-intended"), &passphrase).is_ok()
    );
}

#[cfg(target_os = "linux")]
fn swap_retention_candidate(
    operation: crate::destination::LinuxOperation,
    directory_fd: RawFd,
    name: &std::ffi::CStr,
) -> io::Result<()> {
    if operation != crate::destination::LinuxOperation::RetentionBeforeQuarantine {
        return Ok(());
    }
    if unsafe {
        libc::renameat(
            directory_fd,
            name.as_ptr(),
            directory_fd,
            c".preserved-authenticated".as_ptr(),
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe {
        libc::openat(
            directory_fd,
            name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut replacement = unsafe { fs::File::from_raw_fd(fd) };
    use std::io::Write as _;
    replacement.write_all(b"replacement")
}

#[cfg(target_os = "linux")]
fn retention_manifest(backup_id: &str, created_at_ms: i64) -> NodeBackupManifest {
    use crate::{BackupCounts, SnapshotMode};
    NodeBackupManifest {
        artifact_kind: "iotkit-node-backup".into(),
        format_version: 1,
        backup_id: backup_id.into(),
        edge_node_id: "node-a".into(),
        ledger_epoch: "epoch-a".into(),
        created_at_ms,
        accepted_cursor: 0,
        allocation_high_water: 0,
        snapshot_mode: SnapshotMode::Online,
        shutdown_seal_id: None,
        schema_version: 23,
        database_length: 25,
        database_sha256: "958ec6fc5da916b2f0008194cf46f2e9342ceae562e04e4b035baf5b7339b79c".into(),
        counts: BackupCounts::default(),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn retention_substitution_preserves_the_replacement_and_authenticated_inode() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let destination_path = root.path().join("destination");
    fs::create_dir(&destination_path).unwrap();
    fs::set_permissions(&destination_path, fs::Permissions::from_mode(0o700)).unwrap();
    let database = root.path().join("database");
    fs::write(&database, b"SQLite format 3\0public-db").unwrap();
    let destination = held_destination(&destination_path);
    let passphrase = BackupPassphrase::new("public-format-passphrase".into());
    encrypt_container(
        &database,
        &retention_manifest("backup-new", 20),
        &passphrase,
        destination.capability(),
        ".iotkit-new",
    )
    .unwrap();
    encrypt_container(
        &database,
        &retention_manifest("backup-old", 10),
        &passphrase,
        destination.capability(),
        ".iotkit-old",
    )
    .unwrap();
    let successful = BTreeSet::from(["backup-new".into(), "backup-old".into()]);
    let hook = TestHook {
        fail_once: Cell::new(None),
        mutate: Some(swap_retention_candidate),
    };

    assert_eq!(
        crate::destination::apply_retention_with_hook(
            &destination,
            &passphrase,
            "node-a",
            &successful,
            1,
            &hook,
        ),
        Err(RecoveryError::DestinationInvalid)
    );
    assert_eq!(
        fs::read(destination_path.join(".iotkit-old")).unwrap(),
        b"replacement"
    );
    assert!(
        authenticate_container(
            &destination_path.join(".preserved-authenticated"),
            &passphrase
        )
        .is_ok()
    );
    assert!(
        entry_names(&destination_path)
            .iter()
            .all(|name| !name.starts_with(".iotkit-retention-"))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn retention_removes_authenticated_normal_backup_names() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let destination_path = root.path().join("destination");
    fs::create_dir(&destination_path).unwrap();
    fs::set_permissions(&destination_path, fs::Permissions::from_mode(0o700)).unwrap();
    let database = root.path().join("database");
    fs::write(&database, b"SQLite format 3\0public-db").unwrap();
    let destination = held_destination(&destination_path);
    let passphrase = BackupPassphrase::new("public-format-passphrase".into());
    encrypt_container(
        &database,
        &retention_manifest("backup-new", 20),
        &passphrase,
        destination.capability(),
        "new.iotkit-node-backup",
    )
    .unwrap();
    encrypt_container(
        &database,
        &retention_manifest("backup-old", 10),
        &passphrase,
        destination.capability(),
        "old.iotkit-node-backup",
    )
    .unwrap();

    assert_eq!(
        apply_retention(
            &destination,
            &passphrase,
            "node-a",
            &BTreeSet::from(["backup-new".into(), "backup-old".into()]),
            1,
        )
        .unwrap(),
        1
    );
    assert!(destination_path.join("new.iotkit-node-backup").exists());
    assert!(!destination_path.join("old.iotkit-node-backup").exists());
}
