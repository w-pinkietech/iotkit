use std::{fs, path::PathBuf, time::Duration};

use iotkit_edge::{
    application::{
        output_profiles::OutputProfiles,
        semantics::{SemanticRuleDraft, Semantics},
    },
    composition::registered_output_adapters,
    mqtt::output::{DeliveryAction, DeliveryTracker, OutputRuntime, OutputRuntimeConfig},
    semantics::{Detector, DetectorMode, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, OutputMark, RawRecord, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use rumqttc::MqttOptions;
use serde_json::Map;
use tokio_util::sync::CancellationToken;

#[test]
fn queued_publish_is_not_delivery_and_only_matching_puback_marks() {
    let mut tracker = DeliveryTracker::new("export-01", "claim-01");
    assert_eq!(tracker.queued(), DeliveryAction::None);
    assert_eq!(tracker.outgoing_publish(17), DeliveryAction::None);
    assert_eq!(tracker.incoming_puback(18), DeliveryAction::None);
    assert_eq!(
        tracker.incoming_puback(17),
        DeliveryAction::MarkPublished {
            export_id: "export-01".into(),
            claim_token: "claim-01".into(),
        }
    );
    assert_eq!(tracker.incoming_puback(17), DeliveryAction::None);
}

#[test]
fn packet_identifier_is_ephemeral_and_never_part_of_the_claim_identity() {
    let mut first_session = DeliveryTracker::new("export-01", "claim-01");
    let _ = first_session.outgoing_publish(7);
    let mut reconnected_session = DeliveryTracker::new("export-01", "claim-01");
    let _ = reconnected_session.outgoing_publish(42);
    assert_eq!(
        reconnected_session.incoming_puback(42),
        DeliveryAction::MarkPublished {
            export_id: "export-01".into(),
            claim_token: "claim-01".into(),
        }
    );
}

#[tokio::test]
#[ignore = "requires a real Mosquitto broker; run scripts/test-edge-output.sh"]
async fn actual_mosquitto_outage_retries_same_durable_export_until_puback() {
    assert!(
        std::env::var_os("IOTKIT_REQUIRE_RUST_OUTPUT_GATE").is_some(),
        "the real broker gate must explicitly require its environment"
    );
    let storage = if let Ok(dsn) = std::env::var("IOTKIT_TEST_RUST_OUTPUT_POSTGRES_DSN") {
        Storage::connect(StorageProfile::Postgres { dsn })
            .await
            .expect("connect gate PostgreSQL")
    } else {
        let path = PathBuf::from(
            std::env::var("IOTKIT_TEST_RUST_OUTPUT_SQLITE")
                .expect("IOTKIT_TEST_RUST_OUTPUT_SQLITE"),
        );
        Storage::connect(StorageProfile::Sqlite { path })
            .await
            .expect("connect gate SQLite")
    };
    storage
        .initialize_edge_identity(1_720_000_000_000)
        .await
        .expect("initialize identity");
    let series_key = "018f0000-0000-7000-8000-000000000001:contact:na:primary";
    let descriptor = DescriptorSnapshot::decode(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "edge_node_id": "edge-output-gate",
            "ledger_epoch": "gate-epoch",
            "descriptor_revision": 1,
            "complete": true,
            "devices": [{
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "identifier": "output-gate-device",
                "state": "active",
                "model_id": "contract"
            }],
            "signals": [{
                "series_key": series_key,
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "measurement_key": "contact",
                "channel_index": null,
                "variant": "primary",
                "unit": null,
                "value_type": "float"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    storage
        .apply_descriptor(&descriptor, 1_720_000_000_000)
        .await
        .expect("apply descriptor");
    let semantics = Semantics::new(storage.clone());
    let rule = semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "edge-output-gate".into(),
                series_key: series_key.into(),
                display_name: "Gate production counter".into(),
                spec: RuleSpec {
                    kind: SemanticKind::CumulativeCounter,
                    detector: Detector {
                        mode: DetectorMode::BooleanHighActive,
                        ..Detector::default()
                    },
                    trigger: TriggerMode::OnTransition,
                },
            },
            1,
        )
        .await
        .expect("create semantic rule");
    let profiles = OutputProfiles::new(storage.clone(), registered_output_adapters());
    profiles
        .activate("Gate MQTT", "iotkit.mqtt-json.v1", Map::new(), 2)
        .await
        .expect("activate output profile");
    let pinikiet = profiles
        .activate("Gate Pinikiet", "pinikiet.mqtt.v1", Map::new(), 3)
        .await
        .expect("prepare Pinikiet output profile");
    let pinikiet_binding = pinikiet
        .bindings
        .iter()
        .find(|binding| binding.rule_id == rule.rule_id)
        .expect("Pinikiet production binding");
    profiles
        .confirm(&pinikiet_binding.binding_id, 4)
        .await
        .expect("confirm Pinikiet sensor identity");
    let record = |sequence: i64, value: f64| {
        serde_json::json!({
            "family": "measurement",
            "schema_version": 1,
            "epoch": "gate-epoch",
            "pub_seq": sequence,
            "series_key": series_key,
            "values": [value],
            "event_time": 1_720_000_000_003_i64 + sequence,
            "event_time_source": "received_at",
            "time_source": "edge_node",
            "time_quality": "unsynced",
            "received_at": 1_720_000_000_003_i64 + sequence,
            "device_time": null
        })
    };
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-output-gate".into(),
            ledger_epoch: "gate-epoch".into(),
            publication_id: "gate-publication".into(),
            received_at: 1_720_000_000_003,
            records: vec![
                RawRecord::new(
                    1,
                    serde_json::to_vec(&record(1, 0.0)).expect("serialize record"),
                )
                .expect("valid record"),
                RawRecord::new(
                    2,
                    serde_json::to_vec(&record(2, 1.0)).expect("serialize record"),
                )
                .expect("valid record"),
            ],
        })
        .await
        .expect("accept record");
    semantics
        .project_pending(10, registered_output_adapters())
        .await
        .expect("project durable output");
    assert_eq!(storage.pending_output_count().await.unwrap(), 2);
    let first = storage
        .claim_output("identity-probe-1", 1_800_000_000_000, 1)
        .await
        .expect("inspect durable export")
        .expect("first durable export");
    let second = storage
        .claim_output("identity-probe-2", 1_800_000_000_000, 1)
        .await
        .expect("inspect durable export")
        .expect("second durable export");
    let (generic_export_id, pinikiet_export_id) = if first.topic.starts_with("iotkit/") {
        (first.export_id.clone(), second.export_id.clone())
    } else {
        (second.export_id.clone(), first.export_id.clone())
    };
    assert!(first.topic.starts_with("iotkit/") || first.topic.starts_with("pinikiet/"));
    assert!(second.topic.starts_with("iotkit/") || second.topic.starts_with("pinikiet/"));
    assert_ne!(
        first.topic.starts_with("iotkit/"),
        second.topic.starts_with("iotkit/"),
        "the live outage must cover one generic and one Pinikiet publication"
    );
    assert!(
        storage
            .release_output(&first.export_id, "identity-probe-1")
            .await
            .expect("release identity probe")
    );
    assert!(
        storage
            .release_output(&second.export_id, "identity-probe-2")
            .await
            .expect("release identity probe")
    );
    let control = PathBuf::from(
        std::env::var("IOTKIT_TEST_OUTPUT_CONTROL_DIR").expect("output control directory"),
    );
    fs::write(control.join("ready"), b"ready\n").expect("write ready marker");
    tokio::time::timeout(Duration::from_secs(30), async {
        while !control.join("broker-down").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("broker outage marker");

    let host =
        std::env::var("IOTKIT_TEST_OUTPUT_BROKER_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("IOTKIT_TEST_OUTPUT_BROKER_PORT")
        .expect("IOTKIT_TEST_OUTPUT_BROKER_PORT")
        .parse::<u16>()
        .expect("valid broker port");
    let password =
        std::env::var("IOTKIT_TEST_OUTPUT_PASSWORD").expect("IOTKIT_TEST_OUTPUT_PASSWORD");
    let mut mqtt = MqttOptions::new("iotkit-rust-output-gate", host, port);
    mqtt.set_credentials("edge-output", password);
    mqtt.set_keep_alive(Duration::from_secs(2));
    let cancellation = CancellationToken::new();
    let runtime = tokio::spawn(
        OutputRuntime::new(
            storage.clone(),
            OutputRuntimeConfig {
                mqtt,
                request_capacity: 2,
                claim_lease: Duration::from_secs(30),
                idle_poll: Duration::from_millis(20),
                reconnect_delay: Duration::from_millis(50),
            },
        )
        .run(cancellation.clone()),
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        storage.pending_output_count().await.unwrap(),
        2,
        "broker outage must leave both durable exports pending"
    );
    fs::write(control.join("pending"), b"pending\n").expect("write pending marker");
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if storage.pending_output_count().await.expect("pending count") == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("PUBACK did not mark the durable export");
    assert_eq!(
        storage
            .mark_output_published(&generic_export_id, "identity-probe-1", 1_800_000_000_001,)
            .await
            .expect("inspect published export"),
        OutputMark::Published,
        "reconnect must complete the original generic durable export identity"
    );
    assert_eq!(
        storage
            .mark_output_published(&pinikiet_export_id, "identity-probe-2", 1_800_000_000_001,)
            .await
            .expect("inspect published export"),
        OutputMark::Published,
        "reconnect must complete the original Pinikiet durable export identity"
    );
    cancellation.cancel();
    runtime
        .await
        .expect("join output runtime")
        .expect("output runtime");
}
