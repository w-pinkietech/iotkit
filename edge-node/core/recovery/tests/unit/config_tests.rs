use std::path::Path;

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::os::fd::RawFd;

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

#[test]
fn backup_pair_helpers_are_platform_neutral() {
    use sha2::{Digest, Sha256};

    let path = Path::new("edge-node-backup.json");
    let txid = "backup-config-0123";
    let config_temp_name = format!(
        ".edge-node-backup.json.{}.iotkit-config",
        "0123456789abcdef0123456789abcdef"
    );
    let path_hash = Sha256::digest(path.as_os_str().as_encoded_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let record = BackupPairRecord {
        schema_version: 3,
        txid: txid.into(),
        config_path_hash: path_hash,
        drop_in_path_hash: "a".repeat(64),
        phase: BackupPairPhase::ConfigPublishing,
        request_config_hash: "b".repeat(64),
        request_drop_in_hash: "c".repeat(64),
        config_hash: Some("d".repeat(64)),
        drop_in_hash: Some("e".repeat(64)),
        config_existed: true,
        drop_in_existed: true,
        old_config_hash: Some("f".repeat(64)),
        old_drop_in_hash: Some("0".repeat(64)),
        config_temp_name: Some(config_temp_name),
        drop_in_temp_name: Some(format!(".iotkit-backup-pair.{txid}.drop-in.tmp")),
    };

    assert!(
        record
            .validate_for_paths(path, Some(&"a".repeat(64)))
            .is_ok()
    );
}

#[cfg(target_os = "linux")]
fn mountinfo_for(destination: &Path) -> String {
    use std::os::unix::fs::MetadataExt;
    let device = fs::metadata(destination).unwrap().dev();
    format!(
        "41 24 {}:{} / {} rw - nfs4 server:/derived-live rw\n",
        libc::major(device),
        libc::minor(device),
        destination.display()
    )
}

#[cfg(target_os = "linux")]
fn prepare_config(root: &Path) -> BackupConfig {
    use std::os::unix::fs::PermissionsExt;
    let input = config(root);
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(&input.destination).unwrap();
    fs::set_permissions(&input.destination, fs::Permissions::from_mode(0o700)).unwrap();
    input
}

#[cfg(target_os = "linux")]
fn config_parent_entries(root: &Path) -> Vec<std::ffi::OsString> {
    let mut entries: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    entries.sort();
    entries
}

#[cfg(target_os = "linux")]
#[test]
fn owner_passphrase_parser_accepts_one_terminal_line_ending_only() {
    use std::os::unix::fs::PermissionsExt;

    let root = TempDir::new().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = root.path().join("passphrase");
    fs::write(&path, b"twelve-chars\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let parsed = load_owner_only_passphrase(&path).unwrap();
    assert_eq!(parsed.char_count(), 12);

    fs::write(&path, b"twelve-chars\r\n").unwrap();
    assert_eq!(load_owner_only_passphrase(&path).unwrap().char_count(), 12);

    let reject = |bytes: &[u8]| {
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            load_owner_only_passphrase(&path).unwrap_err(),
            RecoveryError::InvalidPassphrase,
        );
    };
    reject(b"twelve\nchars");
    reject(b"twelve\rchars");
    reject(b"twelve\0chars");
    reject(&[0xff, 0xfe]);
    reject(b"12345678901");
    let too_long = format!("{}x", "a".repeat(1024));
    reject(too_long.as_bytes());

    let unicode = "あ".repeat(1024);
    fs::write(&path, unicode.as_bytes()).unwrap();
    assert_eq!(
        load_owner_only_passphrase(&path).unwrap().char_count(),
        1024
    );
}

#[cfg(target_os = "linux")]
#[test]
fn bounded_owner_reader_rejects_growth_after_metadata_observation() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let _environment_lock = owner_reader_environment_lock().lock().unwrap();
    let root = TempDir::new().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = root.path().join("passphrase");
    fs::write(&path, b"twelve-chars").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let ready = root.path().join("reader.ready");
    let proceed = root.path().join("reader.continue");
    unsafe {
        std::env::set_var("IOTKIT_TEST_OWNER_FILE_PAUSE_AFTER_FSTAT", "1");
        std::env::set_var("IOTKIT_TEST_OWNER_FILE_PAUSE_PATH", &path);
        std::env::set_var("IOTKIT_TEST_OWNER_FILE_READY_FILE", &ready);
        std::env::set_var("IOTKIT_TEST_OWNER_FILE_CONTINUE_FILE", &proceed);
    }
    let reader_path = path.clone();
    let reader = std::thread::spawn(move || load_owner_only_passphrase(&reader_path));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "reader did not pause after fstat"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&vec![b'x'; 8 * 1024])
        .unwrap();
    fs::write(&proceed, b"continue").unwrap();
    let result = reader.join().unwrap();
    unsafe {
        std::env::remove_var("IOTKIT_TEST_OWNER_FILE_PAUSE_AFTER_FSTAT");
        std::env::remove_var("IOTKIT_TEST_OWNER_FILE_PAUSE_PATH");
        std::env::remove_var("IOTKIT_TEST_OWNER_FILE_READY_FILE");
        std::env::remove_var("IOTKIT_TEST_OWNER_FILE_CONTINUE_FILE");
    }
    assert_eq!(result.unwrap_err(), RecoveryError::InvalidPassphrase);
}

