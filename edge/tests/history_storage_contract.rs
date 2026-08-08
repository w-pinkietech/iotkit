use std::{collections::HashMap, path::PathBuf};

use iotkit_edge::{
    application::{
        profiles::{InventoryProfiles, SignalProfileInput},
        semantics::{SemanticRuleDraft, Semantics},
    },
    composition::{StorageWebApplication, registered_output_adapters},
    semantics::{Detector, DetectorMode, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, AuditActor, RawHistoryQuery, RawRecord, Storage, StorageProfile},
    web::{ConsoleRequest, HistoryQuery, Principal, WebApplication},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use sqlx::{
    PgPool, Row,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

async fn prepare_fixture(storage: Storage) -> (Storage, String, String) {
    storage.initialize_edge_identity(1).await.unwrap();
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage.apply_descriptor(&descriptor, 2).await.unwrap();
    let signal = InventoryProfiles::new(storage.clone())
        .signals()
        .await
        .unwrap()[0]
        .clone();
    storage
        .update_signal_profile(
            AuditActor::local_cli(),
            &signal.signal_ref,
            SignalProfileInput {
                display_name: "Boiler temperature".into(),
                display_sensor_type: "temperature".into(),
                display_sensor_type_label: "Temperature".into(),
                display_value_kind: "numeric".into(),
                display_unit_mode: "unit".into(),
                display_unit: "°C".into(),
                decimal_places: 1,
            },
            None,
            3,
        )
        .await
        .unwrap();
    (
        storage,
        signal.signal_ref,
        descriptor.signals[0].series_key.clone(),
    )
}

async fn fixture() -> (tempfile::TempDir, Storage, String, String) {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("edge.db"),
    })
    .await
    .unwrap();
    let (storage, signal_ref, series_key) = prepare_fixture(storage).await;
    (directory, storage, signal_ref, series_key)
}

fn numeric_rule(series_key: &str, display_name: &str) -> SemanticRuleDraft {
    SemanticRuleDraft {
        edge_node_id: "edge-node-01".into(),
        series_key: series_key.into(),
        display_name: display_name.into(),
        spec: RuleSpec {
            kind: SemanticKind::Numeric,
            detector: Detector::default(),
            trigger: TriggerMode::None,
        },
    }
}

