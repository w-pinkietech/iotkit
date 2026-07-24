use std::{
    env,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use iotkit_edge::{
    application::{
        output_profiles::OutputProfiles,
        profiles::{DeviceProfileInput, InventoryProfiles, SignalProfileInput},
        semantics::{SemanticRuleDraft, Semantics},
    },
    composition::registered_output_adapters,
    semantics::{Detector, DetectorMode, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, AuditActor, RawRecord, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::{ActivationRequest, ActivationResult, DescriptorSnapshot};
use serde_json::Map;

#[tokio::main]
async fn main() {
    let fixture_started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("fixture clock must be after Unix epoch")
        .as_millis() as i64;
    let mut args = env::args().skip(1);
    let profile = args.next().expect("storage profile");
    let location = args.next().expect("storage location");
    let storage = Storage::connect(match profile.as_str() {
        "embedded" => StorageProfile::Sqlite {
            path: PathBuf::from(location),
        },
        "postgres" => StorageProfile::Postgres { dsn: location },
        _ => panic!("storage profile must be embedded or postgres"),
    })
    .await
    .expect("connect fixture storage");
    storage
        .initialize_edge_identity(fixture_started_at)
        .await
        .expect("initialize Edge identity");
    let descriptor = DescriptorSnapshot::decode(
        br#"{
          "schema_version": 2,
          "edge_node_id": "edge-node-01",
          "ledger_epoch": "epoch-01",
          "descriptor_revision": 1,
          "complete": true,
          "devices": [{
            "system_id": "018f0000-0000-7000-8000-000000000001",
            "identifier": "01234567",
            "state": "active",
            "model_id": "mcp9600"
          }, {
            "system_id": "018f0000-0000-7000-8000-000000000002",
            "identifier": "01234568",
            "state": "active",
            "model_id": "opt3001"
          }, {
            "system_id": "018f0000-0000-7000-8000-000000000003",
            "identifier": "01234569",
            "state": "active",
            "model_id": "bravepi-contact-input"
          }],
          "signals": [{
            "series_key": "018f0000-0000-7000-8000-000000000001:temperature_c:na:primary",
            "system_id": "018f0000-0000-7000-8000-000000000001",
            "measurement_key": "temperature_c",
            "channel_index": null,
            "variant": "primary",
            "unit": "Cel",
            "value_type": "float"
          }, {
            "series_key": "018f0000-0000-7000-8000-000000000002:illuminance_lux:na:primary",
            "system_id": "018f0000-0000-7000-8000-000000000002",
            "measurement_key": "illuminance_lux",
            "channel_index": null,
            "variant": "primary",
            "unit": "lx",
            "value_type": "float"
          }, {
            "series_key": "018f0000-0000-7000-8000-000000000003:contact_state:na:primary",
            "system_id": "018f0000-0000-7000-8000-000000000003",
            "measurement_key": "contact_state",
            "channel_index": null,
            "variant": "primary",
            "unit": "1",
            "value_type": "bool"
          }]
        }"#,
    )
    .expect("decode Console descriptor fixture");
    storage
        .apply_descriptor(&descriptor, fixture_started_at)
        .await
        .expect("apply descriptor");
    let command = storage
        .request_activation(&descriptor.edge_node_id, fixture_started_at + 1)
        .await
        .expect("request fixture activation");
    let request =
        ActivationRequest::decode(&command.payload_json).expect("decode fixture activation");
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
                applied_at: fixture_started_at + 2,
            },
            fixture_started_at + 2,
        )
        .await
        .expect("complete fixture activation");
    let profiles = InventoryProfiles::new(storage.clone());
    for (identifier, display_name) in [
        ("01234567", "乾燥炉入口 熱電対変換器"),
        ("01234568", "製造機 青色パトランプ照度センサー"),
        ("01234569", "プレス機 稼働接点"),
    ] {
        let device = profiles
            .devices()
            .await
            .expect("list fixture devices")
            .into_iter()
            .find(|device| device.identifier == identifier)
            .expect("fixture device");
        profiles
            .update_device(
                AuditActor::local_cli(),
                &device.device_ref,
                DeviceProfileInput {
                    display_name: display_name.into(),
                    location: "デモ設備".into(),
                },
                None,
                fixture_started_at + 3,
            )
            .await
            .expect("profile fixture device");
    }
    for (measurement_key, profile) in [
        (
            "temperature_c",
            SignalProfileInput {
                display_name: "乾燥炉入口 温度".into(),
                display_sensor_type: "thermocouple".into(),
                display_sensor_type_label: String::new(),
                display_value_kind: "numeric".into(),
                display_unit_mode: "unit".into(),
                display_unit: "°C".into(),
                decimal_places: 1,
            },
        ),
        (
            "illuminance_lux",
            SignalProfileInput {
                display_name: "製造機 青色パトランプ".into(),
                display_sensor_type: "illuminance".into(),
                display_sensor_type_label: String::new(),
                display_value_kind: "numeric".into(),
                display_unit_mode: "unit".into(),
                display_unit: "lx".into(),
                decimal_places: 0,
            },
        ),
        (
            "contact_state",
            SignalProfileInput {
                display_name: "プレス機 稼働接点".into(),
                display_sensor_type: "contact".into(),
                display_sensor_type_label: String::new(),
                display_value_kind: "boolean".into(),
                display_unit_mode: "dimensionless".into(),
                display_unit: String::new(),
                decimal_places: 0,
            },
        ),
    ] {
        let signal = profiles
            .signals()
            .await
            .expect("list fixture signals")
            .into_iter()
            .find(|signal| signal.measurement_key == measurement_key)
            .expect("fixture signal");
        profiles
            .update_signal(
                AuditActor::local_cli(),
                &signal.signal_ref,
                profile,
                None,
                fixture_started_at + 4,
            )
            .await
            .expect("profile fixture signal");
    }
    accept(
        &storage,
        1,
        &descriptor.signals[0].series_key,
        20.5,
        fixture_started_at + 5,
    )
    .await;
    accept(
        &storage,
        2,
        &descriptor.signals[1].series_key,
        120.0,
        fixture_started_at + 6,
    )
    .await;
    accept(
        &storage,
        3,
        &descriptor.signals[2].series_key,
        0.0,
        fixture_started_at + 7,
    )
    .await;
    let semantics = Semantics::new(storage.clone());
    for (series_key, display_name, spec) in [
        (
            descriptor.signals[0].series_key.clone(),
            "現在温度",
            RuleSpec {
                kind: SemanticKind::Numeric,
                detector: Detector::default(),
                trigger: TriggerMode::None,
            },
        ),
        (
            descriptor.signals[0].series_key.clone(),
            "高温アラーム",
            analog_rule(SemanticKind::Alarm, 30.0, 28.0, TriggerMode::None),
        ),
        (
            descriptor.signals[1].series_key.clone(),
            "製造中（青灯点灯）",
            analog_rule(SemanticKind::Boolean, 500.0, 300.0, TriggerMode::None),
        ),
        (
            descriptor.signals[1].series_key.clone(),
            "製造サイクル回数",
            analog_rule(
                SemanticKind::CumulativeCounter,
                500.0,
                300.0,
                TriggerMode::OnTransition,
            ),
        ),
        (
            descriptor.signals[2].series_key.clone(),
            "設備稼働",
            boolean_rule(
                SemanticKind::Boolean,
                DetectorMode::BooleanHighActive,
                TriggerMode::None,
            ),
        ),
        (
            descriptor.signals[2].series_key.clone(),
            "稼働開始回数",
            boolean_rule(
                SemanticKind::CumulativeCounter,
                DetectorMode::BooleanHighActive,
                TriggerMode::OnTransition,
            ),
        ),
        (
            descriptor.signals[2].series_key.clone(),
            "設備停止アラーム",
            boolean_rule(
                SemanticKind::Alarm,
                DetectorMode::BooleanLowActive,
                TriggerMode::None,
            ),
        ),
    ] {
        semantics
            .create_rule(
                SemanticRuleDraft {
                    edge_node_id: descriptor.edge_node_id.clone(),
                    series_key,
                    display_name: display_name.into(),
                    spec,
                },
                fixture_started_at + 10,
            )
            .await
            .expect("create semantic fixture");
    }
    OutputProfiles::new(storage.clone(), registered_output_adapters())
        .activate(
            "IoTKit MQTT 出力",
            "iotkit.mqtt-json.v1",
            Map::new(),
            fixture_started_at + 20,
        )
        .await
        .expect("create output fixture");
    for (sequence, series_index, value) in [
        (4, 0, 28.5),
        // Natural light while the machine waits.
        (5, 1, 140.0),
        (6, 1, 145.0),
        // Blue patrol lamp on while the machine manufactures.
        (7, 1, 680.0),
        (8, 1, 690.0),
        (9, 1, 700.0),
        // Waiting, manufacturing, waiting, and manufacturing repeat.
        (10, 1, 155.0),
        (11, 1, 150.0),
        (12, 1, 670.0),
        (13, 1, 690.0),
        (14, 1, 165.0),
        (15, 1, 150.0),
        (16, 1, 710.0),
        (17, 1, 700.0),
        (18, 1, 145.0),
        (19, 2, 1.0),
        (20, 2, 0.0),
        (21, 2, 1.0),
        (22, 0, 31.0),
        (23, 0, 29.0),
    ] {
        accept(
            &storage,
            sequence,
            &descriptor.signals[series_index].series_key,
            value,
            fixture_started_at + 20 + sequence * 1_000,
        )
        .await;
    }
    for _ in 0..100 {
        let progress = semantics
            .project_pending(10, registered_output_adapters())
            .await
            .expect("project semantic fixture");
        if progress.receipts == 0 {
            break;
        }
    }
    println!("{}", storage.edge_id().await.expect("read Edge identity"));
}

