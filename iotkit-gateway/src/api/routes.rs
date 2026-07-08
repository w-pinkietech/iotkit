use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use iotkit_core_ops::{
    NewOperatorToken, SetOutcome, Tier, TokenKind, hash_passphrase, is_setup_mode, issue_token,
    load_passphrase_hash, set_passphrase_with_hash, verify_passphrase,
};
use iotkit_core_storage::{DbHandle, StorageError};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::ApiConfig;
use crate::health::{HealthState, now_ms};

use super::guard::{Throttle, is_private_source};

const SESSION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Clone)]
pub struct AppState {
    pub db: DbHandle,
    pub health: Arc<Mutex<HealthState>>,
    pub cfg: ApiConfig,
    pub epoch: String,
    pub fingerprint: String,
    pub throttle: Arc<Throttle>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/box", get(get_box))
        .route("/api/v1/session", post(post_session))
        .route("/api/v1/setup/passphrase", post(post_setup_passphrase))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .layer(axum::middleware::from_fn(trace_request))
        .layer(axum::middleware::from_fn(private_source_guard))
        .with_state(state)
}

#[derive(Debug)]
pub struct ApiErrorResponse {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiErrorResponse {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "unauthorized")
    }
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": {
                    "code": self.code,
                    "message": self.message,
                }
            })),
        )
            .into_response()
    }
}

#[derive(Deserialize)]
struct PassphraseRequest {
    passphrase: String,
}

async fn get_box(State(state): State<AppState>) -> Result<Json<Value>, ApiErrorResponse> {
    let setup_mode = state
        .db
        .with_conn(|conn| is_setup_mode(conn).map_err(storage_other))
        .await
        .map_err(internal_error)?;
    let health = state.health.lock().expect("health state mutex poisoned");
    let adapters_alive = health
        .adapters
        .iter()
        .filter(|adapter| adapter.alive)
        .count();
    let status = if health.collector_alive && health.adapters.iter().all(|adapter| adapter.alive) {
        "ok"
    } else {
        "degraded"
    };

    Ok(Json(json!({
        "gateway_name": state.cfg.gateway_name,
        "epoch": state.epoch,
        "version": env!("CARGO_PKG_VERSION"),
        "setup_mode": setup_mode,
        "tls_fingerprint": state.fingerprint,
        "health_summary": {
            "status": status,
            "adapters_alive": adapters_alive,
        },
    })))
}

async fn post_setup_passphrase(
    State(state): State<AppState>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    Json(payload): Json<PassphraseRequest>,
) -> Result<Json<Value>, ApiErrorResponse> {
    let source_ip = source.ip();
    state
        .throttle
        .check_and_record_source(source_ip)
        .map_err(|retry| {
            ApiErrorResponse::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                format!("retry after {} seconds", retry.duration.as_secs().max(1)),
            )
        })?;

    let setup_mode = state
        .db
        .with_conn(|conn| is_setup_mode(conn).map_err(storage_other))
        .await
        .map_err(internal_error)?;
    if !setup_mode {
        state.throttle.record_failure(source_ip);
        return Err(passphrase_already_set());
    }

    let passphrase = payload.passphrase;
    let phc = tokio::task::spawn_blocking(move || hash_passphrase(&passphrase))
        .await
        .map_err(|e| internal_error(format!("passphrase hashing task failed: {e}")))?
        .map_err(internal_error)?;

    let source_string = source_ip.to_string();
    let issued = state
        .db
        .with_conn(move |conn| {
            match set_passphrase_with_hash(conn, &phc, "setup_mode").map_err(storage_other)? {
                SetOutcome::FirstSet => {}
                SetOutcome::AlreadySet => {
                    return Ok(None);
                }
            }
            issue_session_token(conn, "setup", "setup_mode", Some(&source_string))
                .map(Some)
                .map_err(storage_other)
        })
        .await
        .map_err(internal_error)?;

    let Some(issued) = issued else {
        state.throttle.record_failure(source_ip);
        return Err(passphrase_already_set());
    };

    state.throttle.record_success(source_ip);

    Ok(Json(token_response(issued)))
}

