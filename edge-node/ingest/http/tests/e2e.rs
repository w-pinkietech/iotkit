use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use axum::http::Method;
use iotkit_core_collector::{Collector, PermissiveRegistry};
use iotkit_core_ops::{
    Actor, ActorKind, DispatchRequest, DispatchResult, Tier, dispatch, hash_passphrase,
    reset_passphrase_with_hash, standard_catalog,
};
use iotkit_core_storage::{DbHandle, Migration};
use iotkit_ingest_contract::{AckStatus, Envelope, EnvelopeAck, ValidationReport};
use rcgen::{CertificateParams, KeyPair};
use rustls::pki_types::pem::PemObject;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::routes::HttpIngestHooks;
use super::{
    ExposureSnapshot, HttpIngestConfig, HttpIngestService, Listener, ListenerConfig, ListenerMode,
    LocalIngressCidr, ServingListener, SystemMonotonicClock, TlsMaterial, ValidatedListenerConfig,
};

#[test]
fn normative_contract_publishes_the_three_command_pinned_tls_journey() {
    let contract = include_str!("../../../../docs/okf/en/contracts/ingest-v1.md");
    assert!(contract.contains("# IoTKit authenticated ingest contract v1"));
    assert!(contract.contains("export IOTKIT_URL"));
    assert!(contract.contains("printf '%s\\n'"));
    assert!(contract.contains("curl --fail-with-body"));
    assert!(contract.contains("--cacert \"$IOTKIT_CA\""));
}

#[test]
fn documented_json_examples_are_compatible_with_shipped_wire_types() {
    let contract = include_str!("../../../../docs/okf/en/contracts/ingest-v1.md");
    let blocks = json_blocks(contract);
    let envelope = blocks
        .iter()
        .find_map(|block| serde_json::from_str::<Envelope>(block).ok())
        .expect("the contract must contain a request envelope example");
    assert_eq!(envelope.envelope_id, "builder-example-0001");

    let acknowledgements = blocks
        .iter()
        .filter_map(|block| serde_json::from_str::<EnvelopeAck>(block).ok())
        .collect::<Vec<_>>();
    assert!(
        acknowledgements
            .iter()
            .any(|ack| matches!(ack.status, AckStatus::Accepted { .. }))
    );
    assert!(
        acknowledgements
            .iter()
            .any(|ack| matches!(ack.status, AckStatus::Duplicate))
    );
    assert!(
        acknowledgements
            .iter()
            .any(|ack| matches!(ack.status, AckStatus::Rejected { .. }))
    );
    let report = blocks
        .iter()
        .find_map(|block| serde_json::from_str::<ValidationReport>(block).ok())
        .expect("the contract must contain a validation report example");
    assert!(!report.valid);
    assert!(contract.contains("`429` + bounded `Retry-After`"));
    assert!(contract.contains("`503`, no ack"));
}

