//! 暫定ブリッジ(計画3でアダプタ内取り込みクライアントに置き換え、削除予定)。
//! AdapterEvent(旧語彙)→ 取り込み契約Envelope の翻訳と、
//! SensorType → D6初期語彙measurement_key の写像。
use iotkit_core_types::{AdapterId, DeviceKey, SensorReading, SensorType};
use iotkit_ingest_contract::{Envelope, ReadingItem, TimeSource};

/// D6決定11の初期語彙への写像
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

/// DeviceKey → hardware_id 正規形(D5決定2)
/// - BravePI: "bravepi-mainboard:{device_number}:{suffix}" → 個体識別型 "ble:{device_number}"
/// - I2Cポーリング: "i2c:0x44:{suffix}" → 位置識別型(送信者スコープ付き) "{adapter_id}:i2c:0x44"
fn hardware_id_for(adapter_id: &AdapterId, device_key: &DeviceKey) -> Option<String> {
    let parts: Vec<&str> = device_key.as_str().split(':').collect();
    match parts.as_slice() {
        ["bravepi-mainboard", device_number, _suffix] => Some(format!("ble:{device_number}")),
        ["i2c", addr, _suffix] => Some(format!("{}:i2c:{addr}", adapter_id.as_str())),
        _ => None,
    }
}

pub fn adapter_event_to_envelope(
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    reading: &SensorReading,
    rssi: Option<i16>,
    battery_pct: Option<u8>,
) -> Option<Envelope> {
    let key = measurement_key_for(&reading.sensor_type)?;
    let hw = hardware_id_for(adapter_id, device_key)?;
    let items: Vec<ReadingItem> = if reading.values.len() > 1 {
        reading.values.iter().enumerate().map(|(i, v)| ReadingItem {
            subject_hint: Some(hw.clone()),
            measurement_key: key.to_string(),
            channel_index: Some(i as u16),
            series_variant: None,
            values: vec![*v],
            device_time_ms: None,
            time_source: TimeSource::Gateway,
            age_ms: None, rssi, battery_pct,
        }).collect()
    } else {
        vec![ReadingItem {
            subject_hint: Some(hw),
            measurement_key: key.to_string(),
            channel_index: None,
            series_variant: None,
            values: reading.values.clone(),
            device_time_ms: None,
            time_source: TimeSource::Gateway,
            age_ms: None, rssi, battery_pct,
        }]
    };
    Some(Envelope {
        envelope_id: uuid::Uuid::new_v4().to_string(), // プロセス内はUUIDv4可(D1)
        source: adapter_id.as_str().to_string(),
        declaration_version: None,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::{AdapterId, DeviceKey, SensorReading, SensorType};

    #[test]
    fn bravepi_key_maps_to_ble_hardware_id_and_d6_key() {
        let e = adapter_event_to_envelope(
            &AdapterId::new("bravepi-mainboard:/dev/ttyAMA0"),
            &DeviceKey::new("bravepi-mainboard:00000000000000ab:temperature"),
            &SensorReading::new(SensorType::Temperature, vec![21.5], vec!["temp".into()]),
            Some(-60), Some(90),
        ).unwrap();
        // 実物のBravePI AdapterIdは "bravepi-mainboard:{port_path}"(handle.rs:109)
        assert_eq!(e.source, "bravepi-mainboard:/dev/ttyAMA0");
        assert_eq!(e.items.len(), 1);
        let item = &e.items[0];
        assert_eq!(item.subject_hint.as_deref(), Some("ble:00000000000000ab"));
        assert_eq!(item.measurement_key, "temperature_c");
        assert_eq!(item.channel_index, None);
        assert_eq!(item.values, vec![21.5]);
    }

    #[test]
    fn i2c_key_maps_to_sender_scoped_positional_hardware_id() {
        let e = adapter_event_to_envelope(
            &AdapterId::new("rpi-local:default"),
            &DeviceKey::new("i2c:0x44:illuminance"),
            &SensorReading::new(SensorType::Illuminance, vec![512.0], vec!["lux".into()]),
            None, None,
        ).unwrap();
        // 位置識別型は送信者スコープを含む(D5決定2)
        assert_eq!(e.items[0].subject_hint.as_deref(), Some("rpi-local:default:i2c:0x44"));
        assert_eq!(e.items[0].measurement_key, "illuminance_lux");
    }

    #[test]
    fn multi_value_reading_becomes_per_channel_items() {
        let e = adapter_event_to_envelope(
            &AdapterId::new("bravepi-mainboard:/dev/ttyAMA0"),
            &DeviceKey::new("bravepi-mainboard:00000000000000cc:acceleration"),
            &SensorReading::new(SensorType::Acceleration, vec![1.0, 2.0, 3.0],
                vec!["x".into(), "y".into(), "z".into()]),
            Some(-55), Some(80),
        ).unwrap();
        assert_eq!(e.items.len(), 3);
        assert_eq!(e.items[0].channel_index, Some(0));
        assert_eq!(e.items[2].channel_index, Some(2));
        assert_eq!(e.items[2].values, vec![3.0]);
        assert_eq!(e.items[2].measurement_key, "acceleration_mg");
    }

    #[test]
    fn unknown_sensor_type_returns_none() {
        let r = SensorReading::new(SensorType::Unknown("mystery".into()), vec![1.0], vec![]);
        assert!(adapter_event_to_envelope(
            &AdapterId::new("a"), &DeviceKey::new("a:b:c"), &r, None, None
        ).is_none());
    }

    #[tokio::test]
    async fn bridge_output_flows_through_collector_to_readings() {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
        all.sort_by_key(|m| m.version);
        let db = iotkit_core_storage::init_db_memory(&all).unwrap();
        db.with_conn_sync(|conn| {
            iotkit_core_ledger::insert_device(conn, &iotkit_core_ledger::NewDevice {
                hardware_id: "ble:00000000000000ab".into(),
                user_label: None, parent: None,
                kind: iotkit_core_ledger::DeviceKind::Individual,
                initial_state: iotkit_core_ledger::DeviceState::Active,
            }).unwrap();
            Ok(())
        }).unwrap();
        let (collector, _h) = iotkit_core_collector::Collector::spawn(
            db.clone(), std::sync::Arc::new(iotkit_core_collector::PermissiveRegistry), 16);
        let e = adapter_event_to_envelope(
            &iotkit_core_types::AdapterId::new("bravepi-mainboard:/dev/ttyAMA0"),
            &iotkit_core_types::DeviceKey::new("bravepi-mainboard:00000000000000ab:temperature"),
            &iotkit_core_types::SensorReading::new(
                iotkit_core_types::SensorType::Temperature, vec![21.5], vec!["temp".into()]),
            Some(-60), Some(90),
        ).unwrap();
        let ack = collector.submit(e).await.unwrap();
        assert!(matches!(ack.status, iotkit_ingest_contract::AckStatus::Accepted { .. }));
        let n: i64 = db.with_conn_sync(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap())
        }).unwrap();
        assert_eq!(n, 1);
    }
}
