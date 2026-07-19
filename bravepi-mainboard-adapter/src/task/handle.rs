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

    #[cfg(test)]
    fn from_test_parts(
        event_rx: mpsc::Receiver<RuntimeEvent>,
        command_tx: mpsc::Sender<RuntimeCommand>,
        runtime_handle: tokio::task::JoinHandle<()>,
        reader_handle: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            event_rx,
            command_tx,
            source_handle: Some(SerialSourceHandle::from_thread(reader_handle)),
            runtime_handle: Some(runtime_handle),
        }
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
/// this wrapper is the only path used by the generic Edge input host.
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
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use iotkit_ingest_client::channel_for_test;
    use iotkit_input_adapter_host_api::{AdapterInstanceId, ConfiguredSource};

    fn host_context() -> (
        AdapterStartContext,
        iotkit_ingest_client::TestEnvelopeReceiver,
    ) {
        let (client, receiver) = channel_for_test(4);
        let source = ConfiguredSource::new("input:bravepi:test").unwrap();
        (
            AdapterStartContext::new(
                AdapterInstanceId::new("bravepi_test").unwrap(),
                source.clone(),
                iotkit_input_adapter_host_api::SourceBoundIngest::new(source, client),
            ),
            receiver,
        )
    }

    fn test_runtime_worker(
        panic_worker: bool,
        runtime_joined: Arc<AtomicBool>,
        reader_joined: Arc<AtomicBool>,
    ) -> (super::RuntimeWorker, std::sync::mpsc::Sender<()>) {
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, mut command_rx) = mpsc::channel(1);
        let runtime_handle = tokio::spawn(async move {
            struct Joined(Arc<AtomicBool>);
            impl Drop for Joined {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::SeqCst);
                }
            }
            let _joined = Joined(runtime_joined);
            let _event_tx = event_tx;
            if panic_worker {
                panic!("injected decoded runtime panic");
            }
            let _ = command_rx.recv().await;
        });
        let (release_reader, reader_release) = std::sync::mpsc::channel();
        let reader_handle = std::thread::spawn(move || {
            reader_release.recv().unwrap();
            reader_joined.store(true, Ordering::SeqCst);
        });
        (
            super::RuntimeWorker::from_test_parts(
                event_rx,
                command_tx,
                runtime_handle,
                reader_handle,
            ),
            release_reader,
        )
    }

    async fn wait_until_set(flag: &AtomicBool) {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !flag.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("runtime task did not finish");
    }

    /// Tokio runtime が無い状態で start() を呼ぶと panic せず Err を返す。
    /// #[tokio::test] ではなく plain #[test] で実行することで runtime 不在を保証する。
    #[test]
    fn start_without_runtime_returns_error() {
        let result = start("/dev/null".to_string(), None);
        assert!(
            result.is_err(),
            "start() should return Err without tokio runtime"
        );
    }

    #[test]
    fn descriptor_is_vendor_specific_but_uses_generic_host_versioning() {
        let descriptor = descriptor();
        assert_eq!(descriptor.adapter_type_id.as_str(), "bravepi-mainboard");
        assert_eq!(descriptor.adapter_api_major, 1);
        assert_eq!(descriptor.config_schema_version, 1);
    }

    #[tokio::test]
    async fn into_parts_preserves_id_and_channels() {
        use iotkit_core_types::{DeviceKey, SensorReading, SensorType};

        let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(1);
        let (command_tx, mut command_rx) = mpsc::channel::<AdapterCommand>(1);
        let handle = AdapterHandle {
            id: AdapterId::new("test:into-parts"),
            event_rx,
            command_tx,
            source_handle: None,
            event_loop_handle: None,
        };
        let parts = handle.into_parts();

        assert_eq!(parts.id.as_str(), "test:into-parts");

        let mut event_rx = parts.event_rx;
        event_tx
            .send(AdapterEvent::SensorData {
                device_key: DeviceKey::new("test:0"),
                reading: SensorReading::empty(SensorType::Temperature),
                rssi: None,
                battery_pct: None,
                ingested_at: std::time::SystemTime::now(),
            })
            .await
            .unwrap();
        let received = event_rx.recv().await;
        assert!(received.is_some(), "event_rx should receive the sent event");

        parts
            .shutdown
            .shutdown()
            .await
            .expect("shutdown should succeed");
        let cmd = command_rx.recv().await;
        assert!(
            matches!(cmd, Some(AdapterCommand::Shutdown)),
            "shutdown should send Shutdown command"
        );
    }

    #[tokio::test]
    async fn host_stop_joins_runtime_and_reader_before_completion() {
        let runtime_joined = Arc::new(AtomicBool::new(false));
        let reader_joined = Arc::new(AtomicBool::new(false));
        let (worker, release_reader) = test_runtime_worker(
            false,
            Arc::clone(&runtime_joined),
            Arc::clone(&reader_joined),
        );
        let (context, _ingest_rx) = host_context();
        let running = start_host_worker(context, worker);

        running.shutdown.request();
        let mut completion = Box::pin(running.completion.wait());
        wait_until_set(&runtime_joined).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), completion.as_mut())
                .await
                .is_err(),
            "completion resolved before the reader joined"
        );
        release_reader.send(()).unwrap();
        assert_eq!(completion.await, AdapterCompletion::RequestedStop);
        assert!(runtime_joined.load(Ordering::SeqCst));
        assert!(reader_joined.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn runtime_panic_joins_reader_before_panic_completion() {
        let runtime_joined = Arc::new(AtomicBool::new(false));
        let reader_joined = Arc::new(AtomicBool::new(false));
        let (worker, release_reader) = test_runtime_worker(
            true,
            Arc::clone(&runtime_joined),
            Arc::clone(&reader_joined),
        );
        let (context, _ingest_rx) = host_context();
        let running = start_host_worker(context, worker);

        let mut completion = Box::pin(running.completion.wait());
        wait_until_set(&runtime_joined).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), completion.as_mut())
                .await
                .is_err(),
            "panic completion resolved before the reader joined"
        );
        release_reader.send(()).unwrap();
        assert_eq!(completion.await, AdapterCompletion::Panic);
        assert!(runtime_joined.load(Ordering::SeqCst));
        assert!(reader_joined.load(Ordering::SeqCst));
    }
}
