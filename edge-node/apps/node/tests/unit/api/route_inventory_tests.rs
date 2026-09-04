use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;
use iotkit_core_ops::{NewOperatorToken, TokenKind, issue_token};

#[tokio::test]
async fn network_admin_passphrase_setup_route_is_absent() {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    migrations.sort_by_key(|migration| migration.version);
    let db = iotkit_core_storage::init_db_memory(&migrations).unwrap();
    let clock_trust = db
        .with_conn_sync(|conn| {
            iotkit_core_ops::ClockTrust::load(
                conn,
                Arc::new(iotkit_core_ops::SystemClock::default()),
                Duration::from_secs(2),
                Duration::from_secs(300),
            )
            .map(Arc::new)
            .map_err(storage_other)
        })
        .unwrap();
    let bearer = db
        .with_conn_sync(|conn| {
            issue_token(
                conn,
                &NewOperatorToken {
                    name: "route inventory".into(),
                    kind: TokenKind::Human,
                    ceiling: Tier::ReadOnly,
                    is_session: false,
                    expires_at: None,
                },
                "local_cli",
                None,
                None,
            )
            .map(|issued| issued.plaintext.expose().to_string())
            .map_err(storage_other)
        })
        .unwrap();
    let state = AppState {
        db,
        health: Arc::new(Mutex::new(HealthState::new(90))),
        cfg: ApiConfig {
            enabled: true,
            bind: "127.0.0.1:0".parse().unwrap(),
            edge_node_id: "test".parse().unwrap(),
            pipelines_export_path: std::path::PathBuf::from("pipelines.toml"),
        },
        epoch: "test".into(),
        fingerprint: "test".into(),
        throttle: Arc::new(Throttle::default()),
        clock_trust,
        data_dir: std::env::temp_dir(),
    };
    let mut request = Request::post("/api/v1/setup/passphrase")
        .header("authorization", format!("Bearer {bearer}"))
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
    ));

    let response = router(state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
