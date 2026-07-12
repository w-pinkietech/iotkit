use std::net::{IpAddr, Ipv4Addr};
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::task::{Context, Poll};

use crate::ManualMonotonicClock;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use iotkit_core_collector::{Collector, PermissiveRegistry, RegistryPolicy, RegistryVerdict};
use iotkit_core_ledger::{DeviceKind, DeviceState, NewDevice};
use iotkit_core_storage::Migration;
use iotkit_ingest_contract::{AckStatus, EnvelopeAck, ValidationReport};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{HttpIngestConfig, HttpIngestHooks, HttpIngestService};

struct PendingAfterOne(Option<bytes::Bytes>);

struct SignalThenPending {
    entered: Option<std::sync::mpsc::SyncSender<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutationEvent {
    Contended,
    Committed,
}

#[derive(Clone)]
struct BusyBarrier {
    events: std::sync::mpsc::SyncSender<MutationEvent>,
    allow_retry: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
}

static BUSY_BARRIER: LazyLock<std::sync::Mutex<Option<BusyBarrier>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));

fn reserved_writer_busy_handler(_attempts: i32) -> bool {
    let barrier = BUSY_BARRIER
        .lock()
        .unwrap()
        .as_ref()
        .expect("busy barrier installed")
        .clone();
    let _ = barrier.events.try_send(MutationEvent::Contended);
    wait_for_gate(&barrier.allow_retry);
    true
}

fn wait_for_gate(gate: &Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>) {
    let (lock, ready) = &**gate;
    let mut released = lock.lock().unwrap();
    while !*released {
        released = ready.wait(released).unwrap();
    }
}

fn release_gate(gate: &Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>) {
    let (lock, ready) = &**gate;
    *lock.lock().unwrap() = true;
    ready.notify_all();
}

impl http_body::Body for PendingAfterOne {
    type Data = bytes::Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        if let Some(bytes) = self.0.take() {
            Poll::Ready(Some(Ok(http_body::Frame::data(bytes))))
        } else {
            Poll::Pending
        }
    }
}

impl http_body::Body for SignalThenPending {
    type Data = bytes::Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        if let Some(entered) = self.entered.take() {
            entered.send(()).unwrap();
        }
        Poll::Pending
    }
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

fn fixture() -> (
    iotkit_core_storage::DbHandle,
    Collector,
    String,
    HttpIngestService<ManualMonotonicClock>,
) {
    fixture_with_policy(Arc::new(PermissiveRegistry), HttpIngestConfig::for_test())
}

fn fixture_with_policy(
    policy: Arc<dyn RegistryPolicy>,
    config: HttpIngestConfig,
) -> (
    iotkit_core_storage::DbHandle,
    Collector,
    String,
    HttpIngestService<ManualMonotonicClock>,
) {
    fixture_with_policy_and_hooks(policy, config, HttpIngestHooks::default())
}

fn fixture_with_policy_and_hooks(
    policy: Arc<dyn RegistryPolicy>,
    config: HttpIngestConfig,
    hooks: HttpIngestHooks,
) -> (
    iotkit_core_storage::DbHandle,
    Collector,
    String,
    HttpIngestService<ManualMonotonicClock>,
) {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    let secret = seed_route_credential(&db);
    build_service(
        db,
        policy,
        config,
        hooks,
        secret,
        ManualMonotonicClock::new(0),
    )
}

