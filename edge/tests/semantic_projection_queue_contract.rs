use std::{path::PathBuf, time::Duration};

use iotkit_edge::{
    application::semantics::{SemanticRuleDraft, Semantics},
    composition::registered_output_adapters,
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, RawRecord, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use sqlx::{PgPool, Row, SqlitePool, sqlite::SqliteConnectOptions};
use tempfile::TempDir;

const SERIES_KEY: &str = "018f0000-0000-7000-8000-000000000001:temperature:na:primary";

async fn store() -> (TempDir, Storage) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("edge.db"),
    })
    .await
    .expect("open SQLite storage");
    storage
        .initialize_edge_identity(1)
        .await
        .expect("initialize Edge identity");
    apply_queue_descriptor(&storage).await;
    (directory, storage)
}

async fn apply_queue_descriptor(storage: &Storage) {
    let descriptor = DescriptorSnapshot::decode(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "edge_node_id": "edge-node-01",
            "ledger_epoch": "epoch-01",
            "descriptor_revision": 1,
            "complete": true,
            "devices": [{
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "identifier": "queue-contract-device",
                "state": "active",
                "model_id": "contract"
            }],
            "signals": [{
                "series_key": SERIES_KEY,
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "measurement_key": "temperature",
                "channel_index": null,
                "variant": "primary",
                "unit": null,
                "value_type": "float"
            }]
        }))
        .expect("encode descriptor"),
    )
    .expect("decode descriptor");
    storage
        .apply_descriptor(&descriptor, 2)
        .await
        .expect("apply descriptor");
}

async fn postgres_store() -> Option<(String, Storage)> {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return None,
    };
    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .expect("open PostgreSQL storage");
    storage
        .initialize_edge_identity(1)
        .await
        .expect("initialize Edge identity");
    apply_queue_descriptor(&storage).await;
    Some((dsn, storage))
}

fn plan_has_node(plan: &serde_json::Value, node_type: &str, relation: Option<&str>) -> bool {
    match plan {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| plan_has_node(value, node_type, relation)),
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
                    .any(|value| plan_has_node(value, node_type, relation))
        }
        _ => false,
    }
}

fn plan_uses_queue_index(plan: &serde_json::Value) -> bool {
    match plan {
        serde_json::Value::Array(values) => values.iter().any(plan_uses_queue_index),
        serde_json::Value::Object(values) => {
            (values
                .get("Relation Name")
                .and_then(serde_json::Value::as_str)
                == Some("semantic_projection_queue")
                && values
                    .get("Index Name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|name| name.starts_with("ix_semantic_projection_queue_")))
                || values.values().any(plan_uses_queue_index)
        }
        _ => false,
    }
}

fn plan_uses_named_index(plan: &serde_json::Value, relation: &str, index: &str) -> bool {
    match plan {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| plan_uses_named_index(value, relation, index)),
        serde_json::Value::Object(values) => {
            (values
                .get("Relation Name")
                .and_then(serde_json::Value::as_str)
                == Some(relation)
                && values.get("Index Name").and_then(serde_json::Value::as_str) == Some(index))
                || values
                    .values()
                    .any(|value| plan_uses_named_index(value, relation, index))
        }
        _ => false,
    }
}

fn record(sequence: i64) -> RawRecord {
    RawRecord::new(
        sequence,
        serde_json::to_vec(&serde_json::json!({
            "family": "measurement",
            "schema_version": 1,
            "epoch": "epoch-01",
            "pub_seq": sequence,
            "series_key": SERIES_KEY,
            "values": [20.0 + sequence as f64],
            "event_time": sequence,
            "event_time_source": "received_at",
            "time_source": "edge_node",
            "time_quality": "unsynced",
            "received_at": sequence,
            "device_time": null
        }))
        .expect("encode record"),
    )
    .expect("valid record")
}

async fn accept(storage: &Storage, first: i64, last: i64) {
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: format!("queue-contract-{first}-{last}"),
            received_at: last,
            records: (first..=last).map(record).collect(),
        })
        .await
        .expect("accept records");
}

