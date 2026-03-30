use crate::backoff::Backoff;
use rumqttc::{Event, EventLoop, Incoming};
use tokio::sync::watch;
use tracing::{debug, warn};

/// Connection state communicated from eventloop_task to publish_task via watch channel.
/// Level-triggered: receiver always sees the latest state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionState {
    Disconnected,
    Connected,
}

/// Run the MQTT eventloop. Sends connection state changes via `conn_tx`.
///
/// MUST NOT call `client.publish()` -- all publishes happen in publish_task.
/// rumqttc handles TCP reconnection internally; we add backoff sleep between retries.
///
/// Returns only if aborted. Does not exit on transient errors.
pub(crate) async fn eventloop_run(
    mut eventloop: EventLoop,
    conn_tx: watch::Sender<ConnectionState>,
) {
    let mut backoff = Backoff::new();
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                debug!("ConnAck received");
                backoff.reset();
                let _ = conn_tx.send(ConnectionState::Connected);
            }
            Ok(_event) => {
                // PubAck, PingResp, etc. -- no action needed.
            }
            Err(e) => {
                warn!("eventloop error: {e}");
                let _ = conn_tx.send(ConnectionState::Disconnected);
                let delay = backoff.next_delay();
                debug!(delay_ms = delay.as_millis(), "backoff before next poll");
                tokio::time::sleep(delay).await;
            }
        }
    }
}
