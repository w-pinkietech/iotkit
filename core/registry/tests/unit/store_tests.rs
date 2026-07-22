use super::*;
use crate::catalog::standard_catalog;

fn test_db() -> iotkit_core_storage::DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(crate::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    iotkit_core_storage::init_db_memory(&all).unwrap()
}

#[test]
fn enable_entry_copies_catalog_and_stamps_revision_and_audits() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let cat = standard_catalog();
        let t = cat.find("temperature_c").unwrap();
        let row = enable_entry(conn, t, &cat.catalog_version, "auto").unwrap();
        assert_eq!(row.measurement_key, "temperature_c");
        assert_eq!(row.origin, "catalog");
        assert_eq!(row.catalog_version.as_deref(), Some("1.0.0"));
        assert_eq!(row.entry_revision, t.revision());
        assert_eq!(row.physical_min, Some(-200.0));
        assert_eq!(row.physical_max, Some(1372.0));
        assert_eq!(row.local_min, None, "現場既定はWave 0では未設定");
        // 監査イベント必須(D6決定4)
        let detail: String = conn
            .query_row(
                "SELECT detail FROM ledger_events WHERE kind='registry_entry_enabled'
                 ORDER BY event_id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(detail.contains("temperature_c") && detail.contains(&t.revision()));
        Ok(())
    })
    .unwrap();
}

#[test]
fn enable_entry_is_idempotent() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let cat = standard_catalog();
        let t = cat.find("temperature_c").unwrap();
        enable_entry(conn, t, &cat.catalog_version, "auto").unwrap();
        enable_entry(conn, t, &cat.catalog_version, "auto").unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM registry_entries WHERE measurement_key='temperature_c'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(events, 1, "冪等re-enableは監査イベントを重複させない");
        Ok(())
    })
    .unwrap();
}

fn custom_spec(key: &str) -> CustomEntrySpec {
    CustomEntrySpec {
        measurement_key: key.to_string(),
        unit_ucum: Some("Cel".to_string()),
        unit_display: Some("C".to_string()),
        value_type: ValueType::Float,
        semantic_class: "sensor".to_string(),
        channel_mode: ChannelMode::Single,
        channel_roles: Vec::new(),
        physical_min: Some(-50.0),
        physical_max: Some(150.0),
    }
}

#[test]
fn define_custom_entry_requires_custom_prefix() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        assert!(matches!(
            define_custom_entry(conn, &custom_spec("tank_temp")),
            Err(RegistryError::InvalidKey(_))
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn define_custom_entry_rejects_existing_entry_and_alias_collisions() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let spec = custom_spec("custom.tank_temp");
        define_custom_entry(conn, &spec).unwrap();
        assert!(matches!(
            define_custom_entry(conn, &spec),
            Err(RegistryError::NamespaceCollision(_))
        ));

        let cat = standard_catalog();
        enable_entry(
            conn,
            cat.find("temperature_c").unwrap(),
            &cat.catalog_version,
            "auto",
        )
        .unwrap();
        define_alias(
            conn,
            "custom.alias_temp",
            "temperature_c",
            AliasKind::LocationMapping,
        )
        .unwrap();
        assert!(matches!(
            define_custom_entry(conn, &custom_spec("custom.alias_temp")),
            Err(RegistryError::NamespaceCollision(_))
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn define_custom_entry_inserts_custom_origin_revision_and_audit() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let spec = custom_spec("custom.tank_temp");
        let row = define_custom_entry(conn, &spec).unwrap();
        assert_eq!(row.measurement_key, "custom.tank_temp");
        assert_eq!(row.origin, "custom");
        assert_eq!(row.catalog_version, None);
        assert!(!row.entry_revision.is_empty());
        assert_eq!(row.unit_ucum.as_deref(), Some("Cel"));
        assert_eq!(row.channel_mode, ChannelMode::Single);
        assert_eq!(row.physical_min, Some(-50.0));
        assert_eq!(row.physical_max, Some(150.0));

        let detail: String = conn
            .query_row(
                "SELECT detail FROM ledger_events WHERE kind='registry_entry_enabled'
                 ORDER BY event_id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let detail: serde_json::Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(detail["key"], "custom.tank_temp");
        assert_eq!(detail["origin"], "custom");
        assert_eq!(detail["catalog_version"], serde_json::Value::Null);
        assert_eq!(detail["revision"], row.entry_revision);
        Ok(())
    })
    .unwrap();
}

#[test]
fn define_custom_entry_fixed_mode_requires_roles() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let mut spec = custom_spec("custom.vector");
        spec.channel_mode = ChannelMode::Fixed;
        assert!(matches!(
            define_custom_entry(conn, &spec),
            Err(RegistryError::InvalidKey(_))
        ));
        spec.channel_roles = vec!["x".to_string(), "y".to_string(), "z".to_string()];
        let row = define_custom_entry(conn, &spec).unwrap();
        assert_eq!(row.channel_roles, vec!["x", "y", "z"]);
        Ok(())
    })
    .unwrap();
}

