use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use iotkit_edge::web::{WebConfig, router, test_support::StubApplication};
use tower::ServiceExt;

#[tokio::test]
async fn security_headers_and_route_inventory_are_stable() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::default()));
    for path in [
        "/",
        "/login",
        "/password",
        "/status",
        "/monitor",
        "/sensors",
        "/sensors/signal-1",
        "/equipment",
        "/equipment/edge-nodes/edge-1",
        "/equipment/devices/device-1",
        "/equipment/devices/device-1/sensors/signal-1",
        "/setup",
        "/edge-nodes",
        "/devices",
        "/signals",
        "/logs",
        "/output",
        "/audit",
        "/accounts",
        "/system",
        "/api/v1/session",
        "/api/v1/devices",
        "/api/v1/edge-nodes",
        "/api/v1/signals",
        "/api/v1/history",
        "/api/v1/history/series",
        "/api/v1/history.csv",
        "/api/v1/semantic-history.csv",
        "/api/v1/system/storage",
        "/api/v1/system/diagnostics",
        "/api/v1/setup/devices",
        "/api/v1/output/adapters",
        "/api/v1/output/profiles",
        "/api/v1/output/bindings",
        "/api/v1/output/routes",
        "/api/v1/audit",
        "/api/v1/accounts",
        "/static/edge.css",
        "/static/console.js",
        "/static/pinkietech-mark.svg",
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(response.headers()["x-frame-options"], "DENY");
    }

    for path in [
        "/logout",
        "/password",
        "/console/devices/device-1/profile",
        "/console/edge-nodes/edge-1/activation",
        "/console/signals/signal-1/profile",
        "/console/signals/signal-1/calibration",
        "/console/signals/signal-1/rules",
        "/console/signals/signal-1/retire",
        "/console/signals/signal-1/reset",
        "/console/output/profiles/profile-1/activate",
        "/console/output/profiles/profile-1/stop",
        "/console/output/bindings/binding-1/start",
        "/console/accounts",
        "/console/accounts/account-1",
        "/api/v1/session",
        "/api/v1/session/password",
        "/api/v1/edge-nodes/edge-1/activation",
        "/api/v1/devices/device-1/profile",
        "/api/v1/signals/signal-1/profile",
        "/api/v1/signals/signal-1/calibration",
        "/api/v1/signals/signal-1/rules",
        "/api/v1/signals/signal-1/retire",
        "/api/v1/signals/signal-1/reset",
        "/api/v1/output/profiles",
        "/api/v1/output/bindings",
        "/api/v1/output/routes",
        "/api/v1/accounts",
        "/api/v1/mapping-previews",
    ] {
        let response = app
            .clone()
            .oneshot(Request::post(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::NOT_FOUND, "POST {path}");
        assert_ne!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "POST {path}"
        );
    }
}

#[tokio::test]
async fn login_sets_strict_host_only_session_and_csrf_cookies() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::default()));
    let response = app
        .oneshot(
            Request::post("/api/v1/session")
                .header(header::ORIGIN, "http://127.0.0.1:8080")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"login_id":"admin","password":"correct"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let cookies: Vec<_> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect();
    assert!(cookies.iter().any(|c| c.starts_with("iotkit_edge_session=")
        && c.contains("HttpOnly")
        && c.contains("SameSite=Strict")
        && !c.contains("Domain=")));
    assert!(cookies.iter().any(|c| c.starts_with("iotkit_edge_csrf=")
        && !c.contains("HttpOnly")
        && c.contains("SameSite=Strict")));
}

#[tokio::test]
async fn mutation_requires_same_origin_and_csrf() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::default()));
    let request = Request::post("/api/v1/mapping-previews")
        .header(header::ORIGIN, "https://evil.example")
        .header(
            header::COOKIE,
            "iotkit_edge_session=valid; iotkit_edge_csrf=csrf",
        )
        .header("x-csrf-token", "csrf")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "origin_forbidden");
    assert!(body["error"]["field"].is_null());
    assert!(
        body["error"]["request_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("req_") && value.len() == 20)
    );
}

#[tokio::test]
async fn login_rejects_unknown_fields_and_oversized_bodies_with_json_errors() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::default()));
    for body in [
        Body::from(r#"{"login_id":"admin","password":"correct","extra":true}"#),
        Body::from(vec![b'x'; 64 * 1024 + 1]),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/session")
                    .header(header::ORIGIN, "http://127.0.0.1:8080")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/json; charset=utf-8"
        );
    }
}
