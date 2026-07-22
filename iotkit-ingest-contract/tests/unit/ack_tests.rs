use super::*;

mod frozen_v1_reader {
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    pub struct EnvelopeAck {
        pub envelope_id: String,
        pub status: AckStatus,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "snake_case", tag = "kind")]
    pub enum AckStatus {
        Accepted {
            items: Vec<ItemStatus>,
        },
        Duplicate,
        Rejected {
            reason_code: ReasonCode,
            message: String,
        },
        Deferred,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "snake_case", tag = "kind")]
    pub enum ItemStatus {
        Stored {
            disposition: Disposition,
            #[serde(default)]
            quarantine_reason: Option<QuarantineReason>,
        },
        ItemRejected {
            reason_code: ReasonCode,
            message: String,
        },
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "snake_case")]
    pub enum Disposition {
        Durable,
        Staged,
        Quarantined,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "snake_case")]
    pub enum QuarantineReason {
        OutOfRange,
        UnknownKey,
        UndeclaredChannel,
        DeviceQuarantined,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "snake_case")]
    pub enum ReasonCode {
        MalformedMeasurementKey,
        ValueTypeMismatch,
        UnknownSubject,
        SubjectScopeViolation,
        BatchTooLarge,
        StaleTimestamp,
        Internal,
    }
}

#[test]
fn rejection_details_are_absent_compatible_and_old_v1_reader_ignores_them() {
    let envelope = EnvelopeAck {
        envelope_id: "e-envelope".into(),
        status: AckStatus::Rejected {
            reason_code: ReasonCode::SubjectScopeViolation,
            message: "configured source does not match principal".into(),
            field_path: Some("/source".into()),
            schema_hint: Some("source must match the authenticated principal".into()),
        },
    };
    let json = serde_json::to_string(&envelope).unwrap();
    let old: frozen_v1_reader::EnvelopeAck = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        old.status,
        frozen_v1_reader::AckStatus::Rejected { .. }
    ));

    let item = ItemStatus::ItemRejected {
        reason_code: ReasonCode::UnknownSubject,
        message: "subject is unknown".into(),
        field_path: Some("/items/0/subject_hint".into()),
        schema_hint: Some("registered subject identifier".into()),
    };
    let json = serde_json::to_string(&item).unwrap();
    let old: frozen_v1_reader::ItemStatus = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        old,
        frozen_v1_reader::ItemStatus::ItemRejected { .. }
    ));

    let old_rejected = r#"{"kind":"rejected","reason_code":"unknown_subject","message":"old"}"#;
    assert!(matches!(
        serde_json::from_str::<AckStatus>(old_rejected).unwrap(),
        AckStatus::Rejected {
            field_path: None,
            schema_hint: None,
            ..
        }
    ));
}

#[test]
fn legacy_internal_is_read_compatible() {
    let json = r#"{"kind":"rejected","reason_code":"internal","message":"legacy"}"#;
    let parsed = serde_json::from_str::<AckStatus>(json).unwrap();
    assert!(
        serde_json::to_string(&parsed).is_err(),
        "read-only legacy Internal must not be emitted by a producer"
    );
}

#[test]
fn stored_reason_is_additive_on_the_wire() {
    let s = ItemStatus::Stored {
        disposition: Disposition::Durable,
        quarantine_reason: None,
    };
    let json = serde_json::to_string(&s).unwrap();
    assert!(
        !json.contains("quarantine_reason"),
        "additive: 省略時はワイヤに現れない"
    );
    // 旧形式(フィールドなし)のJSONも読める
    let old: ItemStatus =
        serde_json::from_str(r#"{"kind":"stored","disposition":"quarantined"}"#).unwrap();
    assert!(matches!(
        old,
        ItemStatus::Stored {
            quarantine_reason: None,
            ..
        }
    ));
    let with: ItemStatus = serde_json::from_str(
        r#"{"kind":"stored","disposition":"quarantined","quarantine_reason":"out_of_range"}"#,
    )
    .unwrap();
    assert!(matches!(
        with,
        ItemStatus::Stored {
            quarantine_reason: Some(QuarantineReason::OutOfRange),
            ..
        }
    ));
}
