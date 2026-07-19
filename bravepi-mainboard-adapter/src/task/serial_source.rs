//! Serial port source: port open + reader thread + reconnect。
//! event_loop に bytes channel を提供する。

use std::time::Duration;

use rpi4b_transport::SerialTransport;
use tokio::sync::mpsc;

use crate::serial_config;
use crate::transport::{BytesReceiver, BytesSender, TransportError};

pub(crate) struct SerialSource {
    pub bytes_rx: BytesReceiver,
    pub write_tx: BytesSender,
    pub handle: SerialSourceHandle,
}

pub(crate) struct SerialSourceHandle {
    thread_handle: std::thread::JoinHandle<()>,
}

impl SerialSourceHandle {
    #[cfg(test)]
    pub(crate) fn from_thread(thread_handle: std::thread::JoinHandle<()>) -> Self {
        Self { thread_handle }
    }

    pub async fn join(self) -> Result<(), String> {
        tokio::task::spawn_blocking(|| self.thread_handle.join())
            .await
            .map_err(|_| "spawn_blocking failed".to_string())?
            .map_err(|_| "Reader thread panicked".to_string())
    }
}

const MAX_RETRIES: u32 = 10;
const MAX_BACKOFF_SECS: u64 = 30;

/// Reader thread を起動する。
/// port open は thread 内で行い、失敗時は exponential backoff で retry する。
/// start() 自体は thread spawn 失敗時のみ Err を返す。
pub(crate) fn start(port_path: &str) -> Result<SerialSource, std::io::Error> {
    let (bytes_tx, bytes_rx) = mpsc::channel(64);
    let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(16);
    let owned_path = port_path.to_string();
    let thread_handle = std::thread::Builder::new()
        .name(format!("bravepi-serial-{}", port_path))
        .spawn(move || serial_reader_thread(owned_path, bytes_tx, write_rx))?;
    Ok(SerialSource {
        bytes_rx,
        write_tx,
        handle: SerialSourceHandle { thread_handle },
    })
}

/// Initial connection with retry。1回目は即試行、以降は try_reconnect と同じ
/// cancellable backoff。bytes_tx.is_closed() で shutdown 中断可能。
/// 全 retry 失敗時は TransportError を送信して None を返す。
fn connect_initial(
    port_path: &str,
    bytes_tx: &mpsc::Sender<Result<Vec<u8>, TransportError>>,
) -> Option<SerialTransport> {
    let config = serial_config();

    // First attempt without delay.
    match SerialTransport::open(port_path, &config) {
        Ok(t) => return Some(t),
        Err(e) => {
            tracing::warn!(
                error = %e,
                port = %port_path,
                "Initial serial open failed, retrying"
            );
        }
    }

    let mut retry_count: u32 = 0;
    match try_reconnect(port_path, &mut retry_count, bytes_tx) {
        ReconnectResult::Connected(t) => Some(t),
        ReconnectResult::ChannelClosed => None,
        ReconnectResult::RetriesExhausted => {
            let msg = format!("Failed to open {} after {} retries", port_path, MAX_RETRIES);
            tracing::error!("{}", msg);
            let _ = bytes_tx.blocking_send(Err(TransportError { message: msg }));
            None
        }
    }
}

/// Reconnect result: either a new transport, or a terminal condition.
enum ReconnectResult {
    Connected(SerialTransport),
    ChannelClosed,
    RetriesExhausted,
}

