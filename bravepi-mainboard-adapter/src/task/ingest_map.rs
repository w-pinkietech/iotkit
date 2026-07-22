//! measurement写像(D4: ランタイムの責務)。SensorData(旧語彙)→ ReadingItem(取り込み契約)。
//!
//! | SensorType(ドライバ)          | ドライバ出力                       | 変換 | measurement_key(D6)     | 分割規約                                        |
//! |-------------------------------|-----------------------------------|------|--------------------------|-------------------------------------------------|
//! | ContactInput (contact)        | 0/1 ×data_count(時系列サンプル) | ×1   | contact_state            | サンプルごとに複数item・**channel_index=None**   |
//! | ContactOutput (contact)       | 0/1 ×data_count(同上)           | ×1   | contact_output_state     | 同上                                             |
//! | Adc (mcp3427)                 | mV ch1,ch2(物理2ch)             | ×1   | voltage_mv               | 値ごとに Some(0),Some(1)                         |
//! | Ranging (vl53l1x)             | mm                                | ×1   | distance_mm              | 単一item・None                                   |
//! | Temperature (mcp9600)         | ℃                                | ×1   | temperature_c            | 単一item・None                                   |
//! | Acceleration (lis2duxs12)     | mG x,y,z(Task 2改修後)          | ×1   | acceleration_mg          | 値ごとに Some(0..=2)(固定役割=D6決定12)       |
//! | DifferentialPressure (sdp810) | Pa                                | ×1   | differential_pressure_pa | 単一item・None                                   |
//! | Illuminance (opt3001)         | lux                               | ×1   | illuminance_lux          | 単一item・None                                   |
//! | Unknown(_)                    | -                                 | 送出しない(warnログ)                                        |
//!
//! 分割規約の根拠: 多値の意味はSensorTypeごとに異なる——ADC/加速度は「物理チャネル」(channel_index化)、
//! 接点は「1接点の時系列サンプル」(channel化するとサンプル番号をチャネルとして捏造する。計画レビューBLOCKER)。
//! 汎用のlen>1分割は禁止し、型ごとに宣言する。

use iotkit_core_types::{DeviceKey, SensorReading, SensorType};
use iotkit_ingest_contract::{ReadingItem, TimeSource};

pub(crate) const MAX_ITEMS_PER_ENVELOPE: usize = 256;

/// DeviceKey → hardware_id 正規形(D5決定2)。
/// BravePI: "bravepi-mainboard:{device_number}:{suffix}" → 個体識別型 "ble:{device_number}"
pub(crate) fn hardware_id_for(device_key: &DeviceKey) -> Option<String> {
    let parts: Vec<&str> = device_key.as_str().split(':').collect();
    match parts.as_slice() {
        ["bravepi-mainboard", device_number, _suffix] => Some(format!("ble:{device_number}")),
        _ => None,
    }
}

fn measurement_key_for(sensor_type: &SensorType) -> Option<&'static str> {
    Some(match sensor_type {
        SensorType::ContactInput => "contact_state",
        SensorType::ContactOutput => "contact_output_state",
        SensorType::Adc => "voltage_mv",
        SensorType::Ranging => "distance_mm",
        SensorType::Temperature => "temperature_c",
        SensorType::Acceleration => "acceleration_mg",
        SensorType::DifferentialPressure => "differential_pressure_pa",
        SensorType::Illuminance => "illuminance_lux",
        SensorType::Unknown(_) => return None,
    })
}

fn make_item(
    hw: &str,
    key: &str,
    channel_index: Option<u16>,
    values: Vec<f64>,
    rssi: Option<i16>,
    battery_pct: Option<u8>,
) -> ReadingItem {
    ReadingItem {
        subject_hint: Some(hw.to_string()),
        measurement_key: key.to_string(),
        channel_index,
        series_variant: None,
        values,
        device_time_ms: None,
        time_source: TimeSource::EdgeNode,
        age_ms: None,
        rssi,
        battery_pct,
    }
}

/// SensorData 1件 → ReadingItem列。分割規約は単位対応表(冒頭)のとおり**SensorTypeごとに宣言**——
/// 汎用のlen>1分割は禁止(接点の時系列サンプルをチャネル化する事故の再発防止)。
/// Unknown型・非BravePI形式キーはNone(送出しない。warnは呼び出し側)。
pub(crate) fn to_items(
    device_key: &DeviceKey,
    reading: &SensorReading,
    rssi: Option<i16>,
    battery_pct: Option<u8>,
) -> Option<Vec<ReadingItem>> {
    let key = measurement_key_for(&reading.sensor_type)?;
    let hw = hardware_id_for(device_key)?;
    if reading.values.is_empty() {
        return None;
    }
    let items = match reading.sensor_type {
        // 物理チャネル/固定役割: 値ごとにchannel_index付きで分割(D6決定12)
        SensorType::Acceleration | SensorType::Adc if reading.values.len() > 1 => reading
            .values
            .iter()
            .enumerate()
            .map(|(i, v)| make_item(&hw, key, Some(i as u16), vec![*v], rssi, battery_pct))
            .collect(),
        // 接点: 多値は1接点の時系列サンプル——サンプルごとのitem・channelなし
        SensorType::ContactInput | SensorType::ContactOutput if reading.values.len() > 1 => reading
            .values
            .iter()
            .map(|v| make_item(&hw, key, None, vec![*v], rssi, battery_pct))
            .collect(),
        // 単ch型(および全型の単値): 単一item・channelなし
        _ => vec![make_item(
            &hw,
            key,
            None,
            reading.values.clone(),
            rssi,
            battery_pct,
        )],
    };
    Some(items)
}

#[cfg(test)]
#[path = "../../tests/unit/task/ingest_map_tests.rs"]
mod tests;
