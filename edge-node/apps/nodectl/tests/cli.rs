use std::io::Write;
use std::process::{Command, Output, Stdio};

use rusqlite::params;
use serde_json::Value;
use sha2::Digest;

fn edgectl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_iotkit-edge-nodectl"))
}

#[test]
fn version_exits_without_opening_a_database() {
    let output = edgectl().arg("--version").output().unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("iotkit-edge-nodectl {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn device_add_and_credential_cli_show_plaintext_once_and_drive_make_before_break() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("credential-cli.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    drop(db);
    let add = run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "add",
        "--hardware-id",
        "cli-credential-device",
    ]);
    assert!(String::from_utf8_lossy(&add.stderr).contains("shown once"));
    assert!(String::from_utf8_lossy(&add.stderr).contains("revoke"));
    let first = assert_success(add).trim().to_string();
    assert!(first.starts_with("ikd_"));

    let audit_before_list: i64 = rusqlite::Connection::open(&db_path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM ledger_events", [], |row| row.get(0))
        .unwrap();

    let list = assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device-credential",
        "list",
    ]));
    let audit_after_list: i64 = rusqlite::Connection::open(&db_path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM ledger_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        audit_after_list, audit_before_list,
        "read-only list must not audit or mutate"
    );
    assert!(!list.contains(&first));
    assert!(!list.contains("token_hash"));
    let value: Value = serde_json::from_str(list.trim()).unwrap();
    let principal = value["principals"][0]["principal_id"]
        .as_str()
        .unwrap()
        .to_string();
    let first_id = value["credentials"][0]["credential_id"]
        .as_str()
        .unwrap()
        .to_string();

    let reissue = assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device-credential",
        "reissue",
        "--principal-id",
        &principal,
        "--reason-code",
        "credential_reissue",
    ]));
    let pending = reissue.trim().to_string();
    assert!(pending.starts_with("ikd_"));
    assert_ne!(pending, first);
    let second = run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device-credential",
        "reissue",
        "--principal-id",
        &principal,
        "--reason-code",
        "credential_reissue",
    ]);
    assert!(!second.status.success());
    assert!(!String::from_utf8_lossy(&second.stderr).contains(&pending));

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    assert!(
        iotkit_core_ops::authenticate_device(&conn, &pending)
            .unwrap()
            .is_some()
    );
    let pending_id: String = conn
        .query_row(
            "SELECT credential_id FROM device_credentials WHERE state='pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(conn);
    assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device-credential",
        "confirm",
        "--principal-id",
        &principal,
        "--credential-id",
        &pending_id,
        "--reason-code",
        "credential_confirmed",
    ]));

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    assert!(
        iotkit_core_ops::authenticate_device(&conn, &first)
            .unwrap()
            .is_none()
    );
    assert!(
        iotkit_core_ops::authenticate_device(&conn, &pending)
            .unwrap()
            .is_some()
    );
    let old_state: String = conn
        .query_row(
            "SELECT state FROM device_credentials WHERE credential_id=?1",
            [&first_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_state, "revoked");
    let audit: String = conn
        .query_row(
            "SELECT COALESCE(group_concat(detail, '\n'), '') FROM ledger_events WHERE kind='r14_op'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!audit.contains(&first));
    assert!(!audit.contains(&pending));
    assert!(!audit.contains("token_hash"));
}

#[test]
fn confirmation_review_secret_loss_guidance_matches_returned_credential_state() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("credential-guidance.db");
    drop(iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap());
    let add = run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "add",
        "--hardware-id",
        "guidance-device",
    ]);
    let add_stderr = String::from_utf8_lossy(&add.stderr);
    assert!(add.status.success(), "{add_stderr}");
    assert!(add_stderr.contains("revoke it, then issue a new credential"));
    assert!(!add_stderr.contains("abandon"));

    let list = assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device-credential",
        "list",
    ]));
    let value: Value = serde_json::from_str(list.trim()).unwrap();
    let principal = value["principals"][0]["principal_id"].as_str().unwrap();
    let reissue = run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device-credential",
        "reissue",
        "--principal-id",
        principal,
        "--reason-code",
        "credential_reissue",
    ]);
    let reissue_stderr = String::from_utf8_lossy(&reissue.stderr);
    assert!(reissue.status.success(), "{reissue_stderr}");
    assert!(reissue_stderr.contains("abandon it, then reissue a new credential"));
    assert!(!reissue_stderr.contains("revoke it, then issue"));

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "UPDATE device_capacity SET steady_units=2, burst_units=2",
        [],
    )
    .unwrap();
    let sid = iotkit_core_ledger::insert_device(
        &conn,
        &iotkit_core_ledger::NewDevice {
            hardware_id: "guidance-dormant".into(),
            user_label: None,
            parent: None,
            kind: iotkit_core_ledger::DeviceKind::Individual,
            initial_state: iotkit_core_ledger::DeviceState::Active,
        },
    )
    .unwrap();
    conn.execute(
        "INSERT INTO device_ingest_principals
         (principal_id, device_system_id, flow_class, profile, created_at)
         VALUES ('guidance-dormant-principal', ?1, 'default', 'simple_bearer', 1)",
        [sid.as_bytes().as_slice()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO device_principal_scopes (principal_id, system_id)
         VALUES ('guidance-dormant-principal', ?1)",
        [sid.as_bytes().as_slice()],
    )
    .unwrap();
    drop(conn);
    let issue = run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device-credential",
        "issue",
        "--principal-id",
        "guidance-dormant-principal",
        "--reason-code",
        "manual_issue",
    ]);
    let issue_stderr = String::from_utf8_lossy(&issue.stderr);
    assert!(issue.status.success(), "{issue_stderr}");
    assert!(issue_stderr.contains("revoke it, then issue a new credential"));
    assert!(!issue_stderr.contains("abandon"));
}

#[test]
fn capacity_debt_cli_displays_required_and_available_and_requires_deliberate_automation_confirmation()
 {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("capacity-cli.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE device_flow_classes SET steady_units=5, burst_units=6 WHERE flow_class='high'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    drop(db);
    let args = [
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "add",
        "--hardware-id",
        "capacity-cli-device",
        "--flow-class",
        "high",
        "--accept-capacity-debt",
    ];
    let rejected = run(&args);
    let rejected_stderr = assert_failure(rejected);
    assert!(rejected_stderr.contains("required steady/burst = 5/6"));
    assert!(rejected_stderr.contains("available = 1/1"));
    assert!(rejected_stderr.contains("--yes"));

    let mut accepted_args = args.to_vec();
    accepted_args.push("--yes");
    let accepted = run(&accepted_args);
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    assert!(String::from_utf8_lossy(&accepted.stderr).contains("shown once"));
}

fn run_capacity_race(
    db_path: &std::path::Path,
    ready: &std::path::Path,
    proceed: &std::path::Path,
    args: &[&str],
) -> Output {
    let mut command = edgectl();
    command.args(args);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
        .env("IOTKIT_TEST_CAPACITY_PREVIEW_READY_FILE", ready)
        .env("IOTKIT_TEST_CAPACITY_PREVIEW_CONTINUE_FILE", proceed);
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"approve-capacity-debt\n")
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "capacity preview never completed"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute(
        "UPDATE auth_state SET device_credential_generation=device_credential_generation+1 WHERE id=1",
        [],
    )
    .unwrap();
    std::fs::write(proceed, b"continue").unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn capacity_debt_add_atomic_preview_rejects_equal_total_authority_race() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("capacity-add-race.db");
    let ready = dir.path().join("add.ready");
    let proceed = dir.path().join("add.continue");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE device_flow_classes SET steady_units=5, burst_units=5 WHERE flow_class='high'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    drop(db);
    let output = run_capacity_race(
        &db_path,
        &ready,
        &proceed,
        &[
            "--db",
            db_path.to_str().unwrap(),
            "device",
            "add",
            "--hardware-id",
            "capacity-add-race",
            "--flow-class",
            "high",
            "--accept-capacity-debt",
        ],
    );
    let stderr = assert_failure(output);
    assert!(stderr.contains("capacity_approval_stale"));
    let conn = rusqlite::Connection::open(db_path).unwrap();
    let created: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM devices WHERE hardware_id='capacity-add-race'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(created, 0);
}

