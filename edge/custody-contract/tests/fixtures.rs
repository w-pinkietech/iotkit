use iotkit_edge_custody_contract::{
    AcceptedThrough, ActivationRequest, ActivationResult, DescriptorSnapshot, RecordBatch,
};

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .expect("read fixture")
}

#[test]
fn decodes_the_shared_record_batch_and_ack_fixtures() {
    let batch = RecordBatch::decode(&fixture("testdata/egress/v1/record-batch.json"))
        .expect("decode record batch");
    assert_eq!(batch.edge_node_id, "edge-node-01");
    assert_eq!(batch.cursor_start, 1);
    assert_eq!(batch.cursor_end, 1);
    assert_eq!(batch.records.len(), 1);

    let ack = AcceptedThrough::decode(&fixture("testdata/egress/v1/accepted-through.json"))
        .expect("decode acknowledgement");
    ack.validate_for(&batch, 0)
        .expect("acknowledgement matches batch");
}

#[test]
fn decodes_descriptor_and_activation_fixtures_strictly() {
    let descriptor =
        DescriptorSnapshot::decode(&fixture("testdata/egress/v2/descriptor-snapshot.json"))
            .expect("decode descriptor");
    assert_eq!(descriptor.descriptor_revision, 5);
    assert!(!descriptor.devices.is_empty());
    assert!(!descriptor.signals.is_empty());

    ActivationRequest::decode(&fixture("testdata/egress/v1/activation-request.json"))
        .expect("decode activation request");
    ActivationResult::decode(&fixture("testdata/egress/v1/activation-result.json"))
        .expect("decode activation result");
}

#[test]
fn rejects_unknown_fields_and_invalid_activation_boundaries() {
    assert!(
        ActivationRequest::decode(&fixture(
            "testdata/egress/v1/activation-request-unknown-field.json"
        ))
        .is_err()
    );
    assert!(
        ActivationRequest::decode(&fixture(
            "testdata/egress/v1/activation-request-malformed-id.json"
        ))
        .is_err()
    );
    assert!(
        ActivationResult::decode(&fixture(
            "testdata/egress/v1/activation-result-first-seq-2.json"
        ))
        .is_err()
    );
}

#[test]
fn rejects_topic_body_identity_mismatches() {
    let batch = RecordBatch::decode(&fixture("testdata/egress/v1/record-batch.json"))
        .expect("decode record batch");
    assert!(batch.validate_topic_edge_node("edge-node-other").is_err());

    let result = ActivationResult::decode(&fixture("testdata/egress/v1/activation-result.json"))
        .expect("decode result");
    assert!(result.validate_topic_edge_node("edge-node-other").is_err());
}

#[test]
fn rejects_noncanonical_descriptor_fields_before_persistence() {
    type Mutation = (&'static str, Box<dyn Fn(&mut serde_json::Value)>);

    let original = fixture("testdata/egress/v2/descriptor-snapshot.json");
    let mut cases: Vec<Mutation> = vec![
        (
            "revision outside SQL range",
            Box::new(|value| value["descriptor_revision"] = (i64::MAX as u64 + 1).into()),
        ),
        (
            "empty identifier",
            Box::new(|value| value["devices"][0]["identifier"] = "".into()),
        ),
        (
            "invalid model",
            Box::new(|value| value["devices"][0]["model_id"] = "MCP9600".into()),
        ),
        (
            "control character in unit",
            Box::new(|value| value["signals"][0]["unit"] = "deg\nC".into()),
        ),
        (
            "noncanonical measurement key",
            Box::new(|value| {
                value["signals"][0]["measurement_key"] = "ContactState".into();
                value["signals"][0]["series_key"] =
                    "018f0000-0000-7000-8000-000000000001:ContactState:na:primary".into();
            }),
        ),
        (
            "negative channel",
            Box::new(|value| {
                value["signals"][0]["channel_index"] = (-1).into();
                value["signals"][0]["series_key"] =
                    "018f0000-0000-7000-8000-000000000001:contact_state:-1:primary".into();
            }),
        ),
        (
            "empty variant",
            Box::new(|value| {
                value["signals"][0]["variant"] = "".into();
                value["signals"][0]["series_key"] =
                    "018f0000-0000-7000-8000-000000000001:contact_state:na:".into();
            }),
        ),
        (
            "variant containing the identity separator",
            Box::new(|value| {
                value["signals"][0]["variant"] = "primary:extra".into();
                value["signals"][0]["series_key"] =
                    "018f0000-0000-7000-8000-000000000001:contact_state:na:primary:extra".into();
            }),
        ),
    ];

    for (name, mutate) in cases.drain(..) {
        let mut value: serde_json::Value =
            serde_json::from_slice(&original).expect("decode descriptor JSON");
        mutate(&mut value);
        let encoded = serde_json::to_vec(&value).expect("encode descriptor mutation");
        assert!(
            DescriptorSnapshot::decode(&encoded).is_err(),
            "{name} was accepted"
        );
    }
}
