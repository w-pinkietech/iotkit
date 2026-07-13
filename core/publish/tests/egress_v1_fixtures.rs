use iotkit_core_publish::wire::{AcceptedThrough, RecordBatch};

const BATCH_FIXTURE: &str = include_str!("../../../testdata/egress/v1/record-batch.json");
const ACK_FIXTURE: &str = include_str!("../../../testdata/egress/v1/accepted-through.json");

#[test]
fn rust_decodes_and_validates_v1_batch_fixture() {
    let batch: RecordBatch = serde_json::from_str(BATCH_FIXTURE).unwrap();
    batch.validate().unwrap();
    assert_eq!(batch.gateway_identity, "gateway-01");
    assert_eq!(batch.cursor_start, 1);
    assert_eq!(batch.cursor_end, 1);
    assert_eq!(batch.records.len(), 1);
}

#[test]
fn rust_decodes_and_correlates_v1_ack_fixture() {
    let batch: RecordBatch = serde_json::from_str(BATCH_FIXTURE).unwrap();
    let ack: AcceptedThrough = serde_json::from_str(ACK_FIXTURE).unwrap();
    ack.validate_for(&batch, 0).unwrap();
}

#[test]
fn ack_for_another_gateway_is_rejected() {
    let batch: RecordBatch = serde_json::from_str(BATCH_FIXTURE).unwrap();
    let mut ack: AcceptedThrough = serde_json::from_str(ACK_FIXTURE).unwrap();
    ack.gateway_identity = "gateway-other".into();
    assert!(ack.validate_for(&batch, 0).is_err());
}

#[test]
fn puback_cannot_be_represented_as_application_ack() {
    let transport_only = r#"{"packet_id":7}"#;
    assert!(serde_json::from_str::<AcceptedThrough>(transport_only).is_err());
}
