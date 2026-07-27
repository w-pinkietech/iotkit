use std::{collections::HashMap, path::PathBuf};

use iotkit_edge::{
    application::{
        accounts::AccountService,
        output_profiles::OutputProfiles,
        semantics::{SemanticRuleDraft, Semantics},
    },
    auth::password::Password,
    composition::{StorageWebApplication, registered_output_adapters},
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{Storage, StorageProfile},
    web::{ConsoleRequest, WebApplication},
};
use iotkit_edge_custody_contract::{ActivationRequest, ActivationResult, DescriptorSnapshot};
use serde_json::Map;

fn test_directory() -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::TempDir::new_in(root).unwrap()
}

#[tokio::test]
async fn production_console_composes_live_output_delivery_facts_and_frees_stopped_adapter() {
    let directory = test_directory();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("output-console-read-model.db"),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    AccountService::new(storage.clone())
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new("long enough owner password").unwrap(),
            2,
        )
        .await
        .unwrap();

    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage.apply_descriptor(&descriptor, 3).await.unwrap();
    let command = storage
        .request_activation(&descriptor.edge_node_id, 4)
        .await
        .unwrap();
    let activation = ActivationRequest::decode(&command.payload_json).unwrap();
    storage
        .apply_activation_result(
            &ActivationResult {
                schema_version: 1,
                activation_id: activation.activation_id,
                edge_id: activation.edge_id,
                edge_node_id: activation.edge_node_id,
                ledger_epoch: activation.expected_ledger_epoch,
                status: "applied".into(),
                discard_through_reading_seq: 0,
                first_publication_seq: 1,
                applied_at: 5,
            },
            5,
        )
        .await
        .unwrap();

    Semantics::new(storage.clone())
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: descriptor.edge_node_id,
                series_key: descriptor.signals[0].series_key.clone(),
                display_name: "Temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            6,
        )
        .await
        .unwrap();
    let output_profiles = OutputProfiles::new(storage.clone(), registered_output_adapters());
    let profile = output_profiles
        .activate("Generic", "iotkit.mqtt-json.v1", Map::new(), 7)
        .await
        .unwrap();

    let application = StorageWebApplication::new(storage);
    let principal = application
        .login("owner", "long enough owner password")
        .await
        .unwrap()
        .principal;
    let view = application
        .console(ConsoleRequest {
            path: "/output".into(),
            query: HashMap::new(),
            principal: principal.clone(),
        })
        .await
        .unwrap();

    assert_eq!(view.output_summary.sending_count, 1);
    assert_eq!(view.output_summary.needs_configuration_count, 0);
    assert_eq!(view.output_summary.delivery_problem_count, 0);
    assert_eq!(view.outputs.iter().filter(|item| item.active).count(), 1);

    let binding = &view
        .outputs
        .iter()
        .find(|item| item.active)
        .unwrap()
        .bindings[0];
    assert_eq!(binding.rule_name, "Temperature");
    assert_eq!(binding.state_label, "最初の値を待っています");
    assert!(binding.topic.starts_with("iotkit/v1/sources/"));
    assert!(binding.payload.contains("\"schema_version\""));
    assert!(!binding.signal_ref.is_empty());
    assert!(!binding.series_id.is_empty());

    output_profiles.stop(&profile.profile_id, 8).await.unwrap();
    let stopped = application
        .console(ConsoleRequest {
            path: "/output".into(),
            query: HashMap::new(),
            principal,
        })
        .await
        .unwrap();
    let generic: Vec<_> = stopped
        .outputs
        .iter()
        .filter(|item| item.adapter_id == "iotkit.mqtt-json.v1")
        .collect();
    assert_eq!(generic.len(), 1);
    assert!(!generic[0].active);
    assert!(generic[0].profile_id.is_empty());
}
