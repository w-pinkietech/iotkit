use iotkit_core_recovery::{
    RecoveryActivationRequest, RecoveryActivationResult, RecoveryCompletion, RecoveryCompletionAck,
};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("testdata/egress/v1")
            .join(name),
    )
    .expect("read fixture")
}

#[test]
fn edge_node_decodes_the_shared_recovery_control_fixtures() {
    let request = RecoveryActivationRequest::decode(&fixture("recovery-activation-request.json"))
        .expect("decode recovery request");
    let result = RecoveryActivationResult::decode(&fixture("recovery-activation-result.json"))
        .expect("decode recovery result");
    let completion = RecoveryCompletion::decode(&fixture("recovery-completion.json"))
        .expect("decode recovery completion");
    let acknowledgement = RecoveryCompletionAck::decode(&fixture("recovery-completion-ack.json"))
        .expect("decode recovery completion acknowledgement");

    result.validate_for(&request).expect("matching result");
    completion
        .validate_for(&request)
        .expect("matching completion");
    assert_eq!(acknowledgement.recovery_id, request.recovery_id);
}

#[test]
fn edge_node_rejects_unknown_recovery_fields() {
    assert!(
        RecoveryActivationRequest::decode(&fixture(
            "recovery-activation-request-unknown-field.json"
        ))
        .is_err()
    );
}
