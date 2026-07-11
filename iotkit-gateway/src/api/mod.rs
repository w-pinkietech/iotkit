use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum_server::tls_rustls::RustlsConfig;
use iotkit_core_ops::{ClockTrust, OwnershipState, ownership_state};
use iotkit_core_storage::DbHandle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::config::ApiConfig;
use crate::health::{ApiHealth, HealthState};

pub mod auth_layer;
pub mod guard;
pub mod routes;
pub mod tls;

pub struct ApiHandle {
    pub local_addr: SocketAddr,
    pub fingerprint: String,
    pub shutdown: oneshot::Sender<()>,
    pub join: JoinHandle<()>,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    Tls(#[from] tls::TlsError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Storage(#[from] iotkit_core_storage::StorageError),
    #[error("server did not report a local address")]
    NoLocalAddr,
    #[error("control-plane network exposure is blocked: {0}")]
    NotReady(&'static str),
}

pub async fn spawn_api_task(
    db: DbHandle,
    health: Arc<Mutex<HealthState>>,
    cfg: ApiConfig,
    epoch: String,
    data_dir: PathBuf,
    clock_trust: Arc<ClockTrust>,
) -> Result<ApiHandle, ApiError> {
    let ownership = db
        .with_conn(|conn| {
            ownership_state(conn).map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
        })
        .await?;
    match ownership {
        OwnershipState::Owned => {}
        OwnershipState::Unowned => return Err(ApiError::NotReady("unowned")),
        OwnershipState::LocalRecoveryRequired => {
            return Err(ApiError::NotReady("local_recovery_required"));
        }
    }
    if data_dir.join("restore-in-progress").exists() {
        return Err(ApiError::NotReady("restore_in_progress"));
    }
    if data_dir.join("reset-in-progress").exists() {
        return Err(ApiError::NotReady("reset_in_progress"));
    }
    let material = db
        .with_conn(move |conn| {
            tls::ensure_tls_material(conn, &data_dir).map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
        })
        .await?;
    let rustls_config =
        RustlsConfig::from_pem_file(&material.cert_pem_path, &material.key_pem_path).await?;
    let server_handle = axum_server::Handle::new();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let state = routes::AppState {
        db,
        health: health.clone(),
        cfg: cfg.clone(),
        epoch,
        fingerprint: material.fingerprint.clone(),
        throttle: Arc::new(guard::Throttle::default()),
        clock_trust,
    };
    let checkpoint_db = state.db.clone();
    let checkpoint_clock = state.clock_trust.clone();
    let app = routes::router(state).into_make_service_with_connect_info::<SocketAddr>();
    let server = axum_server::bind_rustls(cfg.bind, rustls_config).handle(server_handle.clone());
    let shutdown_handle = server_handle.clone();
    let health_for_cleanup = health.clone();
    let join = tokio::spawn(async move {
        tokio::spawn(async move {
            let _ = shutdown_rx.await;
            shutdown_handle.graceful_shutdown(Some(Duration::from_secs(5)));
        });
        let checkpoint_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let clock = checkpoint_clock.clone();
                let result = checkpoint_db
                    .with_conn(move |conn| {
                        let tx = rusqlite::Transaction::new_unchecked(
                            conn,
                            rusqlite::TransactionBehavior::Immediate,
                        )
                        .map_err(iotkit_core_storage::StorageError::from)?;
                        let checkpoint = clock.checkpoint_if_due(&tx);
                        if checkpoint.is_ok() {
                            tx.commit()
                                .map_err(iotkit_core_storage::StorageError::from)?;
                        }
                        Ok(checkpoint)
                    })
                    .await;
                if let Err(error) = result {
                    tracing::error!(error = %error, "clock checkpoint storage failure");
                } else if let Ok(Err(error)) = result {
                    tracing::warn!(error = %error, "clock checkpoint skipped");
                }
            }
        });
        if let Err(e) = server.serve(app).await {
            tracing::error!(error = %e, "api server exited with error");
        }
        checkpoint_task.abort();
        health_for_cleanup
            .lock()
            .expect("health state mutex poisoned")
            .api = None;
    });
    let local_addr = server_handle
        .listening()
        .await
        .ok_or(ApiError::NoLocalAddr)?;
    {
        let mut health = health.lock().expect("health state mutex poisoned");
        health.api = Some(ApiHealth {
            bind: local_addr.to_string(),
            tls_fingerprint: material.fingerprint.clone(),
        });
    }

    Ok(ApiHandle {
        local_addr,
        fingerprint: material.fingerprint,
        shutdown: shutdown_tx,
        join,
    })
}
