use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use iotkit_edge::web::{WebConfig, router, test_support::StubApplication};
use tower::ServiceExt;

#[tokio::test]
async fn security_headers_and_route_inventory_are_stable() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::system_admin()));
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
        "/api/v1/output-adapters",
        "/api/v1/export-profiles",
        "/api/v1/output-routes",
        "/api/v1/audit-events",
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
        "/console/signals/signal-1/semantic-rules",
        "/console/semantic-rules/rule-1/retire",
        "/console/semantic-rules/rule-1/counter-resets",
        "/console/export-profiles",
        "/console/export-profiles/profile-1/stop",
        "/console/output-bindings/binding-1/start",
        "/console/accounts",
        "/console/accounts/account-1",
        "/api/v1/session",
        "/api/v1/session/password",
        "/api/v1/edge-nodes/edge-1/activation",
        "/api/v1/devices/device-1/profile",
        "/api/v1/signals/signal-1/profile",
        "/api/v1/signals/signal-1/calibration",
        "/api/v1/signals/signal-1/semantic-rules",
        "/api/v1/semantic-rules/rule-1/counter-resets",
        "/api/v1/export-profiles",
        "/api/v1/output-bindings/binding-1/start",
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
async fn admin_output_page_leads_with_delivery_state_and_retains_mutation_controls() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::system_admin()));
    let response = app
        .oneshot(authenticated(
            Request::get("/output").body(Body::empty()).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("正常に送信中"));
    assert!(html.contains("設定が必要"));
    assert!(html.contains("配送に問題"));
    assert!(html.contains("送信対象"));
    assert!(html.contains("最終送信"));
    assert!(html.contains("配送待ち"));
    assert_eq!(html.matches("class=\"output-health-card").count(), 3);
    assert!(html.contains("<details class=\"output-technical\""));
    assert!(html.contains("data-copy-text="));
    assert!(html.contains("data-unix-ms=\"1735689660000\""));
    assert!(html.contains("Profile ID"));
    assert!(html.contains("binding-pinikiet-01"));
    assert!(html.contains("<span>乾燥炉入口 温度</span>"));
    assert_eq!(html.matches("class=\"output-binding-form\"").count(), 1);
    assert!(html.contains(
        "action=\"/console/output-bindings/binding-pinikiet-01\" class=\"output-binding-form\""
    ));
    assert!(!html.contains(
        "action=\"/console/output-bindings/binding-pinikiet-06\" class=\"output-binding-form\""
    ));
    assert!(html.contains("<select name=\"mode\" required>"));
    assert!(html.contains("<option value=\"onoff\">ON/OFF</option>"));
    assert!(html.contains("<option value=\"gantt_chart\">稼働状態</option>"));
    assert!(html.contains("name=\"revision\" value=\"1\""));
    assert!(!html.contains("name=\"mode\" value=\"automatic\""));
    assert!(html.contains("配送状態を確認できません"));
    assert!(!html.contains("semantic or output resource was not found"));
    assert!(html.contains("name=\"display_name\" value=\"汎用MQTT JSONで送る\""));
    assert!(html.contains("name=\"auto_bind_future_rules\" value=\"true\" required"));
    assert!(html.contains("今後追加する対応可能な値も自動で送ります"));
    assert!(html.contains("この内容で送信を開始"));
    assert!(html.contains("class=\"output-stop-form\""));
}

