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
    storage::{AcceptBatch, AccountProvision, AuditActor, RawRecord, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use serde_json::Map;

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
