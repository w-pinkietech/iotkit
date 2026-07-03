//! AdapterHandle: adapter の起動とライフサイクル管理。

use iotkit_core_types::{AdapterCommand, AdapterEvent, AdapterId};
use tokio::sync::mpsc;

use super::event_loop::event_loop;
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
        if let Some(handle) = self.event_loop_handle.take() {
            handle.await.map_err(|e| format!("event_loop panicked: {}", e))?;
        }

        // 4. reader thread の join
        //    event_loop 終了 → bytes_rx drop → bytes_tx.is_closed() = true
        //    → reader thread が次の is_closed() チェックで終了
        if let Some(source) = self.source_handle.take() {
            source.join().await?;
        }

        Ok(())
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
        if let Some(handle) = self.event_loop_handle.take() {
            handle
                .await
                .map_err(|e| format!("event_loop panicked: {e}"))?;
        }
        if let Some(source) = self.source_handle.take() {
            source.join().await?;
        }
        Ok(())
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
    let runtime_handle = tokio::runtime::Handle::try_current()
        .map_err(std::io::Error::other)?;

    let source = serial_source::start(&port_path)?;

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(256);
    let (command_tx, command_rx) = mpsc::channel::<AdapterCommand>(32);
    let id = AdapterId::new(format!("bravepi-mainboard:{}", port_path));

    let write_tx = source.write_tx;
    let event_loop_handle = runtime_handle.spawn(
        event_loop(port_path, source.bytes_rx, event_tx, command_rx, write_tx, ingest)
    );

    Ok(AdapterHandle {
        id,
        event_rx,
        command_tx,
        source_handle: Some(source.handle),
        event_loop_handle: Some(event_loop_handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tokio runtime が無い状態で start() を呼ぶと panic せず Err を返す。
    /// #[tokio::test] ではなく plain #[test] で実行することで runtime 不在を保証する。
    #[test]
    fn start_without_runtime_returns_error() {
        let result = start("/dev/null".to_string(), None);
        assert!(result.is_err(), "start() should return Err without tokio runtime");
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
}