fn counter_rule(series_key: &str, display_name: &str) -> SemanticRuleDraft {
    SemanticRuleDraft {
        edge_node_id: "edge-node-01".into(),
        series_key: series_key.into(),
        display_name: display_name.into(),
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

fn record(sequence: i64, series_key: &str, value: f64, received_at: i64) -> RawRecord {
    record_with_event(sequence, series_key, value, received_at - 1, received_at)
}

fn record_with_event(
    sequence: i64,
    series_key: &str,
    value: f64,
    event_time: i64,
    received_at: i64,
) -> RawRecord {
    RawRecord::new(
        sequence,
        serde_json::to_vec(&serde_json::json!({
            "family":"measurement","schema_version":1,"epoch":"epoch-a",
            "pub_seq":sequence,"series_key":series_key,"values":[value],
            "event_time":event_time,"event_time_source":"received_at",
            "time_source":"edge_node","time_quality":"unsynced",
            "received_at":received_at,"device_time":null
        }))
        .unwrap(),
    )
    .unwrap()
}

fn boolean_record(sequence: i64, series_key: &str, value: bool, received_at: i64) -> RawRecord {
    RawRecord::new(
        sequence,
        serde_json::to_vec(&serde_json::json!({
            "family":"measurement","schema_version":1,"epoch":"epoch-a",
            "pub_seq":sequence,"series_key":series_key,"values":[value],
            "event_time":received_at-1,"event_time_source":"received_at",
            "time_source":"edge_node","time_quality":"unsynced",
            "received_at":received_at,"device_time":null
        }))
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn raw_history_pages_stably_across_epochs_and_keeps_profile_metadata() {
    let (_directory, storage, signal_ref, series_key) = fixture().await;
    for (epoch, sequence, value) in [
        ("epoch-a", 1, 1.0),
        ("epoch-b", 1, 99.0),
        ("epoch-b", 2, 2.0),
    ] {
        storage
            .accept_batch(AcceptBatch {
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: epoch.into(),
                publication_id: format!("{epoch}-{sequence}"),
                received_at: 100,
                records: vec![record(sequence, &series_key, value, 100)],
            })
            .await
            .unwrap();
    }
    let query = |cursor| RawHistoryQuery {
        from: 0,
        to: 1_000,
        limit: 2,
        cursor,
        signal_ref: Some(signal_ref.clone()),
        edge_node_id: None,
    };
    let first = storage.query_raw_history(query(None)).await.unwrap();
    assert!(first.has_more);
    assert_eq!(first.rows.len(), 2);
    assert_eq!(first.rows[0].display_name, "Boiler temperature");
    assert_eq!(first.rows[0].unit, "°C");
    let second = storage
        .query_raw_history(query(first.next_cursor))
        .await
        .unwrap();
    assert_eq!(second.rows.len(), 1);
    assert!(!second.has_more);
    let keys: std::collections::BTreeSet<_> = first
        .rows
        .into_iter()
        .chain(second.rows)
        .map(|row| (row.ledger_epoch, row.pub_seq))
        .collect();
    assert_eq!(keys.len(), 3);
}

async fn assert_semantic_history_series_contract(
    storage: &Storage,
    signal_ref: &str,
    series_key: &str,
) {
    let semantics = Semantics::new(storage.clone());
    assert_semantic_counter_bucket_last_value_contract(storage, series_key).await;
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-a".into(),
            publication_id: "history-seed".into(),
            received_at: 50,
            records: vec![record(5, series_key, 0.0, 50)],
        })
        .await
        .unwrap();
    let rule = semantics
        .create_rule(numeric_rule(series_key, "Temperature"), 10)
        .await
        .unwrap();
    semantics
        .update_calibration(&rule.signal_ref, 2.0, 10.0, 20)
        .await
        .unwrap();
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-a".into(),
            publication_id: "history-observations".into(),
            received_at: 100,
            records: vec![
                record_with_event(6, series_key, 1.0, 150, 100),
                record_with_event(7, series_key, 100.0, 160, 100),
                record_with_event(8, series_key, 2.0, 170, 100),
                record_with_event(9, series_key, -10.0, 50, 100),
            ],
        })
        .await
        .unwrap();
    let progress = semantics
        .project_pending(10, registered_output_adapters())
        .await
        .unwrap();
    assert_eq!(progress.observations, 4, "{progress:?}");
    let rows = storage
        .query_semantic_history(0, 1_000, 100_001, Some(signal_ref), None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|row| row.rule_revision == 1));
    assert!(rows.iter().all(|row| row.calibration_revision == 2));
    assert_eq!(rows[0].source_pub_seq, 8);
    assert_eq!(rows[0].rule_name, rule.display_name);
    let buckets = storage
        .query_history_series(signal_ref, 100, 200, 100)
        .await
        .unwrap();
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].minimum, -10.0);
    assert_eq!(buckets[0].maximum, 100.0);
    assert_eq!(buckets[0].count, 4);
    let (semantic_buckets, latest) = storage
        .query_semantic_history_series(&rule.rule_id, 100, 200, 100)
        .await
        .unwrap();
    assert_eq!(semantic_buckets.len(), 1);
    assert_eq!(semantic_buckets[0].minimum, 12.0);
    assert_eq!(semantic_buckets[0].maximum, 210.0);
    assert_eq!(semantic_buckets[0].count, 3);
    assert_eq!(semantic_buckets[0].last_value, Some(14.0));
    assert_eq!(latest, Some((100, b"-10.0".to_vec())));
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-a".into(),
            publication_id: "history-after-range".into(),
            received_at: 200,
            records: vec![record_with_event(10, series_key, 3.0, 200, 200)],
        })
        .await
        .unwrap();
    semantics
        .project_pending(10, registered_output_adapters())
        .await
        .unwrap();
    let (range_limited_buckets, latest_after_chart_range) = storage
        .query_semantic_history_series(&rule.rule_id, 100, 200, 100)
        .await
        .unwrap();
    assert_eq!(range_limited_buckets.len(), 1);
    assert_eq!(latest_after_chart_range, Some((200, b"16.0".to_vec())));
    let payload = StorageWebApplication::new(storage.clone())
        .history_series(HistoryQuery {
            from: Some("100".into()),
            to: Some("200".into()),
            limit: None,
            cursor: None,
            signal_ref: None,
            rule_id: Some(rule.rule_id.clone()),
            edge_node_id: None,
            bucket_ms: Some(100),
        })
        .await
        .unwrap();
    assert_eq!(payload["display_name"], "Temperature");
    assert_eq!(payload["sample_count"], 3);
    assert_eq!(payload["latest_received_at"], 200);
    assert_eq!(payload["latest_value"], 16.0);
    assert_eq!(payload["points"][0]["maximum"], 210.0);
    assert_eq!(payload["points"][0]["last_value"], 14.0);
}