#[test]
fn capacity_debt_flow_atomic_preview_rejects_equal_total_authority_race() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("capacity-flow-race.db");
    let ready = dir.path().join("flow.ready");
    let proceed = dir.path().join("flow.continue");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let principal = db
        .with_conn_sync(|conn| {
            let (_credential, _plaintext) = seed_device_credential(conn, "flow-race");
            conn.execute(
                "UPDATE device_flow_classes SET steady_units=5, burst_units=5 WHERE flow_class='high'",
                [],
            )?;
            conn.query_row(
                "SELECT principal_id FROM device_ingest_principals LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .map_err(Into::into)
        })
        .unwrap();
    drop(db);
    let output = run_capacity_race(
        &db_path,
        &ready,
        &proceed,
        &[
            "--db",
            db_path.to_str().unwrap(),
            "device-credential",
            "flow-class",
            "--principal-id",
            &principal,
            "--flow-class",
            "high",
            "--accept-capacity-debt",
            "--yes",
        ],
    );
    let stderr = assert_failure(output);
    assert!(stderr.contains("capacity_approval_stale"));
    let conn = rusqlite::Connection::open(db_path).unwrap();
    let flow: String = conn
        .query_row(
            "SELECT flow_class FROM device_ingest_principals WHERE principal_id=?1",
            [&principal],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(flow, "default");
}

fn all_migrations() -> Vec<iotkit_core_storage::Migration> {
    iotkit_core_ops::all_edge_node_migrations()
}

fn run(args: &[&str]) -> Output {
    edgectl()
        .args(args)
        .output()
        .expect("run iotkit-edge-nodectl")
}

fn run_with_stdin(args: &[&str], stdin: &str) -> Output {
    let mut child = edgectl()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn iotkit-edge-nodectl");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().expect("run iotkit-edge-nodectl")
}

fn run_in_dir_without_db_env(args: &[&str], cwd: &std::path::Path) -> Output {
    edgectl()
        .args(args)
        .current_dir(cwd)
        .env_remove("IOTKIT_DB_PATH")
        .output()
        .expect("run iotkit-edge-nodectl")
}

#[cfg(unix)]
struct PtyChild {
    child: std::process::Child,
    master: Option<std::fs::File>,
    inspect_fd: std::os::fd::RawFd,
    output: Vec<u8>,
}

#[cfg(unix)]
impl PtyChild {
    fn spawn(args: &[&str]) -> Self {
        use std::os::fd::FromRawFd;

        let mut master = -1;
        let mut slave = -1;
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            },
            0
        );
        let inspect_fd = unsafe { libc::dup(slave) };
        let stdin_fd = unsafe { libc::dup(slave) };
        let stdout_fd = unsafe { libc::dup(slave) };
        let stderr_fd = unsafe { libc::dup(slave) };
        assert!(inspect_fd >= 0 && stdin_fd >= 0 && stdout_fd >= 0 && stderr_fd >= 0);
        let child = edgectl()
            .args(args)
            .stdin(unsafe { Stdio::from(std::fs::File::from_raw_fd(stdin_fd)) })
            .stdout(unsafe { Stdio::from(std::fs::File::from_raw_fd(stdout_fd)) })
            .stderr(unsafe { Stdio::from(std::fs::File::from_raw_fd(stderr_fd)) })
            .spawn()
            .unwrap();
        unsafe { libc::close(slave) };
        let flags = unsafe { libc::fcntl(master, libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        Self {
            child,
            master: Some(unsafe { std::fs::File::from_raw_fd(master) }),
            inspect_fd,
            output: Vec::new(),
        }
    }

    fn wait_for(&mut self, needle: &str) {
        use std::io::Read;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let mut buf = [0_u8; 1024];
            match self.master.as_mut().unwrap().read(&mut buf) {
                Ok(0) => {}
                Ok(n) => self.output.extend_from_slice(&buf[..n]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.raw_os_error() == Some(libc::EIO) => {}
                Err(error) => panic!("PTY read: {error}"),
            }
            if String::from_utf8_lossy(&self.output).contains(needle) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!(
            "PTY did not contain {needle:?}: {}",
            String::from_utf8_lossy(&self.output)
        );
    }

    fn write(&mut self, value: &str) {
        self.master
            .as_mut()
            .unwrap()
            .write_all(value.as_bytes())
            .unwrap();
    }

    fn echo_enabled(&self) -> bool {
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        assert_eq!(
            unsafe { libc::tcgetattr(self.inspect_fd, termios.as_mut_ptr()) },
            0
        );
        unsafe { termios.assume_init() }.c_lflag & libc::ECHO != 0
    }

    fn finish(mut self) -> (std::process::ExitStatus, String, bool) {
        use std::io::Read;
        let status = self.child.wait().unwrap();
        let mut buf = [0_u8; 1024];
        if let Some(master) = self.master.as_mut() {
            loop {
                match master.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => self.output.extend_from_slice(&buf[..n]),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                    Err(error) => panic!("PTY read: {error}"),
                }
            }
        }
        let echo = self.echo_enabled();
        (
            status,
            String::from_utf8_lossy(&self.output).into_owned(),
            echo,
        )
    }
}

#[cfg(unix)]
impl Drop for PtyChild {
    fn drop(&mut self) {
        unsafe { libc::close(self.inspect_fd) };
    }
}

fn assert_success(output: Output) -> String {
    assert!(
        output.status.success(),
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn assert_failure(output: Output) -> String {
    assert!(
        !output.status.success(),
        "expected failure\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stderr).unwrap()
}

fn seed_device_credential(conn: &rusqlite::Connection, suffix: &str) -> (String, String) {
    let sid = iotkit_core_ledger::insert_device(
        conn,
        &iotkit_core_ledger::NewDevice {
            hardware_id: format!("credential-test-{suffix}"),
            user_label: None,
            parent: None,
            kind: iotkit_core_ledger::DeviceKind::Individual,
            initial_state: iotkit_core_ledger::DeviceState::Active,
        },
    )
    .unwrap();
    let principal = format!("test-principal-{suffix}");
    conn.execute(
        "INSERT INTO device_ingest_principals
         (principal_id, device_system_id, flow_class, profile, created_at)
         VALUES (?1, ?2, 'default', 'simple_bearer', 1)",
        params![principal, sid.as_bytes().as_slice()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO device_principal_scopes (principal_id, system_id) VALUES (?1, ?2)",
        params![principal, sid.as_bytes().as_slice()],
    )
    .unwrap();
    let credential_id = format!("test-credential-{suffix}");
    let plaintext = format!("ikd_test_{suffix}");
    let hash = sha2::Sha256::digest(plaintext.as_bytes());
    conn.execute(
        "INSERT INTO device_credentials
         (credential_id, principal_id, token_hash, auth_epoch, state, issued_at, issue_reason)
         VALUES (?1, ?2, ?3, (SELECT auth_epoch FROM auth_state WHERE id=1),
                 'current', 1, 'manual_issue')",
        params![credential_id, principal, hash.as_slice()],
    )
    .unwrap();
    (credential_id, plaintext)
}

fn seed_replace_target(conn: &rusqlite::Connection) -> iotkit_core_ledger::SystemId {
    let sid = iotkit_core_ledger::insert_device(
        conn,
        &iotkit_core_ledger::NewDevice {
            hardware_id: "ble:old".into(),
            user_label: Some("Target Sensor".into()),
            parent: None,
            kind: iotkit_core_ledger::DeviceKind::Individual,
            initial_state: iotkit_core_ledger::DeviceState::Active,
        },
    )
    .unwrap();
    iotkit_core_ledger::ensure_series(
        conn,
        &sid,
        "temperature_c",
        iotkit_core_ledger::CHANNEL_NA,
        iotkit_core_ledger::DEFAULT_VARIANT,
        false,
        None,
    )
    .unwrap();
    iotkit_core_ledger::ensure_series(
        conn,
        &sid,
        "voltage_mv",
        0,
        iotkit_core_ledger::DEFAULT_VARIANT,
        false,
        None,
    )
    .unwrap();
    sid
}

fn stage_item(
    conn: &rusqlite::Connection,
    hardware_id: &str,
    measurement_key: &str,
    channel_index: Option<u16>,
) {
    let payload = match channel_index {
        Some(ch) => format!(
            r#"{{"measurement_key":"{measurement_key}","channel_index":{ch},"values":[1.0],"time_source":"edge_node"}}"#
        ),
        None => format!(
            r#"{{"measurement_key":"{measurement_key}","values":[1.0],"time_source":"edge_node"}}"#
        ),
    };
    iotkit_core_timeseries::insert_staged_reading(conn, hardware_id, 1000, &payload).unwrap();
}

fn prepare_replace_db() -> (tempfile::TempDir, std::path::PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let sid = db
        .with_conn_sync(|conn| Ok(seed_replace_target(conn).to_text()))
        .unwrap();
    (dir, db_path, sid)
}

fn seed_archive_target(conn: &rusqlite::Connection) {
    iotkit_core_publish::store::target_insert(
        conn,
        &iotkit_core_publish::store::TargetRow {
            target_id: "archive".into(),
            endpoint_url: "https://archive.example/publish".into(),
            credential_token: "token".into(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: None,
            cursor_pub_seq: 0,
        },
        1_000,
    )
    .unwrap();
}

fn token_id_from_list(output: &str) -> String {
    output
        .split_whitespace()
        .find(|part| part.starts_with("tok_"))
        .expect("token id in list output")
        .to_string()
}

fn token_id_from_labeled_stderr(output: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(output);
    stderr
        .lines()
        .find_map(|line| line.strip_prefix("token_id: "))
        .expect("labeled token id in stderr")
        .to_string()
}

#[test]
fn token_issue_list_and_revoke_do_not_expose_token_material() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let issued = run(&[
        "--db",
        db_arg,
        "token",
        "issue",
        "--name",
        "routine-human",
        "--kind",
        "human",
        "--tier",
        "routine",
    ]);
    assert!(
        issued.status.success(),
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        issued.status.code(),
        String::from_utf8_lossy(&issued.stdout),
        String::from_utf8_lossy(&issued.stderr)
    );
    let plaintext = String::from_utf8(issued.stdout).unwrap().trim().to_string();
    assert!(plaintext.starts_with("iko_"), "stdout:\n{plaintext}");
    let issued_token_id = token_id_from_labeled_stderr(&issued.stderr);

    let listed = assert_success(run(&["--db", db_arg, "token", "list"]));
    let token_id = token_id_from_list(&listed);
    assert_eq!(issued_token_id, token_id);
    assert!(listed.contains("routine-human"), "stdout:\n{listed}");
    assert!(
        !listed.contains(&plaintext),
        "stdout leaked plaintext:\n{listed}"
    );
    assert!(
        !listed.contains("token_hash"),
        "stdout leaked hash header:\n{listed}"
    );
    assert!(
        !listed.contains("$argon2"),
        "stdout leaked hash material:\n{listed}"
    );

    assert_success(run(&["--db", db_arg, "token", "revoke", "--id", &token_id]));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let revoked: bool = conn.query_row(
            "SELECT revoked_at IS NOT NULL FROM operator_tokens WHERE token_id = ?1",
            [&token_id],
            |row| row.get(0),
        )?;
        assert!(revoked);
        Ok(())
    })
    .unwrap();
}

#[test]
fn token_issue_rejects_ai_daily() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let stderr = assert_failure(run(&[
        "--db", db_arg, "token", "issue", "--name", "ai-daily", "--kind", "ai", "--tier", "daily",
    ]));

    assert!(
        stderr.contains("ai token tier ceiling cannot exceed routine")
            || stderr.contains("validation"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn passphrase_reset_validates_strength_and_replaces_existing_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let empty = assert_failure(run_with_stdin(
        &["--db", db_arg, "passphrase", "reset"],
        "\n\n",
    ));
    assert!(empty.contains("at least 8"), "stderr:\n{empty}");

    let short_output = run_with_stdin(&["--db", db_arg, "passphrase", "reset"], "1234567\n");
    assert!(
        !String::from_utf8_lossy(&short_output.stdout).contains("confirm passphrase"),
        "stdout:\n{}",
        String::from_utf8_lossy(&short_output.stdout)
    );
    let short = assert_failure(short_output);
    assert!(short.contains("at least 8"), "stderr:\n{short}");

    let captured = run_with_stdin(
        &["--db", db_arg, "passphrase", "reset"],
        "old-pass\nold-pass\n",
    );
    assert!(captured.status.success());
    assert!(!String::from_utf8_lossy(&captured.stdout).contains("old-pass"));
    assert!(!String::from_utf8_lossy(&captured.stderr).contains("old-pass"));
    assert_success(run_with_stdin(
        &["--db", db_arg, "passphrase", "reset"],
        "new-pass\nnew-pass\n",
    ));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let hash = iotkit_core_ops::load_passphrase_hash(conn)
            .unwrap()
            .unwrap();
        assert!(!iotkit_core_ops::verify_passphrase(&hash, "old-pass"));
        assert!(iotkit_core_ops::verify_passphrase(&hash, "new-pass"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn passphrase_reset_rejects_mismatched_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let stderr = assert_failure(run_with_stdin(
        &["--db", db_arg, "passphrase", "reset"],
        "first-pass\nsecond-pass\n",
    ));

    assert!(
        stderr.contains("passphrases do not match"),
        "stderr:\n{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn passphrase_tty_success_hides_secret_and_restores_echo() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("pty-success.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let mut process = PtyChild::spawn(&["--db", db_path.to_str().unwrap(), "passphrase", "reset"]);
    process.wait_for("new passphrase:");
    process.write("pty-secret-one\n");
    process.wait_for("confirm passphrase:");
    process.write("pty-secret-one\n");
    let (status, output, echo) = process.finish();
    assert!(status.success(), "{output}");
    assert!(
        !output.contains("pty-secret-one"),
        "secret echoed: {output}"
    );
    assert!(echo, "ECHO was not restored after success");
}

#[cfg(unix)]
#[test]
fn passphrase_tty_eof_restores_echo() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("pty-eof.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let mut process = PtyChild::spawn(&["--db", db_path.to_str().unwrap(), "passphrase", "reset"]);
    process.wait_for("new passphrase:");
    process.write("\u{4}");
    let (status, output, echo) = process.finish();
    assert!(!status.success(), "{output}");
    assert!(echo, "ECHO was not restored after EOF/error");
}

#[cfg(unix)]
#[test]
fn passphrase_tty_mismatch_hides_both_secrets_and_restores_echo() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("pty-mismatch.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let mut process = PtyChild::spawn(&["--db", db_path.to_str().unwrap(), "passphrase", "reset"]);
    process.wait_for("new passphrase:");
    process.write("pty-secret-first\n");
    process.wait_for("confirm passphrase:");
    process.write("pty-secret-second\n");
    let (status, output, echo) = process.finish();
    assert!(!status.success(), "{output}");
    assert!(output.contains("passphrases do not match"), "{output}");
    assert!(
        !output.contains("pty-secret-first") && !output.contains("pty-secret-second"),
        "secret echoed: {output}"
    );
    assert!(echo, "ECHO was not restored after mismatch");
}

#[cfg(unix)]
#[test]
fn passphrase_tty_sigint_restores_echo_without_echoing_secret() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("pty-sigint.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let mut process = PtyChild::spawn(&["--db", db_path.to_str().unwrap(), "passphrase", "reset"]);
    process.wait_for("new passphrase:");
    assert!(
        !process.echo_enabled(),
        "test must observe ECHO disabled while waiting"
    );
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(
        unsafe { libc::kill(process.child.id() as libc::pid_t, libc::SIGINT) },
        0
    );
    let (status, output, echo) = process.finish();
    assert!(!status.success(), "{output}");
    assert!(!output.contains("pty-secret"), "secret echoed: {output}");
    assert!(echo, "ECHO was not restored after SIGINT");
}

#[test]
fn time_confirm_is_typed_transactional_and_audited_as_local_cli() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let aborted_output = run_with_stdin(&["--db", db_arg, "time", "confirm"], "no\n");
    let display = String::from_utf8_lossy(&aborted_output.stderr);
    assert!(display.contains("current_time_ms="), "stderr:\n{display}");
    assert!(display.contains("current_time_utc="), "stderr:\n{display}");
    let aborted = assert_failure(aborted_output);
    assert!(aborted.contains("time confirmation aborted"));

    assert_success(run_with_stdin(
        &["--db", db_arg, "time", "confirm"],
        "confirm\n",
    ));
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let detail: String = conn.query_row(
            "SELECT detail FROM ledger_events
             WHERE kind = 'clock_trust_confirmed' ORDER BY event_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        let detail: Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(detail["actor"], "local_cli");
        assert_eq!(detail["source"], "manual_local_root");
        Ok(())
    })
    .unwrap();
}

#[test]
fn fingerprint_reads_cert_next_to_db_and_reports_missing_material() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let missing = run(&["--db", db_arg, "fingerprint"]);
    assert!(
        !missing.status.success(),
        "expected missing fingerprint material to fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&missing.stdout),
        String::from_utf8_lossy(&missing.stderr)
    );
    assert!(
        missing.stdout.is_empty(),
        "stdout should be empty when fingerprint is unavailable:\n{}",
        String::from_utf8_lossy(&missing.stdout)
    );
    let missing_stderr = String::from_utf8(missing.stderr).unwrap();
    assert!(
        missing_stderr.contains("未生成") && missing_stderr.contains("Edge Node 未起動"),
        "stderr:\n{missing_stderr}"
    );

    let tls_dir = dir.path().join("tls");
    std::fs::create_dir(&tls_dir).unwrap();
    let cert_pem = "-----BEGIN CERTIFICATE-----\nAQIDBAU=\n-----END CERTIFICATE-----\n";
    std::fs::write(tls_dir.join("cert.pem"), cert_pem).unwrap();
    let expected = iotkit_core_ops::fingerprint_of_pem(cert_pem).unwrap();

    let out = assert_success(run(&["--db", db_arg, "fingerprint"]));

    assert_eq!(out.trim(), expected);
}

#[test]
fn target_add_is_rejected_while_unowned_and_allowed_after_passphrase_reset() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let setup_stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "target",
        "add",
        "https://archive.example/publish",
        "token",
    ]));
    assert!(
        setup_stderr.contains("setupモード中は出口target登録不可"),
        "stderr:\n{setup_stderr}"
    );

    assert_success(run_with_stdin(
        &["--db", db_arg, "passphrase", "reset"],
        "admin-pass\nadmin-pass\n",
    ));

    let post_reset_stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "target",
        "add",
        "http://archive.example/publish",
        "token",
    ]));
    assert!(
        !post_reset_stderr.contains("setupモード中は出口target登録不可"),
        "stderr:\n{post_reset_stderr}"
    );
}

