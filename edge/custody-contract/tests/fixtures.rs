use iotkit_edge_custody_contract::{
    AcceptedThrough, ActivationRequest, ActivationResult, DescriptorSnapshot, RecordBatch,
    RecoveryActivationRequest, RecoveryActivationResult, RecoveryCompletion, RecoveryCompletionAck,
    StatusHeartbeat,
};

type StatusMutation = Box<dyn Fn(&mut serde_json::Value)>;

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
fn decodes_the_shared_status_heartbeat_fixture_strictly() {
    let heartbeat = StatusHeartbeat::decode(&fixture("testdata/egress/v1/status-heartbeat.json"))
        .expect("decode status heartbeat");
    heartbeat
        .validate_topic_edge_node("edge-node-01")
        .expect("topic identity matches");
    assert_eq!(heartbeat.status_seq, 1);
    assert_eq!(heartbeat.adapters.len(), 1);
}

#[test]
fn status_heartbeat_rejects_secrets_and_noncanonical_boundaries() {
    let original = fixture("testdata/egress/v1/status-heartbeat.json");
    let cases: Vec<(&str, StatusMutation)> = vec![
        (
            "unknown secret field",
            Box::new(|value| value["password"] = "must-not-cross-mqtt".into()),
        ),
        (
            "unsafe boot identity",
            Box::new(|value| value["boot_id"] = "boot:unsafe".into()),
        ),
        (
            "zero sequence",
            Box::new(|value| value["status_seq"] = 0.into()),
        ),
        (
            "negative custody cursor",
            Box::new(|value| value["accepted_through"] = (-1).into()),
        ),
        (
            "duplicate adapter",
            Box::new(|value| {
                let adapter = value["adapters"][0].clone();
                value["adapters"].as_array_mut().unwrap().push(adapter);
            }),
        ),
        (
            "more than 64 adapters",
            Box::new(|value| {
                let adapters = value["adapters"].as_array_mut().unwrap();
                while adapters.len() <= 64 {
                    let number = adapters.len();
                    adapters.push(serde_json::json!({
                        "adapter_id": format!("adapter-{number}"),
                        "state": "running"
                    }));
                }
            }),
        ),
    ];

    for (name, mutate) in cases {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        mutate(&mut value);
        assert!(
            StatusHeartbeat::decode(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{name} was accepted"
        );
    }
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
fn decodes_the_shared_recovery_control_fixtures_strictly() {
    let request = RecoveryActivationRequest::decode(&fixture(
        "testdata/egress/v1/recovery-activation-request.json",
    ))
    .expect("decode recovery activation request");
    assert_eq!(request.edge_accepted_through, 45);

    let result = RecoveryActivationResult::decode(&fixture(
        "testdata/egress/v1/recovery-activation-result.json",
    ))
    .expect("decode recovery activation result");
    result
        .validate_for(&request)
        .expect("result matches recovery request");

    let completion =
        RecoveryCompletion::decode(&fixture("testdata/egress/v1/recovery-completion.json"))
            .expect("decode recovery completion");
    completion
        .validate_for(&request)
        .expect("completion matches recovery request");
    let acknowledgement =
        RecoveryCompletionAck::decode(&fixture("testdata/egress/v1/recovery-completion-ack.json"))
            .expect("decode recovery completion acknowledgement");
    assert_eq!(acknowledgement.recovery_id, request.recovery_id);
}

#[test]
fn rejects_unknown_recovery_fields_and_inconsistent_boundaries() {
    assert!(
        RecoveryActivationRequest::decode(&fixture(
            "testdata/egress/v1/recovery-activation-request-unknown-field.json",
        ))
        .is_err()
    );

    let bytes = fixture("testdata/egress/v1/recovery-activation-request.json");
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["edge_accepted_through"] = 39.into();
    assert!(RecoveryActivationRequest::decode(&serde_json::to_vec(&value).unwrap()).is_err());

    let bytes = fixture("testdata/egress/v1/recovery-activation-result.json");
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["last_new_publication_seq"] = 5.into();
    assert!(RecoveryActivationResult::decode(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut overflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    overflow["replayed_records"] = i64::MAX.into();
    overflow["last_new_publication_seq"] = i64::MAX.into();
    assert!(RecoveryActivationResult::decode(&serde_json::to_vec(&overflow).unwrap()).is_err());
}

#[test]
fn rejects_topic_body_identity_mismatches() {
    let batch = RecordBatch::decode(&fixture("testdata/egress/v1/record-batch.json"))
        .expect("decode record batch");
    assert!(batch.validate_topic_edge_node("edge-node-other").is_err());

    let result = ActivationResult::decode(&fixture("testdata/egress/v1/activation-result.json"))
        .expect("decode result");
    assert!(result.validate_topic_edge_node("edge-node-other").is_err());

    let recovery_result = RecoveryActivationResult::decode(&fixture(
        "testdata/egress/v1/recovery-activation-result.json",
    ))
    .expect("decode recovery result");
    assert!(
        recovery_result
            .validate_topic_edge_node("edge-node-other")
            .is_err()
    );
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
