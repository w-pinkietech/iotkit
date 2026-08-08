use iotkit_edge::application::cli_compat::{
    CliQueries, LegacyMappingSpec, LegacyMappings, LegacyRoutes, LegacyTriggerMode,
    legacy_mapping_id, legacy_route_id, route_id_from_legacy_route, rule_id_from_legacy_mapping,
};
use iotkit_edge::application::semantics::Semantics;
use iotkit_edge::composition::generic_output_adapter;
use iotkit_edge::composition::registered_output_adapters;
use iotkit_edge::storage::{AcceptBatch, RawRecord, Storage, StorageProfile};
use iotkit_edge::storage::{StorageError, migrate_sqlite_to_postgres};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use tempfile::TempDir;

async fn apply_contact_descriptor(storage: &Storage, edge_node_id: &str) {
    let descriptor = DescriptorSnapshot::decode(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "edge_node_id": edge_node_id,
            "ledger_epoch": "epoch-01",
            "descriptor_revision": 1,
            "complete": true,
            "devices": [{
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "identifier": "cli-contract-device",
                "state": "active",
                "model_id": "contract"
            }],
            "signals": [{
                "series_key": "018f0000-0000-7000-8000-000000000001:contact:na:primary",
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "measurement_key": "contact",
                "channel_index": null,
                "variant": "primary",
                "unit": null,
                "value_type": "bool"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    storage.apply_descriptor(&descriptor, 1).await.unwrap();
}

#[test]
fn compatibility_ids_are_lowercase_and_reversible() {
    let rule_id = "550e8400-e29b-41d4-a716-446655440000";
    let route_id = "route_550e8400-e29b-41d4-a716-446655440001";

    assert_eq!(
        legacy_mapping_id(rule_id).unwrap(),
        "sm-550e8400e29b41d4a716446655440000"
    );
    assert_eq!(
        rule_id_from_legacy_mapping("sm-550e8400e29b41d4a716446655440000").unwrap(),
        rule_id
    );
    assert_eq!(
        legacy_route_id(route_id).unwrap(),
        "mr-550e8400e29b41d4a716446655440001"
    );
    assert_eq!(
        route_id_from_legacy_route("mr-550e8400e29b41d4a716446655440001").unwrap(),
        route_id
    );
}

#[test]
fn compatibility_ids_reject_noncanonical_input() {
    for mapping in [
        "",
        "sm-550E8400e29b41d4a716446655440000",
        "sm_550e8400e29b41d4a716446655440000",
        "sm-550e8400-e29b-41d4-a716-446655440000",
    ] {
        assert!(rule_id_from_legacy_mapping(mapping).is_err(), "{mapping}");
    }
    assert!(legacy_mapping_id("not-a-uuid").is_err());
    assert!(legacy_route_id("out_550e8400-e29b-41d4-a716-446655440001").is_err());
}

#[tokio::test]
async fn raw_query_preserves_go_order_and_json_shape() {
    let root = TempDir::new_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: root.path().join("edge.db"),
    })
    .await
    .unwrap();
    for (edge, received_at, value) in [("edge-node-a", 10, 1), ("edge-node-b", 20, 2)] {
        storage
            .accept_batch(AcceptBatch {
                edge_node_id: edge.into(),
                ledger_epoch: "epoch-01".into(),
                publication_id: format!("{edge}:epoch-01:1:1"),
                received_at,
                records: vec![
                    RawRecord::new(
                        1,
                        format!("{{\"series_key\":\"contact\",\"values\":[{value}]}}"),
                    )
                    .unwrap(),
                ],
            })
            .await
            .unwrap();
    }

    let records = CliQueries::new(storage).raw_records(100).await.unwrap();
    assert_eq!(records[0].edge_node_id, "edge-node-b");
    assert_eq!(
        serde_json::to_value(&records[0]).unwrap()["record"]["values"][0],
        2
    );
    assert_eq!(records[0].publication_id, "edge-node-b:epoch-01:1:1");
}

#[tokio::test]
async fn raw_query_rejects_go_incompatible_limits() {
    let root = TempDir::new_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: root.path().join("edge.db"),
    })
    .await
    .unwrap();
    let query = CliQueries::new(storage);
    assert!(query.raw_records(0).await.is_err());
    assert!(query.raw_records(10_001).await.is_err());
}

