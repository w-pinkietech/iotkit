use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_core_collector::{Collector, PermissiveRegistry};
use iotkit_core_ops::{
    Actor, ActorKind, DispatchRequest, DispatchResult, Tier, dispatch, dispatch_with_secret_dir,
    standard_catalog,
};
use iotkit_gateway::health::HealthState;
use iotkit_gateway::ingress::{IngressBindFuture, IngressComposition};
use iotkit_gateway::network_authority::{NetworkAuthorityError, require_network_authority};
use iotkit_ingest_http::{
    ExposureSnapshot, HttpIngestConfig, HttpIngestService, Listener, ListenerError,
    SystemMonotonicClock, ValidatedListenerConfig,
};
use rcgen::{CertificateParams, KeyPair, SanType};
use serde_json::json;
use serial_test::serial;

fn load_ingress_config(
    conn: &rusqlite::Connection,
) -> Result<iotkit_core_ops::IngressListenerConfig, iotkit_core_storage::StorageError> {
    iotkit_core_ops::load_ingress_listener_config(conn).map_err(|error| {
        iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
            Box::new(error),
        ))
    })
}

fn migrations() -> Vec<iotkit_core_storage::Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.sort_by_key(|migration| migration.version);
    all
}

fn service_migrations() -> Vec<iotkit_core_storage::Migration> {
    let mut all = migrations();
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    all.sort_by_key(|migration| migration.version);
    all
}

fn own(db: &iotkit_core_storage::DbHandle) {
    db.with_conn_sync(|conn| {
        let hash = iotkit_core_ops::hash_passphrase("test-passphrase-long-enough").unwrap();
        iotkit_core_ops::reset_passphrase_with_hash(conn, &hash, "local_cli").unwrap();
        Ok(())
    })
    .unwrap();
}

fn configure_private_ingress(
    db: &iotkit_core_storage::DbHandle,
    bind: SocketAddr,
    interface: &str,
    site_local_cidr: &str,
) {
    db.with_conn_sync(|conn| {
        iotkit_core_ops::dispatch(
            conn,
            iotkit_core_ops::standard_catalog(),
            iotkit_core_ops::DispatchRequest {
                op: "ingress.listener.configure".into(),
                params: json!({
                    "enabled": true,
                    "bind_addr": bind.to_string(),
                    "interface": interface,
                    "site_local_cidrs": [site_local_cidr],
                    "mode": "private_plaintext"
                }),
                dry_run: false,
                actor: iotkit_core_ops::Actor {
                    actor_id: "local_cli".into(),
                    actor_kind: iotkit_core_ops::ActorKind::LocalCli,
                    tier_ceiling: iotkit_core_ops::Tier::Construction,
                },
                source: Some("local_cli".into()),
                step_up_verified: true,
                clock_trust: None,
            },
        )
        .map_err(|error| {
            iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                Box::new(error),
            ))
        })?;
        Ok(())
    })
    .unwrap();
}

const TEST_SITE_INTERFACE: &str = "plan6-test-site";
const TEST_SITE_CIDR: &str = "10.42.0.0/24";
const TEST_SOCKET_IP: Ipv4Addr = Ipv4Addr::LOCALHOST;
const TEST_LOGICAL_BIND_A: Ipv4Addr = Ipv4Addr::new(10, 42, 0, 1);
const TEST_LOGICAL_BIND_B: Ipv4Addr = Ipv4Addr::new(10, 42, 0, 2);

fn logical_bind_a() -> SocketAddr {
    SocketAddr::new(TEST_LOGICAL_BIND_A.into(), 0)
}

fn logical_bind_b() -> SocketAddr {
    SocketAddr::new(TEST_LOGICAL_BIND_B.into(), 0)
}

#[derive(Clone, Default)]
struct DeterministicIngressComposition {
    bound: Arc<Mutex<Vec<(SocketAddr, SocketAddr)>>>,
}

impl DeterministicIngressComposition {
    fn bound_for(&self, logical: SocketAddr) -> Vec<SocketAddr> {
        self.bound
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(configured, actual)| (*configured == logical).then_some(*actual))
            .collect()
    }
}

impl IngressComposition for DeterministicIngressComposition {
    fn exposure(&self, interface: &str) -> Result<ExposureSnapshot, ListenerError> {
        if interface != TEST_SITE_INTERFACE {
            return Err(ListenerError::UnapprovedInterface);
        }
        ExposureSnapshot::from_inventory(
            TEST_SITE_INTERFACE,
            [
                IpAddr::V4(TEST_LOGICAL_BIND_A),
                IpAddr::V4(TEST_LOGICAL_BIND_B),
            ],
            false,
        )
    }

