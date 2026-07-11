use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_core_storage::{DbHandle, Migration};
use iotkit_gateway::api::{ApiHandle, spawn_api_task};
use iotkit_gateway::config::{
    ApiConfig, ConfigError, ConfigSource, RawApiConfig, RawConfig, resolve,
};
use iotkit_gateway::health::HealthState;
use reqwest::StatusCode;
use serde_json::{Value, json};

fn all_migrations() -> Vec<Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    all
}

fn event_count(db: &DbHandle, kind: &str) -> i64 {
    db.with_conn_sync(|conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM ledger_events WHERE kind = ?1",
            [kind],
            |row| row.get(0),
        )
        .map_err(iotkit_core_storage::StorageError::from)
    })
    .unwrap()
}

fn auth_session_audit_token_ids(db: &DbHandle) -> Vec<String> {
    db.with_conn_sync(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT detail FROM ledger_events
                 WHERE kind = 'auth_session_issued' ORDER BY event_id",
            )
            .map_err(iotkit_core_storage::StorageError::from)?;
        let details = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(iotkit_core_storage::StorageError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(iotkit_core_storage::StorageError::from)?;
        details
            .into_iter()
            .map(|detail| {
                let detail: Value = serde_json::from_str(&detail).map_err(|error| {
                    iotkit_core_storage::StorageError::Sqlite(
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        ),
                    )
                })?;
                detail["token_id"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        iotkit_core_storage::StorageError::Sqlite(
                            rusqlite::Error::InvalidColumnType(
                                0,
                                "token_id".to_string(),
                                rusqlite::types::Type::Null,
                            ),
                        )
                    })
            })
            .collect()
    })
    .unwrap()
}

fn clock_floor(db: &DbHandle) -> i64 {
    db.with_conn_sync(|conn| {
        iotkit_core_ops::ClockTrust::persisted_floor(conn).map_err(|error| {
            iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                Box::new(error),
            ))
        })
    })
    .unwrap()
}

fn api_config(bind: SocketAddr) -> ApiConfig {
    ApiConfig {
        enabled: true,
        bind,
        gateway_name: "test-gateway".to_string(),
    }
}

fn prepare_owned_clock(db: &DbHandle) -> Arc<iotkit_core_ops::ClockTrust> {
    db.with_conn_sync(|conn| {
        let hash = iotkit_core_ops::hash_passphrase("correct horse battery staple").unwrap();
        iotkit_core_ops::reset_passphrase_with_hash(conn, &hash, "local_cli").unwrap();
        let clock = Arc::new(iotkit_core_ops::SystemClock::default());
        let trust = iotkit_core_ops::ClockTrust::load(
            conn,
            clock.clone(),
            Duration::from_secs(2),
            Duration::from_secs(300),
        )
        .unwrap();
        let displayed = iotkit_core_ops::Clock::wall_time_ms(clock.as_ref());
        iotkit_core_ops::confirm_time_with_clock(conn, clock.as_ref(), displayed).unwrap();
        Ok(Arc::new(trust))
    })
    .unwrap()
}

async fn shutdown(handle: ApiHandle) {
    let _ = handle.shutdown.send(());
    tokio::time::timeout(Duration::from_secs(5), handle.join)
        .await
        .expect("api task should stop after shutdown")
        .expect("api task should not panic");
}