async fn seed_applied_reset_history_sqlite(pool: &SqlitePool, rule_id: &str) {
    sqlx::query(
        "WITH RECURSIVE reset_history(value) AS (\
         VALUES(1) UNION ALL SELECT value+1 FROM reset_history WHERE value<1000) \
         INSERT INTO semantic_counter_resets(reset_id,rule_id,requested_at,applied_at,\
         zero_observation_id) SELECT 'applied-reset-' || value,?,value,value,NULL \
         FROM reset_history",
    )
    .bind(rule_id)
    .execute(pool)
    .await
    .expect("seed applied reset history");
}

async fn seed_applied_reset_history_postgres(pool: &PgPool, rule_id: &str) {
    sqlx::query(
        "INSERT INTO semantic_counter_resets(reset_id,rule_id,requested_at,applied_at,\
         zero_observation_id) SELECT 'applied-reset-' || value::TEXT,$1,value,value,NULL \
         FROM generate_series(1,1000) AS value",
    )
    .bind(rule_id)
    .execute(pool)
    .await
    .expect("seed applied reset history");
    sqlx::query("ANALYZE semantic_counter_resets")
        .execute(pool)
        .await
        .expect("analyze reset history");
}

fn value_record(ledger_epoch: &str, sequence: i64, value: f64, received_at: i64) -> RawRecord {
    RawRecord::new(
        sequence,
        serde_json::to_vec(&serde_json::json!({
            "family": "measurement",
            "schema_version": 1,
            "epoch": ledger_epoch,
            "pub_seq": sequence,
            "series_key": SERIES_KEY,
            "values": [value],
            "event_time": received_at,
            "event_time_source": "received_at",
            "time_source": "edge_node",
            "time_quality": "unsynced",
            "received_at": received_at,
            "device_time": null
        }))
        .expect("encode record"),
    )
    .expect("valid record")
}

async fn accept_value_in_epoch_at(
    storage: &Storage,
    ledger_epoch: &str,
    sequence: i64,
    value: f64,
    received_at: i64,
) {
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: ledger_epoch.into(),
            publication_id: format!("queue-contract-{ledger_epoch}-{sequence}"),
            received_at,
            records: vec![value_record(ledger_epoch, sequence, value, received_at)],
        })
        .await
        .expect("accept record");
}

async fn accept_value_at(storage: &Storage, sequence: i64, value: f64, received_at: i64) {
    accept_value_in_epoch_at(storage, "epoch-01", sequence, value, received_at).await;
}

async fn accept_in_epoch(storage: &Storage, ledger_epoch: &str, sequence: i64, received_at: i64) {
    accept_value_in_epoch_at(
        storage,
        ledger_epoch,
        sequence,
        sequence as f64,
        received_at,
    )
    .await;
}

async fn wait_for_advisory_waiters(pool: &PgPool, expected: i64) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let waiters: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND NOT granted",
            )
            .fetch_one(pool)
            .await
            .expect("count advisory waiters");
            if waiters >= expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("race operation reached its advisory gate");
}

