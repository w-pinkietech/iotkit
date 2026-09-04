//! MQTT Output Adapter v1 (#232 child issue 4).
//!
//! Drains the observation outbox to the configured Broker under the contract
//! in `docs/product/<lang>/contracts/mqtt-output-adapter-v1.md`: QoS 1 with
//! retain, one publication in flight, the row deleted only after PUBACK, and
//! the outbox as the only source of retransmission (`clean_session = true`).
//! Publishes the status heartbeat, the immediate `online` / `degraded` and
//! `faults` changes, the Will, and the graceful `offline`.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use iotkit_core_collector::PipelineDelivery;
use iotkit_core_ops::{ClockEvidence, ClockTrust};
use iotkit_core_pipeline::{
    DeviceFaults, InputTime, OutboxRow, Status, StatusValue, WILL_PAYLOAD, outbox, status_topic,
};
use iotkit_core_storage::{DbHandle, StorageError};
use iotkit_core_types::EdgeNodeId;
use iotkit_edge_node::config::{MqttOutputConfig, MqttTrustMode};
use rumqttc::{AsyncClient, Event, Incoming, LastWill, MqttOptions, Outgoing, QoS, Transport};
use tokio::sync::{Notify, oneshot};

const KEEP_ALIVE: Duration = Duration::from_secs(30);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
/// Fallback for outbox rows inserted without a commit notification (nodectl,
/// the Console API); the collector's commits are picked up immediately.
const IDLE_PROBE_INTERVAL: Duration = Duration::from_secs(1);
/// Contract section 7: the graceful `offline` waits for its PUBACK at most this long.
const OFFLINE_ACK_TIMEOUT: Duration = Duration::from_secs(2);
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) struct OutputMqtt {
    connection: MqttOutputConfig,
    edge_node_id: EdgeNodeId,
    heartbeat_interval: Duration,
    password: String,
    ca: Option<Vec<u8>>,
}

/// Everything the adapter reads while running.
pub(crate) struct OutputSources {
    pub db: DbHandle,
    pub faults: DeviceFaults,
    pub pipelines: Arc<PipelineDelivery>,
    pub clock_trust: Arc<ClockTrust>,
}

pub(crate) struct OutputMqttHandle {
    shutdown: oneshot::Sender<()>,
    join: tokio::task::JoinHandle<()>,
}

impl OutputMqttHandle {
    /// Publishes the graceful `offline` and disconnects. Bounded by the
    /// adapter's own PUBACK and disconnect timeouts.
    pub(crate) async fn shutdown(self) {
        let _ = self.shutdown.send(());
        if let Err(error) = self.join.await {
            tracing::error!(error = %error, "MQTT Output Adapter panicked during shutdown");
        }
    }

    pub(crate) async fn wait(&mut self) -> Result<(), tokio::task::JoinError> {
        (&mut self.join).await
    }
}

/// Reads the secrets the connection needs so that a misconfiguration fails at
/// startup instead of on the first connection attempt.
pub(crate) fn prepare(
    connection: MqttOutputConfig,
    edge_node_id: EdgeNodeId,
    heartbeat_interval: Duration,
) -> Result<OutputMqtt, String> {
    let password = read_password(&connection)?;
    let ca = read_ca(&connection)?;
    Ok(OutputMqtt {
        connection,
        edge_node_id,
        heartbeat_interval,
        password,
        ca,
    })
}

