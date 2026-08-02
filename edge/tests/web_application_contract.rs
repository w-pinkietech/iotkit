use std::{collections::HashMap, path::PathBuf};

use iotkit_edge::{
    application::accounts::AccountService,
    application::semantics::{SemanticRuleDraft, Semantics},
    auth::password::Password,
    composition::{LoginPolicy, StorageWebApplication},
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, RawRecord, Storage, StorageProfile},
    web::{ApiMutation, ApiQuery, ConsoleRequest, WebApplication},
};
use iotkit_edge_custody_contract::{ActivationRequest, ActivationResult, DescriptorSnapshot};

fn test_directory() -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::TempDir::new_in(root).unwrap()
}

#[tokio::test]
async fn production_web_adapter_owns_sessions_and_reads_operator_views() {
    let directory = test_directory();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("edge.db"),
    })
    .await
    .unwrap();
    AccountService::new(storage.clone())
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new("long enough owner password").unwrap(),
            1_700_000_000_000,
        )
        .await
        .unwrap();

    let application = StorageWebApplication::new(storage);
    let login = application
        .login("owner", "long enough owner password")
        .await
        .unwrap();
    let principal = application.authenticate(&login.token).await.unwrap();
    assert_eq!(principal.login_id, "owner");
    assert!(application.validate_csrf(&login.token, &login.csrf).await);

    let storage_status = application
        .query(ApiQuery::Named {
            route: "/api/v1/system/storage".into(),
            params: HashMap::new(),
        })
        .await
        .unwrap();
    assert_eq!(storage_status["profile"], "embedded");

    let console = application
        .console(ConsoleRequest {
            path: "/system".into(),
            query: HashMap::new(),
            principal,
        })
        .await
        .unwrap();
    assert_eq!(console.storage.profile_label, "組み込みSQLite");
    assert!(!console.storage.diagnostic_messages.is_empty());

    let created = application
        .mutate(
            &login.principal,
            ApiMutation::Named {
                method: axum::http::Method::POST,
                route: "/api/v1/accounts".into(),
                params: HashMap::new(),
                expected_revision: None,
            },
            serde_json::json!({
                "login_id": "viewer",
                "display_name": "Read Only",
                "role": "viewer",
                "temporary_password": "long enough viewer password",
            }),
        )
        .await
        .unwrap();
    assert_eq!(created.status, axum::http::StatusCode::CREATED);
    assert_eq!(created.body["role"], "viewer");

    application.logout(&login.token).await.unwrap();
    assert!(application.authenticate(&login.token).await.is_err());
}

#[tokio::test]
async fn activation_response_does_not_expose_internal_command_identity() {
    let directory = test_directory();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("activation-response.db"),
    })
    .await
    .unwrap();
    AccountService::new(storage.clone())
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new("long enough owner password").unwrap(),
            1_700_000_000_000,
        )
        .await
        .unwrap();
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage.apply_descriptor(&descriptor, 1).await.unwrap();
    let application = StorageWebApplication::new(storage);
    let principal = application
        .login("owner", "long enough owner password")
        .await
        .unwrap()
        .principal;

    let response = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::POST,
                route: format!("/api/v1/edge-nodes/{}/activation", descriptor.edge_node_id),
                params: HashMap::new(),
                expected_revision: None,
            },
            serde_json::json!({}),
        )
        .await
        .expect("activation request is accepted");

    assert_eq!(response.status, axum::http::StatusCode::ACCEPTED);
    assert_eq!(response.body["state"], "activating");
    assert!(response.body["edge_node_ref"].is_string());
    assert_eq!(response.body.get("activation_id"), None);
    assert_eq!(response.body.get("grant_revision"), None);
}

