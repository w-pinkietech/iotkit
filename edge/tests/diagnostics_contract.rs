use iotkit_edge::{
    application::{
        output_profiles::OutputProfiles,
        semantics::{SemanticRuleDraft, Semantics},
    },
    composition::registered_output_adapters,
    diagnostics::{
        DiagnosticStageKind, DiagnosticStageState, DiagnosticState,
        POSTGRES_DIAGNOSTIC_OUTPUT_DELIVERY_LATEST_SQL,
        POSTGRES_DIAGNOSTIC_OUTPUT_PENDING_OLDEST_SQL, POSTGRES_DIAGNOSTIC_PROJECTION_LATEST_SQL,
        SQLITE_DIAGNOSTIC_OUTPUT_DELIVERY_LATEST_SQL, SQLITE_DIAGNOSTIC_OUTPUT_PENDING_OLDEST_SQL,
        SQLITE_DIAGNOSTIC_PROJECTION_LATEST_SQL, StorageState, diagnostics,
        diagnostics_with_certificate, diagnostics_with_runtime, storage_status,
    },
    mqtt::ingest::{IngestConnectionState, IngestRuntimeHealth},
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{
        AcceptBatch, POSTGRES_DIAGNOSTIC_SIGNAL_RECEIPT_SQL, RawRecord,
        SQLITE_DIAGNOSTIC_SIGNAL_RECEIPTS_SQL, Storage, StorageProfile,
    },
};
use iotkit_edge_custody_contract::{
    AdapterState, CollectorState, DescriptorSnapshot, StatusAdapter, StatusHeartbeat,
};
use serde_json::Map;
use sqlx::{PgPool, Row};
use tempfile::TempDir;

const DIAGNOSTIC_NOW: i64 = 1_000_000;

async fn causal_storage() -> (TempDir, Storage, sqlx::SqlitePool) {
    let directory = TempDir::new().unwrap();
    let database = directory.path().join("edge.db");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: database.clone(),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", database.display()))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,created_at,updated_at) \
         VALUES('node-ref','node','epoch','active',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    (directory, storage, pool)
}

fn ready_health() -> IngestRuntimeHealth {
    IngestRuntimeHealth {
        state: IngestConnectionState::Ready,
        last_ready_at: Some(DIAGNOSTIC_NOW),
    }
}

fn heartbeat(accepted_through: i64, pending_publications: i64) -> StatusHeartbeat {
    heartbeat_with_sequence(accepted_through, pending_publications, 1)
}

fn heartbeat_with_sequence(
    accepted_through: i64,
    pending_publications: i64,
    status_seq: u64,
) -> StatusHeartbeat {
    StatusHeartbeat {
        schema_version: 1,
        edge_node_id: "node".into(),
        ledger_epoch: "epoch".into(),
        boot_id: "boot-0123456789abcdef0123456789abcdef".into(),
        status_seq,
        collector_state: CollectorState::Running,
        adapters: vec![StatusAdapter {
            adapter_id: "adapter-1".into(),
            state: AdapterState::Running,
        }],
        accepted_through,
        pending_publications,
        storage_pressure: false,
    }
}

async fn insert_current_signal(pool: &sqlx::SqlitePool, signal_ref: &str, series_key: &str) {
    sqlx::query(
        "INSERT INTO descriptor_signals(edge_node_id,series_key,system_id,measurement_key,variant,value_type,presence,descriptor_revision,updated_at) \
         VALUES('node',?,'system','temperature','primary','float','current',1,1)",
    )
    .bind(series_key)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO inventory_signals(signal_ref,edge_node_id,series_key,system_id,created_at) \
         VALUES(?,'node',?,'system',1)",
    )
    .bind(signal_ref)
    .bind(series_key)
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_raw(pool: &sqlx::SqlitePool, series_key: &str, pub_seq: i64, received_at: i64) {
    insert_raw_for_epoch(pool, "node", "epoch", series_key, pub_seq, received_at).await;
}

