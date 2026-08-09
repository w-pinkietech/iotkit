use std::collections::BTreeSet;

use iotkit_edge::{
    diagnostics::{diagnostics_with_runtime, storage_status},
    mqtt::ingest::{IngestConnectionState, IngestRuntimeHealth},
    storage::{Storage, StorageProfile},
};
use tempfile::TempDir;

fn keys(value: &serde_json::Value) -> BTreeSet<String> {
    value.as_object().unwrap().keys().cloned().collect()
}

#[tokio::test]
async fn closed_openapi_storage_and_diagnostics_schemas_track_runtime_json() {
    let directory = TempDir::new().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("edge.db"),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();

    let storage_json = serde_json::to_value(storage_status(&storage, 90).await.unwrap()).unwrap();
    let storage_keys = keys(&storage_json);
    assert!(storage_keys.is_subset(&BTreeSet::from([
        "profile".into(),
        "state".into(),
        "filesystem_available".into(),
        "database_bytes".into(),
        "reclaimable_bytes".into(),
        "disk_total_bytes".into(),
        "disk_available_bytes".into(),
        "disk_used_percent".into(),
        "warning_percent".into(),
        "raw_record_count".into(),
        "semantic_observation_count".into(),
        "pending_semantic_projection_count".into(),
        "pending_output_count".into(),
        "projection_failure_count".into(),
        "last_backup_id".into(),
        "last_backup_at".into(),
        "absolute_reserve_state".into(),
    ])));

    let report = diagnostics_with_runtime(
        &storage,
        90,
        1,
        None,
        IngestRuntimeHealth {
            state: IngestConnectionState::Unknown,
            last_ready_at: None,
        },
    )
    .await
    .unwrap();
    let report_json = serde_json::to_value(report).unwrap();
    assert_eq!(
        keys(&report_json),
        BTreeSet::from([
            "generated_at".into(),
            "state".into(),
            "issues".into(),
            "truncated".into(),
            "limitations".into(),
            "stages".into(),
        ])
    );
    for stage in report_json["stages"].as_array().unwrap() {
        let stage_keys = keys(stage);
        assert!(stage_keys.is_subset(&BTreeSet::from([
            "stage".into(),
            "state".into(),
            "code".into(),
            "last_success_at".into(),
            "affected_count".into(),
            "scope".into(),
            "cause".into(),
            "action".into(),
            "href".into(),
            "blocked_by".into(),
        ])));
        for required in [
            "stage",
            "state",
            "code",
            "affected_count",
            "scope",
            "cause",
            "action",
            "href",
        ] {
            assert!(stage_keys.contains(required), "missing {required}");
        }
    }

    let schema = include_str!("../openapi/edge-console-v1.yaml");
    let storage_schema = schema
        .split_once("    StorageStatus:\n")
        .expect("StorageStatus schema")
        .1
        .split_once("    DiagnosticIssue:\n")
        .expect("StorageStatus schema end")
        .0;
    for required in [
        "profile",
        "state",
        "filesystem_available",
        "database_bytes",
        "reclaimable_bytes",
        "disk_total_bytes",
        "disk_available_bytes",
        "disk_used_percent",
        "warning_percent",
        "raw_record_count",
        "semantic_observation_count",
        "pending_semantic_projection_count",
        "pending_output_count",
        "projection_failure_count",
        "absolute_reserve_state",
    ] {
        assert!(
            storage_keys.contains(required),
            "runtime StorageStatus is missing {required}"
        );
        assert!(
            storage_schema.contains(&format!("        - {required}\n")),
            "OpenAPI StorageStatus must require {required}"
        );
    }
    for fragment in [
        "StorageStatus:\n      type: object\n      additionalProperties: false",
        "DiagnosticReport:\n      type: object\n      additionalProperties: false",
        "DiagnosticStage:\n      type: object\n      additionalProperties: false",
        "CertificateStatus:\n      type: object\n      additionalProperties: false",
        "required: [generated_at, state, issues, truncated, limitations, stages]",
        "broker_certificate:\n          $ref: \"#/components/schemas/CertificateStatus\"",
    ] {
        assert!(schema.contains(fragment), "OpenAPI is missing {fragment}");
    }
    let generated = include_str!("../frontend/src/generated/edge-api.d.ts");
    assert!(generated.contains("DiagnosticStage: {"));
    assert!(generated.contains("CertificateStatus: {"));
}