#[tokio::test]
async fn viewer_output_page_keeps_delivery_facts_without_mutation_controls() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::viewer()));
    let response = app
        .oneshot(authenticated(
            Request::get("/output").body(Body::empty()).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("正常に送信中"));
    assert!(html.contains("設定が必要"));
    assert!(html.contains("配送に問題"));
    assert!(html.contains("送信対象"));
    assert!(html.contains("<details class=\"output-technical\""));
    assert!(html.contains("data-copy-text="));
    assert!(html.contains("data-unix-ms=\"1735689660000\""));
    assert!(html.contains("Profile ID"));
    assert!(html.contains("binding-pinikiet-01"));
    assert!(html.contains("class=\"output-activation-preview\""));
    assert!(html.contains("自動設定 0件"));
    assert!(html.contains("要設定 0件"));
    assert!(html.contains("対象外 0件"));
    assert!(!html.contains("class=\"output-add-card\""));
    assert!(!html.contains("class=\"output-binding-form\""));
    assert!(!html.contains("class=\"prepared-output-start\""));
    assert!(!html.contains("class=\"output-stop-form\""));
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

#[tokio::test]
async fn api_and_form_login_preserve_non_enumerating_rate_limit_status() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::rate_limited()));
    let api = app
        .clone()
        .oneshot(
            Request::post("/api/v1/session")
                .header(header::ORIGIN, "http://127.0.0.1:8080")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"login_id":"anything","password":"anything"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(api.status(), StatusCode::TOO_MANY_REQUESTS);
    let api_body: serde_json::Value =
        serde_json::from_slice(&to_bytes(api.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(api_body["error"]["code"], "login_rate_limited");

    let form = app
        .oneshot(
            Request::post("/login")
                .header(header::ORIGIN, "http://127.0.0.1:8080")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("login_id=anything&password=anything"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(form.status(), StatusCode::TOO_MANY_REQUESTS);
    let form_body = to_bytes(form.into_body(), 64 * 1024).await.unwrap();
    assert!(!String::from_utf8_lossy(&form_body).contains("anything"));
}

#[tokio::test]
async fn existing_put_routes_are_registered_and_dispatchable() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::default()));
    for path in [
        "/api/v1/devices/device-1/profile",
        "/api/v1/signals/signal-1/profile",
        "/api/v1/signals/signal-1/calibration",
        "/api/v1/semantic-rules/rule-1",
        "/api/v1/output-bindings/binding-1",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::put(path)
                    .header(header::ORIGIN, "http://127.0.0.1:8080")
                    .header(
                        header::COOKIE,
                        "iotkit_edge_session=valid; iotkit_edge_csrf=csrf",
                    )
                    .header("x-csrf-token", "csrf")
                    .header(header::IF_MATCH, "\"1\"")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::NOT_FOUND, "PUT {path}");
        assert_ne!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "PUT {path}"
        );
    }
}

#[tokio::test]
async fn mutation_status_codes_match_the_existing_api() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::system_admin()));
    for (path, expected) in [
        ("/api/v1/edge-nodes/edge-1/activation", StatusCode::ACCEPTED),
        (
            "/api/v1/signals/signal-1/semantic-rules",
            StatusCode::CREATED,
        ),
        ("/api/v1/accounts", StatusCode::CREATED),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header(header::ORIGIN, "http://127.0.0.1:8080")
                    .header(
                        header::COOKIE,
                        "iotkit_edge_session=valid; iotkit_edge_csrf=csrf",
                    )
                    .header("x-csrf-token", "csrf")
                    .header(header::IF_MATCH, "\"1\"")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{path}");
    }
}

#[tokio::test]
async fn revisioned_resources_require_if_match_and_advance_etag() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::system_admin()));
    let get = app
        .clone()
        .oneshot(authenticated(
            Request::get("/api/v1/signals/signal-1/semantic-configuration")
                .body(Body::empty())
                .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(get.headers()[header::ETAG], "\"1\"");

    for (if_match, expected) in [
        (None, StatusCode::PRECONDITION_REQUIRED),
        (Some("\"malformed\""), StatusCode::PRECONDITION_FAILED),
        (Some("\"0\""), StatusCode::PRECONDITION_FAILED),
        (Some("\"99\""), StatusCode::PRECONDITION_FAILED),
    ] {
        let mut request = authenticated(
            Request::put("/api/v1/semantic-rules/rule-1")
                .header(header::ORIGIN, "http://127.0.0.1:8080")
                .header("x-csrf-token", "csrf")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"display_name":"Rule","kind":"numeric"}"#))
                .unwrap(),
        );
        if let Some(if_match) = if_match {
            request
                .headers_mut()
                .insert(header::IF_MATCH, if_match.parse().unwrap());
        }
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), expected);
    }

    let success = app
        .oneshot(authenticated(
            Request::put("/api/v1/semantic-rules/rule-1")
                .header(header::ORIGIN, "http://127.0.0.1:8080")
                .header("x-csrf-token", "csrf")
                .header(header::IF_MATCH, "\"1\"")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"display_name":"Rule","kind":"numeric"}"#))
                .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(success.status(), StatusCode::OK);
    assert_eq!(success.headers()[header::ETAG], "\"2\"");
}

fn authenticated(mut request: Request<Body>) -> Request<Body> {
    request.headers_mut().insert(
        header::COOKIE,
        "iotkit_edge_session=valid; iotkit_edge_csrf=csrf"
            .parse()
            .unwrap(),
    );
    request
}
