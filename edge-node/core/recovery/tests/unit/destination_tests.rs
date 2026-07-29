use std::path::PathBuf;

#[cfg(target_os = "linux")]
use std::{collections::BTreeSet, fs};

use super::*;
#[cfg(target_os = "linux")]
use tempfile::TempDir;

use crate::tests_support::mountinfo;

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