async fn assert_semantic_counter_bucket_last_value_contract(storage: &Storage, series_key: &str) {
    let semantics = Semantics::new(storage.clone());
    let rule = semantics
        .create_rule(counter_rule(series_key, "Production count"), 300)
        .await
        .unwrap();
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-a".into(),
            publication_id: "counter-bucket-values".into(),
            received_at: 1_000,
            records: vec![
                record_with_event(1, series_key, 0.0, 1_000, 1_000),
                record_with_event(2, series_key, 1.0, 1_010, 1_000),
                record_with_event(3, series_key, 0.0, 1_020, 1_000),
                record_with_event(4, series_key, 1.0, 1_030, 1_000),
            ],
        })
        .await
        .unwrap();
    let counter_progress = semantics
        .project_pending(20, registered_output_adapters())
        .await
        .unwrap();
    assert_eq!(counter_progress.observations, 2, "{counter_progress:?}");
    semantics.reset_counter(&rule.rule_id, 1_050).await.unwrap();
    let reset_progress = semantics
        .project_pending(20, registered_output_adapters())
        .await
        .unwrap();
    assert_eq!(reset_progress.observations, 1, "{reset_progress:?}");

    let (buckets, latest) = storage
        .query_semantic_history_series(&rule.rule_id, 1_000, 2_000, 1_000)
        .await
        .unwrap();
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].minimum, 0.0);
    assert_eq!(buckets[0].average, 1.0);
    assert_eq!(buckets[0].maximum, 2.0);
    assert_eq!(buckets[0].count, 3);
    assert_eq!(buckets[0].last_value, Some(0.0));
    assert_eq!(latest, Some((1_000, b"0".to_vec())));

    let payload = StorageWebApplication::new(storage.clone())
        .history_series(HistoryQuery {
            from: Some("1000".into()),
            to: Some("2000".into()),
            limit: None,
            cursor: None,
            signal_ref: None,
            rule_id: Some(rule.rule_id.clone()),
            edge_node_id: None,
            bucket_ms: Some(1_000),
        })
        .await
        .unwrap();
    assert_eq!(payload["points"][0]["average"], 1.0);
    assert_eq!(payload["points"][0]["last_value"], 0.0);
    semantics.retire_rule(&rule.rule_id, 1_060).await.unwrap();
}