async fn insert_raw_for_epoch(
    pool: &sqlx::SqlitePool,
    edge_node_id: &str,
    ledger_epoch: &str,
    series_key: &str,
    pub_seq: i64,
    received_at: i64,
) {
    sqlx::query(
        "INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,publication_id,record_json,record_sha256,received_at,series_key) \
         VALUES(?,?,?,?,'{}',?, ?,?)",
    )
    .bind(edge_node_id)
    .bind(ledger_epoch)
    .bind(pub_seq)
    .bind(format!("publication-{pub_seq}"))
    .bind(vec![0_u8; 32])
    .bind(received_at)
    .bind(series_key)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn diagnostic_sensor_receipts_ignore_inactive_nodes_and_old_epochs() {
    let (_directory, storage, pool) = causal_storage().await;
    storage
        .apply_edge_node_status(&heartbeat(0, 0), DIAGNOSTIC_NOW, false)
        .await
        .unwrap();
    insert_current_signal(&pool, "active-signal", "active-series").await;
    insert_raw_for_epoch(
        &pool,
        "node",
        "old-epoch",
        "active-series",
        1,
        DIAGNOSTIC_NOW,
    )
    .await;
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,created_at,updated_at) \
         VALUES('inactive-ref','inactive-node','inactive-epoch','discovered',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO descriptor_signals(edge_node_id,series_key,system_id,measurement_key,variant,value_type,presence,descriptor_revision,updated_at) \
         VALUES('inactive-node','inactive-series','system','temperature','primary','float','current',1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO inventory_signals(signal_ref,edge_node_id,series_key,system_id,created_at) \
         VALUES('inactive-signal','inactive-node','inactive-series','system',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    insert_raw_for_epoch(
        &pool,
        "inactive-node",
        "inactive-epoch",
        "inactive-series",
        1,
        DIAGNOSTIC_NOW,
    )
    .await;

    assert_eq!(
        storage.diagnostic_signal_receipts(65).await.unwrap(),
        vec![None],
        "only the current active activation and its current epoch are diagnostic input"
    );
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    assert_ne!(
        stage(&report, DiagnosticStageKind::Sensor).state,
        DiagnosticStageState::Ok,
        "old or inactive raw history must not make the active sensor healthy"
    );
}

fn stage(
    report: &iotkit_edge::diagnostics::DiagnosticReport,
    kind: DiagnosticStageKind,
) -> &iotkit_edge::diagnostics::DiagnosticStage {
    report
        .stages
        .iter()
        .find(|stage| stage.stage == kind)
        .unwrap_or_else(|| panic!("missing {kind:?} stage"))
}

#[tokio::test]
async fn sqlite_capacity_reports_current_projection_queue_and_output_tables() {
    let directory = TempDir::new().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("edge.db"),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let descriptor = DescriptorSnapshot::decode(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "edge_node_id": "node",
            "ledger_epoch": "epoch",
            "descriptor_revision": 1,
            "complete": true,
            "devices": [{
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "identifier": "diagnostic-device",
                "state": "active",
                "model_id": "contract"
            }],
            "signals": [{
                "series_key": "018f0000-0000-7000-8000-000000000001:temperature:na:primary",
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "measurement_key": "temperature",
                "channel_index": null,
                "variant": "primary",
                "unit": null,
                "value_type": "float"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    storage.apply_descriptor(&descriptor, 2).await.unwrap();
    let semantics = Semantics::new(storage.clone());
    semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "node".into(),
                series_key: descriptor.signals[0].series_key.clone(),
                display_name: "Diagnostic temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            3,
        )
        .await
        .unwrap();
    OutputProfiles::new(storage.clone(), registered_output_adapters())
        .activate("Diagnostic output", "iotkit.mqtt-json.v1", Map::new(), 4)
        .await
        .unwrap();
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "node".into(),
            ledger_epoch: "epoch".into(),
            publication_id: "publication".into(),
            received_at: 5,
            records: vec![
                RawRecord::new(
                    1,
                    serde_json::to_vec(&serde_json::json!({
                        "family": "measurement",
                        "schema_version": 1,
                        "epoch": "epoch",
                        "pub_seq": 1,
                        "series_key": descriptor.signals[0].series_key,
                        "values": [21.5],
                        "event_time": 5,
                        "event_time_source": "received_at",
                        "time_source": "edge_node",
                        "time_quality": "unsynced",
                        "received_at": 5,
                        "device_time": null
                    }))
                    .unwrap(),
                )
                .unwrap(),
            ],
        })
        .await
        .unwrap();

    let capacity = storage_status(&storage, 90).await.unwrap();
    assert!(capacity.filesystem_available);
    assert!(matches!(
        capacity.state,
        StorageState::Healthy | StorageState::Warning | StorageState::Critical
    ));
    assert_eq!(capacity.raw_record_count, 1);
    assert_eq!(capacity.semantic_observation_count, 0);
    assert_eq!(capacity.pending_semantic_projection_count, 1);
    assert_eq!(capacity.pending_output_count, 0);
    semantics
        .project_pending(1, registered_output_adapters())
        .await
        .unwrap();
    let capacity = storage_status(&storage, 90).await.unwrap();
    assert_eq!(capacity.semantic_observation_count, 1);
    assert_eq!(capacity.pending_semantic_projection_count, 0);
    assert_eq!(capacity.pending_output_count, 1);

    let report = diagnostics(&storage, 90, 300_006).await.unwrap();
    assert_eq!(report.state, DiagnosticState::Attention);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "edge_backup_missing")
    );
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "output_delivery_stale")
    );
}

#[tokio::test]
async fn invalid_warning_threshold_is_rejected() {
    let directory = TempDir::new().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("edge.db"),
    })
    .await
    .unwrap();
    assert!(storage_status(&storage, 49).await.is_err());
    assert!(storage_status(&storage, 100).await.is_err());
}

#[tokio::test]
async fn recovery_and_certificate_causes_are_visible() {
    let directory = TempDir::new().unwrap();
    let database = directory.path().join("edge.db");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: database.clone(),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", database.display()))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,\
         created_at,updated_at) VALUES('node-ref','node','epoch','recovery_hold',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO edge_restore_events VALUES('restore','backup',1,1,(SELECT edge_id FROM \
         edge_meta),5,?)",
    )
    .bind("0".repeat(64))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO edge_restore_cursor_checks(restore_id,edge_node_id,ledger_epoch,\
         backup_accepted_through,state,observed_cursor_start,updated_at) \
         VALUES('restore','node','epoch',1,'recovery_required',5,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let report = diagnostics_with_certificate(
        &storage,
        90,
        1_000_000,
        Some(&directory.path().join("missing.pem")),
    )
    .await
    .unwrap();
    assert_eq!(report.state, DiagnosticState::Critical);
    let certificate = report
        .broker_certificate
        .as_ref()
        .expect("certificate view");
    assert!(!certificate.available);
    assert!(certificate.needs_action);
    for code in [
        "edge_node_recovery_hold",
        "archive_recovery_required",
        "broker_certificate_unavailable",
    ] {
        assert!(
            report.issues.iter().any(|issue| issue.code == code),
            "missing diagnostic issue {code}"
        );
    }
}

