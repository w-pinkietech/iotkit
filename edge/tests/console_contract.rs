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
                "登録済みの収集ノード",
                r#"<strong>1</strong><small>台</small>"#,
            ][..],
        ),
        (
            "/equipment",
            &[
                r#"class="equipment-row""#,
                "/equipment/edge-nodes/edge-node-02",
                "assembly-edge-02",
                "接続されている収集ノード",
            ][..],
        ),
        (
            "/equipment/edge-nodes/edge-node-01",
            &[
                "乾燥炉入口 BravePI",
                "/equipment/devices/device-01",
                "1件のセンサー",
            ][..],
        ),
        (
            "/equipment/devices/device-01",
            &[
                "factory-edge-01",
                "乾燥炉入口 BravePI",
                "/equipment/devices/device-01/sensors/signal-01",
                "乾燥炉入口 温度",
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
                r#"class="sensor-detail-header""#,
                r#"class="sensor-detail-settings sensor-setting-controls""#,
                r#"class="content-section sensor-settings-panel""#,
                r#"data-default-setting-tab="basic""#,
                "計測ルール",
                r#"data-preview-range"#,
                r#"class="simulation-chart-wrap""#,
                r#"data-signal-ref="signal-01""#,
                r#"data-setting-tabs"#,
                r#"data-signal-profile"#,
                r#"id="rule-create""#,
                r#"data-preview-chart"#,
                r#"data-preview-feed-state"#,
                r#"data-preview-checked-at"#,
                "Edge Nodeから届いた実データ",
                "/equipment/edge-nodes/edge-node-01",
                "/equipment/devices/device-01",
                "乾燥炉入口 BravePI",
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
async fn device_collection_without_a_selected_device_is_not_a_valid_console_page() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = app
        .oneshot(
            Request::get("/equipment/devices")
                .header("cookie", "iotkit_edge_session=valid; iotkit_edge_csrf=csrf")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn numeric_sensor_rule_uses_the_settings_card_without_counter_actions() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = app
        .oneshot(
            Request::get("/equipment/devices/device-01/sensors/signal-01")
                .header("cookie", "iotkit_edge_session=valid; iotkit_edge_csrf=csrf")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = String::from_utf8(
        to_bytes(response.into_body(), 2_000_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains(r#"class="semantic-calibration""#));
    assert!(html.contains(r#"class="semantic-rule-card""#));
    assert!(html.contains("測定値"));
    assert!(!html.contains("/counter-resets"));
}

#[tokio::test]
async fn sensor_rules_expose_the_complete_change_processing_editor() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = app
        .oneshot(
            Request::get("/equipment/devices/device-01/sensors/signal-01")
                .header("cookie", "iotkit_edge_session=valid; iotkit_edge_csrf=csrf")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = String::from_utf8(
        to_bytes(response.into_body(), 2_000_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    for expected in [
        r#"<option value="cumulative_counter">累積値</option>"#,
        r#"data-semantic-detector"#,
        r#"name="detector_mode""#,
        r#"name="rise_threshold""#,
        r#"name="fall_threshold""#,
        r#"name="rise_debounce_seconds""#,
        r#"name="fall_debounce_seconds""#,
        r#"data-semantic-trigger"#,
        r#"value="on_transition""#,
        r#"value="on_notification""#,
        "OFFからONへ変わったとき",
    ] {
        assert!(html.contains(expected), "missing {expected}");
    }
}

#[tokio::test]
async fn basic_sensor_settings_show_the_profile_form_without_an_inner_disclosure() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = app
        .oneshot(
            Request::get("/equipment/devices/device-01/sensors/signal-01")
                .header("cookie", "iotkit_edge_session=valid; iotkit_edge_csrf=csrf")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = String::from_utf8(
        to_bytes(response.into_body(), 2_000_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains(r#"<form id="sensor-profile""#));
    assert!(!html.contains(r#"<details id="sensor-profile""#));
    assert!(html.contains("<span>計測ルール</span>"));
    assert!(!html.contains("<span>通常の値</span>"));
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
