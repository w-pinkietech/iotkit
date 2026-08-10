use std::path::PathBuf;

use iotkit_edge::{
    application::{
        output_profiles::{OutputProfiles, PublicationProvenance},
        profiles::InventoryProfiles,
        semantics::{MappingPreviewRequest, SemanticPreviewRule, SemanticRuleDraft, Semantics},
    },
    auth::{
        password::{Password, hash_password},
        principal::AccountRole,
    },
    composition::registered_output_adapters,
    semantics::{Calibration, Detector, DetectorMode, RuleSpec, SemanticKind, TriggerMode},
    storage::{
        AcceptBatch, AccountProvision, AuditActor, POSTGRES_RECENT_SIGNAL_INPUTS_SQL,
        POSTGRES_SIGNAL_IDENTITY_SQL, RawRecord, SQLITE_RECENT_SIGNAL_INPUTS_SQL,
        SQLITE_SIGNAL_IDENTITY_SQL, Storage, StorageProfile,
    },
};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use serde_json::Map;
use sqlx::{PgPool, Row, SqlitePool, sqlite::SqliteConnectOptions};

const PREVIEW_PLAN_EDGE_NODE_ID: &str = "preview-plan-node";
const PREVIEW_PLAN_SELECTED_SERIES: &str =
    "018f0000-0000-7000-8000-000000000001:temperature:na:primary";
const PREVIEW_PLAN_OTHER_SERIES: &str = "018f0000-0000-7000-8000-000000000001:humidity:na:primary";

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

async fn apply_preview_plan_descriptor(storage: &Storage) -> String {
    storage.initialize_edge_identity(1).await.unwrap();
    let descriptor = DescriptorSnapshot::decode(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "edge_node_id": PREVIEW_PLAN_EDGE_NODE_ID,
            "ledger_epoch": "descriptor-epoch",
            "descriptor_revision": 1,
            "complete": true,
            "devices": [{
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "identifier": "preview-plan-device",
                "state": "active",
                "model_id": "contract"
            }],
            "signals": [{
                "series_key": PREVIEW_PLAN_SELECTED_SERIES,
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "measurement_key": "temperature",
                "channel_index": null,
                "variant": "primary",
                "unit": null,
                "value_type": "float"
            }, {
                "series_key": PREVIEW_PLAN_OTHER_SERIES,
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "measurement_key": "humidity",
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
    InventoryProfiles::new(storage.clone())
        .signals()
        .await
        .unwrap()
        .into_iter()
        .find(|signal| signal.series_key == PREVIEW_PLAN_SELECTED_SERIES)
        .unwrap()
        .signal_ref
}

fn preview_plan_record(ledger_epoch: &str, pub_seq: i64, series_key: &str, value: i64) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "family": "measurement",
        "schema_version": 1,
        "epoch": ledger_epoch,
        "pub_seq": pub_seq,
        "series_key": series_key,
        "values": [value],
        "event_time": value,
        "event_time_source": "received_at",
        "time_source": "edge_node",
        "time_quality": "unsynced",
        "received_at": value,
        "device_time": null
    }))
    .unwrap()
}

