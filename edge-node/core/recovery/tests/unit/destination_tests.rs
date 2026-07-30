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
struct CombinedProbeFaultHook {
    readback_pending: Cell<bool>,
    cleanup_pending: Cell<bool>,
}

#[cfg(target_os = "linux")]
impl crate::destination::LinuxOperationHook for CombinedProbeFaultHook {
    fn before(
        &self,
        operation: crate::destination::LinuxOperation,
        _directory_fd: RawFd,
        _name: &std::ffi::CStr,
    ) -> io::Result<()> {
        if operation == crate::destination::LinuxOperation::ProbeReadback
            && self.readback_pending.replace(false)
        {
            return Err(io::Error::from_raw_os_error(libc::EIO));
        }
        if operation == crate::destination::LinuxOperation::ProbeCleanupUnlink
            && self.cleanup_pending.replace(false)
        {
            return Err(io::Error::from_raw_os_error(libc::EIO));
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
fn operation_guard(root: &std::path::Path) -> RecoveryOperationGuard {
    use std::os::unix::fs::PermissionsExt;
    let control = root.join("control");
    fs::create_dir(&control).unwrap();
    fs::set_permissions(&control, fs::Permissions::from_mode(0o700)).unwrap();
    acquire_recovery_operation(&control.join("backup.json")).unwrap()
}

#[cfg(target_os = "linux")]
fn entry_names(path: &std::path::Path) -> BTreeSet<String> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != "control")
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
fn held_destination_capacity_check_uses_snapshot_length_without_reopening_path() {
    use std::cell::Cell;
    use std::os::unix::fs::PermissionsExt;
    const MIB: u64 = 1024 * 1024;

    let root = TempDir::new().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let destination_path = root.path().join("destination");
    std::fs::create_dir(&destination_path).unwrap();
    std::fs::set_permissions(&destination_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let destination = held_destination(&destination_path);
    let requested = Cell::new(0_u64);
    let snapshot_length = 128_u64 * MIB;
    let preflight_length = 1_u64;

    let result = crate::destination::ensure_capacity_with_probe(
        &destination,
        snapshot_length,
        |_directory| {
            requested.set(preflight_length);
            Ok(required_capacity(preflight_length).unwrap())
        },
    );

    assert_eq!(requested.get(), preflight_length);
    assert_eq!(result, Err(RecoveryError::StorageFull));
}

#[cfg(target_os = "linux")]
fn staging_config(root: &std::path::Path, staging: std::path::PathBuf) -> BackupConfig {
    BackupConfig {
        schema_version: 1,
        database: root.join("database.db"),
        destination: root.join("destination"),
        staging_directory: staging,
        passphrase_file: root.join("passphrase"),
        expected_mount: MountIdentity {
            mount_point: root.join("destination"),
            source: "tmpfs".into(),
            filesystem_type: "tmpfs".into(),
            filesystem_id: "fsid:staging-test".into(),
        },
        freshness_seconds: 60,
        retention_count: 1,
    }
}

#[cfg(target_os = "linux")]
#[test]
fn staging_verification_creates_only_the_exact_absent_leaf_from_a_tmpfs_parent() {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let root = TempDir::new_in("/dev/shm").unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let staging = root.path().join("staging-leaf");
    let guard = operation_guard(root.path());

    let verified =
        verify_staging_directory(&guard, &staging_config(root.path(), staging.clone()), 0)
            .expect("an absent leaf under an existing tmpfs parent is created");
    let metadata = fs::metadata(&staging).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(metadata.permissions().mode() & 0o077, 0);
    assert!(metadata.nlink() >= 2);
    drop(verified);
    assert!(staging.is_dir());
}

#[cfg(target_os = "linux")]
#[test]
fn staging_verification_removes_a_leaf_it_created_when_preflight_fails() {
    let root = TempDir::new_in("/dev/shm").unwrap();
    std::fs::set_permissions(
        root.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let staging = root.path().join("staging-leaf");
    let guard = operation_guard(root.path());

    assert!(matches!(
        verify_staging_directory(
            &guard,
            &staging_config(root.path(), staging.clone()),
            u64::MAX
        ),
        Err(RecoveryError::CapacityOverflow)
    ));
    assert!(
        !staging.exists(),
        "a failed preflight must remove only the exact leaf created for this request"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn staging_verification_rejects_non_tmpfs_parent_without_creating_a_leaf() {
    let root = TempDir::new_in("/dev/shm").unwrap();
    let staging = std::path::Path::new("/proc/self/iotkit-staging-test").to_path_buf();
    let guard = operation_guard(root.path());

    assert!(matches!(
        verify_staging_directory(&guard, &staging_config(root.path(), staging.clone()), 0),
        Err(RecoveryError::DestinationInvalid)
    ));
    assert!(!staging.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn staging_verification_accepts_the_real_run_tmpfs_parent() {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    if unsafe { libc::geteuid() } != 0 {
        // Production systemd runs this operation as root; an unprivileged
        // test account cannot create the owner-bound RuntimeDirectory leaf.
        return;
    }
    let root = TempDir::new_in("/dev/shm").unwrap();
    let leaf = std::path::PathBuf::from(format!("/run/iotkit-staging-test-{}", std::process::id()));
    if leaf.exists() {
        return;
    }
    let guard = operation_guard(root.path());
    let verified = verify_staging_directory(&guard, &staging_config(root.path(), leaf.clone()), 0)
        .expect("/run is an existing non-writable tmpfs parent");
    let metadata = fs::metadata(&leaf).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(metadata.permissions().mode() & 0o077, 0);
    drop(verified);
    fs::remove_dir(&leaf).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn staging_verification_rejects_a_world_writable_tmpfs_parent() {
    let root = TempDir::new_in("/dev/shm").unwrap();
    let guard = operation_guard(root.path());
    let leaf = std::path::PathBuf::from(format!(
        "/dev/shm/iotkit-staging-test-{}",
        std::process::id()
    ));
    assert!(matches!(
        verify_staging_directory(&guard, &staging_config(root.path(), leaf.clone()), 0),
        Err(RecoveryError::DestinationInvalid)
    ));
    assert!(!leaf.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn staging_verification_rejects_symlink_parent_and_insecure_existing_leaf() {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    let root = TempDir::new_in("/dev/shm").unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let actual_parent = root.path().join("actual");
    fs::create_dir(&actual_parent).unwrap();
    fs::set_permissions(&actual_parent, fs::Permissions::from_mode(0o700)).unwrap();
    let linked_parent = root.path().join("linked");
    symlink(&actual_parent, &linked_parent).unwrap();
    let guard = operation_guard(root.path());

    assert!(matches!(
        verify_staging_directory(
            &guard,
            &staging_config(root.path(), linked_parent.join("leaf")),
            0,
        ),
        Err(RecoveryError::DestinationInvalid)
    ));

    let existing = root.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        verify_staging_directory(&guard, &staging_config(root.path(), existing.clone()), 0),
        Err(RecoveryError::DestinationInvalid)
    ));
    fs::set_permissions(&existing, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        verify_staging_directory(&guard, &staging_config(root.path(), existing.clone()), 0),
        Err(RecoveryError::DestinationInvalid)
    ));
    assert!(
        existing.is_dir(),
        "an unsafe pre-existing leaf is never removed"
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
    let guard = operation_guard(root.path());

    assert!(matches!(
        verify_destination(&guard, &config, 1),
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
    let guard = operation_guard(root.path());

    assert_eq!(
        publish_verified_artifact(
            &guard,
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
    let guard = operation_guard(root.path());

    assert_eq!(
        apply_retention(
            &guard,
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
    let guard = operation_guard(root.path());

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
        assert!(crate::destination::probe_directory_with_hook(&guard, &directory, &hook).is_err());
        assert_eq!(entry_names(root.path()), BTreeSet::new(), "{operation:?}");
        crate::destination::probe_directory_with_hook(&guard, &directory, &TestHook::default())
            .unwrap();
        assert_eq!(entry_names(root.path()), BTreeSet::new(), "{operation:?}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn cleanup_uncertainty_wins_over_the_original_probe_failure() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let directory = DirectoryCapability::open(root.path()).unwrap();
    let guard = operation_guard(root.path());
    let hook = CombinedProbeFaultHook {
        readback_pending: Cell::new(true),
        cleanup_pending: Cell::new(true),
    };

    assert_eq!(
        crate::destination::probe_directory_with_hook(&guard, &directory, &hook),
        Err(RecoveryError::ArtifactCleanupFailed)
    );
    assert_eq!(entry_names(root.path()), BTreeSet::new());
    crate::destination::probe_directory_with_hook(&guard, &directory, &TestHook::default())
        .unwrap();
    assert_eq!(entry_names(root.path()), BTreeSet::new());
}

#[cfg(target_os = "linux")]
#[test]
fn next_run_fails_closed_on_exact_cleanup_leftovers_without_deleting_them() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let directory = fs::File::open(root.path()).unwrap();
    let cleanup = ".iotkit-cleanup-0123456789abcdef0123456789abcdef";
    let retention = ".iotkit-retention-fedcba9876543210fedcba9876543210";
    let unrelated = ".iotkit-cleanup-user-note";
    fs::create_dir(root.path().join(cleanup)).unwrap();
    fs::write(root.path().join(retention), b"unknown").unwrap();
    fs::write(root.path().join(unrelated), b"keep").unwrap();

    assert_eq!(
        crate::destination::ensure_no_cleanup_leftovers(&directory),
        Err(RecoveryError::CleanupRequired)
    );
    assert!(root.path().join(cleanup).is_dir());
    assert_eq!(fs::read(root.path().join(retention)).unwrap(), b"unknown");
    assert_eq!(fs::read(root.path().join(unrelated)).unwrap(), b"keep");
    fs::remove_dir(root.path().join(cleanup)).unwrap();
    fs::remove_file(root.path().join(retention)).unwrap();
    assert_eq!(
        crate::destination::ensure_no_cleanup_leftovers(&directory),
        Ok(())
    );
    assert_eq!(fs::read(root.path().join(unrelated)).unwrap(), b"keep");
    assert_eq!(
        RecoveryError::CleanupRequired.reason_code(),
        "cleanup_required"
    );
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
    let guard = operation_guard(root.path());
    let hook = TestHook {
        fail_once: Cell::new(None),
        mutate: Some(broaden_probe_mode),
    };

    assert_eq!(
        crate::destination::probe_directory_with_hook(&guard, &directory, &hook),
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
    let guard = operation_guard(root.path());
    let hook = TestHook {
        fail_once: Cell::new(None),
        mutate: Some(add_probe_hardlink),
    };

    assert_eq!(
        crate::destination::probe_directory_with_hook(&guard, &directory, &hook),
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
    let guard = operation_guard(root.path());
    let hook = NameRecordingHook::default();

    crate::destination::probe_directory_with_hook(&guard, &directory, &hook).unwrap();
    crate::destination::probe_directory_with_hook(&guard, &directory, &hook).unwrap();

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
fn substitute_probe_cleanup(
    operation: crate::destination::LinuxOperation,
    directory_fd: RawFd,
    name: &std::ffi::CStr,
) -> io::Result<()> {
    if operation != crate::destination::LinuxOperation::ProbeCleanupUnlink {
        return Ok(());
    }
    if unsafe {
        libc::renameat(
            directory_fd,
            name.as_ptr(),
            directory_fd,
            c".preserved-probe-owned".as_ptr(),
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
    replacement.write_all(b"unrelated-probe")
}

#[cfg(target_os = "linux")]
#[test]
fn probe_cleanup_substitution_preserves_the_unrelated_file_and_fails() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let directory = DirectoryCapability::open(root.path()).unwrap();
    let guard = operation_guard(root.path());
    let hook = TestHook {
        fail_once: Cell::new(None),
        mutate: Some(substitute_probe_cleanup),
    };

    assert_eq!(
        crate::destination::probe_directory_with_hook(&guard, &directory, &hook),
        Err(RecoveryError::ArtifactCleanupFailed)
    );
    assert_eq!(
        fs::read(root.path().join(".preserved-probe-owned")).unwrap(),
        b"iotkit-probe-v1"
    );
    assert!(fs::read_dir(root.path()).unwrap().any(|entry| {
        fs::read(entry.unwrap().path()).ok().as_deref() == Some(b"unrelated-probe")
    }));

    let before = entry_names(root.path());
    crate::destination::probe_directory_with_hook(&guard, &directory, &TestHook::default())
        .unwrap();
    assert_eq!(entry_names(root.path()), before);
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
    let guard = operation_guard(root.path());
    let hook = TestHook {
        fail_once: Cell::new(None),
        mutate: Some(swap_published_entry),
    };

    assert_eq!(
        crate::destination::publish_verified_artifact_with_hook(
            &guard,
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
    let guard = operation_guard(root.path());
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
            &guard,
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
fn substitute_retention_cleanup_directory(
    operation: crate::destination::LinuxOperation,
    directory_fd: RawFd,
    name: &std::ffi::CStr,
) -> io::Result<()> {
    if operation != crate::destination::LinuxOperation::RetentionBeforeCleanup {
        return Ok(());
    }
    if unsafe {
        libc::renameat(
            directory_fd,
            name.as_ptr(),
            directory_fd,
            c".preserved-retention-owned-dir".as_ptr(),
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::mkdirat(directory_fd, name.as_ptr(), 0o700) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn retention_cleanup_substitution_preserves_the_unrelated_directory_and_fails() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let destination_path = root.path().join("destination");
    fs::create_dir(&destination_path).unwrap();
    fs::set_permissions(&destination_path, fs::Permissions::from_mode(0o700)).unwrap();
    let database = root.path().join("database");
    fs::write(&database, b"SQLite format 3\0public-db").unwrap();
    let destination = held_destination(&destination_path);
    let passphrase = BackupPassphrase::new("public-format-passphrase".into());
    let guard = operation_guard(root.path());
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
    let hook = TestHook {
        fail_once: Cell::new(None),
        mutate: Some(substitute_retention_cleanup_directory),
    };

    assert_eq!(
        crate::destination::apply_retention_with_hook(
            &guard,
            &destination,
            &passphrase,
            "node-a",
            &BTreeSet::from(["backup-new".into(), "backup-old".into()]),
            1,
            &hook,
        ),
        Err(RecoveryError::ArtifactCleanupFailed)
    );
    assert!(destination_path.join(".iotkit-new").exists());
    assert!(!destination_path.join(".iotkit-old").exists());
    assert!(
        destination_path
            .join(".preserved-retention-owned-dir")
            .is_dir()
    );
    assert!(fs::read_dir(&destination_path).unwrap().any(|entry| {
        let entry = entry.unwrap();
        entry.file_type().unwrap().is_dir()
            && entry
                .file_name()
                .to_string_lossy()
                .starts_with(".iotkit-retention-")
    }));
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
    let guard = operation_guard(root.path());
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
            &guard,
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