async fn sqlite_candidate_plan_after_completed_prefix(completed_prefix: i64) -> Vec<String> {
    const PENDING_TAIL: i64 = 2;

    let (directory, storage) = store().await;
    let semantics = Semantics::new(storage.clone());
    let rule = semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "edge-node-01".into(),
                series_key: SERIES_KEY.into(),
                display_name: "Queue temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            3,
        )
        .await
        .expect("create rule");
    accept(&storage, 1, completed_prefix).await;
    let progress = semantics
        .project_pending(completed_prefix as usize, registered_output_adapters())
        .await
        .expect("complete prefix");
    assert_eq!(progress.receipts, completed_prefix as usize);
    accept(
        &storage,
        completed_prefix + 1,
        completed_prefix + PENDING_TAIL,
    )
    .await;

    let inspection = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(PathBuf::from(directory.path()).join("edge.db"))
            .create_if_missing(false),
    )
    .await
    .expect("open inspection connection");
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM semantic_projection_queue")
        .fetch_one(&inspection)
        .await
        .expect("count queue tail");
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM semantic_projection_receipts")
        .fetch_one(&inspection)
        .await
        .expect("count completed prefix");
    assert_eq!(pending, PENDING_TAIL);
    assert_eq!(receipts, completed_prefix);
    seed_applied_reset_history_sqlite(&inspection, &rule.rule_id).await;

    sqlx::query(
        "EXPLAIN QUERY PLAN SELECT queue.rule_id,queue.signal_ref,queue.edge_node_id,\
         queue.ledger_epoch,queue.pub_seq,queue.received_at,raw.record_json,\
         revision.revision,revision.series_id,revision.spec_json,\
         calibration.revision AS calibration_revision,calibration.scale,\
         calibration.calibration_offset AS calibration_offset \
         FROM semantic_projection_queue AS queue \
         JOIN raw_records AS raw ON raw.edge_node_id=queue.edge_node_id \
          AND raw.ledger_epoch=queue.ledger_epoch AND raw.pub_seq=queue.pub_seq \
         JOIN semantic_rule_revisions AS revision ON revision.rule_id=queue.rule_id \
          AND revision.revision=queue.revision \
         JOIN semantic_calibration_revisions AS calibration \
          ON calibration.signal_ref=queue.signal_ref \
          AND calibration.revision=queue.calibration_revision \
         WHERE NOT EXISTS(SELECT 1 FROM semantic_counter_resets AS reset \
           WHERE reset.rule_id=queue.rule_id AND reset.applied_at IS NULL \
           AND NOT EXISTS(SELECT 1 FROM semantic_counter_reset_boundaries AS boundary \
             WHERE boundary.reset_id=reset.reset_id AND boundary.ledger_epoch=queue.ledger_epoch \
             AND queue.pub_seq<=boundary.apply_after_pub_seq)) \
         ORDER BY queue.received_at,queue.edge_node_id,queue.ledger_epoch,queue.pub_seq,\
          queue.rule_created_at,queue.rule_id LIMIT 1",
    )
    .fetch_all(&inspection)
    .await
    .expect("plan queue candidate")
    .into_iter()
    .map(|row| row.try_get::<String, _>("detail").expect("plan detail"))
    .collect()
}