    fn bind(&self, validated: ValidatedListenerConfig) -> IngressBindFuture {
        let bound = self.bound.clone();
        Box::pin(async move {
            let configured = validated.bind_addr();
            let socket = std::net::TcpListener::bind(SocketAddr::new(TEST_SOCKET_IP.into(), 0))?;
            socket.set_nonblocking(true)?;
            let actual = socket.local_addr()?;
            let socket = tokio::net::TcpListener::from_std(socket)?;
            let listener = Listener::from_prebound_socket(validated, socket)?;
            bound.lock().unwrap().push((configured, actual));
            Ok(listener)
        })
    }
}

async fn wait_for_ingress_status(health: &Arc<Mutex<HealthState>>, expected: &'static str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if health.lock().unwrap().ingress.status == expected {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        let ingress = health.lock().unwrap().ingress.clone();
        panic!(
            "ingress supervisor did not reach {expected:?}: status={:?}, last_error={:?}, last_action={:?}, gate_reason={:?}",
            ingress.status, ingress.last_error, ingress.last_action, ingress.gate_reason
        )
    });
}

async fn wait_for_ingress_local_addr(health: &Arc<Mutex<HealthState>>) -> SocketAddr {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(address) = health.lock().unwrap().ingress.local_addr.clone() {
                return address
                    .parse()
                    .expect("health must report the actual supervised listener address");
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("supervisor must publish the actual :0 listener address")
}

async fn wait_for_ingress_local_addr_other_than(
    health: &Arc<Mutex<HealthState>>,
    previous: SocketAddr,
) -> SocketAddr {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(address) = health.lock().unwrap().ingress.local_addr.clone() {
                let address = address
                    .parse()
                    .expect("health must report the actual supervised listener address");
                if address != previous {
                    return address;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        let ingress = health.lock().unwrap().ingress.clone();
        panic!(
            "supervisor did not publish a new listener address: previous={previous}, current={:?}",
            ingress.local_addr
        )
    })
}

async fn wait_for_bound_socket(
    composition: &DeterministicIngressComposition,
    logical: SocketAddr,
    occurrence: usize,
) -> SocketAddr {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let bound = composition.bound_for(logical);
            if bound.len() >= occurrence {
                return bound[occurrence - 1];
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "composition did not bind logical endpoint {logical} occurrence {occurrence}: {:?}",
            composition.bound_for(logical)
        )
    })
}

async fn wait_for_applied_tls_generation(
    db: &iotkit_core_storage::DbHandle,
    expected: u64,
) -> iotkit_core_ops::IngressListenerConfig {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let config = db
                .with_conn_sync(load_ingress_config)
                .unwrap();
            if config
                .applied
                .as_ref()
                .and_then(|state| state.tls_generation)
                == Some(expected)
            {
                return config;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        let config = db
            .with_conn_sync(load_ingress_config)
            .unwrap();
        panic!(
            "supervisor did not apply TLS generation {expected}: desired={:?}, applied={:?}, last_error={:?}",
            config.desired.tls_generation,
            config.applied.as_ref().and_then(|state| state.tls_generation),
            config.last_error,
        )
    })
}

async fn wait_for_applied_bind(
    db: &iotkit_core_storage::DbHandle,
    expected: SocketAddr,
) -> iotkit_core_ops::IngressListenerConfig {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let config = db.with_conn_sync(load_ingress_config).unwrap();
            if config
                .applied
                .as_ref()
                .is_some_and(|state| state.bind_addr == expected.to_string())
            {
                return config;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        let config = db.with_conn_sync(load_ingress_config).unwrap();
        panic!(
            "supervisor did not apply bind {expected}: desired={:?}, applied={:?}, last_error={:?}",
            config.desired.bind_addr,
            config.applied.as_ref().map(|state| &state.bind_addr),
            config.last_error,
        )
    })
}

async fn wait_for_apply_error(health: &Arc<Mutex<HealthState>>, expected: &'static str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if health.lock().unwrap().ingress.last_error.as_deref() == Some(expected) {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        let ingress = health.lock().unwrap().ingress.clone();
        panic!(
            "supervisor did not report {expected:?}: status={:?}, last_error={:?}, last_action={:?}",
            ingress.status, ingress.last_error, ingress.last_action,
        )
    });
}