#[tokio::test]
async fn mapping_lifecycle_is_a_reversible_semantic_rule_view() {
    let root = TempDir::new_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: root.path().join("edge.db"),
    })
    .await
    .unwrap();
    apply_contact_descriptor(&storage, "edge-node-01").await;
    let mappings = LegacyMappings::new(storage.clone());
    let first = mappings
        .put(
            LegacyMappingSpec {
                edge_node_id: "edge-node-01".into(),
                series_key: "018f0000-0000-7000-8000-000000000001:contact:na:primary".into(),
                meaning: "production_pulse".into(),
                trigger_mode: LegacyTriggerMode::ActiveSample,
                active_value: 1,
            },
            10,
        )
        .await
        .unwrap();
    let second = mappings
        .put(
            LegacyMappingSpec {
                edge_node_id: "edge-node-01".into(),
                series_key: "018f0000-0000-7000-8000-000000000001:contact:na:primary".into(),
                meaning: "production_pulse".into(),
                trigger_mode: LegacyTriggerMode::ActiveEdge,
                active_value: 0,
            },
            20,
        )
        .await
        .unwrap();

    assert_eq!(first.mapping_id, second.mapping_id);
    assert_eq!((first.revision, second.revision), (1, 2));
    let revisions = mappings.list().await.unwrap();
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0].trigger_mode, LegacyTriggerMode::ActiveSample);
    assert_eq!(revisions[1].trigger_mode, LegacyTriggerMode::ActiveEdge);
    let retired = mappings
        .deactivate(
            "edge-node-01",
            "018f0000-0000-7000-8000-000000000001:contact:na:primary",
            30,
        )
        .await
        .unwrap();
    assert!(!retired.active);
    assert_eq!(retired.revision, 2);
    let audit = storage.list_audit_events(10).await.unwrap();
    assert_eq!(
        audit
            .iter()
            .map(|event| event.operation.as_str())
            .collect::<Vec<_>>(),
        ["mapping_deactivate", "mapping_set", "mapping_set"]
    );
}

#[tokio::test]
async fn route_add_is_idempotent_and_supports_topic_fanout() {
    let root = TempDir::new_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: root.path().join("edge.db"),
    })
    .await
    .unwrap();
    apply_contact_descriptor(&storage, "edge-node-01").await;
    let mapping = LegacyMappings::new(storage.clone())
        .put(
            LegacyMappingSpec {
                edge_node_id: "edge-node-01".into(),
                series_key: "018f0000-0000-7000-8000-000000000001:contact:na:primary".into(),
                meaning: "production_pulse".into(),
                trigger_mode: LegacyTriggerMode::ActiveEdge,
                active_value: 1,
            },
            10,
        )
        .await
        .unwrap();
    let routes = LegacyRoutes::new(storage, generic_output_adapter());
    let first = routes
        .add(&mapping.mapping_id, "factory/a/pulse", 20)
        .await
        .unwrap();
    let replay = routes
        .add(&mapping.mapping_id, "factory/a/pulse", 30)
        .await
        .unwrap();
    let second = routes
        .add(&mapping.mapping_id, "factory/b/pulse", 40)
        .await
        .unwrap();

    assert_eq!(first.route_id, replay.route_id);
    assert_ne!(first.route_id, second.route_id);
    let statuses = routes.list().await.unwrap();
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[0].qos, 1);
    assert_eq!(statuses[0].pending_count, 0);
    assert_eq!(statuses[0].published_count, 0);
}

#[tokio::test]
async fn route_add_rejects_topics_the_go_cli_rejected() {
    let root = TempDir::new_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: root.path().join("edge.db"),
    })
    .await
    .unwrap();
    let routes = LegacyRoutes::new(storage, generic_output_adapter());
    for topic in ["", "/factory/a", "factory/a/", "factory/+/a", "factory/#"] {
        assert!(
            routes
                .add("sm-550e8400e29b41d4a716446655440000", topic, 1)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn storage_migrate_explicitly_rejects_a_go_era_database() {
    let root = TempDir::new_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .unwrap();
    let path = root.path().join("go-edge.db");
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    sqlx::query("CREATE TABLE raw_records(edge_node_id TEXT,ledger_epoch TEXT,pub_seq INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let error = migrate_sqlite_to_postgres(&path, "postgres://must-not-be-opened")
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::UnsupportedLegacySchema));
}

async fn accept_contact(storage: &Storage, sequence: i64, value: i32) {
    let record = serde_json::json!({
        "family": "measurement",
        "schema_version": 1,
        "epoch": "epoch-01",
        "pub_seq": sequence,
        "series_key": "018f0000-0000-7000-8000-000000000001:contact:na:primary",
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
            records: vec![RawRecord::new(sequence, serde_json::to_vec(&record).unwrap()).unwrap()],
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn routes_are_future_only_and_fan_out_one_observation() {
    let root = TempDir::new_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: root.path().join("edge.db"),
    })
    .await
    .unwrap();
    apply_contact_descriptor(&storage, "edge-node-01").await;
    let mapping = LegacyMappings::new(storage.clone())
        .put(
            LegacyMappingSpec {
                edge_node_id: "edge-node-01".into(),
                series_key: "018f0000-0000-7000-8000-000000000001:contact:na:primary".into(),
                meaning: "production_pulse".into(),
                trigger_mode: LegacyTriggerMode::ActiveEdge,
                active_value: 1,
            },
            1,
        )
        .await
        .unwrap();
    accept_contact(&storage, 1, 0).await;
    accept_contact(&storage, 2, 1).await;
    Semantics::new(storage.clone())
        .project_pending(10, registered_output_adapters())
        .await
        .unwrap();

    let routes = LegacyRoutes::new(storage.clone(), generic_output_adapter());
    routes
        .add(&mapping.mapping_id, "factory/a/pulse", 10)
        .await
        .unwrap();
    routes
        .add(&mapping.mapping_id, "factory/b/pulse", 11)
        .await
        .unwrap();
    assert!(
        routes
            .list()
            .await
            .unwrap()
            .iter()
            .all(|route| route.pending_count == 0)
    );

    accept_contact(&storage, 3, 0).await;
    accept_contact(&storage, 4, 1).await;
    let projected = Semantics::new(storage.clone())
        .project_pending(10, registered_output_adapters())
        .await
        .unwrap();
    assert_eq!(projected.publications, 2);
    assert!(
        routes
            .list()
            .await
            .unwrap()
            .iter()
            .all(|route| route.pending_count == 1)
    );
    let events = CliQueries::new(storage).semantic_events(100).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].mapping_id, mapping.mapping_id);
    assert_eq!(events[0].meaning, "production_pulse");
    assert_eq!(events[1].event_sequence, 2);
}

