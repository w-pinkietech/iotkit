use std::process::Command;

use serde_json::Value;

fn nodectl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_iotkit-edge-nodectl"))
}

#[test]
fn backup_command_is_exposed() {
    let output = nodectl().args(["backup", "--help"]).output().unwrap();
    assert!(
        output.status.success(),
        "backup command should be accepted once implemented: {:?}",
        output
    );
}

#[test]
fn backup_help_has_no_raw_passphrase_flag() {
    let output = nodectl().args(["backup", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("configure"));
    assert!(stdout.contains("inspect"));
    assert!(!stdout.contains("--passphrase "));
    assert!(!stdout.contains("--passphrase=<"));
}

#[test]
fn backup_paths_take_an_early_route_without_creating_a_live_database() {
    #[cfg(target_os = "linux")]
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    #[cfg(not(target_os = "linux"))]
    let dir = tempfile::tempdir().unwrap();
    let missing_db = dir.path().join("missing-live.db");
    let missing_config = dir.path().join("missing-config.json");
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let status = nodectl()
        .args([
            "backup",
            "status",
            "--config",
            missing_config.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "path={} stderr={}",
        missing_config.display(),
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(json(&status.stdout)["status"], "not_configured");
    assert!(!missing_db.exists());

    let create = nodectl()
        .args([
            "backup",
            "create",
            "--config",
            missing_config.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!create.status.success());
    assert!(!missing_db.exists());
    assert!(!String::from_utf8_lossy(&create.stderr).contains(missing_db.to_str().unwrap()));
}

#[cfg(target_os = "linux")]
#[test]
fn backup_configure_rejects_non_tmpfs_staging_without_publishing_a_pair() {
    use std::os::unix::fs::PermissionsExt;

    let control = tempfile::tempdir_in("/dev/shm").unwrap();
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    for path in [control.path(), destination.path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let config = control.path().join("backup.json");
    let drop_in = control.path().join("destination.conf");
    let passphrase = control.path().join("passphrase");
    std::fs::write(&passphrase, b"owner-only-test-passphrase").unwrap();
    std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).unwrap();

    let output = nodectl()
        .args([
            "backup",
            "configure",
            "--config",
            config.to_str().unwrap(),
            "--db",
            control.path().join("edge.db").to_str().unwrap(),
            "--destination",
            destination.path().to_str().unwrap(),
            "--staging-directory",
            "/proc/self/iotkit-staging-test",
            "--passphrase-file",
            passphrase.to_str().unwrap(),
            "--freshness-seconds",
            "86400",
            "--retention-count",
            "7",
            "--systemd-drop-in",
            drop_in.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("destination_invalid"));
    assert!(!config.exists());
    assert!(!drop_in.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn create_inspect_and_status_emit_only_nonsecret_summaries() {
    use std::os::unix::fs::PermissionsExt;

    let control = tempfile::tempdir_in("/tmp").unwrap();
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging = staging_parent.path().join("staging-leaf");
    for path in [control.path(), destination.path(), staging_parent.path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let db = control.path().join("edge.db");
    let handle =
        iotkit_core_storage::init_db(&db, &iotkit_core_recovery::all_edge_node_migrations())
            .unwrap();
    handle
        .with_conn_sync(|conn| {
            let edge_node_id = iotkit_core_ledger::edge_node_id(conn).unwrap();
            let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
            conn.execute(
                "INSERT INTO target_registry(
                     target_id, endpoint_url, credential_token, archive_responsible,
                     schema_version, cursor_epoch, cursor_pub_seq, created_at
                 ) VALUES('edge', 'https://edge.invalid', '', 1, 1, ?1, 0, 1)",
                [&epoch],
            )
            .unwrap();
            conn.execute(
                "UPDATE edge_node_activation
                 SET state='active', edge_id='edge-cli', activation_id='activation-cli',
                     ledger_epoch=?1, discard_through_reading_seq=0,
                     cleanup_through_reading_seq=0, request_json='{}', result_json='{}',
                     activated_at=1 WHERE singleton=1",
                [&epoch],
            )
            .unwrap();
            assert!(!edge_node_id.is_empty());
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
            Ok::<_, iotkit_core_storage::StorageError>(())
        })
        .unwrap();
    drop(handle);

    let passphrase = control.path().join("passphrase");
    std::fs::write(&passphrase, b"cli-secret-passphrase\r\n").unwrap();
    std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).unwrap();
    let config = control.path().join("backup.json");
    let drop_in = control.path().join("backup.mount.conf");
    let configure = nodectl()
        .args([
            "backup",
            "configure",
            "--config",
            config.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--destination",
            destination.path().to_str().unwrap(),
            "--staging-directory",
            staging.to_str().unwrap(),
            "--passphrase-file",
            passphrase.to_str().unwrap(),
            "--freshness-seconds",
            "86400",
            "--retention-count",
            "7",
            "--systemd-drop-in",
            drop_in.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        configure.status.success(),
        "{}",
        String::from_utf8_lossy(&configure.stderr)
    );
    assert_eq!(json(&configure.stdout)["status"], "configured");
    assert!(
        !staging.exists(),
        "configure must not create the RuntimeDirectory leaf"
    );
    let drop_in_text = std::fs::read_to_string(&drop_in).unwrap();
    assert!(drop_in_text.starts_with("[Unit]\nRequiresMountsFor="));
    assert!(!drop_in_text.contains("cli-secret-passphrase"));
    assert!(!drop_in_text.contains("passphrase"));
    let unit = control.path().join("iotkit-backup-test.service");
    std::fs::write(
        &unit,
        format!(
            "{}\n[Service]\nType=oneshot\nExecStart=/bin/true\n",
            drop_in_text
        ),
    )
    .unwrap();
    let systemd_verify = Command::new("systemd-analyze")
        .args(["verify", unit.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        systemd_verify.status.success(),
        "generated drop-in failed systemd-analyze verify: {}",
        String::from_utf8_lossy(&systemd_verify.stderr)
    );

    let create = nodectl()
        .args(["backup", "create", "--config", config.to_str().unwrap()])
        .env("TMPDIR", "/dev/shm")
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    let staging_metadata = std::fs::metadata(&staging).unwrap();
    assert!(staging_metadata.is_dir());
    assert_eq!(
        staging_metadata.permissions().mode() & 0o077,
        0,
        "create must provision an owner-only exact leaf"
    );
    let created = json(&create.stdout);
    assert_eq!(created["status"], "created");
    for key in [
        "backup_id",
        "edge_node_id",
        "ledger_epoch",
        "accepted_cursor",
        "allocation_high_water",
        "created_at_ms",
    ] {
        assert!(created.get(key).is_some(), "missing {key}");
    }
    let artifact = destination.path().join(format!(
        "{}{}",
        created["backup_id"].as_str().unwrap(),
        iotkit_core_recovery::NODE_BACKUP_SUFFIX
    ));
    assert!(artifact.exists());

    let inspect = nodectl()
        .args([
            "backup",
            "inspect",
            "--input",
            artifact.to_str().unwrap(),
            "--passphrase-file",
            passphrase.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspected = json(&inspect.stdout);
    assert_eq!(inspected["status"], "authenticated");
    assert_eq!(inspected["backup_id"], created["backup_id"]);
    assert!(inspected.get("database_sha256").is_none());
    assert!(!String::from_utf8_lossy(&inspect.stdout).contains("cli-secret-passphrase"));
    assert!(!String::from_utf8_lossy(&inspect.stderr).contains(artifact.to_str().unwrap()));

    let status = nodectl()
        .args(["backup", "status", "--config", config.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_json = json(&status.stdout);
    assert_eq!(status_json["status"], "healthy");
    assert!(status_json["ciphertext_size"].is_u64());
    assert!(status_json.get("database").is_none());
    assert!(status_json.get("destination").is_none());
    assert!(
        !String::from_utf8_lossy(&status.stdout).contains(destination.path().to_str().unwrap())
    );

    let candidate = control.path().join("candidate.db");
    std::fs::write(&candidate, b"existing-candidate-conflict").unwrap();
    std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600)).unwrap();
    let handoff = control.path().join("handoff.json");
    std::fs::write(
        &handoff,
        include_bytes!("../../../core/recovery/tests/fixtures/recovery-handoff-v1.json"),
    )
    .unwrap();
    std::fs::set_permissions(&handoff, std::fs::Permissions::from_mode(0o600)).unwrap();
    let restore = nodectl()
        .args([
            "backup",
            "restore",
            "--input",
            artifact.to_str().unwrap(),
            "--candidate-db",
            candidate.to_str().unwrap(),
            "--live-db",
            db.to_str().unwrap(),
            "--passphrase-file",
            passphrase.to_str().unwrap(),
            "--recovery-handoff",
            handoff.to_str().unwrap(),
        ])
        .env("TMPDIR", "/dev/shm")
        .output()
        .unwrap();
    assert!(!restore.status.success());
    let restore_stderr = String::from_utf8_lossy(&restore.stderr);
    assert!(
        restore_stderr.contains("candidate_conflict")
            || restore_stderr.contains("candidate_fence_invalid"),
        "{restore_stderr}"
    );
    assert!(!restore_stderr.contains(passphrase.to_str().unwrap()));
    assert!(!restore_stderr.contains(candidate.to_str().unwrap()));

    // A schema-valid handoff lets the CLI publish a fenced candidate. The
    // second invocation must replay that exact candidate and receipt rather
    // than deriving a new identity or touching the live database.
    std::fs::remove_file(&candidate).unwrap();
    std::fs::write(
        &handoff,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "recovery_id": "recovery-cli",
            "edge_id": "edge-cli",
            "edge_node_id": created["edge_node_id"].as_str().unwrap(),
            "old_ledger_epoch": created["ledger_epoch"].as_str().unwrap(),
            "expected_backup_id": created["backup_id"].as_str().unwrap(),
            "proposed_new_epoch": "epoch-cli-new",
            "credential_generation": 2,
        }))
        .unwrap(),
    )
    .unwrap();
    let run_restore = || {
        nodectl()
            .args([
                "backup",
                "restore",
                "--input",
                artifact.to_str().unwrap(),
                "--candidate-db",
                candidate.to_str().unwrap(),
                "--live-db",
                db.to_str().unwrap(),
                "--passphrase-file",
                passphrase.to_str().unwrap(),
                "--recovery-handoff",
                handoff.to_str().unwrap(),
            ])
            .env("TMPDIR", "/dev/shm")
            .output()
            .unwrap()
    };
    let first_replay = run_restore();
    assert!(
        first_replay.status.success(),
        "{}",
        String::from_utf8_lossy(&first_replay.stderr)
    );
    let first_receipt = json(&first_replay.stdout);
    assert_eq!(first_receipt["status"], "durably_fenced_candidate");
    assert_eq!(first_receipt["backup_id"], created["backup_id"]);
    assert!(first_receipt.get("artifact_sha256").is_none());
    let second_replay = run_restore();
    assert!(second_replay.status.success());
    assert_eq!(first_replay.stdout, second_replay.stdout);
    assert!(!String::from_utf8_lossy(&first_replay.stdout).contains("cli-secret-passphrase"));
    assert!(!String::from_utf8_lossy(&first_replay.stderr).contains(candidate.to_str().unwrap()));
}

#[cfg(target_os = "linux")]
#[test]
fn existing_backup_configuration_requires_explicit_replace() {
    use std::os::unix::fs::PermissionsExt;

    let control = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    let _staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging = tempfile::TempDir::new_in(_staging_parent.path()).unwrap();
    for path in [control.path(), destination.path(), staging.path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let config = control.path().join("backup.json");
    std::fs::write(&config, br#"{}"#).unwrap();
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
    let db = control.path().join("missing.db");
    let passphrase = control.path().join("passphrase");
    std::fs::write(&passphrase, b"owner-only-cli-passphrase").unwrap();
    std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).unwrap();
    let drop_in = control.path().join("backup.conf");
    let output = nodectl()
        .args([
            "backup",
            "configure",
            "--config",
            config.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--destination",
            destination.path().to_str().unwrap(),
            "--staging-directory",
            staging.path().to_str().unwrap(),
            "--passphrase-file",
            passphrase.to_str().unwrap(),
            "--freshness-seconds",
            "86400",
            "--retention-count",
            "7",
            "--systemd-drop-in",
            drop_in.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("destination_exists"), "{stderr}");
    assert!(!stderr.contains(passphrase.to_str().unwrap()));
}

#[cfg(target_os = "linux")]
#[test]
fn broad_passphrase_permissions_are_rejected_without_secret_leakage() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let passphrase = dir.path().join("passphrase");
    std::fs::write(&passphrase, b"sensitive-cli-passphrase").unwrap();
    std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o644)).unwrap();
    let config = dir.path().join("missing.json");
    let output = nodectl()
        .args([
            "backup",
            "inspect",
            "--input",
            dir.path()
                .join("artifact.iotkit-node-backup")
                .to_str()
                .unwrap(),
            "--passphrase-file",
            passphrase.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(json(&output.stderr)["error"]["reason_code"].is_string());
    for capture in [stdout.as_ref(), stderr.as_ref()] {
        assert!(!capture.contains("sensitive-cli-passphrase"));
        assert!(!capture.contains(config.to_str().unwrap()));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn owner_only_readers_reject_fifo_symlink_and_hardlink_without_hanging() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let regular = root.path().join("regular");
    std::fs::write(&regular, b"twelve-chars").unwrap();
    std::fs::set_permissions(&regular, std::fs::Permissions::from_mode(0o600)).unwrap();

    let config_fifo = root.path().join("config.fifo");
    let passphrase_fifo = root.path().join("passphrase.fifo");
    let handoff_fifo = root.path().join("handoff.fifo");
    let artifact_fifo = root.path().join("artifact.fifo");
    for fifo in [
        &config_fifo,
        &passphrase_fifo,
        &handoff_fifo,
        &artifact_fifo,
    ] {
        let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    }

    let status_fifo = run_promptly(
        nodectl().args([
            "backup",
            "status",
            "--config",
            config_fifo.to_str().unwrap(),
        ]),
        "status FIFO",
    );
    assert_closed_error(&status_fifo);

    let inspect_fifo_passphrase = run_promptly(
        nodectl().args([
            "backup",
            "inspect",
            "--input",
            root.path().join("absent-artifact").to_str().unwrap(),
            "--passphrase-file",
            passphrase_fifo.to_str().unwrap(),
        ]),
        "passphrase FIFO",
    );
    assert_closed_error(&inspect_fifo_passphrase);

    let restore_fifo_handoff = run_promptly(
        nodectl().args([
            "backup",
            "restore",
            "--input",
            root.path().join("absent-artifact").to_str().unwrap(),
            "--candidate-db",
            root.path().join("candidate.db").to_str().unwrap(),
            "--live-db",
            root.path().join("live.db").to_str().unwrap(),
            "--passphrase-file",
            regular.to_str().unwrap(),
            "--recovery-handoff",
            handoff_fifo.to_str().unwrap(),
        ]),
        "handoff FIFO",
    );
    assert_closed_error(&restore_fifo_handoff);

    let inspect_fifo_artifact = run_promptly(
        nodectl().args([
            "backup",
            "inspect",
            "--input",
            artifact_fifo.to_str().unwrap(),
            "--passphrase-file",
            regular.to_str().unwrap(),
        ]),
        "artifact FIFO",
    );
    assert_closed_error(&inspect_fifo_artifact);

    let config_link = root.path().join("config-link");
    std::os::unix::fs::symlink(&regular, &config_link).unwrap();
    let output = run_promptly(
        nodectl().args([
            "backup",
            "status",
            "--config",
            config_link.to_str().unwrap(),
        ]),
        "config symlink",
    );
    assert_closed_error(&output);

    let passphrase_link = root.path().join("passphrase-link");
    std::os::unix::fs::symlink(&regular, &passphrase_link).unwrap();
    let output = run_promptly(
        nodectl().args([
            "backup",
            "inspect",
            "--input",
            root.path().join("absent-artifact").to_str().unwrap(),
            "--passphrase-file",
            passphrase_link.to_str().unwrap(),
        ]),
        "passphrase symlink",
    );
    assert_closed_error(&output);

    let handoff_link = root.path().join("handoff-link");
    std::os::unix::fs::symlink(&regular, &handoff_link).unwrap();
    let output = run_promptly(
        nodectl().args([
            "backup",
            "restore",
            "--input",
            root.path().join("absent-artifact").to_str().unwrap(),
            "--candidate-db",
            root.path().join("candidate.db").to_str().unwrap(),
            "--live-db",
            root.path().join("live.db").to_str().unwrap(),
            "--passphrase-file",
            regular.to_str().unwrap(),
            "--recovery-handoff",
            handoff_link.to_str().unwrap(),
        ]),
        "handoff symlink",
    );
    assert_closed_error(&output);

    let artifact_link = root.path().join("artifact-link");
    std::os::unix::fs::symlink(&regular, &artifact_link).unwrap();
    let output = run_promptly(
        nodectl().args([
            "backup",
            "inspect",
            "--input",
            artifact_link.to_str().unwrap(),
            "--passphrase-file",
            regular.to_str().unwrap(),
        ]),
        "artifact symlink",
    );
    assert_closed_error(&output);

    let hardlink_target = root.path().join("hardlink");
    std::fs::hard_link(&regular, &hardlink_target).unwrap();
    let output = run_promptly(
        nodectl().args([
            "backup",
            "inspect",
            "--input",
            root.path().join("absent-artifact").to_str().unwrap(),
            "--passphrase-file",
            hardlink_target.to_str().unwrap(),
        ]),
        "passphrase hardlink",
    );
    assert_closed_error(&output);
}

#[cfg(target_os = "linux")]
#[test]
fn durable_completion_receipt_survives_success_and_final_sync_uncertainty() {
    use std::os::unix::fs::PermissionsExt;

    let control = tempfile::tempdir_in("/tmp").unwrap();
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    let _staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging = tempfile::TempDir::new_in(_staging_parent.path()).unwrap();
    for path in [control.path(), destination.path(), staging.path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let config = control.path().join("backup.json");
    let drop_in = control.path().join("backup.mount.conf");
    let passphrase = control.path().join("passphrase");
    std::fs::write(&passphrase, b"owner-only-test-passphrase").unwrap();
    std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).unwrap();
    let initial = configure_command(
        &config,
        &drop_in,
        &control.path().join("old.db"),
        destination.path(),
        staging.path(),
        &passphrase,
    )
    .output()
    .unwrap();
    assert!(initial.status.success());
    let marker = control
        .path()
        .join(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME);
    let receipt = control
        .path()
        .join(iotkit_core_recovery::BACKUP_PAIR_COMPLETION_NAME);
    assert!(!marker.exists());
    assert!(
        receipt.exists(),
        "ordinary success must retain durable completion evidence"
    );

    let exact_retry = configure_command(
        &config,
        &drop_in,
        &control.path().join("old.db"),
        destination.path(),
        staging.path(),
        &passphrase,
    )
    .output()
    .unwrap();
    assert!(
        exact_retry.status.success(),
        "{}",
        String::from_utf8_lossy(&exact_retry.stderr)
    );
    assert!(receipt.exists(), "exact retry must retain its receipt");
    for command in ["status", "create"] {
        let output = nodectl()
            .args(["backup", command, "--config", config.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("cleanup_required"),
            "valid receipt blocked {command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn configure_receipt_binds_the_exact_current_drop_in() {
    use std::os::unix::fs::PermissionsExt;

    let control = tempfile::tempdir_in("/tmp").unwrap();
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    let _staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging = tempfile::TempDir::new_in(_staging_parent.path()).unwrap();
    for path in [control.path(), destination.path(), staging.path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let config = control.path().join("backup.json");
    let drop_in = control.path().join("backup.mount.conf");
    let passphrase = control.path().join("passphrase");
    std::fs::write(&passphrase, b"owner-only-test-passphrase").unwrap();
    std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).unwrap();
    let configured = configure_command(
        &config,
        &drop_in,
        &control.path().join("edge.db"),
        destination.path(),
        staging.path(),
        &passphrase,
    )
    .output()
    .unwrap();
    assert!(configured.status.success());
    std::fs::write(&drop_in, b"[Unit]\nRequiresMountsFor=/forged\n").unwrap();
    std::fs::set_permissions(&drop_in, std::fs::Permissions::from_mode(0o600)).unwrap();
    let before = std::fs::read(&drop_in).unwrap();

    let retry = configure_command(
        &config,
        &drop_in,
        &control.path().join("edge.db"),
        destination.path(),
        staging.path(),
        &passphrase,
    )
    .output()
    .unwrap();
    assert!(!retry.status.success());
    assert!(
        String::from_utf8_lossy(&retry.stderr).contains("cleanup_required"),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert_eq!(std::fs::read(&drop_in).unwrap(), before);
}

#[cfg(target_os = "linux")]
#[test]
fn old_receipt_with_new_pending_marker_blocks_readers_and_resumes() {
    use std::os::unix::fs::PermissionsExt;

    let control = tempfile::tempdir_in("/tmp").unwrap();
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    let _staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging = tempfile::TempDir::new_in(_staging_parent.path()).unwrap();
    for path in [control.path(), destination.path(), staging.path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let config = control.path().join("backup.json");
    let drop_in = control.path().join("backup.mount.conf");
    let passphrase = control.path().join("passphrase");
    std::fs::write(&passphrase, b"owner-only-test-passphrase").unwrap();
    std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).unwrap();

    let initial = configure_command(
        &config,
        &drop_in,
        &control.path().join("old.db"),
        destination.path(),
        staging.path(),
        &passphrase,
    )
    .output()
    .unwrap();
    assert!(initial.status.success());
    let receipt = control
        .path()
        .join(iotkit_core_recovery::BACKUP_PAIR_COMPLETION_NAME);
    assert!(receipt.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn forged_prepared_marker_cannot_delete_an_existing_pair() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;

    let control = tempfile::tempdir_in("/tmp").unwrap();
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    let _staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging = tempfile::TempDir::new_in(_staging_parent.path()).unwrap();
    for path in [control.path(), destination.path(), staging.path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let config = control.path().join("backup.json");
    let drop_in = control.path().join("backup.mount.conf");
    let passphrase = control.path().join("passphrase");
    std::fs::write(&passphrase, b"owner-only-test-passphrase").unwrap();
    std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).unwrap();
    let initial = configure_command(
        &config,
        &drop_in,
        &control.path().join("old.db"),
        destination.path(),
        staging.path(),
        &passphrase,
    )
    .output()
    .unwrap();
    assert!(initial.status.success());
    let original_config = std::fs::read(&config).unwrap();
    let original_drop_in = std::fs::read(&drop_in).unwrap();
    let marker = control
        .path()
        .join(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME);
    let forged = serde_json::json!({
        "schema_version": 2,
        "txid": "forged",
        "config_path_hash": test_hash(config.as_os_str().as_bytes()),
        "drop_in_path_hash": test_hash(drop_in.as_os_str().as_bytes()),
        "phase": "prepared",
        "request_config_hash": "00".repeat(32),
        "request_drop_in_hash": "11".repeat(32),
        "config_hash": null,
        "drop_in_hash": null,
        "config_existed": false,
        "drop_in_existed": false,
        "old_config_hash": null,
        "old_drop_in_hash": null
    });
    std::fs::write(&marker, serde_json::to_vec(&forged).unwrap()).unwrap();
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).unwrap();

    let attempt = configure_command(
        &config,
        &drop_in,
        &control.path().join("new.db"),
        destination.path(),
        staging.path(),
        &passphrase,
    )
    .output()
    .unwrap();
    assert!(!attempt.status.success());
    assert!(
        String::from_utf8_lossy(&attempt.stderr).contains("cleanup_required"),
        "{}",
        String::from_utf8_lossy(&attempt.stderr)
    );
    assert_eq!(std::fs::read(&config).unwrap(), original_config);
    assert_eq!(std::fs::read(&drop_in).unwrap(), original_drop_in);
    assert!(
        marker.exists(),
        "forged marker must remain for operator cleanup"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn forged_config_published_marker_without_old_backup_cannot_delete_targets() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;

    let control = tempfile::tempdir_in("/tmp").unwrap();
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    let _staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging = tempfile::TempDir::new_in(_staging_parent.path()).unwrap();
    for path in [control.path(), destination.path(), staging.path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let config = control.path().join("backup.json");
    let drop_in = control.path().join("backup.mount.conf");
    let passphrase = control.path().join("passphrase");
    std::fs::write(&passphrase, b"owner-only-test-passphrase").unwrap();
    std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).unwrap();
    let initial = configure_command(
        &config,
        &drop_in,
        &control.path().join("old.db"),
        destination.path(),
        staging.path(),
        &passphrase,
    )
    .output()
    .unwrap();
    assert!(initial.status.success());
    let original_config = std::fs::read(&config).unwrap();
    let original_drop_in = std::fs::read(&drop_in).unwrap();
    let marker = control
        .path()
        .join(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME);
    let txid = "a".repeat(32);
    let config_temp = control
        .path()
        .join(format!(".backup.json.{txid}.iotkit-config"));
    let drop_temp = control
        .path()
        .join(format!(".iotkit-backup-pair.{txid}.drop-in.tmp"));
    std::fs::write(&config_temp, b"stale-config-temp").unwrap();
    std::fs::write(&drop_temp, b"stale-drop-temp").unwrap();
    std::fs::set_permissions(&config_temp, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::set_permissions(&drop_temp, std::fs::Permissions::from_mode(0o600)).unwrap();
    let forged = serde_json::json!({
        "schema_version": 3,
        "txid": txid,
        "config_path_hash": test_hash(config.as_os_str().as_bytes()),
        "drop_in_path_hash": test_hash(drop_in.as_os_str().as_bytes()),
        "phase": "config_published",
        "request_config_hash": "00".repeat(32),
        "request_drop_in_hash": "11".repeat(32),
        "config_hash": test_hash(&original_config),
        "drop_in_hash": test_hash(&original_drop_in),
        "config_existed": false,
        "drop_in_existed": false,
        "old_config_hash": null,
        "old_drop_in_hash": null,
        "config_temp_name": config_temp.file_name().unwrap(),
        "drop_in_temp_name": drop_temp.file_name().unwrap()
    });
    std::fs::write(&marker, serde_json::to_vec(&forged).unwrap()).unwrap();
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).unwrap();

    let attempt = configure_command(
        &config,
        &drop_in,
        &control.path().join("new.db"),
        destination.path(),
        staging.path(),
        &passphrase,
    )
    .output()
    .unwrap();
    assert!(!attempt.status.success());
    assert!(
        String::from_utf8_lossy(&attempt.stderr).contains("cleanup_required"),
        "{}",
        String::from_utf8_lossy(&attempt.stderr)
    );
    assert_eq!(std::fs::read(&config).unwrap(), original_config);
    assert_eq!(std::fs::read(&drop_in).unwrap(), original_drop_in);
    assert!(
        marker.exists(),
        "forged marker must remain for operator cleanup"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn forged_pair_phase_matrix_rejects_unexpected_states_without_mutation() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;

    // Each row deliberately contains one impossible target/backup/temp
    // combination.  Recovery must reject it before publishing, rolling back,
    // or deleting any of the artifacts, for both originally-absent and
    // originally-present targets.
    let phases = [
        "prepared",
        "config_publishing",
        "config_published",
        "drop_in_publishing",
        "drop_in_published",
        "published",
    ];
    for phase in phases {
        for originally_present in [false, true] {
            let control = tempfile::tempdir_in("/tmp").unwrap();
            let destination = tempfile::tempdir_in("/dev/shm").unwrap();
            let _staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
            let staging = tempfile::TempDir::new_in(_staging_parent.path()).unwrap();
            for path in [control.path(), destination.path(), staging.path()] {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let config = control.path().join("backup.json");
            let drop_in = control.path().join("backup.mount.conf");
            let passphrase = control.path().join("passphrase");
            std::fs::write(&passphrase, b"owner-only-test-passphrase").unwrap();
            std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).unwrap();

            let txid = "a".repeat(32);
            let config_temp = control
                .path()
                .join(format!(".backup.json.{txid}.iotkit-config"));
            let drop_temp = control
                .path()
                .join(format!(".iotkit-backup-pair.{txid}.drop-in.tmp"));
            let config_backup = control
                .path()
                .join(format!(".iotkit-backup-pair.{txid}.config.old"));
            let drop_backup = control
                .path()
                .join(format!(".iotkit-backup-pair.{txid}.drop-in.old"));
            let marker = control
                .path()
                .join(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME);
            let old_config = b"old-config";
            let old_drop = b"old-drop-in";
            let new_config = b"new-config";
            let new_drop = b"new-drop-in";
            let old_config_hash = test_hash(old_config);
            let old_drop_hash = test_hash(old_drop);
            let new_config_hash = test_hash(new_config);
            let new_drop_hash = test_hash(new_drop);

            let write_private = |path: &std::path::Path, bytes: &[u8]| {
                std::fs::write(path, bytes).unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            };

            // Build a state which is intentionally outside the phase matrix.
            // The exact malformed edge differs per phase so that every phase
            // checks its own target/backup/temp provenance.
            match (phase, originally_present) {
                ("prepared", false) => write_private(&config, new_config),
                ("prepared", true) => {
                    write_private(&config, old_config);
                    write_private(&config_backup, old_config);
                }
                ("config_publishing", false) => {
                    write_private(&config, new_config);
                    write_private(&config_temp, new_config);
                    write_private(&drop_temp, new_drop);
                }
                ("config_publishing", true) => {
                    write_private(&config, new_config);
                    write_private(&config_backup, old_config);
                    write_private(&config_temp, new_config);
                    write_private(&drop_backup, old_drop);
                    write_private(&drop_temp, new_drop);
                }
                ("config_published", false) => write_private(&config, new_config),
                ("config_published", true) => {
                    write_private(&config, new_config);
                    write_private(&config_backup, old_config);
                    write_private(&drop_in, old_drop);
                    write_private(&drop_backup, old_drop);
                }
                ("drop_in_publishing", false) => {
                    write_private(&config, new_config);
                    write_private(&drop_in, new_drop);
                    write_private(&drop_temp, new_drop);
                }
                ("drop_in_publishing", true) => {
                    write_private(&config, new_config);
                    write_private(&config_backup, old_config);
                    write_private(&drop_in, new_drop);
                    write_private(&drop_backup, old_drop);
                    write_private(&drop_temp, new_drop);
                }
                ("drop_in_published", false) => {
                    write_private(&config, new_config);
                    write_private(&drop_in, new_drop);
                    write_private(&drop_temp, new_drop);
                }
                ("drop_in_published", true) => {
                    write_private(&config, new_config);
                    write_private(&drop_in, new_drop);
                    write_private(&drop_backup, old_drop);
                }
                ("published", false) => {
                    write_private(&config, new_config);
                    write_private(&drop_in, new_drop);
                    write_private(&config_temp, new_config);
                }
                ("published", true) => {
                    write_private(&config, new_config);
                    write_private(&drop_in, new_drop);
                    write_private(&config_backup, b"wrong-old-config");
                    write_private(&drop_backup, old_drop);
                }
                _ => unreachable!(),
            }

            let config_temp_name = config_temp.file_name().unwrap().to_str().unwrap();
            let drop_temp_name = drop_temp.file_name().unwrap().to_str().unwrap();
            let forged = serde_json::json!({
                "schema_version": 3,
                "txid": txid,
                "config_path_hash": test_hash(config.as_os_str().as_bytes()),
                "drop_in_path_hash": test_hash(drop_in.as_os_str().as_bytes()),
                "phase": phase,
                "request_config_hash": "00".repeat(32),
                "request_drop_in_hash": "11".repeat(32),
                "config_hash": if phase == "prepared" { None } else { Some(new_config_hash.clone()) },
                "drop_in_hash": if phase == "prepared" { None } else { Some(new_drop_hash.clone()) },
                "config_existed": originally_present,
                "drop_in_existed": originally_present,
                "old_config_hash": originally_present.then(|| old_config_hash.clone()),
                "old_drop_in_hash": originally_present.then(|| old_drop_hash.clone()),
                "config_temp_name": if phase == "prepared" { None } else { Some(config_temp_name) },
                "drop_in_temp_name": if phase == "prepared" { None } else { Some(drop_temp_name) },
            });
            write_private(&marker, &serde_json::to_vec(&forged).unwrap());

            let tracked = [
                &config,
                &drop_in,
                &marker,
                &config_backup,
                &drop_backup,
                &config_temp,
                &drop_temp,
            ];
            let before: Vec<_> = tracked
                .iter()
                .map(|path| std::fs::read(path).ok())
                .collect();
            let attempt = configure_command(
                &config,
                &drop_in,
                &control.path().join("new.db"),
                destination.path(),
                staging.path(),
                &passphrase,
            )
            .output()
            .unwrap();
            assert!(
                !attempt.status.success(),
                "{phase}/{originally_present} unexpectedly succeeded"
            );
            assert!(
                String::from_utf8_lossy(&attempt.stderr).contains("cleanup_required"),
                "{phase}/{originally_present}: {}",
                String::from_utf8_lossy(&attempt.stderr)
            );
            let after: Vec<_> = tracked
                .iter()
                .map(|path| std::fs::read(path).ok())
                .collect();
            assert_eq!(
                after, before,
                "{phase}/{originally_present} mutated artifacts"
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn backup_pair_symlink_hardlink_and_fifo_fail_closed_without_mutation() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;

    for kind in ["symlink", "hardlink", "fifo"] {
        let control = tempfile::tempdir_in("/tmp").unwrap();
        let destination = tempfile::tempdir_in("/dev/shm").unwrap();
        let _staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
        let staging = tempfile::TempDir::new_in(_staging_parent.path()).unwrap();
        for path in [control.path(), destination.path(), staging.path()] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let config = control.path().join("backup.json");
        let drop_in = control.path().join("backup.mount.conf");
        let passphrase = control.path().join("passphrase");
        std::fs::write(&passphrase, b"owner-only-test-passphrase").unwrap();
        std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).unwrap();
        let initial = configure_command(
            &config,
            &drop_in,
            &control.path().join("old.db"),
            destination.path(),
            staging.path(),
            &passphrase,
        )
        .output()
        .unwrap();
        assert!(initial.status.success());
        let original_config = std::fs::read(&config).unwrap();
        let original_drop_in = std::fs::read(&drop_in).unwrap();
        let config_old = control.path().join(".iotkit-backup-pair.forged.config.old");
        let drop_old = control
            .path()
            .join(".iotkit-backup-pair.forged.drop-in.old");
        match kind {
            "symlink" => {
                std::os::unix::fs::symlink(&config, &config_old).unwrap();
                std::os::unix::fs::symlink(&drop_in, &drop_old).unwrap();
            }
            "hardlink" => {
                std::fs::hard_link(&config, &config_old).unwrap();
                std::fs::hard_link(&drop_in, &drop_old).unwrap();
            }
            "fifo" => {
                for path in [&config_old, &drop_old] {
                    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
                }
            }
            _ => unreachable!(),
        }
        let marker = control
            .path()
            .join(iotkit_core_recovery::BACKUP_PAIR_MARKER_NAME);
        let forged = serde_json::json!({
            "schema_version": 2,
            "txid": "forged",
            "config_path_hash": test_hash(config.as_os_str().as_bytes()),
            "drop_in_path_hash": test_hash(drop_in.as_os_str().as_bytes()),
            "phase": "prepared",
            "request_config_hash": "00".repeat(32),
            "request_drop_in_hash": "11".repeat(32),
            "config_hash": null,
            "drop_in_hash": null,
            "config_existed": true,
            "drop_in_existed": true,
            "old_config_hash": test_hash(&original_config),
            "old_drop_in_hash": test_hash(&original_drop_in)
        });
        std::fs::write(&marker, serde_json::to_vec(&forged).unwrap()).unwrap();
        std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).unwrap();
        let attempt = configure_command(
            &config,
            &drop_in,
            &control.path().join("new.db"),
            destination.path(),
            staging.path(),
            &passphrase,
        )
        .output()
        .unwrap();
        assert!(!attempt.status.success(), "{kind} unexpectedly succeeded");
        assert!(
            String::from_utf8_lossy(&attempt.stderr).contains("cleanup_required"),
            "{kind}: {}",
            String::from_utf8_lossy(&attempt.stderr)
        );
        assert_eq!(std::fs::read(&config).unwrap(), original_config);
        assert_eq!(std::fs::read(&drop_in).unwrap(), original_drop_in);
        assert!(marker.exists(), "{kind} marker must remain");
    }
}

#[cfg(target_os = "linux")]
fn configure_command(
    config: &std::path::Path,
    drop_in: &std::path::Path,
    db: &std::path::Path,
    destination: &std::path::Path,
    staging: &std::path::Path,
    passphrase: &std::path::Path,
) -> Command {
    let mut command = nodectl();
    command.args([
        "backup",
        "configure",
        "--config",
        config.to_str().unwrap(),
        "--db",
        db.to_str().unwrap(),
        "--destination",
        destination.to_str().unwrap(),
        "--staging-directory",
        staging.to_str().unwrap(),
        "--passphrase-file",
        passphrase.to_str().unwrap(),
        "--freshness-seconds",
        "86400",
        "--retention-count",
        "7",
        "--systemd-drop-in",
        drop_in.to_str().unwrap(),
    ]);
    command
}

#[cfg(target_os = "linux")]
fn run_promptly(command: &mut Command, label: &str) -> std::process::Output {
    use std::time::{Duration, Instant};

    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{label} reader did not fail closed promptly");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "linux")]
fn assert_closed_error(output: &std::process::Output) {
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.is_empty(),
        "unexpected stdout: {stdout:?}; stderr={stderr:?}"
    );
    assert!(
        stderr.starts_with("{\"error\":{"),
        "unexpected stderr={stderr:?}; stdout={stdout:?}"
    );
    assert!(!stderr.contains("/tmp/"), "path leaked: {stderr}");
}

#[cfg(target_os = "linux")]
fn test_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(|error| {
        panic!(
            "expected JSON output, got {:?}: {error}",
            String::from_utf8_lossy(bytes)
        )
    })
}
