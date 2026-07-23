use std::{
    future::Future,
    path::Path,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rumqttc::{MqttOptions, Transport};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::{
    application::semantics::Semantics,
    lifecycle::{CriticalTaskError, ExitReason, Supervisor},
    mqtt::{
        ingest::{IngestProcessor, IngestRuntime, IngestRuntimeConfig, IngestTransport},
        output::{OutputRuntime, OutputRuntimeConfig},
    },
    storage::{Storage, StorageError},
    web::{WebApplication, WebConfig, router},
};

use super::{
    StorageWebApplication, registered_output_adapters,
    runtime_config::{MqttConnectionConfig, MqttTransportConfig, RuntimeConfig},
};

pub trait RuntimeFactory: Send + Sync {
    fn web_application(
        &self,
        storage: Storage,
        storage_warning_percent: i32,
        broker_certificate_file: Option<&Path>,
    ) -> Result<Arc<dyn WebApplication>, RuntimeError>;
}

pub struct ProductionRuntimeFactory;

impl RuntimeFactory for ProductionRuntimeFactory {
    fn web_application(
        &self,
        storage: Storage,
        storage_warning_percent: i32,
        broker_certificate_file: Option<&Path>,
    ) -> Result<Arc<dyn WebApplication>, RuntimeError> {
        Ok(Arc::new(StorageWebApplication::with_runtime_settings(
            storage,
            storage_warning_percent,
            broker_certificate_file.map(Path::to_path_buf),
        )))
    }
}

pub async fn run_runtime<F>(
    config: RuntimeConfig,
    factory: &dyn RuntimeFactory,
    shutdown: F,
) -> Result<ExitReason, RuntimeError>
where
    F: Future<Output = ()>,
{
    let storage = Storage::connect(config.storage).await?;
    storage
        .ensure_edge_identity(&config.edge_id, unix_millis()?)
        .await?;
    let web_application = factory.web_application(
        storage.clone(),
        config.storage_warning_percent,
        config.broker_certificate_file.as_deref(),
    )?;
    let listener = TcpListener::bind(config.http_listen)
        .await
        .map_err(RuntimeError::HttpBind)?;
    let ingest = IngestRuntime::new(
        ingest_config(config.ingest)?,
        IngestProcessor::new(storage.clone()),
    );
    let output = config
        .output
        .map(output_config)
        .transpose()?
        .map(|output| OutputRuntime::new(storage.clone(), output));
    let app = router(
        WebConfig {
            public_origin: config.public_origin,
            secure_cookies: config.secure_cookies,
        },
        web_application,
    );
    let cancellation = CancellationToken::new();
    let mut supervisor = Supervisor::with_token(cancellation.clone(), Duration::from_secs(10));

    supervisor.spawn("mqtt-ingest", {
        let cancellation = cancellation.clone();
        async move {
            ingest.run(cancellation).await.map_err(|error| {
                tracing::error!(%error, "critical MQTT ingest task failed");
                CriticalTaskError::new("mqtt-ingest")
            })
        }
    });

    let semantics = Semantics::new(storage.clone());
    supervisor.spawn("semantic-projection", {
        let cancellation = cancellation.clone();
        async move { run_semantic_projection(semantics, cancellation).await }
    });

    if let Some(output) = output {
        supervisor.spawn("mqtt-output", {
            let cancellation = cancellation.clone();
            async move {
                output.run(cancellation).await.map_err(|error| {
                    tracing::error!(%error, "critical MQTT output task failed");
                    CriticalTaskError::new("mqtt-output")
                })
            }
        });
    }

    supervisor.spawn("http", {
        let cancellation = cancellation.clone();
        async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(cancellation.cancelled_owned())
                .await
                .map_err(|error| {
                    tracing::error!(%error, "critical HTTP task failed");
                    CriticalTaskError::new("http")
                })
        }
    });

    Ok(supervisor.run_until(shutdown).await)
}