#[tokio::test]
async fn broker_outage_blocks_node_age_and_keeps_one_causal_root() {
    let (_directory, storage, _pool) = causal_storage().await;
    storage
        .apply_edge_node_status(&heartbeat(0, 0), DIAGNOSTIC_NOW - 400_000, false)
        .await
        .unwrap();

    let report = diagnostics_with_runtime(
        &storage,
        90,
        DIAGNOSTIC_NOW,
        None,
        IngestRuntimeHealth {
            state: IngestConnectionState::Disconnected,
            last_ready_at: Some(DIAGNOSTIC_NOW - 1),
        },
    )
    .await
    .unwrap();

    let broker = stage(&report, DiagnosticStageKind::Broker);
    assert_eq!(broker.state, DiagnosticStageState::Critical);
    assert_eq!(broker.code, "broker_disconnected");
    let node = stage(&report, DiagnosticStageKind::Node);
    assert_eq!(node.state, DiagnosticStageState::Unknown);
    assert_eq!(node.code, "node_blocked_by_broker");
    assert_eq!(node.blocked_by, Some(DiagnosticStageKind::Broker));
    assert_eq!(
        stage(&report, DiagnosticStageKind::Adapter).blocked_by,
        Some(DiagnosticStageKind::Node)
    );
    assert_eq!(
        stage(&report, DiagnosticStageKind::Sensor).blocked_by,
        Some(DiagnosticStageKind::Adapter)
    );
}

#[tokio::test]
async fn one_stale_signal_is_advisory_even_when_another_is_fresh() {
    let (_directory, storage, pool) = causal_storage().await;
    storage
        .apply_edge_node_status(&heartbeat(0, 0), DIAGNOSTIC_NOW, false)
        .await
        .unwrap();
    insert_current_signal(&pool, "sig-fresh", "system:temperature:na:primary").await;
    insert_current_signal(&pool, "sig-stale", "system:humidity:na:primary").await;
    insert_raw(
        &pool,
        "system:temperature:na:primary",
        1,
        DIAGNOSTIC_NOW - 1,
    )
    .await;
    insert_raw(
        &pool,
        "system:humidity:na:primary",
        2,
        DIAGNOSTIC_NOW - 300_001,
    )
    .await;

    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    let sensor = stage(&report, DiagnosticStageKind::Sensor);
    assert_eq!(sensor.state, DiagnosticStageState::Warning);
    assert_eq!(sensor.code, "sensor_no_new_input_advisory");
    assert_eq!(sensor.affected_count, 1);
}

#[tokio::test]
async fn cursor_direction_distinguishes_convergence_from_conflict() {
    let (_directory, storage, pool) = causal_storage().await;
    storage
        .apply_edge_node_status(&heartbeat(10, 3), DIAGNOSTIC_NOW, false)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO accepted_cursors(edge_node_id,ledger_epoch,accepted_through,updated_at) \
         VALUES('node','epoch',12,?)",
    )
    .bind(DIAGNOSTIC_NOW)
    .execute(&pool)
    .await
    .unwrap();

    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    let raw = stage(&report, DiagnosticStageKind::RawCustody);
    assert_eq!(raw.state, DiagnosticStageState::Ok);
    assert_eq!(raw.code, "raw_custody_cursor_converging");

    sqlx::query(
        "UPDATE accepted_cursors SET accepted_through=9,updated_at=? \
         WHERE edge_node_id='node' AND ledger_epoch='epoch'",
    )
    .bind(DIAGNOSTIC_NOW)
    .execute(&pool)
    .await
    .unwrap();
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    let raw = stage(&report, DiagnosticStageKind::RawCustody);
    assert_eq!(raw.state, DiagnosticStageState::Critical);
    assert_eq!(raw.code, "raw_custody_cursor_conflict");
}

#[tokio::test]
async fn raw_custody_last_success_requires_durable_cursor_progress() {
    let (_directory, storage, pool) = causal_storage().await;
    storage
        .apply_edge_node_status(&heartbeat_with_sequence(4, 0, 1), DIAGNOSTIC_NOW, false)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO accepted_cursors(edge_node_id,ledger_epoch,accepted_through,updated_at) \
         VALUES('node','epoch',4,?)",
    )
    .bind(DIAGNOSTIC_NOW - 100)
    .execute(&pool)
    .await
    .unwrap();

    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&report, DiagnosticStageKind::RawCustody).last_success_at,
        Some(DIAGNOSTIC_NOW - 100)
    );

    // A newer heartbeat is Node liveness, not an application acceptance.
    storage
        .apply_edge_node_status(&heartbeat_with_sequence(4, 0, 2), DIAGNOSTIC_NOW + 1, false)
        .await
        .unwrap();
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW + 1, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&report, DiagnosticStageKind::RawCustody).last_success_at,
        Some(DIAGNOSTIC_NOW - 100)
    );

    sqlx::query(
        "UPDATE accepted_cursors SET updated_at=? \
         WHERE edge_node_id='node' AND ledger_epoch='epoch'",
    )
    .bind(DIAGNOSTIC_NOW + 2)
    .execute(&pool)
    .await
    .unwrap();
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW + 2, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&report, DiagnosticStageKind::RawCustody).last_success_at,
        Some(DIAGNOSTIC_NOW + 2)
    );
}

