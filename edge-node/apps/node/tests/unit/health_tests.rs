use super::*;

#[test]
fn a_new_health_state_does_not_claim_the_collector_is_running() {
    assert!(
        !HealthState::new(90).collector_alive,
        "the MQTT publisher must not report a running collector before startup marks it"
    );
}
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn all_migrations() -> Vec<iotkit_core_storage::Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.sort_by_key(|migration| migration.version);
    all
}

#[test]
fn write_health_json_uses_temp_file_then_rename() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("health.json");
    let tmp = dir.path().join("health.json.tmp");
    let state = HealthState {
        started_at: Instant::now() - Duration::from_secs(10),
        collector_alive: true,
        adapters: vec![AdapterHealth {
            id: "bravepi-mainboard".to_string(),
            alive: true,
            status: AdapterRuntimeStatus::Running,
            last_event_at: Some(1234),
        }],
        db: DbHealth {
            size_bytes: 42,
            disk_available_bytes: 1024,
            watermark_exceeded: false,
        },
        retention: RetentionHealth {
            days: 90,
            last_purge_at: Some(4567),
            last_purged_rows: 3,
        },
        publish: Vec::new(),
        api: Some(ApiHealth {
            bind: "127.0.0.1:8443".to_string(),
            tls_fingerprint: "sha256:test".to_string(),
        }),
        ingress: IngressListenerHealth::disabled(0, "disabled".into()),
        ingress_bounds: IngressBoundsHealth::unknown(),
        clock: ClockHealth {
            trusted: false,
            source: None,
            observed_at_ms: None,
        },
        device_credentials: HealthState::new(90).device_credentials,
    };

    write_health_json(&path, "epoch-1", &state).unwrap();

    assert!(path.exists());
    assert!(!tmp.exists());
    let json = std::fs::read_to_string(path).unwrap();
    assert!(json.contains(r#""schema":1"#));
    assert!(json.contains(r#""epoch":"epoch-1""#));
    assert!(json.contains(r#""collector_alive":true"#));
    assert!(json.contains(r#""id":"bravepi-mainboard""#));
    assert!(json.contains(r#""size_bytes":42"#));
    assert!(json.contains(r#""days":90"#));
    assert!(json.contains(r#""api":{"bind":"127.0.0.1:8443","tls_fingerprint":"sha256:test"}"#));
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["clock_trust"]["trusted"], false);
    assert!(parsed["clock_trust"]["source"].is_null());
    assert_eq!(
        parsed["clock_trust"]["recovery_command"],
        "iotkit-edge-nodectl time confirm"
    );
    assert!(json.contains(r#""uptime_s":10"#) || json.contains(r#""uptime_s":11"#));
}

#[test]
fn adapter_health_separates_runtime_state_from_last_activity() {
    let mut state = HealthState::new(90);
    state.note_adapter_running("line_a");
    assert_eq!(state.adapters[0].status, AdapterRuntimeStatus::Running);
    assert_eq!(state.adapters[0].last_event_at, None);

    state.note_adapter_event("line_a", 42);
    state.note_adapter_restarting("line_a");
    assert_eq!(state.adapters[0].status, AdapterRuntimeStatus::Restarting);
    assert_eq!(state.adapters[0].last_event_at, Some(42));

    state.note_adapter_exhausted("line_a");
    assert_eq!(state.adapters[0].status, AdapterRuntimeStatus::Exhausted);
    assert!(!state.adapters[0].alive);
}

#[test]
fn ingress_health_is_stable_coarse_and_secret_free() {
    let mut state = HealthState::new(90);
    state.ingress = IngressListenerHealth {
        query_state: IngressQueryState::Degraded,
        status: "degraded",
        desired_generation: Some(7),
        applied_generation: Some(6),
        bind: Some("local_ingress"),
        local_addr: None,
        mode: Some("private_plaintext"),
        desired_mode: Some("tls"),
        applied_mode: Some("private_plaintext"),
        plaintext_warning: true,
        last_error: Some("bind_failed".into()),
        last_action: "retain_last_safe".into(),
        gate_reason: Some("tls_not_ready".into()),
    };
    let rendered = render_health_json("epoch", &state);
    let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(json["ingress_listener"]["desired_generation"], 7);
    assert_eq!(json["ingress_listener"]["applied_generation"], 6);
    assert_eq!(json["ingress_listener"]["bind"], "local_ingress");
    assert_eq!(json["ingress_listener"]["plaintext_warning"], true);
    assert_eq!(json["ingress_listener"]["gate_reason"], "tls_not_ready");
    assert!(!rendered.contains("PRIVATE KEY"));
    assert!(!rendered.contains("192.168."));
}

#[test]
fn failed_tls_desired_with_plaintext_applied_reports_active_plaintext() {
    let config = ingress_config(
        iotkit_core_ops::IngressListenerMode::Tls,
        iotkit_core_ops::IngressListenerMode::PrivatePlaintext,
    );
    let health = IngressListenerHealth::blocked(config, "bind_failed");
    assert_eq!(health.desired_mode, Some("tls"));
    assert_eq!(health.applied_mode, Some("private_plaintext"));
    assert!(health.plaintext_warning);
}

#[test]
fn failed_plaintext_desired_with_tls_applied_reports_active_tls() {
    let config = ingress_config(
        iotkit_core_ops::IngressListenerMode::PrivatePlaintext,
        iotkit_core_ops::IngressListenerMode::Tls,
    );
    let health = IngressListenerHealth::blocked(config, "bind_failed");
    assert_eq!(health.desired_mode, Some("private_plaintext"));
    assert_eq!(health.applied_mode, Some("tls"));
    assert!(!health.plaintext_warning);
}

#[test]
fn invalidated_plaintext_and_tls_applied_states_report_desired_only_unbound_health() {
    for mode in [
        iotkit_core_ops::IngressListenerMode::PrivatePlaintext,
        iotkit_core_ops::IngressListenerMode::Tls,
    ] {
        let health =
            IngressListenerHealth::invalidated(ingress_config(mode, mode), "authority_invalidated");
        assert_eq!(health.status, "unbound");
        assert_eq!(health.desired_generation, Some(2));
        assert_eq!(health.desired_mode, Some(ingress_mode(mode)));
        assert_eq!(health.applied_generation, None);
        assert_eq!(health.bind, None);
        assert_eq!(health.mode, None);
        assert_eq!(health.applied_mode, None);
        assert!(!health.plaintext_warning);
    }
}

fn ingress_config(
    desired_mode: iotkit_core_ops::IngressListenerMode,
    applied_mode: iotkit_core_ops::IngressListenerMode,
) -> iotkit_core_ops::IngressListenerConfig {
    let state = |generation, mode| iotkit_core_ops::IngressListenerState {
        generation,
        bind_addr: "192.168.1.2:8444".into(),
        interface: "eth0".into(),
        local_ingress_cidrs: vec!["192.168.1.0/24".into()],
        mode,
        tls_generation: (mode == iotkit_core_ops::IngressListenerMode::Tls).then_some(generation),
        tls_fingerprint: (mode == iotkit_core_ops::IngressListenerMode::Tls)
            .then(|| "fingerprint".into()),
    };
    iotkit_core_ops::IngressListenerConfig {
        enabled: true,
        desired: state(2, desired_mode),
        applied: Some(state(1, applied_mode)),
        last_error: None,
        last_action: "apply_failed".into(),
    }
}

#[test]
fn device_credential_health_is_bounded_and_names_plan_6_5_recovery() {
    let mut state = HealthState::new(90);
    state.device_credentials = DeviceCredentialHealth {
        query_state: CredentialHealthQueryState::Ok,
        active_count: Some(10_000),
        stale_count: Some(10_000),
        counts_capped: Some(true),
        capacity_required_steady: Some(120),
        capacity_required_burst: Some(130),
        capacity_steady: Some(100),
        capacity_burst: Some(100),
        capacity_debt: Some(true),
        replacement_backup_unavailable: Some(true),
    };
    let rendered = render_health_json("test-epoch", &state);
    serde_json::from_str::<serde_json::Value>(&rendered).unwrap();
    assert!(rendered.contains(r#""active_count":10000"#));
    assert!(rendered.contains(r#""counts_capped":true"#));
    assert!(rendered.contains(r#""debt":true"#));
    assert!(rendered.contains(r#""replacement_backup_unavailable":true"#));
    assert!(rendered.contains("Install Plan 6.5 encrypted replacement backup support"));
    assert!(!rendered.contains("token_hash"));
    assert!(!rendered.contains("principal_id"));
}

#[test]
fn failed_health_refresh_is_loud_and_preserves_last_good_replacement_alert() {
    let mut state = HealthState::new(90);
    let unknown = render_health_json("epoch", &state);
    let unknown: serde_json::Value = serde_json::from_str(&unknown).unwrap();
    assert_eq!(unknown["device_credentials"]["query_state"], "unknown");
    assert!(unknown["device_credentials"]["replacement_backup_unavailable"].is_null());

    state.device_credentials = DeviceCredentialHealth {
        query_state: CredentialHealthQueryState::Ok,
        active_count: Some(1),
        stale_count: Some(0),
        counts_capped: Some(false),
        capacity_required_steady: Some(1),
        capacity_required_burst: Some(1),
        capacity_steady: Some(1),
        capacity_burst: Some(1),
        capacity_debt: Some(false),
        replacement_backup_unavailable: Some(true),
    };
    state.mark_device_credential_health_failed();
    let degraded: serde_json::Value =
        serde_json::from_str(&render_health_json("epoch", &state)).unwrap();
    assert_eq!(degraded["device_credentials"]["query_state"], "degraded");
    assert_eq!(
        degraded["device_credentials"]["replacement_backup_unavailable"],
        true
    );
    assert!(
        degraded["device_credentials"]["recovery_action"]
            .as_str()
            .unwrap()
            .contains("Plan 6.5")
    );
}

#[test]
fn failed_refresh_drops_last_good_safe_booleans_but_keeps_unsafe_alerts() {
    let mut state = HealthState::new(90);
    state.device_credentials = DeviceCredentialHealth {
        query_state: CredentialHealthQueryState::Ok,
        active_count: Some(0),
        stale_count: Some(0),
        counts_capped: Some(false),
        capacity_required_steady: Some(0),
        capacity_required_burst: Some(0),
        capacity_steady: Some(1),
        capacity_burst: Some(1),
        capacity_debt: Some(false),
        replacement_backup_unavailable: Some(false),
    };
    state.mark_device_credential_health_failed();
    let degraded: serde_json::Value =
        serde_json::from_str(&render_health_json("epoch", &state)).unwrap();
    assert_eq!(degraded["device_credentials"]["query_state"], "degraded");
    assert!(degraded["device_credentials"]["capacity"]["debt"].is_null());
    assert!(degraded["device_credentials"]["replacement_backup_unavailable"].is_null());
    assert!(
        degraded["device_credentials"]["recovery_action"]
            .as_str()
            .unwrap()
            .contains("Plan 6.5")
    );

    state.device_credentials.capacity_debt = Some(true);
    state.device_credentials.replacement_backup_unavailable = Some(true);
    state.mark_device_credential_health_failed();
    let unsafe_snapshot: serde_json::Value =
        serde_json::from_str(&render_health_json("epoch", &state)).unwrap();
    assert_eq!(
        unsafe_snapshot["device_credentials"]["capacity"]["debt"],
        true
    );
    assert_eq!(
        unsafe_snapshot["device_credentials"]["replacement_backup_unavailable"],
        true
    );

    state.apply_device_credential_health(
        iotkit_core_ops::StaleCredentialHealth {
            active_count: 0,
            stale_count: 0,
            counts_capped: false,
        },
        iotkit_core_ops::CapacityHealth {
            status: iotkit_core_ops::CapacityStatus {
                required_steady_units: 0,
                required_burst_units: 0,
                capacity_steady_units: 1,
                capacity_burst_units: 1,
            },
            active_debt: false,
        },
        iotkit_core_ops::device_credentials::ReplacementBackupHealth {
            replacement_backup_unavailable: false,
            recovery_action: None,
        },
    );
    assert_eq!(state.device_credentials.capacity_debt, Some(false));
    assert_eq!(
        state.device_credentials.replacement_backup_unavailable,
        Some(false)
    );
}

#[tokio::test]
async fn health_writer_database_failure_publishes_unknown_not_false_safe_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("health-failure.db");
    let health_path = dir.path().join("health.json");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let trust = db
        .with_conn_sync(|conn| {
            Ok(Arc::new(
                iotkit_core_ops::ClockTrust::load(
                    conn,
                    Arc::new(iotkit_core_ops::SystemClock::default()),
                    Duration::from_secs(1),
                    Duration::from_secs(60),
                )
                .unwrap(),
            ))
        })
        .unwrap();
    db.with_conn_sync(|conn| {
        conn.execute_batch("DROP TABLE device_capacity")?;
        Ok(())
    })
    .unwrap();
    let state = Arc::new(Mutex::new(HealthState::new(90)));
    let task = spawn_health_writer(
        health_path.clone(),
        "epoch".into(),
        state,
        trust,
        db,
        Duration::from_millis(10),
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while !health_path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(health_path).unwrap()).unwrap();
    assert_eq!(json["device_credentials"]["query_state"], "unknown");
    assert!(json["device_credentials"]["replacement_backup_unavailable"].is_null());
    assert!(
        json["device_credentials"]["recovery_action"]
            .as_str()
            .unwrap()
            .contains("unknown")
    );
}

#[test]
fn ingress_bound_health_is_aggregate_actionable_and_serialized_size_is_bounded() {
    let mut state = HealthState::new(90);
    for i in 0_i64..10_000 {
        state.adapters.push(AdapterHealth {
            id: if i == 0 {
                format!("HOSTILE_MARKER{}", "x".repeat(100_000))
            } else {
                format!("adapter-{i}")
            },
            alive: true,
            status: AdapterRuntimeStatus::Running,
            last_event_at: Some(i),
        });
        state.publish.push(TargetDeliveryHealth {
            target_id: format!("target-{i}"),
            cursor_pub_seq: i,
            backlog: i,
            last_push_at: None,
            last_error: (i == 0).then(|| format!("HOSTILE_ERROR{}", "y".repeat(100_000))),
        });
    }
    state.ingress_bounds = IngressBoundsHealth {
        query_state: IngressQueryState::Degraded,
        throttled_drop_count: u64::MAX,
        throttle_active: true,
        queue_current: 15,
        queue_high_water: 16,
        queue_pressure_percent: 93,
        auth_pressure_percent: 75,
        global_flow_pressure_percent: 80,
        principal_pressure_percent: 90,
        request_pressure_percent: 93,
        connection_pressure_percent: 87,
        staging_rows: Some(10_000),
        staging_bytes: Some(64 * 1024 * 1024),
        staging_pinned_rows: Some(9_744),
        dedup_rows: Some(100_000),
        dedup_max_principal_rows: Some(10_000),
        dedup_degraded: Some(true),
    };
    state.device_credentials.capacity_debt = Some(true);

    let rendered = render_health_json("epoch", &state);
    let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert!(rendered.len() < 64 * 1024);
    assert_eq!(json["adapters"].as_array().unwrap().len(), 64);
    assert_eq!(json["publish"].as_array().unwrap().len(), 64);
    assert_eq!(json["ingress_bounds"]["throttled_drop_count"], u64::MAX);
    assert_eq!(json["ingress_bounds"]["throttle_active"], true);
    assert_eq!(json["ingress_bounds"]["queue_current"], 15);
    assert_eq!(
        json["ingress_bounds"]["class_pressure_percent"]["principal"],
        90
    );
    assert_eq!(
        json["ingress_bounds"]["class_pressure_percent"]["queue"],
        93
    );
    assert!(
        json["device_credentials"]["capacity"]["debt_action"]
            .as_str()
            .unwrap()
            .contains("construction-tier")
    );
    assert_eq!(json["ingress_bounds"]["dedup"]["degraded"], true);
    assert!(
        json["ingress_bounds"]["recovery_action"]
            .as_str()
            .unwrap()
            .contains("retention")
    );
    assert!(!rendered.contains("token_id"));
    assert!(!rendered.contains("source_id"));
    assert!(!rendered.contains("HOSTILE_MARKER"));
    assert!(!rendered.contains("HOSTILE_ERROR"));
}

#[test]
fn persisted_staging_and_dedup_health_populate_aggregate_state() {
    let mut state = HealthState::new(90);
    state.apply_ingress_persistence_health(
        iotkit_core_timeseries::StagingHealth {
            rows: 7,
            bytes: 700,
            pinned_rows: 2,
            pinned_bytes: 200,
            principals: 3,
        },
        iotkit_core_timeseries::DedupHealth {
            rows: 9,
            max_principal_rows: 4,
            oldest_age_ms: 50,
        },
        iotkit_core_timeseries::DedupMaintenanceHealth {
            degraded: true,
            episode_started_at: Some(10),
            last_failure_at: Some(20),
            last_success_at: None,
        },
    );
    assert_eq!(
        state.ingress_bounds.query_state,
        IngressQueryState::Degraded
    );
    assert_eq!(state.ingress_bounds.staging_rows, Some(7));
    assert_eq!(state.ingress_bounds.dedup_rows, Some(9));
    assert_eq!(state.ingress_bounds.dedup_degraded, Some(true));
}
