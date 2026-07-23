use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use clap::Parser;
use iotkit_edge::{
    application::{
        output_profiles::OutputProfiles,
        semantics::{SemanticRuleDraft, Semantics},
    },
    cli::{Cli, Command},
    composition::{
        registered_output_adapters,
        runtime::{RuntimeError, RuntimeFactory, run_runtime},
        runtime_config::RuntimeConfig,
    },
    lifecycle::ExitReason,
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{EdgeNodeState, Storage},
    web::{WebApplication, test_support::StubApplication},
};
use iotkit_edge_custody_contract::{ActivationRequest, ActivationResult};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde_json::Map;
use tempfile::TempDir;
use tokio::sync::{mpsc, oneshot};

fn protected_file(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write protected file");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("protect runtime file");
}

fn fixture(path: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(path),
    )
    .expect("read fixture")
}

struct CaptureFactory {
    storage: Mutex<Option<oneshot::Sender<Storage>>>,
}

impl RuntimeFactory for CaptureFactory {
    fn web_application(
        &self,
        storage: Storage,
        _storage_warning_percent: i32,
        _broker_certificate_file: Option<&Path>,
    ) -> Result<Arc<dyn WebApplication>, RuntimeError> {
        assert!(
            self.storage
                .lock()
                .expect("capture factory mutex")
                .take()
                .expect("capture sender")
                .send(storage)
                .is_ok(),
            "receive composed storage"
        );
        Ok(Arc::new(StubApplication::default()))
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a real Mosquitto broker; run scripts/test-rust-edge-runtime.sh"]
async fn composed_runtime_custodies_projects_serves_and_marks_output_puback() {
    let broker_port = std::env::var("IOTKIT_TEST_RUNTIME_MQTT_PORT")
        .expect("IOTKIT_TEST_RUNTIME_MQTT_PORT")
        .parse::<u16>()
        .expect("valid broker port");
    let directory = TempDir::new_in(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target")
            .join("tmp"),
    )
    .expect("runtime temp directory");
    let password = directory.path().join("mqtt-password");
    protected_file(&password, "runtime-test-password");
    let metadata = directory.path().join("storage-profile.json");
    let profile =
        std::env::var("IOTKIT_TEST_RUNTIME_STORAGE_PROFILE").unwrap_or_else(|_| "embedded".into());
    protected_file(&metadata, &format!(r#"{{"profile":"{profile}"}}"#));
    let postgres_config = directory.path().join("postgres.json");
    if profile == "postgres" {
        let dsn =
            std::env::var("IOTKIT_TEST_RUNTIME_POSTGRES_DSN").expect("runtime PostgreSQL DSN");
        protected_file(
            &postgres_config,
            &serde_json::json!({"dsn": dsn}).to_string(),
        );
    }
    let listen: SocketAddr = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve HTTP port");
        listener.local_addr().expect("HTTP address")
    };
    let database = directory.path().join("edge.db");
    let mut arguments = vec![
        "iotkit-edge".to_owned(),
        "serve".into(),
        "--storage-profile".into(),
        profile.clone(),
        "--db".into(),
        database.display().to_string(),
        "--storage-metadata".into(),
        metadata.display().to_string(),
    ];
    if profile == "postgres" {
        arguments.extend([
            "--postgres-config".into(),
            postgres_config.display().to_string(),
        ]);
    }
    arguments.extend([
        "--edge-id".into(),
        "edge-0123456789abcdef0123456789abcdef".into(),
        "--broker-url".into(),
        format!("tcp://127.0.0.1:{broker_port}"),
        "--client-id".into(),
        "edge-runtime-ingest".into(),
        "--username".into(),
        "edge-runtime".into(),
        "--password-file".into(),
        password.display().to_string(),
        "--allow-insecure".into(),
        "--http-listen".into(),
        listen.to_string(),
        "--public-origin".into(),
        format!("http://{listen}"),
        "--development-http".into(),
        "--storage-warning-percent".into(),
        "90".into(),
        "--output-broker-url".into(),
        format!("tcp://127.0.0.1:{broker_port}"),
        "--output-client-id".into(),
        "edge-runtime-output".into(),
        "--output-username".into(),
        "edge-output".into(),
        "--output-password-file".into(),
        password.display().to_string(),
        "--output-allow-insecure".into(),
    ]);
    let args = match Cli::try_parse_from(arguments)
        .expect("parse compose-equivalent serve flags")
        .command
        .expect("serve command")
    {
        Command::Serve(args) => args,
        other => panic!("unexpected command: {other:?}"),
    };
    let config = RuntimeConfig::from_serve_args(&args).expect("typed runtime config");

    let setup = Storage::connect(config.storage.clone())
        .await
        .expect("connect setup storage");
    setup
        .ensure_edge_identity("edge-0123456789abcdef0123456789abcdef", 1_720_000_000_000)
        .await
        .expect("initialize configured identity");
    let rule = Semantics::new(setup.clone())
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "edge-node-01".into(),
                series_key: "series-temperature-01".into(),
                display_name: "Runtime temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            1_720_000_000_001,
        )
        .await
        .expect("create semantic rule");
    OutputProfiles::new(setup.clone(), registered_output_adapters())
        .activate(
            "Runtime MQTT",
            "iotkit.mqtt-json.v1",
            Map::new(),
            1_720_000_000_002,
        )
        .await
        .expect("activate output profile");
    drop(setup);

    let (storage_tx, storage_rx) = oneshot::channel();
    let factory = Box::leak(Box::new(CaptureFactory {
        storage: Mutex::new(Some(storage_tx)),
    }));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runtime = tokio::spawn(async move {
        run_runtime(config, factory, async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    let storage = storage_rx.await.expect("composed storage");

    let options = MqttOptions::new("runtime-composition-driver", "127.0.0.1", broker_port);
    let (client, mut event_loop) = AsyncClient::new(options, 32);
    let (ack_tx, mut ack_rx) = mpsc::channel(4);
    let (activation_tx, mut activation_rx) = mpsc::channel(4);
    let (output_tx, mut output_rx) = mpsc::channel(4);
    let driver = tokio::spawn(async move {
        loop {
            match event_loop.poll().await {
                Ok(Event::Incoming(Incoming::Publish(publication))) => {
                    if publication.topic.ends_with("/accepted-through") {
                        let _ = ack_tx.send(publication.payload.to_vec()).await;
                    } else if publication.topic.ends_with("/activation/request") {
                        let _ = activation_tx.send(publication.payload.to_vec()).await;
                    } else if publication.topic.contains("/observations") {
                        let _ = output_tx.send(publication.payload.to_vec()).await;
                    }
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
    });
    for topic in [
        "iotkit/v1/edge-nodes/+/accepted-through",
        "iotkit/v1/edge-nodes/+/activation/request",
        "iotkit/v1/sources/+/signals/+/observations",
    ] {
        client
            .subscribe(topic, QoS::AtLeastOnce)
            .await
            .expect("subscribe integration driver");
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
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
    let request_payload = tokio::time::timeout(Duration::from_secs(5), activation_rx.recv())
        .await
        .expect("activation timeout")
        .expect("activation publication");
    let request = ActivationRequest::decode(&request_payload).expect("activation request");
    assert_eq!(request.activation_id, command.activation_id);
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
            .expect("encode activation result"),
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
        .expect("publish record");
    tokio::time::timeout(Duration::from_secs(10), ack_rx.recv())
        .await
        .expect("custody ACK timeout")
        .expect("custody ACK");
    tokio::time::timeout(Duration::from_secs(10), output_rx.recv())
        .await
        .expect("output publication timeout")
        .expect("output publication");
    assert_eq!(
        storage
            .raw_records("edge-node-01", "epoch-01")
            .await
            .expect("raw custody")
            .len(),
        1
    );
    assert_eq!(
        storage
            .semantic_observations(&rule.rule_id)
            .await
            .expect("semantic observations")
            .len(),
        1
    );
    wait_for_published(&storage).await;
    assert_http(listen).await;

    shutdown_tx.send(()).expect("request runtime shutdown");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), runtime)
            .await
            .expect("runtime shutdown timeout")
            .expect("runtime join")
            .expect("runtime result"),
        ExitReason::Requested
    );
    driver.abort();
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

async fn wait_for_published(storage: &Storage) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(storage.pending_output_count().await, Ok(0)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("output PUBACK mark timeout");
}

async fn assert_http(listen: SocketAddr) {
    let response = tokio::task::spawn_blocking(move || {
        let mut stream =
            TcpStream::connect_timeout(&listen, Duration::from_secs(1)).expect("connect HTTP");
        stream
            .write_all(b"GET /static/edge.css HTTP/1.1\r\nHost: edge\r\nConnection: close\r\n\r\n")
            .expect("write HTTP request");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("read HTTP response");
        response
    })
    .await
    .expect("HTTP check task");
    assert!(response.starts_with("HTTP/1.1 200 OK"));
}