#[tokio::test]
async fn box_setup_session_throttle_and_graceful_shutdown() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let health = Arc::new(Mutex::new(HealthState::new(90)));
    let cfg = api_config("127.0.0.1:0".parse().unwrap());
    let clock_trust = prepare_owned_clock(&db);

    let handle = spawn_api_task(
        db.clone(),
        health,
        cfg,
        "epoch-test".to_string(),
        dir.path().to_path_buf(),
        clock_trust.clone(),
    )
    .await
    .unwrap();
    let base = format!("https://{}", handle.local_addr);
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    let box_before: Value = client
        .get(format!("{base}/api/v1/box"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(box_before["gateway_name"], "test-gateway");
    assert_eq!(box_before["epoch"], "epoch-test");
    assert_eq!(box_before["ownership"], "owned");
    assert_eq!(box_before["tls_fingerprint"], handle.fingerprint);
    assert_eq!(box_before["health_summary"]["status"], "ok");
    assert_eq!(box_before["health_summary"]["adapters_alive"], 0);
    assert!(box_before.get("devices").is_none());
    assert!(box_before.get("measurements").is_none());

    let removed_setup = client
        .post(format!("{base}/api/v1/setup/passphrase"))
        .json(&json!({"passphrase":"correct horse battery staple"}))
        .send()
        .await
        .unwrap();
    assert_eq!(removed_setup.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(event_count(&db, "admin_passphrase_reset"), 1);
    assert_eq!(event_count(&db, "auth_session_issued"), 0);

    tokio::time::sleep(Duration::from_secs(5)).await;

    let mut saw_429 = false;
    for _ in 0..12 {
        let res = client
            .post(format!("{base}/api/v1/session"))
            .json(&json!({"passphrase":"wrong"}))
            .send()
            .await
            .unwrap();
        if res.status() == StatusCode::TOO_MANY_REQUESTS {
            assert!(
                res.headers().contains_key(reqwest::header::RETRY_AFTER),
                "429 response should include Retry-After"
            );
            saw_429 = true;
            break;
        }
    }
    assert!(saw_429, "wrong passphrase attempts should trigger throttle");
    assert!(event_count(&db, "auth_failed") >= 1);

    tokio::time::sleep(Duration::from_secs(5)).await;
    let floor_before_sessions = clock_floor(&db);
    let first_session: Value = client
        .post(format!("{base}/api/v1/session"))
        .json(&json!({"passphrase":"correct horse battery staple"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(first_session["token"].as_str().unwrap().starts_with("iko_"));
    let first_token_id = first_session["token_id"].as_str().unwrap();
    assert_eq!(
        auth_session_audit_token_ids(&db),
        [first_token_id],
        "the first successful session must commit exactly its own audit event"
    );
    let floor_after_first = clock_floor(&db);
    assert!(
        floor_after_first > floor_before_sessions,
        "the first session issue transaction must advance the persisted clock floor"
    );

    tokio::time::timeout(Duration::from_secs(1), async {
        while clock_trust.wall_time_ms() <= floor_after_first {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("wall clock should move beyond the first committed floor");
    let second_session: Value = client
        .post(format!("{base}/api/v1/session"))
        .json(&json!({"passphrase":"correct horse battery staple"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_token_id = second_session["token_id"].as_str().unwrap();
    assert_ne!(first_token_id, second_token_id);
    assert_eq!(
        auth_session_audit_token_ids(&db),
        [first_token_id, second_token_id],
        "each successful session must commit one attributable audit event"
    );
    assert!(
        clock_floor(&db) > floor_after_first,
        "the second session issue transaction must also advance the persisted clock floor"
    );

    shutdown(handle).await;
}

#[tokio::test]
async fn fresh_database_never_starts_a_control_listener() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("fresh.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let health = Arc::new(Mutex::new(HealthState::new(90)));
    let clock_trust = db
        .with_conn_sync(|conn| {
            iotkit_core_ops::ClockTrust::load(
                conn,
                Arc::new(iotkit_core_ops::SystemClock::default()),
                Duration::from_secs(2),
                Duration::from_secs(300),
            )
            .map(Arc::new)
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
        })
        .unwrap();

    let result = spawn_api_task(
        db,
        health,
        api_config("127.0.0.1:0".parse().unwrap()),
        "epoch-fresh".to_string(),
        dir.path().to_path_buf(),
        clock_trust,
    )
    .await;

    assert!(matches!(
        result,
        Err(iotkit_gateway::api::ApiError::NotReady("unowned"))
    ));
}

#[tokio::test]
async fn known_restore_or_reset_state_blocks_control_listener_before_bind() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("owned.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let health = Arc::new(Mutex::new(HealthState::new(90)));
    let clock_trust = prepare_owned_clock(&db);
    std::fs::write(
        dir.path().join("restore-in-progress"),
        b"known incomplete restore",
    )
    .unwrap();

    let result = spawn_api_task(
        db,
        health,
        api_config("127.0.0.1:0".parse().unwrap()),
        "epoch-fenced".to_string(),
        dir.path().to_path_buf(),
        clock_trust,
    )
    .await;

    assert!(matches!(
        result,
        Err(iotkit_gateway::api::ApiError::NotReady(
            "restore_in_progress"
        ))
    ));
}

#[tokio::test]
async fn network_passphrase_setup_route_is_absent() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let health = Arc::new(Mutex::new(HealthState::new(90)));
    let cfg = api_config("127.0.0.1:0".parse().unwrap());
    let clock_trust = prepare_owned_clock(&db);

    let handle = spawn_api_task(
        db.clone(),
        health,
        cfg,
        "epoch-passphrase-test".to_string(),
        dir.path().to_path_buf(),
        clock_trust,
    )
    .await
    .unwrap();
    let base = format!("https://{}", handle.local_addr);
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    let session: Value = client
        .post(format!("{base}/api/v1/session"))
        .json(&json!({"passphrase": "correct horse battery staple"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let response = client
        .post(format!("{base}/api/v1/setup/passphrase"))
        .bearer_auth(session["token"].as_str().unwrap())
        .json(&json!({"passphrase": "12345678"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    shutdown(handle).await;
}

#[test]
fn private_source_guard_accepts_only_private_or_link_local_sources() {
    use iotkit_gateway::api::guard::is_private_source;

    let accepted = [
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)),
        IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(192, 168, 10, 5)),
        IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1)),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
        IpAddr::V6("fc00::1".parse().unwrap()),
        IpAddr::V6("fd12:3456::1".parse().unwrap()),
        IpAddr::V6("fe80::1".parse().unwrap()),
        IpAddr::V6("::ffff:192.168.1.20".parse().unwrap()),
    ];
    for ip in accepted {
        assert!(is_private_source(ip), "{ip} should be accepted");
    }

    let rejected = [
        IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
        IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),
        IpAddr::V6("2001:4860:4860::8888".parse().unwrap()),
        IpAddr::V6("::ffff:8.8.8.8".parse().unwrap()),
    ];
    for ip in rejected {
        assert!(!is_private_source(ip), "{ip} should be rejected");
    }
}

#[test]
fn api_bind_rejects_ipv6_socket_addresses() {
    let raw = RawConfig {
        api: RawApiConfig {
            bind: Some("[::]:8443".to_string()),
            ..RawApiConfig::default()
        },
        ..RawConfig::default()
    };

    let err = resolve(raw, ConfigSource::DefaultsOnly).unwrap_err();
    assert!(matches!(err, ConfigError::Validation(_)));
    assert!(err.to_string().contains("api.bind"));
}
