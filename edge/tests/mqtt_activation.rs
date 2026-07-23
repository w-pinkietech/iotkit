use iotkit_edge::storage::{
    AcceptBatch, EdgeNodeState, RawRecord, Storage, StorageError, StorageProfile,
};
use iotkit_edge_custody_contract::{
    ActivationRequest, ActivationResult, DescriptorSnapshot, RecordBatch,
};
use std::path::PathBuf;
use tempfile::TempDir;

async fn store() -> (TempDir, Storage) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).expect("create test temp");
    let directory = TempDir::new_in(root).expect("temp directory");
    let store = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("activation.db"),
    })
    .await
    .expect("open store");
    (directory, store)
}

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(path),
    )
    .expect("read fixture")
}

fn storage_batch(batch: &RecordBatch) -> AcceptBatch {
    AcceptBatch {
        edge_node_id: batch.edge_node_id.clone(),
        ledger_epoch: batch.ledger_epoch.clone(),
        publication_id: batch.publication_id.clone(),
        received_at: 1_720_000_001_000,
        records: batch
            .records
            .iter()
            .enumerate()
            .map(|(index, record)| {
                RawRecord::new(batch.cursor_start + index as i64, record.get())
                    .expect("valid raw record")
            })
            .collect(),
    }
}

#[tokio::test]
async fn discovery_requires_activation_before_raw_custody() {
    let (_directory, store) = store().await;
    let descriptor =
        DescriptorSnapshot::decode(&fixture("testdata/egress/v2/descriptor-snapshot.json"))
            .expect("descriptor fixture");
    let discovered = store
        .apply_descriptor(&descriptor, 1_720_000_000_000)
        .await
        .expect("apply descriptor");
    assert!(discovered.applied);
    assert_eq!(discovered.edge_node.state, EdgeNodeState::Discovered);

    let wire_batch =
        RecordBatch::decode(&fixture("testdata/egress/v1/record-batch.json")).expect("batch");
    assert!(matches!(
        store
            .accept_active_batch(storage_batch(&wire_batch))
            .await
            .expect_err("unregistered node must not enter custody"),
        StorageError::EdgeNodeNotActive
    ));

    let command = store
        .request_activation(&descriptor.edge_node_id, 1_720_000_000_100)
        .await
        .expect("request activation");
    assert_eq!(
        command.topic,
        "iotkit/v1/edge-nodes/edge-node-01/activation/request"
    );
    assert_eq!(
        store
            .pending_activation_commands(10)
            .await
            .expect("pending activation commands")
            .len(),
        1
    );
    store
        .mark_activation_attempt(&command.activation_id, 1_720_000_000_150)
        .await
        .expect("record activation attempt");
    assert_eq!(
        store
            .pending_activation_commands(10)
            .await
            .expect("pending activation command")[0]
            .attempts,
        1
    );
    let request =
        ActivationRequest::decode(&command.payload_json).expect("decode generated request");
    let result = ActivationResult {
        schema_version: 1,
        activation_id: request.activation_id,
        edge_id: request.edge_id,
        edge_node_id: request.edge_node_id,
        ledger_epoch: request.expected_ledger_epoch,
        status: "applied".into(),
        discard_through_reading_seq: 12,
        first_publication_seq: 1,
        applied_at: 1_720_000_000_200,
    };
    let active = store
        .apply_activation_result(&result, 1_720_000_000_200)
        .await
        .expect("apply activation result");
    assert_eq!(active.state, EdgeNodeState::Active);
    assert!(
        store
            .pending_activation_commands(10)
            .await
            .expect("completed activation command")
            .is_empty()
    );
    assert_eq!(
        store
            .apply_activation_result(&result, 1_720_000_000_250)
            .await
            .expect("exact duplicate activation result")
            .state,
        EdgeNodeState::Active
    );

    let accepted = store
        .accept_active_batch(storage_batch(&wire_batch))
        .await
        .expect("active node enters custody");
    assert_eq!(accepted.accepted_through, 1);

    let mut conflicting = result;
    conflicting.edge_id = "edge-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
    assert!(matches!(
        store
            .apply_activation_result(&conflicting, 1_720_000_000_400)
            .await
            .expect_err("conflicting result must fail closed"),
        StorageError::ActivationConflict
    ));
    assert_eq!(
        store
            .edge_node("edge-node-01")
            .await
            .expect("read recovery hold")
            .state,
        EdgeNodeState::RecoveryHold
    );

    let mut next_epoch = descriptor;
    next_epoch.ledger_epoch = "epoch-02".into();
    next_epoch.descriptor_revision = 1;
    store
        .apply_descriptor(&next_epoch, 1_720_000_000_500)
        .await
        .expect("new epoch descriptor is accepted for inventory");
    assert_eq!(
        store
            .edge_node("edge-node-01")
            .await
            .expect("read new epoch fence")
            .state,
        EdgeNodeState::RecoveryHold
    );
}
