use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub envelope_id: String,
    /// 送信者の自己記述(D1: adapter_idの出所はチャネルキーでなくエンベロープ自身)
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration_version: Option<u32>,
    pub items: Vec<ReadingItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingItem {
    /// = hardware_id。多subject送信者(親子束ね)は必須(D5決定1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_hint: Option<String>,
    pub measurement_key: String,
    /// None = 'na'(DB内では番兵値-1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_index: Option<u16>,
    /// None = "primary"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_variant: Option<String>,
    pub values: Vec<f64>,
    /// デバイス申告時刻(unix ms)。オプショナル(D1: 時刻がないからrejectedは禁止)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_time_ms: Option<i64>,
    pub time_source: TimeSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rssi: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_pct: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeSource {
    DeviceNtp,
    DeviceRtc,
    Gateway,
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
