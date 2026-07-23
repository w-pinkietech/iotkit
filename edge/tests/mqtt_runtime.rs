use iotkit_edge::{
    mqtt::ingest::{
        IngestProcessor, IngestRuntime, IngestRuntimeConfig, IngestTransport, RuntimeError,
    },
    storage::{Storage, StorageProfile},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

async fn runtime(config: IngestRuntimeConfig) -> (TempDir, IngestRuntime) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).expect("create test temp");
    let directory = TempDir::new_in(root).expect("temp directory");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("runtime.db"),
    })
    .await
    .expect("open store");
    (
        directory,
        IngestRuntime::new(config, IngestProcessor::new(storage)),
    )
}

#[tokio::test]
async fn empty_tls_bundle_is_rejected() {
    let (_directory, runtime) = runtime(IngestRuntimeConfig {
        broker_host: "127.0.0.1".into(),
        broker_port: 1883,
        client_id: "iotkit-edge-test".into(),
        username: None,
        password: None,
        transport: IngestTransport::TlsBundle { ca_pem: vec![] },
    })
    .await;
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        runtime
            .run(cancellation)
            .await
            .expect_err("empty TLS bundle must be rejected before connecting"),
        RuntimeError::Config(_)
    ));
}

#[tokio::test]
async fn system_root_tls_is_a_valid_production_transport() {
    let (_directory, runtime) = runtime(IngestRuntimeConfig {
        broker_host: "broker.example".into(),
        broker_port: 8883,
        client_id: "iotkit-edge-test".into(),
        username: Some("iotkit".into()),
        password: Some("not-used-because-cancelled".into()),
        transport: IngestTransport::TlsSystemRoots,
    })
    .await;
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    runtime
        .run(cancellation)
        .await
        .expect("system-root TLS configuration");
}

#[test]
fn runtime_debug_output_does_not_expose_credentials() {
    let config = IngestRuntimeConfig {
        broker_host: "broker.example".into(),
        broker_port: 8883,
        client_id: "iotkit-edge-test".into(),
        username: Some("iotkit".into()),
        password: Some("secret-password".into()),
        transport: IngestTransport::PlaintextForDevelopment,
    };
    let debug = format!("{config:?}");
    assert!(!debug.contains("secret-password"));
    assert!(debug.contains("[REDACTED]"));
}

#[tokio::test]
async fn a_full_client_queue_cannot_block_shutdown_or_outbox_progress() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).expect("create test temp");
    let directory = TempDir::new_in(root).expect("temp directory");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("saturated-runtime.db"),
    })
    .await
    .expect("open store");
    let fixture = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("testdata/egress/v2/descriptor-snapshot.json"),
    )
    .expect("read descriptor fixture");

    for index in 0..70 {
        let mut value: serde_json::Value =
            serde_json::from_slice(&fixture).expect("decode descriptor fixture");
        let edge_node_id = format!("edge-node-{index:03}");
        value["edge_node_id"] = edge_node_id.clone().into();
        let descriptor =
            DescriptorSnapshot::decode(&serde_json::to_vec(&value).expect("encode descriptor"))
                .expect("decode changed descriptor");
        storage
            .apply_descriptor(&descriptor, 1_720_000_000_000 + index)
            .await
            .expect("apply descriptor");
        storage
            .request_activation(&edge_node_id, 1_720_000_001_000 + index)
            .await
            .expect("queue activation");
    }

    let runtime = IngestRuntime::new(
        IngestRuntimeConfig {
            broker_host: "127.0.0.1".into(),
            broker_port: 9,
            client_id: "iotkit-edge-saturated-runtime".into(),
            username: None,
            password: None,
            transport: IngestTransport::PlaintextForDevelopment,
        },
        IngestProcessor::new(storage.clone()),
    );
    let cancellation = CancellationToken::new();
    let runtime_cancellation = cancellation.clone();
    let task = tokio::spawn(async move { runtime.run(runtime_cancellation).await });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        storage
            .pending_activation_commands(100)
            .await
            .expect("read pending activation commands")
            .iter()
            .all(|command| command.attempts == 0),
        "activation must not fill the MQTT queue before a subscribed connection exists"
    );
    cancellation.cancel();

    tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("runtime deadlocked after filling its MQTT request queue")
        .expect("runtime task")
        .expect("runtime shutdown");
}
