use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use iotkit_edge::{
    cli::{DeploymentProfileArg, ServeArgs, StorageArgs, StorageProfileArg},
    composition::runtime::{ProductionRuntimeFactory, RuntimeError, RuntimeFactory, run_runtime},
    composition::runtime_config::{MqttTransportConfig, RuntimeConfig},
    lifecycle::ExitReason,
    storage::{Storage, StorageError},
    web::{WebApplication, test_support::StubApplication},
};
use tempfile::TempDir;

fn secret(path: &Path, value: &str) {
    fs::write(path, value).expect("write secret");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("protect secret");
}

fn serve_args(directory: &TempDir) -> ServeArgs {
    let password = directory.path().join("mqtt-password");
    let ca = directory.path().join("ca.pem");
    secret(&password, "ingest-secret");
    fs::write(&ca, "ingest-ca-material").expect("write CA");
    ServeArgs {
        storage: StorageArgs {
            profile: StorageProfileArg::Embedded,
            database: directory.path().join("edge.db"),
            postgres_config: None,
            storage_metadata: None,
        },
        edge_id: "edge-0123456789abcdef0123456789abcdef".into(),
        broker_url: "ssl://broker.example:8883".into(),
        client_id: "edge-ingest".into(),
        username: "edge-user".into(),
        password_file: password,
        trust_mode: Some("bundle_only".into()),
        ca_file: Some(ca),
        allow_insecure: false,
        http_listen: "127.0.0.1:0".into(),
        public_origin: "https://edge.example".into(),
        development_http: false,
        deployment_profile: DeploymentProfileArg::Field,
        broker_certificate_file: None,
        storage_warning_percent: 90,
        recovery_control_socket: directory.path().join("recovery-control.sock"),
        output_broker_url: None,
        output_client_id: "edge-output".into(),
        output_username: None,
        output_password_file: None,
        output_trust_mode: None,
        output_ca_file: None,
        output_allow_insecure: false,
    }
}

#[test]
fn trial_profile_requires_loopback_development_http() {
    let directory = TempDir::new().unwrap();
    let mut args = serve_args(&directory);
    args.broker_url = "tcp://127.0.0.1:18883".into();
    args.allow_insecure = true;
    args.trust_mode = None;
    args.ca_file = None;
    args.development_http = true;
    args.public_origin = "http://127.0.0.1:8080".into();
    args.deployment_profile = DeploymentProfileArg::Trial;
    let config = RuntimeConfig::from_serve_args(&args).unwrap();
    assert!(config.trial_profile);

    args.public_origin = "http://192.0.2.1:8080".into();
    assert!(RuntimeConfig::from_serve_args(&args).is_err());
}

#[test]
fn typed_runtime_config_parses_tls_and_redacts_file_contents() {
    let directory = TempDir::new().unwrap();
    let mut args = serve_args(&directory);
    let certificate = directory.path().join("broker-certificate.pem");
    fs::write(&certificate, "certificate").unwrap();
    args.broker_certificate_file = Some(certificate.clone());
    args.storage_warning_percent = 73;
    let config = RuntimeConfig::from_serve_args(&args).expect("typed config");
    assert_eq!(config.ingest.host, "broker.example");
    assert_eq!(config.ingest.port, 8883);
    assert_eq!(config.storage_warning_percent, 73);
    assert_eq!(
        config.broker_certificate_file.as_deref(),
        Some(certificate.as_path())
    );
    assert!(matches!(
        config.ingest.transport,
        MqttTransportConfig::TlsBundle { .. }
    ));
    let debug = format!("{config:?}");
    assert!(!debug.contains("ingest-secret"));
    assert!(!debug.contains("ingest-ca-material"));
    assert!(debug.contains("[REDACTED]"));
}

#[test]
fn typed_runtime_config_rejects_scheme_trust_and_partial_output_conflicts() {
    let directory = TempDir::new().unwrap();
    let mut args = serve_args(&directory);
    args.edge_id = "edge-runtime".into();
    assert!(RuntimeConfig::from_serve_args(&args).is_err());

    let mut args = serve_args(&directory);
    args.broker_url = "tcp://broker.example:1883".into();
    assert!(RuntimeConfig::from_serve_args(&args).is_err());

    let mut args = serve_args(&directory);
    args.allow_insecure = true;
    assert!(RuntimeConfig::from_serve_args(&args).is_err());

    let mut args = serve_args(&directory);
    args.output_username = Some("output-user".into());
    args.output_password_file = Some(PathBuf::from("/does/not/matter"));
    assert!(RuntimeConfig::from_serve_args(&args).is_err());
}

