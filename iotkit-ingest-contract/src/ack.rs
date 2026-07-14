use serde::{Deserialize, Serialize};

/// An acknowledgement for one submitted ingest envelope.
///
/// The receiver returns it only after reaching the outcome represented by
/// [`AckStatus`]; storage or commit failure produces no acknowledgement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeAck {
    /// The identifier of the envelope whose outcome is being reported.
    ///
    /// The receiver copies the submitted identifier so a sender can match
    /// acknowledgements to queued envelopes.
    pub envelope_id: String,
    /// The envelope-level ingest outcome.
    ///
    /// Senders use this value to decide whether the queued envelope is complete
    /// or must be resent unchanged.
    pub status: AckStatus,
}

/// The receiver's outcome for an entire envelope.
///
/// It distinguishes committed processing, duplicate suppression, terminal
/// contract rejection, and temporary overload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AckStatus {
    /// Processing was committed at the Edge acknowledgement durability point.
    ///
    /// The sender treats the envelope as complete; individual entries describe
    /// stored and terminally rejected items.
    Accepted {
        /// Outcomes positionally aligned with the submitted items.
        ///
        /// This vector has exactly the same length and order as
        /// [`Envelope::items`](crate::Envelope::items); no separate item index is
        /// carried.
        items: Vec<ItemStatus>,
    },
    /// The envelope was already handled within the receiver's deduplication window.
    ///
    /// The sender treats the envelope as complete and removes any spooled copy;
    /// the receiver does not process its items again.
    Duplicate,
    /// The entire envelope has a deterministic contract violation.
    ///
    /// This outcome is TERMINAL: the sender removes the envelope from its spool,
    /// records the failure, and fixes future input instead of blind-retrying it.
    Rejected {
        /// The machine-readable class of deterministic violation.
        ///
        /// Senders use it for corrective action and metrics rather than retry
        /// timing.
        reason_code: ReasonCode,
        /// A human-readable explanation of the violation.
        ///
        /// It is diagnostic text; sender behavior is determined by the status and
        /// reason code, not by parsing this string.
        message: String,
        /// Optional JSON Pointer locating the invalid envelope field.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        field_path: Option<String>,
        /// Optional stable hint describing the expected schema or value shape.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema_hint: Option<String>,
    },
    /// The receiver is temporarily overloaded and has not accepted custody.
    ///
    /// The sender resends the identical envelope, including its identifier, with
    /// backoff and jitter. Persistent storage failure instead produces no
    /// acknowledgement.
    Deferred,
}

/// The receiver's outcome for one item in an accepted envelope.
///
/// These outcomes allow valid items to be retained without making a malformed
/// item cause an endlessly retried poison batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ItemStatus {
    /// The receiver retained the item under the reported custody class.
    ///
    /// Consult the disposition before assuming the item is eligible for
    /// downstream delivery or indefinite retention.
    Stored {
        /// The custody and downstream-delivery state assigned by the receiver.
        disposition: Disposition,
        /// The optional explanation for a quarantined disposition.
        ///
        /// It is omitted from JSON when absent and may be absent when reading an
        /// older quarantined acknowledgement.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quarantine_reason: Option<QuarantineReason>,
    },
    /// The item has a deterministic contract violation within an accepted envelope.
    ///
    /// The outcome is TERMINAL for this submitted item. The sender does not retry
    /// the envelope and instead fixes subsequent observations according to the
    /// reason code.
    ItemRejected {
        /// The machine-readable class of deterministic item violation.
        reason_code: ReasonCode,
        /// A human-readable explanation of the item violation.
        ///
        /// It is intended for diagnostics and must not be parsed to choose retry
        /// behavior.
        message: String,
        /// Optional JSON Pointer locating the invalid item field.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        field_path: Option<String>,
        /// Optional stable hint describing the expected schema or value shape.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema_hint: Option<String>,
    },
}