#[tokio::test]
async fn semantic_history_preserves_exact_provenance_and_series_buckets_preserve_spikes() {
    let (_directory, storage, signal_ref, series_key) = fixture().await;
    assert_semantic_history_series_contract(&storage, &signal_ref, &series_key).await;
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_semantic_history_series_obeys_the_shared_contract() {
    let dsn = std::env::var("IOTKIT_TEST_POSTGRES_DSN").expect("PostgreSQL DSN");
    let storage = Storage::connect(StorageProfile::Postgres { dsn })
        .await
        .unwrap();
    let (storage, signal_ref, series_key) = prepare_fixture(storage).await;
    assert_semantic_history_series_contract(&storage, &signal_ref, &series_key).await;
}

#[tokio::test]
async fn sqlite_semantic_history_recent_range_uses_the_observed_at_index() {
    const RETAINED_PREFIX: i64 = 10_000;
    let (directory, storage, _, _) = fixture().await;
    let inspection = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(PathBuf::from(directory.path()).join("edge.db"))
                .create_if_missing(false),
        )
        .await
        .unwrap();
    sqlx::query(
        "WITH RECURSIVE numbers(value) AS (VALUES(1) UNION ALL SELECT value+1 FROM numbers \
         WHERE value<?) INSERT INTO semantic_observations(observation_id,rule_id,revision,\
         calibration_revision,series_id,sequence,kind,value_json,reading,signal_ref,\
         edge_node_id,ledger_epoch,source_pub_seq,observed_at,created_at) \
         SELECT 'history-index-' || value,'history-index-rule',1,1,'history-index-series',\
         value,'numeric',CAST(value AS TEXT),NULL,'signal','node','epoch',value,value,value \
         FROM numbers",
    )
    .bind(RETAINED_PREFIX)
    .execute(&inspection)
    .await
    .unwrap();
    sqlx::query("ANALYZE semantic_observations")
        .execute(&inspection)
        .await
        .unwrap();

    let plan = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT observation.observation_row_id \
         FROM semantic_observations AS observation WHERE observation.rule_id=? \
         AND observation.observed_at>=? AND observation.observed_at<? \
         ORDER BY observation.observed_at,observation.observation_row_id",
    )
    .bind("history-index-rule")
    .bind(RETAINED_PREFIX - 2)
    .bind(RETAINED_PREFIX + 1)
    .fetch_all(&inspection)
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.try_get::<String, _>("detail").unwrap())
    .collect::<Vec<_>>()
    .join("\n");
    assert!(
        plan.contains("ix_semantic_observation_rule_observed_at_row"),
        "recent semantic history must use its rule/time index: {plan}"
    );
    assert!(
        !plan.contains("SCAN observation") && !plan.contains("USE TEMP B-TREE"),
        "recent semantic history must not scan or sort its retained per-rule prefix: {plan}"
    );

    let (buckets, _) = storage
        .query_semantic_history_series(
            "history-index-rule",
            RETAINED_PREFIX - 2,
            RETAINED_PREFIX + 1,
            100,
        )
        .await
        .unwrap();
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].count, 3);
    assert_eq!(buckets[0].minimum, (RETAINED_PREFIX - 2) as f64);
    assert_eq!(buckets[0].maximum, RETAINED_PREFIX as f64);
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_semantic_history_recent_range_uses_the_observed_at_index() {
    const RETAINED_PREFIX: i64 = 10_000;
    let dsn = std::env::var("IOTKIT_TEST_POSTGRES_DSN").expect("PostgreSQL DSN");
    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .unwrap();
    let inspection = PgPool::connect(&dsn).await.unwrap();
    sqlx::query(
        "INSERT INTO semantic_observations(observation_id,rule_id,revision,\
         calibration_revision,series_id,sequence,kind,value_json,reading,signal_ref,\
         edge_node_id,ledger_epoch,source_pub_seq,observed_at,created_at) \
         SELECT concat('history-index-',value),'history-index-rule',1,1,\
         'history-index-series',value,'numeric','1'::jsonb,NULL,'signal','node','epoch',\
         value,value,value FROM generate_series(1,$1) AS value",
    )
    .bind(RETAINED_PREFIX)
    .execute(&inspection)
    .await
    .unwrap();
    sqlx::query("ANALYZE semantic_observations")
        .execute(&inspection)
        .await
        .unwrap();

    let plan = sqlx::query_scalar::<_, String>(
        "EXPLAIN (ANALYZE, COSTS OFF, TIMING OFF, SUMMARY OFF) \
         SELECT observation.observation_row_id FROM semantic_observations AS observation \
         WHERE observation.rule_id=$1 AND observation.observed_at>=$2 \
         AND observation.observed_at<$3 ORDER BY observation.observed_at,\
         observation.observation_row_id",
    )
    .bind("history-index-rule")
    .bind(RETAINED_PREFIX - 2)
    .bind(RETAINED_PREFIX + 1)
    .fetch_all(&inspection)
    .await
    .unwrap()
    .join("\n");
    assert!(
        plan.contains("using ix_semantic_observation_rule_observed_at_row"),
        "recent semantic history must use its rule/time index: {plan}"
    );
    assert!(
        !plan.contains("Seq Scan on semantic_observations") && !plan.contains("Sort"),
        "recent semantic history must not scan or sort its retained per-rule prefix: {plan}"
    );

    let (buckets, _) = storage
        .query_semantic_history_series(
            "history-index-rule",
            RETAINED_PREFIX - 2,
            RETAINED_PREFIX + 1,
            100,
        )
        .await
        .unwrap();
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].count, 3);
    assert_eq!(buckets[0].minimum, 1.0);
    assert_eq!(buckets[0].maximum, 1.0);
    inspection.close().await;
}

#[tokio::test]
async fn semantic_rule_without_an_observation_stays_unreceived_despite_raw_input() {
    let (_directory, storage, signal_ref, series_key) = fixture().await;
    let rule = Semantics::new(storage.clone())
        .create_rule(counter_rule(&series_key, "Production count"), 10)
        .await
        .unwrap();
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-a".into(),
            publication_id: "counter-baseline".into(),
            received_at: 100,
            records: vec![record(1, &series_key, 0.0, 100)],
        })
        .await
        .unwrap();
    let progress = Semantics::new(storage.clone())
        .project_pending(10, registered_output_adapters())
        .await
        .unwrap();
    assert_eq!(progress.observations, 0, "{progress:?}");
    assert_eq!(
        storage
            .query_raw_history(RawHistoryQuery {
                from: 0,
                to: 200,
                limit: 1,
                cursor: None,
                signal_ref: Some(signal_ref.clone()),
                edge_node_id: None,
            })
            .await
            .unwrap()
            .rows
            .len(),
        1,
    );
    let (buckets, latest) = storage
        .query_semantic_history_series(&rule.rule_id, 0, 200, 100)
        .await
        .unwrap();
    assert!(buckets.is_empty());
    assert_eq!(latest, None);
    let payload = StorageWebApplication::new(storage)
        .history_series(HistoryQuery {
            from: Some("0".into()),
            to: Some("200".into()),
            limit: None,
            cursor: None,
            signal_ref: None,
            rule_id: Some(rule.rule_id),
            edge_node_id: None,
            bucket_ms: Some(100),
        })
        .await
        .unwrap();
    assert_eq!(payload["latest_received_at"], serde_json::Value::Null);
    assert_eq!(payload["latest_value"], serde_json::Value::Null);
}