#[test]
fn fixed_channel_roles_round_trip_through_db() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let cat = standard_catalog();
        let acc = cat.find("acceleration_mg").unwrap();
        enable_entry(conn, acc, &cat.catalog_version, "auto").unwrap();
        let row = get_entry(conn, "acceleration_mg").unwrap().unwrap();
        assert_eq!(row.channel_mode, ChannelMode::Fixed);
        assert_eq!(row.channel_roles, vec!["x", "y", "z"]);
        Ok(())
    })
    .unwrap();
}

#[test]
fn define_alias_and_resolve() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let cat = standard_catalog();
        enable_entry(
            conn,
            cat.find("temperature_c").unwrap(),
            &cat.catalog_version,
            "auto",
        )
        .unwrap();
        define_alias(
            conn,
            "temp_old",
            "temperature_c",
            AliasKind::LocationMapping,
        )
        .unwrap();
        match find_resolution(conn, "temp_old").unwrap().unwrap() {
            Resolution::Alias {
                canonical,
                alias_kind,
            } => {
                assert_eq!(canonical.measurement_key, "temperature_c");
                assert_eq!(alias_kind, "location_mapping");
            }
            other => panic!("expected Alias resolution, got {other:?}"),
        }
        match find_resolution(conn, "temperature_c").unwrap().unwrap() {
            Resolution::Entry(e) => assert_eq!(e.measurement_key, "temperature_c"),
            other => panic!("expected Entry resolution, got {other:?}"),
        }
        assert!(find_resolution(conn, "custom.nothing").unwrap().is_none());
        Ok(())
    })
    .unwrap();
}

