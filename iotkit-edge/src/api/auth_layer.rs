use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use iotkit_core_ops::{OpsError, authenticate};

use super::routes::{ApiErrorResponse, AppState};

#[derive(Debug, Clone, Copy)]
pub struct SourceIp(pub std::net::IpAddr);

pub async fn auth_layer(
    State(state): State<AppState>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, ApiErrorResponse> {
    req.extensions_mut().insert(SourceIp(source.ip()));
    let token = bearer_token(&req).ok_or_else(ApiErrorResponse::unauthorized)?;
    let clock_trust = state.clock_trust.clone();
    let actor = state
        .db
        .with_conn(move |conn| {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )
            .map_err(iotkit_core_storage::StorageError::from)?;
            let result = authenticate(&tx, &token, &clock_trust);
            if result.is_ok() {
                tx.commit()
                    .map_err(iotkit_core_storage::StorageError::from)?;
            }
            Ok(result)
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "auth layer internal error");
            ApiErrorResponse::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "internal error",
            )
        })?
        .map_err(|error| match error {
            OpsError::ClockUntrusted => ApiErrorResponse::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "clock_untrusted",
                "trusted wall clock is required; run iotkit-edgectl time confirm locally",
            ),
            other => {
                tracing::error!(error = %other, "auth layer operation failed");
                ApiErrorResponse::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error",
                )
            }
        })?
        .ok_or_else(ApiErrorResponse::unauthorized)?;
    req.extensions_mut().insert(actor);
    Ok(next.run(req).await)
}

fn bearer_token(req: &Request<Body>) -> Option<String> {
    let value = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    value
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
}
