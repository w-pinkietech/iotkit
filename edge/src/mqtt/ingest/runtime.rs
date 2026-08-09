use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use rumqttc::{
    AsyncClient, ClientError, Event, Incoming, MqttOptions, QoS, SubscribeFilter,
    SubscribeReasonCode, Transport,
};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::{AckPublication, IngestError, IngestProcessor};
use iotkit_edge_custody_contract::{
    AcceptedThrough, MAX_BATCH_BYTES, MAX_DESCRIPTOR_BYTES, MAX_STATUS_BYTES,
};

const SUBSCRIPTIONS: [&str; 6] = [
    "iotkit/v1/edge-nodes/+/records",
    "iotkit/v1/edge-nodes/+/status",
    "iotkit/v1/edge-nodes/+/descriptors",
    "iotkit/v1/edge-nodes/+/activation/result",
    "iotkit/v1/edge-nodes/+/recovery/result",
    "iotkit/v1/edge-nodes/+/recovery/completion-ack",
];
const MAX_MQTT_TOPIC_BYTES: usize = u16::MAX as usize;
const MQTT_PACKET_OVERHEAD_BYTES: usize = 16;

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

/// Closed, process-local evidence for the IoTKit Edge MQTT ingest path.
/// It intentionally retains no broker error text or connection endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestConnectionState {
    Unknown,
    Connecting,
    Ready,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IngestRuntimeHealth {
    pub state: IngestConnectionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_ready_at: Option<i64>,
}

#[derive(Clone)]
pub struct IngestHealth(Arc<Mutex<IngestRuntimeHealth>>);

impl Default for IngestHealth {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(IngestRuntimeHealth {
            state: IngestConnectionState::Unknown,
            last_ready_at: None,
        })))
    }
}

impl IngestHealth {
    #[must_use]
    pub fn snapshot(&self) -> IngestRuntimeHealth {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn connecting(&self) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .state = IngestConnectionState::Connecting;
    }

    fn ready(&self, ready_at: i64) {
        let mut health = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.state = IngestConnectionState::Ready;
        health.last_ready_at = Some(ready_at);
    }

    fn disconnected(&self) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .state = IngestConnectionState::Disconnected;
    }
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
    health: IngestHealth,
}

impl IngestRuntime {
    #[must_use]
    pub fn new(config: IngestRuntimeConfig, processor: IngestProcessor) -> Self {
        Self::with_health(config, processor, IngestHealth::default())
    }

    #[must_use]
    pub fn with_health(
        config: IngestRuntimeConfig,
        processor: IngestProcessor,
        health: IngestHealth,
    ) -> Self {
        Self {
            config,
            processor,
            health,
        }
    }

    #[must_use]
    pub fn health(&self) -> IngestHealth {
        self.health.clone()
    }