#[tokio::test]
async fn live_read_model_keeps_each_active_rule_and_excludes_retired_rules() {
    let (_directory, storage, signal_ref, series_key) = fixture().await;
    let semantics = Semantics::new(storage.clone());
    let first = semantics
        .create_rule(numeric_rule(&series_key, "Temperature"), 10)
        .await
        .unwrap();
    let second = semantics
        .create_rule(numeric_rule(&series_key, "Temperature average"), 11)
        .await
        .unwrap();
    let retired = semantics
        .create_rule(numeric_rule(&series_key, "Retired temperature"), 12)
        .await
        .unwrap();
    semantics.retire_rule(&retired.rule_id, 13).await.unwrap();

    let view = StorageWebApplication::new(storage)
        .console(ConsoleRequest {
            path: "/live".into(),
            query: HashMap::new(),
            principal: Principal {
                account_ref: "test-admin".into(),
                login_id: "admin".into(),
                display_name: "Admin".into(),
                role: "admin".into(),
                state: "active".into(),
                must_change_password: false,
                revision: 1,
                created_at: 0,
                updated_at: 0,
            },
        })
        .await
        .unwrap();
    let signal = view
        .signals
        .into_iter()
        .find(|signal| signal.signal_ref == signal_ref)
        .unwrap();

    assert_eq!(
        signal
            .rules
            .iter()
            .map(|rule| rule.rule_id.as_str())
            .collect::<Vec<_>>(),
        vec![first.rule_id.as_str(), second.rule_id.as_str()],
    );
    assert!(
        signal
            .rules
            .iter()
            .all(|rule| rule.rule_id != retired.rule_id)
    );
}

#[tokio::test]
async fn history_series_preserves_boolean_contact_transitions_as_zero_and_one() {
    let (_directory, storage, signal_ref, series_key) = fixture().await;
    for (sequence, value, received_at) in [(1, false, 100), (2, true, 110)] {
        storage
            .accept_batch(AcceptBatch {
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-a".into(),
                publication_id: format!("boolean-history-{sequence}"),
                received_at,
                records: vec![boolean_record(sequence, &series_key, value, received_at)],
            })
            .await
            .unwrap();
    }

    let buckets = storage
        .query_history_series(&signal_ref, 100, 200, 10)
        .await
        .unwrap();

    assert_eq!(buckets.len(), 2);
    assert_eq!(buckets[0].average, 0.0);
    assert_eq!(buckets[1].average, 1.0);
}

#[tokio::test]
async fn history_series_limits_its_exact_latest_value_to_the_requested_range() {
    let (_directory, storage, signal_ref, series_key) = fixture().await;
    let application = StorageWebApplication::new(storage.clone());
    let query = || HistoryQuery {
        from: Some("200".into()),
        to: Some("300".into()),
        limit: None,
        cursor: None,
        signal_ref: Some(signal_ref.clone()),
        rule_id: None,
        edge_node_id: None,
        bucket_ms: Some(10),
    };

    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-a".into(),
            publication_id: "before-range".into(),
            received_at: 100,
            records: vec![record(1, &series_key, 12.5, 100)],
        })
        .await
        .unwrap();

    let before_range = application.history_series(query()).await.unwrap();
    assert_eq!(before_range["sample_count"], 0);
    assert_eq!(before_range["latest_received_at"], serde_json::Value::Null);
    assert_eq!(before_range["latest_value"], serde_json::Value::Null);
    assert_eq!(before_range["points"], serde_json::json!([]));

    for (sequence, value, received_at) in [(2, 20.0, 220), (3, 22.5, 250)] {
        storage
            .accept_batch(AcceptBatch {
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-a".into(),
                publication_id: format!("in-range-{sequence}"),
                received_at,
                records: vec![record(sequence, &series_key, value, received_at)],
            })
            .await
            .unwrap();
    }

    let in_range = application.history_series(query()).await.unwrap();
    assert_eq!(in_range["sample_count"], 2);
    assert_eq!(in_range["latest_received_at"], 250);
    assert_eq!(in_range["latest_value"], 22.5);
    assert!(in_range["points"][0].get("last_value").is_none());
}
