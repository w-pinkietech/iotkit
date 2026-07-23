use std::time::Duration;

use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS, Transport};
use tokio_util::sync::CancellationToken;

use super::IngestProcessor;

const SUBSCRIPTIONS: [&str; 3] = [
    "iotkit/v1/edge-nodes/+/records",
    "iotkit/v1/edge-nodes/+/descriptors",
    "iotkit/v1/edge-nodes/+/activation/result",
];

#[derive(Clone)]
pub struct IngestRuntimeConfig {
    pub broker_host: String,
    pub broker_port: u16,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub transport: IngestTransport,
}

#[derive(Clone)]
pub enum IngestTransport {
    TlsSystemRoots,
    TlsBundle { ca_pem: Vec<u8> },
    PlaintextForDevelopment,
}

impl std::fmt::Debug for IngestTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TlsSystemRoots => formatter.write_str("TlsSystemRoots"),
            Self::TlsBundle { .. } => formatter.write_str("TlsBundle"),
            Self::PlaintextForDevelopment => formatter.write_str("PlaintextForDevelopment"),
        }
    }
}

impl std::fmt::Debug for IngestRuntimeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IngestRuntimeConfig")
            .field("broker_host", &self.broker_host)
            .field("broker_port", &self.broker_port)
            .field("client_id", &self.client_id)
            .field("username", &self.username.as_ref().map(|_| "[REDACTED]"))
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("transport", &self.transport)
            .finish()
    }
}

pub struct IngestRuntime {
    config: IngestRuntimeConfig,
    processor: IngestProcessor,
}

impl IngestRuntime {
    #[must_use]
    pub fn new(config: IngestRuntimeConfig, processor: IngestProcessor) -> Self {
        Self { config, processor }
    }

    pub async fn run(self, cancellation: CancellationToken) -> Result<(), RuntimeError> {
        self.validate_config()?;
        let mut options = MqttOptions::new(
            &self.config.client_id,
            &self.config.broker_host,
            self.config.broker_port,
        );
        options.set_keep_alive(Duration::from_secs(15));
        options.set_clean_session(false);
        match &self.config.transport {
            IngestTransport::TlsSystemRoots => {
                install_crypto_provider()?;
                options.set_transport(Transport::tls_with_default_config());
            }
            IngestTransport::TlsBundle { ca_pem } => {
                install_crypto_provider()?;
                options.set_transport(Transport::tls(ca_pem.clone(), None, None));
            }
            IngestTransport::PlaintextForDevelopment => {}
        }
        if let Some(username) = &self.config.username {
            options.set_credentials(
                username,
                self.config.password.as_deref().unwrap_or_default(),
            );
        }
        let (client, mut event_loop) = AsyncClient::new(options, 64);
        let storage = self.processor.storage();
        let mut connected = false;
        let mut subscribed = false;
        let mut convergence = tokio::time::interval(Duration::from_millis(250));
        convergence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                () = cancellation.cancelled() => {
                    let _ = client.try_disconnect();
                    return Ok(());
                }
                event = event_loop.poll() => {
                    match event {
                        Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                            connected = true;
                            subscribed = try_subscribe(&client);
                        }
                        Ok(Event::Incoming(Incoming::Publish(publication))) => {
                            match self.processor.handle(
                                &publication.topic,
                                &publication.payload,
                                unix_millis(),
                            ).await {
                                Ok(Some(ack)) => {
                                    if let Err(error) = client.try_publish(
                                        ack.topic,
                                        QoS::AtLeastOnce,
                                        ack.retain,
                                        ack.payload,
                                    ) {
                                        tracing::warn!(
                                            %error,
                                            "custody acknowledgement enqueue deferred until replay"
                                        );
                                    }
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    tracing::warn!(
                                        topic = %publication.topic,
                                        %error,
                                        "MQTT message rejected"
                                    );
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            connected = false;
                            subscribed = false;
                            tracing::warn!(%error, "MQTT ingest connection interrupted");
                            tokio::select! {
                                () = cancellation.cancelled() => return Ok(()),
                                () = tokio::time::sleep(Duration::from_secs(1)) => {}
                            }
                        }
                    }
                }
                _ = convergence.tick() => {
                    if connected && !subscribed {
                        subscribed = try_subscribe(&client);
                    }
                    if !subscribed {
                        continue;
                    }
                    for command in storage.pending_activation_commands(256).await? {
                        match client.try_publish(
                            &command.topic,
                            QoS::AtLeastOnce,
                            false,
                            command.payload_json,
                        ) {
                            Ok(()) => {
                                storage
                                    .mark_activation_attempt(&command.activation_id, unix_millis())
                                    .await?;
                            }
                            Err(error) => {
                                tracing::debug!(
                                    activation_id = %command.activation_id,
                                    %error,
                                    "MQTT activation request queue is not currently writable"
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    fn validate_config(&self) -> Result<(), RuntimeError> {
        if self.config.broker_host.is_empty() || self.config.client_id.is_empty() {
            return Err(RuntimeError::Config(
                "broker_host and client_id must not be empty".into(),
            ));
        }
        if matches!(
            &self.config.transport,
            IngestTransport::TlsBundle { ca_pem } if ca_pem.is_empty()
        ) {
            return Err(RuntimeError::Config(
                "TLS bundle must contain at least one CA certificate".into(),
            ));
        }
        if self.config.username.is_none() && self.config.password.is_some() {
            return Err(RuntimeError::Config(
                "MQTT password requires a username".into(),
            ));
        }
        Ok(())
    }
}

fn install_crypto_provider() -> Result<(), RuntimeError> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        return Err(RuntimeError::Config(
            "Rustls cryptographic provider could not be installed".into(),
        ));
    }
    Ok(())
}

fn try_subscribe(client: &AsyncClient) -> bool {
    for topic in SUBSCRIPTIONS {
        if let Err(error) = client.try_subscribe(topic, QoS::AtLeastOnce) {
            tracing::debug!(%error, "MQTT subscription enqueue will be retried");
            return false;
        }
    }
    true
}

fn unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("invalid MQTT runtime configuration: {0}")]
    Config(String),
    #[error("MQTT client error: {0}")]
    Client(#[from] rumqttc::ClientError),
    #[error("MQTT ingest processing error: {0}")]
    Ingest(#[from] super::IngestError),
    #[error("MQTT activation storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
}
