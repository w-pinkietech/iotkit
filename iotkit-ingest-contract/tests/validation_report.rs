use iotkit_ingest_contract::{ReasonCode, ValidationIssue, ValidationReport};

#[test]
fn validation_report_is_a_distinct_non_custody_wire_type() {
    let report = ValidationReport {
        envelope_id: "e-validate".into(),
        valid: false,
        issues: vec![ValidationIssue {
            item_index: Some(1),
            reason_code: ReasonCode::UnknownSubject,
            message: "subject is unknown".into(),
            field_path: Some("/items/1/subject_hint".into()),
            schema_hint: Some("registered subject identifier".into()),
        }],
    };
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["valid"], false);
    assert!(
        json.get("status").is_none(),
        "must not resemble EnvelopeAck"
    );
    assert!(
        json.get("custody").is_none(),
        "validation never claims custody"
    );
    assert_eq!(
        serde_json::from_value::<ValidationReport>(json).unwrap(),
        report
    );
}