struct TestTlsGeneration {
    cert_pem: String,
    fingerprint: String,
    generation: u64,
}

fn test_tls_generation(ip: Ipv4Addr, label: &str) -> (String, String) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec![label.into()]).unwrap();
    params.subject_alt_names.push(SanType::IpAddress(ip.into()));
    let cert = params.self_signed(&key).unwrap();
    (cert.pem(), key.serialize_pem())
}

fn rotate_tls_generation(
    db: &iotkit_core_storage::DbHandle,
    data_dir: &Path,
    cert_pem: &str,
    key_pem: &str,
) -> TestTlsGeneration {
    let fingerprint = iotkit_core_ops::fingerprint_of_pem(cert_pem).unwrap();
    let result = db
        .with_conn_sync(|conn| {
            dispatch_with_secret_dir(
                conn,
                standard_catalog(),
                DispatchRequest {
                    op: "ingress.tls.rotate".into(),
                    params: json!({"cert_pem": cert_pem, "key_pem": key_pem}),
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
                Some(data_dir),
            )
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
        })
        .unwrap();
    TestTlsGeneration {
        cert_pem: cert_pem.to_owned(),
        fingerprint,
        generation: result["generation"].as_u64().unwrap(),
    }
}

fn configure_tls_ingress(
    db: &iotkit_core_storage::DbHandle,
    bind: SocketAddr,
    interface: &str,
    site_local_cidr: &str,
) {
    db.with_conn_sync(|conn| {
        iotkit_core_ops::dispatch(
            conn,
            iotkit_core_ops::standard_catalog(),
            iotkit_core_ops::DispatchRequest {
                op: "ingress.listener.configure".into(),
                params: json!({
                    "enabled": true,
                    "bind_addr": bind.to_string(),
                    "interface": interface,
                    "site_local_cidrs": [site_local_cidr],
                    "mode": "tls"
                }),
                dry_run: false,
                actor: iotkit_core_ops::Actor {
                    actor_id: "local_cli".into(),
                    actor_kind: iotkit_core_ops::ActorKind::LocalCli,
                    tier_ceiling: iotkit_core_ops::Tier::Construction,
                },
                source: Some("local_cli".into()),
                step_up_verified: true,
                clock_trust: None,
            },
        )
        .map_err(|error| {
            iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                Box::new(error),
            ))
        })?;
        Ok(())
    })
    .unwrap();
}

async fn pinned_ingress_get(address: SocketAddr, cert_pem: &str) -> Result<u16, String> {
    let certificate =
        reqwest::Certificate::from_pem(cert_pem.as_bytes()).map_err(|error| error.to_string())?;
    let client = reqwest::Client::builder()
        .add_root_certificate(certificate)
        .timeout(Duration::from_secs(1))
        .build()
        .map_err(|error| error.to_string())?;
    client
        .get(format!("https://{address}/api/v1/ingest"))
        .send()
        .await
        .map(|response| response.status().as_u16())
        .map_err(|error| error.to_string())
}

struct RunningTlsSupervisor {
    _dir: tempfile::TempDir,
    db: iotkit_core_storage::DbHandle,
    health: Arc<Mutex<HealthState>>,
    task: tokio::task::JoinHandle<()>,
    collector_task: tokio::task::JoinHandle<()>,
    address: SocketAddr,
    composition: Arc<DeterministicIngressComposition>,
    interface: String,
    site_local_cidr: String,
}

impl RunningTlsSupervisor {
    async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
        self.collector_task.abort();
        let _ = self.collector_task.await;
    }
}

