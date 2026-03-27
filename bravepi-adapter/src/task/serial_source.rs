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
    pub async fn join(self) -> Result<(), String> {
        tokio::task::spawn_blocking(|| self.thread_handle.join())
            .await
            .map_err(|_| "spawn_blocking failed".to_string())?
            .map_err(|_| "Reader thread panicked".to_string())
    }
}

const MAX_RETRIES: u32 = 10;
const MAX_BACKOFF_SECS: u64 = 30;

/// SerialTransport を開き、reader thread を起動する。
/// reconnect ロジックもこの中に閉じる。
pub(crate) fn start(port_path: &str) -> Result<SerialSource, std::io::Error> {
    let config = serial_config();
    let transport = SerialTransport::open(port_path, &config)
        .map_err(std::io::Error::other)?;
    let (bytes_tx, bytes_rx) = mpsc::channel(64);
    let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(16);
    let owned_path = port_path.to_string();
    let thread_handle = std::thread::Builder::new()
        .name(format!("bravepi-serial-{}", port_path))
        .spawn(move || serial_reader_thread(owned_path, transport, bytes_tx, write_rx))?;
    Ok(SerialSource {
        bytes_rx,
        write_tx,
        handle: SerialSourceHandle { thread_handle },
    })
}

fn serial_reader_thread(
    port_path: String,
    mut transport: SerialTransport,
    bytes_tx: mpsc::Sender<Result<Vec<u8>, TransportError>>,
    mut write_rx: mpsc::Receiver<Vec<u8>>,
) {
    tracing::info!(port = %port_path, "Serial reader thread started");
    let mut buf = [0u8; 4096];
    let timeout = Duration::from_millis(500);
    let mut retry_count: u32 = 0;

    loop {
        if bytes_tx.is_closed() {
            tracing::info!("Bytes channel closed, reader thread exiting");
            return;
        }

        // Drain pending writes before reading.
        // On write error, report via bytes_tx and enter reconnect
        // (a broken port will fail reads too).
        let mut write_failed = false;
        while let Ok(data) = write_rx.try_recv() {
            if let Err(e) = transport.write_all(&data) {
                tracing::error!(error = %e, port = %port_path, "Serial write error");
                let msg = format!("Serial write error on {}: {}", port_path, e);
                let _ = bytes_tx.blocking_send(Err(TransportError { message: msg }));
                write_failed = true;
                break;
            }
        }
        if write_failed {
            // Treat as terminal — same as read error.
            // Drop transport and enter reconnect loop.
            drop(transport);

            loop {
                retry_count += 1;
                if retry_count > MAX_RETRIES {
                    let msg = format!(
                        "Serial write error on {} (max retries {} exceeded)",
                        port_path, MAX_RETRIES
                    );
                    tracing::error!("{}", msg);
                    let _ = bytes_tx.blocking_send(Err(TransportError { message: msg }));
                    return;
                }

                if bytes_tx.is_closed() {
                    tracing::info!("Bytes channel closed during retry, exiting");
                    return;
                }

                let backoff_secs = (1u64 << retry_count.min(5)).min(MAX_BACKOFF_SECS);
                tracing::warn!(
                    port = %port_path,
                    retry = retry_count,
                    backoff_secs = backoff_secs,
                    "Attempting serial reconnect after write failure"
                );
                for _ in 0..backoff_secs {
                    if bytes_tx.is_closed() {
                        tracing::info!("Bytes channel closed during retry, exiting");
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }

                let config = serial_config();
                match SerialTransport::open(&port_path, &config) {
                    Ok(new_transport) => {
                        tracing::info!(port = %port_path, "Serial reconnected after write failure");
                        transport = new_transport;
                        retry_count = 0;
                        break;
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
            continue; // restart main loop with new transport
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

                loop {
                    retry_count += 1;
                    if retry_count > MAX_RETRIES {
                        let msg = format!(
                            "Serial read error on {}: {} (max retries {} exceeded)",
                            port_path, e, MAX_RETRIES
                        );
                        tracing::error!("{}", msg);
                        let _ = bytes_tx.blocking_send(Err(TransportError { message: msg }));
                        return;
                    }

                    if bytes_tx.is_closed() {
                        tracing::info!("Bytes channel closed during retry, exiting");
                        return;
                    }

                    let backoff_secs = (1u64 << retry_count.min(5)).min(MAX_BACKOFF_SECS);
                    tracing::warn!(
                        port = %port_path,
                        retry = retry_count,
                        backoff_secs = backoff_secs,
                        "Attempting serial reconnect"
                    );
                    for _ in 0..backoff_secs {
                        if bytes_tx.is_closed() {
                            tracing::info!("Bytes channel closed during retry, exiting");
                            return;
                        }
                        std::thread::sleep(Duration::from_secs(1));
                    }

                    let config = serial_config();
                    match SerialTransport::open(&port_path, &config) {
                        Ok(new_transport) => {
                            tracing::info!(port = %port_path, "Serial reconnected");
                            transport = new_transport;
                            retry_count = 0;
                            break;
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
        }
    }
}
