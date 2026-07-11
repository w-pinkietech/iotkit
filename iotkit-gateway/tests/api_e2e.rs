use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_core_ledger as ledger;
use iotkit_core_storage::{DbHandle, Migration};
use iotkit_gateway::api::{ApiHandle, spawn_api_task};
use iotkit_gateway::config::ApiConfig;
use iotkit_gateway::health::HealthState;
use reqwest::header;
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

fn api_config() -> ApiConfig {
    ApiConfig {
        enabled: true,
        bind: "127.0.0.1:0".parse().unwrap(),
        gateway_name: "e2e-gateway".to_string(),
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

fn seed_sighting(db: &DbHandle, hardware_id: &str) {
    db.with_conn_sync(|conn| {
        ledger::record_sighting(conn, hardware_id, "api-e2e-test").unwrap();
        Ok(())
    })
    .unwrap();
}

fn r14_details(db: &DbHandle) -> Vec<Value> {
    db.with_conn_sync(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT detail FROM ledger_events
                 WHERE kind = 'r14_op'
                 ORDER BY event_id ASC",
            )
            .unwrap();
        let details = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|row| serde_json::from_str::<Value>(&row.unwrap()).unwrap())
            .collect();
        Ok(details)
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
async fn acceptance_setup_session_approve_sighting_and_api_health_cleanup() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let hardware_id = "rpi-local:default:i2c:0x60";
    seed_sighting(&db, hardware_id);
    let health = Arc::new(Mutex::new(HealthState::new(90)));
    let clock_trust = prepare_owned_clock(&db);

    let handle = spawn_api_task(
        db.clone(),
        health.clone(),
        api_config(),
        "epoch-e2e".to_string(),
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
    assert_eq!(box_before["ownership"], "owned");
    assert_eq!(box_before["tls_fingerprint"], handle.fingerprint);

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
    let bearer = format!("Bearer {}", session["token"].as_str().unwrap());

    let dry_run: Value = client
        .post(format!("{base}/api/v1/ops/device.approve_sighting"))
        .header(header::AUTHORIZATION, &bearer)
        .json(&json!({
            "params": {"hardware_ids": [hardware_id]},
            "dry_run": true
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(dry_run["would"], "approve_sighting_as_quarantined");

    let executed: Value = client
        .post(format!("{base}/api/v1/ops/device.approve_sighting"))
        .header(header::AUTHORIZATION, &bearer)
        .json(&json!({"params": {"hardware_ids": [hardware_id]}}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(executed["approved"][0]["hardware_id"], hardware_id);

    let details = r14_details(&db);
    assert_eq!(details.len(), 2);
    assert_eq!(details[0]["op"], "device.approve_sighting");
    assert_eq!(details[0]["dry_run"], true);
    assert_eq!(details[0]["result"], "ok");
    assert_eq!(details[0]["targets"], json!([hardware_id]));
    assert_eq!(details[1]["op"], "device.approve_sighting");
    assert_eq!(details[1]["dry_run"], false);
    assert_eq!(details[1]["result"], "ok");
    for key in [
        "op",
        "actor",
        "actor_kind",
        "tier",
        "effective_tier",
        "dry_run",
        "params",
        "result",
        "targets",
        "source",
    ] {
        assert!(
            details[1].get(key).is_some(),
            "missing r14_op detail key: {key}"
        );
    }

    assert!(health.lock().unwrap().api.is_some());
    shutdown(handle).await;
    assert!(
        health.lock().unwrap().api.is_none(),
        "API health must be cleared after the server task exits"
    );
}