    pub async fn run(self, cancellation: CancellationToken) -> Result<(), RuntimeError> {
        self.validate_config()?;
        let mut options = MqttOptions::new(
            &self.config.client_id,
            &self.config.broker_host,
            self.config.broker_port,
        );
        configure_packet_limits(&mut options);
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
        self.health.connecting();
        let mut connected = false;
        let mut subscribed = false;
        let mut subscription_requested = false;
        let mut pending_custody_acks = PendingCustodyAcks::default();
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
                            subscribed = false;
                            self.health.connecting();
                            subscription_requested = try_subscribe_all(&client);
                        }
                        Ok(Event::Incoming(Incoming::SubAck(ack))) => {
                            if subscription_requested && subscriptions_confirmed(&ack.return_codes) {
                                subscribed = true;
                                subscription_requested = false;
                                self.health.ready(unix_millis());
                            } else {
                                // An invalid or stale SubAck is not proof that every
                                // required subscription is active. Retry one bounded
                                // Subscribe packet from the convergence tick.
                                subscribed = false;
                                subscription_requested = false;
                                self.health.connecting();
                                tracing::warn!("MQTT Broker did not confirm every ingest subscription");
                            }
                        }
                        Ok(Event::Incoming(Incoming::Publish(publication))) => {
                            match self.processor.handle_publication(
                                &publication.topic,
                                &publication.payload,
                                unix_millis(),
                                publication.retain,
                            ).await {
                                Ok(Some(ack)) => match pending_custody_acks.try_enqueue(&client, ack) {
                                    Ok(()) => {}
                                    Err(RuntimeError::Client(error)) => {
                                        tracing::warn!(
                                            %error,
                                            "custody acknowledgement enqueue deferred until retry"
                                        );
                                    }
                                    Err(error) => return Err(error),
                                },
                                Ok(None) => {}
                                Err(error) if error.is_fatal_runtime() => {
                                    return Err(RuntimeError::Ingest(error));
                                }
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
                            subscription_requested = false;
                            self.health.disconnected();
                            tracing::warn!(%error, "MQTT ingest connection interrupted");
                            tokio::select! {
                                () = cancellation.cancelled() => return Ok(()),
                                () = tokio::time::sleep(Duration::from_secs(1)) => {}
                            }
                        }
                    }
                }
                _ = convergence.tick() => {
                    if let Err(error) = pending_custody_acks.retry(&client) {
                        tracing::debug!(
                            %error,
                            "MQTT custody acknowledgement queue is not currently writable"
                        );
                        continue;
                    }
                    if connected && !subscribed && !subscription_requested {
                        self.health.connecting();
                        subscription_requested = try_subscribe_all(&client);
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
                    for command in storage
                        .pending_recovery_commands_due(256, unix_millis())
                        .await?
                    {
                        match client.try_publish(
                            &command.topic,
                            QoS::AtLeastOnce,
                            false,
                            command.payload_json,
                        ) {
                            Ok(()) => {
                                storage
                                    .mark_recovery_attempt(
                                        &command.recovery_id,
                                        &command.kind,
                                        unix_millis(),
                                    )
                                    .await?;
                            }
                            Err(error) => {
                                tracing::debug!(
                                    recovery_id = %command.recovery_id,
                                    kind = %command.kind,
                                    %error,
                                    "MQTT recovery command queue is not currently writable"
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

#[derive(Default)]
struct PendingCustodyAcks {
    by_topic: BTreeMap<String, PendingCustodyAck>,
}

struct PendingCustodyAck {
    acknowledgement: AckPublication,
    accepted: AcceptedThrough,
}

impl PendingCustodyAcks {
    fn try_enqueue(
        &mut self,
        client: &AsyncClient,
        acknowledgement: AckPublication,
    ) -> Result<(), RuntimeError> {
        let accepted = AcceptedThrough::decode(&acknowledgement.payload)
            .map_err(IngestError::Contract)
            .map_err(RuntimeError::Ingest)?;
        let topic = acknowledgement.topic.clone();
        if let Some(pending) = self.by_topic.get_mut(&topic) {
            if accepted.edge_node_id != pending.accepted.edge_node_id
                || accepted.ledger_epoch != pending.accepted.ledger_epoch
            {
                return Err(RuntimeError::PendingAcknowledgementCorrelation);
            }
            if accepted.accepted_through > pending.accepted.accepted_through {
                *pending = PendingCustodyAck {
                    acknowledgement,
                    accepted,
                };
            } else if accepted.accepted_through == pending.accepted.accepted_through
                && accepted.publication_id != pending.accepted.publication_id
            {
                return Err(RuntimeError::PendingAcknowledgementCorrelation);
            }
            return Ok(());
        }
        match publish_custody_ack(client, &acknowledgement) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.by_topic.insert(
                    topic,
                    PendingCustodyAck {
                        acknowledgement,
                        accepted,
                    },
                );
                Err(RuntimeError::Client(error))
            }
        }
    }

    fn retry(&mut self, client: &AsyncClient) -> Result<(), ClientError> {
        for topic in self.by_topic.keys().cloned().collect::<Vec<_>>() {
            let acknowledgement = &self.by_topic[&topic].acknowledgement;
            publish_custody_ack(client, acknowledgement)?;
            self.by_topic.remove(&topic);
        }
        Ok(())
    }
}

fn publish_custody_ack(
    client: &AsyncClient,
    acknowledgement: &AckPublication,
) -> Result<(), ClientError> {
    client.try_publish(
        &acknowledgement.topic,
        // accepted-through is fixed at QoS 1 by the custody contract.
        QoS::AtLeastOnce,
        acknowledgement.retain,
        acknowledgement.payload.clone(),
    )
}

fn configure_packet_limits(options: &mut MqttOptions) {
    let limit = MAX_BATCH_BYTES
        .max(MAX_DESCRIPTOR_BYTES)
        .max(MAX_STATUS_BYTES)
        + MAX_MQTT_TOPIC_BYTES
        + MQTT_PACKET_OVERHEAD_BYTES;
    options.set_max_packet_size(limit, limit);
}

pub(crate) fn install_crypto_provider() -> Result<(), RuntimeError> {
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

fn try_subscribe_all(client: &AsyncClient) -> bool {
    let filters =
        SUBSCRIPTIONS.map(|topic| SubscribeFilter::new(topic.to_string(), QoS::AtLeastOnce));
    if let Err(error) = client.try_subscribe_many(filters) {
        tracing::debug!(%error, "MQTT subscription enqueue will be retried");
        return false;
    }
    true
}

fn subscriptions_confirmed(return_codes: &[SubscribeReasonCode]) -> bool {
    return_codes.len() == SUBSCRIPTIONS.len()
        && return_codes
            .iter()
            .all(|code| matches!(code, SubscribeReasonCode::Success(QoS::AtLeastOnce)))
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
    #[error("conflicting pending MQTT custody acknowledgement")]
    PendingAcknowledgementCorrelation,
    #[error("MQTT ingest processing error: {0}")]
    Ingest(#[from] super::IngestError),
    #[error("MQTT activation storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
}

#[cfg(test)]
#[path = "../../../tests/unit/mqtt_ingest_runtime_tests.rs"]
mod tests;