#[tokio::test]
async fn newly_pending_work_does_not_inherit_an_idle_cursor_age() {
    let (_directory, storage, pool) = causal_storage().await;
    storage
        .apply_edge_node_status(
            &heartbeat_with_sequence(0, 0, 1),
            DIAGNOSTIC_NOW - 1_000_000,
            false,
        )
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO accepted_cursors(edge_node_id,ledger_epoch,accepted_through,updated_at) \
         VALUES('node','epoch',0,?)",
    )
    .bind(DIAGNOSTIC_NOW - 1_000_000)
    .execute(&pool)
    .await
    .unwrap();
    storage
        .apply_edge_node_status(&heartbeat_with_sequence(0, 1, 2), DIAGNOSTIC_NOW, false)
        .await
        .unwrap();

    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&report, DiagnosticStageKind::RawCustody).code,
        "raw_custody_current"
    );

    let later = DIAGNOSTIC_NOW + 300_001;
    storage
        .apply_edge_node_status(&heartbeat_with_sequence(0, 1, 3), later, false)
        .await
        .unwrap();
    let report = diagnostics_with_runtime(&storage, 90, later, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&report, DiagnosticStageKind::RawCustody).code,
        "raw_custody_pending_stalled"
    );

    storage
        .apply_edge_node_status(&heartbeat_with_sequence(0, 0, 4), later + 1, false)
        .await
        .unwrap();
    let report = diagnostics_with_runtime(&storage, 90, later + 1, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&report, DiagnosticStageKind::RawCustody).code,
        "raw_custody_current"
    );
}

#[tokio::test]
async fn cursor_progress_restarts_the_pending_stall_window() {
    let (_directory, storage, pool) = causal_storage().await;
    let mut initial = heartbeat_with_sequence(10, 3, 1);
    initial.accepted_through = 10;
    storage
        .apply_edge_node_status(&initial, DIAGNOSTIC_NOW - 600_000, false)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO accepted_cursors(edge_node_id,ledger_epoch,accepted_through,updated_at) \
         VALUES('node','epoch',10,?)",
    )
    .bind(DIAGNOSTIC_NOW - 600_000)
    .execute(&pool)
    .await
    .unwrap();

    let progressed = heartbeat_with_sequence(11, 3, 2);
    storage
        .apply_edge_node_status(&progressed, DIAGNOSTIC_NOW - 1, false)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE accepted_cursors SET accepted_through=11,updated_at=? \
         WHERE edge_node_id='node' AND ledger_epoch='epoch'",
    )
    .bind(DIAGNOSTIC_NOW - 1)
    .execute(&pool)
    .await
    .unwrap();

    let current = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&current, DiagnosticStageKind::RawCustody).code,
        "raw_custody_current",
        "continued pending work is not stalled immediately after accepted-through progress"
    );

    let later = DIAGNOSTIC_NOW + 300_001;
    storage
        .apply_edge_node_status(&heartbeat_with_sequence(11, 3, 3), later, false)
        .await
        .unwrap();
    let stalled = diagnostics_with_runtime(&storage, 90, later, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&stalled, DiagnosticStageKind::RawCustody).code,
        "raw_custody_pending_stalled"
    );
}

#[tokio::test]
async fn node_heartbeat_thresholds_are_inclusive_and_stale_reports_keep_their_timestamp() {
    let (_directory, storage, _pool) = causal_storage().await;
    let received_at = DIAGNOSTIC_NOW - 90_000;
    storage
        .apply_edge_node_status(&heartbeat(0, 0), received_at, false)
        .await
        .unwrap();

    let warning = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    let warning_node = stage(&warning, DiagnosticStageKind::Node);
    assert_eq!(warning_node.code, "node_heartbeat_stale_warning");
    assert_eq!(warning_node.last_success_at, Some(received_at));

    let critical =
        diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW + 210_000, None, ready_health())
            .await
            .unwrap();
    let critical_node = stage(&critical, DiagnosticStageKind::Node);
    assert_eq!(critical_node.code, "node_heartbeat_stale_critical");
    assert_eq!(critical_node.last_success_at, Some(received_at));
}

