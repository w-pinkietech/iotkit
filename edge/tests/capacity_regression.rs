use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use iotkit_edge::{
    application::semantics::{SemanticRuleDraft, Semantics},
    backup::create_encrypted_backup,
    composition::registered_output_adapters,
    diagnostics::storage_status,
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, RawRecord, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use serde::Serialize;

const EDGE_NODES: usize = 4;
const SENSORS_PER_EDGE: usize = 8;
const DEFAULT_RECORDS_PER_EDGE: usize = 100_000;
const BATCH_SIZE: usize = 100;
const MAX_PENDING_TAIL_PER_EDGE: usize = 64;
const CAPACITY_SYSTEM_ID: &str = "018f0000-0000-7000-8000-000000000001";
const CAPACITY_SERIES_KEY: &str = "018f0000-0000-7000-8000-000000000001:temperature_c:0:primary";

enum CapacityStorage {
    Sqlite(PathBuf),
    Postgres(String),
}

impl CapacityStorage {
    async fn connect(&self) -> Storage {
        match self {
            Self::Sqlite(path) => Storage::connect(StorageProfile::Sqlite { path: path.clone() })
                .await
                .expect("connect SQLite capacity storage"),
            Self::Postgres(dsn) => Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
                .await
                .expect("connect PostgreSQL capacity storage"),
        }
    }
}

#[derive(Serialize)]
struct CapacityRegressionReport {
    profile: String,
    edge_nodes: usize,
    sensors_per_edge: usize,
    records: usize,
    payload_bytes: usize,
    records_per_second: f64,
    accept_p99_millis: u128,
    history_query_millis: u128,
    backup_millis: u128,
    restart_millis: u128,
    projection_recovery_wall_millis: u128,
    database_bytes: i64,
    semantic_observations: i64,
    projection_pending_before: i64,
    projection_pending_after: i64,
    pending_output: i64,
    projection_failures: i64,
    foreground_storage_completed: bool,
    restart_completed: bool,
    full_retained_history_profile: bool,
    regression_smoke_passed: bool,
}

fn records_per_edge() -> usize {
    std::env::var("IOTKIT_CAPACITY_RECORDS_PER_EDGE")
        .map(|value| {
            value
                .parse()
                .expect("numeric IOTKIT_CAPACITY_RECORDS_PER_EDGE")
        })
        .unwrap_or(DEFAULT_RECORDS_PER_EDGE)
}

fn pending_tail_per_edge(records_per_edge: usize) -> usize {
    (records_per_edge / 4).clamp(1, MAX_PENDING_TAIL_PER_EDGE)
}

fn edge_node_id(edge: usize) -> String {
    format!("capacity-edge-{edge}")
}

fn capacity_series_key(sensor: usize) -> String {
    format!("{CAPACITY_SYSTEM_ID}:temperature_c:{sensor}:primary")
}

fn capacity_record(sequence: usize, series_key: &str) -> RawRecord {
    let encoded = serde_json::to_vec(&serde_json::json!({
        "family": "measurement",
        "schema_version": 1,
        "epoch": "capacity-epoch-1",
        "pub_seq": sequence,
        "series_key": series_key,
        "event_time": sequence * 1_000,
        "event_time_source": "received_at",
        "time_source": "edge_node",
        "time_quality": "unsynced",
        "received_at": sequence * 1_000,
        "device_time": null,
        "values": [20.0 + (sequence % 10) as f64],
    }))
    .expect("encode capacity record");
    RawRecord::new(sequence as i64, encoded).expect("valid capacity record")
}

async fn apply_capacity_descriptor(storage: &Storage, edge: usize) {
    let edge_node_id = edge_node_id(edge);
    let signals = (0..SENSORS_PER_EDGE)
        .map(|sensor| {
            serde_json::json!({
                "series_key": capacity_series_key(sensor),
                "system_id": CAPACITY_SYSTEM_ID,
                "measurement_key": "temperature_c",
                "channel_index": sensor,
                "variant": "primary",
                "unit": null,
                "value_type": "float"
            })
        })
        .collect::<Vec<_>>();
    let descriptor = DescriptorSnapshot::decode(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "edge_node_id": edge_node_id,
            "ledger_epoch": "capacity-epoch-1",
            "descriptor_revision": 1,
            "complete": true,
            "devices": [{
                "system_id": CAPACITY_SYSTEM_ID,
                "identifier": format!("capacity-device-{edge}"),
                "state": "active",
                "model_id": "capacity"
            }],
            "signals": signals
        }))
        .expect("encode capacity descriptor"),
    )
    .expect("decode capacity descriptor");
    assert_eq!(descriptor.signals.len(), SENSORS_PER_EDGE);
    storage
        .apply_descriptor(&descriptor, 1_720_000_000_000 + edge as i64)
        .await
        .expect("apply capacity descriptor");
}