fn seed_route_credential(db: &iotkit_core_storage::DbHandle) -> String {
    let secret_bytes = Sha256::digest(format!(
        "route-fixture-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
    ));
    let secret = secret_bytes
        .iter()
        .fold(String::from("ikd_"), |mut encoded, byte| {
            use std::fmt::Write as _;
            write!(encoded, "{byte:02x}").unwrap();
            encoded
        });
    db.with_conn_sync(|conn| {
        let subject = iotkit_core_ledger::insert_device(
            conn,
            &NewDevice {
                hardware_id: "http-device-hardware".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO device_ingest_principals
             (principal_id, device_system_id, flow_class, profile, created_at)
             VALUES ('http-device', ?1, 'default', 'simple_bearer', 1)",
            [subject.as_bytes().as_slice()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO device_principal_scopes (principal_id, system_id)
             VALUES ('http-device', ?1)",
            [subject.as_bytes().as_slice()],
        )?;
        let hash = Sha256::digest(secret.as_bytes());
        conn.execute(
            "INSERT INTO device_credentials
             (credential_id, principal_id, token_hash, auth_epoch, state, issued_at, issue_reason)
             VALUES ('route-credential', 'http-device', ?1,
               (SELECT auth_epoch FROM auth_state WHERE id=1), 'current', 1, 'manual_issue')",
            [hash.as_slice()],
        )?;
        Ok(())
    })
    .unwrap();
    secret
}

fn build_service(
    db: iotkit_core_storage::DbHandle,
    policy: Arc<dyn RegistryPolicy>,
    config: HttpIngestConfig,
    hooks: HttpIngestHooks,
    secret: String,
    clock: ManualMonotonicClock,
) -> (
    iotkit_core_storage::DbHandle,
    Collector,
    String,
    HttpIngestService<ManualMonotonicClock>,
) {
    let (collector, issuer, _handle) = Collector::spawn_device_composed(db.clone(), policy, 8);
    let service = HttpIngestService::new_with_hooks(
        db.clone(),
        collector.clone(),
        issuer,
        config,
        clock,
        hooks,
    )
    .unwrap();
    (db, collector, secret, service)
}

async fn loopback_stream_pair() -> (tokio::net::TcpStream, crate::AcceptedStream) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback bind for HTTP connection-boundary test");
    let address = listener.local_addr().unwrap();
    let (client, accepted) =
        tokio::join!(tokio::net::TcpStream::connect(address), listener.accept());
    let client = client.expect("loopback client connect");
    let (server, _) = accepted.expect("loopback server accept");
    (client, crate::AcceptedStream::PrivatePlaintext(server))
}

struct PausedTimeAutoAdvanceGuard {
    release: Option<std::sync::mpsc::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl PausedTimeAutoAdvanceGuard {
    async fn start() -> Self {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let task = tokio::task::spawn_blocking(move || {
            started_tx
                .send(())
                .expect("paused-time guard starter must still be waiting");
            let _ = release_rx.recv();
        });
        started_rx
            .await
            .expect("paused-time guard must start before timed connections");
        Self {
            release: Some(release_tx),
            task,
        }
    }

    fn release(&mut self) {
        if let Some(release) = self.release.take() {
            release
                .send(())
                .expect("paused-time guard task must remain alive until release");
        }
    }

    async fn finish(mut self) {
        self.release();
        self.task.await.expect("paused-time guard task must finish");
    }
}

async fn wait_for_connection_permits(
    service: &HttpIngestService<ManualMonotonicClock>,
    expected: usize,
) {
    for _ in 0..100 {
        if service.shared.connections.available_permits() == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!(
        "connection permit barrier was not reached: expected {expected}, observed {}",
        service.shared.connections.available_permits()
    );
}

async fn read_http_head(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut response = Vec::new();
    while !response.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk).await.expect("read HTTP response");
        assert_ne!(read, 0, "connection closed before an HTTP response head");
        response.extend_from_slice(&chunk[..read]);
    }
    response
}

async fn prove_subsequent_http_connection_enters(
    service: &HttpIngestService<ManualMonotonicClock>,
    observed_peer: std::net::SocketAddr,
) {
    let (mut client, server) = loopback_stream_pair().await;
    let permits_before = service.shared.connections.available_permits();
    assert_ne!(
        permits_before, 0,
        "a released connection permit is required"
    );
    let serving = tokio::spawn({
        let service = service.clone();
        async move { service.serve_connection(server, observed_peer).await }
    });
    wait_for_connection_permits(service, permits_before - 1).await;
    client
        .write_all(b"GET /api/v1/ingest HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n")
        .await
        .expect("write subsequent valid HTTP request");
    let mut response = Vec::new();
    client
        .read_to_end(&mut response)
        .await
        .expect("read subsequent HTTP response");
    assert!(
        response.starts_with(b"HTTP/1.1 405"),
        "subsequent valid HTTP connection must reach the service"
    );
    assert_eq!(serving.await.unwrap(), Ok(()));
}

fn file_fixture_with_hooks(
    config: HttpIngestConfig,
    hooks: HttpIngestHooks,
) -> (
    tempfile::TempDir,
    iotkit_core_storage::DbHandle,
    iotkit_core_storage::DbHandle,
    String,
    HttpIngestService<ManualMonotonicClock>,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reserved-linearization.db");
    let db = iotkit_core_storage::init_db(&path, &migrations()).unwrap();
    let secret = seed_route_credential(&db);
    let mutation_db = iotkit_core_storage::init_db(&path, &migrations()).unwrap();
    let (db, _collector, secret, service) = build_service(
        db,
        Arc::new(PermissiveRegistry),
        config,
        hooks,
        secret,
        ManualMonotonicClock::new(0),
    );
    (dir, db, mutation_db, secret, service)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoke_after_http_recheck_is_rejected_at_collector_serialization() {
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let hook_gate = Arc::clone(&gate);
    let hooks = HttpIngestHooks::default().with_before_collector_handoff(move || {
        entered_tx.send(()).unwrap();
        let (lock, ready) = &*hook_gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = ready.wait(released).unwrap();
        }
    });
    let (db, _collector, secret, service) = fixture_with_policy_and_hooks(
        Arc::new(PermissiveRegistry),
        HttpIngestConfig::for_test(),
        hooks,
    );
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 17));
    let request_task = tokio::spawn(async move {
        service
            .handle(
                peer,
                request("/api/v1/ingest", &secret, envelope("revoked-at-handoff")),
            )
            .await
    });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();

    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE device_credentials SET state='revoked', revoked_at=2,
             revoke_reason='operator_revoked' WHERE credential_id='route-credential'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    let (lock, ready) = &*gate;
    *lock.lock().unwrap() = true;
    ready.notify_all();

    let response = request_task.await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let state: (i64, i64) = db
        .with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |row| row.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!(state, (0, 0));
}

