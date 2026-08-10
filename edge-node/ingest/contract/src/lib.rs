//! 取り込み契約 v1。ワイヤ契約が規範、このクレートは正本のRust表現。
//! 正本文書: `docs/product/ja/contracts/ingest-v1.md` と
//! `docs/product/en/contracts/ingest-v1.md`。
//!
//! This is a wire-first contract: the JSON exchanged on the wire is normative,
//! and this crate is its reference Rust expression. Version 1 has a stability
//! intent; the acknowledgement matrix and sender retry obligations are defined
//! by the paired product contract.
#![deny(missing_docs)]

/// Acknowledgement types returned after ingest processing.
pub mod ack;
/// Wire types submitted as an ingest envelope.
pub mod envelope;
/// Measurement-key validation and external envelope-ID helpers.
pub mod measurement_key;
/// Side-effect-free validation report types.
pub mod validation;

pub use ack::{AckStatus, Disposition, EnvelopeAck, ItemStatus, QuarantineReason, ReasonCode};
pub use envelope::{Envelope, ReadingItem, TimeSource};
pub use measurement_key::{
    MAX_MEASUREMENT_KEY_LEN, MeasurementKeyError, external_envelope_id, validate_measurement_key,
};
pub use validation::{ValidationIssue, ValidationReport};