#[tokio::test]
async fn first_semantic_rule_resolves_a_new_inventory_signal_without_an_existing_rule() {
    let directory = test_directory();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("first-rule.db"),
    })
    .await
    .unwrap();
    AccountService::new(storage.clone())
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new("long enough owner password").unwrap(),
            1_700_000_000_000,
        )
        .await
        .unwrap();
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage.apply_descriptor(&descriptor, 1).await.unwrap();
    assert!(
        storage.list_semantic_rules().await.unwrap().is_empty(),
        "the regression requires a signal with no semantic identity fallback"
    );
    let signal = storage
        .inventory_signals()
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let application = StorageWebApplication::new(storage);
    let principal = application
        .login("owner", "long enough owner password")
        .await
        .unwrap()
        .principal;
    let mut params = HashMap::new();
    params.insert("signal_ref".into(), signal.signal_ref.clone());

    let created = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::POST,
                route: format!("/console/signals/{}/semantic-rules", signal.signal_ref),
                params,
                expected_revision: None,
            },
            serde_json::json!({
                "display_name": "First numeric rule",
                "kind": "numeric",
            }),
        )
        .await
        .expect("first semantic rule must resolve through inventory");

    assert_eq!(created.status, axum::http::StatusCode::CREATED);
    assert_eq!(created.body["display_name"], "First numeric rule");
}

