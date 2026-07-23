use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use iotkit_edge::{
    backup::create_encrypted_backup,
    diagnostics::storage_status,
    storage::{AcceptBatch, RawRecord, Storage, StorageProfile},
};
use serde::Serialize;

const EDGE_NODES: usize = 4;
const SENSORS_PER_EDGE: usize = 8;
const RECORDS_PER_EDGE: usize = 2_000;
const BATCH_SIZE: usize = 100;

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
    database_bytes: i64,
    pending_output: i64,
    projection_failures: i64,
    regression_smoke_passed: bool,
}

#[tokio::test]
#[ignore = "run through scripts/test-edge-capacity.sh"]
async fn capacity_regression_smoke_emits_existing_evidence_schema() {
    let report_path =
        PathBuf::from(std::env::var("IOTKIT_CAPACITY_REPORT").expect("capacity report path"));
    let backup_path =
        PathBuf::from(std::env::var("IOTKIT_TEST_CAPACITY_BACKUP").expect("capacity backup path"));
    let profile = std::env::var("IOTKIT_TEST_CAPACITY_PROFILE").expect("capacity profile");
    let storage = match profile.as_str() {
        "embedded" => {
            let database =
                PathBuf::from(std::env::var("IOTKIT_TEST_CAPACITY_SQLITE").expect("SQLite path"));
            Storage::connect(StorageProfile::Sqlite { path: database })
                .await
                .expect("connect SQLite capacity storage")
        }
        "postgres" => {
            let dsn = std::env::var("IOTKIT_TEST_POSTGRES_DSN").expect("PostgreSQL DSN");
            Storage::connect(StorageProfile::Postgres { dsn })
                .await
                .expect("connect PostgreSQL capacity storage")
        }
        other => panic!("unsupported capacity profile: {other}"),
    };
    storage
        .initialize_edge_identity(1_720_000_000_000)
        .await
        .expect("initialize capacity identity");

    let started = Instant::now();
    let mut payload_bytes = 0;
    let mut latencies = Vec::with_capacity(EDGE_NODES * RECORDS_PER_EDGE / BATCH_SIZE);
    for edge in 1..=EDGE_NODES {
        let edge_node_id = format!("capacity-edge-{edge}");
        for batch_start in (1..=RECORDS_PER_EDGE).step_by(BATCH_SIZE) {
            let records = (batch_start..batch_start + BATCH_SIZE)
                .map(|sequence| {
                    let encoded = serde_json::to_vec(&serde_json::json!({
                        "family": "measurement",
                        "schema_version": 1,
                        "epoch": "capacity-epoch-1",
                        "pub_seq": sequence,
                        "series_key": format!(
                            "capacity-sensor-{}:temperature_c:na:primary",
                            (sequence - 1) % SENSORS_PER_EDGE + 1
                        ),
                        "event_time": sequence * 1_000,
                        "event_time_source": "received_at",
                        "time_source": "edge_node",
                        "time_quality": "unsynced",
                        "received_at": sequence * 1_000,
                        "device_time": null,
                        "values": [20.0 + (sequence % 10) as f64],
                    }))
                    .expect("encode capacity record");
                    payload_bytes += encoded.len();
                    RawRecord::new(sequence as i64, encoded).expect("valid capacity record")
                })
                .collect::<Vec<_>>();
            let accepted_at = Instant::now();
            storage
                .accept_batch(AcceptBatch {
                    edge_node_id: edge_node_id.clone(),
                    ledger_epoch: "capacity-epoch-1".into(),
                    publication_id: format!(
                        "{edge_node_id}:capacity-epoch-1:{batch_start}:{}",
                        batch_start + BATCH_SIZE - 1
                    ),
                    received_at: 1_720_000_000_000 + batch_start as i64,
                    records,
                })
                .await
                .expect("accept capacity batch");
            latencies.push(accepted_at.elapsed());
        }
    }
    let ingest_duration = started.elapsed();
    latencies.sort_unstable();
    let accept_p99 = latencies[(latencies.len() * 99 - 1) / 100];

    let query_started = Instant::now();
    let mut queried_records = 0;
    for edge in 1..=EDGE_NODES {
        queried_records += storage
            .raw_records(&format!("capacity-edge-{edge}"), "capacity-epoch-1")
            .await
            .expect("query capacity history")
            .len();
    }
    let query_duration = query_started.elapsed();

    let backup_started = Instant::now();
    create_encrypted_backup(&storage, backup_path, "capacity-test-passphrase")
        .await
        .expect("create capacity backup");
    let backup_duration = backup_started.elapsed();
    let status = storage_status(&storage, 90)
        .await
        .expect("read capacity status");
    let expected_records = EDGE_NODES * RECORDS_PER_EDGE;
    let passed = accept_p99 < Duration::from_secs(10)
        && query_duration < Duration::from_secs(10)
        && backup_duration < Duration::from_secs(60)
        && queried_records == expected_records
        && status.raw_record_count == expected_records as i64
        && status.pending_output_count == 0
        && status.projection_failure_count == 0;
    let report = CapacityRegressionReport {
        profile: status.profile,
        edge_nodes: EDGE_NODES,
        sensors_per_edge: SENSORS_PER_EDGE,
        records: expected_records,
        payload_bytes,
        records_per_second: expected_records as f64 / ingest_duration.as_secs_f64(),
        accept_p99_millis: accept_p99.as_millis(),
        history_query_millis: query_duration.as_millis(),
        backup_millis: backup_duration.as_millis(),
        database_bytes: status.database_bytes,
        pending_output: status.pending_output_count,
        projection_failures: status.projection_failure_count,
        regression_smoke_passed: passed,
    };
    let encoded = serde_json::to_vec_pretty(&report).expect("encode capacity report");
    fs::write(&report_path, [encoded.as_slice(), b"\n"].concat()).expect("write capacity report");
    assert!(passed, "capacity regression smoke failed");
}