#[test]
fn health_command_reads_default_health_json_next_to_db_and_marks_stale() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let health_path = dir.path().join("health.json");
    let stale_written_at = 1_i64;
    std::fs::write(
        &health_path,
        format!(
            r#"{{
                "schema":1,
                "written_at":{stale_written_at},
                "epoch":"epoch-1",
                "uptime_s":12,
                "collector_alive":true,
                "adapters":[],
                "publish":[{{"target_id":"archive","cursor_pub_seq":7,"backlog":3,"last_push_at":1234,"last_error":null}}],
                "db":{{"size_bytes":10,"disk_available_bytes":20,"watermark_exceeded":false}},
                "retention":{{"days":90,"last_purge_at":null,"last_purged_rows":0}}
            }}"#
        ),
    )
    .unwrap();

    let out = assert_success(run(&["--db", db_path.to_str().unwrap(), "health"]));

    assert!(out.contains("STALE (daemon down?)"), "stdout:\n{out}");
    assert!(out.contains("epoch-1"), "stdout:\n{out}");
    assert!(out.contains("collector_alive=true"), "stdout:\n{out}");
    assert!(
        out.contains("publish target=archive cursor=7 backlog=3 last_push_at=1234 last_error=-"),
        "stdout:\n{out}"
    );
}