#[tokio::test]
async fn postgres_migration_copies_and_verifies_a_fresh_rust_schema_when_configured() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let root = TempDir::new_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .unwrap();
    let path = root.path().join("source.db");
    let storage = Storage::connect(StorageProfile::Sqlite { path: path.clone() })
        .await
        .unwrap();
    let edge_id = storage.initialize_edge_identity(1).await.unwrap();
    apply_contact_descriptor(&storage, "edge-node-01").await;
    LegacyMappings::new(storage.clone())
        .put(
            LegacyMappingSpec {
                edge_node_id: "edge-node-01".into(),
                series_key: "018f0000-0000-7000-8000-000000000001:contact:na:primary".into(),
                meaning: "production_pulse".into(),
                trigger_mode: LegacyTriggerMode::ActiveEdge,
                active_value: 1,
            },
            2,
        )
        .await
        .unwrap();
    accept_contact(&storage, 1, 1).await;
    drop(storage);

    let report = migrate_sqlite_to_postgres(&path, &dsn).await.unwrap();
    assert!(report.completed);
    assert_eq!(report.edge_id, edge_id);
    assert_eq!(report.schema_version, 10);
    assert_eq!(report.table_counts["raw_records"], 1);
    assert_eq!(report.table_counts["semantic_projection_queue"], 1);
    assert_eq!(report.cursors[0].accepted_through, 1);
    assert_eq!(report.content_digest.len(), 64);
    let target = sqlx::PgPool::connect(&dsn).await.unwrap();
    let history_index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_indexes WHERE schemaname='public' \
         AND tablename='semantic_observations' \
         AND indexname='ix_semantic_observation_rule_observed_at_row'",
    )
    .fetch_one(&target)
    .await
    .unwrap();
    assert_eq!(history_index_count, 1);
    target.close().await;
}

#[tokio::test]
async fn postgres_mapping_and_route_contracts_match_when_configured() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let storage = Storage::connect(StorageProfile::Postgres { dsn })
        .await
        .unwrap();
    apply_contact_descriptor(&storage, "edge-node-pg").await;
    let mappings = LegacyMappings::new(storage.clone());
    let mapping = mappings
        .put(
            LegacyMappingSpec {
                edge_node_id: "edge-node-pg".into(),
                series_key: "018f0000-0000-7000-8000-000000000001:contact:na:primary".into(),
                meaning: "production_pulse".into(),
                trigger_mode: LegacyTriggerMode::ActiveEdge,
                active_value: 1,
            },
            10,
        )
        .await
        .unwrap();
    let routes = LegacyRoutes::new(storage, generic_output_adapter());
    routes
        .add(&mapping.mapping_id, "factory/postgres/pulse", 20)
        .await
        .unwrap();
    assert_eq!(routes.list().await.unwrap().len(), 1);
    assert!(
        !mappings
            .deactivate(
                "edge-node-pg",
                "018f0000-0000-7000-8000-000000000001:contact:na:primary",
                30,
            )
            .await
            .unwrap()
            .active
    );
}

#[tokio::test]
async fn postgres_migration_failure_rolls_back_every_copied_row_when_configured() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let root = TempDir::new_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .unwrap();
    let path = root.path().join("rollback-source.db");
    let source = Storage::connect(StorageProfile::Sqlite { path: path.clone() })
        .await
        .unwrap();
    source.initialize_edge_identity(1).await.unwrap();
    source
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-rollback".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: "publication-rollback".into(),
            received_at: 2,
            records: vec![RawRecord::new(1, r#"{"series_key":"contact","values":[1]}"#).unwrap()],
        })
        .await
        .unwrap();
    drop(source);

    drop(
        Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
            .await
            .unwrap(),
    );
    let inspection = sqlx::PgPool::connect(&dsn).await.unwrap();
    sqlx::query(
        "CREATE FUNCTION fail_raw_copy() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected migration failure'; END $$",
    )
    .execute(&inspection)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_raw_copy BEFORE INSERT ON raw_records \
         FOR EACH ROW EXECUTE FUNCTION fail_raw_copy()",
    )
    .execute(&inspection)
    .await
    .unwrap();

    assert!(migrate_sqlite_to_postgres(&path, &dsn).await.is_err());
    for table in ["edge_meta", "raw_records", "accepted_cursors"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&inspection)
            .await
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}
