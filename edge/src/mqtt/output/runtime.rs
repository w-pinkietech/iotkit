use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, Outgoing, QoS};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::storage::{ClaimedOutput, OutputMark, Storage, StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryAction {
    None,
    MarkPublished {
        export_id: String,
        claim_token: String,
    },
}

pub struct DeliveryTracker {
    export_id: String,
    claim_token: String,
    packet_id: Option<u16>,
    completed: bool,
}

impl DeliveryTracker {
    #[must_use]
    pub fn new(export_id: impl Into<String>, claim_token: impl Into<String>) -> Self {
        Self {
            export_id: export_id.into(),
            claim_token: claim_token.into(),
            packet_id: None,
            completed: false,
        }
    }

    #[must_use]
    pub const fn queued(&self) -> DeliveryAction {
        DeliveryAction::None
    }

    #[must_use]
    pub fn outgoing_publish(&mut self, packet_id: u16) -> DeliveryAction {
        if !self.completed && self.packet_id.is_none() && packet_id != 0 {
            self.packet_id = Some(packet_id);
        }
        DeliveryAction::None
    }

    #[must_use]
    pub fn incoming_puback(&mut self, packet_id: u16) -> DeliveryAction {
        if self.completed || self.packet_id != Some(packet_id) {
            return DeliveryAction::None;
        }
        self.completed = true;
        DeliveryAction::MarkPublished {
            export_id: self.export_id.clone(),
            claim_token: self.claim_token.clone(),
        }
    }
}

pub struct OutputRuntimeConfig {
    pub mqtt: MqttOptions,
    pub request_capacity: usize,
    pub claim_lease: Duration,
    pub idle_poll: Duration,
    pub reconnect_delay: Duration,
}

pub struct OutputRuntime {
    storage: Storage,
    config: OutputRuntimeConfig,
}

impl OutputRuntime {
    #[must_use]
    pub fn new(storage: Storage, config: OutputRuntimeConfig) -> Self {
        Self { storage, config }
    }

    pub async fn run(self, cancellation: CancellationToken) -> Result<(), OutputRuntimeError> {
        if self.config.request_capacity == 0 {
            return Err(OutputRuntimeError::InvalidConfiguration);
        }
        let (client, mut eventloop) =
            AsyncClient::new(self.config.mqtt.clone(), self.config.request_capacity);
        while !cancellation.is_cancelled() {
            let token = format!("claim-{}", Uuid::new_v4().simple());
            let Some(claimed) = self
                .storage
                .claim_output(
                    &token,
                    unix_millis()?,
                    duration_millis(self.config.claim_lease)?,
                )
                .await?
            else {
                tokio::select! {
                    () = cancellation.cancelled() => break,
                    () = tokio::time::sleep(self.config.idle_poll) => {}
                }
                continue;
            };
            if let Err(error) = client
                .publish(
                    claimed.topic.clone(),
                    QoS::AtLeastOnce,
                    claimed.retain,
                    claimed.payload.clone(),
                )
                .await
            {
                self.storage
                    .release_output(&claimed.export_id, &token)
                    .await?;
                return Err(OutputRuntimeError::Client(error));
            }
            let mut tracker = DeliveryTracker::new(&claimed.export_id, &token);
            if !self
                .drive_publication(&mut eventloop, &claimed, &mut tracker, &cancellation)
                .await?
            {
                break;
            }
        }
        Ok(())
    }

    async fn drive_publication(
        &self,
        eventloop: &mut rumqttc::EventLoop,
        claimed: &ClaimedOutput,
        tracker: &mut DeliveryTracker,
        cancellation: &CancellationToken,
    ) -> Result<bool, OutputRuntimeError> {
        loop {
            let event = tokio::select! {
                () = cancellation.cancelled() => return Ok(false),
                result = eventloop.poll() => result,
            };
            match event {
                Ok(Event::Outgoing(Outgoing::Publish(packet_id))) => {
                    let _ = tracker.outgoing_publish(packet_id);
                }
                Ok(Event::Incoming(Incoming::PubAck(ack))) => {
                    if let DeliveryAction::MarkPublished {
                        export_id,
                        claim_token,
                    } = tracker.incoming_puback(ack.pkid)
                    {
                        loop {
                            match self
                                .storage
                                .mark_output_published(&export_id, &claim_token, unix_millis()?)
                                .await
                            {
                                Ok(OutputMark::Published) => return Ok(true),
                                Ok(OutputMark::ClaimLost) => {
                                    return Err(OutputRuntimeError::ClaimLost(export_id));
                                }
                                Err(error) => {
                                    tokio::select! {
                                        () = cancellation.cancelled() => return Ok(false),
                                        () = tokio::time::sleep(self.config.reconnect_delay) => {
                                            if !matches!(error, StorageError::Database(_)) {
                                                return Err(OutputRuntimeError::Storage(error));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => {
                    tokio::select! {
                        () = cancellation.cancelled() => return Ok(false),
                        () = tokio::time::sleep(self.config.reconnect_delay) => {}
                    }
                    // The same EventLoop retains the QoS 1 state and retransmits it.
                    // Do not queue a second client publication here.
                    let _ = claimed;
                }
            }
        }
    }
}

fn unix_millis() -> Result<i64, OutputRuntimeError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OutputRuntimeError::Clock)?
        .as_millis();
    i64::try_from(millis).map_err(|_| OutputRuntimeError::Clock)
}

fn duration_millis(duration: Duration) -> Result<i64, OutputRuntimeError> {
    i64::try_from(duration.as_millis()).map_err(|_| OutputRuntimeError::InvalidConfiguration)
}

#[derive(Debug, thiserror::Error)]
pub enum OutputRuntimeError {
    #[error("output runtime configuration is invalid")]
    InvalidConfiguration,
    #[error("system clock is before the Unix epoch or out of range")]
    Clock,
    #[error("output storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("queue output publication: {0}")]
    Client(#[from] rumqttc::ClientError),
    #[error("the durable output claim was lost before PUBACK mark: {0}")]
    ClaimLost(String),
}