#[tokio::test]
async fn explicit_node_and_adapter_failures_do_not_borrow_a_healthy_last_success() {
    let (_directory, storage, pool) = causal_storage().await;
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,created_at,updated_at) \
         VALUES('other-ref','other-node','other-epoch','active',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut stopped_node = heartbeat(0, 0);
    stopped_node.collector_state = CollectorState::Stopped;
    storage
        .apply_edge_node_status(&stopped_node, DIAGNOSTIC_NOW - 1, false)
        .await
        .unwrap();
    let mut healthy_node = heartbeat(0, 0);
    healthy_node.edge_node_id = "other-node".into();
    healthy_node.ledger_epoch = "other-epoch".into();
    storage
        .apply_edge_node_status(&healthy_node, DIAGNOSTIC_NOW, false)
        .await
        .unwrap();

    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    let node = stage(&report, DiagnosticStageKind::Node);
    assert_eq!(node.code, "node_collector_stopped");
    assert_eq!(node.last_success_at, None);

    for (sequence, state, code) in [
        (2, AdapterState::Stopped, "adapter_stopped"),
        (3, AdapterState::Restarting, "adapter_restarting"),
        (4, AdapterState::Exhausted, "adapter_exhausted"),
    ] {
        let mut heartbeat = heartbeat(0, 0);
        heartbeat.status_seq = sequence;
        heartbeat.adapters[0].state = state;
        storage
            .apply_edge_node_status(&heartbeat, DIAGNOSTIC_NOW + sequence as i64, false)
            .await
            .unwrap();
        let report = diagnostics_with_runtime(
            &storage,
            90,
            DIAGNOSTIC_NOW + sequence as i64,
            None,
            ready_health(),
        )
        .await
        .unwrap();
        let adapter = stage(&report, DiagnosticStageKind::Adapter);
        assert_eq!(adapter.code, code);
        assert_eq!(adapter.last_success_at, None);
    }

    let mut running = heartbeat(0, 0);
    running.status_seq = 5;
    storage
        .apply_edge_node_status(&running, DIAGNOSTIC_NOW + 5, false)
        .await
        .unwrap();
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW + 5, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&report, DiagnosticStageKind::Adapter).last_success_at,
        Some(DIAGNOSTIC_NOW + 5)
    );
}

#[tokio::test]
async fn projection_failure_recovers_only_after_later_same_rule_epoch_success() {
    let (_directory, storage, pool) = causal_storage().await;
    storage
        .apply_edge_node_status(&heartbeat(0, 0), DIAGNOSTIC_NOW, false)
        .await
        .unwrap();
    insert_current_signal(&pool, "sig-rule", "system:pressure:na:primary").await;
    insert_raw(
        &pool,
        "system:pressure:na:primary",
        1,
        DIAGNOSTIC_NOW - 300_001,
    )
    .await;
    sqlx::query(
        "INSERT INTO semantic_signals(signal_ref,edge_node_id,series_key,created_at) \
         VALUES('sig-rule','node','system:pressure:na:primary',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_rules(rule_id,signal_ref,display_name,kind,series_id,revision,spec_json,active,created_at) \
         VALUES('rule-1','sig-rule','Rule','numeric','series-1',1,'{}',1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_projection_failures(rule_id,ledger_epoch,pub_seq,error_code,attempts,last_failed_at) \
         VALUES('rule-1','epoch',10,'projection_failed',1,?)",
    )
    .bind(DIAGNOSTIC_NOW)
    .execute(&pool)
    .await
    .unwrap();

    let projection_code = |report: &iotkit_edge::diagnostics::DiagnosticReport| {
        stage(report, DiagnosticStageKind::Projection).code.clone()
    };
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&report, DiagnosticStageKind::Sensor).code,
        "sensor_no_new_input_advisory"
    );
    assert_eq!(projection_code(&report), "projection_active_failure");

    insert_observation(&pool, "other-epoch", "other", 11, 1).await;
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    assert_eq!(projection_code(&report), "projection_active_failure");

    insert_observation(&pool, "earlier-seq", "epoch", 10, 2).await;
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    assert_eq!(projection_code(&report), "projection_active_failure");

    insert_observation(&pool, "recovered", "epoch", 11, 3).await;
    insert_raw(&pool, "system:pressure:na:primary", 2, DIAGNOSTIC_NOW).await;
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    assert_eq!(projection_code(&report), "projection_current");

    // A recovery rotates the activation epoch. A historical failure from the
    // old epoch remains evidence, but cannot keep the current active rule in
    // a critical state forever.
    sqlx::query(
        "INSERT INTO semantic_projection_failures(rule_id,ledger_epoch,pub_seq,error_code,attempts,last_failed_at) \
         VALUES('rule-1','epoch',20,'projection_failed',1,?)",
    )
    .bind(DIAGNOSTIC_NOW)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE edge_node_activations SET ledger_epoch='epoch-next' WHERE edge_node_id='node'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut next_epoch = heartbeat_with_sequence(0, 0, 1);
    next_epoch.ledger_epoch = "epoch-next".into();
    next_epoch.boot_id = "boot-abcdefabcdefabcdefabcdefabcdefab".into();
    storage
        .apply_edge_node_status(&next_epoch, DIAGNOSTIC_NOW + 1, false)
        .await
        .unwrap();
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW + 1, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        projection_code(&report),
        "projection_blocked_by_no_new_input",
        "an old-epoch failure is cleared, but old-epoch raw history cannot make the new activation healthy"
    );

    insert_raw_for_epoch(
        &pool,
        "node",
        "epoch-next",
        "system:pressure:na:primary",
        1,
        DIAGNOSTIC_NOW + 1,
    )
    .await;
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW + 1, None, ready_health())
        .await
        .unwrap();
    assert_eq!(projection_code(&report), "projection_current");
}