#[tokio::test]
async fn sqlite_candidate_plan_uses_only_tiny_pending_queue_after_a_large_completed_prefix() {
    let one_thousand = sqlite_candidate_plan_after_completed_prefix(1_000).await;
    let ten_thousand = sqlite_candidate_plan_after_completed_prefix(10_000).await;
    assert_eq!(
        one_thousand, ten_thousand,
        "ten times the completed history must keep the same queue-only candidate plan"
    );
    let plan = one_thousand.join("\n");
    assert!(
        plan.contains("ix_semantic_projection_queue_next"),
        "candidate must use its pending-work ordering index: {plan}"
    );
    assert!(
        !plan.contains("SCAN raw")
            && !plan.contains("SCAN reset")
            && !plan.contains("USE TEMP B-TREE"),
        "candidate must not scan retained raw/reset history or sort it: {plan}"
    );
    assert!(
        plan.contains("ix_semantic_counter_resets_pending_rule"),
        "candidate must use the pending-reset index: {plan}"
    );
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_candidate_plan_uses_the_bounded_pending_queue_lookup() {
    let Some((dsn, storage)) = postgres_store().await else {
        return;
    };
    let semantics = Semantics::new(storage.clone());
    let rule = semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "edge-node-01".into(),
                series_key: SERIES_KEY.into(),
                display_name: "PostgreSQL queue temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            3,
        )
        .await
        .expect("create rule");
    accept(&storage, 1, 1_000).await;
    semantics
        .project_pending(1_000, registered_output_adapters())
        .await
        .expect("complete retained prefix");
    accept(&storage, 1_001, 1_002).await;

    let inspection = PgPool::connect(&dsn).await.expect("open inspection pool");
    seed_applied_reset_history_postgres(&inspection, &rule.rule_id).await;
    let plan: serde_json::Value = sqlx::query_scalar(
        "EXPLAIN (ANALYZE, FORMAT JSON, TIMING OFF) \
         SELECT queue.rule_id,queue.signal_ref,queue.edge_node_id,queue.ledger_epoch,queue.pub_seq,\
         queue.received_at,raw.record_json,revision.revision,revision.series_id,revision.spec_json,\
         calibration.revision AS calibration_revision,calibration.scale,\
         calibration.calibration_offset AS calibration_offset \
         FROM semantic_projection_queue AS queue JOIN raw_records AS raw \
         ON raw.edge_node_id=queue.edge_node_id AND raw.ledger_epoch=queue.ledger_epoch \
         AND raw.pub_seq=queue.pub_seq JOIN semantic_rule_revisions AS revision \
         ON revision.rule_id=queue.rule_id AND revision.revision=queue.revision \
         JOIN semantic_calibration_revisions AS calibration \
         ON calibration.signal_ref=queue.signal_ref AND calibration.revision=queue.calibration_revision \
         WHERE queue.rule_id=$1 AND NOT EXISTS(SELECT 1 FROM semantic_counter_resets AS reset \
           WHERE reset.rule_id=queue.rule_id AND reset.applied_at IS NULL \
           AND NOT EXISTS(SELECT 1 FROM semantic_counter_reset_boundaries AS boundary \
             WHERE boundary.reset_id=reset.reset_id AND boundary.ledger_epoch=queue.ledger_epoch \
             AND queue.pub_seq<=boundary.apply_after_pub_seq)) \
         ORDER BY queue.received_at,queue.edge_node_id,queue.ledger_epoch,queue.pub_seq \
         LIMIT 1 FOR UPDATE OF queue",
    )
    .bind(&rule.rule_id)
    .fetch_one(&inspection)
    .await
    .expect("explain candidate lookup");
    assert!(
        plan_uses_queue_index(&plan),
        "candidate must use a semantic projection queue index: {plan}"
    );
    assert!(
        !plan_has_node(&plan, "Seq Scan", Some("raw_records")),
        "candidate must not scan retained raw history: {plan}"
    );
    assert!(
        !plan_has_node(&plan, "Seq Scan", Some("semantic_counter_resets")),
        "candidate must not scan applied reset history: {plan}"
    );
    assert!(
        plan_uses_named_index(
            &plan,
            "semantic_counter_resets",
            "ix_semantic_counter_resets_pending_rule"
        ),
        "candidate must use the pending-reset index: {plan}"
    );
    assert!(
        !plan_has_node(&plan, "Sort", None),
        "candidate must not sort retained work to find its head: {plan}"
    );
    inspection.close().await;
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_first_unseen_epoch_accept_and_rule_creation_serialize_at_the_edge_lock() {
    const CREATE_GATE: i64 = 9_100_050_001;
    const ACCEPT_GATE: i64 = 9_100_050_002;
    const EPOCH: &str = "epoch-unseen-race";
    const DISPLAY_NAME: &str = "Unseen epoch race rule";

    let Some((dsn, storage)) = postgres_store().await else {
        return;
    };
    let inspection = PgPool::connect(&dsn).await.expect("open inspection pool");
    sqlx::query(
        "CREATE FUNCTION block_unseen_epoch_rule_creation() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.display_name='Unseen epoch race rule' THEN \
           PERFORM pg_advisory_xact_lock(9100050001); END IF; RETURN NEW; END $$",
    )
    .execute(&inspection)
    .await
    .expect("install rule creation gate");
    sqlx::query(
        "CREATE TRIGGER block_unseen_epoch_rule_creation BEFORE INSERT ON semantic_rules \
         FOR EACH ROW EXECUTE FUNCTION block_unseen_epoch_rule_creation()",
    )
    .execute(&inspection)
    .await
    .expect("install rule creation trigger");
    sqlx::query(
        "CREATE FUNCTION block_unseen_epoch_accept() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.edge_node_id='edge-node-01' AND NEW.ledger_epoch='epoch-unseen-race' THEN \
           PERFORM pg_advisory_xact_lock(9100050002); END IF; RETURN NEW; END $$",
    )
    .execute(&inspection)
    .await
    .expect("install accept gate");
    sqlx::query(
        "CREATE TRIGGER block_unseen_epoch_accept BEFORE UPDATE OF accepted_through \
         ON accepted_cursors FOR EACH ROW EXECUTE FUNCTION block_unseen_epoch_accept()",
    )
    .execute(&inspection)
    .await
    .expect("install accept trigger");

    let gates = PgPool::connect(&dsn).await.expect("open gate pool");
    let mut create_gate = gates.begin().await.expect("begin create gate");
    sqlx::query("SELECT pg_advisory_xact_lock($1::bigint)")
        .bind(CREATE_GATE)
        .execute(&mut *create_gate)
        .await
        .expect("hold create gate");
    let mut accept_gate = gates.begin().await.expect("begin accept gate");
    sqlx::query("SELECT pg_advisory_xact_lock($1::bigint)")
        .bind(ACCEPT_GATE)
        .execute(&mut *accept_gate)
        .await
        .expect("hold accept gate");

    let create_storage = storage.clone();
    let create = tokio::spawn(async move {
        Semantics::new(create_storage)
            .create_rule(
                SemanticRuleDraft {
                    edge_node_id: "edge-node-01".into(),
                    series_key: SERIES_KEY.into(),
                    display_name: DISPLAY_NAME.into(),
                    spec: RuleSpec {
                        kind: SemanticKind::Numeric,
                        detector: Detector::default(),
                        trigger: TriggerMode::None,
                    },
                },
                3,
            )
            .await
    });
    wait_for_advisory_waiters(&inspection, 1).await;

    let accept_storage = storage.clone();
    let accept = tokio::spawn(async move {
        accept_storage
            .accept_batch(AcceptBatch {
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: EPOCH.into(),
                publication_id: "unseen-epoch-race".into(),
                received_at: 4,
                records: vec![value_record(EPOCH, 1, 1.0, 4)],
            })
            .await
    });
    wait_for_advisory_waiters(&inspection, 2).await;
    create_gate.commit().await.expect("release create gate");
    let rule = tokio::time::timeout(Duration::from_secs(10), create)
        .await
        .expect("rule creation completed")
        .expect("rule creation task joined")
        .expect("create rule");
    accept_gate.commit().await.expect("release accept gate");
    tokio::time::timeout(Duration::from_secs(10), accept)
        .await
        .expect("acceptance completed")
        .expect("acceptance task joined")
        .expect("accept record");

    for statement in [
        "DROP TRIGGER block_unseen_epoch_accept ON accepted_cursors",
        "DROP FUNCTION block_unseen_epoch_accept()",
        "DROP TRIGGER block_unseen_epoch_rule_creation ON semantic_rules",
        "DROP FUNCTION block_unseen_epoch_rule_creation()",
    ] {
        sqlx::query(statement)
            .execute(&inspection)
            .await
            .expect("remove deterministic race gate");
    }
    let queue_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM semantic_projection_queue \
         WHERE rule_id=$1 AND ledger_epoch=$2 AND pub_seq=1",
    )
    .bind(&rule.rule_id)
    .bind(EPOCH)
    .fetch_one(&inspection)
    .await
    .expect("count enqueued race record");
    let starts: Vec<i64> = sqlx::query_scalar(
        "SELECT start_after_pub_seq FROM semantic_rule_starts \
         WHERE rule_id=$1 AND ledger_epoch=$2",
    )
    .bind(&rule.rule_id)
    .bind(EPOCH)
    .fetch_all(&inspection)
    .await
    .expect("read captured race boundary");
    assert!(
        (queue_count == 1 && starts.is_empty()) || (queue_count == 0 && starts == vec![1]),
        "the unseen epoch must serialize as exactly one queue row or one start boundary, \
         never neither or both: queue={queue_count}, starts={starts:?}"
    );
    gates.close().await;
    inspection.close().await;
}

