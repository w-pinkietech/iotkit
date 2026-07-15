use iotkit_core_publish::descriptor::{DescriptorDevice, DescriptorSnapshot, MAX_DESCRIPTOR_BYTES};

#[test]
fn strict_decode_accepts_fixture_and_rejects_unknown_or_inconsistent_content() {
    let fixture = std::fs::read("../../testdata/egress/v1/descriptor-snapshot.json").unwrap();
    let snapshot = DescriptorSnapshot::decode_bounded(&fixture).unwrap();
    assert_eq!(snapshot.descriptor_revision, 4);

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
fn bounded_encoding_rejects_oversize_without_truncation() {
    let mut snapshot = DescriptorSnapshot {
        schema_version: 1,
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
    });

    assert!(snapshot.encode_bounded().is_err());
    assert_eq!(snapshot.devices.len(), 1, "oversize data was truncated");
}