#[tokio::test]
async fn console_commissioning_distinguishes_discovery_registration_and_setup() {
    let directory = test_directory();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("console-state.db"),
    })
    .await
    .unwrap();
    storage
        .initialize_edge_identity(1_720_000_000_000)
        .await
        .unwrap();
    AccountService::new(storage.clone())
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new("long enough owner password").unwrap(),
            1_720_000_000_000,
        )
        .await
        .unwrap();
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    let mut reduced_descriptor = descriptor.clone();
    let mut restored_descriptor = descriptor.clone();
    storage.apply_descriptor(&descriptor, 1).await.unwrap();
    let application = StorageWebApplication::new(storage.clone());
    let principal = application
        .login("owner", "long enough owner password")
        .await
        .unwrap()
        .principal;

    let discovered = application
        .console(ConsoleRequest {
            path: "/status".into(),
            query: HashMap::new(),
            principal: principal.clone(),
        })
        .await
        .unwrap();
    assert_eq!(discovered.commissioning.stage, "activate-edge-node");
    assert_eq!(discovered.registered_edge_node_count, 0);
    assert_eq!(discovered.receiving_signal_count, 0);
    assert_eq!(discovered.signals[0].status_label, "未受信");
    assert_eq!(discovered.signals[0].value, "—");

    let command = storage
        .request_activation(&descriptor.edge_node_id, 2)
        .await
        .unwrap();
    let request = ActivationRequest::decode(&command.payload_json).unwrap();
    storage
        .apply_activation_result(
            &ActivationResult {
                schema_version: 1,
                activation_id: request.activation_id,
                edge_id: request.edge_id,
                edge_node_id: request.edge_node_id,
                ledger_epoch: request.expected_ledger_epoch,
                status: "applied".into(),
                discard_through_reading_seq: 0,
                first_publication_seq: 1,
                applied_at: 3,
            },
            3,
        )
        .await
        .unwrap();
    let record = serde_json::json!({
        "family": "measurement",
        "schema_version": 1,
        "epoch": "epoch-01",
        "pub_seq": 1,
        "series_key": descriptor.signals[0].series_key,
        "values": [true],
        "event_time": 4,
        "received_at": 4
    });
    let unmatched_record = serde_json::json!({
        "family": "measurement",
        "schema_version": 1,
        "epoch": "epoch-01",
        "pub_seq": 2,
        "series_key": "unconfigured-series",
        "values": [999.0],
        "event_time": 4,
        "received_at": 4
    });
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: descriptor.edge_node_id,
            ledger_epoch: descriptor.ledger_epoch,
            publication_id: "console-state-1".into(),
            received_at: 4,
            records: vec![
                RawRecord::new(1, serde_json::to_vec(&record).unwrap()).unwrap(),
                RawRecord::new(2, serde_json::to_vec(&unmatched_record).unwrap()).unwrap(),
            ],
        })
        .await
        .unwrap();

    let active = application
        .console(ConsoleRequest {
            path: "/status".into(),
            query: HashMap::new(),
            principal: principal.clone(),
        })
        .await
        .unwrap();
    assert_eq!(active.commissioning.stage, "setup-device");
    assert_eq!(active.commissioning.pending_devices, 1);
    assert_eq!(active.registered_edge_node_count, 1);
    assert_eq!(active.receiving_signal_count, 1);
    assert_eq!(active.signals[0].status_label, "受信中");
    assert_eq!(active.signals[0].value, "ON");
    assert_eq!(active.devices.len(), 1);
    assert_eq!(active.edge_nodes[0].devices.len(), 1);

    let history = application
        .console(ConsoleRequest {
            path: "/logs".into(),
            query: HashMap::from([("from".into(), "0".into()), ("to".into(), "10".into())]),
            principal: principal.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history.history_signal_ref, active.signals[0].signal_ref);
    assert_eq!(history.history.len(), 1);
    assert_eq!(history.history[0].signal_ref, history.history_signal_ref);
    assert!(!history.history_chart_path.is_empty());
    assert!(
        history
            .history_raw_export_url
            .contains(&format!("signal_ref={}", history.history_signal_ref))
    );

    reduced_descriptor.descriptor_revision += 1;
    reduced_descriptor.devices.clear();
    reduced_descriptor.signals.clear();
    storage
        .apply_descriptor(&reduced_descriptor, 5)
        .await
        .unwrap();
    let reduced = application
        .console(ConsoleRequest {
            path: "/equipment".into(),
            query: HashMap::new(),
            principal: principal.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        storage.inventory_devices().await.unwrap().len(),
        1,
        "durable historical inventory remains intact"
    );
    assert!(reduced.devices.is_empty());
    assert!(reduced.signals.is_empty());
    assert!(reduced.edge_nodes[0].devices.is_empty());
    assert_eq!(reduced.edge_nodes[0].descriptor_device_count, 0);
    assert_eq!(reduced.edge_nodes[0].descriptor_signal_count, 0);
    assert_eq!(reduced.commissioning.stage, "complete");
    assert_eq!(reduced.commissioning.pending_devices, 0);
    assert_eq!(reduced.commissioning.pending_signals, 0);
    assert_eq!(reduced.commissioning.action_href, "/sensors");
    assert_eq!(reduced.edge_nodes[0].first_detected_at, "1");

    restored_descriptor.descriptor_revision += 2;
    storage
        .apply_descriptor(&restored_descriptor, 6)
        .await
        .unwrap();

    let device_ref = active.devices[0].device_ref.clone();
    let mut device_params = HashMap::new();
    device_params.insert("device_ref".into(), device_ref.clone());
    application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::POST,
                route: format!("/console/devices/{device_ref}/profile"),
                params: device_params,
                expected_revision: None,
            },
            serde_json::json!({
                "display_name": "接点デバイス",
                "location": "第1工場",
            }),
        )
        .await
        .unwrap();
    let needs_signal_setup = application
        .console(ConsoleRequest {
            path: "/status".into(),
            query: HashMap::new(),
            principal: principal.clone(),
        })
        .await
        .unwrap();
    assert_eq!(needs_signal_setup.commissioning.stage, "setup-sensor");
    assert_eq!(needs_signal_setup.commissioning.pending_devices, 0);
    assert_eq!(needs_signal_setup.commissioning.pending_signals, 1);

    let signal_ref = active.signals[0].signal_ref.clone();
    let mut params = HashMap::new();
    params.insert("signal_ref".into(), signal_ref.clone());
    let profile = serde_json::json!({
        "display_name": "接点入力",
        "display_sensor_type": "contact",
        "display_value_kind": "boolean",
        "display_unit_mode": "dimensionless",
        "display_unit": "",
        "decimal_places": "0"
    });
    let created = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::POST,
                route: format!("/console/signals/{signal_ref}/profile"),
                params: params.clone(),
                expected_revision: None,
            },
            profile.clone(),
        )
        .await
        .unwrap();
    assert_eq!(created.body["revision"], 1);
    let mut revised = profile;
    revised["display_name"] = serde_json::json!("接点入力 更新後");
    revised["revision"] = serde_json::json!("1");
    let updated = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::POST,
                route: format!("/console/signals/{signal_ref}/profile"),
                params,
                expected_revision: None,
            },
            revised,
        )
        .await
        .expect("HTML form revision strings must update existing profiles");
    assert_eq!(updated.body["revision"], 2);

    let mut temperature_params = HashMap::new();
    temperature_params.insert("signal_ref".into(), signal_ref.clone());
    application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::POST,
                route: format!("/console/signals/{signal_ref}/profile"),
                params: temperature_params,
                expected_revision: None,
            },
            serde_json::json!({
                "display_name": "方式未確認の温度",
                "display_sensor_type": "temperature",
                "display_value_kind": "numeric",
                "display_unit_mode": "unit",
                "display_unit": "°C",
                "decimal_places": "1",
                "revision": "2",
            }),
        )
        .await
        .unwrap();
    let temperature = application
        .console(ConsoleRequest {
            path: format!("/equipment/devices/{device_ref}/sensors/{signal_ref}"),
            query: HashMap::new(),
            principal: principal.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        temperature.selected_signal.unwrap().sensor_type,
        "温度（方式未確認）"
    );
}

