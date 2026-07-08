use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use iotkit_core_ledger as ledger;
use iotkit_core_ops::{
    Actor, DispatchRequest, NewOperatorToken, OpError, SetOutcome, Tier, TokenKind, dispatch,
    hash_passphrase, is_setup_mode, issue_token, load_passphrase_hash, set_passphrase_with_hash,
    standard_catalog, verify_passphrase,
};
use iotkit_core_storage::{DbHandle, StorageError};
use iotkit_core_timeseries::query::{latest_by_series, query_readings_v3};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower_http::timeout::TimeoutLayer;

use crate::config::ApiConfig;
use crate::health::{HealthState, now_ms, render_health_json};

use super::auth_layer::auth_layer;
use super::guard::{RetryAfter, Throttle, is_private_source};

const SESSION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const RESERVED_OP_PARAM_KEYS: &[&str] = &["step_up_passphrase"];
const MIN_PASSPHRASE_LEN: usize = 8;
const READINGS_LIMIT_MAX: u32 = 10_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

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
    let protected = Router::new()
        .route("/api/v1/health", get(get_health))
        .route("/api/v1/series", get(get_series))
        .route("/api/v1/live", get(get_live))
        .route("/api/v1/readings", get(get_readings))
        .route("/api/v1/ops", get(get_ops_catalog))
        .route("/api/v1/ops/{name}", post(post_ops_dispatch))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_layer,
        ));

    Router::new()
        .route("/api/v1/box", get(get_box))
        .route("/api/v1/session", post(post_session))
        .route("/api/v1/setup/passphrase", post(post_setup_passphrase))
        .merge(protected)
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(axum::middleware::from_fn(trace_request))
        .layer(axum::middleware::from_fn(private_source_guard))
        .with_state(state)
}

#[derive(Debug)]
pub struct ApiErrorResponse {
    status: StatusCode,
    code: &'static str,
    message: String,
    retry_after: Option<HeaderValue>,
}

impl ApiErrorResponse {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "unauthorized")
    }

    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after = Some(retry_after_header(seconds));
        self
    }
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        }));
        match self.retry_after {
            Some(retry_after) => {
                (self.status, [(header::RETRY_AFTER, retry_after)], body).into_response()
            }
            None => (self.status, body).into_response(),
        }
    }
}

#[derive(Deserialize)]
struct PassphraseRequest {
    passphrase: String,
}

#[derive(Serialize)]
struct SeriesResponse {
    series_key: String,
    system_id: String,
    user_label: Option<String>,
}

#[derive(Serialize)]
struct LiveResponse {
    series_key: String,
    event_time: i64,
    event_time_source: String,
    quarantined: bool,
    values: Vec<f64>,
}

#[derive(Deserialize)]
struct ReadingsQuery {
    series_key: String,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    limit: Option<u32>,
    include_quarantined: Option<bool>,
}

#[derive(Serialize)]
struct ReadingsResponse {
    series_key: String,
    rows: Vec<ReadingResponse>,
}

#[derive(Serialize)]
struct ReadingResponse {
    seq: i64,
    event_time: i64,
    event_time_source: String,
    quarantined: bool,
    values: Vec<f64>,
}

#[derive(Serialize)]
struct OpCatalogResponse {
    name: &'static str,
    tier: &'static str,
    bulk_escalates: bool,
    params_schema: Value,
}