async fn authority_mutation_at_handoff(
    envelope_id: &'static str,
    mutate: fn(&rusqlite::Connection) -> rusqlite::Result<()>,
) {
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let hook_gate = Arc::clone(&gate);
    let hooks = HttpIngestHooks::default().with_before_collector_handoff(move || {
        entered_tx.send(()).unwrap();
        let (lock, ready) = &*hook_gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = ready.wait(released).unwrap();
        }
    });
    let (db, _collector, secret, service) = fixture_with_policy_and_hooks(
        Arc::new(PermissiveRegistry),
        HttpIngestConfig::for_test(),
        hooks,
    );
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 19));
    let task = tokio::spawn(async move {
        service
            .handle(
                peer,
                request("/api/v1/ingest", &secret, envelope(envelope_id)),
            )
            .await
    });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();
    db.with_conn_sync(|conn| {
        let tx = conn.unchecked_transaction()?;
        mutate(&tx)?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let (lock, ready) = &*gate;
    *lock.lock().unwrap() = true;
    ready.notify_all();

    assert_eq!(task.await.unwrap().status(), StatusCode::UNAUTHORIZED);
    let state: (i64, i64) = db
        .with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |row| row.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!(state, (0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reissue_reset_and_restore_generation_races_reject_stale_proofs() {
    authority_mutation_at_handoff("reissue-race", |conn| {
        conn.execute(
            "INSERT INTO device_credentials
             (credential_id, principal_id, token_hash, auth_epoch, state, issued_at,
              proven_at, issue_reason)
             VALUES ('pending-race', 'http-device', ?1,
               (SELECT auth_epoch FROM auth_state WHERE id=1), 'pending', 2, 2,
               'credential_reissue')",
            [vec![7_u8; 32]],
        )?;
        conn.execute(
            "UPDATE device_credentials SET state='revoked', revoked_at=3,
             revoke_reason='credential_confirmed' WHERE credential_id='route-credential'",
            [],
        )?;
        conn.execute(
            "UPDATE device_credentials SET state='current', confirmed_at=3
             WHERE credential_id='pending-race'",
            [],
        )?;
        Ok(())
    })
    .await;
    authority_mutation_at_handoff("reset-race", |conn| {
        conn.execute(
            "UPDATE auth_state SET auth_generation=auth_generation+1 WHERE id=1",
            [],
        )?;
        Ok(())
    })
    .await;
    authority_mutation_at_handoff("restore-race", |conn| {
        conn.execute("DELETE FROM device_credentials", [])?;
        conn.execute(
            "UPDATE auth_state SET auth_epoch='restored-race-epoch',
             auth_generation=auth_generation+1 WHERE id=1",
            [],
        )?;
        Ok(())
    })
    .await;
}

struct BlockingOncePolicy {
    entered: std::sync::mpsc::SyncSender<()>,
    gate: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
}

impl RegistryPolicy for BlockingOncePolicy {
    fn evaluate(
        &self,
        _conn: &rusqlite::Connection,
        _system_id: &iotkit_core_ledger::SystemId,
        item: &iotkit_ingest_contract::ReadingItem,
    ) -> Result<RegistryVerdict, String> {
        let _ = self.entered.try_send(());
        let (lock, ready) = &*self.gate;
        let mut released = lock.lock().expect("blocking policy mutex poisoned");
        while !*released {
            released = ready
                .wait(released)
                .expect("blocking policy mutex poisoned");
        }
        Ok(RegistryVerdict::Accept {
            resolved_key: item.measurement_key.clone(),
            channel_index: -1,
            quarantine: None,
        })
    }
}

struct FailingPolicy;

impl RegistryPolicy for FailingPolicy {
    fn evaluate(
        &self,
        _conn: &rusqlite::Connection,
        _system_id: &iotkit_core_ledger::SystemId,
        _item: &iotkit_ingest_contract::ReadingItem,
    ) -> Result<RegistryVerdict, String> {
        Err("injected commit-path storage failure".into())
    }
}

fn envelope(id: &str) -> String {
    envelope_for("http-device", id)
}

fn envelope_for(source: &str, id: &str) -> String {
    serde_json::json!({
        "envelope_id": id,
        "source": source,
        "items": [{
            "measurement_key": "temperature_c",
            "values": [21.5],
            "time_source": "gateway"
        }]
    })
    .to_string()
}

fn add_second_credential(db: &iotkit_core_storage::DbHandle, secret: &str) {
    db.with_conn_sync(|conn| {
        let second_subject = iotkit_core_ledger::insert_device(
            conn,
            &NewDevice {
                hardware_id: "http-device-hardware-2".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO device_ingest_principals
             (principal_id, device_system_id, flow_class, profile, created_at)
             VALUES ('http-device-2', ?1, 'default', 'simple_bearer', 1)",
            [second_subject.as_bytes().as_slice()],
        )?;
        conn.execute(
            "INSERT INTO device_principal_scopes (principal_id, system_id)
             VALUES ('http-device-2', ?1)",
            [second_subject.as_bytes().as_slice()],
        )?;
        let hash = Sha256::digest(secret.as_bytes());
        conn.execute(
            "INSERT INTO device_credentials
             (credential_id, principal_id, token_hash, auth_epoch, state, issued_at, issue_reason)
             VALUES ('route-credential-2', 'http-device-2', ?1,
               (SELECT auth_epoch FROM auth_state WHERE id=1), 'current', 1, 'manual_issue')",
            [hash.as_slice()],
        )?;
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn stale_cache_entry_cannot_consume_the_reserved_auth_lane() {
    let mut config = HttpIngestConfig::for_test();
    config.admission = config
        .admission
        .with_auth_work_limit(1, 3)
        .with_initial_auth_tokens(3)
        .with_reserved_auth_work_limit(1, 1, 1);
    let (db, _collector, stale_secret, service) =
        fixture_with_policy(Arc::new(PermissiveRegistry), config);
    let current_secret = format!("{stale_secret}-current-peer");
    db.with_conn_sync(|conn| {
        let second_subject = iotkit_core_ledger::insert_device(
            conn,
            &NewDevice {
                hardware_id: "http-device-hardware-2".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO device_ingest_principals
             (principal_id, device_system_id, flow_class, profile, created_at)
             VALUES ('http-device-2', ?1, 'default', 'simple_bearer', 1)",
            [second_subject.as_bytes().as_slice()],
        )?;
        conn.execute(
            "INSERT INTO device_principal_scopes (principal_id, system_id)
             VALUES ('http-device-2', ?1)",
            [second_subject.as_bytes().as_slice()],
        )?;
        let hash = Sha256::digest(current_secret.as_bytes());
        conn.execute(
            "INSERT INTO device_credentials
             (credential_id, principal_id, token_hash, auth_epoch, state, issued_at, issue_reason)
             VALUES ('route-credential-2', 'http-device-2', ?1,
               (SELECT auth_epoch FROM auth_state WHERE id=1), 'current', 1, 'manual_issue')",
            [hash.as_slice()],
        )?;
        Ok(())
    })
    .unwrap();
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 18));

    assert_eq!(
        service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &stale_secret,
                    envelope("stale-cache-prime")
                ),
            )
            .await
            .status(),
        StatusCode::OK
    );
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE device_credentials SET state='revoked', revoked_at=2,
             revoke_reason='operator_revoked' WHERE credential_id='route-credential'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &current_secret,
                    envelope_for("http-device-2", "current-cache-prime"),
                ),
            )
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        service
            .handle(peer, request("/api/v1/ingest", "ikd_invalid", "{".into()))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let stale = service
        .handle(
            peer,
            request("/api/v1/ingest", &stale_secret, envelope("stale-cache-use")),
        )
        .await;
    assert_eq!(stale.status(), StatusCode::TOO_MANY_REQUESTS);
    let protected = service
        .handle(
            peer,
            request(
                "/api/v1/ingest",
                &current_secret,
                envelope_for("http-device-2", "current-reserved-use"),
            ),
        )
        .await;
    assert_eq!(protected.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_body_releases_reserved_worker_without_refunding_auth_work() {
    let mut config = HttpIngestConfig::for_test();
    config.admission = config
        .admission
        .with_auth_workers(1)
        .with_auth_work_limit(1, 3)
        .with_initial_auth_tokens(3)
        .with_reserved_auth_work_limit(1, 2, 2);
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    let first_secret = seed_route_credential(&db);
    let second_secret = format!("{first_secret}-second-slow-body");
    add_second_credential(&db, &second_secret);
    let (db, _collector, first_secret, service) = build_service(
        db,
        Arc::new(PermissiveRegistry),
        config,
        HttpIngestHooks::default(),
        first_secret,
        ManualMonotonicClock::new(0),
    );
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 33));

    assert_eq!(
        service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &first_secret,
                    envelope("slow-body-first-prime"),
                ),
            )
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &second_secret,
                    envelope_for("http-device-2", "slow-body-second-prime"),
                ),
            )
            .await
            .status(),
        StatusCode::OK
    );
    let _general_worker = service
        .shared
        .admission
        .try_begin_auth(peer, false)
        .expect("the test must occupy the only general auth worker");

    let (body_entered_tx, body_entered_rx) = std::sync::mpsc::sync_channel(1);
    let stalled_request = Request::builder()
        .method("POST")
        .uri("/api/v1/ingest")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {first_secret}"))
        .body(Body::new(SignalThenPending {
            entered: Some(body_entered_tx),
        }))
        .unwrap();
    let stalled_service = service.clone();
    let stalled = tokio::spawn(async move { stalled_service.handle(peer, stalled_request).await });
    tokio::task::spawn_blocking(move || body_entered_rx.recv().unwrap())
        .await
        .unwrap();

    let while_stalled = service.admission_snapshot();
    assert_eq!(
        while_stalled.reserved_auth_tokens_milli, 1000,
        "the first request's reserved auth-work token remains consumed",
    );
    assert_eq!(
        while_stalled.reserved_auth_workers_available, 1,
        "the reserved worker must be returned before body polling",
    );

    let second = service
        .handle(
            peer,
            request(
                "/api/v1/ingest",
                &second_secret,
                envelope_for("http-device-2", "slow-body-second-use"),
            ),
        )
        .await;
    assert_eq!(second.status(), StatusCode::OK);
    let after_second = service.admission_snapshot();
    assert_eq!(after_second.reserved_auth_tokens_milli, 0);
    assert_eq!(after_second.reserved_auth_workers_available, 1);

    stalled.abort();
    assert!(stalled.await.unwrap_err().is_cancelled());
    drop(db);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mutation_winning_before_reserved_acquire_preserves_exact_reserved_state() {
    let (general_held_tx, general_held_rx) = std::sync::mpsc::sync_channel(1);
    let general_gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let collector_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hook_general_gate = Arc::clone(&general_gate);
    let hook_collector_calls = Arc::clone(&collector_calls);

    let (boundary_entered_tx, boundary_entered_rx) = std::sync::mpsc::sync_channel(1);
    let boundary_gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let boundary_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_boundary_gate = Arc::clone(&boundary_gate);
    let hook_boundary_called = Arc::clone(&boundary_called);

    let hooks = HttpIngestHooks::default()
        .with_before_collector_handoff(move || {
            if hook_collector_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                general_held_tx.send(()).unwrap();
                wait_for_gate(&hook_general_gate);
            }
        })
        .with_before_cached_reserved_admission(move || {
            if !hook_boundary_called.swap(true, std::sync::atomic::Ordering::SeqCst) {
                boundary_entered_tx.send(()).unwrap();
                wait_for_gate(&hook_boundary_gate);
            }
        });
    let mut config = HttpIngestConfig::for_test();
    config.admission = config
        .admission
        .with_auth_workers(1)
        .with_auth_work_limit(1, 3)
        .with_initial_auth_tokens(3)
        .with_reserved_auth_work_limit(1, 1, 1);
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    let stale_secret = seed_route_credential(&db);
    let clock = ManualMonotonicClock::new(0);
    let (db, _collector, stale_secret, service) = build_service(
        db,
        Arc::new(PermissiveRegistry),
        config,
        hooks,
        stale_secret,
        clock.clone(),
    );
    let valid_secret = format!("{stale_secret}-valid-reserved");
    add_second_credential(&db, &valid_secret);
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 31));

    assert_eq!(
        service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &stale_secret,
                    envelope("mutation-first-stale-prime"),
                ),
            )
            .await
            .status(),
        StatusCode::OK
    );

    let held_service = service.clone();
    let held_secret = valid_secret.clone();
    let held = tokio::spawn(async move {
        held_service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &held_secret,
                    envelope_for("http-device-2", "mutation-first-general-held"),
                ),
            )
            .await
    });
    tokio::task::spawn_blocking(move || general_held_rx.recv().unwrap())
        .await
        .unwrap();
    let general_worker = service
        .shared
        .admission
        .try_begin_auth(peer, false)
        .expect("the test must occupy the only general auth worker");
    let reserved_before = service.admission_snapshot();
    assert_eq!(reserved_before.reserved_auth_tokens_milli, 1000);
    assert_eq!(reserved_before.reserved_auth_workers_available, 1);

    let stale_service = service.clone();
    let stale_for_task = stale_secret.clone();
    let stale = tokio::spawn(async move {
        stale_service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &stale_for_task,
                    envelope("mutation-first-stale-use"),
                ),
            )
            .await
    });
    tokio::task::spawn_blocking(move || boundary_entered_rx.recv().unwrap())
        .await
        .unwrap();
    db.with_conn_sync(|conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE device_credentials SET state='revoked', revoked_at=2,
             revoke_reason='operator_revoked' WHERE credential_id='route-credential'",
            [],
        )?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    release_gate(&boundary_gate);

    let stale_response = stale.await.unwrap();
    let reserved_after_stale = service.admission_snapshot();
    let stale_cache_evicted = !service.auth_cache_contains(&stale_secret);
    release_gate(&general_gate);
    let held_status = held.await.unwrap().status();
    drop(general_worker);

    clock.advance_ms(1000);
    let valid_refresh = service
        .handle(
            peer,
            request(
                "/api/v1/ingest",
                &valid_secret,
                envelope_for("http-device-2", "mutation-first-valid-refresh"),
            ),
        )
        .await;

    let valid_response = service
        .handle(
            peer,
            request(
                "/api/v1/ingest",
                &valid_secret,
                envelope_for("http-device-2", "mutation-first-valid-reserved"),
            ),
        )
        .await;
    let reserved_after_valid = service.admission_snapshot();

    assert_eq!(
        stale_response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "stale cache must be evicted and retry only the non-reserved path",
    );
    assert_eq!(reserved_after_stale, reserved_before);
    assert!(stale_cache_evicted);
    assert_eq!(held_status, StatusCode::UNAUTHORIZED);
    assert_eq!(valid_refresh.status(), StatusCode::OK);
    assert_eq!(valid_response.status(), StatusCode::OK);
    assert_eq!(reserved_after_valid.reserved_auth_tokens_milli, 0);
    assert_eq!(reserved_after_valid.reserved_auth_workers_available, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reserved_acquire_wins_before_mutation_and_collector_rejects_stale_proof() {
    let (general_held_tx, general_held_rx) = std::sync::mpsc::sync_channel(1);
    let general_gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let (custody_entered_tx, custody_entered_rx) = std::sync::mpsc::sync_channel(1);
    let allow_mutation = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let mutation_committed = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let collector_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hook_general_gate = Arc::clone(&general_gate);
    let hook_allow_mutation = Arc::clone(&allow_mutation);
    let hook_mutation_committed = Arc::clone(&mutation_committed);
    let hook_collector_calls = Arc::clone(&collector_calls);

    let (acquired_tx, acquired_rx) = std::sync::mpsc::sync_channel(1);
    let acquired_gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let acquired_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_acquired_gate = Arc::clone(&acquired_gate);
    let hook_acquired_called = Arc::clone(&acquired_called);

    let hooks = HttpIngestHooks::default()
        .with_after_cached_reserved_admission(move || {
            if !hook_acquired_called.swap(true, std::sync::atomic::Ordering::SeqCst) {
                acquired_tx.send(()).unwrap();
                wait_for_gate(&hook_acquired_gate);
            }
        })
        .with_before_collector_handoff(move || {
            match hook_collector_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
                1 => {
                    general_held_tx.send(()).unwrap();
                    wait_for_gate(&hook_general_gate);
                }
                2 => {
                    custody_entered_tx.send(()).unwrap();
                    release_gate(&hook_allow_mutation);
                    wait_for_gate(&hook_mutation_committed);
                }
                _ => {}
            }
        });
    let mut config = HttpIngestConfig::for_test();
    config.admission = config
        .admission
        .with_auth_workers(1)
        .with_auth_work_limit(1, 3)
        .with_initial_auth_tokens(3)
        .with_reserved_auth_work_limit(1, 1, 1);
    let (_dir, db, mutation_db, stale_secret, service) = file_fixture_with_hooks(config, hooks);
    let valid_secret = format!("{stale_secret}-valid-reserved");
    add_second_credential(&db, &valid_secret);
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 32));

    assert_eq!(
        service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &stale_secret,
                    envelope("acquire-first-stale-prime"),
                ),
            )
            .await
            .status(),
        StatusCode::OK
    );
    let held_service = service.clone();
    let held_secret = valid_secret.clone();
    let held = tokio::spawn(async move {
        held_service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &held_secret,
                    envelope_for("http-device-2", "acquire-first-general-held"),
                ),
            )
            .await
    });
    tokio::task::spawn_blocking(move || general_held_rx.recv().unwrap())
        .await
        .unwrap();
    let _general_worker = service
        .shared
        .admission
        .try_begin_auth(peer, false)
        .expect("the test must occupy the only general auth worker");

    let stale_service = service.clone();
    let stale_for_task = stale_secret.clone();
    let stale = tokio::spawn(async move {
        stale_service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &stale_for_task,
                    envelope("acquire-first-stale-use"),
                ),
            )
            .await
    });
    tokio::task::spawn_blocking(move || acquired_rx.recv().unwrap())
        .await
        .unwrap();
    let acquired_snapshot = service.admission_snapshot();
    assert_eq!(acquired_snapshot.reserved_auth_tokens_milli, 0);
    assert_eq!(acquired_snapshot.reserved_auth_workers_available, 0);

    let (event_tx, event_rx) = std::sync::mpsc::sync_channel(4);
    *BUSY_BARRIER.lock().unwrap() = Some(BusyBarrier {
        events: event_tx.clone(),
        allow_retry: Arc::clone(&allow_mutation),
    });
    let mutator_committed = Arc::clone(&mutation_committed);
    let mutation = tokio::task::spawn_blocking(move || {
        mutation_db
            .with_conn_sync(|conn| {
                conn.busy_handler(Some(reserved_writer_busy_handler))?;
                let tx = rusqlite::Transaction::new_unchecked(
                    conn,
                    rusqlite::TransactionBehavior::Immediate,
                )?;
                tx.execute(
                    "UPDATE device_credentials SET state='revoked', revoked_at=3,
                     revoke_reason='operator_revoked' WHERE credential_id='route-credential'",
                    [],
                )?;
                tx.commit()?;
                Ok(())
            })
            .unwrap();
        let _ = event_tx.send(MutationEvent::Committed);
        release_gate(&mutator_committed);
    });
    let first_event = tokio::task::spawn_blocking(move || event_rx.recv().unwrap())
        .await
        .unwrap();
    release_gate(&acquired_gate);

    let stale_response = stale.await.unwrap();
    release_gate(&general_gate);
    assert_eq!(held.await.unwrap().status(), StatusCode::UNAUTHORIZED);
    mutation.await.unwrap();
    *BUSY_BARRIER.lock().unwrap() = None;

    assert_eq!(
        first_event,
        MutationEvent::Contended,
        "authority writer must contend until the reserved admission boundary releases",
    );
    assert!(custody_entered_rx.try_recv().is_ok());
    assert_eq!(stale_response.status(), StatusCode::UNAUTHORIZED);
    let target_claims: i64 = db
        .with_conn_sync(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM ingest_dedup
                 WHERE sender_id='http-device' AND envelope_id='acquire-first-stale-use'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(target_claims, 0, "collector must not grant stale custody");
}

