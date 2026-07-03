use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeAck {
    pub envelope_id: String,
    pub status: AckStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AckStatus {
    /// エンベロープ全体が耐久化された。items は入力itemsと同数・同順(部分受理の内訳)
    Accepted { items: Vec<ItemStatus> },
    Duplicate,
    /// エンベロープ単位の終端拒否(送信側はspoolから除去=D1)
    Rejected { reason_code: ReasonCode, message: String },
    /// 一時的過負荷専用。同一エンベロープを不変のまま再試行(D1)
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ItemStatus {
    Stored { disposition: Disposition },
    ItemRejected { reason_code: ReasonCode, message: String },
}

/// D1監査追記(durable|staged)+D6決定6(quarantined)の3値
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Durable,
    Staged,
    Quarantined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