#[tokio::test]
async fn durable_output_puback_wait_remains_visible_when_projection_failed() {
    let (_directory, storage, pool) = causal_storage().await;
    storage
        .apply_edge_node_status(&heartbeat(0, 0), DIAGNOSTIC_NOW, false)
        .await
        .unwrap();
    insert_current_signal(&pool, "sig-output", "system:flow:na:primary").await;
    insert_raw(&pool, "system:flow:na:primary", 1, DIAGNOSTIC_NOW).await;
    sqlx::query(
        "INSERT INTO semantic_signals(signal_ref,edge_node_id,series_key,created_at) \
         VALUES('sig-output','node','system:flow:na:primary',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_rules(rule_id,signal_ref,display_name,kind,series_id,revision,spec_json,active,created_at) \
         VALUES('rule-output','sig-output','Output rule','numeric','series-output',1,'{}',1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_projection_failures(rule_id,ledger_epoch,pub_seq,error_code,attempts,last_failed_at) \
         VALUES('rule-output','epoch',2,'projection_failed',1,?)",
    )
    .bind(DIAGNOSTIC_NOW)
    .execute(&pool)
    .await
    .unwrap();
    insert_output_observation(&pool).await;
    sqlx::query(
        "INSERT INTO export_profiles(profile_id,display_name,adapter_id,adapter_schema_version,setup_json,state,revision,created_at) \
         VALUES('profile-output','Output','adapter-output',1,'{}','active',1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO output_bindings(binding_id,profile_id,rule_id,state,revision,created_at) \
         VALUES('binding-output','profile-output','rule-output','active',1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO output_routes(route_id,binding_id,rule_id,adapter_id,config_schema_version,config_json,active,lifecycle_state,last_transform_success_at,created_at) \
         VALUES('route-output','binding-output','rule-output','adapter-output',1,'{}',1,'active',?,1)",
    )
    .bind(DIAGNOSTIC_NOW - 1)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_observations(observation_id,rule_id,revision,calibration_revision,series_id,sequence,kind,value_json,reading,signal_ref,edge_node_id,ledger_epoch,source_pub_seq,observed_at,created_at) \
         VALUES('observation-delivered','rule-output',1,1,'series-output',2,'numeric','1',1,'sig-output','node','epoch',1,?,?)",
    )
    .bind(DIAGNOSTIC_NOW - 100)
    .bind(DIAGNOSTIC_NOW - 100)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO output_outbox(export_id,route_id,observation_id,topic,qos,retain,payload_json,attempts,created_at,published_at) \
         VALUES('export-output-delivered','route-output','observation-delivered','output/topic',1,0,'{}',1,?,?)",
    )
    .bind(DIAGNOSTIC_NOW - 120)
    .bind(DIAGNOSTIC_NOW - 80)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO output_outbox(export_id,route_id,observation_id,topic,qos,retain,payload_json,attempts,created_at) \
         VALUES('export-output','route-output','observation-output','output/topic',1,0,'{}',0,?)",
    )
    .bind(DIAGNOSTIC_NOW - 300_001)
    .execute(&pool)
    .await
    .unwrap();

    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&report, DiagnosticStageKind::Projection).code,
        "projection_active_failure"
    );
    let output = stage(&report, DiagnosticStageKind::ExternalOutput);
    assert_eq!(output.state, DiagnosticStageState::Warning);
    assert_eq!(output.code, "external_output_pending");
    assert_eq!(output.blocked_by, None);
    assert_eq!(output.last_success_at, Some(DIAGNOSTIC_NOW - 80));
}

async fn insert_output_observation(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "INSERT INTO semantic_observations(observation_id,rule_id,revision,calibration_revision,series_id,sequence,kind,value_json,reading,signal_ref,edge_node_id,ledger_epoch,source_pub_seq,observed_at,created_at) \
         VALUES('observation-output','rule-output',1,1,'series-output',1,'numeric','1',1,'sig-output','node','epoch',1,?,?)",
    )
    .bind(DIAGNOSTIC_NOW)
    .bind(DIAGNOSTIC_NOW)
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_observation(
    pool: &sqlx::SqlitePool,
    observation_id: &str,
    ledger_epoch: &str,
    source_pub_seq: i64,
    sequence: i64,
) {
    sqlx::query(
        "INSERT INTO semantic_observations(observation_id,rule_id,revision,calibration_revision,series_id,sequence,kind,value_json,reading,signal_ref,edge_node_id,ledger_epoch,source_pub_seq,observed_at,created_at) \
         VALUES(?,'rule-1',1,1,'series-1',?,'numeric','1',1,'sig-rule','node',?,?,?,?)",
    )
    .bind(observation_id)
    .bind(sequence)
    .bind(ledger_epoch)
    .bind(source_pub_seq)
    .bind(DIAGNOSTIC_NOW)
    .bind(DIAGNOSTIC_NOW)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn causal_queries_keep_per_signal_and_recovery_lookups_index_bounded() {
    let (_directory, _storage, pool) = causal_storage().await;
    let raw_plan = sqlx::query(&format!(
        "EXPLAIN QUERY PLAN {SQLITE_DIAGNOSTIC_SIGNAL_RECEIPTS_SQL}"
    ))
    .bind(1_i64)
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.try_get::<String, _>("detail").unwrap())
    .collect::<Vec<_>>()
    .join("\n");
    assert!(
        raw_plan.contains("ix_raw_records_diagnostic_epoch_signal_received"),
        "{raw_plan}"
    );
    assert!(
        !raw_plan.contains("SCAN raw") && !raw_plan.contains("USE TEMP B-TREE"),
        "raw receipt lookup must not scan or sort retained history: {raw_plan}"
    );
    let recovery_plan = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT 1 FROM semantic_observations AS observation \
         WHERE observation.rule_id=? AND observation.ledger_epoch=? AND observation.source_pub_seq>? LIMIT 1",
    )
    .bind("rule-1")
    .bind("epoch")
    .bind(10_i64)
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.try_get::<String, _>("detail").unwrap())
    .collect::<Vec<_>>()
    .join("\n");
    assert!(
        recovery_plan.contains("ix_semantic_observation_recovery"),
        "{recovery_plan}"
    );
    for (label, sql, index, relation) in [
        (
            "projection latest",
            SQLITE_DIAGNOSTIC_PROJECTION_LATEST_SQL,
            "ix_semantic_observation_diagnostic_latest",
            "semantic_observations",
        ),
        (
            "output delivery latest",
            SQLITE_DIAGNOSTIC_OUTPUT_DELIVERY_LATEST_SQL,
            "ix_output_outbox_diagnostic_route_published",
            "output_outbox",
        ),
        (
            "output pending oldest",
            SQLITE_DIAGNOSTIC_OUTPUT_PENDING_OLDEST_SQL,
            "ix_output_outbox_diagnostic_route_pending",
            "output_outbox",
        ),
    ] {
        let plan = sqlx::query(&format!("EXPLAIN QUERY PLAN {sql}"))
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get::<String, _>("detail").unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plan.contains(index), "{label}: {plan}");
        assert!(
            !plan.contains(&format!("SCAN {relation}")) && !plan.contains("USE TEMP B-TREE"),
            "{label} must not scan or sort retained history: {plan}"
        );
    }
}