#[tokio::test]
async fn counter_reset_fences_post_boundary_rows_even_when_their_received_at_is_earlier() {
    let (directory, storage) = store().await;
    let semantics = Semantics::new(storage.clone());
    let rule = semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "edge-node-01".into(),
                series_key: SERIES_KEY.into(),
                display_name: "Reset fence counter".into(),
                spec: RuleSpec {
                    kind: SemanticKind::CumulativeCounter,
                    detector: Detector {
                        mode: iotkit_edge::semantics::DetectorMode::BooleanHighActive,
                        ..Detector::default()
                    },
                    trigger: TriggerMode::OnTransition,
                },
            },
            3,
        )
        .await
        .expect("create counter rule");
    accept_value_at(&storage, 1, 0.0, 100).await;
    semantics
        .reset_counter(&rule.rule_id, 200)
        .await
        .expect("capture reset boundary");
    // These rows would win the global received-at comparator without the reset fence.
    accept_value_at(&storage, 2, 1.0, 0).await;
    accept_value_at(&storage, 3, 0.0, 1).await;
    accept_value_at(&storage, 4, 1.0, 2).await;
    let progress = semantics
        .project_pending(5, registered_output_adapters())
        .await
        .expect("project through reset boundary");
    assert_eq!(
        progress.receipts, 5,
        "four raw receipts plus the reset observation"
    );

    let inspection = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(PathBuf::from(directory.path()).join("edge.db"))
            .create_if_missing(false),
    )
    .await
    .expect("open inspection connection");
    let values: Vec<String> = sqlx::query_scalar(
        "SELECT CAST(value_json AS TEXT) FROM semantic_observations \
         WHERE rule_id=? ORDER BY observation_row_id",
    )
    .bind(&rule.rule_id)
    .fetch_all(&inspection)
    .await
    .expect("read reset ordering");
    assert_eq!(values, vec!["0", "1"]);
}

