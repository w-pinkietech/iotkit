use super::*;
use iotkit_core_types::{DeviceKey, SensorReading, SensorType};

#[test]
fn bravepi_key_maps_to_ble_hardware_id_and_d6_key() {
    let items = to_items(
        &DeviceKey::new("bravepi-mainboard:00000000000000ab:temperature"),
        &SensorReading::new(SensorType::Temperature, vec![21.5], vec!["celsius".into()]),
        Some(-60),
        Some(90),
    )
    .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].subject_hint.as_deref(),
        Some("ble:00000000000000ab")
    );
    assert_eq!(items[0].measurement_key, "temperature_c");
    assert_eq!(items[0].channel_index, None);
    assert_eq!(items[0].values, vec![21.5]);
    assert_eq!(items[0].rssi, Some(-60));
}

#[test]
fn acceleration_mg_splits_into_three_fixed_channels_without_conversion() {
    // ドライバ出力は既にmG(Task 2)——写像は×1(単位対応表どおり)
    let items = to_items(
        &DeviceKey::new("bravepi-mainboard:00000000000000cc:acceleration"),
        &SensorReading::new(
            SensorType::Acceleration,
            vec![12.0, -34.0, 998.0],
            vec!["x_mg".into(), "y_mg".into(), "z_mg".into()],
        ),
        None,
        None,
    )
    .unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].channel_index, Some(0));
    assert_eq!(items[0].values, vec![12.0]);
    assert_eq!(items[2].channel_index, Some(2));
    assert_eq!(items[2].values, vec![998.0]);
}

#[test]
fn adc_two_channels_split() {
    let items = to_items(
        &DeviceKey::new("bravepi-mainboard:00000000000000dd:adc"),
        &SensorReading::new(
            SensorType::Adc,
            vec![1650.0, 3300.0],
            vec!["ch1_mv".into(), "ch2_mv".into()],
        ),
        None,
        None,
    )
    .unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1].channel_index, Some(1));
    assert_eq!(items[1].measurement_key, "voltage_mv");
}

#[test]
fn contact_samples_split_into_items_without_channel() {
    // 接点の多値は時系列サンプル(contact.rs)——サンプル番号をチャネル化しない
    let items = to_items(
        &DeviceKey::new("bravepi-mainboard:00000000000000ee:contact_input"),
        &SensorReading::new(SensorType::ContactInput, vec![1.0, 0.0, 1.0], vec![]),
        None,
        None,
    )
    .unwrap();
    assert_eq!(items.len(), 3, "サンプルごとにitem分割");
    assert!(
        items.iter().all(|i| i.channel_index.is_none()),
        "channel_index=None(サンプル番号のチャネル捏造禁止)"
    );
    assert_eq!(items[0].values, vec![1.0]);
    assert_eq!(items[1].values, vec![0.0]);
    assert!(items.iter().all(|i| i.measurement_key == "contact_state"));
}

#[test]
fn contact_samples_over_256_remain_scalar_items_without_sample_loss() {
    let values: Vec<f64> = (0..300).map(|i| (i % 2) as f64).collect();
    let expected_values = values.clone();
    let items = to_items(
        &DeviceKey::new("bravepi-mainboard:00000000000000ee:contact_input"),
        &SensorReading::new(SensorType::ContactInput, values, vec![]),
        None,
        None,
    )
    .unwrap();
    assert_eq!(items.len(), expected_values.len());
    assert!(items.iter().all(|i| i.channel_index.is_none()));
    assert!(
        items.iter().all(|i| i.values.len() == 1),
        "contact_state is scalar; every sample must remain a single-value item"
    );
    let actual_values: Vec<f64> = items.iter().map(|i| i.values[0]).collect();
    assert_eq!(actual_values, expected_values);
}

#[test]
fn unknown_sensor_type_and_foreign_key_form_return_none() {
    assert!(
        to_items(
            &DeviceKey::new("bravepi-mainboard:aa:x"),
            &SensorReading::new(SensorType::Unknown("mystery".into()), vec![1.0], vec![]),
            None,
            None,
        )
        .is_none()
    );
    assert!(
        to_items(
            &DeviceKey::new("i2c:0x44:illuminance"),
            &SensorReading::new(SensorType::Illuminance, vec![512.0], vec!["lux".into()]),
            None,
            None,
        )
        .is_none(),
        "非BravePI形式キーはこの写像の担当外"
    );
}

#[test]
fn empty_values_are_not_emitted() {
    assert!(
        to_items(
            &DeviceKey::new("bravepi-mainboard:00000000000000ab:temperature"),
            &SensorReading::new(SensorType::Temperature, vec![], vec!["celsius".into()]),
            None,
            None,
        )
        .is_none()
    );
}