/// Shared reconnect logic for both read and write errors.
/// Attempts up to MAX_RETRIES with exponential backoff.
/// Does NOT send Err(TransportError) — caller decides what to do on exhaustion.
fn try_reconnect(
    port_path: &str,
    retry_count: &mut u32,
    bytes_tx: &mpsc::Sender<Result<Vec<u8>, TransportError>>,
) -> ReconnectResult {
    loop {
        *retry_count += 1;
        if *retry_count > MAX_RETRIES {
            return ReconnectResult::RetriesExhausted;
        }

        if bytes_tx.is_closed() {
            tracing::info!("Bytes channel closed during retry, exiting");
            return ReconnectResult::ChannelClosed;
        }

        let backoff_secs = (1u64 << (*retry_count).min(5)).min(MAX_BACKOFF_SECS);
        tracing::warn!(
            port = %port_path,
            retry = *retry_count,
            backoff_secs = backoff_secs,
            "Attempting serial reconnect"
        );
        for _ in 0..backoff_secs {
            if bytes_tx.is_closed() {
                tracing::info!("Bytes channel closed during retry, exiting");
                return ReconnectResult::ChannelClosed;
            }
            std::thread::sleep(Duration::from_secs(1));
        }

        let config = serial_config();
        match SerialTransport::open(port_path, &config) {
            Ok(new_transport) => {
                tracing::info!(port = %port_path, "Serial reconnected");
                *retry_count = 0;
                return ReconnectResult::Connected(new_transport);
            }
            Err(open_err) => {
                tracing::warn!(
                    error = %open_err,
                    port = %port_path,
                    "Reconnect failed"
                );
            }
        }
    }
}

fn serial_reader_thread(
    port_path: String,
    bytes_tx: mpsc::Sender<Result<Vec<u8>, TransportError>>,
    mut write_rx: mpsc::Receiver<Vec<u8>>,
) {
    tracing::info!(port = %port_path, "Serial reader thread started");

    let Some(mut transport) = connect_initial(&port_path, &bytes_tx) else {
        return;
    };

    let mut buf = [0u8; 4096];
    let timeout = Duration::from_millis(500);
    let mut retry_count: u32 = 0;

    loop {
        if bytes_tx.is_closed() {
            tracing::info!("Bytes channel closed, reader thread exiting");
            return;
        }

        // Drain pending writes before reading.
        // On write error, drop transport and enter reconnect.
        // Err(TransportError) is sent only after retry exhaustion,
        // so the adapter stays alive during reconnect attempts.
        //
        // NOTE: the failed write and any remaining queued writes are lost.
        // Individual command success/failure tracking requires an orchestrator
        // layer with request_id / timeout / retry — out of scope for Sub-project D.
        let mut write_failed = false;
        while let Ok(data) = write_rx.try_recv() {
            if let Err(e) = transport.write_all(&data) {
                tracing::error!(
                    error = %e,
                    port = %port_path,
                    bytes = data.len(),
                    "Serial write error — command bytes lost, entering reconnect"
                );
                write_failed = true;
                break;
            }
        }
        if write_failed {
            // Drain and discard remaining queued writes — stale after reconnect.
            let mut discarded = 0usize;
            while write_rx.try_recv().is_ok() {
                discarded += 1;
            }
            if discarded > 0 {
                tracing::warn!(
                    count = discarded,
                    "Discarded queued downlink writes due to transport failure"
                );
            }
            drop(transport);
            match try_reconnect(&port_path, &mut retry_count, &bytes_tx) {
                ReconnectResult::Connected(new_transport) => {
                    transport = new_transport;
                    continue;
                }
                ReconnectResult::ChannelClosed => return,
                ReconnectResult::RetriesExhausted => {
                    let msg = format!(
                        "Serial write error on {} (max retries {} exceeded)",
                        port_path, MAX_RETRIES
                    );
                    tracing::error!("{}", msg);
                    let _ = bytes_tx.blocking_send(Err(TransportError { message: msg }));
                    return;
                }
            }
        }

        match transport.read(&mut buf, timeout) {
            Ok(0) => continue,
            Ok(n) => {
                retry_count = 0;
                if bytes_tx.blocking_send(Ok(buf[..n].to_vec())).is_err() {
                    tracing::info!("Bytes channel closed, reader thread exiting");
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => {
                tracing::error!(error = %e, port = %port_path, "Serial read error");
                drop(transport);
                match try_reconnect(&port_path, &mut retry_count, &bytes_tx) {
                    ReconnectResult::Connected(new_transport) => {
                        transport = new_transport;
                        // continue to main loop
                    }
                    ReconnectResult::ChannelClosed => return,
                    ReconnectResult::RetriesExhausted => {
                        let msg = format!(
                            "Serial read error on {}: {} (max retries {} exceeded)",
                            port_path, e, MAX_RETRIES
                        );
                        tracing::error!("{}", msg);
                        let _ = bytes_tx.blocking_send(Err(TransportError { message: msg }));
                        return;
                    }
                }
            }
        }
    }
}