async fn prepare_multiple_pending_resets(
    storage: &Storage,
    display_name: &str,
) -> (Semantics, String) {
    let semantics = Semantics::new(storage.clone());
    let rule = semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "edge-node-01".into(),
                series_key: SERIES_KEY.into(),
                display_name: display_name.into(),
                spec: RuleSpec {
                    kind: SemanticKind::CumulativeCounter,
                    detector: Detector {
                        mode: iotkit_edge::semantics::DetectorMode::BooleanHighActive,
                        ..Detector::default()
                    },
                    trigger: TriggerMode::OnTransition,
                },
            },
            3,
        )
        .await
        .expect("create counter rule");
    accept_value_in_epoch_at(storage, "epoch-01", 1, 0.0, 100).await;
    semantics
        .reset_counter(&rule.rule_id, 200)
        .await
        .expect("capture first reset boundary");
    // Both later rows would win by received_at without the pending-reset fence.
    accept_value_in_epoch_at(storage, "epoch-01", 2, 1.0, 0).await;
    semantics
        .reset_counter(&rule.rule_id, 201)
        .await
        .expect("capture second reset boundary");
    // This epoch did not exist when either boundary was captured.
    accept_value_in_epoch_at(storage, "epoch-02", 1, 1.0, 1).await;
    (semantics, rule.rule_id)
}

async fn project_one(semantics: &Semantics) {
    assert_eq!(
        semantics
            .project_pending(1, registered_output_adapters())
            .await
            .expect("project one pending item")
            .receipts,
        1
    );
}