fn analog_rule(
    kind: SemanticKind,
    rise_threshold: f64,
    fall_threshold: f64,
    trigger: TriggerMode,
) -> RuleSpec {
    RuleSpec {
        kind,
        detector: Detector {
            mode: DetectorMode::HighActive,
            rise_threshold,
            fall_threshold,
            rise_debounce_ms: 0,
            fall_debounce_ms: 0,
        },
        trigger,
    }
}

fn boolean_rule(kind: SemanticKind, mode: DetectorMode, trigger: TriggerMode) -> RuleSpec {
    RuleSpec {
        kind,
        detector: Detector {
            mode,
            rise_threshold: 0.0,
            fall_threshold: 0.0,
            rise_debounce_ms: 0,
            fall_debounce_ms: 0,
        },
        trigger,
    }
}

async fn accept(storage: &Storage, sequence: i64, series_key: &str, value: f64, received_at: i64) {
    let record = serde_json::json!({
        "family": "measurement",
        "schema_version": 1,
        "epoch": "epoch-01",
        "pub_seq": sequence,
        "series_key": series_key,
        "values": [value],
        "event_time": received_at,
        "event_time_source": "received_at",
        "time_source": "edge_node",
        "time_quality": "unsynced",
        "received_at": received_at,
        "device_time": null
    });
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: format!("console-fixture-{sequence}"),
            received_at,
            records: vec![RawRecord::new(sequence, serde_json::to_vec(&record).unwrap()).unwrap()],
        })
        .await
        .expect("accept fixture record");
}
