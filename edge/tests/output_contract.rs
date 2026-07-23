use std::path::PathBuf;

use iotkit_edge::{
    application::{
        output_profiles::{OutputProfiles, ProfileState},
        semantics::{SemanticRuleDraft, Semantics},
    },
    composition::registered_output_adapters,
    semantics::{Detector, DetectorMode, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, RawRecord, Storage, StorageProfile},
};
use serde_json::{Map, Value};
use sqlx::{Executor, PgPool, SqlitePool, sqlite::SqliteConnectOptions};
use tempfile::TempDir;

async fn store() -> (TempDir, Storage) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).expect("create test temp");
    let directory = TempDir::new_in(root).expect("temp directory");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("output.db"),
    })
    .await
    .expect("open storage");
    storage
        .initialize_edge_identity(1_720_000_000_000)
        .await
        .expect("initialize Edge identity");
    (directory, storage)
}

fn numeric_rule() -> SemanticRuleDraft {
    SemanticRuleDraft {
        edge_node_id: "edge-node-01".into(),
        series_key: "series-temperature-01".into(),
        display_name: "Temperature".into(),
        spec: RuleSpec {
            kind: SemanticKind::Numeric,
            detector: Detector::default(),
            trigger: TriggerMode::None,
        },
    }
}

fn counter_rule() -> SemanticRuleDraft {
    SemanticRuleDraft {
        edge_node_id: "edge-node-01".into(),
        series_key: "series-contact-01".into(),
        display_name: "Production".into(),
        spec: RuleSpec {
            kind: SemanticKind::CumulativeCounter,
            detector: Detector {
                mode: DetectorMode::BooleanHighActive,
                ..Detector::default()
            },
            trigger: TriggerMode::OnTransition,
        },
    }
}

async fn accept(storage: &Storage, sequence: i64, series_key: &str, value: f64) {
    let record = serde_json::json!({
        "family": "measurement",
        "schema_version": 1,
        "epoch": "epoch-01",
        "pub_seq": sequence,
        "series_key": series_key,
        "values": [value],
        "event_time": 1_720_000_000_000_i64 + sequence,
        "event_time_source": "received_at",
        "time_source": "edge_node",
        "time_quality": "unsynced",
        "received_at": 1_720_000_000_000_i64 + sequence,
        "device_time": null
    });
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: format!("publication-{sequence}"),
            received_at: 1_720_000_000_000 + sequence,
            records: vec![
                RawRecord::new(sequence, serde_json::to_vec(&record).unwrap())
                    .expect("valid record"),
            ],
        })
        .await
        .expect("accept raw record");
}

async fn accept_values(storage: &Storage, records: &[(i64, &str, Vec<f64>)]) {
    let raw = records
        .iter()
        .map(|(sequence, series_key, values)| {
            let record = serde_json::json!({
                "family": "measurement",
                "schema_version": 1,
                "epoch": "epoch-01",
                "pub_seq": sequence,
                "series_key": series_key,
                "values": values,
                "event_time": 1_720_000_000_000_i64 + sequence,
                "event_time_source": "received_at",
                "time_source": "edge_node",
                "time_quality": "unsynced",
                "received_at": 1_720_000_000_000_i64 + sequence,
                "device_time": null
            });
            RawRecord::new(
                *sequence,
                serde_json::to_vec(&record).expect("serialize record"),
            )
            .expect("valid raw record")
        })
        .collect();
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: "publication-multiple".into(),
            received_at: 1_720_000_000_100,
            records: raw,
        })
        .await
        .expect("accept raw records");
}

#[tokio::test]
async fn semantic_projection_and_exact_outbox_are_one_atomic_operation() {
    let (_directory, storage) = store().await;
    let semantics = Semantics::new(storage.clone());
    let profiles = OutputProfiles::new(storage.clone(), registered_output_adapters());
    let rule = semantics
        .create_rule(numeric_rule(), 1_720_000_000_010)
        .await
        .expect("create rule");
    let profile = profiles
        .activate(
            "Generic MQTT",
            "iotkit.mqtt-json.v1",
            Map::new(),
            1_720_000_000_020,
        )
        .await
        .expect("activate generic profile");
    assert_eq!(profile.state, ProfileState::Active);
    assert_eq!(profile.bindings.len(), 1);
    assert_eq!(profile.bindings[0].rule_id, rule.rule_id);

    accept(&storage, 1, "series-temperature-01", 21.5).await;
    let projected = semantics
        .project_pending(10, registered_output_adapters())
        .await
        .expect("project");
    assert_eq!(projected.observations, 1);
    assert_eq!(projected.publications, 1);

    let observations = storage
        .semantic_observations(&rule.rule_id)
        .await
        .expect("observations");
    let pending = storage.pending_output_count().await.expect("pending count");
    assert_eq!(observations.len(), 1);
    assert_eq!(pending, 1);
    let claimed = storage
        .claim_output("claim-01", 1_720_000_000_100, 30_000)
        .await
        .expect("claim")
        .expect("pending publication");
    assert_eq!(claimed.qos, 1);
    assert!(!claimed.retain);
    assert_eq!(
        claimed.topic,
        format!(
            "iotkit/v1/sources/{}/signals/{}/observations",
            storage.edge_id().await.expect("edge id"),
            profile.bindings[0].external_id
        )
    );
    let payload: Value = serde_json::from_slice(&claimed.payload).expect("valid JSON");
    assert_eq!(payload["kind"], "numeric");
    assert_eq!(payload["value"], 21.5);
}

