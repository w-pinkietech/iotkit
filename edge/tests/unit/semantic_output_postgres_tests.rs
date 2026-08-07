use crate::{
    application::semantics::{SemanticRuleDraft, Semantics},
    composition::registered_output_adapters,
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, RawRecord, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use sqlx::PgPool;

use super::*;

const EDGE_NODE_ID: &str = "edge-node-ready-plan";
const SERIES_A: &str = "018f0000-0000-7000-8000-000000000101:temperature:na:primary";
const SERIES_B: &str = "018f0000-0000-7000-8000-000000000101:humidity:na:primary";

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
    let descriptor = DescriptorSnapshot::decode(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "edge_node_id": EDGE_NODE_ID,
            "ledger_epoch": "epoch-ready-plan",
            "descriptor_revision": 1,
            "complete": true,
            "devices": [{
                "system_id": "018f0000-0000-7000-8000-000000000101",
                "identifier": "ready-plan-device",
                "state": "active",
                "model_id": "contract"
            }],
            "signals": [{
                "series_key": SERIES_A,
                "system_id": "018f0000-0000-7000-8000-000000000101",
                "measurement_key": "temperature",
                "channel_index": null,
                "variant": "primary",
                "unit": null,
                "value_type": "float"
            }, {
                "series_key": SERIES_B,
                "system_id": "018f0000-0000-7000-8000-000000000101",
                "measurement_key": "humidity",
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
    Some((dsn, storage))
}

fn record(sequence: i64) -> RawRecord {
    let series_key = if sequence % 2 == 0 {
        SERIES_A
    } else {
        SERIES_B
    };
    RawRecord::new(
        sequence,
        serde_json::to_vec(&serde_json::json!({
            "family": "measurement",
            "schema_version": 1,
            "epoch": "epoch-ready-plan",
            "pub_seq": sequence,
            "series_key": series_key,
            "values": [sequence as f64],
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
            edge_node_id: EDGE_NODE_ID.into(),
            ledger_epoch: "epoch-ready-plan".into(),
            publication_id: format!("ready-rule-plan-{first}-{last}"),
            received_at: last,
            records: (first..=last).map(record).collect(),
        })
        .await
        .expect("accept records");
}

async fn seed_applied_reset_history(pool: &PgPool, rule_id: &str) {
    sqlx::query(
        "INSERT INTO semantic_counter_resets(reset_id,rule_id,requested_at,applied_at,\
         zero_observation_id) SELECT concat('ready-rule-plan-', $1, '-', value),$1,value,value,NULL \
         FROM generate_series(1,1000) AS value",
    )
    .bind(rule_id)
    .execute(pool)
    .await
    .expect("seed applied reset history");
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

fn plan_actual_rows(plan: &serde_json::Value, node_type: &str) -> Vec<i64> {
    match plan {
        serde_json::Value::Array(values) => values
            .iter()
            .flat_map(|value| plan_actual_rows(value, node_type))
            .collect(),
        serde_json::Value::Object(values) => {
            let mut rows = Vec::new();
            if values.get("Node Type").and_then(serde_json::Value::as_str) == Some(node_type) {
                rows.push(
                    values
                        .get("Actual Rows")
                        .and_then(serde_json::Value::as_i64)
                        .expect("analyzed plan reports actual rows"),
                );
            }
            rows.extend(
                values
                    .values()
                    .flat_map(|value| plan_actual_rows(value, node_type)),
            );
            rows
        }
        _ => Vec::new(),
    }
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn ready_rule_plan_uses_indexes_and_sorts_only_rule_heads() {
    let Some((dsn, storage)) = postgres_store().await else {
        return;
    };
    let semantics = Semantics::new(storage.clone());
    let first_rule = semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: EDGE_NODE_ID.into(),
                series_key: SERIES_A.into(),
                display_name: "Ready rule plan temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            3,
        )
        .await
        .expect("create first rule");
    let second_rule = semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: EDGE_NODE_ID.into(),
                series_key: SERIES_B.into(),
                display_name: "Ready rule plan humidity".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            4,
        )
        .await
        .expect("create second rule");
    accept(&storage, 1, 1_000).await;
    semantics
        .project_pending(1_000, registered_output_adapters())
        .await
        .expect("complete retained prefix");
    accept(&storage, 1_001, 2_000).await;

    let inspection = PgPool::connect(&dsn).await.expect("open inspection pool");
    seed_applied_reset_history(&inspection, &first_rule.rule_id).await;
    seed_applied_reset_history(&inspection, &second_rule.rule_id).await;
    for table in [
        "semantic_projection_queue",
        "semantic_counter_resets",
        "semantic_rule_runtime",
    ] {
        sqlx::query(&format!("ANALYZE {table}"))
            .execute(&inspection)
            .await
            .expect("analyze ready-rule plan tables");
    }
    let plan: serde_json::Value = sqlx::query_scalar(&format!(
        "EXPLAIN (ANALYZE, FORMAT JSON, TIMING OFF) {READY_RULE_POSTGRES}"
    ))
    .fetch_one(&inspection)
    .await
    .expect("explain production ready-rule query");

    assert!(
        READY_RULE_POSTGRES.contains("FOR UPDATE OF runtime SKIP LOCKED"),
        "ready-rule selection must retain per-rule SKIP LOCKED"
    );
    assert!(
        plan_uses_named_index(
            &plan,
            "semantic_projection_queue",
            "ix_semantic_projection_queue_next"
        ) || plan_uses_named_index(
            &plan,
            "semantic_projection_queue",
            "ix_semantic_projection_queue_rule_next"
        ),
        "ready-rule selection must use a pending queue index: {plan}"
    );
    assert!(
        plan_uses_named_index(
            &plan,
            "semantic_counter_resets",
            "ix_semantic_counter_resets_pending_rule"
        ),
        "ready-rule selection must use the pending-reset index: {plan}"
    );
    for relation in [
        "raw_records",
        "semantic_projection_receipts",
        "semantic_counter_resets",
    ] {
        assert!(
            !plan_has_node(&plan, "Seq Scan", Some(relation)),
            "ready-rule selection must not scan retained {relation}: {plan}"
        );
    }
    let sort_rows = plan_actual_rows(&plan, "Sort");
    assert!(
        sort_rows.iter().all(|rows| *rows <= 2),
        "only the two rule heads may be globally sorted, never retained queue history: {plan}"
    );
    inspection.close().await;
}
