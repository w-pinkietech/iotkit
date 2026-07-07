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
    Accepted {
        items: Vec<ItemStatus>,
    },
    Duplicate,
    /// エンベロープ単位の終端拒否(送信側はspoolから除去=D1)
    Rejected {
        reason_code: ReasonCode,
        message: String,
    },
    /// 一時的過負荷専用。同一エンベロープを不変のまま再試行(D1)
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ItemStatus {
    Stored {
        disposition: Disposition,
        /// disposition=quarantined のとき理由を可視化(D1追補。省略時はワイヤに現れない)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quarantine_reason: Option<QuarantineReason>,
    },
    ItemRejected {
        reason_code: ReasonCode,
        message: String,
    },
}

/// D1追補(2026-07-03): 検疫理由の可視化。D6判別表と1:1(実装はレジストリ実装=本計画と同時)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuarantineReason {
    OutOfRange,
    UnknownKey,
    UndeclaredChannel,
    DeviceQuarantined,
}

impl QuarantineReason {
    /// ワイヤserde表現とDB(series.quarantine_reason列)で同じ正準文字列を使う
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OutOfRange => "out_of_range",
            Self::UnknownKey => "unknown_key",
            Self::UndeclaredChannel => "undeclared_channel",
            Self::DeviceQuarantined => "device_quarantined",
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