#[test]
fn normative_contract_matches_shipped_wire_schema_and_finite_defaults() {
    let contract = include_str!("../../../../docs/okf/en/contracts/ingest-v1.md");
    for heading in [
        "### Envelope field table",
        "### ReadingItem field table",
        "### EnvelopeAck and status field table",
        "### ValidationReport field table",
        "### Stable enum vocabulary",
        "### Shipped finite receiver defaults",
    ] {
        assert!(
            contract.contains(heading),
            "missing normative section: {heading}"
        );
    }
    for field in [
        "| `envelope_id` | JSON string | required |",
        "| `declaration_version` | JSON unsigned integer (`u32`) or `null` | optional |",
        "| `items` | JSON array of `ReadingItem` | required | 0..=256 items",
        "| `channel_index` | JSON unsigned integer (`u16`) | optional | 0..=65535",
        "| `device_time_ms` | JSON signed integer (`i64`) | optional |",
        "| `age_ms` | JSON unsigned integer (`u64`) | optional |",
        "| `rssi` | JSON signed integer (`i16`) | optional |",
        "| `battery_pct` | JSON unsigned integer (`u8`) | optional | 0..=255",
        "| `values` | JSON array of finite numbers (`f64`) | required |",
        "| `measurement_key` | JSON string | required | One or more dot-separated",
        "| `valid` | JSON boolean | required |",
        "| `item_index` | JSON non-negative integer (`usize`) or `null` | optional |",
    ] {
        assert!(contract.contains(field), "missing wire field row: {field}");
    }
    for value in [
        "accepted",
        "duplicate",
        "rejected",
        "deferred",
        "stored",
        "item_rejected",
        "durable",
        "staged",
        "quarantined",
        "out_of_range",
        "unknown_key",
        "undeclared_channel",
        "device_quarantined",
        "malformed_measurement_key",
        "value_type_mismatch",
        "unknown_subject",
        "subject_scope_violation",
        "batch_too_large",
        "stale_timestamp",
        "internal",
        "device_ntp",
        "device_rtc",
        "edge_node",
        "edge_node_adjusted",
    ] {
        assert!(
            contract.contains(&format!("`{value}`")),
            "missing stable enum value: {value}"
        );
    }
    for default in [
        "| request headers | 32 headers / 8,192 bytes |",
        "| decoded JSON body | 65,536 bytes |",
        "| items per envelope | 256 hard maximum / 256 HTTP default |",
        "| concurrent requests / connections | 16 / 32 |",
        "| TLS handshake | 5 seconds per peer |",
        "| authentication workers / reserved workers | 2 / 1 |",
        "| general authentication rate / burst / initial tokens | 16 / 32 / 1 |",
        "| reserved authentication rate / burst / initial tokens | 8 / 8 / 1 |",
        "| principal-state capacity | 64 principals |",
        "| global flow rate / burst | 4,000,000 / 4,000,000 units |",
        "| throttle cooldown | 5,000 ms |",
        "| deduplication rows / principal rows / age | 100,000 / 10,000 / 72 hours |",
        "| unknown-subject staging rows / bytes / age | 10,000 / 64 MiB global; 1,000 / 8 MiB per principal; 30 days |",
    ] {
        assert!(
            contract.contains(default),
            "missing finite default row: {default}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn documented_journey_is_pinned_tls_and_survives_duplicate_and_restart() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let passphrase = test_passphrase();
    let (material, ca_path) = test_tls_material(&dir);
    let trace_capture = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer({
            let trace_capture = Arc::clone(&trace_capture);
            move || TraceWriter(Arc::clone(&trace_capture))
        })
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let db_path = dir.path().join("journey.db");
    let (principal, token) = provision_device(&db_path);
    let running = start_edge(
        &db_path,
        material.clone(),
        HttpIngestConfig::for_test(),
        HttpIngestHooks::default(),
        principal.clone(),
        token.clone(),
    )
    .await;

    let (export, write, curl) = journey_commands();
    let setup = shell(
        &format!("{export}\n{write}"),
        dir.path(),
        &[
            ("IOTKIT_OPERATOR_URL", running.url()),
            ("IOTKIT_OPERATOR_TOKEN", running.token.clone()),
            ("IOTKIT_OPERATOR_CA", ca_path.display().to_string()),
            ("IOTKIT_OPERATOR_SOURCE", running.principal.clone()),
        ],
    )
    .await;
    assert!(
        setup.status.success(),
        "the documented export/write steps must succeed"
    );
    let unpinned = run_unpinned_request(&running, &dir.path().join("one-envelope.json")).await;
    assert!(
        !unpinned.status.success(),
        "an untrusted self-signed TLS server must fail"
    );

    let journey = shell(
        &format!("{export}\n{curl}"),
        dir.path(),
        &[
            ("IOTKIT_OPERATOR_URL", running.url()),
            ("IOTKIT_OPERATOR_TOKEN", running.token.clone()),
            ("IOTKIT_OPERATOR_CA", ca_path.display().to_string()),
            ("IOTKIT_OPERATOR_SOURCE", running.principal.clone()),
        ],
    )
    .await;
    assert!(
        journey.status.success(),
        "the documented pinned journey must succeed"
    );
    let first_ack: EnvelopeAck = serde_json::from_slice(&journey.stdout).unwrap();
    assert!(matches!(first_ack.status, AckStatus::Accepted { .. }));
    assert_eq!(database_counts(&running.db).0, 1);

    let duplicate = run_documented_journey(&running, &dir, &ca_path, &export, &write, &curl).await;
    assert!(duplicate.status.success());
    let duplicate_ack: EnvelopeAck = serde_json::from_slice(&duplicate.stdout).unwrap();
    assert!(matches!(duplicate_ack.status, AckStatus::Duplicate));
    assert_eq!(database_counts(&running.db).0, 1);

    let mixed_path = dir.path().join("mixed-envelope.json");
    write_json_envelope(
        &mixed_path,
        &principal,
        "mixed-items-0001",
        json!([
            {
                "measurement_key": "temperature_c",
                "values": [22.0],
                "time_source": "edge_node"
            },
            {
                "subject_hint": "not-registered",
                "measurement_key": "temperature_c",
                "values": [23.0],
                "time_source": "edge_node"
            }
        ]),
    );
    let mixed = run_pinned_body(&running, &ca_path, &mixed_path, "mixed-response").await;
    assert!(mixed.status.success());
    let mixed_ack: EnvelopeAck = serde_json::from_slice(&mixed.stdout).unwrap();
    match mixed_ack.status {
        AckStatus::Accepted { items } => {
            assert_eq!(items.len(), 2);
            assert!(matches!(
                items[0],
                iotkit_ingest_contract::ItemStatus::Stored { .. }
            ));
            assert!(matches!(
                items[1],
                iotkit_ingest_contract::ItemStatus::ItemRejected {
                    reason_code: iotkit_ingest_contract::ReasonCode::UnknownSubject,
                    ..
                }
            ));
        }
        other => panic!("mixed item response was not accepted: {other:?}"),
    }
    assert_eq!(database_counts(&running.db).0, 2);

    let rejected_path = dir.path().join("rejected-envelope.json");
    write_json_envelope(
        &rejected_path,
        "forged-source",
        "source-mismatch-0001",
        json!([{
            "measurement_key": "temperature_c",
            "values": [24.0],
            "time_source": "edge_node"
        }]),
    );
    let rejected = run_pinned_body(&running, &ca_path, &rejected_path, "rejected-response").await;
    assert!(rejected.status.success());
    let rejected_ack: EnvelopeAck = serde_json::from_slice(&rejected.stdout).unwrap();
    assert!(matches!(rejected_ack.status, AckStatus::Rejected { .. }));
    assert_eq!(database_counts(&running.db).0, 2);

    let before_validation = database_counts(&running.db);
    let validation_path = dir.path().join("validation-envelope.json");
    write_json_envelope(
        &validation_path,
        &principal,
        "validate-no-write-0001",
        json!([{
            "measurement_key": "temperature_c",
            "values": [25.0],
            "time_source": "edge_node"
        }]),
    );
    let validation = run_pinned_validation(&running, &ca_path, &validation_path).await;
    assert!(validation.status.success());
    let report: ValidationReport = serde_json::from_slice(&validation.stdout).unwrap();
    assert!(report.valid);
    assert_eq!(database_counts(&running.db), before_validation);

    let captures = [
        &journey,
        &duplicate,
        &mixed,
        &rejected,
        &validation,
        &unpinned,
    ];
    assert_captures_redact(&captures, &token, &passphrase);
    let mut malformed = tokio::net::TcpStream::connect(running.serving.local_addr())
        .await
        .unwrap();
    malformed.write_all(b"malformed TLS").await.unwrap();
    malformed.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if !trace_capture.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the malformed peer must produce captured service/transport error output");
    let trace_output = String::from_utf8_lossy(&trace_capture.lock().unwrap()).into_owned();
    assert!(!trace_output.contains(&token));
    assert!(!trace_output.contains(&passphrase));
    let audit = audit_text(&running.db);
    assert!(!audit.contains(&token));
    assert!(!audit.contains(&passphrase));

    let secret_for_restart = running.token.clone();
    stop_edge(running).await;
    let restarted = start_edge(
        &db_path,
        material,
        HttpIngestConfig::for_test(),
        HttpIngestHooks::default(),
        principal,
        secret_for_restart.clone(),
    )
    .await;
    let after_restart =
        run_documented_journey(&restarted, &dir, &ca_path, &export, &write, &curl).await;
    assert!(after_restart.status.success());
    let after_restart_ack: EnvelopeAck = serde_json::from_slice(&after_restart.stdout).unwrap();
    assert!(matches!(after_restart_ack.status, AckStatus::Duplicate));
    assert_eq!(database_counts(&restarted.db).0, 2);

    stop_edge(restarted).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_tls_retry_probes_keep_429_and_503_without_ack() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let (material, ca_path) = test_tls_material(&dir);

    let db_429 = dir.path().join("throttle-429.db");
    let (principal_429, token_429) = provision_device(&db_429);
    let (entered_429_tx, entered_429_rx) = std::sync::mpsc::sync_channel(1);
    let gate_429 = Arc::new((Mutex::new(false), Condvar::new()));
    let hook_gate_429 = Arc::clone(&gate_429);
    let hooks_429 = HttpIngestHooks::default().with_before_collector_handoff(move || {
        let _ = entered_429_tx.try_send(());
        wait_for_gate(&hook_gate_429);
    });
    let mut config_429 = HttpIngestConfig::for_test();
    config_429.concurrent_requests = 1;
    let running_429 = start_edge(
        &db_429,
        material.clone(),
        config_429,
        hooks_429,
        principal_429.clone(),
        token_429,
    )
    .await;
    let payload_429 = dir.path().join("retry-429.json");
    write_json_envelope(
        &payload_429,
        &principal_429,
        "retry-after-429-0001",
        json!([{"measurement_key":"temperature_c","values":[27.0],"time_source":"edge_node"}]),
    );
    let first_429 = tokio::spawn(run_pinned_body_owned(
        running_429.url(),
        running_429.token.clone(),
        ca_path.clone(),
        payload_429.clone(),
        "first-429".into(),
    ));
    let _gate_guard_429 = GateReleaseGuard::new(Arc::clone(&gate_429));
    tokio::task::spawn_blocking(move || {
        entered_429_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("429 gate hook must be reached within the test deadline")
    })
    .await
    .unwrap();
    let throttle_429 =
        run_pinned_status(&running_429, &ca_path, &payload_429, "throttle-429").await;
    assert_eq!(throttle_429.output.stdout_string().trim(), "429");
    assert_retry_response_without_ack(&throttle_429, true);
    let original_429 = std::fs::read(&payload_429).unwrap();
    release_gate(&gate_429);
    let first_429 = first_429.await.unwrap();
    assert!(first_429.status.success());
    assert_eq!(std::fs::read(&payload_429).unwrap(), original_429);
    let retry_429 = run_pinned_body(&running_429, &ca_path, &payload_429, "retry-429").await;
    assert!(retry_429.status.success());
    let retry_429_ack: EnvelopeAck = serde_json::from_slice(&retry_429.stdout).unwrap();
    assert!(matches!(retry_429_ack.status, AckStatus::Duplicate));
    assert_eq!(database_counts(&running_429.db).0, 1);
    stop_edge(running_429).await;

    let db_503 = dir.path().join("throttle-503.db");
    let (principal_503, token_503) = provision_device(&db_503);
    let (entered_503_tx, entered_503_rx) = std::sync::mpsc::sync_channel(1);
    let gate_503 = Arc::new((Mutex::new(false), Condvar::new()));
    let hook_gate_503 = Arc::clone(&gate_503);
    let hooks_503 = HttpIngestHooks::default().with_after_queue_acquired(move || {
        let _ = entered_503_tx.try_send(());
        wait_for_gate(&hook_gate_503);
    });
    let mut config_503 = HttpIngestConfig::for_test();
    config_503.collector_queue_slots = 1;
    let running_503 = start_edge(
        &db_503,
        material,
        config_503,
        hooks_503,
        principal_503.clone(),
        token_503,
    )
    .await;
    let payload_503 = dir.path().join("retry-503.json");
    write_json_envelope(
        &payload_503,
        &principal_503,
        "retry-after-503-0001",
        json!([{"measurement_key":"temperature_c","values":[28.0],"time_source":"edge_node"}]),
    );
    let first_503 = tokio::spawn(run_pinned_body_owned(
        running_503.url(),
        running_503.token.clone(),
        ca_path.clone(),
        payload_503.clone(),
        "first-503".into(),
    ));
    let _gate_guard_503 = GateReleaseGuard::new(Arc::clone(&gate_503));
    tokio::task::spawn_blocking(move || {
        entered_503_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("503 gate hook must be reached within the test deadline")
    })
    .await
    .unwrap();
    let throttle_503 =
        run_pinned_status(&running_503, &ca_path, &payload_503, "throttle-503").await;
    assert_eq!(throttle_503.output.stdout_string().trim(), "503");
    assert_retry_response_without_ack(&throttle_503, false);
    let original_503 = std::fs::read(&payload_503).unwrap();
    release_gate(&gate_503);
    let first_503 = first_503.await.unwrap();
    assert!(first_503.status.success());
    assert_eq!(std::fs::read(&payload_503).unwrap(), original_503);
    let retry_503 = run_pinned_body(&running_503, &ca_path, &payload_503, "retry-503").await;
    assert!(retry_503.status.success());
    let retry_503_ack: EnvelopeAck = serde_json::from_slice(&retry_503.stdout).unwrap();
    assert!(matches!(retry_503_ack.status, AckStatus::Duplicate));
    assert_eq!(database_counts(&running_503.db).0, 1);
    stop_edge(running_503).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_tls_route_inventory_has_no_setup_or_control_api_route() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let (material, ca_path) = test_tls_material(&dir);
    let db_path = dir.path().join("routes.db");
    let (principal, token) = provision_device(&db_path);
    let running = start_edge(
        &db_path,
        material,
        HttpIngestConfig::for_test(),
        HttpIngestHooks::default(),
        principal,
        token,
    )
    .await;

    assert_eq!(
        run_path_status(&running, &ca_path, Method::GET, "/api/v1/ingest").await,
        405
    );
    assert_eq!(
        run_path_status(&running, &ca_path, Method::POST, "/api/v1/setup/passphrase").await,
        404
    );
    assert_eq!(
        run_path_status(&running, &ca_path, Method::POST, "/api/v1/control/ops").await,
        404
    );
    assert_eq!(
        run_path_status(&running, &ca_path, Method::POST, "/api/v1/ingest/validate").await,
        401
    );

    let bad_key = KeyPair::generate().unwrap().serialize_pem();
    assert!(
        TlsMaterial::validate(
            std::fs::read(&ca_path).unwrap(),
            bad_key.into_bytes(),
            &material_fingerprint(&ca_path),
            1,
        )
        .is_err()
    );
    stop_edge(running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_accept_loop_survives_junk_tls_and_keeps_serving() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let (material, ca_path) = test_tls_material(&dir);
    let db_path = dir.path().join("tls-junk.db");
    let (principal, token) = provision_device(&db_path);
    let running = start_edge(
        &db_path,
        material,
        HttpIngestConfig::for_test(),
        HttpIngestHooks::default(),
        principal,
        token,
    )
    .await;
    let address = running.serving.local_addr();

    let mut junk = tokio::net::TcpStream::connect(address).await.unwrap();
    junk.write_all(b"not a TLS ClientHello").await.unwrap();
    junk.shutdown().await.unwrap();

    let response = tokio::time::timeout(
        Duration::from_secs(1),
        pinned_http_request(address, &ca_path, "localhost"),
    )
    .await
    .expect("a junk TLS peer must not stop the listener")
    .expect("the pinned request after junk TLS must complete");
    assert!(response.starts_with(b"HTTP/1.1 405"));
    stop_edge(running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_tls_does_not_monopolize_accept_loop() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let (material, ca_path) = test_tls_material(&dir);
    let db_path = dir.path().join("tls-stalled.db");
    let (principal, token) = provision_device(&db_path);
    let running = start_edge(
        &db_path,
        material,
        HttpIngestConfig::for_test(),
        HttpIngestHooks::default(),
        principal,
        token,
    )
    .await;
    let address = running.serving.local_addr();
    let stalled = tokio::net::TcpStream::connect(address).await.unwrap();

    let response = tokio::time::timeout(
        Duration::from_secs(1),
        pinned_http_request(address, &ca_path, "localhost"),
    )
    .await
    .expect("a stalled TLS peer must not delay another accepted peer")
    .expect("the pinned request beside a stalled TLS peer must complete");
    assert!(response.starts_with(b"HTTP/1.1 405"));

    drop(stalled);
    stop_edge(running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_hostname_tls_client_does_not_stop_listener() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let (material, ca_path) = test_tls_material(&dir);
    let db_path = dir.path().join("tls-hostname.db");
    let (principal, token) = provision_device(&db_path);
    let running = start_edge(
        &db_path,
        material,
        HttpIngestConfig::for_test(),
        HttpIngestHooks::default(),
        principal,
        token,
    )
    .await;
    let address = running.serving.local_addr();

    let untrusted = untrusted_tls_request(address, "localhost").await;
    assert!(
        untrusted.is_err(),
        "an untrusted client CA must fail the client"
    );
    let failed = pinned_http_request(address, &ca_path, "wrong-hostname.example").await;
    assert!(
        failed.is_err(),
        "hostname validation must fail for the client"
    );

    let response = tokio::time::timeout(
        Duration::from_secs(1),
        pinned_http_request(address, &ca_path, "localhost"),
    )
    .await
    .expect("a failed hostname validation must not stop the listener")
    .expect("the pinned request after a hostname failure must complete");
    assert!(response.starts_with(b"HTTP/1.1 405"));
    stop_edge(running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peer_cidr_rejection_closes_only_that_connection() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let (material, ca_path) = test_tls_material(&dir);
    let db_path = dir.path().join("tls-peer-cidr.db");
    let (principal, token) = provision_device(&db_path);
    let running = start_edge_with_cidr(
        &db_path,
        material,
        HttpIngestConfig::for_test(),
        HttpIngestHooks::default(),
        principal,
        token,
        "127.0.0.1/32",
    )
    .await;
    let address = running.serving.local_addr();
    let rejected_socket = tokio::net::TcpSocket::new_v4().unwrap();
    rejected_socket
        .bind("127.0.0.2:0".parse().unwrap())
        .unwrap();
    let mut rejected = rejected_socket.connect(address).await.unwrap();
    rejected
        .write_all(b"peer outside configured CIDR")
        .await
        .unwrap();
    rejected.shutdown().await.unwrap();

    let response = tokio::time::timeout(
        Duration::from_secs(1),
        pinned_http_request(address, &ca_path, "localhost"),
    )
    .await
    .expect("peer-CIDR rejection must not stop the listener")
    .expect("a permitted peer after CIDR rejection must complete");
    assert!(response.starts_with(b"HTTP/1.1 405"));
    stop_edge(running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pre_tls_connection_permit_is_bounded_and_released_on_shutdown() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let (material, ca_path) = test_tls_material(&dir);
    let db_path = dir.path().join("tls-permit.db");
    let (principal, token) = provision_device(&db_path);
    let mut config = HttpIngestConfig::for_test();
    config.concurrent_connections = 1;
    let running = start_edge(
        &db_path,
        material,
        config,
        HttpIngestHooks::default(),
        principal,
        token,
    )
    .await;
    let address = running.serving.local_addr();
    let stalled = tokio::net::TcpStream::connect(address).await.unwrap();
    let service = running.service.clone();

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if service.admission_health().connection_pressure_percent == 100 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the stalled pre-TLS peer must consume the one configured connection permit");

    let second = tokio::time::timeout(
        Duration::from_secs(1),
        pinned_http_request(address, &ca_path, "localhost"),
    )
    .await
    .expect("a second peer must not wait behind an unbounded TLS task set");
    assert!(
        second.is_err(),
        "the bounded connection gate must reject peer two"
    );

    drop(stalled);
    stop_edge(running).await;
    assert_eq!(service.admission_health().connection_pressure_percent, 0);
}

fn json_blocks(document: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut lines = document.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "```json" {
            continue;
        }
        let mut block = String::new();
        for line in &mut lines {
            if line.trim() == "```" {
                break;
            }
            block.push_str(line);
            block.push('\n');
        }
        blocks.push(Box::leak(block.into_boxed_str()) as &str);
    }
    blocks
}

fn journey_commands() -> (String, String, String) {
    let contract = include_str!("../../../../docs/okf/en/contracts/ingest-v1.md");
    let block = contract
        .split("```sh\n")
        .nth(1)
        .and_then(|body| body.split("\n```").next())
        .expect("normative contract must contain one shell journey");
    let commands = block.lines().map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(
        commands.len(),
        3,
        "the first journey must contain exactly three commands"
    );
    assert!(commands[0].starts_with("export IOTKIT_URL"));
    assert!(commands[1].starts_with("printf '%s\\n'"));
    assert!(commands[2].starts_with("curl --fail-with-body"));
    assert!(!commands[2].contains("--insecure"));
    (
        commands[0].clone(),
        commands[1].clone(),
        commands[2].clone(),
    )
}

async fn run_documented_journey(
    running: &RunningEdge,
    dir: &TempDir,
    ca_path: &Path,
    export: &str,
    write: &str,
    curl: &str,
) -> Output {
    let script = format!("{export}\n{write}\n{curl}");
    shell(
        &script,
        dir.path(),
        &[
            ("IOTKIT_OPERATOR_URL", running.url()),
            ("IOTKIT_OPERATOR_TOKEN", running.token.clone()),
            ("IOTKIT_OPERATOR_CA", ca_path.display().to_string()),
            ("IOTKIT_OPERATOR_SOURCE", running.principal.clone()),
        ],
    )
    .await
}

async fn run_unpinned_request(running: &RunningEdge, payload: &Path) -> Output {
    shell(
        "curl --fail-with-body --silent --show-error --output /dev/null --header \"Authorization: Bearer $IOTKIT_TOKEN\" --header \"Content-Type: application/json\" --data-binary \"@$IOTKIT_PAYLOAD\" \"$IOTKIT_URL/api/v1/ingest\"",
        payload.parent().unwrap_or_else(|| Path::new(".")),
        &[
            ("IOTKIT_URL", running.url()),
            ("IOTKIT_TOKEN", running.token.clone()),
            ("IOTKIT_PAYLOAD", payload.display().to_string()),
        ],
    )
    .await
}

async fn run_pinned_body(
    running: &RunningEdge,
    ca_path: &Path,
    payload: &Path,
    response_name: &str,
) -> Output {
    run_pinned_body_owned(
        running.url(),
        running.token.clone(),
        ca_path.to_owned(),
        payload.to_owned(),
        response_name.to_owned(),
    )
    .await
}

async fn run_pinned_body_owned(
    url: String,
    token: String,
    ca_path: PathBuf,
    payload: PathBuf,
    response_name: String,
) -> Output {
    let response = payload
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{response_name}.json"));
    shell(
        "curl --fail-with-body --silent --show-error --cacert \"$IOTKIT_CA\" --header \"Authorization: Bearer $IOTKIT_TOKEN\" --header \"Content-Type: application/json\" --data-binary \"@$IOTKIT_PAYLOAD\" \"$IOTKIT_URL/api/v1/ingest\"",
        payload.parent().unwrap_or_else(|| Path::new(".")),
        &[
            ("IOTKIT_URL", url),
            ("IOTKIT_TOKEN", token),
            ("IOTKIT_CA", ca_path.display().to_string()),
            ("IOTKIT_PAYLOAD", payload.display().to_string()),
            ("IOTKIT_RESPONSE", response.display().to_string()),
        ],
    )
    .await
}

async fn run_pinned_validation(running: &RunningEdge, ca_path: &Path, payload: &Path) -> Output {
    shell(
        "curl --fail-with-body --silent --show-error --cacert \"$IOTKIT_CA\" --header \"Authorization: Bearer $IOTKIT_TOKEN\" --header \"Content-Type: application/json\" --data-binary \"@$IOTKIT_PAYLOAD\" \"$IOTKIT_URL/api/v1/ingest/validate\"",
        payload.parent().unwrap_or_else(|| Path::new(".")),
        &[
            ("IOTKIT_URL", running.url()),
            ("IOTKIT_TOKEN", running.token.clone()),
            ("IOTKIT_CA", ca_path.display().to_string()),
            ("IOTKIT_PAYLOAD", payload.display().to_string()),
        ],
    )
    .await
}

async fn pinned_http_request(
    address: SocketAddr,
    ca_path: &Path,
    server_name: &str,
) -> Result<Vec<u8>, String> {
    let cert_pem = std::fs::read(ca_path).map_err(|error| error.to_string())?;
    let cert = rustls::pki_types::CertificateDer::pem_slice_iter(&cert_pem)
        .next()
        .ok_or_else(|| "CA file did not contain a certificate".to_owned())
        .and_then(|result| result.map_err(|error| error.to_string()))?;
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert).map_err(|error| error.to_string())?;
    let client = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
    let raw = tokio::net::TcpStream::connect(address)
        .await
        .map_err(|error| error.to_string())?;
    let name = server_name
        .to_owned()
        .try_into()
        .map_err(|error: rustls::pki_types::InvalidDnsNameError| error.to_string())?;
    let mut stream = connector
        .connect(name, raw)
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(b"GET /api/v1/ingest HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .map_err(|error| error.to_string())?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .map_err(|error| error.to_string())?;
    Ok(response)
}

async fn untrusted_tls_request(address: SocketAddr, server_name: &str) -> Result<Vec<u8>, String> {
    let roots = rustls::RootCertStore::empty();
    let client = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
    let raw = tokio::net::TcpStream::connect(address)
        .await
        .map_err(|error| error.to_string())?;
    let name = server_name
        .to_owned()
        .try_into()
        .map_err(|error: rustls::pki_types::InvalidDnsNameError| error.to_string())?;
    let mut stream = connector
        .connect(name, raw)
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(b"GET /api/v1/ingest HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .map_err(|error| error.to_string())?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .map_err(|error| error.to_string())?;
    Ok(response)
}

async fn run_pinned_status(
    running: &RunningEdge,
    ca_path: &Path,
    payload: &Path,
    response_name: &str,
) -> CapturedHttpResponse {
    let response = payload
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{response_name}.json"));
    let headers = payload
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{response_name}.headers"));
    let output = shell(
        "curl --silent --show-error --cacert \"$IOTKIT_CA\" --dump-header \"$IOTKIT_HEADERS\" --output \"$IOTKIT_RESPONSE\" --write-out '%{http_code}' --header \"Authorization: Bearer $IOTKIT_TOKEN\" --header \"Content-Type: application/json\" --data-binary \"@$IOTKIT_PAYLOAD\" \"$IOTKIT_URL/api/v1/ingest\"",
        payload.parent().unwrap_or_else(|| Path::new(".")),
        &[
            ("IOTKIT_URL", running.url()),
            ("IOTKIT_TOKEN", running.token.clone()),
            ("IOTKIT_CA", ca_path.display().to_string()),
            ("IOTKIT_PAYLOAD", payload.display().to_string()),
            ("IOTKIT_RESPONSE", response.display().to_string()),
            ("IOTKIT_HEADERS", headers.display().to_string()),
        ],
    )
    .await;
    CapturedHttpResponse {
        output,
        headers: std::fs::read(headers).expect("curl must write response headers"),
        body: std::fs::read(response).expect("curl must write response body"),
    }
}

async fn run_path_status(running: &RunningEdge, ca_path: &Path, method: Method, path: &str) -> u16 {
    let method = method.as_str().to_owned();
    let output = shell(
        &format!(
            "curl --silent --show-error --cacert \"$IOTKIT_CA\" --output /dev/null --write-out '%{{http_code}}' --header 'Content-Type: application/json' --data '{{}}' --request {method} \"$IOTKIT_URL{path}\""
        ),
        ca_path.parent().unwrap_or_else(|| Path::new(".")),
        &[
            ("IOTKIT_URL", running.url()),
            ("IOTKIT_CA", ca_path.display().to_string()),
        ],
    )
    .await;
    assert!(output.status.success());
    output.stdout_string().trim().parse().unwrap()
}

async fn shell(script: &str, cwd: &Path, env: &[(impl AsRef<str>, String)]) -> Output {
    let script = script.to_owned();
    let cwd = cwd.to_owned();
    let env = env
        .iter()
        .map(|(name, value)| (name.as_ref().to_owned(), value.clone()))
        .collect::<Vec<_>>();
    tokio::task::spawn_blocking(move || {
        let mut command = Command::new("sh");
        command
            .args(["-eu", "-c", &script])
            .current_dir(cwd)
            .env("NO_PROXY", "localhost,127.0.0.1")
            .env("no_proxy", "localhost,127.0.0.1")
            .env_remove("CURL_CA_BUNDLE")
            .env_remove("SSL_CERT_FILE")
            .env_remove("SSL_CERT_DIR");
        for (name, value) in env {
            command.env(name, value);
        }
        command.output().expect("curl shell command must start")
    })
    .await
    .expect("curl shell task must finish")
}

trait OutputText {
    fn stdout_string(&self) -> String;
}

impl OutputText for Output {
    fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

#[derive(Clone)]
struct TraceWriter(Arc<Mutex<Vec<u8>>>);

impl Write for TraceWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct CapturedHttpResponse {
    output: Output,
    headers: Vec<u8>,
    body: Vec<u8>,
}

fn assert_retry_response_without_ack(response: &CapturedHttpResponse, require_retry_after: bool) {
    assert!(
        response.body.is_empty(),
        "overload response must have an empty body, got {:?}",
        String::from_utf8_lossy(&response.body)
    );
    assert!(
        serde_json::from_slice::<EnvelopeAck>(&response.body).is_err(),
        "overload response must not deserialize as EnvelopeAck"
    );
    let retry_after = response
        .headers
        .split(|byte| *byte == b'\n')
        .find_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let colon = line.iter().position(|byte| *byte == b':')?;
            let (name, value) = line.split_at(colon);
            let value = &value[1..];
            (name.eq_ignore_ascii_case(b"retry-after")).then(|| {
                String::from_utf8_lossy(value)
                    .trim()
                    .parse::<u64>()
                    .expect("Retry-After must be numeric")
            })
        });
    if require_retry_after {
        assert!(
            retry_after.is_some_and(|seconds| (1..=3600).contains(&seconds)),
            "429 must carry a bounded numeric Retry-After header; headers={:?}",
            String::from_utf8_lossy(&response.headers)
        );
    }
}

struct GateReleaseGuard {
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl GateReleaseGuard {
    fn new(gate: Arc<(Mutex<bool>, Condvar)>) -> Self {
        Self { gate }
    }
}

impl Drop for GateReleaseGuard {
    fn drop(&mut self) {
        release_gate(&self.gate);
    }
}

struct RunningEdge {
    db: DbHandle,
    service: HttpIngestService<SystemMonotonicClock>,
    serving: ServingListener,
    collector_task: tokio::task::JoinHandle<()>,
    principal: String,
    token: String,
}

impl RunningEdge {
    fn url(&self) -> String {
        format!("https://localhost:{}", self.serving.local_addr().port())
    }
}

async fn start_edge(
    db_path: &Path,
    material: TlsMaterial,
    config: HttpIngestConfig,
    hooks: HttpIngestHooks,
    principal: String,
    token: String,
) -> RunningEdge {
    start_edge_with_cidr(
        db_path,
        material,
        config,
        hooks,
        principal,
        token,
        "127.0.0.0/8",
    )
    .await
}

async fn start_edge_with_cidr(
    db_path: &Path,
    material: TlsMaterial,
    config: HttpIngestConfig,
    hooks: HttpIngestHooks,
    principal: String,
    token: String,
    site_cidr: &str,
) -> RunningEdge {
    let db = iotkit_core_storage::init_db(db_path, &migrations()).unwrap();
    let (collector, issuer, collector_task) =
        Collector::spawn_device_composed(db.clone(), Arc::new(PermissiveRegistry), 8);
    let service = HttpIngestService::new_with_hooks(
        db.clone(),
        collector,
        issuer,
        config,
        SystemMonotonicClock::default(),
        hooks,
    )
    .unwrap();
    let exposure = ExposureSnapshot::new("lo", [IpAddr::V4(Ipv4Addr::LOCALHOST)], false);
    let config = ListenerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        interface: "lo".into(),
        local_ingress_cidrs: vec![site_cidr.parse::<LocalIngressCidr>().unwrap()],
        mode: ListenerMode::Tls(material),
    };
    let listener =
        Listener::bind(ValidatedListenerConfig::new_for_test(config, &exposure).unwrap())
            .await
            .unwrap();
    let serving = listener.serve(service.clone()).unwrap();
    RunningEdge {
        db,
        service,
        serving,
        collector_task,
        principal,
        token,
    }
}

async fn stop_edge(running: RunningEdge) {
    let RunningEdge {
        serving,
        collector_task,
        service,
        db,
        ..
    } = running;
    serving.shutdown().await;
    collector_task.abort();
    let _ = collector_task.await;
    drop(service);
    drop(db);
}

fn test_tls_material(dir: &TempDir) -> (TlsMaterial, PathBuf) {
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let cert_pem = cert.pem();
    let key_pem = key.serialize_pem();
    let fingerprint = iotkit_core_ops::fingerprint_of_pem(&cert_pem).unwrap();
    let material = TlsMaterial::validate(
        cert_pem.as_bytes().to_vec(),
        key_pem.as_bytes().to_vec(),
        &fingerprint,
        1,
    )
    .unwrap();
    let ca_path = dir.path().join("edge-ca.pem");
    std::fs::write(&ca_path, cert_pem).unwrap();
    (material, ca_path)
}

fn material_fingerprint(ca_path: &Path) -> String {
    let pem = std::fs::read_to_string(ca_path).unwrap();
    iotkit_core_ops::fingerprint_of_pem(&pem).unwrap()
}

fn provision_device(path: &Path) -> (String, String) {
    let db = iotkit_core_storage::init_db(path, &migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let hash = hash_passphrase(&test_passphrase()).unwrap();
        reset_passphrase_with_hash(conn, &hash, "local_cli")
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        Ok(())
    })
    .unwrap();
    let result = db
        .with_conn_sync(|conn| {
            Ok(dispatch(
                conn,
                standard_catalog(),
                DispatchRequest {
                    op: "device.add_with_credential".into(),
                    params: json!({
                        "hardware_id": "task7-e2e-device",
                        "flow_class": "default",
                        "reason_code": "device_commissioning"
                    }),
                    dry_run: false,
                    actor: Actor {
                        actor_id: "local_cli".into(),
                        actor_kind: ActorKind::LocalCli,
                        tier_ceiling: Tier::Construction,
                    },
                    source: Some("local_cli".into()),
                    step_up_verified: true,
                    clock_trust: None,
                },
            )
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?)
        })
        .unwrap();
    let debug_capture = format!("{result:?}");
    let (metadata, plaintext) = match result {
        DispatchResult::DeviceCredential(secret) => secret.consume(),
        DispatchResult::Public(_) => panic!("device commissioning must return one-shot token"),
    };
    let principal = metadata["principal_id"].as_str().unwrap().to_owned();
    let token = plaintext.as_str().to_owned();
    assert!(!debug_capture.contains(&token));
    assert!(!debug_capture.contains(&test_passphrase()));
    (principal, token)
}

fn test_passphrase() -> String {
    format!("task7-local-passphrase-{}", std::process::id())
}

fn write_json_envelope(path: &Path, source: impl Into<String>, id: &str, items: Value) {
    let envelope = json!({
        "envelope_id": id,
        "source": source.into(),
        "items": items,
    });
    std::fs::write(path, serde_json::to_vec(&envelope).unwrap()).unwrap();
}

fn database_counts(db: &DbHandle) -> (i64, i64, i64, i64) {
    db.with_conn_sync(|conn| {
        Ok((
            conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?,
            conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |row| row.get(0))?,
            conn.query_row("SELECT COUNT(*) FROM staged_readings", [], |row| row.get(0))?,
            conn.query_row("SELECT COUNT(*) FROM ledger_events", [], |row| row.get(0))?,
        ))
    })
    .unwrap()
}

fn audit_text(db: &DbHandle) -> String {
    db.with_conn_sync(|conn| {
        Ok(conn.query_row(
            "SELECT COALESCE(group_concat(detail, '\\n'), '') FROM ledger_events",
            [],
            |row| row.get(0),
        )?)
    })
    .unwrap()
}

fn database_secret_scan(captures: &[&Output], token: &str, passphrase: &str) {
    for capture in captures {
        assert!(!capture.stdout_string().contains(token));
        assert!(!String::from_utf8_lossy(&capture.stderr).contains(token));
        assert!(!capture.stdout_string().contains(passphrase));
        assert!(!String::from_utf8_lossy(&capture.stderr).contains(passphrase));
    }
}

fn assert_captures_redact(captures: &[&Output], token: &str, passphrase: &str) {
    database_secret_scan(captures, token, passphrase);
}

fn wait_for_gate(gate: &Arc<(Mutex<bool>, Condvar)>) {
    let (lock, ready) = &**gate;
    let released = lock.lock().unwrap();
    let (released, result) = ready
        .wait_timeout_while(released, Duration::from_secs(5), |released| !*released)
        .unwrap();
    assert!(
        *released,
        "overload gate was not released before its deadline"
    );
    assert!(!result.timed_out(), "overload gate wait timed out");
}

fn release_gate(gate: &Arc<(Mutex<bool>, Condvar)>) {
    let (lock, ready) = &**gate;
    *lock.lock().unwrap() = true;
    ready.notify_all();
}

fn migrations() -> Vec<Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.sort_by_key(|migration| migration.version);
    all
}
