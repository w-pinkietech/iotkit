use std::{collections::HashMap, path::PathBuf};

use iotkit_edge::{
    application::{
        accounts::AccountService,
        output_profiles::OutputProfiles,
        profiles::{InventoryProfiles, SignalProfileInput},
        semantics::{SemanticRuleDraft, Semantics},
    },
    auth::password::Password,
    composition::{StorageWebApplication, registered_output_adapters},
    semantics::{Detector, DetectorMode, RuleSpec, SemanticKind, TriggerMode},
    storage::{AuditActor, Storage, StorageProfile},
    web::{ConsoleRequest, Principal, WebApplication},
};
use iotkit_edge_custody_contract::{ActivationRequest, ActivationResult, DescriptorSnapshot};
use serde_json::Map;
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};

fn test_directory() -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::TempDir::new_in(root).unwrap()
}

struct Fixture {
    _directory: tempfile::TempDir,
    database_path: PathBuf,
    storage: Storage,
    principal: Principal,
    edge_node_id: String,
    series_key: String,
}

async fn fixture(database_name: &str) -> Fixture {
    let directory = test_directory();
    let database_path = PathBuf::from(directory.path()).join(database_name);
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: database_path.clone(),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let owner = AccountService::new(storage.clone())
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new(uuid::Uuid::new_v4().to_string()).unwrap(),
            2,
        )
        .await
        .unwrap();

    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage.apply_descriptor(&descriptor, 3).await.unwrap();
    let command = storage
        .request_activation(&descriptor.edge_node_id, 4)
        .await
        .unwrap();
    let activation = ActivationRequest::decode(&command.payload_json).unwrap();
    storage
        .apply_activation_result(
            &ActivationResult {
                schema_version: 1,
                activation_id: activation.activation_id,
                edge_id: activation.edge_id,
                edge_node_id: activation.edge_node_id,
                ledger_epoch: activation.expected_ledger_epoch,
                status: "applied".into(),
                discard_through_reading_seq: 0,
                first_publication_seq: 1,
                applied_at: 5,
            },
            5,
        )
        .await
        .unwrap();
    let inventory = InventoryProfiles::new(storage.clone());
    let signal = inventory.signals().await.unwrap().remove(0);
    inventory
        .update_signal(
            AuditActor::local_cli(),
            &signal.signal_ref,
            SignalProfileInput {
                display_name: "乾燥炉入口 接点".into(),
                display_sensor_type: "contact".into(),
                display_sensor_type_label: String::new(),
                display_value_kind: "boolean".into(),
                display_unit_mode: "dimensionless".into(),
                display_unit: String::new(),
                decimal_places: 0,
            },
            None,
            6,
        )
        .await
        .unwrap();
    let principal = Principal {
        account_ref: owner.account_ref,
        login_id: owner.login_id,
        display_name: owner.display_name,
        role: owner.role.as_str().into(),
        state: owner.state.as_str().into(),
        must_change_password: owner.must_change_password,
        revision: owner.revision,
        created_at: owner.created_at,
        updated_at: owner.updated_at,
    };
    Fixture {
        _directory: directory,
        database_path,
        storage,
        principal,
        edge_node_id: descriptor.edge_node_id,
        series_key: signal.series_key,
    }
}

async fn create_rule(fixture: &Fixture, display_name: &str, kind: SemanticKind) {
    Semantics::new(fixture.storage.clone())
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: fixture.edge_node_id.clone(),
                series_key: fixture.series_key.clone(),
                display_name: display_name.into(),
                spec: RuleSpec {
                    kind,
                    detector: if kind == SemanticKind::Boolean {
                        Detector {
                            mode: DetectorMode::BooleanHighActive,
                            ..Detector::default()
                        }
                    } else {
                        Detector::default()
                    },
                    trigger: TriggerMode::None,
                },
            },
            7,
        )
        .await
        .unwrap();
}