#[tokio::test]
async fn login_rate_limit_is_non_enumerating_and_recovers_after_its_window() {
    let directory = test_directory();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("rate-limit.db"),
    })
    .await
    .unwrap();
    AccountService::new(storage.clone())
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new("long enough owner password").unwrap(),
            1_700_000_000_000,
        )
        .await
        .unwrap();
    let application = StorageWebApplication::with_login_policy(
        storage.clone(),
        LoginPolicy {
            max_failures: 1,
            failure_window: std::time::Duration::from_secs(10),
            max_concurrent: 1,
            max_tracked_accounts: 16,
        },
    );

    let mut limited_responses = Vec::new();
    for login_id in ["owner", "missing"] {
        let error = application
            .login(login_id, "wrong password")
            .await
            .unwrap_err();
        assert_eq!(error.status, axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(error.code, "invalid_credentials");
        let limited = application
            .login(login_id, "wrong password")
            .await
            .unwrap_err();
        assert_eq!(limited.status, axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(limited.code, "login_rate_limited");
        assert_eq!(
            limited.message, "login cannot be attempted again yet",
            "rate-limit response must not reveal account existence"
        );
        limited_responses.push((limited.code, limited.message));
    }
    assert_eq!(limited_responses[0], limited_responses[1]);
    let variant = application
        .login(" OWNER ", "wrong password")
        .await
        .expect_err("login identifier variants must share one failure bucket");
    assert_eq!(variant.status, axum::http::StatusCode::TOO_MANY_REQUESTS);

    let recovering = StorageWebApplication::with_login_policy(
        storage,
        LoginPolicy {
            max_failures: 1,
            failure_window: std::time::Duration::from_millis(20),
            max_concurrent: 1,
            max_tracked_accounts: 16,
        },
    );
    assert_eq!(
        recovering
            .login("missing", "wrong password")
            .await
            .unwrap_err()
            .status,
        axum::http::StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        recovering
            .login("missing", "wrong password")
            .await
            .unwrap_err()
            .status,
        axum::http::StatusCode::TOO_MANY_REQUESTS
    );
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let recovered = recovering
        .login("missing", "wrong password")
        .await
        .expect_err("unknown account remains invalid after recovery");
    assert_eq!(recovered.status, axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_login_performs_the_same_password_verification_work() {
    let directory = test_directory();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("login-timing.db"),
    })
    .await
    .unwrap();
    AccountService::new(storage.clone())
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new("long enough owner password").unwrap(),
            1_700_000_000_000,
        )
        .await
        .unwrap();
    let application = StorageWebApplication::new(storage);

    let known_started = std::time::Instant::now();
    application
        .login("owner", "wrong password")
        .await
        .expect_err("known account password is invalid");
    let known_elapsed = known_started.elapsed();

    let unknown_started = std::time::Instant::now();
    application
        .login("missing", "wrong password")
        .await
        .expect_err("unknown account is invalid");
    let unknown_elapsed = unknown_started.elapsed();

    assert!(
        unknown_elapsed.saturating_mul(2) >= known_elapsed,
        "unknown login returned in {unknown_elapsed:?}, known login took {known_elapsed:?}"
    );
}

