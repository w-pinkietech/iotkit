use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use iotkit_edge::{
    storage::EdgeNodeState,
    web::{
        ConsoleDevice, ConsoleEdgeNode, ConsoleRule, ConsoleSignal, WebConfig,
        console::commissioning::commissioning_view, router, test_support::StubApplication,
    },
};
use tower::ServiceExt;

fn commissioning_signal(profile_complete: bool, has_rule: bool) -> ConsoleSignal {
    ConsoleSignal {
        signal_ref: "signal-01".into(),
        device_ref: "device-01".into(),
        edge_node_id: "edge-01".into(),
        name: "温度".into(),
        sensor_type: "温度".into(),
        sensor_type_code: "temperature".into(),
        value: "—".into(),
        unit: "℃".into(),
        value_kind: "numeric".into(),
        unit_mode: "unit".into(),
        decimal_places: 1,
        revision: usize::from(profile_complete) as i64,
        status_label: "未受信".into(),
        status_class: "never".into(),
        profile_complete,
        input_is_boolean: false,
        calibration_scale: 1.0,
        calibration_offset: 0.0,
        calibration_revision: 1,
        has_alarm_rules: false,
        rules: has_rule
            .then(|| ConsoleRule {
                rule_id: "rule-01".into(),
                display_name: "現在温度".into(),
                kind: "numeric".into(),
                kind_label: "測定値".into(),
                count_summary: String::new(),
                revision: 1,
                detector_mode: String::new(),
                detector_is_boolean: false,
                rise_threshold: 0.0,
                fall_threshold: 0.0,
                rise_debounce_seconds: 0.0,
                fall_debounce_seconds: 0.0,
                trigger: String::new(),
            })
            .into_iter()
            .collect(),
    }
}

fn commissioning_device(revision: i64, signals: Vec<ConsoleSignal>) -> ConsoleDevice {
    ConsoleDevice {
        device_ref: "device-01".into(),
        edge_node_ref: "node-01".into(),
        edge_node_id: "edge-01".into(),
        name: "設備".into(),
        location: "工場".into(),
        state_label: "登録済み".into(),
        state_class: "configured".into(),
        identifier: "device".into(),
        model_id: "model".into(),
        revision,
        signals,
    }
}

fn commissioning_node(state: EdgeNodeState) -> ConsoleEdgeNode {
    ConsoleEdgeNode {
        edge_node_ref: "node-01".into(),
        edge_node_id: "edge-01".into(),
        name: "Edge Node".into(),
        location: "工場".into(),
        state,
        state_label: "登録済み".into(),
        state_class: "configured".into(),
        can_activate: state == EdgeNodeState::Discovered,
        devices: Vec::new(),
        signal_count: 0,
    }
}

#[test]
fn commissioning_projection_prioritizes_edge_node_activation() {
    let view = commissioning_view(&[commissioning_node(EdgeNodeState::Discovered)], &[], &[]);

    assert_eq!(view.stage, "activate-edge-node");
    assert_eq!(view.action_href, "/equipment/edge-nodes/node-01");
    assert_eq!(view.completed_steps, 0);
    assert_eq!(view.total_steps, 4);
    assert_eq!(view.pending_edge_nodes, 1);
}

#[test]
fn commissioning_projection_orders_recovery_activation_and_resource_setup() {
    let recovery = commissioning_view(
        &[
            commissioning_node(EdgeNodeState::Discovered),
            commissioning_node(EdgeNodeState::Activating),
            commissioning_node(EdgeNodeState::RecoveryHold),
        ],
        &[],
        &[],
    );
    assert_eq!(recovery.stage, "recovery");

    let activating = commissioning_view(
        &[
            commissioning_node(EdgeNodeState::Discovered),
            commissioning_node(EdgeNodeState::Activating),
        ],
        &[],
        &[],
    );
    assert_eq!(activating.stage, "activation-in-progress");

    let signal = commissioning_signal(false, false);
    let unconfigured_device = commissioning_device(0, vec![signal.clone()]);
    let setup_device = commissioning_view(
        &[commissioning_node(EdgeNodeState::Active)],
        &[unconfigured_device],
        std::slice::from_ref(&signal),
    );
    assert_eq!(setup_device.stage, "setup-device");
    assert_eq!(setup_device.action_href, "/equipment/devices/device-01");
    assert_eq!(setup_device.completed_steps, 1);
    assert_eq!(setup_device.pending_devices, 1);

    let configured_device = commissioning_device(1, vec![signal.clone()]);
    let setup_sensor = commissioning_view(
        &[commissioning_node(EdgeNodeState::Active)],
        &[configured_device],
        &[signal],
    );
    assert_eq!(setup_sensor.stage, "setup-sensor");
    assert_eq!(
        setup_sensor.action_href,
        "/equipment/devices/device-01/sensors/signal-01"
    );
    assert_eq!(setup_sensor.completed_steps, 2);
    assert_eq!(setup_sensor.pending_devices, 0);
    assert_eq!(setup_sensor.pending_signals, 1);
}

#[test]
fn commissioning_projection_requires_rules_before_completion() {
    let signal = commissioning_signal(true, false);
    let device = commissioning_device(1, vec![signal.clone()]);
    let setup_rule = commissioning_view(
        &[commissioning_node(EdgeNodeState::Active)],
        std::slice::from_ref(&device),
        &[signal],
    );

    assert_eq!(setup_rule.stage, "setup-rule");
    assert_eq!(
        setup_rule.action_href,
        "/equipment/devices/device-01/sensors/signal-01"
    );
    assert_eq!(setup_rule.completed_steps, 3);
    assert_eq!(setup_rule.pending_signals, 1);

    let signal = commissioning_signal(true, true);
    let complete = commissioning_view(
        &[commissioning_node(EdgeNodeState::Active)],
        &[device],
        &[signal],
    );
    assert_eq!(complete.stage, "complete");
    assert_eq!(complete.completed_steps, 4);
    assert_eq!(complete.pending_edge_nodes, 0);
    assert_eq!(complete.pending_devices, 0);
    assert_eq!(complete.pending_signals, 0);
}

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
