use super::*;
use crate::{
    application::accounts::AccountService,
    auth::{
        password::Password,
        session::{SessionSecrets, SessionWindow},
    },
    diagnostics::storage_status,
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, RawRecord, StorageProfile},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;

const SERIES_KEY: &str = "018f0000-0000-7000-8000-000000000001:temperature:na:primary";

#[tokio::test]
async fn semantic_tick_keeps_account_and_session_work_available_with_backlog_remaining() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("edge.db"),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let descriptor = DescriptorSnapshot::decode(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "edge_node_id": "node",
            "ledger_epoch": "epoch",
            "descriptor_revision": 1,
            "complete": true,
            "devices": [{
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "identifier": "runtime-fairness-device",
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
        .unwrap(),
    )
    .unwrap();
    storage.apply_descriptor(&descriptor, 2).await.unwrap();
    let semantics = Semantics::new(storage.clone());
    semantics
        .create_rule(
            crate::application::semantics::SemanticRuleDraft {
                edge_node_id: "node".into(),
                series_key: SERIES_KEY.into(),
                display_name: "Runtime fairness temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            3,
        )
        .await
        .unwrap();
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "node".into(),
            ledger_epoch: "epoch".into(),
            publication_id: "runtime-fairness-backlog".into(),
            received_at: 4,
            records: (1_i64..=64)
                .map(|pub_seq| {
                    RawRecord::new(
                        pub_seq,
                        serde_json::to_vec(&serde_json::json!({
                            "family": "measurement",
                            "schema_version": 1,
                            "epoch": "epoch",
                            "pub_seq": pub_seq,
                            "series_key": SERIES_KEY,
                            "values": [pub_seq as f64],
                            "event_time": pub_seq,
                            "event_time_source": "received_at",
                            "time_source": "edge_node",
                            "time_quality": "unsynced",
                            "received_at": pub_seq,
                            "device_time": null
                        }))
                        .unwrap(),
                    )
                    .unwrap()
                })
                .collect(),
        })
        .await
        .unwrap();

    let cancellation = CancellationToken::new();
    let projection = tokio::spawn({
        let semantics = semantics.clone();
        let cancellation = cancellation.clone();
        async move {
            project_semantic_tick(
                &semantics,
                &cancellation,
                SEMANTIC_PROJECTION_ITEMS_PER_TICK,
                SEMANTIC_PROJECTION_TIME_BUDGET,
            )
            .await
        }
    });
    tokio::task::yield_now().await;
    let owner = tokio::time::timeout(
        Duration::from_secs(5),
        AccountService::new(storage.clone()).create_initial_system_admin(
            "owner",
            "System owner",
            Password::new("runtime fairness password").unwrap(),
            5,
        ),
    )
    .await
    .expect("account creation must not be starved")
    .unwrap();
    let secrets = SessionSecrets::generate().unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        storage.create_session(
            &owner.account_ref,
            owner.revision,
            secrets.session_ref().as_str(),
            secrets.token_digest(),
            secrets.csrf_digest(),
            SessionWindow::issued(6).unwrap(),
            6,
        ),
    )
    .await
    .expect("session creation must not be starved")
    .unwrap();
    projection.await.unwrap().unwrap();

    assert!(
        storage_status(&storage, 90)
            .await
            .unwrap()
            .pending_semantic_projection_count
            >= 48,
        "one tick must leave backlog for foreground storage work"
    );
}
