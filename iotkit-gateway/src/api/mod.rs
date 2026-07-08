use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum_server::tls_rustls::RustlsConfig;
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
    #[error("server did not report a local address")]
    NoLocalAddr,
}

pub async fn spawn_api_task(
    db: DbHandle,
    health: Arc<Mutex<HealthState>>,
    cfg: ApiConfig,
    epoch: String,
    data_dir: PathBuf,
) -> Result<ApiHandle, ApiError> {
    let material = tls::ensure_tls_material(&data_dir)?;
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
    };
    let app = routes::router(state).into_make_service_with_connect_info::<SocketAddr>();
    let server = axum_server::bind_rustls(cfg.bind, rustls_config).handle(server_handle.clone());
    let shutdown_handle = server_handle.clone();
    let join = tokio::spawn(async move {
        tokio::spawn(async move {
            let _ = shutdown_rx.await;
            shutdown_handle.graceful_shutdown(Some(Duration::from_secs(5)));
        });
        if let Err(e) = server.serve(app).await {
            tracing::error!(error = %e, "api server exited with error");
        }
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
