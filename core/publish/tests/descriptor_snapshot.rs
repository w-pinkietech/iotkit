use iotkit_core_publish::descriptor::{DescriptorDevice, DescriptorSnapshot, MAX_DESCRIPTOR_BYTES};

#[test]
fn strict_decode_accepts_fixture_and_rejects_unknown_or_inconsistent_content() {
    let fixture = std::fs::read("../../testdata/egress/v2/descriptor-snapshot.json").unwrap();
    let snapshot = DescriptorSnapshot::decode_bounded(&fixture).unwrap();
    assert_eq!(snapshot.descriptor_revision, 5);
    assert_eq!(snapshot.devices[0].model_id.as_deref(), Some("mcp9600"));

    let mut unknown: serde_json::Value = serde_json::from_slice(&fixture).unwrap();
    unknown["provider_payload"] = serde_json::json!({"secret": true});
    assert!(DescriptorSnapshot::decode_bounded(&serde_json::to_vec(&unknown).unwrap()).is_err());

    let mut inconsistent: serde_json::Value = serde_json::from_slice(&fixture).unwrap();
    inconsistent["signals"][0]["series_key"] = serde_json::json!("wrong");
    assert!(
        DescriptorSnapshot::decode_bounded(&serde_json::to_vec(&inconsistent).unwrap()).is_err()
    );
}

#[test]
fn schema_two_accepts_valid_model_id_and_schema_one_is_unsupported() {
    let fixture = std::fs::read("../../testdata/egress/v2/descriptor-snapshot.json").unwrap();
    let snapshot = DescriptorSnapshot::decode_bounded(&fixture).unwrap();
    assert_eq!(snapshot.schema_version, 2);
    assert_eq!(snapshot.devices[0].model_id.as_deref(), Some("mcp9600"));

    let mut unsupported_v1: serde_json::Value = serde_json::from_slice(&fixture).unwrap();
    unsupported_v1["schema_version"] = serde_json::json!(1);
    unsupported_v1["devices"][0]
        .as_object_mut()
        .unwrap()
        .remove("model_id");
    assert!(
        DescriptorSnapshot::decode_bounded(&serde_json::to_vec(&unsupported_v1).unwrap()).is_err()
    );
}

#[test]
fn schema_two_rejects_non_canonical_model_ids() {
    let fixture = std::fs::read("../../testdata/egress/v2/descriptor-snapshot.json").unwrap();
    for invalid in [
        "",
        "MCP9600",
        "-mcp9600",
        "vendor//model",
        "model id",
        "model-",
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(&fixture).unwrap();
        value["devices"][0]["model_id"] = serde_json::json!(invalid);
        assert!(
            DescriptorSnapshot::decode_bounded(&serde_json::to_vec(&value).unwrap()).is_err(),
            "accepted invalid model_id {invalid:?}"
        );
    }
}

#[test]
fn bounded_encoding_rejects_oversize_without_truncation() {
    let mut snapshot = DescriptorSnapshot {
        schema_version: 2,
        edge_node_id: "edge-node-01".into(),
        ledger_epoch: "epoch-01".into(),
        descriptor_revision: 1,
        complete: true,
        devices: Vec::new(),
        signals: Vec::new(),
    };
    snapshot.devices.push(DescriptorDevice {
        system_id: "x".repeat(MAX_DESCRIPTOR_BYTES),
        identifier: None,
        state: "active".into(),
        model_id: None,
    });

    assert!(snapshot.encode_bounded().is_err());
    assert_eq!(snapshot.devices.len(), 1, "oversize data was truncated");
}
