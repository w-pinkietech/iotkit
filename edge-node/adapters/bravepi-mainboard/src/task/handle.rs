//! AdapterHandle: adapter の起動とライフサイクル管理。

use iotkit_core_supervision::{AdapterCommand, AdapterEvent};
use iotkit_core_types::AdapterId;
use iotkit_input_adapter_host_api::{
    AdapterCompletion, AdapterDiagnostic, AdapterStartContext, DiagnosticKind,
    InputAdapterTypeDescriptor, PhysicalTransportKind, QueueSubmitError, RunningInputAdapter,
    UnexpectedExitReason, runtime_channels,
};
use tokio::sync::mpsc;

use super::event_loop::{RuntimeCommand, RuntimeEvent, decoded_event_loop};
use super::legacy_projection::event_loop;
use super::serial_source::{self, SerialSourceHandle};

/// adapter 起動結果。core はこの handle を使って adapter と通信する。
pub struct AdapterHandle {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
    source_handle: Option<SerialSourceHandle>,
    event_loop_handle: Option<tokio::task::JoinHandle<()>>,
}

impl AdapterHandle {
    /// シャットダウン: event_rx close → Shutdown cmd → event_loop join → reader thread join。
    pub async fn shutdown(mut self) -> Result<(), String> {
        // 1. event_rx を close → event_loop の send() が Err で抜ける (buffer 詰まり対策)
        self.event_rx.close();

        // 2. Shutdown コマンド送信 → event_loop が select で観測して return
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;

        // 3. event_loop の完了を待つ
        let mut errors = Vec::new();
        if let Some(handle) = self.event_loop_handle.take()
            && let Err(error) = handle.await
        {
            errors.push(format!("event_loop panicked: {error}"));
        }

        // 4. reader thread の join
        //    event_loop 終了 → bytes_rx drop → bytes_tx.is_closed() = true
        //    → reader thread が次の is_closed() チェックで終了
        if let Some(source) = self.source_handle.take()
            && let Err(error) = source.join().await
        {
            errors.push(error);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// Parts returned by [`AdapterHandle::into_parts`].
pub struct AdapterParts {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub shutdown: ShutdownHandle,
}

/// Opaque handle for shutting down the BravePI adapter.
///
/// Does NOT close the event receiver — that is the caller's responsibility.
/// ShutdownHandle sends `Shutdown` and joins both the event loop task and
/// the reader thread.
pub struct ShutdownHandle {
    command_tx: mpsc::Sender<AdapterCommand>,
    source_handle: Option<SerialSourceHandle>,
    event_loop_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ShutdownHandle {
    pub async fn shutdown(mut self) -> Result<(), String> {
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        let mut errors = Vec::new();
        if let Some(handle) = self.event_loop_handle.take()
            && let Err(error) = handle.await
        {
            errors.push(format!("event_loop panicked: {error}"));
        }
        if let Some(source) = self.source_handle.take()
            && let Err(error) = source.join().await
        {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

impl AdapterHandle {
    /// Decompose this handle into parts for use with an adapter host.
    ///
    /// The existing [`AdapterHandle::shutdown`] method remains available
    /// for direct use — `into_parts` is an additive API.
    pub fn into_parts(self) -> AdapterParts {
        AdapterParts {
            id: self.id,
            event_rx: self.event_rx,
            shutdown: ShutdownHandle {
                command_tx: self.command_tx,
                source_handle: self.source_handle,
                event_loop_handle: self.event_loop_handle,
            },
        }
    }
}

/// BravePI adapter を起動する。
///
/// 戻り値の `AdapterHandle` 経由で event を受信し、command を送信する。
/// serial read は専用スレッド、フレーム処理は tokio task で動作する。
///
/// Tokio runtime 上で呼び出す必要がある。runtime が無い場合は `Err` を返す
/// (panic しない)。
pub fn start(
    port_path: String,
    ingest: Option<iotkit_ingest_client::IngestClient>,
) -> Result<AdapterHandle, std::io::Error> {
    let runtime_handle = tokio::runtime::Handle::try_current().map_err(std::io::Error::other)?;

    let source = serial_source::start(&port_path)?;

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(256);
    let (command_tx, command_rx) = mpsc::channel::<AdapterCommand>(32);
    let id = AdapterId::new(format!("bravepi-mainboard:{}", port_path));
    let adapter_id = id.as_str().to_string();

    let write_tx = source.write_tx;
    let event_loop_handle = runtime_handle.spawn(event_loop(
        adapter_id,
        port_path,
        source.bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        ingest,
    ));

    Ok(AdapterHandle {
        id,
        event_rx,
        command_tx,
        source_handle: Some(source.handle),
        event_loop_handle: Some(event_loop_handle),
    })
}

pub fn descriptor() -> InputAdapterTypeDescriptor {
    InputAdapterTypeDescriptor {
        adapter_type_id: iotkit_input_adapter_host_api::AdapterTypeId::new("bravepi-mainboard")
            .expect("static adapter type id"),
        adapter_api_major: 1,
        config_schema_version: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        display_name: "BravePI Mainboard",
        physical_transport_kind: PhysicalTransportKind::Serial,
    }
}

struct RuntimeWorker {
    event_rx: mpsc::Receiver<RuntimeEvent>,
    command_tx: mpsc::Sender<RuntimeCommand>,
    source_handle: Option<SerialSourceHandle>,
    runtime_handle: Option<tokio::task::JoinHandle<()>>,
}

impl RuntimeWorker {
    fn start(port_path: &str) -> Result<Self, std::io::Error> {
        let runtime_handle =
            tokio::runtime::Handle::try_current().map_err(std::io::Error::other)?;
        let source = serial_source::start(port_path)?;
        let (event_tx, event_rx) = mpsc::channel(256);
        let (command_tx, command_rx) = mpsc::channel(32);
        let decoded_handle = runtime_handle.spawn(decoded_event_loop(
            port_path.to_owned(),
            source.bytes_rx,
            event_tx,
            command_rx,
            source.write_tx,
        ));
        Ok(Self {
            event_rx,
            command_tx,
            source_handle: Some(source.handle),
            runtime_handle: Some(decoded_handle),
        })
    }

    fn into_parts(self) -> (mpsc::Receiver<RuntimeEvent>, RuntimeShutdown) {
        (
            self.event_rx,
            RuntimeShutdown {
                command_tx: self.command_tx,
                source_handle: self.source_handle,
                runtime_handle: self.runtime_handle,
            },
        )
    }
}

struct RuntimeShutdown {
    command_tx: mpsc::Sender<RuntimeCommand>,
    source_handle: Option<SerialSourceHandle>,
    runtime_handle: Option<tokio::task::JoinHandle<()>>,
}

impl RuntimeShutdown {
    async fn shutdown(mut self) -> Result<(), String> {
        let _ = self.command_tx.send(RuntimeCommand::Shutdown).await;
        let mut errors = Vec::new();
        if let Some(handle) = self.runtime_handle.take()
            && let Err(error) = handle.await
        {
            errors.push(format!("decoded runtime panicked: {error}"));
        }
        if let Some(source) = self.source_handle.take()
            && let Err(error) = source.join().await
        {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// Start the official northbound host integration.
///
/// The existing `start` path remains for the separate frozen care projection;
/// this wrapper is the only path used by the generic Edge Node input host.
pub fn start_host(
    context: AdapterStartContext,
    port_path: String,
) -> Result<RunningInputAdapter, std::io::Error> {
    let worker = RuntimeWorker::start(&port_path)?;
    Ok(start_host_worker(context, worker))
}

fn start_host_worker(context: AdapterStartContext, worker: RuntimeWorker) -> RunningInputAdapter {
    let instance_id = context.instance_id.clone();
    let (runtime, running) = runtime_channels(instance_id, 64);
    let (mut event_rx, shutdown) = worker.into_parts();
    tokio::spawn(async move {
        let iotkit_input_adapter_host_api::AdapterRuntimeEndpoint {
            activity,
            diagnostics,
            completion,
            mut stop,
        } = runtime;
        let composition_handle = tokio::spawn(async move {
            'run: loop {
                tokio::select! {
                    requested = stop.changed() => {
                        if requested {
                            break AdapterCompletion::RequestedStop;
                        }
                    }
                    event = event_rx.recv() => match event {
                        Some(RuntimeEvent::Observation(observation)) => {
                            activity.physical_decode();
                            match super::ingest_map::to_items(
                                &observation.device_key,
                                &observation.reading,
                                observation.rssi,
                                observation.battery_pct,
                            ) {
                                Some(items) => {
                                    for chunk in
                                        items.chunks(super::ingest_map::MAX_ITEMS_PER_ENVELOPE)
                                    {
                                        match context.ingest.try_submit(chunk.to_vec()) {
                                            Ok(_enqueued) => activity.queue_admission(),
                                            Err(QueueSubmitError::Full(_)) => {
                                                let _ = diagnostics.try_emit(
                                                    AdapterDiagnostic::new(
                                                        DiagnosticKind::ClientQueueFull,
                                                        "ingest queue is full",
                                                    ),
                                                );
                                            }
                                            Err(QueueSubmitError::Closed(_)) => {
                                                let _ = diagnostics.try_emit(
                                                    AdapterDiagnostic::new(
                                                        DiagnosticKind::ClientClosed,
                                                        "ingest client is closed",
                                                    ),
                                                );
                                                break 'run AdapterCompletion::UnexpectedExit(
                                                    UnexpectedExitReason::ClientClosed,
                                                );
                                            }
                                        }
                                    }
                                }
                                None => {
                                    let _ = diagnostics.try_emit(AdapterDiagnostic::new(
                                        DiagnosticKind::MeasurementMapping,
                                        "observation has no declared measurement mapping",
                                    ));
                                }
                            }
                        }
                        Some(RuntimeEvent::Error { error, .. }) => {
                            let _ = diagnostics.try_emit(AdapterDiagnostic::new(
                                DiagnosticKind::Transport,
                                error,
                            ));
                        }
                        Some(RuntimeEvent::DeviceDiscovered { .. })
                        | Some(RuntimeEvent::DeviceConfig { .. }) => {}
                        None => break AdapterCompletion::UnexpectedExit(
                            UnexpectedExitReason::WorkerReturned,
                        ),
                    }
                }
            }
        });
        let outcome = match composition_handle.await {
            Ok(outcome) => outcome,
            Err(_) => AdapterCompletion::Panic,
        };
        let outcome = if shutdown.shutdown().await.is_err() {
            AdapterCompletion::Panic
        } else {
            outcome
        };
        completion.complete(outcome);
    });
    running
}

#[cfg(test)]
#[path = "../../tests/unit/task/handle_tests.rs"]
mod tests;