async fn insert_preview_tail_sqlite(
    pool: &SqlitePool,
    ledger_epoch: &str,
    pub_seq: i64,
    received_at: i64,
    series_key: &str,
) {
    let record_json = preview_plan_record(ledger_epoch, pub_seq, series_key, received_at);
    sqlx::query(
        "INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,publication_id,record_json,\
         record_sha256,received_at,series_key) VALUES(?,?,?,?,?,?,?,?)",
    )
    .bind(PREVIEW_PLAN_EDGE_NODE_ID)
    .bind(ledger_epoch)
    .bind(pub_seq)
    .bind(format!("preview-{ledger_epoch}-{pub_seq}"))
    .bind(record_json)
    .bind(vec![0_u8; 32])
    .bind(received_at)
    .bind(series_key)
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_preview_tail_postgres(
    pool: &PgPool,
    ledger_epoch: &str,
    pub_seq: i64,
    received_at: i64,
    series_key: &str,
) {
    let record_json = preview_plan_record(ledger_epoch, pub_seq, series_key, received_at);
    sqlx::query(
        "INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,publication_id,record_json,\
         record_sha256,received_at,series_key) VALUES($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(PREVIEW_PLAN_EDGE_NODE_ID)
    .bind(ledger_epoch)
    .bind(pub_seq)
    .bind(format!("preview-{ledger_epoch}-{pub_seq}"))
    .bind(record_json)
    .bind(vec![0_u8; 32])
    .bind(received_at)
    .bind(series_key)
    .execute(pool)
    .await
    .unwrap();
}

fn preview_identity(input: &iotkit_edge::storage::StoredPreviewInput) -> (i64, String, i64) {
    let record: serde_json::Value = serde_json::from_slice(&input.record_json).unwrap();
    (
        input.received_at,
        record["epoch"].as_str().unwrap().into(),
        record["pub_seq"].as_i64().unwrap(),
    )
}

async fn assert_preview_tail(storage: &Storage, signal_ref: &str) {
    let inputs = storage.recent_signal_inputs(signal_ref, 4).await.unwrap();
    assert_eq!(
        inputs.iter().map(preview_identity).collect::<Vec<_>>(),
        vec![
            (102_000, "epoch-b".into(), 1),
            (103_000, "epoch-a".into(), 10_002),
            (103_000, "epoch-c".into(), 1),
            (103_000, "epoch-c".into(), 2),
        ],
        "the bounded tail must be returned ascending after selecting the latest rows"
    );
    assert!(
        storage
            .recent_signal_inputs("missing-signal", 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(storage.recent_signal_inputs(signal_ref, 0).await.is_err());
    assert!(
        storage
            .recent_signal_inputs(signal_ref, 2_000)
            .await
            .is_ok()
    );
    assert!(
        storage
            .recent_signal_inputs(signal_ref, 2_001)
            .await
            .is_err()
    );
}

async fn fixture() -> (tempfile::TempDir, Storage, String) {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("edge.db"),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage.apply_descriptor(&descriptor, 2).await.unwrap();
    let signal_ref = InventoryProfiles::new(storage.clone())
        .signals()
        .await
        .unwrap()[0]
        .signal_ref
        .clone();
    const BASE_RECEIVED_AT: i64 = 100_000;
    const BASE_OBSERVED_AT: i64 = 200_000;
    for (sequence, value) in [(1, 18.0), (2, 21.0), (3, 19.0), (4, 22.0)] {
        let record = serde_json::json!({
            "family":"measurement","schema_version":1,"epoch":"epoch-01",
            "pub_seq":sequence,"series_key":descriptor.signals[0].series_key,
            "values":[value],"event_time":BASE_OBSERVED_AT+sequence*1000,
            "event_time_source":"received_at","time_source":"edge_node",
            "time_quality":"unsynced","received_at":BASE_RECEIVED_AT,"device_time":null
        });
        storage
            .accept_batch(AcceptBatch {
                edge_node_id: descriptor.edge_node_id.clone(),
                ledger_epoch: descriptor.ledger_epoch.clone(),
                publication_id: format!("preview-{sequence}"),
                received_at: BASE_RECEIVED_AT,
                records: vec![
                    RawRecord::new(sequence, serde_json::to_vec(&record).unwrap()).unwrap(),
                ],
            })
            .await
            .unwrap();
    }
    (directory, storage, signal_ref)
}

#[tokio::test]
async fn sqlite_recent_signal_inputs_uses_the_indexed_bounded_signal_tail() {
    const RETAINED_PREFIX: i64 = 20_000;

    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("edge.db");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: database.clone(),
    })
    .await
    .unwrap();
    let signal_ref = apply_preview_plan_descriptor(&storage).await;
    let inspection = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(database)
            .create_if_missing(false),
    )
    .await
    .unwrap();
    sqlx::query(
        "WITH RECURSIVE sequence(value) AS (VALUES(1) UNION ALL SELECT value+1 FROM sequence \
         WHERE value<$4) INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,\
         publication_id,record_json,record_sha256,received_at,series_key) \
         SELECT $1,'prefix-epoch',value,'prefix-' || value,CAST($2 AS BLOB),zeroblob(32),\
         200000+value,$3 FROM sequence",
    )
    .bind(PREVIEW_PLAN_EDGE_NODE_ID)
    .bind(r#"{"family":"measurement","series_key":"other","values":[0],"event_time":0}"#)
    .bind(PREVIEW_PLAN_OTHER_SERIES)
    .bind(RETAINED_PREFIX)
    .execute(&inspection)
    .await
    .unwrap();
    for (epoch, sequence, received_at, series_key) in [
        ("epoch-a", 10_001, 101_000, PREVIEW_PLAN_SELECTED_SERIES),
        ("epoch-b", 1, 102_000, PREVIEW_PLAN_SELECTED_SERIES),
        ("epoch-a", 10_002, 103_000, PREVIEW_PLAN_SELECTED_SERIES),
        ("epoch-c", 1, 103_000, PREVIEW_PLAN_SELECTED_SERIES),
        ("epoch-c", 2, 103_000, PREVIEW_PLAN_SELECTED_SERIES),
        ("epoch-z", 1, 999_999, PREVIEW_PLAN_OTHER_SERIES),
    ] {
        insert_preview_tail_sqlite(&inspection, epoch, sequence, received_at, series_key).await;
    }
    sqlx::query("ANALYZE raw_records")
        .execute(&inspection)
        .await
        .unwrap();
    let identity: (String, String) = sqlx::query_as(SQLITE_SIGNAL_IDENTITY_SQL)
        .bind(&signal_ref)
        .fetch_one(&inspection)
        .await
        .unwrap();
    let plan = sqlx::query(&format!(
        "EXPLAIN QUERY PLAN {SQLITE_RECENT_SIGNAL_INPUTS_SQL}"
    ))
    .bind(&identity.0)
    .bind(&identity.1)
    .bind(2_000_i64)
    .fetch_all(&inspection)
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.try_get::<String, _>("detail").unwrap())
    .collect::<Vec<_>>()
    .join("\n");
    assert!(
        plan.contains("ix_raw_records_preview_signal_received"),
        "preview tail must use the edge-node/signal receipt index: {plan}"
    );
    assert!(
        !plan.contains("SCAN raw") && !plan.contains("USE TEMP B-TREE"),
        "preview tail must not scan or sort retained raw history: {plan}"
    );
    assert_preview_tail(&storage, &signal_ref).await;
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_recent_signal_inputs_uses_the_indexed_bounded_signal_tail() {
    const RETAINED_PREFIX: i64 = 20_000;

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
    let signal_ref = apply_preview_plan_descriptor(&storage).await;
    let inspection = PgPool::connect(&dsn).await.unwrap();
    sqlx::query(
        "INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,publication_id,record_json,\
         record_sha256,received_at,series_key) SELECT $1,'prefix-epoch',value,\
         'prefix-' || value,convert_to($2,'UTF8'),decode(repeat('00',32),'hex'),200000+value,$3 \
         FROM generate_series(1,$4) AS sequence(value)",
    )
    .bind(PREVIEW_PLAN_EDGE_NODE_ID)
    .bind(r#"{"family":"measurement","series_key":"other","values":[0],"event_time":0}"#)
    .bind(PREVIEW_PLAN_OTHER_SERIES)
    .bind(RETAINED_PREFIX)
    .execute(&inspection)
    .await
    .unwrap();
    for (epoch, sequence, received_at, series_key) in [
        ("epoch-a", 10_001, 101_000, PREVIEW_PLAN_SELECTED_SERIES),
        ("epoch-b", 1, 102_000, PREVIEW_PLAN_SELECTED_SERIES),
        ("epoch-a", 10_002, 103_000, PREVIEW_PLAN_SELECTED_SERIES),
        ("epoch-c", 1, 103_000, PREVIEW_PLAN_SELECTED_SERIES),
        ("epoch-c", 2, 103_000, PREVIEW_PLAN_SELECTED_SERIES),
        ("epoch-z", 1, 999_999, PREVIEW_PLAN_OTHER_SERIES),
    ] {
        insert_preview_tail_postgres(&inspection, epoch, sequence, received_at, series_key).await;
    }
    sqlx::query("ANALYZE raw_records")
        .execute(&inspection)
        .await
        .unwrap();
    let identity: (String, String) = sqlx::query_as(POSTGRES_SIGNAL_IDENTITY_SQL)
        .bind(&signal_ref)
        .fetch_one(&inspection)
        .await
        .unwrap();
    let plan: serde_json::Value = sqlx::query_scalar(&format!(
        "EXPLAIN (ANALYZE, FORMAT JSON, COSTS OFF, TIMING OFF, SUMMARY OFF) \
         {POSTGRES_RECENT_SIGNAL_INPUTS_SQL}"
    ))
    .bind(&identity.0)
    .bind(&identity.1)
    .bind(2_000_i64)
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert!(
        POSTGRES_RECENT_SIGNAL_INPUTS_SQL.contains("md5(series_key)=md5($2) AND series_key=$2"),
        "the PostgreSQL digest discriminator must recheck the complete key"
    );
    assert!(
        plan_uses_named_index(
            &plan,
            "raw_records",
            "ix_raw_records_preview_signal_received"
        ),
        "preview tail must use the edge-node/signal receipt index: {plan}"
    );
    assert!(
        !plan_has_node(&plan, "Seq Scan", Some("raw_records"))
            && !plan_has_node(&plan, "Sort", None),
        "preview tail must not scan or sort retained raw history: {plan}"
    );
    assert_preview_tail(&storage, &signal_ref).await;
    inspection.close().await;
}

#[tokio::test]
async fn semantic_preview_uses_real_calibration_evaluator_and_bounded_raw_window() {
    let (_directory, storage, signal_ref) = fixture().await;
    let response = Semantics::new(storage)
        .preview(MappingPreviewRequest {
            signal_ref,
            calibration: Calibration {
                scale: 2.0,
                offset: 1.0,
            },
            rules: vec![SemanticPreviewRule {
                rule_id: "draft-counter".into(),
                display_name: "Production".into(),
                spec: RuleSpec {
                    kind: SemanticKind::CumulativeCounter,
                    detector: Detector {
                        mode: DetectorMode::HighActive,
                        rise_threshold: 20.0,
                        fall_threshold: 19.0,
                        ..Detector::default()
                    },
                    trigger: TriggerMode::OnTransition,
                },
            }],
            test_value: Some(10.0),
        })
        .await
        .unwrap();
    assert_eq!(response.rules.len(), 1);
    assert_eq!(response.rules[0].input_count, 4);
    assert_eq!(response.rules[0].points[0].calibrated, 37.0);
    assert_eq!(response.rules[0].points[0].received_at, 100_000);
    assert_eq!(response.rules[0].points[0].plot_at, 201_000);
    assert_eq!(response.rules[0].latest_point.unwrap().received_at, 100_000);
    assert_eq!(response.rules[0].latest_point.unwrap().plot_at, 204_000);
    assert_eq!(
        response.rules[0].test_result.as_ref().unwrap().calibrated,
        21.0
    );
    assert_eq!(response.window_start, Some(144_000));
    assert_eq!(response.window_end, Some(204_000));
}

#[tokio::test]
async fn semantic_preview_rejects_blank_rule_identity_and_display_name() {
    let (_directory, storage, signal_ref) = fixture().await;
    for (rule_id, display_name) in [(" ", "Preview"), ("draft-preview", " \t")] {
        let result = Semantics::new(storage.clone())
            .preview(MappingPreviewRequest {
                signal_ref: signal_ref.clone(),
                calibration: Calibration {
                    scale: 1.0,
                    offset: 0.0,
                },
                rules: vec![SemanticPreviewRule {
                    rule_id: rule_id.into(),
                    display_name: display_name.into(),
                    spec: RuleSpec {
                        kind: SemanticKind::Numeric,
                        detector: Detector::default(),
                        trigger: TriggerMode::None,
                    },
                }],
                test_value: None,
            })
            .await;
        assert!(result.is_err(), "{rule_id:?} / {display_name:?} must fail");
    }
}

#[tokio::test]
async fn output_preview_uses_policy_transform_and_durable_puback_state() {
    let (_directory, storage, signal_ref) = fixture().await;
    let semantics = Semantics::new(storage.clone());
    let rule = semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "edge-node-01".into(),
                series_key: "018f0000-0000-7000-8000-000000000001:contact_state:na:primary".into(),
                display_name: "Temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            2000,
        )
        .await
        .unwrap();
    assert_eq!(rule.signal_ref, signal_ref);
    let outputs = OutputProfiles::new(storage.clone(), registered_output_adapters());
    let activation = outputs
        .preview_activation("iotkit.mqtt-json.v1")
        .await
        .unwrap();
    assert_eq!(activation.automatic_count, 1);
    let profile = outputs
        .activate("Generic", "iotkit.mqtt-json.v1", Map::new(), 2001)
        .await
        .unwrap();
    let publication = outputs
        .publication(&profile.bindings[0].binding_id, 2002)
        .await
        .unwrap();
    assert_eq!(publication.provenance, PublicationProvenance::Sample);
    assert!(publication.topic.starts_with("iotkit/v1/sources/edge-"));
    assert_eq!(publication.delivery.pending_count, 0);
    assert_eq!(publication.delivery.state, "waiting_for_observation");
}

#[tokio::test]
async fn semantic_and_output_mutations_attribute_the_authenticated_actor() {
    let (_directory, storage, _signal_ref) = fixture().await;
    let account = storage
        .create_account(
            AccountProvision {
                login_id: "console".into(),
                display_name: "Console operator".into(),
                role: AccountRole::SystemAdmin,
                password_hash: hash_password(
                    &Password::new("correct horse battery staple").unwrap(),
                )
                .unwrap(),
                must_change_password: false,
                require_unowned: true,
            },
            AuditActor::local_cli(),
            1_999,
        )
        .await
        .unwrap();
    let actor = AuditActor::account(&account.account_ref);
    let rule = Semantics::new(storage.clone())
        .create_rule_as(
            actor.clone(),
            SemanticRuleDraft {
                edge_node_id: "edge-node-01".into(),
                series_key: "018f0000-0000-7000-8000-000000000001:contact_state:na:primary".into(),
                display_name: "Temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            2_000,
        )
        .await
        .unwrap();
    let profile = OutputProfiles::new(storage.clone(), registered_output_adapters())
        .activate_as(actor, "Generic", "iotkit.mqtt-json.v1", Map::new(), 2_001)
        .await
        .unwrap();
    assert_eq!(profile.bindings[0].rule_id, rule.rule_id);
    let audit = storage.list_audit_events(100).await.unwrap();
    for operation in ["semantic_rule.create", "export_profile.activate"] {
        let event = audit
            .iter()
            .find(|event| event.operation == operation)
            .unwrap();
        assert_eq!(event.actor_ref, account.account_ref);
    }
}