#[test]
fn missing_db_path_is_error_and_does_not_create_empty_db() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("missing.db");

    let output = run(&["--db", db_path.to_str().unwrap(), "device", "list"]);

    assert!(!output.status.success());
    assert!(!db_path.exists());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("database file does not exist"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn init_creates_fresh_database_and_reports_identity_as_json() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("edge.db");

    let stdout = assert_success(run(&["--db", db_path.to_str().unwrap(), "init"]));

    let reported: Value = serde_json::from_str(&stdout).unwrap();
    let edge_node_id = reported["edge_node_id"].as_str().unwrap();
    let ledger_epoch = reported["ledger_epoch"].as_str().unwrap();
    assert!(!edge_node_id.is_empty());
    assert!(!ledger_epoch.is_empty());

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let stored_edge_node_id: String = conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'edge_node_id'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(edge_node_id, stored_edge_node_id);
    assert_eq!(
        ledger_epoch,
        iotkit_core_ledger::ledger_epoch(&conn).unwrap()
    );
}

#[test]
fn init_refuses_existing_database_without_changing_it() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("edge.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| Ok(iotkit_core_ledger::edge_node_id(conn).unwrap()))
        .unwrap();
    drop(db);
    let bytes_before = std::fs::read(&db_path).unwrap();

    let stderr = assert_failure(run(&["--db", db_path.to_str().unwrap(), "init"]));

    assert!(
        stderr.contains("init requires an absent database"),
        "{stderr}"
    );
    assert_eq!(std::fs::read(&db_path).unwrap(), bytes_before);
}

#[test]
fn identity_reports_initialized_values_without_changing_database() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("edge.db");
    let initialized: Value = serde_json::from_str(&assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "init",
    ])))
    .unwrap();
    let bytes_before = std::fs::read(&db_path).unwrap();

    let reported: Value = serde_json::from_str(&assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "identity",
    ])))
    .unwrap();

    assert_eq!(reported, initialized);
    assert_eq!(std::fs::read(&db_path).unwrap(), bytes_before);
}

#[test]
fn mqtt_binding_reports_only_non_secret_d9_connection_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("edge.db");
    let initialized: Value = serde_json::from_str(&assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "init",
    ])))
    .unwrap();
    let edge_node_id = initialized["edge_node_id"].as_str().unwrap();
    let bytes_before = std::fs::read(&db_path).unwrap();

    let reported: Value = serde_json::from_str(&assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "mqtt-binding",
    ])))
    .unwrap();

    assert_eq!(reported.as_object().unwrap().len(), 17);
    assert_eq!(reported["edge_node_id"], edge_node_id);
    assert_eq!(reported["username"], edge_node_id);
    assert_eq!(
        reported["client_id"],
        format!("iotkit-edge-node-{edge_node_id}")
    );
    assert_eq!(
        reported["records_topic"],
        format!("iotkit/v1/edge-nodes/{edge_node_id}/records")
    );
    assert_eq!(
        reported["status_topic"],
        format!("iotkit/v1/edge-nodes/{edge_node_id}/status")
    );
    assert_eq!(
        reported["accepted_through_topic"],
        format!("iotkit/v1/edge-nodes/{edge_node_id}/accepted-through")
    );
    assert_eq!(
        reported["descriptor_topic"],
        format!("iotkit/v1/edge-nodes/{edge_node_id}/descriptors")
    );
    assert_eq!(
        reported["activation_request_topic"],
        format!("iotkit/v1/edge-nodes/{edge_node_id}/activation/request")
    );
    assert_eq!(
        reported["activation_result_topic"],
        format!("iotkit/v1/edge-nodes/{edge_node_id}/activation/result")
    );
    assert_eq!(
        reported["recovery_request_topic"],
        format!("iotkit/v1/edge-nodes/{edge_node_id}/recovery/request")
    );
    assert_eq!(
        reported["recovery_result_topic"],
        format!("iotkit/v1/edge-nodes/{edge_node_id}/recovery/result")
    );
    assert_eq!(
        reported["recovery_completion_topic"],
        format!("iotkit/v1/edge-nodes/{edge_node_id}/recovery/completion")
    );
    assert_eq!(
        reported["recovery_completion_ack_topic"],
        format!("iotkit/v1/edge-nodes/{edge_node_id}/recovery/completion-ack")
    );
    assert_eq!(reported["qos"], 1);
    assert_eq!(reported["retain"], false);
    assert_eq!(reported["descriptor_retain"], true);
    assert_eq!(reported["status_retain"], true);
    assert_eq!(std::fs::read(&db_path).unwrap(), bytes_before);
}

