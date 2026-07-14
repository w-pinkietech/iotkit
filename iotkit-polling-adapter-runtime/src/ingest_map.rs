//! measurement写像(polling系)。
//!
//! | SensorType(ドライバ)   | ドライバ出力 | 変換 | measurement_key(D6) | channel_index |
//! |------------------------|-------------|------|----------------------|---------------|
//! | Temperature (mcp9600)  | ℃          | ×1   | temperature_c        | None          |
//! | Illuminance (opt3001)  | lux         | ×1   | illuminance_lux      | None          |
//! | その他(将来ドライバ)  | 対応表未宣言の型は送出しない(warnログ)——表の更新を強制する |

use iotkit_core_types::{AdapterId, DeviceKey, SensorReading, SensorType};
use iotkit_ingest_contract::{ReadingItem, TimeSource};

fn measurement_key_for(sensor_type: &SensorType) -> Option<&'static str> {
    Some(match sensor_type {
        SensorType::Temperature => "temperature_c",
        SensorType::Illuminance => "illuminance_lux",
        _ => return None,
    })
}

/// DeviceKey "i2c:0x{addr:02x}:{suffix}" → 位置識別型hardware_id(送信者スコープ付き=D5決定2)。
pub(crate) fn to_items(
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    reading: &SensorReading,
) -> Option<Vec<ReadingItem>> {
    let key = measurement_key_for(&reading.sensor_type)?;
    if reading.values.is_empty() {
        return None;
    }
    let parts: Vec<&str> = device_key.as_str().split(':').collect();
    let hw = match parts.as_slice() {
        ["i2c", addr, _suffix] => format!("{}:i2c:{addr}", adapter_id.as_str()),
        _ => return None,
    };
    Some(vec![ReadingItem {
        subject_hint: Some(hw),
        measurement_key: key.to_string(),
        channel_index: None,
        series_variant: None,
        values: reading.values.clone(),
        device_time_ms: None,
        time_source: TimeSource::Edge,
        age_ms: None,
        rssi: None,
        battery_pct: None,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2c_key_maps_to_sender_scoped_positional_hardware_id() {
        let items = to_items(
            &AdapterId::new("rpi-local:default"),
            &DeviceKey::new("i2c:0x44:illuminance"),
            &SensorReading::new(SensorType::Illuminance, vec![512.0], vec!["lux".into()]),
        )
        .unwrap();
        assert_eq!(
            items[0].subject_hint.as_deref(),
            Some("rpi-local:default:i2c:0x44")
        );
        assert_eq!(items[0].measurement_key, "illuminance_lux");
        assert_eq!(items[0].channel_index, None);
    }

    #[test]
    fn undeclared_sensor_type_and_foreign_key_form_are_not_emitted() {
        assert!(
            to_items(
                &AdapterId::new("rpi-local:default"),
                &DeviceKey::new("i2c:0x29:ranging"),
                &SensorReading::new(SensorType::Ranging, vec![100.0], vec![]),
            )
            .is_none(),
            "対応表未宣言の型は送出しない(単位対応表の更新を強制)"
        );
        assert!(
            to_items(
                &AdapterId::new("rpi-local:default"),
                &DeviceKey::new("bravepi-mainboard:aa:temperature"),
                &SensorReading::new(SensorType::Temperature, vec![21.5], vec![]),
            )
            .is_none(),
            "非polling形式キーはこの写像の担当外"
        );
    }

    #[test]
    fn empty_values_are_not_emitted() {
        assert!(
            to_items(
                &AdapterId::new("rpi-local:default"),
                &DeviceKey::new("i2c:0x44:illuminance"),
                &SensorReading::new(SensorType::Illuminance, vec![], vec!["lux".into()]),
            )
            .is_none()
        );
    }
}
