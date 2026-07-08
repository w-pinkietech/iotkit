use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{Method, Request, header};
use axum::middleware::Next;
use axum::response::Response;
use iotkit_core_ops::{Actor, ActorKind, Tier, authenticate, is_setup_mode};

use crate::health::now_ms;

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
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    let setup_mode = state
        .db
        .with_conn(|conn| {
            is_setup_mode(conn).map_err(|e| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(e),
                ))
            })
        })
        .await
        .map_err(|e| {
            ApiErrorResponse::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("internal error: {e}"),
            )
        })?;
    if setup_mode {
        if !is_setup_allowed(&method, &path) {
            return Err(ApiErrorResponse::unauthorized());
        }
        req.extensions_mut().insert(Actor {
            actor_id: "setup_mode".to_string(),
            actor_kind: ActorKind::SetupMode,
            tier_ceiling: Tier::Daily,
        });
        return Ok(next.run(req).await);
    }

    let token = bearer_token(&req).ok_or_else(ApiErrorResponse::unauthorized)?;
    let actor = state
        .db
        .with_conn(move |conn| {
            authenticate(conn, &token, now_ms()).map_err(|e| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(e),
                ))
            })
        })
        .await
        .map_err(|e| {
            ApiErrorResponse::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("internal error: {e}"),
            )
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

fn is_setup_allowed(method: &Method, path: &str) -> bool {
    match (method, path) {
        (&Method::GET, "/api/v1/series" | "/api/v1/live" | "/api/v1/ops") => true,
        (&Method::POST, path) => path
            .strip_prefix("/api/v1/ops/")
            .is_some_and(|name| !name.is_empty()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Method;

    use super::is_setup_allowed;

    #[test]
    fn setup_allowlist_is_explicit_and_fail_closed() {
        assert!(is_setup_allowed(&Method::GET, "/api/v1/series"));
        assert!(is_setup_allowed(&Method::GET, "/api/v1/live"));
        assert!(is_setup_allowed(&Method::GET, "/api/v1/ops"));
        assert!(is_setup_allowed(
            &Method::POST,
            "/api/v1/ops/device.approve_sighting"
        ));

        assert!(!is_setup_allowed(&Method::POST, "/api/v1/ops"));
        assert!(!is_setup_allowed(
            &Method::GET,
            "/api/v1/ops/device.approve_sighting"
        ));
        assert!(!is_setup_allowed(&Method::GET, "/api/v1/health"));
        assert!(!is_setup_allowed(&Method::GET, "/api/v1/readings"));
        assert!(!is_setup_allowed(&Method::POST, "/api/v1/setup/passphrase"));
        assert!(!is_setup_allowed(&Method::GET, "/api/v1/unknown"));
    }
}