async fn direct_connection(fixture: &Fixture) -> SqliteConnection {
    SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&fixture.database_path)
            .create_if_missing(false),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn production_console_composes_live_output_delivery_facts_and_frees_stopped_adapter() {
    let fixture = fixture("output-console-read-model.db").await;
    create_rule(&fixture, "Temperature", SemanticKind::Numeric).await;
    let output_profiles =
        OutputProfiles::new(fixture.storage.clone(), registered_output_adapters());
    let profile = output_profiles
        .activate("Generic", "iotkit.mqtt-json.v1", Map::new(), 8)
        .await
        .unwrap();

    let application = StorageWebApplication::new(fixture.storage.clone());
    let view = application
        .console(ConsoleRequest {
            path: "/output".into(),
            query: HashMap::new(),
            principal: fixture.principal.clone(),
        })
        .await
        .unwrap();

    assert_eq!(view.output_summary.sending_count, 1);
    assert_eq!(view.output_summary.needs_configuration_count, 0);
    assert_eq!(view.output_summary.delivery_problem_count, 0);
    assert_eq!(view.outputs.iter().filter(|item| item.active).count(), 1);

    let binding = &view
        .outputs
        .iter()
        .find(|item| item.active)
        .unwrap()
        .bindings[0];
    assert_eq!(binding.rule_name, "Temperature");
    assert_eq!(binding.state_label, "最初の値を待っています");
    assert!(binding.topic.starts_with("iotkit/v1/sources/"));
    assert!(binding.payload.contains("\"schema_version\""));
    assert!(!binding.signal_ref.is_empty());
    assert!(!binding.series_id.is_empty());
    assert_eq!(binding.sensor_name, "乾燥炉入口 接点");
    assert_ne!(binding.sensor_name, fixture.series_key);

    output_profiles.stop(&profile.profile_id, 9).await.unwrap();
    let stopped = application
        .console(ConsoleRequest {
            path: "/output".into(),
            query: HashMap::new(),
            principal: fixture.principal,
        })
        .await
        .unwrap();
    let generic: Vec<_> = stopped
        .outputs
        .iter()
        .filter(|item| item.adapter_id == "iotkit.mqtt-json.v1")
        .collect();
    assert_eq!(generic.len(), 1);
    assert!(!generic[0].active);
    assert!(generic[0].profile_id.is_empty());
}

#[tokio::test]
async fn production_console_localizes_available_output_adapter_names() {
    let fixture = fixture("output-console-available-adapter-names.db").await;
    let view = StorageWebApplication::new(fixture.storage)
        .console(ConsoleRequest {
            path: "/output".into(),
            query: HashMap::new(),
            principal: fixture.principal,
        })
        .await
        .expect("compose output console");

    let generic = view
        .outputs
        .iter()
        .find(|output| output.adapter_id == "iotkit.mqtt-json.v1")
        .expect("generic MQTT output is available");
    assert!(!generic.active);
    assert!(!generic.draining);
    assert_eq!(generic.display_name, "汎用MQTT JSONで送る");
    assert_eq!(generic.adapter_name, "IoTKit MQTT JSON v1");

    let pinikiet = view
        .outputs
        .iter()
        .find(|output| output.adapter_id == "pinikiet.mqtt.v1")
        .expect("Pinikiet output is available");
    assert!(!pinikiet.active);
    assert!(!pinikiet.draining);
    assert_eq!(pinikiet.display_name, "Pinikietへ送る");
    assert_eq!(pinikiet.adapter_name, "Pinikiet MQTT v1");
}

#[tokio::test]
async fn production_console_exposes_real_mode_options_and_configuration_advances() {
    let fixture = fixture("output-console-mode-options.db").await;
    create_rule(&fixture, "設備稼働", SemanticKind::Boolean).await;
    let output_profiles =
        OutputProfiles::new(fixture.storage.clone(), registered_output_adapters());
    let profile = output_profiles
        .activate("Pinikiet", "pinikiet.mqtt.v1", Map::new(), 8)
        .await
        .unwrap();
    let binding_id = profile
        .bindings
        .iter()
        .find(|binding| binding.needs_configuration)
        .unwrap()
        .binding_id
        .clone();
    let application = StorageWebApplication::new(fixture.storage.clone());

    let needs_configuration = application
        .console(ConsoleRequest {
            path: "/output".into(),
            query: HashMap::new(),
            principal: fixture.principal.clone(),
        })
        .await
        .unwrap();
    let binding = needs_configuration
        .outputs
        .iter()
        .find(|output| output.adapter_id == "pinikiet.mqtt.v1" && output.active)
        .unwrap()
        .bindings
        .iter()
        .find(|binding| binding.binding_id == binding_id)
        .unwrap();
    assert_eq!(binding.sensor_name, "乾燥炉入口 接点");
    assert_eq!(
        binding
            .compatible_modes
            .iter()
            .map(|mode| (mode.key.as_str(), mode.display_name.as_str()))
            .collect::<Vec<_>>(),
        vec![("onoff", "ON/OFF"), ("gantt_chart", "稼働状態")]
    );
    assert_eq!(binding.revision, profile.revision);
    assert!(binding.configuration_required);

    output_profiles
        .configure(&binding_id, "onoff", Map::new(), 9)
        .await
        .unwrap();
    let prepared = application
        .console(ConsoleRequest {
            path: "/output".into(),
            query: HashMap::new(),
            principal: fixture.principal,
        })
        .await
        .unwrap();
    let output = prepared
        .outputs
        .iter()
        .find(|output| output.adapter_id == "pinikiet.mqtt.v1" && output.active)
        .unwrap();
    let binding = output
        .bindings
        .iter()
        .find(|binding| binding.binding_id == binding_id)
        .unwrap();
    assert_eq!(binding.state_label, "外部登録待ち");
    assert!(binding.prepared);
    assert!(!binding.needs_configuration);
    assert_eq!(output.status_label, "外部登録待ち");
    assert_eq!(prepared.output_summary.needs_configuration_count, 1);
}

