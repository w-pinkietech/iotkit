//! 専用スレッド: serial port から読んで bytes channel に送る。
//! エラー時は exponential backoff で再接続を試みる。

use rpi4b_transport::SerialTransport;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::serial_config;

pub(crate) const MAX_RETRIES: u32 = 10;
pub(crate) const MAX_BACKOFF_SECS: u64 = 30;

pub(crate) fn serial_reader_thread(
    port_path: String,
    mut transport: SerialTransport,
    bytes_tx: mpsc::Sender<Result<Vec<u8>, String>>,
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

        match transport.read(&mut buf, timeout) {
            Ok(0) => continue,
            Ok(n) => {
                // エラーごとに MAX_RETRIES まで再試行。
                // 読み取り成功でリセットするため、断続的な接続断には永続的に対応する。
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
                        let _ = bytes_tx.blocking_send(Err(msg));
                        return;
                    }

                    if bytes_tx.is_closed() {
                        tracing::info!("Bytes channel closed during retry, exiting");
                        return;
                    }

                    // sleep を 1 秒刻みに分割して Shutdown への応答性を確保
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
