use serde::{Deserialize, Serialize};

use crate::ReasonCode;

/// Side-effect-free validation output.
///
/// This type is deliberately separate from [`crate::EnvelopeAck`]. Receiving it
/// never means that the Edge accepted custody and never authorizes a sender to
/// delete a spooled envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    /// The sender-provided envelope identifier used only to correlate the report.
    pub envelope_id: String,
    /// Whether deterministic validation found no issues.
    pub valid: bool,
    /// Deterministic envelope- or item-level issues. This is diagnostic output,
    /// not an acknowledgement status.
    pub issues: Vec<ValidationIssue>,
}

/// One deterministic issue found by side-effect-free validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// The zero-based input item position, or `None` for an envelope-wide issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_index: Option<usize>,
    /// Stable machine-readable violation category.
    pub reason_code: ReasonCode,
    /// Human-readable diagnostic text.
    pub message: String,
    /// Optional JSON Pointer locating the invalid field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_path: Option<String>,
    /// Optional stable hint describing the expected schema or value shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_hint: Option<String>,
}