#[tokio::test]
async fn concurrent_same_envelope_has_one_commit_and_one_duplicate() {
    let (db, _collector, secret, service) = fixture();
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));
    let first_service = service.clone();
    let second_service = service.clone();
    let first = first_service.handle(
        peer,
        request("/api/v1/ingest", &secret, envelope("concurrent-same")),
    );
    let second = second_service.handle(
        peer,
        request("/api/v1/ingest", &secret, envelope("concurrent-same")),
    );
    let (first, second) = tokio::join!(first, second);
    let first: EnvelopeAck =
        serde_json::from_slice(&first.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let second: EnvelopeAck =
        serde_json::from_slice(&second.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(
        (matches!(first.status, AckStatus::Accepted { .. })
            && second.status == AckStatus::Duplicate)
            || (matches!(second.status, AckStatus::Accepted { .. })
                && first.status == AckStatus::Duplicate)
    );
    let readings: i64 = db
        .with_conn_sync(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?)
        })
        .unwrap();
    assert_eq!(readings, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_http_collector_lane_returns_503_without_ack() {
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let hook_gate = Arc::clone(&gate);
    let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_called = Arc::clone(&called);
    let hooks = HttpIngestHooks::default().with_after_queue_acquired(move || {
        if hook_called.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        entered_tx.send(()).unwrap();
        let (lock, ready) = &*hook_gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = ready.wait(released).unwrap();
        }
    });
    let mut config = HttpIngestConfig::for_test();
    config.collector_queue_slots = 1;
    let (_db, _collector, secret, service) =
        fixture_with_policy_and_hooks(Arc::new(PermissiveRegistry), config, hooks);
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 21));
    let first_service = service.clone();
    let first_secret = secret.clone();
    let first = tokio::spawn(async move {
        first_service
            .handle(
                peer,
                request("/api/v1/ingest", &first_secret, envelope("queue-held")),
            )
            .await
    });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();
    let full = service
        .handle(
            peer,
            request("/api/v1/ingest", &secret, envelope("queue-full")),
        )
        .await;
    assert_eq!(full.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        full.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
    let (lock, ready) = &*gate;
    *lock.lock().unwrap() = true;
    ready.notify_all();
    assert_eq!(first.await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn storage_failure_maps_to_503_without_ack_or_custody() {
    let (db, _collector, secret, service) =
        fixture_with_policy(Arc::new(FailingPolicy), HttpIngestConfig::for_test());
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 22));
    let failed = service
        .handle(
            peer,
            request("/api/v1/ingest", &secret, envelope("storage-failure")),
        )
        .await;
    assert_eq!(failed.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        failed
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
    let state: (i64, i64) = db
        .with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |row| row.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!(state, (0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_before_enqueue_releases_capacity_without_custody() {
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let hook_gate = Arc::clone(&gate);
    let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_called = Arc::clone(&called);
    let hooks = HttpIngestHooks::default().with_before_collector_handoff(move || {
        if hook_called.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        entered_tx.send(()).unwrap();
        let (lock, ready) = &*hook_gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = ready.wait(released).unwrap();
        }
    });
    let (_db, _collector, secret, service) = fixture_with_policy_and_hooks(
        Arc::new(PermissiveRegistry),
        HttpIngestConfig::for_test(),
        hooks,
    );
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 23));
    let cancelled_service = service.clone();
    let cancelled_secret = secret.clone();
    let cancelled = tokio::spawn(async move {
        cancelled_service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &cancelled_secret,
                    envelope("cancel-before-enqueue"),
                ),
            )
            .await
    });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();
    cancelled.abort();
    let (lock, ready) = &*gate;
    *lock.lock().unwrap() = true;
    ready.notify_all();
    assert!(cancelled.await.unwrap_err().is_cancelled());

    let retry = service
        .handle(
            peer,
            request("/api/v1/ingest", &secret, envelope("cancel-before-enqueue")),
        )
        .await;
    let ack: EnvelopeAck =
        serde_json::from_slice(&retry.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(matches!(ack.status, AckStatus::Accepted { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_after_enqueue_before_commit_finishes_detached() {
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let policy = Arc::new(BlockingOncePolicy {
        entered: entered_tx,
        gate: Arc::clone(&gate),
    });
    let (_db, _collector, secret, service) =
        fixture_with_policy(policy, HttpIngestConfig::for_test());
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 24));
    let cancelled_service = service.clone();
    let cancelled_secret = secret.clone();
    let cancelled = tokio::spawn(async move {
        cancelled_service
            .handle(
                peer,
                request(
                    "/api/v1/ingest",
                    &cancelled_secret,
                    envelope("cancel-after-enqueue"),
                ),
            )
            .await
    });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();
    cancelled.abort();
    let (lock, ready) = &*gate;
    *lock.lock().unwrap() = true;
    ready.notify_all();
    assert!(cancelled.await.unwrap_err().is_cancelled());

    let retry = service
        .handle(
            peer,
            request("/api/v1/ingest", &secret, envelope("cancel-after-enqueue")),
        )
        .await;
    let ack: EnvelopeAck =
        serde_json::from_slice(&retry.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(ack.status, AckStatus::Duplicate);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_after_commit_before_serialization_retries_duplicate() {
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let hook_gate = Arc::clone(&gate);
    let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_called = Arc::clone(&called);
    let hooks = HttpIngestHooks::default().with_after_collector_result(move || {
        if hook_called.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        entered_tx.send(()).unwrap();
        let (lock, ready) = &*hook_gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = ready.wait(released).unwrap();
        }
    });
    let (_db, _collector, secret, service) = fixture_with_policy_and_hooks(
        Arc::new(PermissiveRegistry),
        HttpIngestConfig::for_test(),
        hooks,
    );
    assert_lost_response_retries_duplicate(
        service,
        secret,
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 25)),
        "cancel-after-commit",
        entered_rx,
        gate,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_after_response_serialization_retries_duplicate() {
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let hook_gate = Arc::clone(&gate);
    let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_called = Arc::clone(&called);
    let hooks = HttpIngestHooks::default().with_after_response_serialization(move || {
        if hook_called.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        entered_tx.send(()).unwrap();
        let (lock, ready) = &*hook_gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = ready.wait(released).unwrap();
        }
    });
    let (_db, _collector, secret, service) = fixture_with_policy_and_hooks(
        Arc::new(PermissiveRegistry),
        HttpIngestConfig::for_test(),
        hooks,
    );
    assert_lost_response_retries_duplicate(
        service,
        secret,
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 26)),
        "cancel-after-serialization",
        entered_rx,
        gate,
    )
    .await;
}

