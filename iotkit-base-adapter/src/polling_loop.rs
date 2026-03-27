use std::sync::Arc;
use tokio::sync::mpsc;

use iotkit_core_types::{AdapterCommand, AdapterEvent};

use crate::SensorDriver;

/// Stub polling loop. Will be fleshed out in a later task.
pub(crate) async fn polling_loop(
    _bus_path: String,
    _targets: Vec<(u8, Arc<dyn SensorDriver>, Option<String>)>,
    _poll_interval_ms: u64,
    _event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
) {
    // Wait for shutdown or channel close.
    while let Some(cmd) = command_rx.recv().await {
        if matches!(cmd, AdapterCommand::Shutdown) {
            break;
        }
    }
}
