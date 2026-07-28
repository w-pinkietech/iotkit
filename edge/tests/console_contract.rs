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
        descriptor_current: true,
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
        descriptor_current: true,
        revision,
        signals,
    }
}

fn commissioning_node(state: EdgeNodeState) -> ConsoleEdgeNode {
    ConsoleEdgeNode {
        edge_node_ref: "node-01".into(),
        edge_node_id: "edge-01".into(),
        ledger_epoch: "epoch-01".into(),
        first_detected_at: "2025-01-01T00:00:00Z".into(),
        name: "Edge Node".into(),
        location: "工場".into(),
        state,
        state_label: "登録済み".into(),
        state_class: "configured".into(),
        can_activate: state == EdgeNodeState::Discovered,
        needs_recovery_review: state == EdgeNodeState::RecoveryHold,
        devices: Vec::new(),
        descriptor_device_count: 0,
        descriptor_signal_count: 0,
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
fn commissioning_projection_waits_for_the_first_descriptor_without_claiming_completion() {
    let view = commissioning_view(&[], &[], &[]);

    assert_eq!(view.stage, "waiting-edge-node");
    assert_eq!(view.completed_steps, 0);
    assert_eq!(view.title, "収集ノードの接続を待っています");
    assert_eq!(view.action_href, "/equipment");
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

#[test]
fn commissioning_projection_ignores_stale_descriptor_resources() {
    let mut stale_signal = commissioning_signal(false, false);
    stale_signal.descriptor_current = false;
    let mut stale_device = commissioning_device(0, vec![stale_signal.clone()]);
    stale_device.descriptor_current = false;

    let view = commissioning_view(
        &[commissioning_node(EdgeNodeState::Active)],
        &[stale_device],
        &[stale_signal],
    );

    assert_eq!(view.stage, "complete");
    assert_eq!(view.pending_devices, 0);
    assert_eq!(view.pending_signals, 0);
    assert_eq!(view.action_href, "/sensors");
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
        r#"class="menu-button""#,
        r#"aria-controls="sidebar""#,
        r#"class="mobile-overlay""#,
        r#"aria-label="メニューを閉じる""#,
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
    let app = router(WebConfig::test(), Arc::new(StubApplication::system_admin()));
    for (path, hooks) in [
        (
            "/status",
            &[
                r#"class="health-banner"#,
                r#"id="signal-table""#,
                r#"class="signal-table-wrap status-signal-table""#,
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
                r#"class="table-wrap history-table-wrap""#,
                "加工後CSV",
                "受信した生データCSV",
            ][..],
        ),
        (
            "/output",
            &[
                r#"class="output-add-card"#,
                r#"value="iotkit.mqtt-json.v1""#,
                r#"class="output-destinations""#,
                r#"class="output-destination-card"#,
                r#"class="output-rule-list""#,
            ][..],
        ),
        (
            "/audit",
            &[
                r#"id="audit-table""#,
                r#"class="table-wrap audit-table-wrap""#,
            ][..],
        ),
        (
            "/accounts",
            &[
                r#"class="account-table""#,
                r#"data-label="ログインID""#,
                r#"class="account-create-form""#,
            ][..],
        ),
        (
            "/system",
            &[
                "IoTKit Edge 0.1.0",
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
async fn commissioning_panel_leads_status_and_equipment_with_one_admin_next_action() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );

    for path in ["/status", "/equipment"] {
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
        let html = String::from_utf8(
            to_bytes(response.into_body(), 2_000_000)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();

        assert!(
            html.contains(
                r#"<section class="onboarding" data-commissioning-stage="activate-edge-node">"#
            ),
            "{path} must expose the stable commissioning stage"
        );
        let mut previous = 0;
        for concept in [
            "収集ノードを登録",
            "機器を確認",
            "センサーを設定",
            "計測を開始",
        ] {
            let position = html.find(concept).expect("ordered commissioning concept");
            assert!(
                position >= previous,
                "{concept} must remain in journey order"
            );
            previous = position;
        }
        assert_eq!(
            html.matches(r#"class="button onboarding-primary""#).count(),
            1,
            "{path} must present exactly one primary next action"
        );
        assert!(
            html.find(r#"class="onboarding-next""#).unwrap()
                < html.find(r#"class="onboarding-steps""#).unwrap(),
            "{path} must put the contextual next action before progress details"
        );

        let onboarding = html.find(r#"class="onboarding""#).unwrap();
        let page_content = if path == "/status" {
            html.find(r#"class="health-banner"#).unwrap()
        } else {
            html.find(r#"class="equipment-overview""#).unwrap()
        };
        assert!(
            onboarding < page_content,
            "{path} must place the next action before supporting content"
        );
    }
}

#[tokio::test]
async fn discovered_edge_node_detail_explains_descriptor_and_history_boundary() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = app
        .oneshot(
            Request::get("/equipment/edge-nodes/edge-node-02")
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

    for fact in [
        "Edge Node ID",
        "assembly-edge-02",
        "データ流の世代ID",
        "データの連続性を区別する識別子です",
        "epoch-02",
        "初回検出時刻",
        r#"<time data-unix-ms="1735689602000">1735689602000 (Unix ms)</time>"#,
        "検出したデバイス",
        "0台",
        "検出したセンサー",
        "0件",
        "正式な履歴は登録完了後に受信した値から始まります",
    ] {
        assert!(html.contains(fact), "missing discovered-node fact: {fact}");
    }
}

#[tokio::test]
async fn activating_pages_explain_live_checks_and_only_mark_activation_views_for_reload() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::activating()));
    for path in [
        "/status",
        "/equipment",
        "/equipment/edge-nodes/edge-node-02",
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
        let html = String::from_utf8(
            to_bytes(response.into_body(), 2_000_000)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(html.contains(r#"data-activation-refresh="true""#), "{path}");
        assert!(html.contains("3秒ごとに登録状態を自動確認します"), "{path}");
        assert!(
            html.contains("この画面を離れてもサーバー側の登録処理は続きます"),
            "{path}"
        );
        assert!(html.contains("最終確認"), "{path}");
        assert!(
            html.contains(r#"data-activation-check-now>今すぐ確認</button>"#),
            "{path}"
        );
        assert!(html.contains("サーバー側の登録処理は続きます"), "{path}");
    }

    let response = app
        .oneshot(
            Request::get("/equipment/devices/device-01")
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
    assert!(!html.contains(r#"data-activation-refresh="true""#));
}

#[tokio::test]
async fn successful_console_mutations_render_notices_and_preserve_rule_context() {
    let complete = router(WebConfig::test(), Arc::new(StubApplication::complete()));
    let response = complete
        .oneshot(
            Request::get(
                "/equipment/devices/device-01/sensors/signal-01?saved=1&result=semantic-rule&tab=normal",
            )
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
    assert!(html.contains("計測ルールを保存しました"));
    assert!(html.contains("初回設定が完了しました"));
    assert!(html.contains("「概要」で現在の計測状態を確認できます"));
    assert!(html.contains(r#"data-default-setting-tab="normal""#));

    let configured = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = configured
        .oneshot(
            Request::get("/equipment/devices/device-01?saved=1&result=device-profile")
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
    assert!(html.contains("機器の設定を保存しました"));
}

#[tokio::test]
async fn completed_activation_on_node_detail_names_the_next_device_setup_action() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::post_activation()),
    );
    let response = app
        .oneshot(
            Request::get("/equipment/edge-nodes/edge-node-01?saved=1&result=activation")
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

    assert!(html.contains("収集ノードの登録が完了しました"));
    assert!(!html.contains("収集ノードの登録を受け付けました"));
    assert_eq!(html.matches("次へ: 機器を設定").count(), 1);
    assert!(html.contains(r#"data-next-device-setup href="/equipment/devices/device-01""#));
}

#[tokio::test]
async fn viewer_sees_post_activation_state_without_an_actionable_setup_link() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::post_activation_viewer()),
    );
    let response = app
        .oneshot(
            Request::get("/equipment/edge-nodes/edge-node-01")
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

    assert!(html.contains("登録済み"));
    assert!(html.contains("設定が必要"));
    assert!(html.contains("閲覧のみ"));
    assert!(html.contains("設定管理者に機器設定を依頼してください"));
    assert!(!html.contains("data-next-device-setup"));
    assert!(!html.contains("次へ: 機器を設定"));
}

#[tokio::test]
async fn console_mutation_redirect_replaces_transient_query_values() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = app
        .oneshot(
            Request::post("/console/signals/signal-01/profile")
                .header("origin", "http://127.0.0.1:8080")
                .header(
                    "referer",
                    "http://127.0.0.1:8080/equipment/devices/device-01/sensors/signal-01?keep=1&saved=1&result=semantic-rule&tab=normal&error=old",
                )
                .header("cookie", "iotkit_edge_session=valid; iotkit_edge_csrf=csrf")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("_csrf=csrf"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()["location"].to_str().unwrap();
    assert_eq!(location.matches("saved=").count(), 1, "{location}");
    assert_eq!(location.matches("result=").count(), 1, "{location}");
    assert_eq!(location.matches("tab=").count(), 1, "{location}");
    assert!(!location.contains("error="), "{location}");
    assert!(location.contains("keep=1"), "{location}");
    assert!(location.contains("saved=1"), "{location}");
    assert!(location.contains("result=signal-profile"), "{location}");
    assert!(location.contains("tab=basic"), "{location}");
}

#[tokio::test]
async fn sensor_profile_offers_generic_temperature_and_humanizes_ucum_celsius() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::unconfigured()));
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
    assert!(html.contains(r#"<option value="temperature">温度（方式未確認）</option>"#));
    assert!(html.contains(r#"<option value="thermocouple""#));
    assert!(!html.contains(">Cel<"));
    assert!(html.contains("°C"));
}

#[tokio::test]
async fn completed_commissioning_does_not_displace_the_normal_monitor() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::complete()));
    let response = app
        .oneshot(
            Request::get("/status")
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

    assert!(!html.contains(r#"class="onboarding""#));
    assert!(html.contains("センサーデータを受信しています"));
    assert!(html.contains("センサーの現在値"));
}

#[tokio::test]
async fn commissioning_is_read_only_for_viewers_and_recovery_never_offers_activation() {
    let viewer = router(WebConfig::test(), Arc::new(StubApplication::viewer()));
    for path in ["/status", "/equipment"] {
        let response = viewer
            .clone()
            .oneshot(
                Request::get(path)
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

        assert!(html.contains(r#"data-commissioning-stage="activate-edge-node""#));
        assert!(html.contains(r#"class="onboarding-read-only""#));
        assert!(html.contains("次の操作は設定管理者が行います"));
        assert!(!html.contains(r#"class="button onboarding-primary""#));
    }

    let recovery = router(WebConfig::test(), Arc::new(StubApplication::recovery()));
    for path in ["/status", "/equipment/edge-nodes/edge-node-02"] {
        let response = recovery
            .clone()
            .oneshot(
                Request::get(path)
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

        assert!(!html.contains(r#"/activation""#), "{path}");
        assert!(!html.contains(">登録する<"), "{path}");
        if path == "/status" {
            assert!(html.contains(r#"data-commissioning-stage="recovery""#));
            assert!(html.contains("収集ノードの復旧を確認"));
        } else {
            for guidance in [
                "両方のデータベースを保全",
                "identityとrestore履歴を調査",
                "行の削除、新しいEdge Node identityの発行、状態の手動編集は行わない",
            ] {
                assert!(html.contains(guidance), "missing recovery help: {guidance}");
            }
        }
    }
}

#[tokio::test]
async fn active_unconfigured_resources_keep_raw_values_visible_beside_setup_help() {
    let app = router(WebConfig::test(), Arc::new(StubApplication::unconfigured()));

    for path in [
        "/equipment/devices/device-01",
        "/equipment/devices/device-01/sensors/signal-01",
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
        let html = String::from_utf8(
            to_bytes(response.into_body(), 2_000_000)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();

        assert!(html.contains("設定が必要"), "{path}");
        assert!(
            html.contains("28.5"),
            "{path} must preserve the received raw value"
        );
        assert!(
            html.contains("受信中"),
            "{path} must preserve raw reception state"
        );
    }
}

#[tokio::test]
async fn device_location_editor_uses_revision_to_separate_placeholder_from_saved_value() {
    let unconfigured = router(WebConfig::test(), Arc::new(StubApplication::unconfigured()));
    let response = unconfigured
        .oneshot(
            Request::get("/equipment/devices/device-01")
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
    assert!(html.contains("<p>設置場所 未設定</p>"));
    assert!(html.contains(r#"name="location" value="" placeholder="例：乾燥炉入口" required"#));
    assert!(!html.contains(r#"name="location" value="設置場所 未設定""#));

    let configured = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = configured
        .oneshot(
            Request::get("/equipment/devices/device-01")
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
    assert!(
        html.contains(r#"name="location" value="乾燥炉" placeholder="例：乾燥炉入口" required"#)
    );
}

#[tokio::test]
async fn numeric_rule_creation_explains_the_measurement_choice() {
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

    let create = html.find(r#"<details id="rule-create""#).unwrap();
    let help = html
        .find("センサーの値をそのまま記録・出力するときに選びます")
        .unwrap();
    assert!(help > create);
}

#[tokio::test]
async fn sensor_rule_creation_and_preview_targets_are_scoped_by_tab() {
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

    assert!(html.contains("data-preview-rule-result"));
    assert!(html.contains("data-preview-rule-name"));
    assert!(html.contains("data-preview-rule-kind"));
    assert!(html.contains("data-preview-rule-value"));
    assert!(html.contains("data-preview-rule-detail"));

    let normal_start = html.find(r#"id="setting-panel-normal""#).unwrap();
    let alarm_start = html.find(r#"id="setting-panel-alarm""#).unwrap();
    let normal = &html[normal_start..alarm_start];
    let alarm = &html[alarm_start..];

    assert!(normal.contains(r#"data-preview-id="draft-normal""#));
    assert!(!normal.contains(r#"<option value="alarm">"#));
    assert!(alarm.contains(r#"id="alarm-rule-create""#));
    assert!(alarm.contains(r#"data-preview-id="draft-alarm""#));
    assert!(alarm.contains(r#"name="kind" value="alarm""#));
    for label in [
        "異常とみなす側",
        "異常になるしきい値",
        "正常に戻るしきい値",
        "異常確定待ち",
        "復帰確定待ち",
    ] {
        assert!(alarm.contains(label), "missing alarm label: {label}");
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