#[tokio::test]
async fn mutation_dispatch_preserves_put_and_delete_semantics() {
    let directory = test_directory();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("methods.db"),
    })
    .await
    .unwrap();
    AccountService::new(storage.clone())
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new("long enough owner password").unwrap(),
            1_700_000_000_000,
        )
        .await
        .unwrap();
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage.apply_descriptor(&descriptor, 1).await.unwrap();
    let rule = Semantics::new(storage.clone())
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: descriptor.edge_node_id,
                series_key: descriptor.signals[0].series_key.clone(),
                display_name: "Temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            1,
        )
        .await
        .unwrap();
    let application = StorageWebApplication::new(storage.clone());
    let principal = application
        .login("owner", "long enough owner password")
        .await
        .unwrap()
        .principal;

    let invalid_put = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id.clone())]),
                expected_revision: Some(1),
            },
            serde_json::json!({}),
        )
        .await
        .expect_err("an empty PUT must not be interpreted as DELETE");
    assert_eq!(invalid_put.status, axum::http::StatusCode::BAD_REQUEST);
    assert!(
        storage
            .list_semantic_rules()
            .await
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.rule_id == rule.rule_id)
            .unwrap()
            .active
    );

    let calibrated = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/signals/{}/calibration", rule.signal_ref),
                params: HashMap::from([("signal_ref".into(), rule.signal_ref.clone())]),
                expected_revision: Some(1),
            },
            serde_json::json!({"scale":2.0,"offset":1.0}),
        )
        .await
        .expect("matching calibration revision updates the signal");
    assert_eq!(calibrated.body["revision"], 2);
    let stale_calibration = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/signals/{}/calibration", rule.signal_ref),
                params: HashMap::from([("signal_ref".into(), rule.signal_ref.clone())]),
                expected_revision: Some(1),
            },
            serde_json::json!({"scale":3.0,"offset":2.0}),
        )
        .await
        .expect_err("stale calibration revision must fail");
    assert_eq!(
        stale_calibration.status,
        axum::http::StatusCode::PRECONDITION_FAILED
    );

    let revised = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id.clone())]),
                expected_revision: Some(1),
            },
            serde_json::json!({"display_name":"Temperature revised","kind":"numeric"}),
        )
        .await
        .expect("matching revision updates the rule");
    assert_eq!(revised.body["revision"], 2);
    let stale = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id.clone())]),
                expected_revision: Some(1),
            },
            serde_json::json!({"display_name":"Stale update","kind":"numeric"}),
        )
        .await
        .expect_err("stale revision must fail");
    assert_eq!(stale.status, axum::http::StatusCode::PRECONDITION_FAILED);

    application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::DELETE,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id.clone())]),
                expected_revision: Some(2),
            },
            serde_json::json!({}),
        )
        .await
        .expect("DELETE retires the rule");
    assert!(
        !storage
            .list_semantic_rules()
            .await
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.rule_id == rule.rule_id)
            .unwrap()
            .active
    );
}