#[tokio::test]
#[ignore = "run through scripts/test-edge-capacity.sh"]
async fn capacity_regression_profile_emits_semantic_backlog_evidence() {
    let report_path =
        PathBuf::from(std::env::var("IOTKIT_CAPACITY_REPORT").expect("capacity report path"));
    let backup_path =
        PathBuf::from(std::env::var("IOTKIT_TEST_CAPACITY_BACKUP").expect("capacity backup path"));
    let profile = std::env::var("IOTKIT_TEST_CAPACITY_PROFILE").expect("capacity profile");
    let capacity_storage = match profile.as_str() {
        "embedded" => CapacityStorage::Sqlite(PathBuf::from(
            std::env::var("IOTKIT_TEST_CAPACITY_SQLITE").expect("SQLite path"),
        )),
        "postgres" => CapacityStorage::Postgres(
            std::env::var("IOTKIT_TEST_POSTGRES_DSN").expect("PostgreSQL DSN"),
        ),
        other => panic!("unsupported capacity profile: {other}"),
    };
    let records_per_edge = records_per_edge();
    let pending_tail_per_edge = pending_tail_per_edge(records_per_edge);
    assert!(
        records_per_edge > pending_tail_per_edge,
        "capacity profile needs a retained prefix before the pending tail"
    );
    let prefix_records = records_per_edge - pending_tail_per_edge;
    let storage = capacity_storage.connect().await;
    storage
        .initialize_edge_identity(1_720_000_000_000)
        .await
        .expect("initialize capacity identity");

    for edge in 1..=EDGE_NODES {
        apply_capacity_descriptor(&storage, edge).await;
    }

    let started = Instant::now();
    let mut payload_bytes = 0;
    let mut latencies = Vec::with_capacity(EDGE_NODES * records_per_edge / BATCH_SIZE);
    for edge in 1..=EDGE_NODES {
        let edge_node_id = edge_node_id(edge);
        for batch_start in (1..=prefix_records).step_by(BATCH_SIZE) {
            let batch_end = (batch_start + BATCH_SIZE - 1).min(prefix_records);
            let records = (batch_start..=batch_end)
                .map(|sequence| {
                    let series_key = capacity_series_key((sequence - 1) % SENSORS_PER_EDGE);
                    let record = capacity_record(sequence, &series_key);
                    payload_bytes += record.record_json.len();
                    record
                })
                .collect::<Vec<_>>();
            let accepted_at = Instant::now();
            storage
                .accept_batch(AcceptBatch {
                    edge_node_id: edge_node_id.clone(),
                    ledger_epoch: "capacity-epoch-1".into(),
                    publication_id: format!(
                        "{edge_node_id}:capacity-epoch-1:{batch_start}:{}",
                        batch_end
                    ),
                    received_at: 1_720_000_000_000 + batch_start as i64,
                    records,
                })
                .await
                .expect("accept capacity batch");
            latencies.push(accepted_at.elapsed());
        }
    }

    let semantics = Semantics::new(storage.clone());
    for edge in 1..=EDGE_NODES {
        semantics
            .create_rule(
                SemanticRuleDraft {
                    edge_node_id: edge_node_id(edge),
                    series_key: CAPACITY_SERIES_KEY.into(),
                    display_name: format!("Capacity temperature {edge}"),
                    spec: RuleSpec {
                        kind: SemanticKind::Numeric,
                        detector: Detector::default(),
                        trigger: TriggerMode::None,
                    },
                },
                1_720_000_100_000 + edge as i64,
            )
            .await
            .expect("create capacity semantic rule");
    }
    for edge in 1..=EDGE_NODES {
        let first = prefix_records + 1;
        let edge_node_id = edge_node_id(edge);
        let records = (first..=records_per_edge)
            .map(|sequence| {
                let record = capacity_record(sequence, CAPACITY_SERIES_KEY);
                payload_bytes += record.record_json.len();
                record
            })
            .collect();
        storage
            .accept_batch(AcceptBatch {
                edge_node_id: edge_node_id.clone(),
                ledger_epoch: "capacity-epoch-1".into(),
                publication_id: format!(
                    "{edge_node_id}:capacity-epoch-1:{first}:{records_per_edge}"
                ),
                received_at: 1_720_000_000_000 + first as i64,
                records,
            })
            .await
            .expect("accept pending semantic tail");
    }
    let ingest_duration = started.elapsed();
    latencies.sort_unstable();
    let accept_p99 = latencies[(latencies.len() * 99 - 1) / 100];

    let query_started = Instant::now();
    let queried_records = storage
        .raw_records(&edge_node_id(1), "capacity-epoch-1")
        .await
        .expect("query capacity history")
        .len();
    let query_duration = query_started.elapsed();

    let status_before = storage_status(&storage, 90)
        .await
        .expect("read capacity status with pending semantic work");
    let expected_records = EDGE_NODES * records_per_edge;
    let expected_pending = (EDGE_NODES * pending_tail_per_edge) as i64;
    assert_eq!(
        status_before.pending_semantic_projection_count,
        expected_pending
    );

    let backup_started = Instant::now();
    create_encrypted_backup(&storage, backup_path, "capacity-test-passphrase")
        .await
        .expect("create capacity backup");
    let backup_duration = backup_started.elapsed();
    drop(semantics);
    drop(storage);

    let restart_started = Instant::now();
    let restarted = capacity_storage.connect().await;
    let restart_duration = restart_started.elapsed();
    let recovery_started = Instant::now();
    let recovery_storage = restarted.clone();
    let recovery = tokio::spawn(async move {
        Semantics::new(recovery_storage)
            .project_pending(expected_pending as usize, registered_output_adapters())
            .await
    });
    tokio::task::yield_now().await;
    let foreground_status = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let status = storage_status(&restarted, 90)
                .await
                .expect("read storage status while projection recovery is active");
            if (0..expected_pending).contains(&status.pending_semantic_projection_count) {
                return status;
            }
            assert_ne!(
                status.pending_semantic_projection_count, 0,
                "projection finished before the foreground storage operation could run"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("foreground storage operation completed during projection recovery");
    let projected = tokio::time::timeout(Duration::from_secs(30), recovery)
        .await
        .expect("projection recovery completed")
        .expect("projection recovery task did not panic")
        .expect("recover pending semantic tail after restart");
    let recovery_duration = recovery_started.elapsed();
    let status_after = storage_status(&restarted, 90)
        .await
        .expect("read capacity status after semantic recovery");
    let full_retained_history_profile = expected_records >= 396_000;
    let foreground_storage_completed = foreground_status.pending_semantic_projection_count > 0
        && foreground_status.pending_semantic_projection_count < expected_pending;
    let passed = queried_records == records_per_edge
        && status_before.raw_record_count == expected_records as i64
        && status_before.pending_output_count == 0
        && status_before.projection_failure_count == 0
        && projected.receipts == expected_pending as usize
        && status_after.semantic_observation_count == expected_pending
        && status_after.pending_semantic_projection_count == 0
        && status_after.pending_output_count == 0
        && status_after.projection_failure_count == 0
        && foreground_storage_completed;
    let report = CapacityRegressionReport {
        profile: status_after.profile,
        edge_nodes: EDGE_NODES,
        sensors_per_edge: SENSORS_PER_EDGE,
        records: expected_records,
        payload_bytes,
        records_per_second: expected_records as f64 / ingest_duration.as_secs_f64(),
        accept_p99_millis: accept_p99.as_millis(),
        history_query_millis: query_duration.as_millis(),
        backup_millis: backup_duration.as_millis(),
        restart_millis: restart_duration.as_millis(),
        projection_recovery_wall_millis: recovery_duration.as_millis(),
        database_bytes: status_after.database_bytes,
        semantic_observations: status_after.semantic_observation_count,
        projection_pending_before: status_before.pending_semantic_projection_count,
        projection_pending_after: status_after.pending_semantic_projection_count,
        pending_output: status_after.pending_output_count,
        projection_failures: status_after.projection_failure_count,
        foreground_storage_completed,
        restart_completed: true,
        full_retained_history_profile,
        regression_smoke_passed: passed,
    };
    let encoded = serde_json::to_vec_pretty(&report).expect("encode capacity report");
    fs::write(&report_path, [encoded.as_slice(), b"\n"].concat()).expect("write capacity report");
    assert!(passed, "capacity regression smoke failed");
}
