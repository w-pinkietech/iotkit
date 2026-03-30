mod inventory;
mod mqtt_client;
mod publish_loop;
mod session;

pub(crate) use session::generate_session_id;

use iotkit_core_types::{AdapterId, AdapterEvent};
use std::path::PathBuf;
use tokio::sync::mpsc;

/// MQTT connection configuration.
pub struct MqttConfig {
    pub broker_url: String,
    pub client_id: Option<String>,
    pub keepalive_secs: Option<u16>,
    pub ca_path: Option<PathBuf>,
    pub client_cert_path: Option<PathBuf>,
    pub client_key_path: Option<PathBuf>,
}

/// Errors returned by `run()`.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("MQTT client initialization failed: {0}")]
    MqttInit(String),

    #[error("eventloop task died unexpectedly")]
    EventLoopDied,

    #[error("publish task failed: {0}")]
    PublishTaskFailed(String),
}

/// Run the MQTT adapter runner until event_rx closes.
///
/// Creates an MQTT client, spawns eventloop + publish tasks,
/// processes events until event_rx closes, publishes offline status.
///
/// Returns Ok(()) on clean event_rx closure.
/// Returns Err on MQTT init failure or internal task crash.
pub async fn run(
    adapter_id: AdapterId,
    mqtt_config: MqttConfig,
    event_rx: mpsc::Receiver<AdapterEvent>,
) -> Result<(), RunnerError> {
    // Implementation in subsequent tasks.
    // For now, just drain and return Ok.
    let _ = adapter_id;
    let _ = mqtt_config;
    let mut rx = event_rx;
    while rx.recv().await.is_some() {}
    Ok(())
}
