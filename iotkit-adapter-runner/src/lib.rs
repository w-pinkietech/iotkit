mod inventory;
mod mqtt_client;
mod publish_loop;

use iotkit_core_mqtt_contract::{encode_status, now_ms, topic, EventType};
use iotkit_core_types::{AdapterId, AdapterEvent};
use rumqttc::{Event, Incoming, QoS};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};

/// MQTT broker connection configuration.
#[derive(Debug, Clone)]
pub struct MqttConfig {
    pub broker_url: String,
    pub client_id: Option<String>,
    pub keepalive_secs: Option<u32>,
    pub ca_path: Option<PathBuf>,
    pub client_cert_path: Option<PathBuf>,
    pub client_key_path: Option<PathBuf>,
}

/// Errors from the adapter runner.
#[derive(Debug)]
pub enum RunnerError {
    /// Invalid MQTT configuration
    Config(String),
    /// MQTT connection error
    Mqtt(String),
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(msg) => write!(f, "config error: {msg}"),
            Self::Mqtt(msg) => write!(f, "mqtt error: {msg}"),
        }
    }
}

impl std::error::Error for RunnerError {}

/// Compute exponential backoff with jitter.
/// base_ms doubles up to max_ms, then jitter of +/-30% is applied.
fn backoff_with_jitter(attempt: u32, base_ms: u64, max_ms: u64) -> std::time::Duration {
    let exp = base_ms.saturating_mul(1u64 << attempt.min(15));
    let capped = exp.min(max_ms);
    // Jitter: +/- 30%
    let jitter_range = (capped as f64 * 0.3) as u64;
    let jitter = if jitter_range > 0 {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        rng.gen_range(0..=jitter_range * 2) as i64 - jitter_range as i64
    } else {
        0
    };
    let result = (capped as i64 + jitter).max(100) as u64;
    std::time::Duration::from_millis(result)
}

/// Run the adapter event loop: receive events from adapter, publish to MQTT.
/// Blocks until SIGTERM/SIGINT or fatal error.
pub async fn run(
    adapter_id: AdapterId,
    mqtt_config: MqttConfig,
    event_rx: mpsc::Receiver<AdapterEvent>,
) -> Result<(), RunnerError> {
    let (client, mut eventloop) = mqtt_client::connect(&adapter_id, &mqtt_config)?;

    let connected = Arc::new(AtomicBool::new(false));
    let reconnect_notify = Arc::new(Notify::new());
    let inventory = inventory::InventoryTracker::new(adapter_id.clone());

    // Publish online status (retained) - will be sent once connected
    let status_topic = topic(&adapter_id, EventType::Status);
    let online_payload = encode_status(&adapter_id, true, now_ms(), "");
    client
        .publish(&status_topic, QoS::AtLeastOnce, true, online_payload)
        .await
        .map_err(|e| RunnerError::Mqtt(format!("failed to publish online status: {e}")))?;
    tracing::info!(adapter_id = adapter_id.as_str(), "published online status");

    // Spawn MQTT eventloop pump with exponential backoff reconnect
    let connected_el = connected.clone();
    let reconnect_notify_el = reconnect_notify.clone();
    let client_el = client.clone();
    let adapter_id_el = adapter_id.clone();
    let eventloop_handle = tokio::spawn(async move {
        let mut reconnect_attempt: u32 = 0;

        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                    tracing::info!("MQTT connected");
                    connected_el.store(true, Ordering::Relaxed);
                    reconnect_attempt = 0;

                    // Re-publish online status on reconnect
                    let online_payload =
                        encode_status(&adapter_id_el, true, now_ms(), "");
                    let _ = client_el
                        .publish(
                            &topic(&adapter_id_el, EventType::Status),
                            QoS::AtLeastOnce,
                            true,
                            online_payload,
                        )
                        .await;

                    // Signal publish_loop to republish inventory
                    reconnect_notify_el.notify_one();
                }
                Ok(_) => {} // PUBACK, PINGRESP, etc.
                Err(e) => {
                    connected_el.store(false, Ordering::Relaxed);
                    let delay =
                        backoff_with_jitter(reconnect_attempt, 1000, 30000);
                    tracing::warn!(
                        error = %e,
                        attempt = reconnect_attempt,
                        delay_ms = delay.as_millis() as u64,
                        "MQTT eventloop error, reconnecting"
                    );
                    reconnect_attempt = reconnect_attempt.saturating_add(1);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    });

    // Spawn publish loop as dedicated task
    let publish_handle = tokio::spawn(publish_loop::run(
        adapter_id.clone(),
        client.clone(),
        event_rx,
        inventory,
        connected,
        reconnect_notify,
    ));

    // Wait for publish_loop to exit (happens when event_rx is closed by adapter shutdown).
    // Signal handling is the caller's responsibility — the caller shuts down the adapter
    // (closing event_rx) before dropping the runner, ensuring no events are emitted
    // after offline status is published.
    let _ = publish_handle.await;
    tracing::info!("publish loop exited, publishing offline status");

    // Publish offline status with current timestamp (graceful shutdown)
    let offline_payload = encode_status(&adapter_id, false, now_ms(), "");
    let _ = client
        .publish(&status_topic, QoS::AtLeastOnce, true, offline_payload)
        .await;
    let _ = client.disconnect().await;

    // Grace period for eventloop to flush offline status and disconnect.
    // rumqttc's eventloop has no clean stop mechanism, so we abort after a
    // generous timeout. 2 seconds is sufficient for local brokers; remote
    // brokers over slow networks may not fully drain, which is acceptable for v1.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    eventloop_handle.abort();

    Ok(())
}
