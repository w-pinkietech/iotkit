use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use iotkit_ingest_client::channel_for_test;
use iotkit_input_adapter_host_api::{AdapterInstanceId, ConfiguredSource};

impl super::RuntimeWorker {
    fn from_test_parts(
        event_rx: mpsc::Receiver<RuntimeEvent>,
        command_tx: mpsc::Sender<RuntimeCommand>,
        runtime_handle: tokio::task::JoinHandle<()>,
        reader_handle: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            event_rx,
            command_tx,
            source_handle: Some(SerialSourceHandle {
                thread_handle: reader_handle,
            }),
            runtime_handle: Some(runtime_handle),
        }
    }
}

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
        super::RuntimeWorker::from_test_parts(event_rx, command_tx, runtime_handle, reader_handle),
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