#[test]
fn single_namespace_collisions_are_blocked_both_ways() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let cat = standard_catalog();
        enable_entry(
            conn,
            cat.find("temperature_c").unwrap(),
            &cat.catalog_version,
            "auto",
        )
        .unwrap();
        // alias名がエントリキーと衝突 → 拒否
        assert!(matches!(
            define_alias(
                conn,
                "temperature_c",
                "temperature_c",
                AliasKind::LocationMapping
            ),
            Err(RegistryError::NamespaceCollision(_))
        ));
        // 逆方向: 既存aliasと同名のエントリ有効化 → 拒否(D6決定3の衝突検査)
        define_alias(
            conn,
            "voltage_mv",
            "temperature_c",
            AliasKind::LocationMapping,
        )
        .unwrap();
        assert!(matches!(
            enable_entry(
                conn,
                cat.find("voltage_mv").unwrap(),
                &cat.catalog_version,
                "auto"
            ),
            Err(RegistryError::NamespaceCollision(_))
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn define_alias_releases_unknown_key_quarantined_series_and_audits() {
    // D6決定3(a): 実体化済み申告キーへのエイリアス確立=canonical定義バインド → 検疫解除
    let db = test_db();
    db.with_conn_sync(|conn| {
        let cat = standard_catalog();
        enable_entry(
            conn,
            cat.find("temperature_c").unwrap(),
            &cat.catalog_version,
            "auto",
        )
        .unwrap();
        let sid = iotkit_core_ledger::insert_device(
            conn,
            &iotkit_core_ledger::NewDevice {
                hardware_id: "ble:aa".into(),
                user_label: None,
                parent: None,
                kind: iotkit_core_ledger::DeviceKind::Individual,
                initial_state: iotkit_core_ledger::DeviceState::Active,
            },
        )
        .unwrap();
        // 検疫期にunknown_keyとして実体化済みのseries
        iotkit_core_ledger::ensure_series(
            conn,
            &sid,
            "temp_old",
            iotkit_core_ledger::CHANNEL_NA,
            iotkit_core_ledger::DEFAULT_VARIANT,
            true,
            Some("unknown_key"),
        )
        .unwrap();
        define_alias(
            conn,
            "temp_old",
            "temperature_c",
            AliasKind::LocationMapping,
        )
        .unwrap();
        let meta = iotkit_core_ledger::find_series_meta(
            conn,
            &sid,
            "temp_old",
            iotkit_core_ledger::CHANNEL_NA,
            iotkit_core_ledger::DEFAULT_VARIANT,
        )
        .unwrap()
        .unwrap();
        assert!(!meta.quarantined, "series_keyは不変のまま検疫解除される");
        assert_eq!(meta.quarantine_reason, None);
        let detail: String = conn
            .query_row(
                "SELECT detail FROM ledger_events WHERE kind='series_quarantine_released'
                 ORDER BY event_id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(detail.contains("temp_old") && detail.contains("temperature_c"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn define_alias_keeps_channel_mismatched_unknown_key_series_quarantined() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let cat = standard_catalog();
        enable_entry(
            conn,
            cat.find("temperature_c").unwrap(),
            &cat.catalog_version,
            "auto",
        )
        .unwrap();
        let sid = iotkit_core_ledger::insert_device(
            conn,
            &iotkit_core_ledger::NewDevice {
                hardware_id: "ble:mix".into(),
                user_label: None,
                parent: None,
                kind: iotkit_core_ledger::DeviceKind::Individual,
                initial_state: iotkit_core_ledger::DeviceState::Active,
            },
        )
        .unwrap();
        let good = iotkit_core_ledger::ensure_series(
            conn,
            &sid,
            "temp_old",
            iotkit_core_ledger::CHANNEL_NA,
            iotkit_core_ledger::DEFAULT_VARIANT,
            true,
            Some("unknown_key"),
        )
        .unwrap();
        let bad = iotkit_core_ledger::ensure_series(
            conn,
            &sid,
            "temp_old",
            1,
            iotkit_core_ledger::DEFAULT_VARIANT,
            true,
            Some("unknown_key"),
        )
        .unwrap();

        define_alias(
            conn,
            "temp_old",
            "temperature_c",
            AliasKind::LocationMapping,
        )
        .unwrap();

        let released = iotkit_core_ledger::find_series_meta(
            conn,
            &sid,
            "temp_old",
            iotkit_core_ledger::CHANNEL_NA,
            iotkit_core_ledger::DEFAULT_VARIANT,
        )
        .unwrap()
        .unwrap();
        assert!(!released.quarantined);
        let mismatched = iotkit_core_ledger::find_series_meta(
            conn,
            &sid,
            "temp_old",
            1,
            iotkit_core_ledger::DEFAULT_VARIANT,
        )
        .unwrap()
        .unwrap();
        assert!(mismatched.quarantined);
        assert_eq!(
            mismatched.quarantine_reason.as_deref(),
            Some("undeclared_channel")
        );
        let detail: String = conn
            .query_row(
                "SELECT detail FROM ledger_events WHERE kind='series_quarantine_released'
                 ORDER BY event_id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            detail.contains(&format!("\"series_ids\":[{good}]")),
            "{detail}"
        );
        assert!(
            detail.contains(&format!("\"channel_mismatch_ids\":[{bad}]")),
            "{detail}"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn define_alias_releases_single_zero_channel_unknown_key_series() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let cat = standard_catalog();
        enable_entry(
            conn,
            cat.find("temperature_c").unwrap(),
            &cat.catalog_version,
            "auto",
        )
        .unwrap();
        let sid = iotkit_core_ledger::insert_device(
            conn,
            &iotkit_core_ledger::NewDevice {
                hardware_id: "ble:zero".into(),
                user_label: None,
                parent: None,
                kind: iotkit_core_ledger::DeviceKind::Individual,
                initial_state: iotkit_core_ledger::DeviceState::Active,
            },
        )
        .unwrap();
        let zero = iotkit_core_ledger::ensure_series(
            conn,
            &sid,
            "temp_old",
            0,
            iotkit_core_ledger::DEFAULT_VARIANT,
            true,
            Some("unknown_key"),
        )
        .unwrap();
        let bad = iotkit_core_ledger::ensure_series(
            conn,
            &sid,
            "temp_old",
            5,
            iotkit_core_ledger::DEFAULT_VARIANT,
            true,
            Some("unknown_key"),
        )
        .unwrap();

        define_alias(
            conn,
            "temp_old",
            "temperature_c",
            AliasKind::LocationMapping,
        )
        .unwrap();

        let released = iotkit_core_ledger::find_series_meta(
            conn,
            &sid,
            "temp_old",
            0,
            iotkit_core_ledger::DEFAULT_VARIANT,
        )
        .unwrap()
        .unwrap();
        assert!(!released.quarantined, "single channel=0 は検疫解除対象");
        assert_eq!(released.quarantine_reason, None);
        let mismatched = iotkit_core_ledger::find_series_meta(
            conn,
            &sid,
            "temp_old",
            5,
            iotkit_core_ledger::DEFAULT_VARIANT,
        )
        .unwrap()
        .unwrap();
        assert!(mismatched.quarantined);
        assert_eq!(
            mismatched.quarantine_reason.as_deref(),
            Some("undeclared_channel")
        );
        let detail: String = conn
            .query_row(
                "SELECT detail FROM ledger_events WHERE kind='series_quarantine_released'
                 ORDER BY event_id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            detail.contains(&format!("\"series_ids\":[{zero}]")),
            "{detail}"
        );
        assert!(
            detail.contains(&format!("\"channel_mismatch_ids\":[{bad}]")),
            "{detail}"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn define_alias_rejects_missing_target_dup_and_bad_grammar() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let cat = standard_catalog();
        assert!(matches!(
            define_alias(conn, "temp_old", "temperature_c", AliasKind::Rename),
            Err(RegistryError::TargetNotFound(_))
        ));
        enable_entry(
            conn,
            cat.find("temperature_c").unwrap(),
            &cat.catalog_version,
            "auto",
        )
        .unwrap();
        define_alias(conn, "temp_old", "temperature_c", AliasKind::Rename).unwrap();
        assert!(matches!(
            define_alias(conn, "temp_old", "temperature_c", AliasKind::Rename),
            Err(RegistryError::AliasExists(_))
        ));
        assert!(matches!(
            define_alias(conn, "Bad:Alias", "temperature_c", AliasKind::Rename),
            Err(RegistryError::InvalidKey(_))
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn legacy_map_seeds_and_resolves() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let n = seed_legacy_sensor_map(conn).unwrap();
        assert_eq!(n, LEGACY_SENSOR_MAP.len());
        assert_eq!(
            lookup_legacy(conn, 261).unwrap().as_deref(),
            Some("temperature_c")
        );
        assert_eq!(
            lookup_legacy(conn, 294).unwrap().as_deref(),
            Some("contact_state")
        );
        assert_eq!(lookup_legacy(conn, 9999).unwrap(), None);
        // 冪等
        assert_eq!(seed_legacy_sensor_map(conn).unwrap(), 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn list_entries_and_aliases_return_inserted_rows() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let cat = standard_catalog();
        enable_entry(
            conn,
            cat.find("temperature_c").unwrap(),
            &cat.catalog_version,
            "test",
        )
        .unwrap();
        define_alias(conn, "temp_old", "temperature_c", AliasKind::Rename).unwrap();

        let entries = list_entries(conn).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].measurement_key, "temperature_c");
        assert_eq!(entries[0].unit_ucum.as_deref(), Some("Cel"));

        let aliases = list_aliases(conn).unwrap();
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].alias, "temp_old");
        assert_eq!(aliases[0].measurement_key, "temperature_c");
        assert_eq!(aliases[0].alias_kind, "rename");
        Ok(())
    })
    .unwrap();
}