#[tokio::test]
async fn non_output_console_page_does_not_read_invalid_output_profiles() {
    let fixture = fixture("output-console-page-gating.db").await;
    create_rule(&fixture, "Temperature", SemanticKind::Numeric).await;
    OutputProfiles::new(fixture.storage.clone(), registered_output_adapters())
        .activate("Generic", "iotkit.mqtt-json.v1", Map::new(), 8)
        .await
        .unwrap();
    let mut connection = direct_connection(&fixture).await;
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("UPDATE export_profiles SET state='invalid'")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();

    let view = StorageWebApplication::new(fixture.storage)
        .console(ConsoleRequest {
            path: "/status".into(),
            query: HashMap::new(),
            principal: fixture.principal,
        })
        .await
        .unwrap();

    assert!(view.outputs.is_empty());
    assert_eq!(view.output_summary.sending_count, 0);
    assert_eq!(view.output_summary.needs_configuration_count, 0);
    assert_eq!(view.output_summary.delivery_problem_count, 0);
}

#[tokio::test]
async fn production_console_localizes_transform_and_delivery_read_failures() {
    let fixture = fixture("output-console-safe-failure.db").await;
    create_rule(&fixture, "Temperature", SemanticKind::Numeric).await;
    let output_profiles =
        OutputProfiles::new(fixture.storage.clone(), registered_output_adapters());
    let profile = output_profiles
        .activate("Generic", "iotkit.mqtt-json.v1", Map::new(), 8)
        .await
        .unwrap();
    let binding_id = &profile.bindings[0].binding_id;
    let mut connection = direct_connection(&fixture).await;
    sqlx::query("UPDATE output_routes SET config_json=? WHERE binding_id=?")
        .bind(br#"{"schema_version":99,"topic":"invalid"}"#.to_vec())
        .bind(binding_id)
        .execute(&mut connection)
        .await
        .unwrap();
    let application = StorageWebApplication::new(fixture.storage.clone());

    let transform_failure = application
        .console(ConsoleRequest {
            path: "/output".into(),
            query: HashMap::new(),
            principal: fixture.principal.clone(),
        })
        .await
        .unwrap();
    let binding = &transform_failure
        .outputs
        .iter()
        .find(|output| output.adapter_id == "iotkit.mqtt-json.v1" && output.active)
        .unwrap()
        .bindings[0];
    assert_eq!(binding.state_label, "変換エラー");
    assert_eq!(binding.technical_error, "送信内容を確認できません");
    assert!(binding.needs_configuration);
    assert!(!binding.configuration_required);
    assert!(!binding.delivery_problem);

    sqlx::query("DELETE FROM output_routes WHERE binding_id=?")
        .bind(binding_id)
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    let delivery_failure = application
        .console(ConsoleRequest {
            path: "/output".into(),
            query: HashMap::new(),
            principal: fixture.principal,
        })
        .await
        .unwrap();
    let output = delivery_failure
        .outputs
        .iter()
        .find(|output| output.adapter_id == "iotkit.mqtt-json.v1" && output.active)
        .unwrap();
    let binding = &output.bindings[0];
    assert_eq!(binding.state_label, "配送状態を確認できません");
    assert_eq!(binding.technical_error, "配送状態を確認できません");
    assert!(!binding.needs_configuration);
    assert!(binding.delivery_problem);
    assert!(binding.delivery_unavailable);
    assert_eq!(output.status_label, "配送状態を確認できません");
    assert_eq!(delivery_failure.output_summary.delivery_problem_count, 1);
    assert!(!binding.technical_error.contains("not found"));
}