async fn post_session(
    State(state): State<AppState>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    Json(payload): Json<PassphraseRequest>,
) -> Result<Json<Value>, ApiErrorResponse> {
    let source_ip = source.ip();
    let setup_mode = state
        .db
        .with_conn(|conn| is_setup_mode(conn).map_err(storage_other))
        .await
        .map_err(internal_error)?;
    if setup_mode {
        return Err(ApiErrorResponse::new(
            StatusCode::CONFLICT,
            "setup_mode",
            "admin passphrase is not set",
        ));
    }

    state
        .throttle
        .check_and_record_source(source_ip)
        .map_err(|retry| {
            ApiErrorResponse::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                format!("retry after {} seconds", retry.duration.as_secs().max(1)),
            )
        })?;

    let phc = state
        .db
        .with_conn(|conn| load_passphrase_hash(conn).map_err(storage_other))
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            ApiErrorResponse::new(
                StatusCode::CONFLICT,
                "setup_mode",
                "admin passphrase is not set",
            )
        })?;
    let passphrase = payload.passphrase;
    let verified = tokio::task::spawn_blocking(move || verify_passphrase(&phc, &passphrase))
        .await
        .map_err(|e| internal_error(format!("passphrase verification task failed: {e}")))?;

    if !verified {
        state.throttle.record_failure(source_ip);
        record_auth_failed(&state.db, source_ip, "session").await?;
        return Err(ApiErrorResponse::unauthorized());
    }

    state.throttle.record_success(source_ip);
    let source_string = source_ip.to_string();
    let issued = state
        .db
        .with_conn(move |conn| {
            issue_session_token(conn, "session", "self", Some(&source_string))
                .map_err(storage_other)
        })
        .await
        .map_err(internal_error)?;

    Ok(Json(token_response(issued)))
}

fn issue_session_token(
    conn: &rusqlite::Connection,
    name: &str,
    audit_actor: &str,
    audit_source: Option<&str>,
) -> Result<iotkit_core_ops::IssuedToken, iotkit_core_ops::OpsError> {
    issue_token(
        conn,
        &NewOperatorToken {
            name: name.to_string(),
            kind: TokenKind::Human,
            ceiling: Tier::Construction,
            is_session: true,
            expires_at: Some(now_ms() + SESSION_TTL_MS),
        },
        audit_actor,
        audit_source,
    )
}

fn token_response(issued: iotkit_core_ops::IssuedToken) -> Value {
    json!({
        "token_id": issued.token_id,
        "token": issued.plaintext.expose(),
    })
}

async fn record_auth_failed(
    db: &DbHandle,
    source: IpAddr,
    target: &'static str,
) -> Result<(), ApiErrorResponse> {
    db.with_conn(move |conn| {
        iotkit_core_ledger::record_event(
            conn,
            "auth_failed",
            None,
            &json!({
                "source": source.to_string(),
                "target": target,
            })
            .to_string(),
        )
        .map_err(storage_other)
    })
    .await
    .map_err(internal_error)
}

pub async fn private_source_guard(
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if !is_private_source(source.ip()) {
        return ApiErrorResponse::new(StatusCode::FORBIDDEN, "forbidden", "forbidden")
            .into_response();
    }
    next.run(req).await
}

pub async fn trace_request(req: Request<Body>, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let started = Instant::now();
    let response = next.run(req).await;
    tracing::info!(
        method = %method,
        path = %path,
        status = response.status().as_u16(),
        latency_ms = started.elapsed().as_millis() as u64,
        "api request"
    );
    response
}

pub fn retry_after_header(seconds: u64) -> HeaderValue {
    HeaderValue::from_str(&seconds.to_string()).unwrap_or_else(|_| HeaderValue::from_static("1"))
}

fn storage_other<E>(e: E) -> StorageError
where
    E: std::error::Error + Send + Sync + 'static,
{
    StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

fn internal_error<E: std::fmt::Display>(e: E) -> ApiErrorResponse {
    ApiErrorResponse::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        format!("internal error: {e}"),
    )
}

fn passphrase_already_set() -> ApiErrorResponse {
    ApiErrorResponse::new(StatusCode::CONFLICT, "conflict", "passphrase already set")
}