#[derive(Deserialize)]
struct OpDispatchBody {
    params: serde_json::Map<String, Value>,
    #[serde(default)]
    dry_run: bool,
    step_up_passphrase: Option<String>,
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

async fn get_health(State(state): State<AppState>) -> Result<Response, ApiErrorResponse> {
    let snapshot = state
        .health
        .lock()
        .expect("health state mutex poisoned")
        .clone();
    Ok((
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        render_health_json(&state.epoch, &snapshot),
    )
        .into_response())
}

async fn get_series(
    State(state): State<AppState>,
) -> Result<Json<Vec<SeriesResponse>>, ApiErrorResponse> {
    let rows = state
        .db
        .with_conn(|conn| {
            ledger::list_series(conn)
                .map(|rows| {
                    rows.into_iter()
                        .map(|row| SeriesResponse {
                            series_key: row.series_key,
                            system_id: row.system_id,
                            user_label: row.user_label,
                        })
                        .collect()
                })
                .map_err(storage_other)
        })
        .await
        .map_err(internal_error)?;

    Ok(Json(rows))
}

async fn get_live(
    State(state): State<AppState>,
) -> Result<Json<Vec<LiveResponse>>, ApiErrorResponse> {
    let rows = state
        .db
        .with_conn(|conn| {
            let series = ledger::list_series(conn).map_err(storage_other)?;
            let mut out = Vec::new();
            for series in series {
                if let Some(row) =
                    latest_by_series(conn, series.series_id).map_err(storage_other)?
                {
                    out.push(LiveResponse {
                        series_key: series.series_key,
                        event_time: row.event_time,
                        event_time_source: row.event_time_source,
                        quarantined: row.quarantined,
                        values: row.values,
                    });
                }
            }
            Ok(out)
        })
        .await
        .map_err(internal_error)?;

    Ok(Json(rows))
}

async fn get_readings(
    State(state): State<AppState>,
    Query(query): Query<ReadingsQuery>,
) -> Result<Json<ReadingsResponse>, ApiErrorResponse> {
    let Some(from_ms) = query.from_ms else {
        return Err(ApiErrorResponse::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "missing_range",
            "from_ms and to_ms are required",
        ));
    };
    let Some(to_ms) = query.to_ms else {
        return Err(ApiErrorResponse::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "missing_range",
            "from_ms and to_ms are required",
        ));
    };
    let series_key = query.series_key;
    let limit = query.limit.unwrap_or(100).min(READINGS_LIMIT_MAX);
    let include_quarantined = query.include_quarantined.unwrap_or(false);

    let response = state
        .db
        .with_conn(move |conn| {
            let Some(series_id) = find_series_for_api(conn, &series_key)? else {
                return Ok(None);
            };
            let rows =
                query_readings_v3(conn, series_id, from_ms, to_ms, limit, include_quarantined)
                    .map_err(storage_other)?
                    .into_iter()
                    .map(|row| ReadingResponse {
                        seq: row.seq,
                        event_time: row.event_time,
                        event_time_source: row.event_time_source,
                        quarantined: row.quarantined,
                        values: row.values,
                    })
                    .collect();
            Ok(Some(ReadingsResponse { series_key, rows }))
        })
        .await
        .map_err(internal_error)?;

    response.map(Json).ok_or_else(|| {
        ApiErrorResponse::new(StatusCode::NOT_FOUND, "unknown_series", "unknown series")
    })
}

async fn get_ops_catalog() -> Json<Vec<OpCatalogResponse>> {
    Json(
        standard_catalog()
            .iter()
            .map(|op| OpCatalogResponse {
                name: op.name,
                tier: op.tier.as_str(),
                bulk_escalates: op.bulk_escalates,
                params_schema: (op.params_schema)(),
            })
            .collect(),
    )
}

async fn post_ops_dispatch(
    State(state): State<AppState>,
    Path(name): Path<String>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<OpDispatchBody>,
) -> Result<Json<Value>, ApiErrorResponse> {
    let source_ip = source.ip();
    reject_reserved_op_params(&payload.params)?;
    let step_up_verified = match payload.step_up_passphrase {
        Some(passphrase) => verify_step_up_passphrase(&state, source_ip, passphrase).await?,
        None => false,
    };
    let params = Value::Object(payload.params);
    let req = DispatchRequest {
        op: name,
        params,
        dry_run: payload.dry_run,
        actor,
        source: Some(source_ip.to_string()),
        step_up_verified,
    };

    let result = state
        .db
        .with_conn(move |conn| Ok(dispatch(conn, standard_catalog(), req)))
        .await
        .map_err(op_storage_error)?;

    result.map(Json).map_err(op_error_response)
}

fn reject_reserved_op_params(
    params: &serde_json::Map<String, Value>,
) -> Result<(), ApiErrorResponse> {
    for key in RESERVED_OP_PARAM_KEYS {
        if params.contains_key(*key) {
            return Err(ApiErrorResponse::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "reserved_param",
                format!("{key} must not appear in params"),
            ));
        }
    }
    Ok(())
}