#[test]
fn commissioning_smoke_enqueue_and_status_use_public_cli_state() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("edge.db");
    let initialized: Value = serde_json::from_str(&assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "init",
    ])))
    .unwrap();
    let epoch = initialized["ledger_epoch"].as_str().unwrap();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        iotkit_core_publish::store::target_insert(
            conn,
            &iotkit_core_publish::store::TargetRow {
                target_id: "edge".into(),
                endpoint_url: "mqtt://broker:1883".into(),
                credential_token: String::new(),
                archive_responsible: true,
                schema_version: 1,
                cursor_epoch: None,
                cursor_pub_seq: 0,
            },
            1,
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    drop(db);

    let enqueued: Value = serde_json::from_str(&assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "smoke",
        "enqueue",
    ])))
    .unwrap();
    assert_eq!(enqueued["target_id"], "edge");
    assert_eq!(enqueued["ledger_epoch"], epoch);
    let pub_seq = enqueued["pub_seq"].as_i64().unwrap();
    let pub_seq_arg = pub_seq.to_string();
    let test_id = enqueued["test_id"].as_str().unwrap();
    assert!(test_id.starts_with("smoke-"));

    let pending: Value = serde_json::from_str(&assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "smoke",
        "status",
        "--ledger-epoch",
        epoch,
        "--pub-seq",
        &pub_seq_arg,
    ])))
    .unwrap();
    assert_eq!(pending["status"], "pending");
    assert_eq!(pending["accepted_through"], 0);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    iotkit_core_publish::store::target_advance_cursor(&conn, "edge", epoch, pub_seq).unwrap();
    drop(conn);
    let bytes_before_status = std::fs::read(&db_path).unwrap();

    let delivered: Value = serde_json::from_str(&assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "smoke",
        "status",
        "--ledger-epoch",
        epoch,
        "--pub-seq",
        &pub_seq_arg,
    ])))
    .unwrap();
    assert_eq!(delivered["status"], "delivered");
    assert_eq!(delivered["accepted_through"], pub_seq);
    assert_eq!(std::fs::read(&db_path).unwrap(), bytes_before_status);
}

#[test]
fn commissioning_smoke_enqueue_requires_the_runtime_mqtt_target() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("edge.db");
    assert_success(run(&["--db", db_path.to_str().unwrap(), "init"]));

    let stderr = assert_failure(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "smoke",
        "enqueue",
    ]));

    assert!(
        stderr.contains("start Edge Node with MQTT exit enabled first"),
        "{stderr}"
    );
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM publication_log WHERE kind='commissioning_smoke'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn read_only_identity_commands_refuse_missing_or_uninitialized_database_without_creation() {
    for command in ["identity", "mqtt-binding"] {
        let dir = tempfile::tempdir().unwrap();
        let missing_path = dir.path().join("missing.db");
        let missing = assert_failure(run(&["--db", missing_path.to_str().unwrap(), command]));
        assert!(
            missing.contains("database file does not exist"),
            "{missing}"
        );
        assert!(!missing_path.exists());

        let empty_path = dir.path().join("empty.db");
        std::fs::File::create(&empty_path).unwrap();
        let bytes_before = std::fs::read(&empty_path).unwrap();
        let uninitialized = assert_failure(run(&["--db", empty_path.to_str().unwrap(), command]));
        assert!(
            uninitialized.contains("Edge Node identity is not initialized"),
            "{uninitialized}"
        );
        assert_eq!(std::fs::read(&empty_path).unwrap(), bytes_before);
    }
}

#[test]
fn read_only_identity_commands_reject_legacy_database_without_mutation() {
    for command in ["identity", "mqtt-binding"] {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("gateway.db");
        create_pre_cutover_cli_database(&db_path, Some("gateway_identity"));
        let bytes_before = std::fs::read(&db_path).unwrap();

        let stderr = assert_failure(run(&["--db", db_path.to_str().unwrap(), command]));

        assert!(stderr.contains("unsupported pre-release"), "{stderr}");
        assert_eq!(std::fs::read(&db_path).unwrap(), bytes_before);
    }
}

#[test]
fn mutate_command_without_db_argument_or_env_is_error() {
    let dir = tempfile::tempdir().unwrap();
    let fallback_path = dir.path().join("iotkit.db");

    let output =
        run_in_dir_without_db_env(&["device", "add", "--hardware-id", "ble:no-db"], dir.path());

    assert!(!output.status.success());
    assert!(!fallback_path.exists());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no database specified"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn existing_empty_db_gets_edge_migration_version_set() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    std::fs::File::create(&db_path).unwrap();

    assert_success(run(&["--db", db_path.to_str().unwrap(), "device", "list"]));

    let conn = rusqlite::Connection::open(db_path).unwrap();
    let versions: Vec<u32> = conn
        .prepare("SELECT version FROM _schema_version ORDER BY version")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        versions,
        vec![
            1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 25,
        ]
    );
    let edge_node_id: String = conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'edge_node_id'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!edge_node_id.is_empty());
}

#[test]
fn fresh_database_gets_edge_identity_before_command_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    std::fs::File::create(&db_path).unwrap();

    assert_failure(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "activate",
        "not-a-system-id",
    ]));

    let conn = rusqlite::Connection::open(db_path).unwrap();
    let edge_node_id: String = conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'edge_node_id'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!edge_node_id.is_empty());
}

fn create_pre_cutover_cli_database(db_path: &std::path::Path, identity_key: Option<&str>) {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE _schema_version (
            version INTEGER NOT NULL PRIMARY KEY,
            label TEXT NOT NULL,
            applied_at INTEGER NOT NULL
        );
        INSERT INTO _schema_version VALUES (1, 'init', 0);
        CREATE TABLE ledger_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .unwrap();
    if let Some(identity_key) = identity_key {
        conn.execute(
            "INSERT INTO ledger_meta (key, value) VALUES (?1, 'legacy-edge')",
            [identity_key],
        )
        .unwrap();
    }
}

#[test]
fn readonly_cli_rejects_gateway_database_without_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("gateway.db");
    create_pre_cutover_cli_database(&db_path, Some("gateway_identity"));
    let bytes_before = std::fs::read(&db_path).unwrap();

    let stderr = assert_failure(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device-credential",
        "list",
    ]));

    assert!(
        stderr.contains(
            "unsupported pre-release Edge Node database; recreate the Edge Node database"
        ),
        "stderr:\n{stderr}"
    );
    assert_eq!(std::fs::read(db_path).unwrap(), bytes_before);
}

#[test]
fn writable_cli_rejects_unmarked_pre_cutover_database_without_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("pre-cutover.db");
    create_pre_cutover_cli_database(&db_path, None);
    let bytes_before = std::fs::read(&db_path).unwrap();

    let stderr = assert_failure(run(&["--db", db_path.to_str().unwrap(), "device", "list"]));

    assert!(
        stderr.contains(
            "unsupported pre-release Edge Node database; recreate the Edge Node database"
        ),
        "stderr:\n{stderr}"
    );
    assert_eq!(std::fs::read(db_path).unwrap(), bytes_before);
}

