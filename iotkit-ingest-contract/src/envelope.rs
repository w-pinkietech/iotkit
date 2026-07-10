use serde::{Deserialize, Serialize};

/// A sender-defined batch submitted as one deduplication and acknowledgement unit.
///
/// All fields are required on the wire except `declaration_version`. A resend
/// must preserve the entire envelope, especially its identifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// A sender-generated identifier for this acknowledgement unit.
    ///
    /// It must remain unchanged across retries. The receiver deduplicates by the
    /// authenticated sender identity together with this value, within a bounded
    /// deduplication window; it is not globally unique by itself.
    pub envelope_id: String,
    /// A required self-description of the sender.
    ///
    /// The receiver uses it for diagnostics and as the in-process sender
    /// identity. Authenticated bindings derive authorization and deduplication
    /// identity from credentials rather than trusting this value.
    pub source: String,
    /// An optional version of the sender's accompanying declaration.
    ///
    /// It is omitted from JSON when absent; the current collector carries no
    /// declaration-dependent ingest behavior for this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration_version: Option<u32>,
    /// The required batch of observations handled by this envelope.
    ///
    /// A single observation is represented by a one-element batch. An accepted
    /// acknowledgement returns exactly one item status per entry, in this order.
    pub items: Vec<ReadingItem>,
}

/// One measurement observation within an ingest envelope.
///
/// Subject identity, measurement identity, numeric values, and time provenance
/// are required; timestamps and radio or battery metadata are optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingItem {
    /// The hardware identifier naming the observed subject.
    ///
    /// Multi-subject senders, where one sender identity writes for several subjects
    /// (for example, a parent adapter bundling child devices), MUST supply this hint
    /// per item. The D5 decision 1 contract allows senders whose token maps 1:1 to a
    /// single subject to omit it.
    ///
    /// Resolution of the 1:1 omission is not yet implemented. The gateway currently
    /// rejects any missing `subject_hint` terminally with
    /// [`ReasonCode::UnknownSubject`](crate::ReasonCode::UnknownSubject), and a
    /// spooling sender deletes the envelope. Until network ingress ships that
    /// resolution, ALWAYS supply `subject_hint`. A supplied but unknown identifier
    /// is accepted into staging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_hint: Option<String>,
    /// The required canonical key identifying the measured quantity.
    ///
    /// The receiver validates it with [`validate_measurement_key`](crate::validate_measurement_key)
    /// before registry lookup.
    pub measurement_key: String,
    /// An optional logical channel within the measurement.
    ///
    /// Omit it when channel selection is not applicable; the receiver validates
    /// supplied indices against the measurement declaration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_index: Option<u16>,
    /// An optional variant distinguishing parallel series for the same key and channel.
    ///
    /// The receiver uses `primary` when this field is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_variant: Option<String>,
    /// The required numeric payload of the observation.
    ///
    /// The receiver validates its length and values against the registered
    /// measurement type; every value must be finite.
    pub values: Vec<f64>,
    /// An optional device-supplied Unix timestamp in milliseconds.
    ///
    /// A missing device timestamp is valid. When present with a device-derived
    /// time source, the receiver considers it before the collector receive time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_time_ms: Option<i64>,
    /// The required provenance tag for the observation time.
    ///
    /// The receiver uses it to decide whether a supplied or reconstructed time is
    /// an event-time candidate.
    pub time_source: TimeSource,
    /// An optional sender-reported age of the observation in milliseconds.
    ///
    /// The collector considers it only when `device_time_ms` is absent. If the
    /// value converts to `i64` and checked subtraction succeeds, the collector
    /// reconstructs the observation time as receive time minus this duration and
    /// records the effective source as [`TimeSource::GatewayAdjusted`]. If either
    /// operation fails, it ignores `age_ms` and event time falls back to receive
    /// time. When `device_time_ms` is present, it takes precedence and `age_ms` is
    /// ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<u64>,
    /// Optional sender-provided received-signal-strength metadata.
    ///
    /// The receiver stores it with the observation when present and does not
    /// require it for ingest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rssi: Option<i16>,
    /// Optional sender-provided battery percentage metadata.
    ///
    /// The receiver stores it with the observation when present and does not
    /// require it for ingest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_pct: Option<u8>,
}

/// The provenance of an observation's timestamp.
///
/// The receiver carries this tag into storage unless successful `age_ms`
/// reconstruction replaces it with [`TimeSource::GatewayAdjusted`]. It uses
/// device or adjusted times as event-time candidates, falling back to its receive
/// time when necessary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeSource {
    /// A timestamp supplied by a device whose clock is synchronized by NTP.
    ///
    /// The receiver considers the associated `device_time_ms` as the event time,
    /// subject to its timestamp-validity checks.
    DeviceNtp,
    /// A timestamp supplied by a device's real-time clock.
    ///
    /// The receiver considers the associated `device_time_ms` as the event time,
    /// subject to its timestamp-validity checks.
    DeviceRtc,
    /// Timing is gateway-owned rather than taken from `device_time_ms`.
    ///
    /// With no device timestamp, a valid `age_ms` makes event time receive time
    /// minus `age_ms` and changes the effective source to
    /// [`TimeSource::GatewayAdjusted`]. If `age_ms` is absent, cannot convert to
    /// `i64`, or cannot be subtracted without overflow, the receiver ignores it
    /// and uses receive time. A `device_time_ms` tagged `Gateway` is not used as
    /// event time.
    Gateway,
    /// The effective source for an observation time reconstructed from relative age.
    ///
    /// The collector records this source after converting `age_ms` to `i64` and
    /// successfully subtracting it from receive time. The reconstructed timestamp
    /// is an event-time candidate. If a `GatewayAdjusted` input has no usable
    /// reconstructed or supplied timestamp, event time falls back to receive time.
    GatewayAdjusted,
}

#[cfg(test)]
mod tests {
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
                time_source: TimeSource::Gateway,
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
        // オプショナル欄は省略される(ワイヤの軽さ)
        assert!(!json.contains("device_time_ms"));
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
}
