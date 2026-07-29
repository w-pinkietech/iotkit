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
fn create_inspect_and_status_emit_only_nonsecret_summaries() {
    use std::os::unix::fs::PermissionsExt;

    let control = tempfile::tempdir_in("/tmp").unwrap();
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging = tempfile::tempdir_in("/dev/shm").unwrap();
    for path in [control.path(), destination.path(), staging.path()] {
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
    assert!(
        configure.status.success(),
        "{}",
        String::from_utf8_lossy(&configure.stderr)
    );
    assert_eq!(json(&configure.stdout)["status"], "configured");
    let drop_in_text = std::fs::read_to_string(&drop_in).unwrap();
    assert!(drop_in_text.starts_with("[Unit]\nRequiresMountsFor="));
    assert!(!drop_in_text.contains("cli-secret-passphrase"));
    assert!(!drop_in_text.contains("passphrase"));

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
    let staging = tempfile::tempdir_in("/dev/shm").unwrap();
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

fn json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(|error| {
        panic!(
            "expected JSON output, got {:?}: {error}",
            String::from_utf8_lossy(bytes)
        )
    })
}
