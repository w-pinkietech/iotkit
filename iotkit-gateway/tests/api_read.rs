use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_core_ledger::{self as ledger, DeviceKind};
use iotkit_core_storage::{DbHandle, Migration};
use iotkit_core_timeseries::{NewReading, insert_reading_v3};
use iotkit_gateway::api::{ApiHandle, spawn_api_task};
use iotkit_gateway::config::ApiConfig;
use iotkit_gateway::health::HealthState;
use reqwest::{StatusCode, header};
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
        gateway_name: "read-test-gateway".to_string(),
    }
}

async fn shutdown(handle: ApiHandle) {
    let _ = handle.shutdown.send(());
    tokio::time::timeout(Duration::from_secs(5), handle.join)
        .await
        .expect("api task should stop after shutdown")
        .expect("api task should not panic");
}

struct Seeded {
    temp_key: String,
    humidity_key: String,
    temp_expected_seqs: Vec<i64>,
    temp_latest_seq: i64,
}

fn seed_readings(db: &DbHandle) -> Seeded {
    db.with_conn_sync(|conn| {
        ledger::record_sighting(conn, "ble:read-node", "test-adapter").unwrap();
        let system_id = ledger::approve_sighting(
            conn,
            "ble:read-node",
            Some("Kitchen node"),
            DeviceKind::Individual,
        )
        .unwrap();
        ledger::activate_device(conn, &system_id).unwrap();

        let temp_id = ledger::ensure_series(
            conn,
            &system_id,
            "temperature_c",
            ledger::CHANNEL_NA,
            ledger::DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();
        let humidity_id = ledger::ensure_series(
            conn,
            &system_id,
            "humidity_pct",
            ledger::CHANNEL_NA,
            ledger::DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();

        let temp_key = ledger::series_key_of(
            &system_id,
            "temperature_c",
            ledger::CHANNEL_NA,
            ledger::DEFAULT_VARIANT,
        );
        let humidity_key = ledger::series_key_of(
            &system_id,
            "humidity_pct",
            ledger::CHANNEL_NA,
            ledger::DEFAULT_VARIANT,
        );

        let temp_seq_1 = insert_reading_v3(
            conn,
            &NewReading {
                series_id: temp_id,
                received_at_ms: 1000,
                device_time_ms: None,
                time_source: "gateway".to_string(),
                values: vec![20.5],
                rssi: None,
                battery_pct: None,
                quarantined: false,
            },
        )
        .unwrap();
        let _temp_quarantined = insert_reading_v3(
            conn,
            &NewReading {
                series_id: temp_id,
                received_at_ms: 1500,
                device_time_ms: None,
                time_source: "gateway".to_string(),
                values: vec![99.9],
                rssi: None,
                battery_pct: None,
                quarantined: true,
            },
        )
        .unwrap();
        let temp_seq_2 = insert_reading_v3(
            conn,
            &NewReading {
                series_id: temp_id,
                received_at_ms: 2000,
                device_time_ms: None,
                time_source: "gateway".to_string(),
                values: vec![21.0],
                rssi: None,
                battery_pct: None,
                quarantined: false,
            },
        )
        .unwrap();
        insert_reading_v3(
            conn,
            &NewReading {
                series_id: humidity_id,
                received_at_ms: 1800,
                device_time_ms: None,
                time_source: "gateway".to_string(),
                values: vec![55.0],
                rssi: None,
                battery_pct: None,
                quarantined: false,
            },
        )
        .unwrap();

        Ok(Seeded {
            temp_key,
            humidity_key,
            temp_expected_seqs: vec![temp_seq_1, temp_seq_2],
            temp_latest_seq: temp_seq_2,
        })
    })
    .unwrap()
}

#[tokio::test]
async fn protected_read_routes_match_setup_and_bearer_matrix() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let seeded = seed_readings(&db);
    let health = Arc::new(Mutex::new(HealthState::new(90)));

    let handle = spawn_api_task(
        db.clone(),
        health,
        api_config(),
        "epoch-read-test".to_string(),
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();
    let base = format!("https://{}", handle.local_addr);
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    let setup_series: Value = client
        .get(format!("{base}/api/v1/series"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let setup_series = setup_series.as_array().unwrap();
    assert_eq!(setup_series.len(), 2);
    assert!(setup_series[0].get("series_id").is_none());
    assert!(setup_series[0].get("series_key").is_some());
    assert!(setup_series[0].get("system_id").is_some());
    assert_eq!(setup_series[0]["user_label"], "Kitchen node");

    let setup_live: Value = client
        .get(format!("{base}/api/v1/live"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let setup_live = setup_live.as_array().unwrap();
    assert_eq!(setup_live.len(), 2);
    let temp_live = setup_live
        .iter()
        .find(|row| row["series_key"] == seeded.temp_key)
        .unwrap();
    assert_eq!(temp_live["event_time"], 2000);
    assert_eq!(temp_live["event_time_source"], "received_at");
    assert_eq!(temp_live["quarantined"], false);
    assert_eq!(temp_live["values"], json!([21.0]));
    assert!(temp_live.get("seq").is_none());
    assert!(
        setup_live
            .iter()
            .any(|row| row["series_key"] == seeded.humidity_key)
    );

    let setup_readings = client
        .get(format!(
            "{base}/api/v1/readings?series_key={}&from_ms=0&to_ms=3000",
            seeded.temp_key
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(setup_readings.status(), StatusCode::UNAUTHORIZED);
    let setup_health = client
        .get(format!("{base}/api/v1/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(setup_health.status(), StatusCode::UNAUTHORIZED);

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
    let token = setup["token"].as_str().unwrap();

    for path in [
        "/api/v1/health",
        "/api/v1/series",
        "/api/v1/live",
        &format!(
            "/api/v1/readings?series_key={}&from_ms=0&to_ms=3000",
            seeded.temp_key
        ),
    ] {
        let res = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{path}");
    }

    let bearer = format!("Bearer {token}");
    let health_json: Value = client
        .get(format!("{base}/api/v1/health"))
        .header(header::AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health_json["epoch"], "epoch-read-test");
    assert_eq!(health_json["api"]["bind"], handle.local_addr.to_string());
    assert_eq!(health_json["api"]["tls_fingerprint"], handle.fingerprint);

    for path in ["/api/v1/series", "/api/v1/live"] {
        let res = client
            .get(format!("{base}{path}"))
            .header(header::AUTHORIZATION, &bearer)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
    }

    let missing_range = client
        .get(format!(
            "{base}/api/v1/readings?series_key={}&to_ms=3000",
            seeded.temp_key
        ))
        .header(header::AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_range.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let missing_range_body: Value = missing_range.json().await.unwrap();
    assert_eq!(missing_range_body["error"]["code"], "missing_range");

    let unknown = client
        .get(format!(
            "{base}/api/v1/readings?series_key=00000000000000000000000000000000:unknown:na:primary&from_ms=0&to_ms=3000"
        ))
        .header(header::AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let unknown_body: Value = unknown.json().await.unwrap();
    assert_eq!(unknown_body["error"]["code"], "unknown_series");

    let readings: Value = client
        .get(format!(
            "{base}/api/v1/readings?series_key={}&from_ms=0&to_ms=3000&limit=100",
            seeded.temp_key
        ))
        .header(header::AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(readings["series_key"], seeded.temp_key);
    let rows = readings["rows"].as_array().unwrap();
    let seqs: Vec<i64> = rows
        .iter()
        .map(|row| row["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, seeded.temp_expected_seqs);
    assert_eq!(seqs.last().copied(), Some(seeded.temp_latest_seq));

    let expected = db
        .with_conn_sync(|conn| {
            let series_id = ledger::find_series_by_key(conn, &seeded.temp_key)
                .unwrap()
                .unwrap();
            let rows = iotkit_core_timeseries::query::query_readings_v3(
                conn, series_id, 0, 3000, 100, false,
            )
            .unwrap();
            Ok(rows.into_iter().map(|row| row.seq).collect::<Vec<_>>())
        })
        .unwrap();
    assert_eq!(seqs, expected);

    shutdown(handle).await;
}