#[cfg(target_os = "linux")]
fn owner_reader_environment_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(target_os = "linux")]
fn test_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(target_os = "linux")]
fn completion_record(
    config_path: &Path,
    config_hash: &str,
    drop_in_hash: &str,
) -> serde_json::Value {
    use std::os::unix::ffi::OsStrExt;

    serde_json::json!({
        "schema_version": 3,
        "txid": "receipt-1",
        "config_path_hash": test_digest(config_path.as_os_str().as_bytes()),
        "drop_in_path_hash": "11".repeat(32),
        "phase": "published",
        "request_config_hash": "22".repeat(32),
        "request_drop_in_hash": "33".repeat(32),
        "config_hash": config_hash,
        "drop_in_hash": drop_in_hash,
        "config_existed": false,
        "drop_in_existed": false,
        "old_config_hash": null,
        "old_drop_in_hash": null,
        "config_temp_name": ".backup.json.0123456789abcdef0123456789abcdef.iotkit-config",
        "drop_in_temp_name": ".iotkit-backup-pair.receipt-1.drop-in.tmp"
    })
}

#[cfg(target_os = "linux")]
fn write_private_json(path: &Path, value: &serde_json::Value) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn completion_receipt_is_closed_and_bound_to_the_current_config() {
    use std::os::unix::fs::PermissionsExt;

    let root = TempDir::new().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let config_path = root.path().join("backup.json");
    let config_bytes = serde_json::to_vec(&config(root.path())).unwrap();
    fs::write(&config_path, &config_bytes).unwrap();
    fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600)).unwrap();
    let receipt_path = root.path().join(BACKUP_PAIR_COMPLETION_NAME);
    let valid = completion_record(&config_path, &test_digest(&config_bytes), &"44".repeat(32));
    write_private_json(&receipt_path, &valid);
    assert!(load_owner_only_config(&config_path).is_ok());

    let mut invalid_rows = Vec::new();
    let mut invalid = valid.clone();
    invalid["schema_version"] = serde_json::json!(4);
    invalid_rows.push(invalid);
    let mut invalid = valid.clone();
    invalid["txid"] = serde_json::json!("UPPER");
    invalid_rows.push(invalid);
    let mut invalid = valid.clone();
    invalid["config_path_hash"] = serde_json::json!("55".repeat(32));
    invalid_rows.push(invalid);
    let mut invalid = valid.clone();
    invalid["config_hash"] = serde_json::json!("short");
    invalid_rows.push(invalid);
    let mut invalid = valid.clone();
    invalid["drop_in_hash"] = serde_json::json!("GG".repeat(32));
    invalid_rows.push(invalid);
    let mut invalid = valid.clone();
    invalid["unexpected"] = serde_json::json!(true);
    invalid_rows.push(invalid);
    let mut invalid = valid.clone();
    invalid["config_temp_name"] = serde_json::json!(".wrong.iotkit-config");
    invalid_rows.push(invalid);
    let mut invalid = valid.clone();
    invalid["drop_in_temp_name"] = serde_json::json!(".wrong.tmp");
    invalid_rows.push(invalid);

    for invalid in invalid_rows {
        write_private_json(&receipt_path, &invalid);
        assert_eq!(
            load_owner_only_config(&config_path),
            Err(RecoveryError::CleanupRequired)
        );
    }

    write_private_json(&receipt_path, &valid);
    let mut modified = config(root.path());
    modified.retention_count += 1;
    fs::write(&config_path, serde_json::to_vec(&modified).unwrap()).unwrap();
    assert_eq!(
        load_owner_only_config(&config_path),
        Err(RecoveryError::CleanupRequired)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn completion_receipt_rejects_special_files_without_hanging_or_mutating_config() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{PermissionsExt, symlink};

    for kind in ["symlink", "hardlink", "fifo"] {
        let root = TempDir::new().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let config_path = root.path().join("backup.json");
        let config_bytes = serde_json::to_vec(&config(root.path())).unwrap();
        fs::write(&config_path, &config_bytes).unwrap();
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600)).unwrap();
        let receipt_path = root.path().join(BACKUP_PAIR_COMPLETION_NAME);
        let source = root.path().join("receipt-source");
        let valid = completion_record(&config_path, &test_digest(&config_bytes), &"44".repeat(32));
        write_private_json(&source, &valid);
        match kind {
            "symlink" => symlink(&source, &receipt_path).unwrap(),
            "hardlink" => fs::hard_link(&source, &receipt_path).unwrap(),
            "fifo" => {
                fs::remove_file(&source).unwrap();
                let name = CString::new(receipt_path.as_os_str().as_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            }
            _ => unreachable!(),
        }

        assert_eq!(
            load_owner_only_config(&config_path),
            Err(RecoveryError::CleanupRequired),
            "{kind}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_bytes, "{kind}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn receipt_and_marker_coexist_only_for_a_valid_post_commit_cleanup_state() {
    use std::os::unix::fs::PermissionsExt;

    let root = TempDir::new().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let config_path = root.path().join("backup.json");
    let config_bytes = serde_json::to_vec(&config(root.path())).unwrap();
    fs::write(&config_path, &config_bytes).unwrap();
    fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600)).unwrap();
    let receipt_path = root.path().join(BACKUP_PAIR_COMPLETION_NAME);
    let marker_path = root.path().join(BACKUP_PAIR_MARKER_NAME);

    let old_config_hash = "55".repeat(32);
    let old_drop_in_hash = "66".repeat(32);
    let mut old_receipt = completion_record(&config_path, &old_config_hash, &old_drop_in_hash);
    old_receipt["txid"] = serde_json::json!("receipt-old");
    old_receipt["drop_in_temp_name"] =
        serde_json::json!(".iotkit-backup-pair.receipt-old.drop-in.tmp");
    let mut current_receipt =
        completion_record(&config_path, &test_digest(&config_bytes), &"44".repeat(32));
    current_receipt["config_existed"] = serde_json::json!(true);
    current_receipt["drop_in_existed"] = serde_json::json!(true);
    current_receipt["old_config_hash"] = serde_json::json!(old_config_hash);
    current_receipt["old_drop_in_hash"] = serde_json::json!(old_drop_in_hash);
    write_private_json(&marker_path, &old_receipt);
    write_private_json(&receipt_path, &current_receipt);
    assert!(
        load_owner_only_config(&config_path).is_ok(),
        "strict post-commit cleanup state should remain readable"
    );

    let mut pending = current_receipt.clone();
    pending["phase"] = serde_json::json!("prepared");
    pending["config_hash"] = serde_json::Value::Null;
    pending["drop_in_hash"] = serde_json::Value::Null;
    pending["config_temp_name"] = serde_json::Value::Null;
    pending["drop_in_temp_name"] = serde_json::Value::Null;
    write_private_json(&marker_path, &pending);
    assert_eq!(
        load_owner_only_config(&config_path),
        Err(RecoveryError::CleanupRequired)
    );

    write_private_json(&marker_path, &serde_json::json!({"schema_version": 3}));
    assert_eq!(
        load_owner_only_config(&config_path),
        Err(RecoveryError::CleanupRequired)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn configure_backup_writes_schema_one_owner_only_json_and_refuses_replacement() {
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let input = prepare_config(root.path());
    let mountinfo = mountinfo_for(&input.destination);

    crate::config::configure_backup_with(
        &config_path,
        &input,
        BackupConfigReplace::Refuse,
        &mountinfo,
        crate::config::ConfigWriteOps::system(),
    )
    .unwrap();
    let configured = load_owner_only_config(&config_path).unwrap();
    assert_eq!(configured.expected_mount.mount_point, input.destination);
    assert_eq!(configured.expected_mount.source, "server:/derived-live");
    assert_eq!(configured.expected_mount.filesystem_type, "nfs4");
    assert!(
        configured
            .expected_mount
            .filesystem_id
            .ends_with("|server:/derived-live")
    );
    assert_ne!(configured.expected_mount, input.expected_mount);
    assert_eq!(
        crate::config::configure_backup_with(
            &config_path,
            &input,
            BackupConfigReplace::Refuse,
            &mountinfo,
            crate::config::ConfigWriteOps::system(),
        ),
        Err(RecoveryError::DestinationExists)
    );
    let mut replacement = input.clone();
    replacement.retention_count = 7;
    crate::config::configure_backup_with(
        &config_path,
        &replacement,
        BackupConfigReplace::ReplaceExisting,
        &mountinfo,
        crate::config::ConfigWriteOps::system(),
    )
    .unwrap();
    let loaded = load_owner_only_config(&config_path).unwrap();
    assert_eq!(loaded.retention_count, 7);
    assert_eq!(loaded.expected_mount, configured.expected_mount);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(config_path).unwrap().permissions().mode() & 0o077,
            0
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn configure_rejects_a_staging_parent_that_is_not_tmpfs() {
    use std::os::unix::fs::PermissionsExt;

    let root = TempDir::new_in("/dev/shm").unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let destination = root.path().join("destination");
    fs::create_dir(&destination).unwrap();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).unwrap();
    let config_path = root.path().join("backup.json");
    let mut input = config(root.path());
    input.destination = destination.clone();
    input.staging_directory = "/proc/self/iotkit-staging-test".into();
    input.expected_mount.mount_point = destination;

    assert_eq!(
        configure_backup(&config_path, &input, BackupConfigReplace::Refuse),
        Err(RecoveryError::DestinationInvalid)
    );
    assert!(!config_path.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn competing_configure_is_busy_before_creating_any_temporary_name() {
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let input = prepare_config(root.path());
    let guard = acquire_recovery_operation(&config_path).unwrap();
    let lock_path = root.path().join(".iotkit-recovery.lock");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&lock_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let mut before: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    before.sort();

    assert_eq!(
        configure_backup(&config_path, &input, BackupConfigReplace::Refuse),
        Err(RecoveryError::OperationBusy)
    );
    let mut after: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    after.sort();
    assert_eq!(after, before);
    assert_eq!(RecoveryError::OperationBusy.reason_code(), "operation_busy");

    drop(guard);
    configure_backup(&config_path, &input, BackupConfigReplace::Refuse).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn configure_detects_exact_config_cleanup_markers_before_creating_a_temporary_name() {
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let input = prepare_config(root.path());
    let exact_file = ".iotkit-cleanup-0123456789abcdef0123456789abcdef";
    let exact_directory = ".iotkit-cleanup-dir-fedcba9876543210fedcba9876543210";
    let near_file = ".iotkit-cleanup-user-note";
    let near_directory = ".iotkit-cleanup-dir-0123456789abcdef0123456789abcde";

    fs::write(root.path().join(exact_file), b"preserve").unwrap();
    assert_eq!(
        configure_backup(&config_path, &input, BackupConfigReplace::Refuse),
        Err(RecoveryError::CleanupRequired)
    );
    assert_eq!(fs::read(root.path().join(exact_file)).unwrap(), b"preserve");
    assert!(root.path().join(".iotkit-recovery.lock").is_file());
    assert!(!config_path.exists());
    assert!(
        config_parent_entries(root.path())
            .iter()
            .all(|name| !name.to_string_lossy().ends_with(".iotkit-config"))
    );

    fs::remove_file(root.path().join(exact_file)).unwrap();
    fs::create_dir(root.path().join(exact_directory)).unwrap();
    let entries_before = config_parent_entries(root.path());
    assert_eq!(
        configure_backup(&config_path, &input, BackupConfigReplace::Refuse),
        Err(RecoveryError::CleanupRequired)
    );
    assert_eq!(config_parent_entries(root.path()), entries_before);
    assert!(root.path().join(exact_directory).is_dir());
    assert!(!config_path.exists());

    fs::remove_dir(root.path().join(exact_directory)).unwrap();
    fs::write(root.path().join(near_file), b"keep").unwrap();
    fs::create_dir(root.path().join(near_directory)).unwrap();
    configure_backup(&config_path, &input, BackupConfigReplace::Refuse).unwrap();
    assert_eq!(fs::read(root.path().join(near_file)).unwrap(), b"keep");
    assert!(root.path().join(near_directory).is_dir());
    assert!(root.path().join(".iotkit-recovery.lock").is_file());
}

#[cfg(target_os = "linux")]
#[test]
fn competing_configure_reports_busy_before_scanning_config_cleanup_markers() {
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let input = prepare_config(root.path());
    let marker = ".iotkit-cleanup-0123456789abcdef0123456789abcdef";
    fs::write(root.path().join(marker), b"preserve").unwrap();
    let guard = acquire_recovery_operation(&config_path).unwrap();
    let entries_before = config_parent_entries(root.path());

    assert_eq!(
        configure_backup(&config_path, &input, BackupConfigReplace::Refuse),
        Err(RecoveryError::OperationBusy)
    );
    assert_eq!(config_parent_entries(root.path()), entries_before);

    drop(guard);
    assert_eq!(
        configure_backup(&config_path, &input, BackupConfigReplace::Refuse),
        Err(RecoveryError::CleanupRequired)
    );
    assert_eq!(config_parent_entries(root.path()), entries_before);
    assert_eq!(fs::read(root.path().join(marker)).unwrap(), b"preserve");
    assert!(!config_path.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn configuration_rejects_non_owner_only_parent_before_lock_or_temporary_creation() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let input = prepare_config(root.path());
    let original_entries: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();

    for mode in [0o777, 0o720] {
        fs::set_permissions(root.path(), fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(
            configure_backup(&config_path, &input, BackupConfigReplace::Refuse),
            Err(RecoveryError::InvalidConfiguration)
        );
        let entries: Vec<_> = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, original_entries);
    }
}

#[cfg(target_os = "linux")]
fn substitute_validated_config(parent_fd: RawFd, name: &std::ffi::CStr) -> std::io::Result<()> {
    use std::os::fd::FromRawFd;
    let preserved = c"validated-old";
    if unsafe { libc::renameat(parent_fd, name.as_ptr(), parent_fd, preserved.as_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let replacement_fd = unsafe {
        libc::openat(
            parent_fd,
            name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if replacement_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut replacement = unsafe { fs::File::from_raw_fd(replacement_fd) };
    use std::io::Write as _;
    replacement.write_all(b"attacker replacement")
}

#[cfg(target_os = "linux")]
#[test]
fn replacement_rolls_back_when_the_validated_inode_is_substituted() {
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let input = prepare_config(root.path());
    let mountinfo = mountinfo_for(&input.destination);
    crate::config::configure_backup_with(
        &config_path,
        &input,
        BackupConfigReplace::Refuse,
        &mountinfo,
        crate::config::ConfigWriteOps::system(),
    )
    .unwrap();
    let old = fs::read(&config_path).unwrap();
    let mut changed = input.clone();
    changed.retention_count = 9;
    let mut ops = crate::config::ConfigWriteOps::system();
    ops.after_existing_open = substitute_validated_config;

    assert_eq!(
        crate::config::configure_backup_with(
            &config_path,
            &changed,
            BackupConfigReplace::ReplaceExisting,
            &mountinfo,
            ops,
        ),
        Err(RecoveryError::InvalidConfiguration)
    );
    assert_eq!(fs::read(&config_path).unwrap(), b"attacker replacement");
    assert_eq!(fs::read(root.path().join("validated-old")).unwrap(), old);
    assert!(fs::read_dir(root.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".iotkit-config-")
    }));
}

#[cfg(target_os = "linux")]
fn substitute_config_cleanup(parent_fd: RawFd, name: &std::ffi::CStr) -> std::io::Result<()> {
    use std::os::fd::FromRawFd;
    if unsafe {
        libc::renameat(
            parent_fd,
            name.as_ptr(),
            parent_fd,
            c".preserved-config-owned".as_ptr(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let replacement_fd = unsafe {
        libc::openat(
            parent_fd,
            name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if replacement_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut replacement = unsafe { fs::File::from_raw_fd(replacement_fd) };
    use std::io::Write as _;
    replacement.write_all(b"unrelated-config")
}

#[cfg(target_os = "linux")]
#[test]
fn config_cleanup_substitution_preserves_the_unrelated_file_and_fails() {
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let input = prepare_config(root.path());
    let mountinfo = mountinfo_for(&input.destination);
    crate::config::configure_backup_with(
        &config_path,
        &input,
        BackupConfigReplace::Refuse,
        &mountinfo,
        crate::config::ConfigWriteOps::system(),
    )
    .unwrap();
    let mut ops = crate::config::ConfigWriteOps::system();
    ops.before_cleanup = substitute_config_cleanup;

    assert_eq!(
        crate::config::configure_backup_with(
            &config_path,
            &input,
            BackupConfigReplace::Refuse,
            &mountinfo,
            ops,
        ),
        Err(RecoveryError::ArtifactCleanupFailed)
    );
    assert_eq!(
        fs::read(root.path().join(".preserved-config-owned"))
            .unwrap()
            .first(),
        Some(&b'{')
    );
    assert!(fs::read_dir(root.path()).unwrap().any(|entry| {
        fs::read(entry.unwrap().path()).ok().as_deref() == Some(b"unrelated-config")
    }));

    let mut before: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    before.sort();
    assert_eq!(
        crate::config::configure_backup_with(
            &config_path,
            &input,
            BackupConfigReplace::Refuse,
            &mountinfo,
            crate::config::ConfigWriteOps::system(),
        ),
        Err(RecoveryError::DestinationExists)
    );
    let mut after: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    after.sort();
    assert_eq!(after, before);
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

#[cfg(target_os = "linux")]
#[test]
fn replacement_refuses_a_dangling_symlink_without_publishing_configuration() {
    use std::os::unix::fs::symlink;
    let root = TempDir::new().unwrap();
    let config_path = root.path().join("backup.json");
    let input = prepare_config(root.path());
    symlink(root.path().join("missing"), &config_path).unwrap();

    assert_eq!(
        configure_backup(&config_path, &input, BackupConfigReplace::ReplaceExisting,),
        Err(RecoveryError::InvalidConfiguration)
    );
    assert!(
        fs::symlink_metadata(&config_path)
            .unwrap()
            .file_type()
            .is_symlink()
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