pub(crate) fn spawn(adapter: OutputMqtt, sources: OutputSources) -> OutputMqttHandle {
    let (shutdown, shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(run(adapter, sources, shutdown_rx));
    OutputMqttHandle { shutdown, join }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Publication {
    Status,
    Outbox(i64),
}

/// A publish handed to rumqttc whose PUBACK has not arrived. rumqttc assigns
/// the packet id when it writes the packet, reported as `Outgoing::Publish`
/// in the order the publishes were requested.
struct Pending {
    publication: Publication,
    pkid: Option<u16>,
}

struct Session {
    client: AsyncClient,
    status_topic: String,
    heartbeat_interval: Duration,
    sources: OutputSources,
    connected: bool,
    pending: VecDeque<Pending>,
    /// The outbox row currently in flight; at most one.
    inflight: Option<i64>,
}

async fn run(adapter: OutputMqtt, sources: OutputSources, mut shutdown: oneshot::Receiver<()>) {
    let status_topic = status_topic(&adapter.edge_node_id);
    let mut options = MqttOptions::new(
        format!("iotkit-edge-node-{}", adapter.edge_node_id),
        adapter.connection.host.clone(),
        adapter.connection.port,
    );
    options.set_keep_alive(KEEP_ALIVE);
    options.set_clean_session(true);
    options.set_credentials(adapter.edge_node_id.as_str(), &adapter.password);
    options.set_last_will(LastWill::new(
        status_topic.clone(),
        WILL_PAYLOAD,
        QoS::AtLeastOnce,
        true,
    ));
    options.set_transport(if adapter.connection.allow_insecure {
        Transport::tcp()
    } else if let Some(ca) = adapter.ca {
        Transport::tls(ca, None, None)
    } else {
        Transport::tls_with_default_config()
    });
    let (client, mut event_loop) = AsyncClient::new(options, 16);

    let fault_changed = Arc::new(Notify::new());
    {
        let notify = fault_changed.clone();
        sources.faults.set_listener(move || notify.notify_one());
    }
    let mut session = Session {
        client,
        status_topic,
        heartbeat_interval: adapter.heartbeat_interval,
        sources,
        connected: false,
        pending: VecDeque::new(),
        inflight: None,
    };
    let mut heartbeat = tokio::time::interval(session.heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut idle_probe = tokio::time::interval(IDLE_PROBE_INTERVAL);
    idle_probe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            event = event_loop.poll() => match event {
                Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                    tracing::info!("MQTT Output Adapter connected");
                    session.connected = true;
                    session.pending.clear();
                    session.inflight = None;
                    // Contract section 6: status first, then the outbox.
                    session.publish_status(StatusValue::Online).await;
                    heartbeat.reset();
                    session.publish_next_outbox_row().await;
                }
                Ok(Event::Outgoing(Outgoing::Publish(pkid))) => session.note_packet_written(pkid),
                Ok(Event::Incoming(Incoming::PubAck(ack))) => session.note_puback(ack.pkid).await,
                Ok(_) => {}
                Err(error) => {
                    if session.connected {
                        tracing::warn!(error = %error, "MQTT connection lost; the outbox keeps the unacknowledged publication");
                    } else {
                        tracing::warn!(error = %error, "MQTT connection attempt failed; retrying");
                    }
                    session.connected = false;
                    session.pending.clear();
                    session.inflight = None;
                    tokio::time::sleep(RECONNECT_DELAY).await;
                }
            },
            _ = session.sources.pipelines.committed().notified(), if session.connected && session.inflight.is_none() => {
                session.publish_next_outbox_row().await;
            }
            _ = idle_probe.tick(), if session.connected && session.inflight.is_none() => {
                session.publish_next_outbox_row().await;
            }
            _ = heartbeat.tick(), if session.connected => {
                session.publish_status(StatusValue::Online).await;
            }
            _ = fault_changed.notified(), if session.connected => {
                session.publish_status(StatusValue::Online).await;
                heartbeat.reset();
            }
            _ = &mut shutdown => break,
        }
    }

    if !session.connected {
        return;
    }
    // Graceful offline: publish with the shutdown time and the current faults,
    // wait for its PUBACK, then a clean DISCONNECT so the Broker does not
    // publish the Will.
    session.pending.clear();
    session.publish_status(StatusValue::Offline).await;
    let acked = tokio::time::timeout(OFFLINE_ACK_TIMEOUT, async {
        loop {
            match event_loop.poll().await {
                Ok(Event::Outgoing(Outgoing::Publish(pkid))) => session.note_packet_written(pkid),
                Ok(Event::Incoming(Incoming::PubAck(ack))) => {
                    session.note_puback(ack.pkid).await;
                    if session
                        .pending
                        .iter()
                        .all(|pending| pending.publication != Publication::Status)
                    {
                        return true;
                    }
                }
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    if !acked {
        tracing::warn!("graceful offline status was not acknowledged before disconnecting");
    }
    if session.client.disconnect().await.is_ok() {
        let _ = tokio::time::timeout(DISCONNECT_TIMEOUT, async {
            loop {
                match event_loop.poll().await {
                    Ok(Event::Outgoing(Outgoing::Disconnect)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        })
        .await;
    }
    tracing::info!(acked, "MQTT Output Adapter disconnected");
}

impl Session {
    async fn publish_status(&mut self, value: StatusValue) {
        let now = self.now().await;
        let snapshot = self.sources.faults.snapshot();
        let value = match value {
            StatusValue::Offline => StatusValue::Offline,
            _ if snapshot.degraded() => StatusValue::Degraded,
            _ => StatusValue::Online,
        };
        let status = Status {
            at: now,
            value,
            faults: snapshot.faults(now),
        };
        let topic = self.status_topic.clone();
        self.publish(Publication::Status, &topic, true, status.payload())
            .await;
    }

    /// Loads the oldest outbox row and publishes it unless one is in flight.
    async fn publish_next_outbox_row(&mut self) {
        if self.inflight.is_some() {
            return;
        }
        let row = match self
            .sources
            .db
            .with_conn(|conn| outbox::oldest(conn).map_err(StorageError::from))
            .await
        {
            Ok(Some(row)) => row,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(error = %error, "failed to read the observation outbox");
                return;
            }
        };
        let OutboxRow {
            outbox_seq,
            topic,
            payload,
            retain,
            ..
        } = row;
        self.inflight = Some(outbox_seq);
        self.publish(Publication::Outbox(outbox_seq), &topic, retain, payload)
            .await;
    }

    async fn publish(
        &mut self,
        publication: Publication,
        topic: &str,
        retain: bool,
        payload: Vec<u8>,
    ) {
        match self
            .client
            .publish(topic, QoS::AtLeastOnce, retain, payload)
            .await
        {
            Ok(()) => self.pending.push_back(Pending {
                publication,
                pkid: None,
            }),
            Err(error) => {
                tracing::warn!(error = %error, topic, "failed to queue an MQTT publish");
                if let Publication::Outbox(_) = publication {
                    self.inflight = None;
                }
            }
        }
    }

    fn note_packet_written(&mut self, pkid: u16) {
        if let Some(pending) = self
            .pending
            .iter_mut()
            .find(|pending| pending.pkid.is_none())
        {
            pending.pkid = Some(pkid);
        }
    }

    /// PUBACK is the boundary of IoTKit's delivery responsibility: the
    /// acknowledged outbox row is deleted and the next row goes out.
    async fn note_puback(&mut self, pkid: u16) {
        let Some(position) = self
            .pending
            .iter()
            .position(|pending| pending.pkid == Some(pkid))
        else {
            tracing::debug!(pkid, "PUBACK for a publication this session does not track");
            return;
        };
        let Some(pending) = self.pending.remove(position) else {
            return;
        };
        match pending.publication {
            Publication::Status => {}
            Publication::Outbox(outbox_seq) => {
                if let Err(error) = self
                    .sources
                    .db
                    .with_conn(move |conn| {
                        outbox::delete(conn, outbox_seq).map_err(StorageError::from)
                    })
                    .await
                {
                    // The row stays and is re-sent; the contract allows a
                    // duplicate of the most recent publication.
                    tracing::error!(error = %error, outbox_seq, "acknowledged outbox row could not be deleted");
                }
                if self.inflight == Some(outbox_seq) {
                    self.inflight = None;
                }
                self.publish_next_outbox_row().await;
            }
        }
    }

    /// The two clocks now: uptime always, the wall clock only while trusted.
    async fn now(&self) -> InputTime {
        let clock_trust = self.sources.clock_trust.clone();
        let trusted = self
            .sources
            .db
            .with_conn(move |conn| {
                Ok(matches!(
                    clock_trust.refresh(conn),
                    Ok(ClockEvidence::Trusted { .. })
                ))
            })
            .await
            .unwrap_or(false);
        InputTime::now(trusted.then(|| self.sources.clock_trust.wall_time_ms()))
    }
}

fn read_password(config: &MqttOutputConfig) -> Result<String, String> {
    let contents = std::fs::read_to_string(&config.password_file).map_err(|error| {
        format!(
            "failed to read MQTT password file {}: {error}",
            config.password_file.display()
        )
    })?;
    let password = contents.trim_end_matches(['\r', '\n']);
    if password.is_empty() {
        return Err("MQTT password file is empty".to_string());
    }
    Ok(password.to_string())
}

fn read_ca(config: &MqttOutputConfig) -> Result<Option<Vec<u8>>, String> {
    match (&config.trust_mode, &config.ca_file) {
        (MqttTrustMode::SystemRoots, None) => Ok(None),
        (MqttTrustMode::BundleOnly, Some(path)) => std::fs::read(path)
            .map(Some)
            .map_err(|error| format!("failed to read MQTT CA file {}: {error}", path.display())),
        _ => Err("invalid resolved MQTT trust configuration".to_string()),
    }
}