async fn run_semantic_projection(
    semantics: Semantics,
    cancellation: CancellationToken,
) -> Result<(), CriticalTaskError> {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            _ = interval.tick() => {
                semantics
                    .project_pending(256, registered_output_adapters())
                    .await
                    .map_err(|error| {
                        tracing::error!(%error, "critical semantic projection task failed");
                        CriticalTaskError::new("semantic-projection")
                    })?;
            }
        }
    }
}

fn ingest_config(config: MqttConnectionConfig) -> Result<IngestRuntimeConfig, RuntimeError> {
    if !matches!(
        &config.transport,
        MqttTransportConfig::PlaintextForDevelopment
    ) {
        crate::mqtt::ingest::install_crypto_provider()
            .map_err(|error| RuntimeError::MqttConfiguration(error.to_string()))?;
    }
    Ok(IngestRuntimeConfig {
        broker_host: config.host,
        broker_port: config.port,
        client_id: config.client_id,
        username: Some(config.username),
        password: Some(config.password),
        transport: match config.transport {
            MqttTransportConfig::TlsSystemRoots => IngestTransport::TlsSystemRoots,
            MqttTransportConfig::TlsBundle { ca_pem } => IngestTransport::TlsBundle { ca_pem },
            MqttTransportConfig::PlaintextForDevelopment => {
                IngestTransport::PlaintextForDevelopment
            }
        },
    })
}

fn output_config(config: MqttConnectionConfig) -> Result<OutputRuntimeConfig, RuntimeError> {
    let mut mqtt = MqttOptions::new(config.client_id, config.host, config.port);
    mqtt.set_keep_alive(Duration::from_secs(15));
    mqtt.set_clean_session(false);
    mqtt.set_credentials(config.username, config.password);
    match config.transport {
        MqttTransportConfig::TlsSystemRoots => {
            crate::mqtt::ingest::install_crypto_provider()
                .map_err(|error| RuntimeError::MqttConfiguration(error.to_string()))?;
            mqtt.set_transport(Transport::tls_with_default_config());
        }
        MqttTransportConfig::TlsBundle { ca_pem } => {
            crate::mqtt::ingest::install_crypto_provider()
                .map_err(|error| RuntimeError::MqttConfiguration(error.to_string()))?;
            mqtt.set_transport(Transport::tls(ca_pem, None, None));
        }
        MqttTransportConfig::PlaintextForDevelopment => {}
    }
    Ok(OutputRuntimeConfig {
        mqtt,
        request_capacity: 64,
        claim_lease: Duration::from_secs(30),
        idle_poll: Duration::from_millis(100),
        reconnect_delay: Duration::from_secs(1),
    })
}

pub type ShutdownSignal = Pin<Box<dyn Future<Output = ()> + Send>>;

pub fn shutdown_signal() -> Result<ShutdownSignal, RuntimeError> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut interrupt = signal(SignalKind::interrupt()).map_err(RuntimeError::Signal)?;
        let mut terminate = signal(SignalKind::terminate()).map_err(RuntimeError::Signal)?;
        Ok(Box::pin(async move {
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
        }))
    }
    #[cfg(not(unix))]
    {
        Ok(Box::pin(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::error!(%error, "shutdown signal handler failed");
            }
        }))
    }
}

fn unix_millis() -> Result<i64, RuntimeError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RuntimeError::Clock)?
        .as_millis();
    i64::try_from(millis).map_err(|_| RuntimeError::Clock)
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("storage startup failed: {0}")]
    Storage(#[from] StorageError),
    #[error("bind HTTP listener: {0}")]
    HttpBind(std::io::Error),
    #[error("install shutdown signal handler: {0}")]
    Signal(std::io::Error),
    #[error("system clock is before the Unix epoch or out of range")]
    Clock,
    #[error("MQTT runtime configuration failed: {0}")]
    MqttConfiguration(String),
}
