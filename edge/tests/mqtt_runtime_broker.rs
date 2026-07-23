use iotkit_edge::{
    mqtt::ingest::{IngestProcessor, IngestRuntime, IngestRuntimeConfig, IngestTransport},
    storage::{EdgeNodeState, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::{ActivationRequest, ActivationResult};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use std::{path::PathBuf, time::Duration};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(path),
    )
    .expect("read fixture")
}

#[tokio::test]
#[ignore = "requires a real Mosquitto broker; run scripts/test-rust-edge-custody.sh"]
async fn actual_broker_delivers_descriptor_activation_records_and_ack() {
    let Some(port) = std::env::var("IOTKIT_TEST_MQTT_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
    else {
        return;
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).expect("create test temp");
    let directory = TempDir::new_in(root).expect("temp directory");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("broker.db"),
    })
    .await
    .expect("open storage");
    let cancellation = CancellationToken::new();
    let runtime = IngestRuntime::new(
        IngestRuntimeConfig {
            broker_host: "127.0.0.1".into(),
            broker_port: port,
            client_id: "iotkit-edge-runtime-test".into(),
            username: None,
            password: None,
            transport: IngestTransport::PlaintextForDevelopment,
        },
        IngestProcessor::new(storage.clone()),
    );
    let runtime_cancellation = cancellation.clone();
    let runtime_task = tokio::spawn(async move { runtime.run(runtime_cancellation).await });

    let options = MqttOptions::new("iotkit-edge-runtime-driver", "127.0.0.1", port);
    let (client, mut event_loop) = AsyncClient::new(options, 32);
    let (ack_sender, mut ack_receiver) = mpsc::channel(4);
    let (activation_sender, mut activation_receiver) = mpsc::channel(4);
    let driver_task = tokio::spawn(async move {
        loop {
            match event_loop.poll().await {
                Ok(Event::Incoming(Incoming::Publish(publication))) => {
                    if publication.topic.ends_with("/accepted-through")
                        && ack_sender.send(publication.payload.to_vec()).await.is_err()
                    {
                        return;
                    }
                    if publication.topic.ends_with("/activation/request")
                        && activation_sender
                            .send(publication.payload.to_vec())
                            .await
                            .is_err()
                    {
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
    });
    client
        .subscribe("iotkit/v1/edge-nodes/+/accepted-through", QoS::AtLeastOnce)
        .await
        .expect("subscribe ack");
    client
        .subscribe(
            "iotkit/v1/edge-nodes/+/activation/request",
            QoS::AtLeastOnce,
        )
        .await
        .expect("subscribe activation request");
    tokio::time::sleep(Duration::from_millis(200)).await;

    client
        .publish(
            "iotkit/v1/edge-nodes/edge-node-01/descriptors",
            QoS::AtLeastOnce,
            false,
            br#"{"malformed":true}"#.as_slice(),
        )
        .await
        .expect("publish rejected message");
    client
        .publish(
            "iotkit/v1/edge-nodes/edge-node-01/descriptors",
            QoS::AtLeastOnce,
            false,
            fixture("testdata/egress/v2/descriptor-snapshot.json"),
        )
        .await
        .expect("publish descriptor");
    wait_for_state(&storage, EdgeNodeState::Discovered).await;

    let command = storage
        .request_activation("edge-node-01", 1_720_000_000_100)
        .await
        .expect("request activation");
    let delivered_request =
        tokio::time::timeout(Duration::from_secs(5), activation_receiver.recv())
            .await
            .expect("activation request timeout")
            .expect("activation request channel");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&delivered_request)
            .expect("decode delivered activation request"),
        serde_json::from_slice::<serde_json::Value>(&command.payload_json)
            .expect("decode stored activation request")
    );
    let request =
        ActivationRequest::decode(&command.payload_json).expect("decode activation request");
    client
        .publish(
            "iotkit/v1/edge-nodes/edge-node-01/activation/result",
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&ActivationResult {
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
            .expect("encode result"),
        )
        .await
        .expect("publish activation result");
    wait_for_state(&storage, EdgeNodeState::Active).await;

    client
        .publish(
            "iotkit/v1/edge-nodes/edge-node-01/records",
            QoS::AtLeastOnce,
            false,
            fixture("testdata/egress/v1/record-batch.json"),
        )
        .await
        .expect("publish records");
    let ack = tokio::time::timeout(Duration::from_secs(5), ack_receiver.recv())
        .await
        .expect("ack timeout")
        .expect("ack channel");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&ack).expect("decode ack"),
        serde_json::from_slice::<serde_json::Value>(&fixture(
            "testdata/egress/v1/accepted-through.json"
        ))
        .expect("decode expected ack"),
    );

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(3), runtime_task)
        .await
        .expect("runtime shutdown timeout")
        .expect("runtime task")
        .expect("runtime result");
    driver_task.abort();
}

async fn wait_for_state(storage: &Storage, expected: EdgeNodeState) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if storage
                .edge_node("edge-node-01")
                .await
                .is_ok_and(|node| node.state == expected)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("Edge Node state timeout");
}
