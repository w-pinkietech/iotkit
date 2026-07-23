use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use iotkit_edge::web::{WebConfig, router, test_support::StubApplication};
use tower::ServiceExt;

#[tokio::test]
async fn login_page_keeps_console_hooks() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::default()));
    let response = app
        .oneshot(Request::get("/login").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(
        to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains(r#"action="/login""#));
    assert!(html.contains(r#"name="login_id""#));
}

#[tokio::test]
async fn static_assets_are_served_from_the_existing_frontend_build() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::default()));
    for (path, content_type) in [
        ("/static/edge.css", "text/css; charset=utf-8"),
        ("/static/console.js", "text/javascript; charset=utf-8"),
        ("/static/pinkietech-mark.svg", "image/svg+xml"),
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], content_type);
    }
}

#[tokio::test]
async fn console_redirects_anonymous_users_and_preserves_shell_hooks() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::default()));
    let anonymous = app
        .clone()
        .oneshot(Request::get("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::SEE_OTHER);
    assert_eq!(anonymous.headers()["location"], "/login");

    let authenticated = app
        .oneshot(
            Request::get("/status")
                .header("cookie", "iotkit_edge_session=valid; iotkit_edge_csrf=csrf")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = String::from_utf8(
        to_bytes(authenticated.into_body(), 1_000_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    for hook in [
        r#"class="console-shell""#,
        r#"class="side-nav""#,
        r#"aria-current="page""#,
        r#"id="main-content""#,
        r#"class="logout-form""#,
        r#"data-console-page="status""#,
    ] {
        assert!(html.contains(hook), "missing {hook}");
    }
}

#[tokio::test]
async fn console_pages_render_the_existing_operator_content_and_form_hooks() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    for (path, hooks) in [
        (
            "/status",
            &[
                r#"class="health-banner"#,
                r#"id="signal-table""#,
                "センサーの現在値",
            ][..],
        ),
        (
            "/equipment",
            &[
                r#"class="equipment-row""#,
                "/equipment/edge-nodes/edge-node-02",
                "assembly-edge-02",
            ][..],
        ),
        (
            "/equipment/edge-nodes/edge-node-02",
            &[
                r#"action="/console/edge-nodes/edge-node-02/activation""#,
                "登録する",
            ][..],
        ),
        (
            "/equipment/devices/device-01/sensors/signal-01",
            &[
                r#"data-setting-tabs"#,
                r#"data-signal-profile"#,
                r#"id="rule-create""#,
                r#"data-preview-chart"#,
            ][..],
        ),
        (
            "/logs",
            &[
                r#"id="history-filter""#,
                r#"class="history-chart""#,
                r#"id="log-table""#,
                "加工後CSV",
                "受信した生データCSV",
            ][..],
        ),
        (
            "/output",
            &[
                r#"class="output-add-card"#,
                r#"value="iotkit.mqtt-json.v1""#,
                r#"value="pinikiet.mqtt.v1""#,
                r#"class="output-binding-table""#,
            ][..],
        ),
        (
            "/system",
            &[
                "保存データの状態",
                "raw受信データ",
                "確認が必要なこと",
                r#"class="storage-meter""#,
            ][..],
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(path)
                    .header("cookie", "iotkit_edge_session=valid; iotkit_edge_csrf=csrf")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let html = String::from_utf8(
            to_bytes(response.into_body(), 2_000_000)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        for hook in hooks {
            assert!(html.contains(hook), "{path} missing {hook}");
        }
    }
}

#[tokio::test]
async fn deprecated_monitor_and_signals_urls_redirect_to_sensors() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    for path in ["/monitor", "/signals"] {
        let response = app
            .clone()
            .oneshot(
                Request::get(path)
                    .header("cookie", "iotkit_edge_session=valid; iotkit_edge_csrf=csrf")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{path}");
        assert_eq!(response.headers()["location"], "/sensors");
    }
}
