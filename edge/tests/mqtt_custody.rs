use iotkit_edge::{
    mqtt::ingest::IngestProcessor,
    storage::{EdgeNodeState, Storage, StorageError, StorageProfile},
};
use iotkit_edge_custody_contract::{ActivationRequest, ActivationResult};
use serde_json::Value;
use std::path::PathBuf;
use tempfile::TempDir;

async fn processor() -> (TempDir, Storage, IngestProcessor) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).expect("create test temp");
    let directory = TempDir::new_in(root).expect("temp directory");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("custody.db"),
    })
    .await
    .expect("open store");
    let processor = IngestProcessor::new(storage.clone());
    (directory, storage, processor)
}

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(path),
    )
    .expect("read fixture")
}

#[tokio::test]
async fn acknowledges_records_only_after_activation_and_commit() {
    let (_directory, storage, processor) = processor().await;
    let descriptor = fixture("testdata/egress/v2/descriptor-snapshot.json");
    assert!(
        processor
            .handle(
                "iotkit/v1/edge-nodes/edge-node-01/descriptors",
                &descriptor,
                1_720_000_000_000,
            )
            .await
            .expect("process descriptor")
            .is_none()
    );

    let records = fixture("testdata/egress/v1/record-batch.json");
    let error = processor
        .handle(
            "iotkit/v1/edge-nodes/edge-node-01/records",
            &records,
            1_720_000_000_050,
        )
        .await
        .expect_err("unregistered records must fail");
    assert!(matches!(
        error,
        iotkit_edge::mqtt::ingest::IngestError::Storage(StorageError::EdgeNodeNotActive)
    ));

    let command = storage
        .request_activation("edge-node-01", 1_720_000_000_100)
        .await
        .expect("request activation");
    let request =
        ActivationRequest::decode(&command.payload_json).expect("decode activation request");
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
    let result_payload = serde_json::to_vec(&result).expect("encode activation result");
    processor
        .handle(
            "iotkit/v1/edge-nodes/edge-node-01/activation/result",
            &result_payload,
            1_720_000_000_200,
        )
        .await
        .expect("process activation result");
    assert_eq!(
        storage
            .edge_node("edge-node-01")
            .await
            .expect("read Edge Node")
            .state,
        EdgeNodeState::Active
    );

    let ack = processor
        .handle(
            "iotkit/v1/edge-nodes/edge-node-01/records",
            &records,
            1_720_000_000_300,
        )
        .await
        .expect("process records")
        .expect("custody acknowledgement");
    assert_eq!(
        ack.topic,
        "iotkit/v1/edge-nodes/edge-node-01/accepted-through"
    );
    assert_eq!(ack.qos, 1);
    assert!(!ack.retain);
    assert_eq!(
        serde_json::from_slice::<Value>(&ack.payload).expect("decode actual ack"),
        serde_json::from_slice::<Value>(&fixture("testdata/egress/v1/accepted-through.json"))
            .expect("decode fixture ack"),
    );
    assert_eq!(
        storage
            .accepted_through("edge-node-01", "epoch-01")
            .await
            .expect("read committed cursor"),
        1
    );

    let mut next: Value = serde_json::from_slice(&records).expect("decode next batch");
    next["publication_id"] = "edge-node-01:epoch-01:2:2".into();
    next["cursor_start"] = 2.into();
    next["cursor_end"] = 2.into();
    next["records"][0]["pub_seq"] = 2.into();
    let next = serde_json::to_vec(&next).expect("encode next batch");
    processor
        .handle(
            "iotkit/v1/edge-nodes/edge-node-01/records",
            &next,
            1_720_000_000_400,
        )
        .await
        .expect("advance custody cursor")
        .expect("next custody acknowledgement");

    let stale_ack = processor
        .handle(
            "iotkit/v1/edge-nodes/edge-node-01/records",
            &records,
            1_720_000_000_500,
        )
        .await
        .expect("accept exact stale replay")
        .expect("stale replay acknowledgement");
    let stale_ack: Value =
        serde_json::from_slice(&stale_ack.payload).expect("decode stale replay acknowledgement");
    assert_eq!(stale_ack["accepted_through"], 1);
    assert_eq!(
        storage
            .accepted_through("edge-node-01", "epoch-01")
            .await
            .expect("read high-water cursor"),
        2
    );
}

#[tokio::test]
async fn rejects_topic_body_identity_mismatch_without_custody_ack() {
    let (_directory, _storage, processor) = processor().await;
    let error = processor
        .handle(
            "iotkit/v1/edge-nodes/edge-node-other/descriptors",
            &fixture("testdata/egress/v2/descriptor-snapshot.json"),
            1,
        )
        .await
        .expect_err("topic/body mismatch");
    assert!(matches!(
        error,
        iotkit_edge::mqtt::ingest::IngestError::Contract(_)
    ));
}