async fn start_tls_supervisor() -> (RunningTlsSupervisor, TestTlsGeneration) {
    let dir = tempfile::tempdir().unwrap();
    let db =
        iotkit_core_storage::init_db(&dir.path().join("tls-rotation.db"), &service_migrations())
            .unwrap();
    own(&db);
    db.with_conn_sync(|conn| {
        iotkit_gateway::api::tls::ensure_tls_material(conn, dir.path())
            .map(|_| ())
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
    })
    .unwrap();

    let interface = TEST_SITE_INTERFACE.to_owned();
    let site_local_cidr = TEST_SITE_CIDR.to_owned();
    let logical_bind = logical_bind_a();
    let (cert_pem, key_pem) = test_tls_generation(TEST_SOCKET_IP, "task7-generation-1");
    let generation = rotate_tls_generation(&db, dir.path(), &cert_pem, &key_pem);
    configure_tls_ingress(&db, logical_bind, &interface, &site_local_cidr);

    let (collector, device_issuer, collector_task) =
        Collector::spawn_device_composed(db.clone(), Arc::new(PermissiveRegistry), 8);
    let service = HttpIngestService::new(
        db.clone(),
        collector,
        device_issuer,
        HttpIngestConfig::default(),
        SystemMonotonicClock::default(),
    )
    .unwrap();
    let health = Arc::new(Mutex::new(HealthState::new(90)));
    let composition = Arc::new(DeterministicIngressComposition::default());
    let task = iotkit_gateway::ingress::spawn_ingress_supervisor_serving_with_composition(
        db.clone(),
        dir.path().to_path_buf(),
        health.clone(),
        Duration::from_millis(1),
        service,
        composition.clone(),
    );
    wait_for_ingress_status(&health, "listening").await;
    let actual_address = wait_for_ingress_local_addr(&health).await;
    assert_eq!(actual_address.ip(), IpAddr::V4(TEST_SOCKET_IP));
    assert_eq!(
        wait_for_bound_socket(&composition, logical_bind, 1).await,
        actual_address
    );
    let applied = wait_for_applied_tls_generation(&db, generation.generation).await;
    assert_eq!(applied.desired.bind_addr, logical_bind.to_string());
    assert_eq!(
        applied.applied.unwrap().tls_fingerprint,
        Some(generation.fingerprint.clone())
    );

    (
        RunningTlsSupervisor {
            _dir: dir,
            db,
            health,
            task,
            collector_task,
            address: actual_address,
            composition,
            interface,
            site_local_cidr,
        },
        generation,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn same_address_tls_rotation_applies_new_generation_without_losing_service() {
    let (running, generation_1) = start_tls_supervisor().await;
    assert_eq!(
        pinned_ingress_get(running.address, &generation_1.cert_pem)
            .await
            .unwrap(),
        405
    );

    let (cert_pem, key_pem) = test_tls_generation(TEST_SOCKET_IP, "task7-generation-2");
    let generation_2 = rotate_tls_generation(&running.db, running._dir.path(), &cert_pem, &key_pem);
    assert_eq!(generation_2.generation, generation_1.generation + 1);
    let applied = wait_for_applied_tls_generation(&running.db, generation_2.generation).await;
    assert_eq!(
        applied.applied.unwrap().tls_fingerprint,
        Some(generation_2.fingerprint.clone())
    );
    assert_eq!(
        pinned_ingress_get(running.address, &generation_2.cert_pem)
            .await
            .unwrap(),
        405
    );
    assert_eq!(
        running.composition.bound_for(logical_bind_a()).len(),
        1,
        "same-bind TLS rotation must retain the owned socket"
    );
    assert!(
        pinned_ingress_get(running.address, &generation_1.cert_pem)
            .await
            .is_err()
    );
    running.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn failed_tls_publication_keeps_last_safe_generation_and_certificate_serving() {
    let (running, generation_1) = start_tls_supervisor().await;
    running
        .db
        .with_conn_sync(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER fail_ingress_applied_publication
                 BEFORE UPDATE OF applied_generation ON ingress_listener_config
                 WHEN NEW.applied_generation > OLD.applied_generation
                 BEGIN SELECT RAISE(ABORT, 'injected applied publication failure'); END;",
            )?;
            Ok(())
        })
        .unwrap();

    let (cert_pem, key_pem) = test_tls_generation(TEST_SOCKET_IP, "task7-generation-2-fault");
    let generation_2 = rotate_tls_generation(&running.db, running._dir.path(), &cert_pem, &key_pem);
    wait_for_apply_error(&running.health, "applied_state_write_failed").await;
    let config = running.db.with_conn_sync(load_ingress_config).unwrap();
    assert_eq!(
        config
            .applied
            .as_ref()
            .and_then(|state| state.tls_generation),
        Some(generation_1.generation)
    );
    assert_eq!(config.desired.tls_generation, Some(generation_2.generation));
    assert_eq!(
        pinned_ingress_get(running.address, &generation_1.cert_pem)
            .await
            .unwrap(),
        405
    );
    assert!(
        pinned_ingress_get(running.address, &generation_2.cert_pem)
            .await
            .is_err()
    );

    running
        .db
        .with_conn_sync(|conn| {
            conn.execute_batch("DROP TRIGGER fail_ingress_applied_publication")?;
            Ok(())
        })
        .unwrap();
    wait_for_applied_tls_generation(&running.db, generation_2.generation).await;
    assert_eq!(
        pinned_ingress_get(running.address, &generation_2.cert_pem)
            .await
            .unwrap(),
        405
    );
    running.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn different_bind_publication_failure_keeps_staged_socket_inert() {
    let (running, generation_1) = start_tls_supervisor().await;
    running
        .db
        .with_conn_sync(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER fail_ingress_applied_publication
                 BEFORE UPDATE OF applied_generation ON ingress_listener_config
                 WHEN NEW.applied_generation > OLD.applied_generation
                 BEGIN SELECT RAISE(ABORT, 'injected applied publication failure'); END;",
            )?;
            Ok(())
        })
        .unwrap();

    let next_bind = logical_bind_b();
    configure_tls_ingress(
        &running.db,
        next_bind,
        &running.interface,
        &running.site_local_cidr,
    );
    let staged_address = wait_for_bound_socket(&running.composition, next_bind, 1).await;
    wait_for_apply_error(&running.health, "applied_state_write_failed").await;
    assert_eq!(
        pinned_ingress_get(running.address, &generation_1.cert_pem)
            .await
            .unwrap(),
        405
    );
    assert!(
        pinned_ingress_get(staged_address, &generation_1.cert_pem)
            .await
            .is_err()
    );

    running
        .db
        .with_conn_sync(|conn| {
            conn.execute_batch("DROP TRIGGER fail_ingress_applied_publication")?;
            Ok(())
        })
        .unwrap();
    wait_for_applied_bind(&running.db, next_bind).await;
    let old_address = running.address;
    let new_address = wait_for_ingress_local_addr_other_than(&running.health, old_address).await;
    assert!(
        pinned_ingress_get(old_address, &generation_1.cert_pem)
            .await
            .is_err()
    );
    assert_eq!(
        pinned_ingress_get(new_address, &generation_1.cert_pem)
            .await
            .unwrap(),
        405
    );
    running.stop().await;
}

