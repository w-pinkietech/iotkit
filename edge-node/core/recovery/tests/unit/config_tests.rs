use std::path::Path;

#[cfg(target_os = "linux")]
use std::fs;

use super::*;
use tempfile::TempDir;

fn config(root: &Path) -> BackupConfig {
    BackupConfig {
        schema_version: 1,
        database: root.join("data/edge.db"),
        destination: root.join("backup"),
        staging_directory: root.join("stage"),
        passphrase_file: root.join("secrets/passphrase"),
        expected_mount: MountIdentity {
            mount_point: root.join("backup"),
            source: "server:/edge-node".into(),
            filesystem_type: "nfs4".into(),
            filesystem_id: "fsid:1234".into(),
        },
        freshness_seconds: 60,
        retention_count: 2,
    }
}

#[cfg(target_os = "linux")]
#[test]
fn configure_backup_writes_schema_one_owner_only_json_and_refuses_replacement() {
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let input = config(root.path());

    configure_backup(&config_path, &input, BackupConfigReplace::Refuse).unwrap();
    assert_eq!(load_owner_only_config(&config_path).unwrap(), input);
    assert_eq!(
        configure_backup(&config_path, &input, BackupConfigReplace::Refuse),
        Err(RecoveryError::DestinationExists)
    );
    let mut replacement = input.clone();
    replacement.retention_count = 7;
    configure_backup(
        &config_path,
        &replacement,
        BackupConfigReplace::ReplaceExisting,
    )
    .unwrap();
    assert_eq!(load_owner_only_config(&config_path).unwrap(), replacement);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(config_path).unwrap().permissions().mode() & 0o077,
            0
        );
    }
}

#[test]
fn configuration_rejects_relative_and_overlapping_paths_before_writing() {
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let mut input = config(root.path());
    input.destination = Path::new("relative-backup").to_path_buf();

    assert_eq!(
        configure_backup(&config_path, &input, BackupConfigReplace::Refuse),
        Err(RecoveryError::InvalidConfiguration)
    );
    assert!(!config_path.exists());

    let mut input = config(root.path());
    input.destination = input.staging_directory.join("encrypted");
    assert_eq!(
        configure_backup(&config_path, &input, BackupConfigReplace::Refuse),
        Err(RecoveryError::InvalidConfiguration)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn configuration_rejects_zero_retention_before_creating_a_file() {
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let mut input = config(root.path());
    input.retention_count = 0;

    assert_eq!(
        configure_backup(&config_path, &input, BackupConfigReplace::Refuse),
        Err(RecoveryError::InvalidConfiguration)
    );
    assert!(!config_path.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn owner_only_config_reader_rejects_symlink_hardlink_broad_mode_and_oversize() {
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let input = config(root.path());
    fs::write(&config_path, serde_json::to_vec(&input).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            load_owner_only_config(&config_path),
            Err(RecoveryError::InvalidConfiguration)
        );
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600)).unwrap();
        let alias = root.path().join("alias.json");
        fs::hard_link(&config_path, &alias).unwrap();
        assert_eq!(
            load_owner_only_config(&config_path),
            Err(RecoveryError::InvalidConfiguration)
        );
        fs::remove_file(&alias).unwrap();
        let link = root.path().join("link.json");
        symlink(&config_path, &link).unwrap();
        assert_eq!(
            load_owner_only_config(&link),
            Err(RecoveryError::InvalidConfiguration)
        );
    }

    fs::write(&config_path, vec![b'x'; 64 * 1024 + 1]).unwrap();
    assert_eq!(
        load_owner_only_config(&config_path),
        Err(RecoveryError::InvalidConfiguration)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn handoff_loader_accepts_only_bounded_owner_only_closed_json() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let handoff_path = root.path().join("handoff.json");
    let handoff = RecoveryHandoff {
        schema_version: 1,
        recovery_id: "recovery-1".into(),
        edge_id: "edge-1".into(),
        edge_node_id: "node-1".into(),
        old_ledger_epoch: "epoch-1".into(),
        expected_backup_id: Some("backup-1".into()),
        proposed_new_epoch: "epoch-2".into(),
        credential_generation: 1,
    };
    fs::write(&handoff_path, serde_json::to_vec(&handoff).unwrap()).unwrap();
    fs::set_permissions(&handoff_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(load_owner_only_handoff(&handoff_path).unwrap(), handoff);

    fs::set_permissions(&handoff_path, fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(
        load_owner_only_handoff(&handoff_path),
        Err(RecoveryError::InvalidConfiguration)
    );
    fs::set_permissions(&handoff_path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&handoff_path, vec![b'x'; 64 * 1024 + 1]).unwrap();
    assert_eq!(
        load_owner_only_handoff(&handoff_path),
        Err(RecoveryError::InvalidConfiguration)
    );
}

#[cfg(not(target_os = "linux"))]
#[test]
fn non_linux_configuration_fails_closed_before_creating_a_file() {
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    assert_eq!(
        configure_backup(
            &config_path,
            &config(root.path()),
            BackupConfigReplace::Refuse
        ),
        Err(RecoveryError::PlatformUnsupported)
    );
    assert!(!config_path.exists());
}
