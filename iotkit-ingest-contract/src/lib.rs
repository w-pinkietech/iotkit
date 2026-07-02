//! 取り込み契約 v1(安定意図)。ワイヤ契約が規範、このクレートは正本のRust表現。
//! 正本文書: docs/redesign/decisions/D1-ingest-model.md, D6-measurement-registry.md
pub mod ack;
pub mod envelope;
pub mod measurement_key;

pub use ack::{AckStatus, Disposition, EnvelopeAck, ItemStatus, ReasonCode};
pub use envelope::{Envelope, ReadingItem, TimeSource};
pub use measurement_key::{
    external_envelope_id, validate_measurement_key, MeasurementKeyError, MAX_MEASUREMENT_KEY_LEN,
};
