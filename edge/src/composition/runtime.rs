use std::{
    future::Future,
    path::Path,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rumqttc::{MqttOptions, Transport};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::{
    application::recovery::RecoveryService,
    application::semantics::Semantics,
    lifecycle::{CriticalTaskError, ExitReason, Supervisor},
    mqtt::{
        ingest::{
            IngestHealth, IngestProcessor, IngestRuntime, IngestRuntimeConfig, IngestTransport,
        },
        output::{OutputRuntime, OutputRuntimeConfig},
    },
    recovery_control::run_recovery_control,
    storage::{Storage, StorageError},
    web::{WebApplication, WebConfig, router},
};

use super::{
    StorageWebApplication, registered_output_adapters,
    runtime_config::{MqttConnectionConfig, MqttTransportConfig, RuntimeConfig},
};

const SEMANTIC_PROJECTION_ITEMS_PER_TICK: usize = 16;
const SEMANTIC_PROJECTION_TIME_BUDGET: Duration = Duration::from_millis(20);

pub trait RuntimeFactory: Send + Sync {
    fn web_application(
        &self,
        storage: Storage,
        storage_warning_percent: i32,
        broker_certificate_file: Option<&Path>,
    ) -> Result<Arc<dyn WebApplication>, RuntimeError>;

    fn web_application_with_ingest_health(
        &self,
        storage: Storage,
        storage_warning_percent: i32,
        broker_certificate_file: Option<&Path>,
        _ingest_health: IngestHealth,
    ) -> Result<Arc<dyn WebApplication>, RuntimeError> {
        self.web_application(storage, storage_warning_percent, broker_certificate_file)
    }
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

    fn web_application_with_ingest_health(
        &self,
        storage: Storage,
        storage_warning_percent: i32,
        broker_certificate_file: Option<&Path>,
        ingest_health: IngestHealth,
    ) -> Result<Arc<dyn WebApplication>, RuntimeError> {
        Ok(Arc::new(
            StorageWebApplication::with_runtime_settings_and_ingest_health(
                storage,
                storage_warning_percent,
                broker_certificate_file.map(Path::to_path_buf),
                ingest_health,
            ),
        ))
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
    let ingest_health = IngestHealth::default();
    let web_application = factory.web_application_with_ingest_health(
        storage.clone(),
        config.storage_warning_percent,
        config.broker_certificate_file.as_deref(),
        ingest_health.clone(),
    )?;
    let listener = TcpListener::bind(config.http_listen)
        .await
        .map_err(RuntimeError::HttpBind)?;
    let ingest = IngestRuntime::with_health(
        ingest_config(config.ingest)?,
        IngestProcessor::new(storage.clone()),
        ingest_health,
    );
    let output = config
        .output
        .map(output_config)
        .transpose()?
        .map(|output| OutputRuntime::new(storage.clone(), output));
    let app = router(
        WebConfig {
            public_origin: config.public_origin,
            display_time_zone: config.display_time_zone,
            secure_cookies: config.secure_cookies,
            trial_profile: config.trial_profile,
        },
        web_application,
    );
    let cancellation = CancellationToken::new();
    let mut supervisor = Supervisor::with_token(cancellation.clone(), Duration::from_secs(10));

    supervisor.spawn("recovery-control", {
        let cancellation = cancellation.clone();
        let socket_path = config.recovery_control_socket;
        let service = RecoveryService::new(storage.clone());
        async move {
            run_recovery_control(socket_path, service, cancellation)
                .await
                .map_err(|error| {
                    tracing::error!(%error, "critical recovery control task failed");
                    CriticalTaskError::new("recovery-control")
                })
        }
    });

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
                if !project_semantic_tick(
                    &semantics,
                    &cancellation,
                    SEMANTIC_PROJECTION_ITEMS_PER_TICK,
                    SEMANTIC_PROJECTION_TIME_BUDGET,
                )
                    .await
                    .map_err(|error| {
                        tracing::error!(%error, "critical semantic projection task failed");
                        CriticalTaskError::new("semantic-projection")
                    })?
                {
                    return Ok(());
                }
            }
        }
    }
}

async fn project_semantic_tick(
    semantics: &Semantics,
    cancellation: &CancellationToken,
    item_budget: usize,
    time_budget: Duration,
) -> Result<bool, StorageError> {
    let started = Instant::now();
    for _ in 0..item_budget {
        if started.elapsed() >= time_budget {
            break;
        }
        let progress = tokio::select! {
            () = cancellation.cancelled() => return Ok(false),
            result = semantics.project_pending(1, registered_output_adapters()) => result?,
        };
        if progress == Default::default() {
            break;
        }
        tokio::select! {
            () = cancellation.cancelled() => return Ok(false),
            () = tokio::task::yield_now() => {}
        }
    }
    Ok(true)
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

#[cfg(test)]
#[path = "../../tests/unit/composition_runtime_tests.rs"]
mod tests;
