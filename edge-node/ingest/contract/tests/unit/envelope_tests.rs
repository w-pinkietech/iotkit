use super::*;
use crate::ack::{AckStatus, Disposition, EnvelopeAck, ItemStatus};

fn sample_envelope() -> Envelope {
    Envelope {
        envelope_id: "gw-1-1".into(),
        source: "bravepi-mainboard".into(),
        declaration_version: None,
        items: vec![ReadingItem {
            subject_hint: Some("ble:00000000000000ab".into()),
            measurement_key: "temperature_c".into(),
            channel_index: None,
            series_variant: None,
            values: vec![21.5],
            device_time_ms: None,
            time_source: TimeSource::EdgeNode,
            age_ms: None,
            rssi: Some(-60),
            battery_pct: Some(88),
        }],
    }
}

#[test]
fn envelope_json_round_trip() {
    let e = sample_envelope();
    let json = serde_json::to_string(&e).unwrap();
    assert_eq!(serde_json::from_str::<Envelope>(&json).unwrap(), e);
    assert!(json.contains("\"time_source\":\"edge_node\""));
    // オプショナル欄は省略される(ワイヤの軽さ)
    assert!(!json.contains("device_time_ms"));
}

#[test]
fn edge_node_adjusted_time_source_uses_edge_node_adjusted_wire_value() {
    let json = serde_json::to_string(&TimeSource::EdgeNodeAdjusted).unwrap();
    assert_eq!(json, "\"edge_node_adjusted\"");
    assert_eq!(
        serde_json::from_str::<TimeSource>(&json).unwrap(),
        TimeSource::EdgeNodeAdjusted
    );
}

#[test]
fn legacy_gateway_time_sources_are_rejected() {
    assert!(serde_json::from_str::<TimeSource>("\"gateway\"").is_err());
    assert!(serde_json::from_str::<TimeSource>("\"gateway_adjusted\"").is_err());
}

#[test]
fn v1_ignores_unknown_object_fields_but_rejects_unknown_enum_values() {
    let original = sample_envelope();
    let mut value = serde_json::to_value(&original).unwrap();
    value["future_envelope_field"] = serde_json::json!(true);
    value["items"][0]["future_item_field"] = serde_json::json!(true);

    assert_eq!(serde_json::from_value::<Envelope>(value).unwrap(), original);
    assert!(serde_json::from_str::<TimeSource>("\"future_time_source\"").is_err());
}

#[test]
fn ack_json_round_trip() {
    let ack = EnvelopeAck {
        envelope_id: "gw-1-1".into(),
        status: AckStatus::Accepted {
            items: vec![ItemStatus::Stored {
                disposition: Disposition::Quarantined,
                quarantine_reason: None,
            }],
        },
    };
    let json = serde_json::to_string(&ack).unwrap();
    assert!(json.contains("\"disposition\":\"quarantined\""));
    assert_eq!(serde_json::from_str::<EnvelopeAck>(&json).unwrap(), ack);
}