#[tokio::test]
async fn outbox_insert_failure_rolls_back_observation_and_projection_receipt() {
    let (directory, storage) = store().await;
    let semantics = Semantics::new(storage.clone());
    let profiles = OutputProfiles::new(storage.clone(), registered_output_adapters());
    semantics
        .create_rule(numeric_rule(), 1)
        .await
        .expect("create rule");
    profiles
        .activate("Generic", "iotkit.mqtt-json.v1", Map::new(), 2)
        .await
        .expect("activate");
    accept(&storage, 1, "series-temperature-01", 21.5).await;

    let options = SqliteConnectOptions::new()
        .filename(directory.path().join("output.db"))
        .create_if_missing(false);
    let inspection = SqlitePool::connect_with(options)
        .await
        .expect("open inspection connection");
    inspection
        .execute(
            "CREATE TRIGGER fail_output_insert BEFORE INSERT ON output_outbox \
             BEGIN SELECT RAISE(ABORT, 'injected output failure'); END",
        )
        .await
        .expect("install fault");
    assert!(
        semantics
            .project_pending(1, registered_output_adapters())
            .await
            .is_err()
    );
    for table in [
        "semantic_projection_receipts",
        "semantic_observations",
        "output_outbox",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&inspection)
            .await
            .expect("count atomic rows");
        assert_eq!(count, 0, "{table} must roll back with the failed outbox");
    }
}

#[tokio::test]
async fn poison_semantic_input_is_durable_and_does_not_block_an_independent_rule() {
    let (directory, storage) = store().await;
    let semantics = Semantics::new(storage.clone());
    let poison = semantics
        .create_rule(numeric_rule(), 1)
        .await
        .expect("create poison rule");
    let healthy = semantics
        .create_rule(
            SemanticRuleDraft {
                series_key: "series-temperature-02".into(),
                display_name: "Healthy temperature".into(),
                ..numeric_rule()
            },
            2,
        )
        .await
        .expect("create healthy rule");
    accept_values(
        &storage,
        &[
            (1, "series-temperature-01", Vec::new()),
            (2, "series-temperature-02", vec![19.25]),
        ],
    )
    .await;

    let progress = semantics
        .project_pending(10, registered_output_adapters())
        .await
        .expect("poison input is isolated");
    assert_eq!(progress.receipts, 2);
    assert_eq!(progress.observations, 1);
    assert!(
        storage
            .semantic_observations(&poison.rule_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        storage
            .semantic_observations(&healthy.rule_id)
            .await
            .unwrap()
            .len(),
        1
    );

    let inspection = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(directory.path().join("output.db"))
            .create_if_missing(false),
    )
    .await
    .expect("open inspection connection");
    let failures: i64 = sqlx::query_scalar("SELECT count(*) FROM semantic_projection_failures")
        .fetch_one(&inspection)
        .await
        .expect("count failures");
    assert_eq!(failures, 1);
}

