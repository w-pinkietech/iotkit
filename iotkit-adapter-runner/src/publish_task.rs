use crate::eventloop_task::ConnectionState;
use crate::inventory::Inventory;
use iotkit_core_mqtt_contract::{
    encode_event, encode_inventory, encode_status, inventory_topic, now_ms, topic, EventType,
};
use iotkit_core_types::{AdapterId, AdapterEvent};
use rumqttc::{AsyncClient, QoS};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

const PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);

/// Error type for publish_run, replacing magic string sentinels.
#[derive(Debug)]
pub(crate) enum PublishError {
    /// The eventloop task's watch sender was dropped (eventloop died).
    WatchSenderDropped,
    /// Any other error.
    Other(String),
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PublishError::WatchSenderDropped => {
                write!(f, "eventloop_task watch sender dropped")
            }
            PublishError::Other(s) => write!(f, "{s}"),
        }
    }
}

/// Publish a message with a 5-second timeout. Returns true on success.
async fn publish_with_timeout(
    client: &AsyncClient,
    topic: String,
    qos: QoS,
    retain: bool,
    payload: Vec<u8>,
) -> bool {
    match tokio::time::timeout(PUBLISH_TIMEOUT, client.publish(topic, qos, retain, payload)).await
    {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            warn!("publish error: {e}");
            false
        }
        Err(_) => {
            warn!("publish timed out after 5s");
            false
        }
    }
}

/// Run the reconcile sequence on ConnAck. Fail-fast: stop on first publish failure.
async fn reconcile(
    client: &AsyncClient,
    adapter_id: &AdapterId,
    session_id: &str,
    inventory: &Inventory,
) {
    // Step 1: Publish online status
    let status_topic = topic(adapter_id, EventType::Status);
    let status_payload = encode_status(adapter_id, true, now_ms(), session_id);
    if !publish_with_timeout(client, status_topic, QoS::AtLeastOnce, true, status_payload).await {
        warn!("reconcile: failed to publish online status, aborting reconcile");
        return;
    }

    // Step 2: Publish all inventory entries
    let total = inventory.desired.len();
    for (i, (device_key_str, maybe_data)) in inventory.desired.iter().enumerate() {
        let inv_topic = {
            let dk = iotkit_core_types::DeviceKey::new(device_key_str.clone());
            inventory_topic(adapter_id, &dk)
        };

        let payload = match maybe_data {
            Some(data) => encode_inventory(adapter_id, data, session_id, now_ms()),
            None => Vec::new(), // tombstone: empty retained
        };

        if !publish_with_timeout(client, inv_topic, QoS::AtLeastOnce, true, payload).await {
            let remaining = total - i - 1;
            warn!(
                "reconcile: publish failed, {remaining} entries not reconciled; will retry on next ConnAck"
            );
            return;
        }
    }

    info!(
        "reconcile complete: {} active, {} tombstones",
        inventory.desired.values().filter(|v| v.is_some()).count(),
        inventory.desired.values().filter(|v| v.is_none()).count(),
    );
}

/// The publish task's main loop.
///
/// Exclusively owns: event_rx, desired_inventory, conn_rx, client clone.
/// Exits when event_rx is closed (adapter stopped).
pub(crate) async fn publish_run(
    mut event_rx: mpsc::Receiver<AdapterEvent>,
    client: AsyncClient,
    mut conn_rx: watch::Receiver<ConnectionState>,
    adapter_id: AdapterId,
    session_id: String,
) -> Result<(), PublishError> {
    let mut inventory = Inventory::new();

    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(ev) => {
                        // Always track inventory
                        inventory.track_event(&ev);

                        if *conn_rx.borrow() == ConnectionState::Connected {
                            publish_event(&client, &adapter_id, &ev, &session_id, &inventory).await;
                        } else {
                            warn!("disconnected, dropping non-retained event");
                        }
                    }
                    None => {
                        // event_rx closed -- adapter stopped
                        debug!("event_rx closed, exiting publish loop");
                        break;
                    }
                }
            }

            result = conn_rx.changed() => {
                if result.is_err() {
                    // watch sender dropped -- eventloop_task exited unexpectedly.
                    // Return error so run() can classify this as EventLoopDied.
                    warn!("conn_rx sender dropped -- eventloop_task died");
                    return Err(PublishError::WatchSenderDropped);
                }
                if *conn_rx.borrow() == ConnectionState::Connected {
                    reconcile(&client, &adapter_id, &session_id, &inventory).await;
                }
                // Disconnected: no action. Next event_rx.recv() will check conn_rx.borrow().
            }
        }
    }

    // Only reached when event_rx closes (adapter stopped) -- this is the normal exit.
    Ok(())
}

