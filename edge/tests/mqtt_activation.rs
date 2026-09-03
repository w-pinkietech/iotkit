use iotkit_edge::{
    mqtt::ingest::{IngestError, IngestProcessor},
    storage::{AcceptBatch, EdgeNodeState, RawRecord, Storage, StorageError, StorageProfile},
};
use iotkit_edge_custody_contract::{
    ActivationRequest, ActivationResult, DescriptorSnapshot, RecordBatch,
};
use serde_json::Value;
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

fn invalid_descriptor_case(name: &str) -> Vec<u8> {
    let corpus: Value = serde_json::from_slice(&fixture(
        "testdata/egress/v2/descriptor-conformance-cases.json",
    ))
    .expect("decode shared descriptor conformance corpus");
    let case = corpus["cases"]
        .as_array()
        .expect("descriptor conformance cases")
        .iter()
        .find(|case| case["name"] == name)
        .expect("named descriptor conformance case");
    assert_eq!(case["valid"], false, "case must be invalid");
    serde_json::to_vec(&case["descriptor"]).expect("encode descriptor conformance case")
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

#[tokio::test]
async fn unconfigured_inactive_node_stores_no_raw_records_and_emits_no_acknowledgement() {
    let (_directory, store) = store().await;
    let processor = IngestProcessor::new(store.clone());
    let descriptor = fixture("testdata/egress/v2/descriptor-snapshot.json");
    assert!(
        processor
            .handle(
                "iotkit/v1/edge-nodes/edge-node-01/descriptors",
                &descriptor,
                1_720_000_000_000,
            )
            .await
            .expect("apply descriptor")
            .is_none(),
        "descriptors do not produce custody acknowledgements"
    );

    let error = processor
        .handle(
            "iotkit/v1/edge-nodes/edge-node-01/records",
            &fixture("testdata/egress/v1/record-batch.json"),
            1_720_000_000_050,
        )
        .await
        .expect_err("inactive records must not produce an acknowledgement");

    assert!(matches!(
        error,
        IngestError::Storage(StorageError::EdgeNodeNotActive)
    ));
    assert!(
        store
            .raw_records("edge-node-01", "epoch-01")
            .await
            .expect("read raw records")
            .is_empty()
    );
    assert_eq!(
        store
            .accepted_through("edge-node-01", "epoch-01")
            .await
            .expect("read custody cursor"),
        0
    );
}

#[tokio::test]
async fn invalid_descriptor_leaves_descriptor_inventory_and_activation_unchanged() {
    let (_directory, store) = store().await;
    let processor = IngestProcessor::new(store.clone());
    let topic = "iotkit/v1/edge-nodes/edge-node-01/descriptors";
    processor
        .handle(
            topic,
            &fixture("testdata/egress/v2/descriptor-snapshot.json"),
            1_720_000_000_000,
        )
        .await
        .expect("apply valid descriptor");

    let command = store
        .request_activation("edge-node-01", 1_720_000_000_100)
        .await
        .expect("request activation");
    let request =
        ActivationRequest::decode(&command.payload_json).expect("decode activation request");
    processor
        .handle(
            "iotkit/v1/edge-nodes/edge-node-01/activation/result",
            &serde_json::to_vec(&ActivationResult {
                schema_version: 1,
                activation_id: request.activation_id,
                edge_id: request.edge_id,
                edge_node_id: request.edge_node_id,
                ledger_epoch: request.expected_ledger_epoch,
                status: "applied".into(),
                discard_through_reading_seq: 12,
                first_publication_seq: 1,
                applied_at: 1_720_000_000_200,
            })
            .expect("encode activation result"),
            1_720_000_000_200,
        )
        .await
        .expect("activate Edge Node");

    processor
        .handle(
            "iotkit/v1/edge-nodes/edge-node-01/records",
            &fixture("testdata/egress/v1/record-batch.json"),
            1_720_000_000_250,
        )
        .await
        .expect("accept custody record")
        .expect("emit custody acknowledgement");

    let node_before = store
        .edge_node("edge-node-01")
        .await
        .expect("read Edge Node");
    let descriptor_devices_before = store
        .list_descriptor_devices()
        .await
        .expect("read descriptor devices");
    let descriptor_signals_before = store
        .list_descriptor_signals()
        .await
        .expect("read descriptor signals");
    let inventory_devices_before = store
        .inventory_devices()
        .await
        .expect("read inventory devices");
    let inventory_signals_before = store
        .inventory_signals()
        .await
        .expect("read inventory signals");
    let commands_before = store
        .pending_activation_commands(10)
        .await
        .expect("read activation commands");
    let raw_records_before = store
        .raw_records("edge-node-01", "epoch-01")
        .await
        .expect("read raw records");
    let accepted_through_before = store
        .accepted_through("edge-node-01", "epoch-01")
        .await
        .expect("read custody cursor");
    assert_eq!(accepted_through_before, 1, "establish nonzero custody");

    let error = processor
        .handle(
            topic,
            &invalid_descriptor_case("measurement_key_invalid_segment"),
            1_720_000_000_300,
        )
        .await
        .expect_err("invalid descriptor must not reach storage");
    assert!(matches!(error, IngestError::Contract(_)));

    assert_eq!(
        store
            .edge_node("edge-node-01")
            .await
            .expect("read Edge Node"),
        node_before
    );
    assert_eq!(
        store
            .list_descriptor_devices()
            .await
            .expect("read descriptor devices"),
        descriptor_devices_before
    );
    assert_eq!(
        store
            .list_descriptor_signals()
            .await
            .expect("read descriptor signals"),
        descriptor_signals_before
    );
    assert_eq!(
        store
            .inventory_devices()
            .await
            .expect("read inventory devices"),
        inventory_devices_before
    );
    assert_eq!(
        store
            .inventory_signals()
            .await
            .expect("read inventory signals"),
        inventory_signals_before
    );
    assert_eq!(
        store
            .pending_activation_commands(10)
            .await
            .expect("read activation commands"),
        commands_before
    );
    assert_eq!(
        store
            .raw_records("edge-node-01", "epoch-01")
            .await
            .expect("read raw records"),
        raw_records_before
    );
    assert_eq!(
        store
            .accepted_through("edge-node-01", "epoch-01")
            .await
            .expect("read custody cursor"),
        accepted_through_before
    );
}