#[test]
fn typed_runtime_config_rejects_every_non_loopback_http_listener() {
    let directory = TempDir::new().unwrap();
    for listener in [
        "0.0.0.0:8080",
        "192.168.1.20:8080",
        "[::]:8080",
        "[2001:db8::1]:8080",
    ] {
        let mut args = serve_args(&directory);
        args.http_listen = listener.into();
        assert!(
            RuntimeConfig::from_serve_args(&args).is_err(),
            "accepted non-loopback listener {listener}"
        );
    }
    for listener in ["127.0.0.1:8080", "[::1]:8080"] {
        let mut args = serve_args(&directory);
        args.http_listen = listener.into();
        assert!(
            RuntimeConfig::from_serve_args(&args).is_ok(),
            "rejected loopback listener {listener}"
        );
    }
}

#[tokio::test]
async fn production_runtime_composes_the_storage_backed_web_adapter() {
    let directory = TempDir::new().unwrap();
    let storage = Storage::connect(iotkit_edge::storage::StorageProfile::Sqlite {
        path: directory.path().join("web.db"),
    })
    .await
    .unwrap();

    assert!(
        ProductionRuntimeFactory
            .web_application(storage, 90, None)
            .is_ok()
    );
}

#[tokio::test]
async fn runtime_identity_is_stable_across_restart_and_rejects_reconfiguration() {
    let directory = TempDir::new().unwrap();
    for edge_id in [
        "edge-0123456789abcdef0123456789abcdef",
        "edge-0123456789abcdef0123456789abcdef",
        "edge-fedcba9876543210fedcba9876543210",
    ] {
        let mut args = serve_args(&directory);
        args.edge_id = edge_id.into();
        args.broker_url = "tcp://127.0.0.1:1883".into();
        args.trust_mode = None;
        args.ca_file = None;
        args.allow_insecure = true;
        args.development_http = true;
        args.public_origin = "http://127.0.0.1:8080".into();
        let result = run_runtime(
            RuntimeConfig::from_serve_args(&args).unwrap(),
            &ProductionRuntimeFactory,
            std::future::ready(()),
        )
        .await;
        if edge_id == "edge-fedcba9876543210fedcba9876543210" {
            assert!(matches!(
                result,
                Err(RuntimeError::Storage(StorageError::EdgeIdentityMismatch))
            ));
        } else {
            assert_eq!(result.unwrap(), ExitReason::Requested);
        }
    }
}

struct StubRuntimeFactory;

impl RuntimeFactory for StubRuntimeFactory {
    fn web_application(
        &self,
        _storage: Storage,
        _storage_warning_percent: i32,
        _broker_certificate_file: Option<&Path>,
    ) -> Result<Arc<dyn WebApplication>, RuntimeError> {
        Ok(Arc::new(StubApplication::default()))
    }
}

static STUB_RUNTIME_FACTORY: StubRuntimeFactory = StubRuntimeFactory;

#[tokio::test(flavor = "multi_thread")]
async fn injected_web_adapter_binds_http_and_signal_drains_runtime() {
    let directory = TempDir::new().unwrap();
    let listen: SocketAddr = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let mut args = serve_args(&directory);
    args.broker_url = "tcp://127.0.0.1:9".into();
    args.trust_mode = None;
    args.ca_file = None;
    args.allow_insecure = true;
    args.http_listen = listen.to_string();
    args.development_http = true;
    args.public_origin = format!("http://{listen}");
    let config = RuntimeConfig::from_serve_args(&args).unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let runtime = tokio::spawn(async move {
        run_runtime(config, &STUB_RUNTIME_FACTORY, async {
            let _ = shutdown_rx.await;
        })
        .await
    });

    let response = tokio::task::spawn_blocking(move || {
        for _ in 0..100 {
            match TcpStream::connect_timeout(&listen, Duration::from_millis(50)) {
                Ok(mut stream) => {
                    stream
                        .write_all(
                            b"GET /static/edge.css HTTP/1.1\r\nHost: edge\r\nConnection: close\r\n\r\n",
                        )
                        .unwrap();
                    let mut response = String::new();
                    stream.read_to_string(&mut response).unwrap();
                    return response;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        panic!("HTTP listener did not bind");
    })
    .await
    .unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"));

    shutdown_tx.send(()).unwrap();
    assert_eq!(
        runtime.await.unwrap().unwrap(),
        ExitReason::Requested,
        "signal shutdown must not look like a critical failure"
    );
}