/// Publish a single event (when connected).
async fn publish_event(
    client: &AsyncClient,
    adapter_id: &AdapterId,
    event: &AdapterEvent,
    session_id: &str,
    inventory: &Inventory,
) {
    // Encode the non-retained event
    match encode_event(adapter_id, event) {
        Ok((event_type, payload)) => {
            let t = topic(adapter_id, event_type);
            publish_with_timeout(client, t, QoS::AtLeastOnce, false, payload).await;
        }
        Err(e) => {
            debug!("skipping unsupported event: {e}");
        }
    }

    // For DeviceDiscovered: also publish retained inventory
    if let AdapterEvent::DeviceDiscovered { device_key, .. } = event {
        let key = device_key.as_str();
        if let Some(Some(data)) = inventory.desired.get(key) {
            let inv_topic = inventory_topic(adapter_id, device_key);
            let payload = encode_inventory(adapter_id, data, session_id, now_ms());
            publish_with_timeout(client, inv_topic, QoS::AtLeastOnce, true, payload).await;
        }
    }

    // For DeviceLost: publish empty retained to clear inventory
    if let AdapterEvent::DeviceLost { device_key, .. } = event {
        let inv_topic = inventory_topic(adapter_id, device_key);
        publish_with_timeout(client, inv_topic, QoS::AtLeastOnce, true, Vec::new()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eventloop_task::ConnectionState;
    use iotkit_core_types::*;
    use std::collections::BTreeMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Minimal MQTT broker: accepts CONNECT -> sends CONNACK, accepts PUBLISH -> sends PUBACK.
    async fn fake_broker(listener: TcpListener) {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];

        // Read CONNECT
        let n = stream.read(&mut buf).await.unwrap();
        assert!(n > 0 && buf[0] >> 4 == 1, "expected CONNECT");
        // Send CONNACK (session present = 0, return code = 0)
        stream.write_all(&[0x20, 0x02, 0x00, 0x00]).await.unwrap();

        // Read and ACK publishes until stream closes
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if buf[0] >> 4 == 3 {
                        // PUBLISH with QoS 1: extract packet ID and send PUBACK
                        let qos = (buf[0] >> 1) & 0x03;
                        if qos == 1 {
                            let topic_len =
                                u16::from_be_bytes([buf[2], buf[3]]) as usize;
                            let pkt_id_offset = 2 + 2 + topic_len;
                            if pkt_id_offset + 1 < n {
                                let pkt_id = [buf[pkt_id_offset], buf[pkt_id_offset + 1]];
                                stream
                                    .write_all(&[0x40, 0x02, pkt_id[0], pkt_id[1]])
                                    .await
                                    .ok();
                            }
                        }
                    }
                }
            }
        }
    }

    fn make_discovery_event() -> AdapterEvent {
        let mut params = BTreeMap::new();
        params.insert("address".into(), "0x60".into());
        AdapterEvent::DeviceDiscovered {
            device_key: DeviceKey::new("i2c:0x60:mcp9600"),
            identity: SensorIdentity {
                manufacturer: "Microchip".into(),
                ic_part_number: "MCP9600".into(),
                sensor_type: SensorType::Temperature,
                connection: ConnectionInfo {
                    kind: ConnectionKind::I2c,
                    parameters: params,
                },
            },
        }
    }

    #[tokio::test]
    async fn disconnect_drops_telemetry_event() {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (_conn_tx, conn_rx) = watch::channel(ConnectionState::Disconnected);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(fake_broker(listener));

        let mut opts =
            rumqttc::MqttOptions::new("test-drop", addr.ip().to_string(), addr.port());
        opts.set_keep_alive(std::time::Duration::from_secs(5));
        let (client, _eventloop) = rumqttc::AsyncClient::new(opts, 10);

        let aid = AdapterId::new("test");
        let sid = "a".repeat(32);

        let join = tokio::spawn(publish_run(event_rx, client, conn_rx, aid, sid));

        // Send telemetry while disconnected
        let reading =
            SensorReading::new(SensorType::Temperature, vec![25.0], vec!["celsius".into()]);
        event_tx
            .send(AdapterEvent::SensorData {
                device_key: DeviceKey::new("test"),
                reading,
                rssi: None,
                battery_pct: None,
                ingested_at: std::time::SystemTime::now(),
            })
            .await
            .unwrap();

        // Close channel to exit publish_run
        drop(event_tx);
        let result = join.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn inventory_tracked_while_disconnected() {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (_conn_tx, conn_rx) = watch::channel(ConnectionState::Disconnected);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut opts =
            rumqttc::MqttOptions::new("test-inv", addr.ip().to_string(), addr.port());
        opts.set_keep_alive(std::time::Duration::from_secs(5));
        let (client, _eventloop) = rumqttc::AsyncClient::new(opts, 10);

        let aid = AdapterId::new("test");
        let sid = "b".repeat(32);

        let join = tokio::spawn(publish_run(event_rx, client, conn_rx, aid, sid));

        // Send discovery while disconnected
        event_tx.send(make_discovery_event()).await.unwrap();

        // Close to exit
        drop(event_tx);
        let result = join.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn watch_sender_drop_returns_error() {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (conn_tx, conn_rx) = watch::channel(ConnectionState::Disconnected);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut opts = rumqttc::MqttOptions::new(
            "test-watch-drop",
            addr.ip().to_string(),
            addr.port(),
        );
        opts.set_keep_alive(std::time::Duration::from_secs(5));
        let (client, _eventloop) = rumqttc::AsyncClient::new(opts, 10);

        let aid = AdapterId::new("test");
        let sid = "c".repeat(32);

        let join = tokio::spawn(publish_run(event_rx, client, conn_rx, aid, sid));

        // Drop conn_tx to simulate eventloop death
        drop(conn_tx);
        // Keep event_tx alive so the exit is from conn_rx, not event_rx
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        drop(event_tx);

        let result = join.await.unwrap();
        assert!(result.is_err(), "should return Err when watch sender dropped");
        assert!(
            matches!(result.unwrap_err(), PublishError::WatchSenderDropped),
            "should be WatchSenderDropped variant"
        );
    }

    #[tokio::test]
    async fn device_lost_then_rediscovered_while_disconnected() {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (_conn_tx, conn_rx) = watch::channel(ConnectionState::Disconnected);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut opts = rumqttc::MqttOptions::new(
            "test-lost-redis",
            addr.ip().to_string(),
            addr.port(),
        );
        opts.set_keep_alive(std::time::Duration::from_secs(5));
        let (client, _eventloop) = rumqttc::AsyncClient::new(opts, 10);

        let aid = AdapterId::new("test");
        let sid = "d".repeat(32);

        let join = tokio::spawn(publish_run(event_rx, client, conn_rx, aid, sid));

        // Discovery -> Loss -> Rediscovery while disconnected
        event_tx.send(make_discovery_event()).await.unwrap();
        event_tx
            .send(AdapterEvent::DeviceLost {
                device_key: DeviceKey::new("i2c:0x60:mcp9600"),
                reason: "test".into(),
            })
            .await
            .unwrap();
        event_tx.send(make_discovery_event()).await.unwrap();

        drop(event_tx);
        let result = join.await.unwrap();
        assert!(result.is_ok());
    }
}
