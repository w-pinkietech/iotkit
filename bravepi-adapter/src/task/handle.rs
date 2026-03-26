//! AdapterHandle: adapter の起動とライフサイクル管理。

use iotkit_core_types::{AdapterCommand, AdapterEvent, AdapterId};
use rpi4b_transport::SerialTransport;
use tokio::sync::mpsc;

use crate::serial_config;
use super::event_loop::event_loop;
use super::reader::serial_reader_thread;

/// adapter 起動結果。core はこの handle を使って adapter と通信する。
pub struct AdapterHandle {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
    reader_thread: Option<std::thread::JoinHandle<()>>,
}

impl AdapterHandle {
    /// シャットダウンコマンドを送信し、reader スレッドの終了を待つ。
    pub async fn shutdown(mut self) -> Result<(), String> {
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.reader_thread.take() {
            tokio::task::spawn_blocking(|| handle.join())
                .await
                .map_err(|_| "spawn_blocking failed".to_string())?
                .map_err(|_| "Reader thread panicked".to_string())?;
        }
        Ok(())
    }
}

/// BravePI adapter を起動する。
///
/// 戻り値の `AdapterHandle` 経由で event を受信し、command を送信する。
/// serial read は専用スレッド、フレーム処理は tokio task で動作する。
pub fn start(port_path: String) -> Result<AdapterHandle, std::io::Error> {
    let config = serial_config();
    let transport = SerialTransport::open(&port_path, &config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(256);
    let (command_tx, command_rx) = mpsc::channel::<AdapterCommand>(32);

    // serial read 用の専用スレッド → async task へ raw bytes (またはエラー) を送る
    let (bytes_tx, bytes_rx) = mpsc::channel::<Result<Vec<u8>, String>>(64);
    let reader_port = port_path.clone();
    let join_handle = std::thread::Builder::new()
        .name(format!("bravepi-serial-{}", port_path))
        .spawn(move || serial_reader_thread(reader_port, transport, bytes_tx))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let id = AdapterId::new(format!("bravepi:{}", port_path));

    tokio::spawn(event_loop(port_path, bytes_rx, event_tx, command_rx));

    Ok(AdapterHandle {
        id,
        event_rx,
        command_tx,
        reader_thread: Some(join_handle),
    })
}