fn postgres_plan_has(plan: &serde_json::Value, node_type: &str, relation: Option<&str>) -> bool {
    match plan {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| postgres_plan_has(value, node_type, relation)),
        serde_json::Value::Object(values) => {
            (values.get("Node Type").and_then(serde_json::Value::as_str) == Some(node_type)
                && relation.is_none_or(|relation| {
                    values
                        .get("Relation Name")
                        .and_then(serde_json::Value::as_str)
                        == Some(relation)
                }))
                || values
                    .values()
                    .any(|value| postgres_plan_has(value, node_type, relation))
        }
        _ => false,
    }
}

fn postgres_plan_uses_index(plan: &serde_json::Value, index: &str) -> bool {
    match plan {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| postgres_plan_uses_index(value, index)),
        serde_json::Value::Object(values) => {
            (values.get("Index Name").and_then(serde_json::Value::as_str) == Some(index))
                || values
                    .values()
                    .any(|value| postgres_plan_uses_index(value, index))
        }
        _ => false,
    }
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_causal_queries_keep_per_signal_and_recovery_lookups_index_bounded() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let pool = PgPool::connect(&dsn).await.unwrap();
    // Exercise the PostgreSQL causal aggregate queries too; the output stage
    // uses durable PUBACK evidence rather than transform success.
    let report = diagnostics_with_runtime(&storage, 90, DIAGNOSTIC_NOW, None, ready_health())
        .await
        .unwrap();
    assert_eq!(
        stage(&report, DiagnosticStageKind::ExternalOutput).code,
        "external_output_not_configured"
    );
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,created_at,updated_at) \
         VALUES('diagnostic-node-ref','diagnostic-node','selected-epoch','active',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO descriptor_signals(edge_node_id,series_key,system_id,measurement_key,variant,value_type,presence,descriptor_revision,updated_at) \
         VALUES('diagnostic-node','selected','system','temperature','primary','float','current',1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO inventory_signals(signal_ref,edge_node_id,series_key,system_id,created_at) \
         VALUES('diagnostic-signal','diagnostic-node','selected','system',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let rule = Semantics::new(storage.clone())
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "diagnostic-node".into(),
                series_key: "selected".into(),
                display_name: "diagnostic rule".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            1,
        )
        .await
        .unwrap();
    OutputProfiles::new(storage.clone(), registered_output_adapters())
        .activate("Diagnostic output", "iotkit.mqtt-json.v1", Map::new(), 1)
        .await
        .unwrap();
    let route_id: String =
        sqlx::query_scalar("SELECT route_id FROM output_routes WHERE rule_id=$1 AND active=TRUE")
            .bind(&rule.rule_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO export_profiles(profile_id,display_name,adapter_id,adapter_schema_version,setup_json,state,revision,created_at,stopped_at) \
         VALUES('historical-profile','historical','historical-adapter',1,'{}','stopped',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO output_bindings(binding_id,profile_id,rule_id,state,revision,created_at,stopped_at) \
         VALUES('historical-binding','historical-profile',$1,'stopped',1,1,1)",
    )
    .bind(&rule.rule_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO output_routes(route_id,binding_id,rule_id,adapter_id,config_schema_version,config_json,active,lifecycle_state,created_at) \
         VALUES('historical-route','historical-binding',$1,'historical-adapter',1,'{}',FALSE,'stopped',1)",
    )
    .bind(&rule.rule_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,publication_id,record_json,record_sha256,received_at,series_key) \
         SELECT 'diagnostic-node','selected-epoch',value,'prefix-' || value,convert_to('{}','UTF8'),decode(repeat('00',32),'hex'),value,'other' \
         FROM generate_series(1,20000) AS sequence(value)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,publication_id,record_json,record_sha256,received_at,series_key) \
         VALUES('diagnostic-node','selected-epoch',20001,'selected',convert_to('{}','UTF8'),decode(repeat('00',32),'hex'),20001,'selected')",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        storage.diagnostic_signal_receipts(65).await.unwrap(),
        vec![Some(20_001)],
        "the PostgreSQL implementation executes one direct-bound batch after its bounded identity lookup"
    );
    sqlx::query(
        "INSERT INTO semantic_observations(observation_id,rule_id,revision,calibration_revision,series_id,sequence,kind,value_json,reading,signal_ref,edge_node_id,ledger_epoch,source_pub_seq,observed_at,created_at) \
         SELECT 'selected-prefix-' || value,$1,1,1,$2,value,'numeric','1'::jsonb,1,'diagnostic-signal','diagnostic-node','selected-epoch',value,value,value \
         FROM generate_series(1,20000) AS sequence(value)",
    )
    .bind(&rule.rule_id)
    .bind(&rule.series_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_observations(observation_id,rule_id,revision,calibration_revision,series_id,sequence,kind,value_json,reading,signal_ref,edge_node_id,ledger_epoch,source_pub_seq,observed_at,created_at) \
         VALUES('selected-observation',$1,1,1,$2,20001,'numeric','1'::jsonb,1,'diagnostic-signal','diagnostic-node','selected-epoch',20001,20001,20001)",
    )
    .bind(&rule.rule_id)
    .bind(&rule.series_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO output_outbox(export_id,route_id,observation_id,topic,qos,retain,payload_json,attempts,created_at,published_at) \
         SELECT 'output-prefix-' || value,'historical-route','selected-prefix-' || value,'test/output',1,FALSE,convert_to('{}','UTF8'),0,value, \
           CASE WHEN value<=10000 THEN value ELSE NULL END FROM generate_series(1,20000) AS sequence(value)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO output_outbox(export_id,route_id,observation_id,topic,qos,retain,payload_json,attempts,created_at,published_at) \
         VALUES('output-selected',$1,'selected-observation','test/output',1,FALSE,convert_to('{}','UTF8'),0,20001,20001)",
    )
    .bind(&route_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("ANALYZE raw_records")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("ANALYZE semantic_observations")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("ANALYZE output_outbox")
        .execute(&pool)
        .await
        .unwrap();

    let raw_plan: serde_json::Value = sqlx::query_scalar(&format!(
        "EXPLAIN (ANALYZE, FORMAT JSON, COSTS OFF, TIMING OFF, SUMMARY OFF) {POSTGRES_DIAGNOSTIC_SIGNAL_RECEIPT_SQL}"
    ))
    .bind("diagnostic-node")
    .bind("selected-epoch")
    .bind("selected")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        postgres_plan_uses_index(&raw_plan, "ix_raw_records_diagnostic_epoch_signal_received"),
        "raw receipt lookup must use its bounded receipt index: {raw_plan}"
    );
    assert!(
        !postgres_plan_has(&raw_plan, "Seq Scan", Some("raw_records"))
            && !postgres_plan_has(&raw_plan, "Sort", None),
        "raw receipt lookup must not scan or sort retained history: {raw_plan}"
    );

    let recovery_plan: serde_json::Value = sqlx::query_scalar(
        "EXPLAIN (ANALYZE, FORMAT JSON, COSTS OFF, TIMING OFF, SUMMARY OFF) \
         SELECT 1 FROM semantic_observations AS observation \
         WHERE observation.rule_id=$1 AND observation.ledger_epoch=$2 \
         AND observation.source_pub_seq>$3 LIMIT 1",
    )
    .bind(&rule.rule_id)
    .bind("selected-epoch")
    .bind(20_000_i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        postgres_plan_uses_index(&recovery_plan, "ix_semantic_observation_recovery"),
        "projection recovery must use its exact rule/epoch index: {recovery_plan}"
    );
    assert!(
        !postgres_plan_has(&recovery_plan, "Seq Scan", Some("semantic_observations"))
            && !postgres_plan_has(&recovery_plan, "Sort", None),
        "projection recovery must not scan or sort retained observations: {recovery_plan}"
    );
    for (label, sql, index, relation) in [
        (
            "projection latest",
            POSTGRES_DIAGNOSTIC_PROJECTION_LATEST_SQL,
            "ix_semantic_observation_diagnostic_latest",
            "semantic_observations",
        ),
        (
            "output delivery latest",
            POSTGRES_DIAGNOSTIC_OUTPUT_DELIVERY_LATEST_SQL,
            "ix_output_outbox_diagnostic_route_published",
            "output_outbox",
        ),
        (
            "output pending oldest",
            POSTGRES_DIAGNOSTIC_OUTPUT_PENDING_OLDEST_SQL,
            "ix_output_outbox_diagnostic_route_pending",
            "output_outbox",
        ),
    ] {
        let plan: serde_json::Value = sqlx::query_scalar(&format!(
            "EXPLAIN (ANALYZE, FORMAT JSON, COSTS OFF, TIMING OFF, SUMMARY OFF) {sql}"
        ))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            postgres_plan_uses_index(&plan, index),
            "{label} must use its active-config history index: {plan}"
        );
        assert!(
            !postgres_plan_has(&plan, "Seq Scan", Some(relation))
                && !postgres_plan_has(&plan, "Sort", None),
            "{label} must not scan or sort retained history: {plan}"
        );
    }
    pool.close().await;
}