#[tokio::test]
async fn rule_and_calibration_revisions_apply_only_after_their_captured_cursors() {
    let (directory, storage) = store().await;
    let semantics = Semantics::new(storage.clone());
    let rule = semantics
        .create_rule(numeric_rule(), 1)
        .await
        .expect("create rule");
    accept(&storage, 1, "series-temperature-01", 10.0).await;
    semantics
        .revise_rule(&rule.rule_id, "Temperature revised", numeric_rule().spec, 2)
        .await
        .expect("revise after accepted input");
    accept(&storage, 2, "series-temperature-01", 20.0).await;
    semantics
        .update_calibration(&rule.signal_ref, 2.0, 0.0, 3)
        .await
        .expect("calibrate after accepted input");
    accept(&storage, 3, "series-temperature-01", 30.0).await;
    semantics
        .project_pending(10, registered_output_adapters())
        .await
        .expect("project lagged inputs with historical revisions");

    let inspection = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(directory.path().join("output.db"))
            .create_if_missing(false),
    )
    .await
    .expect("open inspection connection");
    let rows: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT revision,calibration_revision,CAST(value_json AS TEXT) \
         FROM semantic_observations \
         ORDER BY source_pub_seq",
    )
    .fetch_all(&inspection)
    .await
    .expect("read historical projections");
    assert_eq!(
        rows,
        vec![
            (1, 1, "10.0".into()),
            (2, 1, "20.0".into()),
            (2, 2, "60.0".into())
        ]
    );
}

#[tokio::test]
async fn failed_routes_retry_fairly_oldest_first_and_converge_after_storage_restart() {
    let (directory, storage) = store().await;
    let database = directory.path().join("output.db");
    let semantics = Semantics::new(storage.clone());
    semantics
        .create_rule(numeric_rule(), 1)
        .await
        .expect("create first rule");
    semantics
        .create_rule(
            SemanticRuleDraft {
                series_key: "series-temperature-02".into(),
                display_name: "Temperature two".into(),
                ..numeric_rule()
            },
            2,
        )
        .await
        .expect("create second rule");
    OutputProfiles::new(storage.clone(), registered_output_adapters())
        .activate("Generic", "iotkit.mqtt-json.v1", Map::new(), 3)
        .await
        .expect("activate routes");
    let inspection = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(false),
    )
    .await
    .expect("open inspection connection");
    inspection
        .execute("UPDATE output_routes SET config_schema_version=99")
        .await
        .expect("inject transform failures");
    accept_values(
        &storage,
        &[
            (1, "series-temperature-01", vec![21.0]),
            (2, "series-temperature-02", vec![22.0]),
        ],
    )
    .await;
    semantics
        .project_pending(6, registered_output_adapters())
        .await
        .expect("failed routes do not block semantic progress");
    let attempts: Vec<i64> =
        sqlx::query_scalar("SELECT attempts FROM output_route_attempts ORDER BY route_id")
            .fetch_all(&inspection)
            .await
            .expect("route attempts");
    assert_eq!(attempts.len(), 2);
    assert!(
        attempts.iter().max().unwrap() - attempts.iter().min().unwrap() <= 1,
        "retry attempts must interleave fairly: {attempts:?}"
    );
    assert_eq!(storage.pending_output_count().await.unwrap(), 0);
    inspection
        .execute("UPDATE output_routes SET config_schema_version=1")
        .await
        .expect("repair route configuration");
    inspection.close().await;
    drop(semantics);
    drop(storage);

    let restarted = Storage::connect(StorageProfile::Sqlite { path: database })
        .await
        .expect("restart storage");
    Semantics::new(restarted.clone())
        .project_pending(10, registered_output_adapters())
        .await
        .expect("retry after restart");
    assert_eq!(restarted.pending_output_count().await.unwrap(), 2);
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run scripts/test-edge-output.sh with the postgres profile"]
async fn postgres_failed_routes_retry_fairly_and_converge_after_storage_restart() {
    let dsn =
        std::env::var("IOTKIT_TEST_RUST_OUTPUT_POSTGRES_CONTRACT_DSN").expect("contract test DSN");
    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .expect("connect PostgreSQL storage");
    storage
        .initialize_edge_identity(1_720_000_000_000)
        .await
        .expect("initialize identity");
    let semantics = Semantics::new(storage.clone());
    semantics
        .create_rule(numeric_rule(), 1)
        .await
        .expect("create first rule");
    semantics
        .create_rule(
            SemanticRuleDraft {
                series_key: "series-temperature-02".into(),
                display_name: "Temperature two".into(),
                ..numeric_rule()
            },
            2,
        )
        .await
        .expect("create second rule");
    OutputProfiles::new(storage.clone(), registered_output_adapters())
        .activate("Generic", "iotkit.mqtt-json.v1", Map::new(), 3)
        .await
        .expect("activate routes");
    let inspection = PgPool::connect(&dsn).await.expect("inspection pool");
    inspection
        .execute("UPDATE output_routes SET config_schema_version=99")
        .await
        .expect("inject transform failures");
    accept_values(
        &storage,
        &[
            (1, "series-temperature-01", vec![21.0]),
            (2, "series-temperature-02", vec![22.0]),
        ],
    )
    .await;
    semantics
        .project_pending(6, registered_output_adapters())
        .await
        .expect("failed routes do not block semantic progress");
    let attempts: Vec<i64> =
        sqlx::query_scalar("SELECT attempts FROM output_route_attempts ORDER BY route_id")
            .fetch_all(&inspection)
            .await
            .expect("route attempts");
    assert_eq!(attempts.len(), 2);
    assert!(
        attempts.iter().max().unwrap() - attempts.iter().min().unwrap() <= 1,
        "retry attempts must interleave fairly: {attempts:?}"
    );
    inspection
        .execute("UPDATE output_routes SET config_schema_version=1")
        .await
        .expect("repair routes");
    inspection.close().await;
    drop(semantics);
    drop(storage);
    let restarted = Storage::connect(StorageProfile::Postgres { dsn })
        .await
        .expect("restart PostgreSQL storage");
    Semantics::new(restarted.clone())
        .project_pending(10, registered_output_adapters())
        .await
        .expect("retry after restart");
    assert_eq!(restarted.pending_output_count().await.unwrap(), 2);
}

