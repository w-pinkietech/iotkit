use std::{
    fs,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use iotkit_edge_custody_contract::RecoveryActivationRequest;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};
use tokio_util::sync::CancellationToken;

use crate::application::recovery::{
    BackupInspection, BrokerFenceReceipt, RecoveryApplicationError, RecoveryHandoff,
    RecoveryReport, RecoveryService, RestoreReceipt,
};

pub const DEFAULT_RECOVERY_CONTROL_SOCKET: &str = "/data/recovery-control.sock";
const MAX_FRAME_BYTES: u64 = 1_048_576;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryControlRequest {
    Prepare {
        inspection: BackupInspection,
        fence: BrokerFenceReceipt,
    },
    Authorize {
        receipt: RestoreReceipt,
    },
    Report {
        recovery_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryControlResponse {
    Prepared { handoff: RecoveryHandoff },
    Authorized { request: RecoveryActivationRequest },
    Report { report: RecoveryReport },
    Rejected { code: String },
}

pub async fn run_recovery_control(
    socket_path: PathBuf,
    service: RecoveryService,
    cancellation: CancellationToken,
) -> Result<(), RecoveryControlError> {
    let listener = bind(&socket_path)?;
    let _cleanup = SocketCleanup(socket_path);
    loop {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                tokio::select! {
                    () = cancellation.cancelled() => return Ok(()),
                    outcome = tokio::time::timeout(
                        Duration::from_secs(5),
                        serve_connection(stream, &service),
                    ) => {
                        match outcome {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                tracing::warn!(%error, "recovery control client failed");
                            }
                            Err(_) => {
                                tracing::warn!("recovery control client timed out");
                            }
                        }
                    }
                }
            }
        }
    }
}

pub async fn call_recovery_control(
    socket_path: &Path,
    request: &RecoveryControlRequest,
) -> Result<RecoveryControlResponse, RecoveryControlError> {
    tokio::time::timeout(
        Duration::from_secs(6),
        call_recovery_control_inner(socket_path, request),
    )
    .await
    .map_err(|_| RecoveryControlError::Timeout)?
}

async fn call_recovery_control_inner(
    socket_path: &Path,
    request: &RecoveryControlRequest,
) -> Result<RecoveryControlResponse, RecoveryControlError> {
    let mut stream = UnixStream::connect(socket_path).await?;
    let encoded = serde_json::to_vec(request)?;
    if encoded.len() as u64 > MAX_FRAME_BYTES {
        return Err(RecoveryControlError::FrameTooLarge);
    }
    stream.write_all(&encoded).await?;
    stream.shutdown().await?;
    let mut response = Vec::new();
    (&mut stream)
        .take(MAX_FRAME_BYTES + 1)
        .read_to_end(&mut response)
        .await?;
    if response.len() as u64 > MAX_FRAME_BYTES {
        return Err(RecoveryControlError::FrameTooLarge);
    }
    Ok(serde_json::from_slice(&response)?)
}

async fn serve_connection(
    mut stream: UnixStream,
    service: &RecoveryService,
) -> Result<(), RecoveryControlError> {
    let mut request = Vec::new();
    (&mut stream)
        .take(MAX_FRAME_BYTES + 1)
        .read_to_end(&mut request)
        .await?;
    let response = if request.len() as u64 > MAX_FRAME_BYTES {
        RecoveryControlResponse::Rejected {
            code: "request_too_large".into(),
        }
    } else {
        match serde_json::from_slice::<RecoveryControlRequest>(&request) {
            Ok(RecoveryControlRequest::Prepare { inspection, fence }) => match unix_millis() {
                Ok(now) => match service.prepare(&inspection, &fence, now).await {
                    Ok(handoff) => RecoveryControlResponse::Prepared { handoff },
                    Err(error) => rejected(error),
                },
                Err(_) => RecoveryControlResponse::Rejected {
                    code: "clock_unavailable".into(),
                },
            },
            Ok(RecoveryControlRequest::Authorize { receipt }) => match unix_millis() {
                Ok(now) => match service.authorize(&receipt, now).await {
                    Ok(request) => RecoveryControlResponse::Authorized { request },
                    Err(error) => rejected(error),
                },
                Err(_) => RecoveryControlResponse::Rejected {
                    code: "clock_unavailable".into(),
                },
            },
            Ok(RecoveryControlRequest::Report { recovery_id }) => {
                match service.report(&recovery_id).await {
                    Ok(report) => RecoveryControlResponse::Report { report },
                    Err(error) => rejected(error),
                }
            }
            Err(_) => RecoveryControlResponse::Rejected {
                code: "invalid_request".into(),
            },
        }
    };
    let encoded = serde_json::to_vec(&response)?;
    stream.write_all(&encoded).await?;
    stream.shutdown().await?;
    Ok(())
}

fn rejected(error: RecoveryApplicationError) -> RecoveryControlResponse {
    match error {
        RecoveryApplicationError::InvalidEvidence => RecoveryControlResponse::Rejected {
            code: "recovery_conflict".into(),
        },
        RecoveryApplicationError::Storage(_) => {
            tracing::error!("recovery control storage operation failed");
            RecoveryControlResponse::Rejected {
                code: "storage_unavailable".into(),
            }
        }
    }
}

fn bind(path: &Path) -> Result<UnixListener, RecoveryControlError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_socket() {
            return Err(RecoveryControlError::UnsafeSocketPath);
        }
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => return Err(RecoveryControlError::SocketInUse),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                fs::remove_file(path)?;
            }
            Err(error) => return Err(RecoveryControlError::Io(error)),
        }
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn unix_millis() -> Result<i64, RecoveryControlError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RecoveryControlError::Clock)?
        .as_millis();
    i64::try_from(millis).map_err(|_| RecoveryControlError::Clock)
}

struct SocketCleanup(PathBuf);

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RecoveryControlError {
    #[error("recovery control I/O failed")]
    Io(#[from] std::io::Error),
    #[error("recovery control message is invalid")]
    Json(#[from] serde_json::Error),
    #[error("recovery control message is too large")]
    FrameTooLarge,
    #[error("recovery control socket path is not a socket")]
    UnsafeSocketPath,
    #[error("recovery control socket is already in use")]
    SocketInUse,
    #[error("system clock is unavailable")]
    Clock,
    #[error("recovery control request timed out")]
    Timeout,
}