/// The reason a stored item is quarantined.
///
/// The receiver exposes this value so operators can resolve data that is visible
/// at IoTKit Edge but withheld from downstream delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuarantineReason {
    /// The numeric observation falls outside the applicable accepted range.
    ///
    /// The receiver stores the observation but withholds the row from downstream
    /// delivery for operator review.
    OutOfRange,
    /// The measurement key is syntactically valid but has no registered definition.
    ///
    /// The receiver stores it in quarantine so an operator can define or map the
    /// key later.
    UnknownKey,
    /// The item names a channel that the measurement declaration does not allow.
    ///
    /// The receiver stores it in quarantine until the declaration or mapping is
    /// corrected.
    UndeclaredChannel,
    /// The subject itself is currently in the Edge's quarantined state.
    ///
    /// The receiver stores the observation visibly but prevents downstream
    /// delivery until the subject is approved.
    DeviceQuarantined,
}

impl QuarantineReason {
    /// Returns the canonical snake-case representation of this reason.
    ///
    /// The receiver uses the same string for the wire representation and stored
    /// quarantine metadata.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OutOfRange => "out_of_range",
            Self::UnknownKey => "unknown_key",
            Self::UndeclaredChannel => "undeclared_channel",
            Self::DeviceQuarantined => "device_quarantined",
        }
    }
}

/// The custody and delivery state of a stored item.
///
/// The three states are mutually exclusive; subject staging is decided before
/// registry validation and therefore takes precedence over quarantine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// IoTKit Edge now owns the retained data at the acknowledgement durability point.
    ///
    /// The item is eligible for normal downstream delivery and the sender may
    /// discard its spooled copy.
    Durable,
    /// An unknown hardware observation is held in a bounded staging buffer.
    ///
    /// Custody is provisional: the observation may be evicted if the subject is
    /// not approved, while its deduplication entry can remain until its own TTL.
    Staged,
    /// The observation is stored and visible at IoTKit Edge but isolated from delivery.
    ///
    /// IoTKit Edge retains it for operator resolution and withholds it from
    /// downstream consumers.
    Quarantined,
}

/// A machine-readable reason for terminal contract rejection.
///
/// Except for the unused legacy `Internal` value, each code identifies input that
/// produces the same violation when resent unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    /// The measurement key violates the required grammar or length limit.
    ///
    /// This is TERMINAL: fix the key, do not blind-retry, and remove the rejected
    /// envelope from a spool.
    MalformedMeasurementKey,
    /// The values cannot be interpreted as the declared measurement type.
    ///
    /// This includes a wrong value count, invalid Boolean or integer values, and
    /// non-finite numbers. It is TERMINAL: fix the payload or declaration, do not
    /// blind-retry, and remove the rejected envelope from a spool.
    ValueTypeMismatch,
    /// The receiver cannot resolve a required subject identity from the item.
    ///
    /// IoTKit Edge produces this TERMINAL item rejection for multi-subject
    /// omission and for unknown subjects from externally authenticated device
    /// principals. One-subject omission resolves from receiver-owned principal
    /// scope; only trusted official adapters can stage unknown sightings.
    UnknownSubject,
    /// The authenticated sender is not authorized for the resolved subject.
    ///
    /// This is TERMINAL: correct the subject or sender authorization, do not
    /// blind-retry, and remove the rejected envelope from a spool.
    SubjectScopeViolation,
    /// The envelope exceeds the receiver's batch item or byte limit.
    ///
    /// This is TERMINAL for the unchanged envelope: split or reduce the batch, do
    /// not blind-retry, and remove the rejected envelope from a spool.
    BatchTooLarge,
    /// The observation timestamp is older than the configured ingest freshness window.
    ///
    /// This is TERMINAL for the unchanged observation: correct an erroneous clock
    /// value or record the expired sample as failed, do not blind-retry, and remove
    /// the rejected envelope from a spool.
    StaleTimestamp,
    /// A legacy reason with no D1-conforming production condition.
    ///
    /// The current receiver never constructs this value, and storage or commit
    /// failures must produce no acknowledgement instead. If received with a
    /// rejected status, preserve that status's TERMINAL handling, remove the
    /// envelope from a spool, and report a receiver defect rather than blind-retry.
    #[deprecated(
        note = "read-only v1 vocabulary; producers must return no ack on internal failure"
    )]
    #[serde(skip_serializing)]
    Internal,
}

#[cfg(test)]
mod tests {
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
}