#[test]
fn device_lifecycle_commands_round_trip_and_bump_generation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        iotkit_core_ledger::record_sighting(conn, "ble:cli", "cli-test").unwrap();
        Ok(())
    })
    .unwrap();
    let db_arg = db_path.to_str().unwrap();

    let sid = assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "approve",
        "ble:cli",
        "--label",
        "CLI Sensor",
        "--kind",
        "individual",
    ]))
    .trim()
    .to_string();
    assert!(!sid.is_empty());

    assert_success(run(&["--db", db_arg, "device", "activate", &sid]));

    let listed = assert_success(run(&["--db", db_arg, "device", "list"]));
    assert!(listed.contains("ble:cli"));
    assert!(listed.contains("active"));

    assert_success(run(&["--db", db_arg, "device", "retire", &sid, "--yes"]));

    let live = assert_success(run(&["--db", db_arg, "device", "list"]));
    assert!(!live.contains("ble:cli"));
    let all = assert_success(run(&["--db", db_arg, "device", "list", "--all"]));
    assert!(all.contains("ble:cli"));
    assert!(all.contains("retired"));

    db.with_conn_sync(|conn| {
        assert_eq!(iotkit_core_ledger::current_generation(conn).unwrap(), 3);
        Ok(())
    })
    .unwrap();
}

#[test]
fn registry_and_series_commands_round_trip_and_bump_generation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let sid = db
        .with_conn_sync(|conn| {
            let sid = iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:series".into(),
                    user_label: None,
                    parent: None,
                    kind: iotkit_core_ledger::DeviceKind::Individual,
                    initial_state: iotkit_core_ledger::DeviceState::Active,
                },
            )
            .unwrap();
            iotkit_core_ledger::ensure_series(
                conn,
                &sid,
                "distance_mm",
                iotkit_core_ledger::CHANNEL_NA,
                iotkit_core_ledger::DEFAULT_VARIANT,
                false,
                None,
            )
            .unwrap();
            Ok(sid.to_text())
        })
        .unwrap();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&["--db", db_arg, "registry", "enable", "distance_mm"]));
    let entries = assert_success(run(&["--db", db_arg, "registry", "list"]));
    assert!(entries.contains("distance_mm"));
    assert!(entries.contains("single"));

    assert_success(run(&[
        "--db",
        db_arg,
        "registry",
        "alias",
        "range_mm",
        "distance_mm",
    ]));
    let aliases = assert_success(run(&["--db", db_arg, "registry", "list", "--aliases"]));
    assert!(aliases.contains("range_mm"));
    assert!(aliases.contains("distance_mm"));

    let series = assert_success(run(&["--db", db_arg, "series", "list", &sid]));
    assert!(series.contains("distance_mm"));
    assert!(series.contains("-1"));

    db.with_conn_sync(|conn| {
        assert_eq!(iotkit_core_ledger::current_generation(conn).unwrap(), 2);
        Ok(())
    })
    .unwrap();
}

#[test]
fn release_rejected_while_archive_target_registered_without_override() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let catalog = iotkit_core_registry::standard_catalog();
        let entry = catalog.find("temperature_c").unwrap();
        iotkit_core_registry::enable_entry(conn, entry, &catalog.catalog_version, "test").unwrap();
        let sid = iotkit_core_ledger::insert_device(
            conn,
            &iotkit_core_ledger::NewDevice {
                hardware_id: "ble:quarantined-alias".into(),
                user_label: None,
                parent: None,
                kind: iotkit_core_ledger::DeviceKind::Individual,
                initial_state: iotkit_core_ledger::DeviceState::Active,
            },
        )
        .unwrap();
        iotkit_core_ledger::ensure_series(
            conn,
            &sid,
            "temp_alias",
            iotkit_core_ledger::CHANNEL_NA,
            iotkit_core_ledger::DEFAULT_VARIANT,
            true,
            Some("unknown_key"),
        )
        .unwrap();
        seed_archive_target(conn);
        Ok(())
    })
    .unwrap();
    let db_arg = db_path.to_str().unwrap();

    let stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "registry",
        "alias",
        "temp_alias",
        "temperature_c",
    ]));
    assert!(
        stderr.contains("refused") && stderr.contains("--release-abandon-past"),
        "stderr did not explain archive custody refusal:\n{stderr}"
    );
    db.with_conn_sync(|conn| {
        let quarantined: i64 = conn
            .query_row(
                "SELECT quarantined FROM series WHERE measurement_key = 'temp_alias'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let aliases: i64 = conn
            .query_row("SELECT COUNT(*) FROM registry_aliases", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(quarantined, 1);
        assert_eq!(aliases, 0);
        Ok(())
    })
    .unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "registry",
        "alias",
        "temp_alias",
        "temperature_c",
        "--release-abandon-past",
    ]));
    db.with_conn_sync(|conn| {
        let quarantined: i64 = conn
            .query_row(
                "SELECT quarantined FROM series WHERE measurement_key = 'temp_alias'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events
                 WHERE kind = 'quarantine_release_abandon_past'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(quarantined, 0);
        assert_eq!(event_count, 1);
        Ok(())
    })
    .unwrap();

    let no_archive_dir = tempfile::tempdir().unwrap();
    let no_archive_path = no_archive_dir.path().join("iotkit.db");
    let no_archive_db = iotkit_core_storage::init_db(&no_archive_path, &all_migrations()).unwrap();
    no_archive_db
        .with_conn_sync(|conn| {
            let catalog = iotkit_core_registry::standard_catalog();
            let entry = catalog.find("temperature_c").unwrap();
            iotkit_core_registry::enable_entry(conn, entry, &catalog.catalog_version, "test")
                .unwrap();
            let sid = iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:no-archive-alias".into(),
                    user_label: None,
                    parent: None,
                    kind: iotkit_core_ledger::DeviceKind::Individual,
                    initial_state: iotkit_core_ledger::DeviceState::Active,
                },
            )
            .unwrap();
            iotkit_core_ledger::ensure_series(
                conn,
                &sid,
                "temp_alias",
                iotkit_core_ledger::CHANNEL_NA,
                iotkit_core_ledger::DEFAULT_VARIANT,
                true,
                Some("unknown_key"),
            )
            .unwrap();
            Ok(())
        })
        .unwrap();

    assert_success(run(&[
        "--db",
        no_archive_path.to_str().unwrap(),
        "registry",
        "alias",
        "temp_alias",
        "temperature_c",
    ]));
    no_archive_db
        .with_conn_sync(|conn| {
            let quarantined: i64 = conn
                .query_row(
                    "SELECT quarantined FROM series WHERE measurement_key = 'temp_alias'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(quarantined, 0);
            Ok(())
        })
        .unwrap();
}

#[test]
fn replace_hardware_allows_exact_observed_profile_from_staged_readings() {
    let (_dir, db_path, sid) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        stage_item(conn, "ble:new", "temperature_c", None);
        stage_item(conn, "ble:new", "voltage_mv", Some(0));
        Ok(())
    })
    .unwrap();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:new",
        "--yes",
    ]));

    db.with_conn_sync(|conn| {
        let row = iotkit_core_ledger::get_device(
            conn,
            &iotkit_core_ledger::SystemId::from_text(&sid).unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(row.hardware_id, "ble:new");
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_hardware_normalizes_single_channel_zero_in_observed_profile() {
    let (_dir, db_path, sid) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let catalog = iotkit_core_registry::standard_catalog();
        let temperature = catalog.find("temperature_c").unwrap();
        iotkit_core_registry::enable_entry(conn, temperature, &catalog.catalog_version, "test")
            .unwrap();
        stage_item(conn, "ble:new", "temperature_c", Some(0));
        stage_item(conn, "ble:new", "voltage_mv", Some(0));
        Ok(())
    })
    .unwrap();

    assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:new",
        "--yes",
    ]));
}

#[test]
fn replace_hardware_normalizes_unenabled_standard_single_channel_zero_in_observed_profile() {
    let (_dir, db_path, sid) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        stage_item(conn, "ble:new", "temperature_c", Some(0));
        stage_item(conn, "ble:new", "voltage_mv", Some(0));
        Ok(())
    })
    .unwrap();

    assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:new",
        "--yes",
    ]));
}