#[tokio::test]
async fn semantic_rule_listing_preserves_the_current_change_processing_spec() {
    let directory = test_directory();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("semantic-rule-spec.db"),
    })
    .await
    .unwrap();
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage.apply_descriptor(&descriptor, 1).await.unwrap();
    let expected = RuleSpec {
        kind: SemanticKind::CumulativeCounter,
        detector: Detector {
            mode: iotkit_edge::semantics::DetectorMode::HighActive,
            rise_threshold: 12.5,
            fall_threshold: 8.0,
            rise_debounce_ms: 1_500,
            fall_debounce_ms: 2_500,
        },
        trigger: TriggerMode::OnNotification,
    };
    let created = Semantics::new(storage.clone())
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: descriptor.edge_node_id,
                series_key: descriptor.signals[0].series_key.clone(),
                display_name: "Production count".into(),
                spec: expected,
            },
            2,
        )
        .await
        .unwrap();

    let loaded = storage
        .list_semantic_rules()
        .await
        .unwrap()
        .into_iter()
        .find(|rule| rule.rule_id == created.rule_id)
        .unwrap();
    assert_eq!(loaded.spec, expected);
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN"]
async fn postgres_enforces_the_same_web_revision_precondition() {
    let dsn = std::env::var("IOTKIT_TEST_POSTGRES_DSN").expect("PostgreSQL DSN");
    let storage = Storage::connect(StorageProfile::Postgres { dsn })
        .await
        .unwrap();
    storage
        .initialize_edge_identity(1_700_000_000_000)
        .await
        .unwrap();
    AccountService::new(storage.clone())
        .create_initial_system_admin(
            "postgres-test",
            "PostgreSQL Test",
            Password::new("long enough postgres password").unwrap(),
            1_700_000_000_000,
        )
        .await
        .unwrap();
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage.apply_descriptor(&descriptor, 1).await.unwrap();
    let rule = Semantics::new(storage.clone())
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: descriptor.edge_node_id,
                series_key: descriptor.signals[0].series_key.clone(),
                display_name: format!("Temperature {unique}"),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            1,
        )
        .await
        .unwrap();
    assert!(
        storage
            .list_semantic_rules()
            .await
            .unwrap()
            .iter()
            .any(|candidate| candidate.rule_id == rule.rule_id),
        "new PostgreSQL semantic rule must be visible before revision"
    );
    let application = StorageWebApplication::new(storage);
    let login = application
        .login("postgres-test", "long enough postgres password")
        .await
        .unwrap();
    let revised = application
        .mutate(
            &login.principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id.clone())]),
                expected_revision: Some(1),
            },
            serde_json::json!({"display_name":"PostgreSQL revised","kind":"numeric"}),
        )
        .await
        .unwrap();
    assert_eq!(revised.body["revision"], 2);
    let stale = application
        .mutate(
            &login.principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id)]),
                expected_revision: Some(1),
            },
            serde_json::json!({"display_name":"Stale","kind":"numeric"}),
        )
        .await
        .unwrap_err();
    assert_eq!(stale.status, axum::http::StatusCode::PRECONDITION_FAILED);
}

#[tokio::test]
async fn production_web_diagnostics_use_the_runtime_threshold_and_certificate_file() {
    let directory = test_directory();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("diagnostics.db"),
    })
    .await
    .unwrap();
    let certificate = directory.path().join("missing-broker.pem");
    let application = StorageWebApplication::with_runtime_settings(storage, 73, Some(certificate));
    let status = application
        .query(ApiQuery::Named {
            route: "/api/v1/system/storage".into(),
            params: HashMap::new(),
        })
        .await
        .unwrap();
    assert_eq!(status["warning_percent"], 73);
    let report = application
        .query(ApiQuery::Named {
            route: "/api/v1/system/diagnostics".into(),
            params: HashMap::new(),
        })
        .await
        .unwrap();
    assert_eq!(report["broker_certificate"]["available"], false);
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["code"] == "broker_certificate_unavailable")
    );
}
