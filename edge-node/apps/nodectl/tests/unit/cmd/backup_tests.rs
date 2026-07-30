#[cfg(target_os = "linux")]
#[test]
fn requires_mounts_for_path_uses_systemd_hex_escapes_for_special_bytes() {
    let path = std::path::Path::new("/mnt/a b\\c\"%/制御");
    assert_eq!(
        super::systemd_mount_path(path).unwrap(),
        "/mnt/a\\x20b\\x5cc\\x22\\x25/\\xe5\\x88\\xb6\\xe5\\xbe\\xa1"
    );
    let drop_in = String::from_utf8(super::systemd_drop_in_bytes(path).unwrap()).unwrap();
    assert_eq!(
        drop_in,
        "[Unit]\nRequiresMountsFor=/mnt/a\\x20b\\x5cc\\x22\\x25/\\xe5\\x88\\xb6\\xe5\\xbe\\xa1\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn requires_mounts_for_path_rejects_non_utf8_input() {
    use std::os::unix::ffi::OsStringExt;

    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![
        b'/', b'm', b'n', b't', b'/', 0xff,
    ]));
    assert!(matches!(
        super::systemd_mount_path(&path),
        Err(iotkit_core_recovery::RecoveryError::InvalidConfiguration)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn restore_staging_is_created_owner_only_even_with_umask_zero() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let _environment_lock = environment_lock().lock().unwrap();

    let old_umask = unsafe { libc::umask(0) };
    let path = super::create_restore_staging(None).unwrap();
    unsafe { libc::umask(old_umask) };
    let metadata = std::fs::metadata(&path).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
    std::fs::remove_dir(path).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn restore_staging_post_create_failure_removes_private_directory() {
    let _environment_lock = environment_lock().lock().unwrap();
    let base = tempfile::tempdir_in("/tmp").unwrap();
    let previous_tmpdir = std::env::var_os("TMPDIR");
    unsafe { std::env::set_var("TMPDIR", base.path()) };
    let result = super::create_restore_staging_with(None, |_| {
        Err(iotkit_core_recovery::RecoveryError::Storage)
    });
    match previous_tmpdir {
        Some(value) => unsafe { std::env::set_var("TMPDIR", value) },
        None => unsafe { std::env::remove_var("TMPDIR") },
    }
    assert!(matches!(
        result,
        Err(iotkit_core_recovery::RecoveryError::Storage)
    ));
    assert_eq!(std::fs::read_dir(base.path()).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
fn environment_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(target_os = "linux")]
struct PairFixture {
    _control: tempfile::TempDir,
    _destination: tempfile::TempDir,
    _staging_parent: tempfile::TempDir,
    _staging: tempfile::TempDir,
    config_path: std::path::PathBuf,
    drop_in_path: std::path::PathBuf,
    config: iotkit_core_recovery::BackupConfig,
}

#[cfg(target_os = "linux")]
fn pair_fixture() -> PairFixture {
    use std::os::unix::fs::PermissionsExt;

    let control = tempfile::tempdir_in("/tmp").unwrap();
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging = tempfile::TempDir::new_in(staging_parent.path()).unwrap();
    for path in [
        control.path(),
        destination.path(),
        staging_parent.path(),
        staging.path(),
    ] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let passphrase_file = control.path().join("passphrase");
    std::fs::write(&passphrase_file, b"owner-only-test-passphrase").unwrap();
    std::fs::set_permissions(&passphrase_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let config_path = control.path().join("backup.json");
    let drop_in_path = control.path().join("backup.mount.conf");
    let config = iotkit_core_recovery::BackupConfig {
        schema_version: 1,
        database: control.path().join("missing.db"),
        destination: destination.path().to_path_buf(),
        staging_directory: staging.path().to_path_buf(),
        passphrase_file,
        expected_mount: iotkit_core_recovery::MountIdentity {
            mount_point: destination.path().to_path_buf(),
            source: "pending".into(),
            filesystem_type: "pending".into(),
            filesystem_id: "pending".into(),
        },
        freshness_seconds: 86_400,
        retention_count: 7,
    };
    PairFixture {
        _control: control,
        _destination: destination,
        _staging_parent: staging_parent,
        _staging: staging,
        config_path,
        drop_in_path,
        config,
    }
}

#[cfg(target_os = "linux")]
struct FailPair(&'static str);

#[cfg(target_os = "linux")]
impl super::PairFault for FailPair {
    fn at(&self, phase: &str) -> Result<(), iotkit_core_recovery::RecoveryError> {
        if phase == self.0 {
            Err(iotkit_core_recovery::RecoveryError::Storage)
        } else {
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn pair_fault_matrix_rolls_back_without_a_mixed_pair() {
    for phase in [
        "after_backup",
        "after_config_publish",
        "after_drop_in_publish",
        "after_parent_sync",
    ] {
        let fixture = pair_fixture();
        let result = super::configure_backup_pair_with_fault(
            &fixture.config_path,
            &fixture.config,
            &fixture.drop_in_path,
            iotkit_core_recovery::BackupConfigReplace::Refuse,
            &FailPair(phase),
        );
        assert!(result.is_err(), "{phase} unexpectedly succeeded");
        assert!(!fixture.config_path.exists(), "config survived {phase}");
        assert!(!fixture.drop_in_path.exists(), "drop-in survived {phase}");
        assert!(
            !fixture
                .config_path
                .parent()
                .unwrap()
                .join(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME)
                .exists(),
            "pending marker survived {phase}"
        );
    }
}

#[cfg(target_os = "linux")]
struct AbortPair(&'static str);

#[cfg(target_os = "linux")]
impl super::PairFault for AbortPair {
    fn at(&self, phase: &str) -> Result<(), iotkit_core_recovery::RecoveryError> {
        if phase == self.0 {
            std::process::abort();
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[test]
fn pair_fault_abort_windows_retry_to_durable_completion() {
    use std::{os::unix::process::ExitStatusExt, process::Command};

    for phase in [
        "after_config_rename_before_marker",
        "after_drop_in_rename_before_marker",
        "after_drop_in_publish",
        "after_published_marker",
        "after_config_backup_unlink",
        "after_completion_receipt",
        "after_completion_receipt_sync",
        "after_completion_marker_unlink",
        "after_completion_marker_unlink_sync",
    ] {
        let fixture = pair_fixture();
        super::configure_backup_pair_with_fault(
            &fixture.config_path,
            &fixture.config,
            &fixture.drop_in_path,
            iotkit_core_recovery::BackupConfigReplace::Refuse,
            &super::NoPairFault,
        )
        .unwrap();
        let mut replacement = fixture.config.clone();
        replacement.retention_count += 1;
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cmd::backup::tests::pair_fault_abort_child",
                "--nocapture",
            ])
            .env("IOTKIT_NODectl_PAIR_CONFIG", &fixture.config_path)
            .env("IOTKIT_NODectl_PAIR_DROP_IN", &fixture.drop_in_path)
            .env("IOTKIT_NODectl_PAIR_PHASE", phase)
            .status()
            .unwrap();
        assert_eq!(
            child.signal(),
            Some(libc::SIGABRT),
            "{phase} child did not abort at its requested hook"
        );
        super::configure_backup_pair_with_fault(
            &fixture.config_path,
            &replacement,
            &fixture.drop_in_path,
            iotkit_core_recovery::BackupConfigReplace::ReplaceExisting,
            &super::NoPairFault,
        )
        .unwrap_or_else(|error| panic!("{phase} retry failed: {error:?}"));
        assert!(fixture.config_path.exists(), "{phase} lost config");
        assert!(fixture.drop_in_path.exists(), "{phase} lost drop-in");
        assert!(
            fixture
                .config_path
                .parent()
                .unwrap()
                .join(iotkit_core_recovery::BACKUP_PAIR_COMPLETION_NAME)
                .exists()
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn published_marker_binds_retry_identity_before_refuse_and_explicit_replace() {
    use std::{os::unix::process::ExitStatusExt, process::Command};

    let fixture = pair_fixture();
    super::configure_backup_pair_with_fault(
        &fixture.config_path,
        &fixture.config,
        &fixture.drop_in_path,
        iotkit_core_recovery::BackupConfigReplace::Refuse,
        &super::NoPairFault,
    )
    .unwrap();
    let config_before = std::fs::read(&fixture.config_path).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cmd::backup::tests::pair_fault_abort_child",
            "--nocapture",
        ])
        .env("IOTKIT_NODectl_PAIR_CONFIG", &fixture.config_path)
        .env("IOTKIT_NODectl_PAIR_DROP_IN", &fixture.drop_in_path)
        .env("IOTKIT_NODectl_PAIR_PHASE", "after_published_marker")
        .status()
        .unwrap();
    assert_eq!(
        child.signal(),
        Some(libc::SIGABRT),
        "published-marker child did not abort at its requested hook"
    );
    let parent = fixture.config_path.parent().unwrap();
    assert!(
        parent
            .join(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME)
            .exists()
    );
    let published_config = std::fs::read(&fixture.config_path).unwrap();
    let published_drop_in = std::fs::read(&fixture.drop_in_path).unwrap();
    assert_ne!(published_config, config_before);

    let mut different = fixture.config.clone();
    different.retention_count += 2;
    assert_eq!(
        super::configure_backup_pair_with_fault(
            &fixture.config_path,
            &different,
            &fixture.drop_in_path,
            iotkit_core_recovery::BackupConfigReplace::Refuse,
            &super::NoPairFault,
        ),
        Err(iotkit_core_recovery::RecoveryError::DestinationExists)
    );
    assert_eq!(
        std::fs::read(&fixture.config_path).unwrap(),
        published_config
    );
    assert_eq!(
        std::fs::read(&fixture.drop_in_path).unwrap(),
        published_drop_in
    );
    assert!(
        !parent
            .join(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME)
            .exists(),
        "published marker was not finalized before refusing a different request"
    );
    assert!(
        parent
            .join(iotkit_core_recovery::BACKUP_PAIR_COMPLETION_NAME)
            .exists()
    );

    super::configure_backup_pair_with_fault(
        &fixture.config_path,
        &different,
        &fixture.drop_in_path,
        iotkit_core_recovery::BackupConfigReplace::ReplaceExisting,
        &super::NoPairFault,
    )
    .unwrap();
    let persisted: iotkit_core_recovery::BackupConfig =
        serde_json::from_slice(&std::fs::read(&fixture.config_path).unwrap()).unwrap();
    assert_eq!(persisted.retention_count, different.retention_count);
    assert!(
        !parent
            .join(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME)
            .exists()
    );
    assert!(
        parent
            .join(iotkit_core_recovery::BACKUP_PAIR_COMPLETION_NAME)
            .exists()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn pair_fault_initial_abort_windows_retry_with_refuse_converges_without_mixed_pair() {
    use std::{os::unix::process::ExitStatusExt, process::Command};

    for phase in [
        "after_backup",
        "before_config_rename",
        "after_config_rename_before_marker",
        "after_config_publish",
        "before_drop_in_rename",
        "after_drop_in_rename_before_marker",
        "after_drop_in_publish",
        "after_parent_sync",
    ] {
        let fixture = pair_fixture();
        let parent = fixture.config_path.parent().unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cmd::backup::tests::pair_fault_initial_abort_child",
                "--nocapture",
            ])
            .env("IOTKIT_NODectl_PAIR_CONFIG", &fixture.config_path)
            .env("IOTKIT_NODectl_PAIR_DROP_IN", &fixture.drop_in_path)
            .env("IOTKIT_NODectl_PAIR_DATABASE", &fixture.config.database)
            .env(
                "IOTKIT_NODectl_PAIR_DESTINATION",
                &fixture.config.destination,
            )
            .env(
                "IOTKIT_NODectl_PAIR_STAGING",
                &fixture.config.staging_directory,
            )
            .env(
                "IOTKIT_NODectl_PAIR_PASSPHRASE",
                &fixture.config.passphrase_file,
            )
            .env("IOTKIT_NODectl_PAIR_PHASE", phase)
            .status()
            .unwrap();
        assert_eq!(
            child.signal(),
            Some(libc::SIGABRT),
            "{phase} child did not abort at its requested hook"
        );
        assert_eq!(
            fixture.config_path.exists(),
            !matches!(phase, "after_backup" | "before_config_rename"),
            "unexpected config publication boundary at {phase}"
        );
        assert_eq!(
            fixture.drop_in_path.exists(),
            matches!(
                phase,
                "after_drop_in_rename_before_marker"
                    | "after_drop_in_publish"
                    | "after_parent_sync"
            ),
            "unexpected drop-in publication boundary at {phase}"
        );
        assert!(
            parent
                .join(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME)
                .exists(),
            "pending marker lost after {phase}"
        );
        if phase == "after_backup" {
            assert_eq!(
                iotkit_core_recovery::backup_status(&fixture.config_path, 0),
                Err(iotkit_core_recovery::RecoveryError::CleanupRequired)
            );
        }

        super::configure_backup_pair_with_fault(
            &fixture.config_path,
            &fixture.config,
            &fixture.drop_in_path,
            iotkit_core_recovery::BackupConfigReplace::Refuse,
            &super::NoPairFault,
        )
        .unwrap_or_else(|error| panic!("{phase} retry failed: {error:?}"));
        assert!(fixture.config_path.exists(), "{phase} lost config");
        assert!(fixture.drop_in_path.exists(), "{phase} lost drop-in");
        let persisted: iotkit_core_recovery::BackupConfig =
            serde_json::from_slice(&std::fs::read(&fixture.config_path).unwrap()).unwrap();
        assert_eq!(
            std::fs::read(&fixture.drop_in_path).unwrap(),
            super::systemd_drop_in_bytes(&persisted.expected_mount.mount_point).unwrap()
        );
        assert!(
            !parent
                .join(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME)
                .exists(),
            "pending marker survived {phase} retry"
        );
        for entry in std::fs::read_dir(parent).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if name.starts_with(".iotkit-backup-pair.") {
                assert_eq!(
                    name,
                    iotkit_core_recovery::BACKUP_PAIR_COMPLETION_NAME,
                    "pair temporary/backup leftover after {phase}"
                );
            }
            assert!(
                !name.starts_with(".backup.json."),
                "config temporary leftover after {phase}"
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn pair_fault_abort_child() {
    let (Some(config), Some(drop_in), Some(phase)) = (
        std::env::var_os("IOTKIT_NODectl_PAIR_CONFIG"),
        std::env::var_os("IOTKIT_NODectl_PAIR_DROP_IN"),
        std::env::var_os("IOTKIT_NODectl_PAIR_PHASE"),
    ) else {
        return;
    };
    let phase = phase.to_string_lossy().to_string();
    let config_path = std::path::PathBuf::from(config);
    let drop_in_path = std::path::PathBuf::from(drop_in);
    let persisted = iotkit_core_recovery::load_owner_only_config(&config_path).unwrap();
    let mut request = persisted;
    request.retention_count += 1;
    let _ = super::configure_backup_pair_with_fault(
        &config_path,
        &request,
        &drop_in_path,
        iotkit_core_recovery::BackupConfigReplace::ReplaceExisting,
        &AbortPair(Box::leak(phase.into_boxed_str())),
    );
}

#[cfg(target_os = "linux")]
#[test]
fn pair_fault_initial_abort_child() {
    let (
        Some(config),
        Some(drop_in),
        Some(database),
        Some(destination),
        Some(staging),
        Some(passphrase),
        Some(phase),
    ) = (
        std::env::var_os("IOTKIT_NODectl_PAIR_CONFIG"),
        std::env::var_os("IOTKIT_NODectl_PAIR_DROP_IN"),
        std::env::var_os("IOTKIT_NODectl_PAIR_DATABASE"),
        std::env::var_os("IOTKIT_NODectl_PAIR_DESTINATION"),
        std::env::var_os("IOTKIT_NODectl_PAIR_STAGING"),
        std::env::var_os("IOTKIT_NODectl_PAIR_PASSPHRASE"),
        std::env::var_os("IOTKIT_NODectl_PAIR_PHASE"),
    )
    else {
        return;
    };
    let config_path = std::path::PathBuf::from(config);
    let destination = std::path::PathBuf::from(destination);
    let config = iotkit_core_recovery::BackupConfig {
        schema_version: 1,
        database: std::path::PathBuf::from(database),
        destination: destination.clone(),
        staging_directory: std::path::PathBuf::from(staging),
        passphrase_file: std::path::PathBuf::from(passphrase),
        expected_mount: iotkit_core_recovery::MountIdentity {
            mount_point: destination,
            source: "pending".into(),
            filesystem_type: "pending".into(),
            filesystem_id: "pending".into(),
        },
        freshness_seconds: 86_400,
        retention_count: 7,
    };
    let phase = phase.to_string_lossy().to_string();
    let _ = super::configure_backup_pair_with_fault(
        &config_path,
        &config,
        &std::path::PathBuf::from(drop_in),
        iotkit_core_recovery::BackupConfigReplace::Refuse,
        &AbortPair(Box::leak(phase.into_boxed_str())),
    );
}

#[cfg(target_os = "linux")]
struct BlockingPair {
    entered: std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    release: std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
}

#[cfg(target_os = "linux")]
impl super::PairFault for BlockingPair {
    fn at(&self, phase: &str) -> Result<(), iotkit_core_recovery::RecoveryError> {
        if phase != "after_backup" {
            return Ok(());
        }
        let (lock, wake) = &*self.entered;
        *lock.lock().unwrap() = true;
        wake.notify_one();
        let (release, wake) = &*self.release;
        let mut released = release.lock().unwrap();
        while !*released {
            released = wake.wait(released).unwrap();
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[test]
fn pair_operation_guard_serializes_configure_attempts() {
    let fixture = pair_fixture();
    let entered = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let release = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let hook = BlockingPair {
        entered: entered.clone(),
        release: release.clone(),
    };
    let config_path = fixture.config_path.clone();
    let drop_in_path = fixture.drop_in_path.clone();
    let config = fixture.config.clone();
    let first = std::thread::spawn(move || {
        super::configure_backup_pair_with_fault(
            &config_path,
            &config,
            &drop_in_path,
            iotkit_core_recovery::BackupConfigReplace::Refuse,
            &hook,
        )
    });
    let (lock, wake) = &*entered;
    let mut reached = lock.lock().unwrap();
    while !*reached {
        reached = wake.wait(reached).unwrap();
    }
    let second = super::configure_backup_pair_with_fault(
        &fixture.config_path,
        &fixture.config,
        &fixture.drop_in_path,
        iotkit_core_recovery::BackupConfigReplace::Refuse,
        &super::NoPairFault,
    );
    assert_eq!(
        second,
        Err(iotkit_core_recovery::RecoveryError::OperationBusy)
    );
    let (release_lock, release_wake) = &*release;
    *release_lock.lock().unwrap() = true;
    release_wake.notify_one();
    first.join().unwrap().unwrap();
}