fn enter_restored_local_recovery(db: &iotkit_core_storage::DbHandle) {
    db.with_conn_sync(|conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
        let epoch = iotkit_core_ops::new_auth_epoch()
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        iotkit_core_ops::enter_restored_local_recovery(&tx, &epoch)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        Ok(tx.commit()?)
    })
    .unwrap();
}

#[tokio::test]
async fn listener_supervisor_exit_clears_health_without_stopping_collection() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    own(&db);
    let dir = tempfile::tempdir().unwrap();
    db.with_conn_sync(|conn| {
        iotkit_gateway::api::tls::ensure_tls_material(conn, dir.path())
            .map(|_| ())
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
    })
    .unwrap();
    let health = Arc::new(Mutex::new(HealthState::new(90)));
    let task = iotkit_gateway::ingress::spawn_ingress_supervisor(
        db,
        dir.path().to_path_buf(),
        health.clone(),
        Duration::from_millis(1),
    );
    wait_for_ingress_status(&health, "disabled").await;
    task.abort();
    let _ = task.await;
    let health = health.lock().unwrap();
    assert_eq!(health.ingress.status, "error");
    assert_eq!(
        health.ingress.last_error.as_deref(),
        Some("listener_task_exited")
    );
    assert!(
        health.collector_alive,
        "listener failure must not stop in-process collection"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restored_local_recovery_drains_the_edge_ingress_listener() {
    let dir = tempfile::tempdir().unwrap();
    let db = iotkit_core_storage::init_db(
        &dir.path().join("recovery-ingress.db"),
        &service_migrations(),
    )
    .unwrap();
    own(&db);
    db.with_conn_sync(|conn| {
        iotkit_gateway::api::tls::ensure_tls_material(conn, dir.path())
            .map(|_| ())
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
    })
    .unwrap();

    let bind = logical_bind_a();
    let interface = TEST_SITE_INTERFACE;
    let site_local_cidr = TEST_SITE_CIDR;
    configure_private_ingress(&db, bind, interface, site_local_cidr);

    let (principal, token) = provision_device(&db);

    let (collector, device_issuer, collector_task) =
        Collector::spawn_device_composed(db.clone(), Arc::new(PermissiveRegistry), 8);
    let service = HttpIngestService::new(
        db.clone(),
        collector,
        device_issuer,
        HttpIngestConfig::default(),
        SystemMonotonicClock::default(),
    )
    .unwrap();
    let health = Arc::new(Mutex::new(HealthState::new(90)));
    let composition = Arc::new(DeterministicIngressComposition::default());
    let supervisor = iotkit_gateway::ingress::spawn_ingress_supervisor_serving_with_composition(
        db.clone(),
        dir.path().to_path_buf(),
        health.clone(),
        Duration::from_millis(1),
        service,
        composition,
    );
    wait_for_ingress_status(&health, "degraded").await;
    let before_bind = wait_for_ingress_local_addr(&health).await;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    let before_recovery = client
        .get(format!("http://{before_bind}/api/v1/ingest"))
        .send()
        .await
        .expect("the supervised listener must be reachable before recovery");
    assert_eq!(
        before_recovery.status(),
        reqwest::StatusCode::METHOD_NOT_ALLOWED
    );
    let before_ingest = client
        .post(format!("http://{before_bind}/api/v1/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "envelope_id": "recovery-token-continuity-before-0001",
            "source": principal.clone(),
            "items": [{
                "subject_hint": "task7-recovery-device",
                "measurement_key": "temperature_c",
                "values": [30.5],
                "time_source": "edge"
            }]
        }))
        .send()
        .await
        .expect("the initial supervised listener must accept the device token");
    assert_eq!(before_ingest.status(), reqwest::StatusCode::OK);

    enter_restored_local_recovery(&db);
    wait_for_ingress_status(&health, "unbound").await;
    assert_eq!(
        health.lock().unwrap().ingress.last_error.as_deref(),
        Some("local_recovery_required")
    );
    let applied_generation: i64 = db
        .with_conn_sync(|conn| {
            Ok(conn.query_row(
                "SELECT applied_generation FROM ingress_listener_config WHERE id=1",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(applied_generation, 0);

    let after_recovery = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap()
        .get(format!("http://{before_bind}/api/v1/ingest"))
        .send()
        .await;
    assert!(
        after_recovery.is_err(),
        "recovery authority must drain the Edge listener, not return an ingest auth response"
    );

    own(&db);
    wait_for_ingress_status(&health, "degraded").await;
    wait_for_applied_bind(&db, bind).await;
    let after_bind = wait_for_ingress_local_addr(&health).await;
    let restored = client
        .post(format!("http://{after_bind}/api/v1/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "envelope_id": "recovery-token-continuity-0001",
            "source": principal,
            "items": [{
                "subject_hint": "task7-recovery-device",
                "measurement_key": "temperature_c",
                "values": [31.5],
                "time_source": "edge"
            }]
        }))
        .send()
        .await
        .expect("the re-applied listener must accept the restored device token");
    assert_eq!(restored.status(), reqwest::StatusCode::OK);
    let restored_ack: serde_json::Value = restored
        .json()
        .await
        .expect("the restored ingest response must be JSON");
    assert_eq!(restored_ack["status"]["kind"], "accepted");
    let reading_count: i64 = db
        .with_conn_sync(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?)
        })
        .unwrap();
    assert_eq!(reading_count, 2);

    supervisor.abort();
    let _ = supervisor.await;
    collector_task.abort();
    let _ = collector_task.await;
}

fn provision_device(db: &iotkit_core_storage::DbHandle) -> (String, String) {
    let result = db
        .with_conn_sync(|conn| {
            Ok(dispatch(
                conn,
                standard_catalog(),
                DispatchRequest {
                    op: "device.add_with_credential".into(),
                    params: json!({
                        "hardware_id": "task7-recovery-device",
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
    let (metadata, secret) = match result {
        DispatchResult::DeviceCredential(secret) => secret.consume(),
        DispatchResult::Public(_) => panic!("device commissioning must issue a credential"),
    };
    (
        metadata["principal_id"].as_str().unwrap().to_owned(),
        secret.as_str().to_owned(),
    )
}

#[test]
fn common_gate_closes_unowned_recovery_fences_and_restored_generation_mismatch() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::Unowned)
        );
        Ok(())
    })
    .unwrap();
    own(&db);
    db.with_conn_sync(|conn| {
        iotkit_gateway::api::tls::ensure_tls_material(conn, dir.path())
            .map(|_| ())
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
    })
    .unwrap();
    db.with_conn_sync(|conn| {
        conn.execute("DELETE FROM admin_credential", [])?;
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::LocalRecoveryRequired)
        );
        Ok(())
    })
    .unwrap();
    own(&db);
    std::fs::write(dir.path().join("restore-in-progress"), b"fence").unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::RestoreInProgress)
        );
        Ok(())
    })
    .unwrap();
    std::fs::remove_file(dir.path().join("restore-in-progress")).unwrap();
    std::fs::write(dir.path().join("reset-in-progress"), b"fence").unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::ResetInProgress)
        );
        Ok(())
    })
    .unwrap();
    std::fs::remove_file(dir.path().join("reset-in-progress")).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE ingress_listener_config SET enabled=1,desired_generation=1,
             applied_generation=1,bind_addr='192.168.1.2:8444',interface='eth0',
             site_local_cidrs='[\"192.168.1.0/24\"]',mode='private_plaintext',
             applied_bind_addr='192.168.1.2:8444',applied_interface='eth0',
             applied_site_local_cidrs='[\"192.168.1.0/24\"]',
             applied_mode='private_plaintext' WHERE id=1",
            [],
        )?;
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
        iotkit_core_ops::enter_restored_local_recovery(
            &tx,
            &iotkit_core_ops::new_auth_epoch().unwrap(),
        )
        .unwrap();
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    own(&db);
    db.with_conn_sync(|conn| {
        assert!(
            iotkit_gateway::network_authority::require_common_network_authority(conn, dir.path())
                .is_ok(),
            "local recovery must restore control-plane authority even while ingress awaits reapply"
        );
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::UnsafeIngressGeneration)
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn common_gate_rejects_partial_corrupt_and_mismatched_control_tls_for_both_listeners() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    own(&db);
    let dir = tempfile::tempdir().unwrap();
    db.with_conn_sync(|conn| {
        iotkit_gateway::api::tls::ensure_tls_material(conn, dir.path())
            .map(|_| ())
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
    })
    .unwrap();
    let key = dir.path().join("tls/key.pem");
    let original = std::fs::read(&key).unwrap();
    std::fs::remove_file(&key).unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::TlsNotReady)
        );
        Ok(())
    })
    .unwrap();
    std::fs::write(&key, b"corrupt private key").unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::TlsNotReady)
        );
        Ok(())
    })
    .unwrap();
    let mismatched = rcgen::KeyPair::generate().unwrap().serialize_pem();
    std::fs::write(&key, mismatched).unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::TlsNotReady)
        );
        Ok(())
    })
    .unwrap();
    std::fs::write(&key, original).unwrap();
    db.with_conn_sync(|conn| {
        assert!(require_network_authority(conn, dir.path()).is_ok());
        Ok(())
    })
    .unwrap();
}

