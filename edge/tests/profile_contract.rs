use std::path::PathBuf;

use iotkit_edge::{
    application::profiles::{DeviceProfileInput, InventoryProfiles, SignalProfileInput},
    storage::{AuditActor, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;

async fn sqlite_store() -> (tempfile::TempDir, Storage) {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("edge.db"),
    })
    .await
    .unwrap();
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage
        .apply_descriptor(&descriptor, 1_720_000_000_000)
        .await
        .unwrap();
    (directory, storage)
}

#[tokio::test]
async fn sqlite_profiles_keep_stable_refs_revisions_audit_and_reload() {
    let (directory, storage) = sqlite_store().await;
    let inventory = InventoryProfiles::new(storage.clone());
    let devices = inventory.devices().await.unwrap();
    let signals = inventory.signals().await.unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(signals.len(), 1);
    assert!(devices[0].device_ref.starts_with("dev_"));
    assert!(signals[0].signal_ref.starts_with("sig_"));

    let device = inventory
        .update_device(
            AuditActor::local_cli(),
            &devices[0].device_ref,
            DeviceProfileInput {
                display_name: "  乾燥炉入口  ".into(),
                location: " 第1工場 乾燥工程 ".into(),
            },
            None,
            1_720_000_000_100,
        )
        .await
        .unwrap();
    assert_eq!(device.display_name, "乾燥炉入口");
    assert_eq!(device.revision, 1);

    let signal = inventory
        .update_signal(
            AuditActor::local_cli(),
            &signals[0].signal_ref,
            SignalProfileInput {
                display_name: "乾燥炉入口 温度".into(),
                display_sensor_type: "temperature".into(),
                display_sensor_type_label: String::new(),
                display_value_kind: "numeric".into(),
                display_unit_mode: "unit".into(),
                display_unit: "°C".into(),
                decimal_places: 2,
            },
            None,
            1_720_000_000_200,
        )
        .await
        .unwrap();
    assert_eq!(signal.revision, 1);

    assert!(
        inventory
            .update_signal(
                AuditActor::local_cli(),
                &signals[0].signal_ref,
                SignalProfileInput {
                    display_name: "stale".into(),
                    ..SignalProfileInput::numeric("temperature", "°C", 1)
                },
                Some(9),
                1_720_000_000_300,
            )
            .await
            .is_err()
    );
    let original_signal_ref = signals[0].signal_ref.clone();
    drop(inventory);
    drop(storage);

    let reopened = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("edge.db"),
    })
    .await
    .unwrap();
    let inventory = InventoryProfiles::new(reopened.clone());
    let device = &inventory.devices().await.unwrap()[0];
    assert_eq!(device.display_name, "乾燥炉入口");
    assert_eq!(device.location, "第1工場 乾燥工程");
    let signal = &inventory.signals().await.unwrap()[0];
    assert_eq!(signal.signal_ref, original_signal_ref);
    assert_eq!(signal.display_name, "乾燥炉入口 温度");
    assert_eq!(signal.display_unit, "°C");
    assert_eq!(signal.decimal_places, 2);
    let audit = reopened.list_audit_events(10).await.unwrap();
    assert!(
        audit
            .iter()
            .any(|event| event.operation == "device_profile.update")
    );
    assert!(
        audit
            .iter()
            .any(|event| event.operation == "signal_profile.update")
    );
}

#[tokio::test]
async fn clean_edge_node_replacement_creates_unconfigured_signal_without_reusing_old_profile() {
    let (_directory, storage) = sqlite_store().await;
    let inventory = InventoryProfiles::new(storage.clone());
    let old_signal = inventory.signals().await.unwrap().remove(0);
    inventory
        .update_signal(
            AuditActor::local_cli(),
            &old_signal.signal_ref,
            SignalProfileInput {
                display_name: "乾燥炉入口 接点".into(),
                display_sensor_type: "contact".into(),
                display_sensor_type_label: String::new(),
                display_value_kind: "boolean".into(),
                display_unit_mode: "dimensionless".into(),
                display_unit: String::new(),
                decimal_places: 0,
            },
            None,
            1_720_000_000_100,
        )
        .await
        .unwrap();

    let mut clean_replacement = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    clean_replacement.edge_node_id = "edge-node-clean-replacement".into();
    clean_replacement.ledger_epoch = "epoch-clean-replacement".into();
    clean_replacement.devices[0].system_id = "018f0000-0000-7000-8000-000000000002".into();
    clean_replacement.signals[0].system_id = "018f0000-0000-7000-8000-000000000002".into();
    clean_replacement.signals[0].series_key =
        "018f0000-0000-7000-8000-000000000002:contact_state:na:primary".into();
    storage
        .apply_descriptor(&clean_replacement, 1_720_000_000_200)
        .await
        .unwrap();

    let signals = inventory.signals().await.unwrap();
    assert_eq!(signals.len(), 2);
    let preserved = signals
        .iter()
        .find(|signal| signal.edge_node_id == "edge-node-01")
        .unwrap();
    let replacement = signals
        .iter()
        .find(|signal| signal.edge_node_id == "edge-node-clean-replacement")
        .unwrap();

    assert_eq!(preserved.signal_ref, old_signal.signal_ref);
    assert_eq!(preserved.display_name, "乾燥炉入口 接点");
    assert_eq!(preserved.profile_revision, Some(1));
    assert_ne!(replacement.signal_ref, old_signal.signal_ref);
    assert!(replacement.display_name.is_empty());
    assert_eq!(replacement.profile_revision, None);
}

#[tokio::test]
async fn boolean_profile_is_dimensionless() {
    let (_directory, storage) = sqlite_store().await;
    let inventory = InventoryProfiles::new(storage);
    let signal_ref = inventory.signals().await.unwrap()[0].signal_ref.clone();
    assert!(
        inventory
            .update_signal(
                AuditActor::local_cli(),
                &signal_ref,
                SignalProfileInput {
                    display_name: "接点".into(),
                    display_sensor_type: "contact".into(),
                    display_sensor_type_label: String::new(),
                    display_value_kind: "boolean".into(),
                    display_unit_mode: "unit".into(),
                    display_unit: "V".into(),
                    decimal_places: 1,
                },
                None,
                2,
            )
            .await
            .is_err()
    );
}
