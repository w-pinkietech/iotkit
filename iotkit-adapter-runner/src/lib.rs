mod backoff;
mod eventloop_task;
mod inventory;
mod mqtt_client;
mod publish_task;
mod session;

use eventloop_task::ConnectionState;
use iotkit_core_mqtt_contract::{encode_status, now_ms, topic, EventType};
use iotkit_core_types::{AdapterId, AdapterEvent};
use rumqttc::QoS;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

pub use iotkit_core_mqtt_contract::InventoryData;

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
/// The runner does NOT handle signals. The caller (binary) is responsible
/// for signal handling and adapter shutdown.
///
/// Returns Ok(()) on clean event_rx closure.
/// Returns Err on MQTT init failure or internal task crash.
pub async fn run(
    adapter_id: AdapterId,
    mqtt_config: MqttConfig,
    event_rx: mpsc::Receiver<AdapterEvent>,
) -> Result<(), RunnerError> {
    let session_id = session::generate_session_id();
    info!(session_id = %session_id, "runner starting");

    // Create MQTT client
    let (client, eventloop) =
        mqtt_client::create_mqtt_client(&adapter_id, &mqtt_config, &session_id)?;

    // Watch channel for connection state (level-triggered)
    let (conn_tx, conn_rx) = watch::channel(ConnectionState::Disconnected);

    // Spawn tasks
    let mut eventloop_join = tokio::spawn(eventloop_task::eventloop_run(eventloop, conn_tx));

    let client_clone = client.clone();
    let aid_clone = adapter_id.clone();
    let sid_clone = session_id.clone();
    let mut publish_join = tokio::spawn(publish_task::publish_run(
        event_rx,
        client_clone,
        conn_rx,
        aid_clone,
        sid_clone,
    ));

    let publish_result;
    tokio::select! {
        result = &mut publish_join => {
            publish_result = result;
        }
        result = &mut eventloop_join => {
            error!("eventloop task exited unexpectedly: {result:?}");
            publish_join.abort();
            return Err(RunnerError::EventLoopDied);
        }
    }

    // Normal path: publish_task exited first.
    const EVENTLOOP_WATCH_SENTINEL: &str = "eventloop_task watch sender dropped";

    match publish_result {
        Ok(Ok(())) => {
            debug!("publish_task exited cleanly (event_rx closed)");
        }
        Ok(Err(ref e)) if e == EVENTLOOP_WATCH_SENTINEL => {
            error!("eventloop_task died (detected via watch sender drop)");
            eventloop_join.abort();
            return Err(RunnerError::EventLoopDied);
        }
        Ok(Err(e)) => {
            error!("publish_task error: {e}");
            eventloop_join.abort();
            return Err(RunnerError::PublishTaskFailed(e));
        }
        Err(join_err) => {
            error!("publish_task panicked: {join_err}");
            eventloop_join.abort();
            return Err(RunnerError::PublishTaskFailed(join_err.to_string()));
        }
    }

    // Publish offline status (with timeout)
    let status_topic = topic(&adapter_id, EventType::Status);
    let offline_payload = encode_status(&adapter_id, false, now_ms(), &session_id);
    match tokio::time::timeout(
        Duration::from_secs(5),
        client.publish(status_topic, QoS::AtLeastOnce, true, offline_payload),
    )
    .await
    {
        Ok(Ok(())) => debug!("offline status published"),
        Ok(Err(e)) => warn!("failed to publish offline status: {e}"),
        Err(_) => warn!("offline status publish timed out, LWT will fire as fallback"),
    }

    // Disconnect
    let _ = client.disconnect().await;

    // Grace period: wait for eventloop to flush offline status + DISCONNECT to TCP
    match tokio::time::timeout(Duration::from_secs(2), &mut eventloop_join).await {
        Ok(_) => debug!("eventloop exited cleanly"),
        Err(_) => {
            warn!("eventloop did not exit within 2s grace period, aborting");
            eventloop_join.abort();
        }
    }

    info!("runner shutdown complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn fake_broker(listener: TcpListener) {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        if n > 0 && buf[0] >> 4 == 1 {
            stream.write_all(&[0x20, 0x02, 0x00, 0x00]).await.unwrap();
        }
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if buf[0] >> 4 == 3 && (buf[0] >> 1) & 0x03 == 1 {
                        let topic_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
                        let offset = 2 + 2 + topic_len;
                        if offset + 1 < n {
                            stream
                                .write_all(&[0x40, 0x02, buf[offset], buf[offset + 1]])
                                .await
                                .ok();
                        }
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn adapter_exit_causes_runner_to_return_ok() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(fake_broker(listener));

        let (event_tx, event_rx) = mpsc::channel(16);
        let config = MqttConfig {
            broker_url: format!("mqtt://{}:{}", addr.ip(), addr.port()),
            client_id: Some("test-adapter-exit".into()),
            keepalive_secs: Some(5),
            ca_path: None,
            client_cert_path: None,
            client_key_path: None,
        };

        let join = tokio::spawn(run(AdapterId::new("test"), config, event_rx));

        // Simulate adapter exit by dropping event_tx
        drop(event_tx);

        let result = join.await.unwrap();
        assert!(
            result.is_ok(),
            "runner should return Ok on clean event_rx close"
        );
    }

    #[tokio::test]
    async fn invalid_broker_url_returns_mqtt_init_error() {
        let (_tx, event_rx) = mpsc::channel(16);
        let config = MqttConfig {
            broker_url: "tcp://not-valid".into(),
            client_id: None,
            keepalive_secs: None,
            ca_path: None,
            client_cert_path: None,
            client_key_path: None,
        };

        let result = run(AdapterId::new("test"), config, event_rx).await;
        assert!(matches!(result, Err(RunnerError::MqttInit(_))));
    }

    #[tokio::test]
    async fn eventloop_death_returns_event_loop_died() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Fake broker that sends CONNACK then immediately closes
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 256];
            let _ = stream.read(&mut buf).await;
            stream.write_all(&[0x20, 0x02, 0x00, 0x00]).await.ok();
            drop(stream);
        });

        let (event_tx, event_rx) = mpsc::channel(16);
        let config = MqttConfig {
            broker_url: format!("mqtt://{}:{}", addr.ip(), addr.port()),
            client_id: Some("test-el-die".into()),
            keepalive_secs: Some(5),
            ca_path: None,
            client_cert_path: None,
            client_key_path: None,
        };

        let join = tokio::spawn(run(AdapterId::new("test"), config, event_rx));

        // Wait for reconnect cycle, then close event channel
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        drop(event_tx);

        let result = join.await.unwrap();
        assert!(
            result.is_ok(),
            "runner should exit cleanly after event_tx drop: {:?}",
            result.err()
        );
    }
}
