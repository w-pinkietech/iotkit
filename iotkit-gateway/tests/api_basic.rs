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

fn api_config(bind: SocketAddr) -> ApiConfig {
    ApiConfig {
        enabled: true,
        bind,
        gateway_name: "test-gateway".to_string(),
    }
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

    let handle = spawn_api_task(
        db.clone(),
        health,
        cfg,
        "epoch-test".to_string(),
        dir.path().to_path_buf(),
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
    assert_eq!(box_before["setup_mode"], true);
    assert_eq!(box_before["tls_fingerprint"], handle.fingerprint);
    assert_eq!(box_before["health_summary"]["status"], "ok");
    assert_eq!(box_before["health_summary"]["adapters_alive"], 0);
    assert!(box_before.get("devices").is_none());
    assert!(box_before.get("measurements").is_none());

    let setup: Value = client
        .post(format!("{base}/api/v1/setup/passphrase"))
        .json(&json!({"passphrase":"correct horse battery staple"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let setup_token = setup["token"].as_str().expect("setup returns token");
    assert!(setup_token.starts_with("iko_"));
    assert_eq!(event_count(&db, "admin_passphrase_set"), 1);
    assert_eq!(event_count(&db, "auth_session_issued"), 1);

    let box_after: Value = client
        .get(format!("{base}/api/v1/box"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(box_after["setup_mode"], false);

    let second_setup = client
        .post(format!("{base}/api/v1/setup/passphrase"))
        .json(&json!({"passphrase":"another passphrase"}))
        .send()
        .await
        .unwrap();
    assert_eq!(second_setup.status(), StatusCode::CONFLICT);
    assert_eq!(event_count(&db, "admin_passphrase_set"), 1);
    assert_eq!(event_count(&db, "auth_session_issued"), 1);

    let throttled_setup = client
        .post(format!("{base}/api/v1/setup/passphrase"))
        .json(&json!({"passphrase":"another passphrase"}))
        .send()
        .await
        .unwrap();
    assert_eq!(throttled_setup.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(event_count(&db, "admin_passphrase_set"), 1);
    assert_eq!(event_count(&db, "auth_session_issued"), 1);

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
            saw_429 = true;
            break;
        }
    }
    assert!(saw_429, "wrong passphrase attempts should trigger throttle");
    assert!(event_count(&db, "auth_failed") >= 1);

    tokio::time::sleep(Duration::from_secs(5)).await;
    let session: Value = client
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
    assert!(session["token"].as_str().unwrap().starts_with("iko_"));
    assert!(event_count(&db, "auth_session_issued") >= 2);

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