#[test]
fn replace_hardware_rejects_same_hardware_id() {
    let (_dir, db_path, sid) = prepare_replace_db();

    let stderr = assert_failure(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:old",
        "--force",
        "--yes",
    ]));
    assert!(
        stderr.contains("invalid replace") || stderr.contains("same hardware_id"),
        "stderr did not explain same-hardware replace:\n{stderr}"
    );

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind = 'hardware_replaced'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_hardware_rejects_retired_target() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
        iotkit_core_ledger::retire_device(conn, &sid).unwrap();
        Ok(())
    })
    .unwrap();

    let stderr = assert_failure(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));
    assert!(
        stderr.contains("non-retired device"),
        "stderr did not explain retired target:\n{stderr}"
    );
}

#[test]
fn replace_hardware_blocks_missing_extra_and_empty_observed_profiles_unless_forced() {
    for (case, staged) in [
        ("missing", vec![("temperature_c", None)]),
        (
            "extra",
            vec![
                ("temperature_c", None),
                ("voltage_mv", Some(0)),
                ("humidity_pct", None),
            ],
        ),
        ("empty", vec![]),
    ] {
        let (_dir, db_path, sid) = prepare_replace_db();
        let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            for (measurement_key, channel_index) in staged {
                stage_item(conn, "ble:new", measurement_key, channel_index);
            }
            Ok(())
        })
        .unwrap();

        let stderr = assert_failure(run(&[
            "--db",
            db_path.to_str().unwrap(),
            "device",
            "replace",
            &sid,
            "--new-hardware-id",
            "ble:new",
            "--yes",
        ]));
        assert!(
            stderr.contains("observed profile"),
            "{case} stderr did not explain profile mismatch:\n{stderr}"
        );
    }

    let (_dir, db_path, sid) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();
    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));
}

#[test]
fn replace_hardware_uses_alive_candidate_series_as_observed_profile_and_retires_candidate() {
    let (_dir, db_path, sid) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let candidate = db
        .with_conn_sync(|conn| {
            let candidate = iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:new".into(),
                    user_label: None,
                    parent: None,
                    kind: iotkit_core_ledger::DeviceKind::Individual,
                    initial_state: iotkit_core_ledger::DeviceState::Quarantined,
                },
            )
            .unwrap();
            iotkit_core_ledger::ensure_series(
                conn,
                &candidate,
                "temperature_c",
                iotkit_core_ledger::CHANNEL_NA,
                iotkit_core_ledger::DEFAULT_VARIANT,
                false,
                None,
            )
            .unwrap();
            iotkit_core_ledger::ensure_series(
                conn,
                &candidate,
                "voltage_mv",
                0,
                iotkit_core_ledger::DEFAULT_VARIANT,
                false,
                None,
            )
            .unwrap();
            Ok(candidate)
        })
        .unwrap();

    assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:new",
        "--yes",
    ]));

    db.with_conn_sync(|conn| {
        let candidate_row = iotkit_core_ledger::get_device(conn, &candidate)
            .unwrap()
            .unwrap();
        assert_eq!(
            candidate_row.state,
            iotkit_core_ledger::DeviceState::Retired
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_undo_restores_hardware_id_marks_since_range_and_records_event() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        stage_item(conn, "ble:new", "temperature_c", None);
        stage_item(conn, "ble:new", "voltage_mv", Some(0));
        Ok(())
    })
    .unwrap();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--yes",
    ]));

    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
        let series = iotkit_core_ledger::list_series_for_device(conn, &sid).unwrap();
        let first = series[0].series_id;
        let second = series[1].series_id;
        conn.execute(
            "UPDATE ledger_events
             SET at = 1000, detail = '{\"old_hw\":\"ble:old\",\"new_hw\":\"ble:new\",\"at\":1000}'
             WHERE kind = 'hardware_replaced'",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO readings
                (seq, series_id, received_at, device_time, time_source, time_quality,
                 event_time, event_time_source, values_json, rssi, battery_pct, quarantined)
             VALUES
                (1, ?1, 500, 500, 'edge', 'unsynced', 500, 'received_at', '[1.0]', NULL, NULL, 0),
                (2, ?1, 1500, 100, 'device_ntp', 'unsynced', 100, 'device', '[2.0]', NULL, NULL, 0),
                (3, ?2, 1600, 200, 'device_ntp', 'unsynced', 200, 'device', '[3.0]', NULL, NULL, 0)",
            params![first, second],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
    ]));

    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
        let row = iotkit_core_ledger::get_device(conn, &sid).unwrap().unwrap();
        assert_eq!(row.hardware_id, "ble:old");
        let rows: Vec<(i64, i64)> = conn
            .prepare("SELECT seq, quarantined FROM readings ORDER BY seq")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![(1, 0), (2, 1), (3, 1)]);
        let (kind, detail): (String, String) = conn
            .query_row(
                "SELECT kind, detail FROM ledger_events ORDER BY event_id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "hardware_replace_undone");
        assert!(detail.contains("\"rows\":2"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_undo_rejected_while_archive_target_registered_without_abandon() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        seed_archive_target(conn);
        Ok(())
    })
    .unwrap();

    let stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
    ]));
    assert!(
        stderr.contains("refused") && stderr.contains("--abandon-custody"),
        "stderr did not explain archive custody refusal:\n{stderr}"
    );
    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
        let row = iotkit_core_ledger::get_device(conn, &sid).unwrap().unwrap();
        assert_eq!(row.hardware_id, "ble:new");
        let undone: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind = 'hardware_replace_undone'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(undone, 0);
        Ok(())
    })
    .unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
        "--abandon-custody",
    ]));
    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
        let row = iotkit_core_ledger::get_device(conn, &sid).unwrap().unwrap();
        assert_eq!(row.hardware_id, "ble:old");
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_undo_prunes_outbox_for_retroactively_quarantined_rows_same_tx() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let (before_seq, in_range_seq, second_in_range_seq, outbox_annotation_seq) = db
        .with_conn_sync(|conn| {
            let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
            let series = iotkit_core_ledger::list_series_for_device(conn, &sid).unwrap();
            let first_series = series[0].series_id;
            let second_series = series[1].series_id;
            conn.execute(
                "UPDATE ledger_events
                 SET at = 1000, detail = '{\"old_hw\":\"ble:old\",\"new_hw\":\"ble:new\",\"at\":1000}'
                 WHERE kind = 'hardware_replaced'",
                [],
            )
            .unwrap();
            let before_seq = iotkit_core_timeseries::insert_reading_v3(
                conn,
                &iotkit_core_timeseries::NewReading {
                    series_id: first_series,
                    received_at_ms: 500,
                    device_time_ms: None,
                    time_source: "edge_node".into(),
                    values: vec![1.0],
                    rssi: None,
                    battery_pct: None,
                    quarantined: false,
                },
            )
            .unwrap();
            let in_range_seq = iotkit_core_timeseries::insert_reading_v3(
                conn,
                &iotkit_core_timeseries::NewReading {
                    series_id: first_series,
                    received_at_ms: 1_500,
                    device_time_ms: None,
                    time_source: "edge_node".into(),
                    values: vec![2.0],
                    rssi: None,
                    battery_pct: None,
                    quarantined: false,
                },
            )
            .unwrap();
            let second_in_range_seq = iotkit_core_timeseries::insert_reading_v3(
                conn,
                &iotkit_core_timeseries::NewReading {
                    series_id: second_series,
                    received_at_ms: 1_600,
                    device_time_ms: None,
                    time_source: "edge_node".into(),
                    values: vec![3.0],
                    rssi: None,
                    battery_pct: None,
                    quarantined: false,
                },
            )
            .unwrap();
            let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
            iotkit_core_publish::store::enqueue_measurement(conn, &epoch, before_seq, 501)
                .unwrap();
            iotkit_core_publish::store::enqueue_measurement(conn, &epoch, in_range_seq, 1_501)
                .unwrap();
            iotkit_core_publish::store::enqueue_measurement(
                conn,
                &epoch,
                second_in_range_seq,
                1_601,
            )
                .unwrap();
            let outbox_annotation_seq =
                iotkit_core_publish::store::enqueue_annotation(conn, &epoch, "test", "{}", 1_700)
                    .unwrap()
                    .unwrap();
            Ok((
                before_seq,
                in_range_seq,
                second_in_range_seq,
                outbox_annotation_seq,
            ))
        })
        .unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
    ]));

    db.with_conn_sync(|conn| {
        let rows: Vec<(i64, i64)> = conn
            .prepare("SELECT seq, quarantined FROM readings ORDER BY seq")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![(before_seq, 0), (in_range_seq, 1), (second_in_range_seq, 1)]
        );
        let outbox: Vec<(String, Option<i64>)> = conn
            .prepare("SELECT kind, reading_seq FROM publication_log ORDER BY pub_seq")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            outbox,
            vec![
                ("measurement".to_string(), Some(before_seq)),
                ("annotation".to_string(), None)
            ]
        );
        let annotation_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM publication_log WHERE pub_seq = ?1",
                params![outbox_annotation_seq],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(annotation_exists, 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_undo_rejects_old_hardware_id_mismatch() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:wrong",
    ]));
    assert!(
        stderr.contains("old_hw") || stderr.contains("old hardware"),
        "stderr did not explain old_hw mismatch:\n{stderr}"
    );

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
        let row = iotkit_core_ledger::get_device(conn, &sid).unwrap().unwrap();
        assert_eq!(row.hardware_id, "ble:new");
        let undone: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind = 'hardware_replace_undone'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(undone, 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_undo_rejects_old_hardware_id_used_by_other_alive_device() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        iotkit_core_ledger::insert_device(
            conn,
            &iotkit_core_ledger::NewDevice {
                hardware_id: "ble:old".into(),
                user_label: Some("Other".into()),
                parent: None,
                kind: iotkit_core_ledger::DeviceKind::Individual,
                initial_state: iotkit_core_ledger::DeviceState::Active,
            },
        )
        .unwrap();
        Ok(())
    })
    .unwrap();

    let stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
    ]));
    assert!(
        stderr.contains("hardware_id already in use"),
        "stderr did not explain hardware conflict:\n{stderr}"
    );
}