#[test]
fn ingress_gate_requires_the_exact_approved_tls_generation_bytes() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    own(&db);
    let dir = tempfile::tempdir().unwrap();
    db.with_conn_sync(|conn| {
        iotkit_gateway::api::tls::ensure_tls_material(conn, dir.path())
            .map(|_| ())
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
    })
    .unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["ingress.test".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let cert_pem = cert.pem();
    let key_pem = key.serialize_pem();
    let fingerprint = iotkit_core_ops::fingerprint_of_pem(&cert_pem).unwrap();
    let generation = dir.path().join("ingress-tls/generation-1");
    std::fs::create_dir_all(&generation).unwrap();
    std::fs::write(generation.join("cert.pem"), cert_pem).unwrap();
    std::fs::write(generation.join("key.pem"), key_pem).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE ingress_listener_config SET enabled=1,desired_generation=1,
             applied_generation=1,bind_addr='192.168.1.2:8444',interface='eth0',
             site_local_cidrs='[\"192.168.1.0/24\"]',mode='tls',
             desired_tls_generation=1,desired_tls_fingerprint=?1,
             applied_bind_addr='192.168.1.2:8444',applied_interface='eth0',
             applied_site_local_cidrs='[\"192.168.1.0/24\"]',applied_mode='tls',
             applied_tls_generation=1,applied_tls_fingerprint=?1 WHERE id=1",
            [&fingerprint],
        )?;
        assert!(require_network_authority(conn, dir.path()).is_ok());
        Ok(())
    })
    .unwrap();
    std::fs::remove_file(generation.join("key.pem")).unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::TlsNotReady)
        );
        Ok(())
    })
    .unwrap();
}