#[tokio::test]
async fn pinikiet_uses_one_signal_scoped_identity_and_waits_for_confirmation() {
    let (_directory, storage) = store().await;
    let semantics = Semantics::new(storage.clone());
    let profiles = OutputProfiles::new(storage.clone(), registered_output_adapters());
    let production = semantics
        .create_rule(counter_rule(), 1_720_000_000_010)
        .await
        .expect("create counter");
    let alarm = semantics
        .create_rule(
            SemanticRuleDraft {
                display_name: "Alarm".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Alarm,
                    detector: Detector {
                        mode: DetectorMode::BooleanHighActive,
                        ..Detector::default()
                    },
                    trigger: TriggerMode::None,
                },
                ..counter_rule()
            },
            1_720_000_000_011,
        )
        .await
        .expect("create alarm");
    let profile = profiles
        .activate(
            "Pinikiet",
            "pinikiet.mqtt.v1",
            Map::new(),
            1_720_000_000_020,
        )
        .await
        .expect("prepare profile");
    assert_eq!(profile.state, ProfileState::Preparing);
    let production_binding = profile
        .bindings
        .iter()
        .find(|binding| binding.rule_id == production.rule_id)
        .expect("production binding");
    let alarm_binding = profile
        .bindings
        .iter()
        .find(|binding| binding.rule_id == alarm.rule_id)
        .expect("alarm binding");
    assert_eq!(production_binding.external_id, alarm_binding.external_id);
    assert!(production_binding.external_id.starts_with("sen-"));
    assert!(!production_binding.active);

    profiles
        .confirm(&production_binding.binding_id, 1_720_000_000_030)
        .await
        .expect("confirm shared sensor");
    let reloaded = profiles.list().await.expect("list profiles");
    assert!(
        reloaded[0]
            .bindings
            .iter()
            .filter(|binding| binding.external_id == production_binding.external_id)
            .all(|binding| binding.active)
    );
}

#[tokio::test]
async fn claim_is_read_only_until_puback_mark_and_stale_claim_cannot_mark() {
    let (_directory, storage) = store().await;
    let semantics = Semantics::new(storage.clone());
    let profiles = OutputProfiles::new(storage.clone(), registered_output_adapters());
    semantics
        .create_rule(numeric_rule(), 1)
        .await
        .expect("create rule");
    profiles
        .activate("Generic", "iotkit.mqtt-json.v1", Map::new(), 2)
        .await
        .expect("activate");
    accept(&storage, 1, "series-temperature-01", 21.5).await;
    semantics
        .project_pending(10, registered_output_adapters())
        .await
        .expect("project");

    let claimed = storage
        .claim_output("claim-old", 10, 10)
        .await
        .expect("claim")
        .expect("row");
    assert_eq!(storage.pending_output_count().await.unwrap(), 1);
    storage
        .release_output(&claimed.export_id, "claim-old")
        .await
        .expect("release");
    let reclaimed = storage
        .claim_output("claim-new", 21, 10)
        .await
        .expect("reclaim")
        .expect("row");
    assert_eq!(reclaimed.export_id, claimed.export_id);
    assert!(
        !storage
            .mark_output_published(&claimed.export_id, "claim-old", 22)
            .await
            .expect("stale mark")
    );
    assert!(
        storage
            .mark_output_published(&claimed.export_id, "claim-new", 23)
            .await
            .expect("matching mark")
    );
    assert_eq!(storage.pending_output_count().await.unwrap(), 0);
}