#[tokio::test]
async fn multiple_pending_resets_fence_a_new_epoch_until_each_boundary_is_applied_sqlite() {
    let (directory, storage) = store().await;
    let (semantics, rule_id) =
        prepare_multiple_pending_resets(&storage, "SQLite multiple pending reset fences").await;
    let inspection = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(PathBuf::from(directory.path()).join("edge.db"))
            .create_if_missing(false),
    )
    .await
    .expect("open inspection connection");
    let boundary_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM semantic_counter_reset_boundaries AS boundary \
         JOIN semantic_counter_resets AS reset ON reset.reset_id=boundary.reset_id \
         WHERE reset.rule_id=? AND boundary.ledger_epoch='epoch-02'",
    )
    .bind(&rule_id)
    .fetch_one(&inspection)
    .await
    .expect("count uncaptured new-epoch boundaries");
    assert_eq!(boundary_count, 0);

    project_one(&semantics).await;
    let receipts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT ledger_epoch,pub_seq FROM semantic_projection_receipts \
         WHERE rule_id=? ORDER BY ledger_epoch,pub_seq",
    )
    .bind(&rule_id)
    .fetch_all(&inspection)
    .await
    .expect("read first raw receipt");
    assert_eq!(receipts, vec![("epoch-01".into(), 1)]);

    project_one(&semantics).await;
    project_one(&semantics).await;
    let receipts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT ledger_epoch,pub_seq FROM semantic_projection_receipts \
         WHERE rule_id=? ORDER BY ledger_epoch,pub_seq",
    )
    .bind(&rule_id)
    .fetch_all(&inspection)
    .await
    .expect("read old-epoch receipts after first reset");
    assert_eq!(
        receipts,
        vec![("epoch-01".into(), 1), ("epoch-01".into(), 2)]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM semantic_projection_queue WHERE rule_id=? AND ledger_epoch='epoch-02'",
        )
        .bind(&rule_id)
        .fetch_one(&inspection)
        .await
        .expect("count fenced new-epoch queue row"),
        1
    );

    project_one(&semantics).await;
    project_one(&semantics).await;
    let receipts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT ledger_epoch,pub_seq FROM semantic_projection_receipts \
         WHERE rule_id=? ORDER BY ledger_epoch,pub_seq",
    )
    .bind(&rule_id)
    .fetch_all(&inspection)
    .await
    .expect("read fully drained receipts");
    assert_eq!(
        receipts,
        vec![
            ("epoch-01".into(), 1),
            ("epoch-01".into(), 2),
            ("epoch-02".into(), 1),
        ]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM semantic_counter_resets WHERE rule_id=? AND applied_at IS NOT NULL",
        )
        .bind(&rule_id)
        .fetch_one(&inspection)
        .await
        .expect("count applied resets"),
        2
    );
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_multiple_pending_resets_fence_a_new_epoch_until_each_boundary_is_applied() {
    let Some((dsn, storage)) = postgres_store().await else {
        return;
    };
    let (semantics, rule_id) =
        prepare_multiple_pending_resets(&storage, "PostgreSQL multiple pending reset fences").await;
    let inspection = PgPool::connect(&dsn).await.expect("open inspection pool");
    let boundary_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM semantic_counter_reset_boundaries AS boundary \
         JOIN semantic_counter_resets AS reset ON reset.reset_id=boundary.reset_id \
         WHERE reset.rule_id=$1 AND boundary.ledger_epoch='epoch-02'",
    )
    .bind(&rule_id)
    .fetch_one(&inspection)
    .await
    .expect("count uncaptured new-epoch boundaries");
    assert_eq!(boundary_count, 0);

    project_one(&semantics).await;
    let receipts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT ledger_epoch,pub_seq FROM semantic_projection_receipts \
         WHERE rule_id=$1 ORDER BY ledger_epoch,pub_seq",
    )
    .bind(&rule_id)
    .fetch_all(&inspection)
    .await
    .expect("read first raw receipt");
    assert_eq!(receipts, vec![("epoch-01".into(), 1)]);

    project_one(&semantics).await;
    project_one(&semantics).await;
    let receipts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT ledger_epoch,pub_seq FROM semantic_projection_receipts \
         WHERE rule_id=$1 ORDER BY ledger_epoch,pub_seq",
    )
    .bind(&rule_id)
    .fetch_all(&inspection)
    .await
    .expect("read old-epoch receipts after first reset");
    assert_eq!(
        receipts,
        vec![("epoch-01".into(), 1), ("epoch-01".into(), 2)]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM semantic_projection_queue \
             WHERE rule_id=$1 AND ledger_epoch='epoch-02'",
        )
        .bind(&rule_id)
        .fetch_one(&inspection)
        .await
        .expect("count fenced new-epoch queue row"),
        1
    );

    project_one(&semantics).await;
    project_one(&semantics).await;
    let receipts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT ledger_epoch,pub_seq FROM semantic_projection_receipts \
         WHERE rule_id=$1 ORDER BY ledger_epoch,pub_seq",
    )
    .bind(&rule_id)
    .fetch_all(&inspection)
    .await
    .expect("read fully drained receipts");
    assert_eq!(
        receipts,
        vec![
            ("epoch-01".into(), 1),
            ("epoch-01".into(), 2),
            ("epoch-02".into(), 1),
        ]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM semantic_counter_resets \
             WHERE rule_id=$1 AND applied_at IS NOT NULL",
        )
        .bind(&rule_id)
        .fetch_one(&inspection)
        .await
        .expect("count applied resets"),
        2
    );
    inspection.close().await;
}