async fn verify_step_up_passphrase(
    state: &AppState,
    source_ip: IpAddr,
    passphrase: String,
) -> Result<bool, ApiErrorResponse> {
    state
        .throttle
        .check_and_record_source(source_ip)
        .map_err(rate_limited_error)?;

    let Some(phc) = state
        .db
        .with_conn(|conn| load_passphrase_hash(conn).map_err(storage_other))
        .await
        .map_err(op_storage_error)?
    else {
        state.throttle.record_failure(source_ip);
        record_auth_failed(&state.db, source_ip, "step_up").await?;
        return Err(ApiErrorResponse::new(
            StatusCode::FORBIDDEN,
            "step_up_required",
            "step-up required",
        ));
    };
    let verified = tokio::task::spawn_blocking(move || verify_passphrase(&phc, &passphrase))
        .await
        .map_err(|e| op_internal_error(format!("passphrase verification task failed: {e}")))?;

    if !verified {
        state.throttle.record_failure(source_ip);
        record_auth_failed(&state.db, source_ip, "step_up").await?;
        return Err(ApiErrorResponse::new(
            StatusCode::FORBIDDEN,
            "step_up_required",
            "step-up required",
        ));
    }

    state.throttle.record_success(source_ip);
    Ok(true)
}

async fn post_setup_passphrase(
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
    if !setup_mode {
        return Err(passphrase_already_set());
    }

    let passphrase = payload.passphrase;
    validate_new_passphrase(&passphrase)?;
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
        return Err(passphrase_already_set());
    };

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
        .map_err(rate_limited_error)?;

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

fn validate_new_passphrase(passphrase: &str) -> Result<(), ApiErrorResponse> {
    if passphrase.len() < MIN_PASSPHRASE_LEN {
        return Err(ApiErrorResponse::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "weak_passphrase",
            "passphrase must be at least 8 characters",
        ));
    }
    Ok(())
}

fn rate_limited_error(retry: RetryAfter) -> ApiErrorResponse {
    let seconds = retry.duration.as_secs().max(1);
    ApiErrorResponse::new(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        format!("retry after {seconds} seconds"),
    )
    .with_retry_after(seconds)
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
    tracing::trace!(
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

fn find_series_for_api(
    conn: &rusqlite::Connection,
    series_key: &str,
) -> Result<Option<i64>, StorageError> {
    match ledger::find_series_by_key(conn, series_key) {
        Ok(found) => Ok(found),
        Err(ledger::LedgerError::InvalidId(_)) => Ok(None),
        Err(e) => Err(storage_other(e)),
    }
}

fn internal_error<E: std::fmt::Display>(e: E) -> ApiErrorResponse {
    tracing::error!(error = %e, "api internal error");
    ApiErrorResponse::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        "internal error",
    )
}

fn op_error_response(err: OpError) -> ApiErrorResponse {
    match err {
        OpError::NotFound => {
            ApiErrorResponse::new(StatusCode::NOT_FOUND, "unknown_op", "unknown op")
        }
        OpError::Forbidden(reason) => {
            ApiErrorResponse::new(StatusCode::FORBIDDEN, "forbidden", reason)
        }
        OpError::StepUpRequired => ApiErrorResponse::new(
            StatusCode::FORBIDDEN,
            "step_up_required",
            "step-up required",
        ),
        OpError::PreconditionFailed(message) => {
            ApiErrorResponse::new(StatusCode::CONFLICT, "precondition_failed", message)
        }
        OpError::Validation(message) => {
            ApiErrorResponse::new(StatusCode::UNPROCESSABLE_ENTITY, "validation", message)
        }
        OpError::Internal(message) => op_internal_error(message),
    }
}

fn op_storage_error<E: std::fmt::Display>(e: E) -> ApiErrorResponse {
    op_internal_error(e)
}

fn op_internal_error<E: std::fmt::Display>(e: E) -> ApiErrorResponse {
    tracing::error!(error = %e, "ops api internal error");
    ApiErrorResponse::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        "internal error",
    )
}

fn passphrase_already_set() -> ApiErrorResponse {
    ApiErrorResponse::new(StatusCode::CONFLICT, "conflict", "passphrase already set")
}