#[test]
fn replace_undo_without_since_requires_matching_replace_event() {
    let (_dir, db_path, sid_text) = prepare_replace_db();

    let stderr = assert_failure(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:previous",
    ]));
    assert!(
        stderr.contains("no hardware_replaced event"),
        "stderr did not explain missing replace event:\n{stderr}"
    );
}

#[test]
fn replace_undo_rejects_future_since_even_when_event_matches() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
        "--since",
        "9223372036854775807",
    ]));
    assert!(
        stderr.contains("--since") && stderr.contains("future"),
        "stderr did not explain future --since:\n{stderr}"
    );
}

#[test]
fn replace_undo_rejects_since_after_replace_event() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE ledger_events
             SET at = 1000, detail = '{\"old_hw\":\"ble:old\",\"new_hw\":\"ble:new\",\"at\":1000}'
             WHERE kind = 'hardware_replaced'",
            [],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();

    let stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
        "--since",
        "1500",
    ]));
    assert!(
        stderr.contains("--since") && stderr.contains("replace event"),
        "stderr did not explain --since lower bound:\n{stderr}"
    );
}

#[test]
fn replace_undo_allows_since_before_replace_event() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE ledger_events
             SET at = 1000, detail = '{\"old_hw\":\"ble:old\",\"new_hw\":\"ble:new\",\"at\":1000}'
             WHERE kind = 'hardware_replaced'",
            [],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
        "--since",
        "500",
    ]));
}

#[test]
fn pipeline_cli_lists_exports_imports_and_resets_through_typed_operations() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("pipelines.db");
    assert_success(run(&["--db", db_path.to_str().unwrap(), "init"]));

    // Before the node ever started with this database, mutations are refused.
    let refused = run(&[
        "--db",
        db_path.to_str().unwrap(),
        "pipeline",
        "reset",
        "press-01",
    ]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("edge-node-id"));

    let engine = iotkit_core_pipeline::PipelineEngine::new("rpi1".parse().unwrap());
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        engine
            .reconcile(conn, iotkit_core_pipeline::InputTime::now(None))
            .unwrap();
        Ok(())
    })
    .unwrap();
    drop(db);

    let listed = assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "pipeline",
        "list",
    ]));
    assert_eq!(
        serde_json::from_str::<Value>(&listed).unwrap(),
        Value::Array(vec![])
    );

    let import_file = dir.path().join("import.toml");
    std::fs::write(
        &import_file,
        r#"
[[pipeline]]
id = "press-01-cycle-count"
kind = "accumulated-count"
trigger = "on-transition"

[pipeline.input]
adapter = "trial_sample"
measurement_key = "contact_state"

[pipeline.detector]
mode = "high-active"
rise_threshold = 0.5
fall_threshold = 0.5
"#,
    )
    .unwrap();

    let unconfirmed = run(&[
        "--db",
        db_path.to_str().unwrap(),
        "pipeline",
        "import",
        import_file.to_str().unwrap(),
    ]);
    assert!(!unconfirmed.status.success());
    assert!(String::from_utf8_lossy(&unconfirmed.stderr).contains("--replace-all"));

    let imported = assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "pipeline",
        "import",
        import_file.to_str().unwrap(),
        "--replace-all",
    ]));
    let imported: Value = serde_json::from_str(&imported).unwrap();
    assert_eq!(imported["imported"], 1);
    let first_series = imported["series"][0]["series_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let exported = dir.path().join("pipelines.toml");
    assert!(
        exported.exists(),
        "import writes the backup next to the database"
    );
    let backup = std::fs::read_to_string(&exported).unwrap();
    assert!(backup.contains("id = \"press-01-cycle-count\""));

    let reset = assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "pipeline",
        "reset",
        "press-01-cycle-count",
    ]));
    let reset: Value = serde_json::from_str(&reset).unwrap();
    assert_ne!(reset["series_id"].as_str().unwrap(), first_series);
    assert_eq!(reset["published_sequence"], 1);

    let elsewhere = dir.path().join("copy.toml");
    assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "pipeline",
        "export",
        "--export-path",
        elsewhere.to_str().unwrap(),
    ]));
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), backup);

    let listed = assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "pipeline",
        "list",
    ]));
    let listed: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed[0]["id"], "press-01-cycle-count");

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let outbox: i64 = conn
        .query_row("SELECT COUNT(*) FROM observation_outbox", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        outbox, 2,
        "import and reset each published sequence 1 value 0"
    );

    // A tuning change through `update` keeps the series (#232 child issue 4).
    let tuned_file = dir.path().join("tuned.toml");
    std::fs::write(
        &tuned_file,
        backup.replace("rise_threshold = 0.5", "rise_threshold = 0.7"),
    )
    .unwrap();
    let updated = assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "pipeline",
        "update",
        tuned_file.to_str().unwrap(),
    ]));
    let updated: Value = serde_json::from_str(&updated).unwrap();
    assert_eq!(updated[0]["id"], "press-01-cycle-count");
    assert!(
        updated[0]["new_series"].is_null(),
        "a tuning change keeps the series: {updated}"
    );
    assert!(
        std::fs::read_to_string(&exported)
            .unwrap()
            .contains("rise_threshold = 0.7")
    );

    let deleted = assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "pipeline",
        "delete",
        "press-01-cycle-count",
    ]));
    let deleted: Value = serde_json::from_str(&deleted).unwrap();
    assert_eq!(deleted["deleted"], true);
    let listed = assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "pipeline",
        "list",
    ]));
    assert_eq!(
        serde_json::from_str::<Value>(&listed).unwrap(),
        serde_json::json!([])
    );
    let (rows, empty_retained): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), SUM(LENGTH(payload) = 0 AND retain) FROM observation_outbox",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (rows, empty_retained),
        (3, 1),
        "delete enqueues the zero-length retained payload"
    );
}