async fn assert_lost_response_retries_duplicate(
    service: HttpIngestService<ManualMonotonicClock>,
    secret: String,
    peer: IpAddr,
    envelope_id: &'static str,
    entered_rx: std::sync::mpsc::Receiver<()>,
    gate: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
) {
    let cancelled_service = service.clone();
    let cancelled_secret = secret.clone();
    let cancelled = tokio::spawn(async move {
        cancelled_service
            .handle(
                peer,
                request("/api/v1/ingest", &cancelled_secret, envelope(envelope_id)),
            )
            .await
    });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();
    cancelled.abort();
    let (lock, ready) = &*gate;
    *lock.lock().unwrap() = true;
    ready.notify_all();
    assert!(cancelled.await.unwrap_err().is_cancelled());

    let retry = service
        .handle(
            peer,
            request("/api/v1/ingest", &secret, envelope(envelope_id)),
        )
        .await;
    let ack: EnvelopeAck =
        serde_json::from_slice(&retry.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(ack.status, AckStatus::Duplicate);
}

fn request(path: &str, bearer: &str, body: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-length", body.len())
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn routes_map_auth_commit_duplicate_and_validation_without_custody_confusion() {
    let (db, _collector, secret, service) = fixture();
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));

    let unauthorized = service
        .handle(
            peer,
            request("/api/v1/ingest", &format!("{secret}x"), "{".repeat(1000)),
        )
        .await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let accepted = service
        .handle(
            peer,
            request("/api/v1/ingest", &secret, envelope("route-1")),
        )
        .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let ack: EnvelopeAck =
        serde_json::from_slice(&accepted.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(matches!(ack.status, AckStatus::Accepted { .. }));

    let duplicate = service
        .handle(
            peer,
            request("/api/v1/ingest", &secret, envelope("route-1")),
        )
        .await;
    let duplicate: EnvelopeAck =
        serde_json::from_slice(&duplicate.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(duplicate.status, AckStatus::Duplicate);

    let before: (i64, i64) = db
        .with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |row| row.get(0))?,
            ))
        })
        .unwrap();
    let validation = service
        .handle(
            peer,
            request(
                "/api/v1/ingest/validate",
                &secret,
                envelope("validate-route-1"),
            ),
        )
        .await;
    assert_eq!(validation.status(), StatusCode::OK);
    let report: ValidationReport =
        serde_json::from_slice(&validation.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert!(report.valid);
    let after: (i64, i64) = db
        .with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |row| row.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn validation_does_not_prove_pending_but_later_ingest_does() {
    let (db, _collector, secret, service) = fixture();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE device_credentials SET state='pending' WHERE credential_id='route-credential'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 11));

    let validation = service
        .handle(
            peer,
            request(
                "/api/v1/ingest/validate",
                &secret,
                envelope("pending-validate"),
            ),
        )
        .await;
    assert_eq!(validation.status(), StatusCode::OK);
    let after_validation: Option<i64> = db
        .with_conn_sync(|conn| {
            Ok(conn.query_row(
                "SELECT proven_at FROM device_credentials WHERE credential_id='route-credential'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(after_validation, None);

    let ingest = service
        .handle(
            peer,
            request("/api/v1/ingest", &secret, envelope("pending-ingest")),
        )
        .await;
    assert_eq!(ingest.status(), StatusCode::OK);
    let after_ingest: Option<i64> = db
        .with_conn_sync(|conn| {
            Ok(conn.query_row(
                "SELECT proven_at FROM device_credentials WHERE credential_id='route-credential'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert!(after_ingest.is_some());
}

#[tokio::test]
async fn cached_auth_is_rejected_after_committed_device_generation_change() {
    let (db, _collector, secret, service) = fixture();
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 12));
    assert_eq!(
        service
            .handle(
                peer,
                request("/api/v1/ingest", &secret, envelope("cache-1"))
            )
            .await
            .status(),
        StatusCode::OK
    );
    db.with_conn_sync(|conn| {
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE device_credentials SET state='revoked', revoked_at=2,
               revoke_reason='operator_revoked' WHERE credential_id='route-credential'",
            [],
        )?;
        tx.execute(
            "UPDATE auth_state SET device_credential_generation=device_credential_generation+1
             WHERE id=1",
            [],
        )?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();

    let rejected = service
        .handle(
            peer,
            request("/api/v1/ingest", &secret, envelope("cache-2")),
        )
        .await;
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    assert!(
        rejected
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
}

#[tokio::test]
async fn source_mismatch_is_terminal_ack_but_untrusted_absolute_time_is_503_without_ack() {
    let (_db, _collector, secret, service) = fixture();
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 13));
    let forged = serde_json::json!({
        "envelope_id": "forged-source",
        "source": "somebody-else",
        "items": [{"measurement_key":"temperature_c","values":[1.0],"time_source":"gateway"}]
    })
    .to_string();
    let result = service
        .handle(peer, request("/api/v1/ingest", &secret, forged))
        .await;
    assert_eq!(result.status(), StatusCode::OK);
    let ack: EnvelopeAck =
        serde_json::from_slice(&result.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(matches!(ack.status, AckStatus::Rejected { .. }));

    let absolute = serde_json::json!({
        "envelope_id": "absolute-untrusted",
        "source": "http-device",
        "items": [{
            "measurement_key":"temperature_c", "values":[1.0],
            "device_time_ms":1000, "time_source":"device_rtc"
        }]
    })
    .to_string();
    let unavailable = service
        .handle(peer, request("/api/v1/ingest", &secret, absolute))
        .await;
    assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        unavailable
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
}

#[tokio::test]
async fn malformed_oversize_and_encoding_fail_without_an_ingest_ack() {
    let (_db, _collector, secret, service) = fixture();
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 14));
    let malformed = service
        .handle(peer, request("/api/v1/ingest", &secret, "{".into()))
        .await;
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

    let oversized_body = "x".repeat(64 * 1024 + 1);
    let oversized = service
        .handle(peer, request("/api/v1/ingest", &secret, oversized_body))
        .await;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let mut encoded = request("/api/v1/ingest", &secret, envelope("encoded"));
    encoded
        .headers_mut()
        .insert("content-encoding", "gzip".parse().unwrap());
    let encoded = service.handle(peer, encoded).await;
    assert_eq!(encoded.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let too_many = serde_json::json!({
        "envelope_id":"too-many-items",
        "source":"http-device",
        "items": (0..257).map(|_| serde_json::json!({
            "measurement_key":"temperature_c", "values":[1.0], "time_source":"gateway"
        })).collect::<Vec<_>>()
    })
    .to_string();
    let too_many = service
        .handle(peer, request("/api/v1/ingest", &secret, too_many))
        .await;
    assert_eq!(too_many.status(), StatusCode::OK);
    let ack: EnvelopeAck =
        serde_json::from_slice(&too_many.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(matches!(
        ack.status,
        AckStatus::Rejected {
            reason_code: iotkit_ingest_contract::ReasonCode::BatchTooLarge,
            ..
        }
    ));
}

#[tokio::test(start_paused = true)]
async fn pre_auth_does_not_consume_a_stalled_body_and_timeout_releases_capacity() {
    let (_db, _collector, secret, service) = fixture();
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 15));
    let pending = || Body::new(PendingAfterOne(Some(bytes::Bytes::from_static(b"{"))));
    let unauthorized = Request::builder()
        .method("POST")
        .uri("/api/v1/ingest")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {secret}x"))
        .body(pending())
        .unwrap();
    assert_eq!(
        service.handle(peer, unauthorized).await.status(),
        StatusCode::UNAUTHORIZED
    );

    let stalled = Request::builder()
        .method("POST")
        .uri("/api/v1/ingest")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {secret}"))
        .body(pending())
        .unwrap();
    assert_eq!(
        service.handle(peer, stalled).await.status(),
        StatusCode::REQUEST_TIMEOUT
    );
    assert_eq!(
        service
            .handle(
                peer,
                request("/api/v1/ingest", &secret, envelope("after-timeout"))
            )
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test(start_paused = true)]
async fn stalled_request_headers_release_every_connection_permit_after_read_timeout() {
    let mut config = HttpIngestConfig::for_test();
    config.concurrent_connections = 2;
    config.read_timeout = std::time::Duration::from_millis(10);
    let (_db, _collector, _secret, service) =
        fixture_with_policy(Arc::new(PermissiveRegistry), config);
    let observed_peer = "127.0.0.1:41001".parse().unwrap();

    // Finish all asynchronous loopback setup before arming either header timer. A running
    // blocking task inhibits Tokio's paused-clock auto-advance while the test waits for socket
    // I/O and the permit barrier below.
    let (idle_client, idle_server) = loopback_stream_pair().await;
    let (mut partial_client, partial_server) = loopback_stream_pair().await;
    let (_blocked_client, blocked_server) = loopback_stream_pair().await;
    let mut time_guard = PausedTimeAutoAdvanceGuard::start().await;
    let paused_at = tokio::time::Instant::now();

    let first = tokio::spawn({
        let service = service.clone();
        async move { service.serve_connection(idle_server, observed_peer).await }
    });
    let second = tokio::spawn({
        let service = service.clone();
        async move {
            service
                .serve_connection(partial_server, observed_peer)
                .await
        }
    });
    partial_client
        .write_all(b"POST /api/v1/ingest HTTP/1.1\r\nHost:")
        .await
        .unwrap();
    wait_for_connection_permits(&service, 0).await;
    assert_eq!(tokio::time::Instant::now(), paused_at);

    assert_eq!(
        service
            .serve_connection(blocked_server, observed_peer)
            .await,
        Err(super::ServeConnectionError::Busy)
    );
    assert_eq!(service.shared.connections.available_permits(), 0);
    assert_eq!(tokio::time::Instant::now(), paused_at);

    time_guard.release();
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    for serving in [first, second] {
        let result = tokio::time::timeout(std::time::Duration::from_millis(1), serving)
            .await
            .expect("stalled header connection must finish at read_timeout")
            .unwrap();
        assert_eq!(result, Err(super::ServeConnectionError::HeaderReadTimeout));
    }
    drop(idle_client);
    drop(partial_client);
    time_guard.finish().await;
    assert_eq!(service.shared.connections.available_permits(), 2);

    prove_subsequent_http_connection_enters(&service, observed_peer).await;
}

#[tokio::test(start_paused = true)]
async fn idle_keep_alive_releases_connection_permit_after_read_timeout() {
    let mut config = HttpIngestConfig::for_test();
    config.concurrent_connections = 1;
    config.read_timeout = std::time::Duration::from_millis(10);
    let (_db, _collector, _secret, service) =
        fixture_with_policy(Arc::new(PermissiveRegistry), config);
    let observed_peer = "127.0.0.1:41002".parse().unwrap();

    // Both sockets needed before the timeout boundary are ready before the timed connection
    // starts. The blocking guard keeps paused time fixed across response I/O and Busy proof.
    let (mut idle_client, idle_server) = loopback_stream_pair().await;
    let (_blocked_client, blocked_server) = loopback_stream_pair().await;
    let mut time_guard = PausedTimeAutoAdvanceGuard::start().await;
    let paused_at = tokio::time::Instant::now();

    let serving = tokio::spawn({
        let service = service.clone();
        async move { service.serve_connection(idle_server, observed_peer).await }
    });
    wait_for_connection_permits(&service, 0).await;
    idle_client
        .write_all(b"GET /api/v1/ingest HTTP/1.1\r\nHost: local\r\n\r\n")
        .await
        .unwrap();
    let response = read_http_head(&mut idle_client).await;
    assert!(response.starts_with(b"HTTP/1.1 405"));
    assert_eq!(service.shared.connections.available_permits(), 0);
    assert_eq!(tokio::time::Instant::now(), paused_at);

    assert_eq!(
        service
            .serve_connection(blocked_server, observed_peer)
            .await,
        Err(super::ServeConnectionError::Busy)
    );
    assert_eq!(service.shared.connections.available_permits(), 0);
    assert_eq!(tokio::time::Instant::now(), paused_at);

    time_guard.release();
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    let result = tokio::time::timeout(std::time::Duration::from_millis(1), serving)
        .await
        .expect("idle keep-alive connection must finish at read_timeout")
        .unwrap();
    assert_eq!(result, Err(super::ServeConnectionError::HeaderReadTimeout));
    drop(idle_client);
    time_guard.finish().await;
    assert_eq!(service.shared.connections.available_permits(), 1);

    prove_subsequent_http_connection_enters(&service, observed_peer).await;
}

#[tokio::test(start_paused = true)]
async fn timeout_after_collector_handoff_finishes_detached_and_retry_is_duplicate() {
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let policy = Arc::new(BlockingOncePolicy {
        entered: entered_tx,
        gate: Arc::clone(&gate),
    });
    let mut config = HttpIngestConfig::for_test();
    config.collector_timeout = std::time::Duration::from_millis(10);
    let (_db, _collector, secret, service) = fixture_with_policy(policy, config);
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 16));
    let first_service = service.clone();
    let first_secret = secret.clone();
    let first = tokio::spawn(async move {
        first_service
            .handle(
                peer,
                request("/api/v1/ingest", &first_secret, envelope("lost-response")),
            )
            .await
    });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    let timed_out = first.await.unwrap();
    assert_eq!(timed_out.status(), StatusCode::SERVICE_UNAVAILABLE);

    let (lock, ready) = &*gate;
    *lock.lock().unwrap() = true;
    ready.notify_all();

    let retry = service
        .handle(
            peer,
            request("/api/v1/ingest", &secret, envelope("lost-response")),
        )
        .await;
    assert_eq!(retry.status(), StatusCode::OK);
    let ack: EnvelopeAck =
        serde_json::from_slice(&retry.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(ack.status, AckStatus::Duplicate);
}