#[tokio::test]
async fn queue_survives_restart_and_drains_only_rule_end_boundaries_across_epochs() {
    let (directory, storage) = store().await;
    let semantics = Semantics::new(storage.clone());
    let rule = semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "edge-node-01".into(),
                series_key: SERIES_KEY.into(),
                display_name: "Restart boundary temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            3,
        )
        .await
        .expect("create rule");
    accept_in_epoch(&storage, "epoch-01", 1, 10).await;
    accept_in_epoch(&storage, "epoch-02", 1, 11).await;
    semantics
        .retire_rule(&rule.rule_id, 12)
        .await
        .expect("capture rule end boundaries");
    accept_in_epoch(&storage, "epoch-01", 2, 13).await;
    accept_in_epoch(&storage, "epoch-02", 2, 14).await;
    drop(semantics);
    drop(storage);

    let restarted = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("edge.db"),
    })
    .await
    .expect("restart storage with pending queue");
    let projected = Semantics::new(restarted.clone())
        .project_pending(10, registered_output_adapters())
        .await
        .expect("drain pre-end pending queue after restart");
    assert_eq!(projected.receipts, 2);
    let inspection = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(PathBuf::from(directory.path()).join("edge.db"))
            .create_if_missing(false),
    )
    .await
    .expect("open inspection connection");
    let receipts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT ledger_epoch,pub_seq FROM semantic_projection_receipts \
         WHERE rule_id=? ORDER BY ledger_epoch,pub_seq",
    )
    .bind(&rule.rule_id)
    .fetch_all(&inspection)
    .await
    .expect("read durable receipts");
    assert_eq!(
        receipts,
        vec![("epoch-01".into(), 1), ("epoch-02".into(), 1)]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM semantic_projection_queue WHERE rule_id=?",
        )
        .bind(&rule.rule_id)
        .fetch_one(&inspection)
        .await
        .expect("count drained queue"),
        0
    );
}

#[tokio::test]
async fn raw_acceptance_rolls_back_enqueued_work_with_the_raw_cursor() {
    let (directory, storage) = store().await;
    Semantics::new(storage.clone())
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "edge-node-01".into(),
                series_key: SERIES_KEY.into(),
                display_name: "Acceptance atomicity temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            3,
        )
        .await
        .expect("create rule");
    let inspection = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(PathBuf::from(directory.path()).join("edge.db"))
            .create_if_missing(false),
    )
    .await
    .expect("open fault connection");
    sqlx::query(
        "CREATE TRIGGER fail_projection_cursor BEFORE INSERT ON accepted_cursors \
         BEGIN SELECT RAISE(ABORT, 'injected cursor failure'); END",
    )
    .execute(&inspection)
    .await
    .expect("inject post-enqueue failure");
    let error = storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: "atomic-queue".into(),
            received_at: 4,
            records: vec![record(1)],
        })
        .await
        .expect_err("cursor failure must roll back the entire accepted record");
    assert!(error.to_string().contains("injected cursor failure"));
    for table in [
        "raw_records",
        "accepted_cursors",
        "semantic_projection_queue",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&inspection)
                .await
                .expect("count rolled back state"),
            0,
            "{table} must roll back with raw acceptance"
        );
    }
}
