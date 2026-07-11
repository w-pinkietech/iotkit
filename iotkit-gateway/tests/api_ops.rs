use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_core_ledger::{self as ledger, DeviceKind, DeviceState, NewDevice, SystemId};
use iotkit_core_storage::{DbHandle, Migration};
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
        gateway_name: "ops-test-gateway".to_string(),
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

fn seed_sightings(db: &DbHandle, hardware_ids: &[&str]) {
    db.with_conn_sync(|conn| {
        for hardware_id in hardware_ids {
            ledger::record_sighting(conn, hardware_id, "api-ops-test").unwrap();
        }
        Ok(())
    })
    .unwrap();
}

fn seed_active_device(db: &DbHandle, hardware_id: &str) -> SystemId {
    db.with_conn_sync(|conn| {
        Ok(ledger::insert_device(
            conn,
            &NewDevice {
                hardware_id: hardware_id.to_string(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap())
    })
    .unwrap()
}

fn device_state(db: &DbHandle, sid: &SystemId) -> DeviceState {
    db.with_conn_sync(|conn| {
        Ok(ledger::get_device(conn, sid)
            .unwrap()
            .expect("device exists")
            .state)
    })
    .unwrap()
}

fn latest_r14_detail(db: &DbHandle) -> Value {
    db.with_conn_sync(|conn| {
        let detail: String = conn
            .query_row(
                "SELECT detail FROM ledger_events WHERE kind = 'r14_op'
                 ORDER BY event_id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        Ok(serde_json::from_str(&detail).unwrap())
    })
    .unwrap()
}

fn r14_details_for_op(db: &DbHandle, op: &str) -> Vec<Value> {
    db.with_conn_sync(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT detail FROM ledger_events WHERE kind = 'r14_op'
                 ORDER BY event_id ASC",
            )
            .unwrap();
        let details = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|row| serde_json::from_str::<Value>(&row.unwrap()).unwrap())
            .filter(|detail| detail["op"] == op)
            .collect::<Vec<_>>();
        Ok(details)
    })
    .unwrap()
}

#[tokio::test]
async fn ops_catalog_dispatch_step_up_and_audit_behaviour() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    seed_sightings(
        &db,
        &[
            "rpi-local:default:i2c:0x60",
            "rpi-local:default:i2c:0x61",
            "rpi-local:default:i2c:0x62",
        ],
    );
    let dry_run_sid = seed_active_device(&db, "rpi-local:default:i2c:0x63");
    let health = Arc::new(Mutex::new(HealthState::new(90)));
    let clock_trust = prepare_owned_clock(&db);

    let handle = spawn_api_task(
        db.clone(),
        health,
        api_config(),
        "epoch-ops-test".to_string(),
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

    let unauthenticated_catalog = client
        .get(format!("{base}/api/v1/ops"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated_catalog.status(), StatusCode::UNAUTHORIZED);

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
    let session_bearer = format!("Bearer {}", session["token"].as_str().unwrap());

    let catalog: Value = client
        .get(format!("{base}/api/v1/ops"))
        .header(header::AUTHORIZATION, &session_bearer)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let catalog = catalog.as_array().unwrap();
    assert!(catalog.iter().any(|op| {
        op["name"] == "operator_token.issue"
            && op["tier"] == "construction"
            && op["params_schema"].get("required").is_some()
    }));

    let generic_secret_issue = client
        .post(format!("{base}/api/v1/ops/device.add_with_credential"))
        .header(header::AUTHORIZATION, &session_bearer)
        .json(&json!({"params": {
            "hardware_id":"generic-api-denied-operation",
            "flow_class":"default",
            "reason_code":"device_commissioning"
        }}))
        .send()
        .await
        .unwrap();
    assert_eq!(generic_secret_issue.status(), StatusCode::FORBIDDEN);
    let generic_secret_body: Value = generic_secret_issue.json().await.unwrap();
    assert_eq!(generic_secret_body["error"]["code"], "forbidden");
    db.with_conn_sync(|conn| {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM devices WHERE hardware_id='generic-api-denied-operation')",
            [],
            |row| row.get(0),
        )?;
        assert!(!exists);
        let audit: String = conn.query_row(
            "SELECT detail FROM ledger_events WHERE kind='r14_op' ORDER BY event_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        assert!(!audit.contains("ikd_"));
        assert!(!audit.contains("plaintext"));
        Ok(())
    })
    .unwrap();
    assert!(catalog.iter().any(|op| {
        op["name"] == "device.approve_sighting"
            && op["tier"] == "daily"
            && op["bulk_escalates"] == true
    }));

    let approved: Value = client
        .post(format!("{base}/api/v1/ops/device.approve_sighting"))
        .header(header::AUTHORIZATION, &session_bearer)
        .json(&json!({
            "params": {"hardware_ids": ["rpi-local:default:i2c:0x60"]}
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let setup_sid =
        SystemId::from_text(approved["approved"][0]["system_id"].as_str().unwrap()).unwrap();

    let setup_bulk = client
        .post(format!("{base}/api/v1/ops/device.approve_sighting"))
        .header(header::AUTHORIZATION, &session_bearer)
        .json(&json!({
            "params": {
                "hardware_ids": [
                    "rpi-local:default:i2c:0x61",
                    "rpi-local:default:i2c:0x62"
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(setup_bulk.status(), StatusCode::FORBIDDEN);
    let setup_bulk_body: Value = setup_bulk.json().await.unwrap();
    assert_eq!(setup_bulk_body["error"]["code"], "step_up_required");

    let setup_retire = client
        .post(format!("{base}/api/v1/ops/device.retire"))
        .json(&json!({"params": {"system_ids": [setup_sid.to_text()]}}))
        .send()
        .await
        .unwrap();
    assert_eq!(setup_retire.status(), StatusCode::UNAUTHORIZED);
    let setup_retire_body: Value = setup_retire.json().await.unwrap();
    assert_eq!(setup_retire_body["error"]["code"], "unauthorized");

    let reserved_param_passphrase = "reserved param should not be audited";
    let reserved_param = client
        .post(format!("{base}/api/v1/ops/operator_token.issue"))
        .header(header::AUTHORIZATION, &session_bearer)
        .json(&json!({
            "params": {
                "name": "reserved-param-denied",
                "kind": "ai",
                "tier_ceiling": "routine",
                "step_up_passphrase": reserved_param_passphrase
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(reserved_param.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let reserved_param_body: Value = reserved_param.json().await.unwrap();
    assert_eq!(reserved_param_body["error"]["code"], "reserved_param");
    assert_eq!(
        reserved_param_body["error"]["message"],
        "step_up_passphrase must not appear in params"
    );
    let r14_details = r14_details_for_op(&db, "operator_token.issue");
    assert!(
        r14_details
            .iter()
            .all(|detail| !detail.to_string().contains(reserved_param_passphrase)),
        "reserved params must be rejected before r14_op audit detail can store passphrases"
    );

    let retired: Value = client
        .post(format!("{base}/api/v1/ops/device.retire"))
        .header(header::AUTHORIZATION, &session_bearer)
        .json(&json!({"params": {"system_ids": [setup_sid.to_text()]}}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(retired["retired"], json!([setup_sid.to_text()]));

    let issue_without_step_up = client
        .post(format!("{base}/api/v1/ops/operator_token.issue"))
        .header(header::AUTHORIZATION, &session_bearer)
        .json(&json!({
            "params": {"name": "ai-issued", "kind": "ai", "tier_ceiling": "routine"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(issue_without_step_up.status(), StatusCode::FORBIDDEN);
    let issue_without_step_up_body: Value = issue_without_step_up.json().await.unwrap();
    assert_eq!(
        issue_without_step_up_body["error"]["code"],
        "step_up_required"
    );

    let issued_ai: Value = client
        .post(format!("{base}/api/v1/ops/operator_token.issue"))
        .header(header::AUTHORIZATION, &session_bearer)
        .json(&json!({
            "params": {"name": "ai-issued", "kind": "ai", "tier_ceiling": "routine"},
            "step_up_passphrase": "correct horse battery staple"
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let ai_token = issued_ai["plaintext"].as_str().unwrap();
    assert!(ai_token.starts_with("iko_"));
    let issue_details = r14_details_for_op(&db, "operator_token.issue");
    let issue_ok_detail = issue_details
        .iter()
        .rev()
        .find(|detail| detail["result"] == "ok")
        .unwrap();
    assert!(
        !issue_ok_detail.to_string().contains(ai_token),
        "plaintext token must not be present in r14_op audit detail"
    );
    assert!(
        !issue_ok_detail
            .to_string()
            .contains("correct horse battery staple"),
        "step_up_passphrase must not be present in r14_op audit detail"
    );

    let ai_bearer = format!("Bearer {ai_token}");
    let ai_issue = client
        .post(format!("{base}/api/v1/ops/operator_token.issue"))
        .header(header::AUTHORIZATION, &ai_bearer)
        .json(&json!({
            "params": {"name": "ai-denied", "kind": "ai", "tier_ceiling": "routine"},
            "step_up_passphrase": "correct horse battery staple"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ai_issue.status(), StatusCode::FORBIDDEN);
    let ai_issue_body: Value = ai_issue.json().await.unwrap();
    assert_eq!(ai_issue_body["error"]["message"], "tier");

    let ai_series = client
        .get(format!("{base}/api/v1/series"))
        .header(header::AUTHORIZATION, &ai_bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(ai_series.status(), StatusCode::OK);

    let state_before = device_state(&db, &dry_run_sid);
    let dry_run: Value = client
        .post(format!("{base}/api/v1/ops/device.retire"))
        .header(header::AUTHORIZATION, &session_bearer)
        .json(&json!({
            "params": {"system_ids": [dry_run_sid.to_text()]},
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
    assert_eq!(dry_run["would"], "retire_device");
    assert_eq!(device_state(&db, &dry_run_sid), state_before);
    let dry_run_detail = latest_r14_detail(&db);
    assert_eq!(dry_run_detail["op"], "device.retire");
    assert_eq!(dry_run_detail["dry_run"], true);

    let unknown = client
        .post(format!("{base}/api/v1/ops/nope.nope"))
        .header(header::AUTHORIZATION, &session_bearer)
        .json(&json!({"params": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let unknown_body: Value = unknown.json().await.unwrap();
    assert_eq!(unknown_body["error"]["code"], "unknown_op");

    let audit = latest_r14_detail(&db);
    for key in [
        "op",
        "actor",
        "actor_kind",
        "tier",
        "effective_tier",
        "dry_run",
        "result",
        "targets",
        "source",
    ] {
        assert!(audit.get(key).is_some(), "missing r14_op detail key: {key}");
    }

    shutdown(handle).await;
}
