use iotkit_core_types::{AdapterId, DeviceKey};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

/// Characters that must be percent-encoded in MQTT topic segments.
/// MQTT forbids `+`, `#`, `/` in topic level names; we also encode `:`
/// so adapter IDs like "rpi-local:default" become "rpi-local%3Adefault".
const TOPIC_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b'+')
    .add(b'#')
    .add(b'/')
    .add(b':')
    .add(b'%'); // encode % itself for reversibility

/// Percent-encode a string for use in an MQTT topic segment.
pub fn encode_topic_segment(s: &str) -> String {
    utf8_percent_encode(s, TOPIC_ENCODE_SET).to_string()
}

/// Event types for topic routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    Telemetry,
    Discovery,
    Loss,
    Error,
    Status,
}

impl EventType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Telemetry => "telemetry",
            Self::Discovery => "discovery",
            Self::Loss => "loss",
            Self::Error => "error",
            Self::Status => "status",
        }
    }
}

/// Build the MQTT topic for a given adapter and event type.
pub fn topic(adapter_id: &AdapterId, event_type: EventType) -> String {
    let encoded = encode_topic_segment(adapter_id.as_str());
    format!("iotkit/v1/{encoded}/{}", event_type.as_str())
}

/// Build the MQTT topic for a device inventory retained message.
pub fn inventory_topic(adapter_id: &AdapterId, device_key: &DeviceKey) -> String {
    let encoded_adapter = encode_topic_segment(adapter_id.as_str());
    let encoded_device = encode_topic_segment(device_key.as_str());
    format!("iotkit/v1/{encoded_adapter}/inventory/{encoded_device}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_telemetry() {
        let id = AdapterId::new("rpi-local:default");
        assert_eq!(
            topic(&id, EventType::Telemetry),
            "iotkit/v1/rpi-local%3Adefault/telemetry"
        );
    }

    #[test]
    fn topic_status() {
        let id = AdapterId::new("rpi-local:default");
        assert_eq!(
            topic(&id, EventType::Status),
            "iotkit/v1/rpi-local%3Adefault/status"
        );
    }

    #[test]
    fn topic_encodes_slash() {
        let id = AdapterId::new("bravepi:/dev/ttyAMA0");
        let t = topic(&id, EventType::Telemetry);
        assert!(!t.contains("//"), "slash in adapter_id must be encoded");
        assert!(t.contains("%2F"));
    }

    #[test]
    fn topic_encodes_percent() {
        let id = AdapterId::new("test%id");
        let t = topic(&id, EventType::Telemetry);
        assert!(t.contains("%25"), "percent sign must be double-encoded");
    }

    #[test]
    fn inventory_topic_format() {
        let aid = AdapterId::new("rpi-local:default");
        let dk = DeviceKey::new("i2c:0x60:mcp9600");
        assert_eq!(
            inventory_topic(&aid, &dk),
            "iotkit/v1/rpi-local%3Adefault/inventory/i2c%3A0x60%3Amcp9600"
        );
    }

    #[test]
    fn encode_topic_segment_roundtrip() {
        let original = "rpi-local:default";
        let encoded = encode_topic_segment(original);
        let decoded = percent_encoding::percent_decode_str(&encoded)
            .decode_utf8()
            .unwrap();
        assert_eq!(decoded, original);
    }
}
