use super::*;
use crate::store::{AliasKind, define_alias, enable_entry};
use iotkit_core_ledger::{
    CHANNEL_NA, DEFAULT_VARIANT, DeviceKind, DeviceState, NewDevice, SystemId, ensure_series,
    insert_device,
};
use iotkit_ingest_contract::TimeSource;

fn test_db() -> iotkit_core_storage::DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(crate::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    iotkit_core_storage::init_db_memory(&all).unwrap()
}

fn device(conn: &rusqlite::Connection) -> SystemId {
    insert_device(
        conn,
        &NewDevice {
            hardware_id: "ble:aa".into(),
            user_label: None,
            parent: None,
            kind: DeviceKind::Individual,
            initial_state: DeviceState::Active,
        },
    )
    .unwrap()
}

fn item(key: &str, channel: Option<u16>, values: Vec<f64>) -> ReadingItem {
    ReadingItem {
        subject_hint: Some("ble:aa".into()),
        measurement_key: key.into(),
        channel_index: channel,
        series_variant: None,
        values,
        device_time_ms: None,
        time_source: TimeSource::EdgeNode,
        age_ms: None,
        rssi: None,
        battery_pct: None,
    }
}

fn eval(conn: &rusqlite::Connection, sid: &SystemId, it: &ReadingItem) -> RegistryVerdict {
    evaluate_item(conn, sid, it).unwrap()
}

#[test]
fn catalog_key_first_arrival_auto_enables_with_audit() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        let v = eval(conn, &sid, &item("temperature_c", None, vec![21.5]));
        assert!(matches!(v,
                RegistryVerdict::Accept { ref resolved_key, quarantine: None, .. }
                if resolved_key == "temperature_c"));
        // copy-on-enableされている
        let entry = store::get_entry(conn, "temperature_c").unwrap().unwrap();
        assert_eq!(entry.origin, "catalog");
        // 監査イベント(D6決定4で必須)
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        // 2回目はauto-enableしない(冪等)
        eval(conn, &sid, &item("temperature_c", None, vec![22.0]));
        let n2: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n2, 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn unknown_key_is_quarantined_not_enabled() {
    let db = test_db();
    db.with_conn_sync(|conn| {
            let sid = device(conn);
            let v = eval(conn, &sid, &item("custom.tank_level", None, vec![42.0]));
            assert!(matches!(v,
                RegistryVerdict::Accept { ref resolved_key, quarantine: Some(QuarantineReason::UnknownKey), .. }
                if resolved_key == "custom.tank_level"));
            assert!(store::get_entry(conn, "custom.tank_level").unwrap().is_none(),
                "カタログ外キーは有効化されない(D6決定4)");
            Ok(())
        }).unwrap();
}

#[test]
fn in_range_and_out_of_range_against_catalog_physical_limit() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        let ok = eval(conn, &sid, &item("temperature_c", None, vec![21.5]));
        assert!(matches!(
            ok,
            RegistryVerdict::Accept {
                quarantine: None,
                ..
            }
        ));
        let hot = eval(conn, &sid, &item("temperature_c", None, vec![5000.0]));
        assert!(matches!(
            hot,
            RegistryVerdict::Accept {
                quarantine: Some(QuarantineReason::OutOfRange),
                ..
            }
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn series_range_override_narrows_catalog_range() {
    let db = test_db();
    db.with_conn_sync(|conn| {
            let sid = device(conn);
            // series実体化+個別値域を直接設定(設定APIはR14=Wave 1のためSQL直書きで模擬)
            ensure_series(conn, &sid, "temperature_c", CHANNEL_NA, DEFAULT_VARIANT, false, None).unwrap();
            conn.execute(
                "UPDATE series SET range_min = -10.0, range_max = 50.0 WHERE measurement_key='temperature_c'",
                [],
            ).unwrap();
            // 物理限界内(-200..1372)だがseries個別(-10..50)の外 → OutOfRange
            let v = eval(conn, &sid, &item("temperature_c", None, vec![100.0]));
            assert!(matches!(v,
                RegistryVerdict::Accept { quarantine: Some(QuarantineReason::OutOfRange), .. }));
            let ok = eval(conn, &sid, &item("temperature_c", None, vec![25.0]));
            assert!(matches!(ok, RegistryVerdict::Accept { quarantine: None, .. }));
            Ok(())
        }).unwrap();
}

#[test]
fn local_default_range_applies_when_no_series_override() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        eval(conn, &sid, &item("temperature_c", None, vec![21.5])); // auto-enable
        conn.execute(
            "UPDATE registry_entries SET local_min = 0.0, local_max = 100.0
                 WHERE measurement_key='temperature_c'",
            [],
        )
        .unwrap();
        let v = eval(conn, &sid, &item("temperature_c", None, vec![150.0]));
        assert!(matches!(
            v,
            RegistryVerdict::Accept {
                quarantine: Some(QuarantineReason::OutOfRange),
                ..
            }
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn bool_value_type_mismatch_is_terminal_reject() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        let ok = eval(conn, &sid, &item("contact_state", None, vec![1.0]));
        assert!(matches!(
            ok,
            RegistryVerdict::Accept {
                quarantine: None,
                ..
            }
        ));
        let bad = eval(conn, &sid, &item("contact_state", None, vec![3.0]));
        assert!(matches!(
            bad,
            RegistryVerdict::RejectItem {
                reason_code: ReasonCode::ValueTypeMismatch,
                ..
            }
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn scalar_with_multiple_values_and_empty_values_are_rejected() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        let multi = eval(conn, &sid, &item("temperature_c", None, vec![1.0, 2.0]));
        assert!(matches!(
            multi,
            RegistryVerdict::RejectItem {
                reason_code: ReasonCode::ValueTypeMismatch,
                ..
            }
        ));
        let empty = eval(conn, &sid, &item("temperature_c", None, vec![]));
        assert!(matches!(
            empty,
            RegistryVerdict::RejectItem {
                reason_code: ReasonCode::ValueTypeMismatch,
                ..
            }
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn vibration_spectrum_record_is_rejected_in_wave0() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        let v = eval(conn, &sid, &item("vibration_spectrum", None, vec![1.0]));
        assert!(
            matches!(
                v,
                RegistryVerdict::RejectItem {
                    reason_code: ReasonCode::ValueTypeMismatch,
                    ..
                }
            ),
            "record型のワイヤ表現は第二波(D6決定10)——f64配列としては解釈不能"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn fixed_channel_bounds_are_enforced() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        for ch in 0..3u16 {
            let v = eval(conn, &sid, &item("acceleration_mg", Some(ch), vec![100.0]));
            assert!(
                matches!(
                    v,
                    RegistryVerdict::Accept {
                        quarantine: None,
                        ..
                    }
                ),
                "channel {ch} is declared"
            );
        }
        let v = eval(conn, &sid, &item("acceleration_mg", Some(3), vec![100.0]));
        assert!(matches!(
            v,
            RegistryVerdict::Accept {
                quarantine: Some(QuarantineReason::UndeclaredChannel),
                ..
            }
        ));
        let none = eval(conn, &sid, &item("acceleration_mg", None, vec![100.0]));
        assert!(
            matches!(
                none,
                RegistryVerdict::Accept {
                    quarantine: Some(QuarantineReason::UndeclaredChannel),
                    ..
                }
            ),
            "fixed型でchannel_indexなしは帰属不能=宣言外扱い"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn single_channel_accepts_none_or_zero_only() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        assert!(matches!(
            eval(conn, &sid, &item("distance_mm", None, vec![100.0])),
            RegistryVerdict::Accept {
                quarantine: None,
                ..
            }
        ));
        assert!(matches!(
            eval(conn, &sid, &item("distance_mm", Some(0), vec![100.0])),
            RegistryVerdict::Accept {
                quarantine: None,
                ..
            }
        ));
        assert!(matches!(
            eval(conn, &sid, &item("distance_mm", Some(1), vec![100.0])),
            RegistryVerdict::Accept {
                quarantine: Some(QuarantineReason::UndeclaredChannel),
                ..
            }
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn generic_channel_accepts_any_index_in_wave0() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        for ch in [None, Some(0), Some(1), Some(7)] {
            let v = eval(conn, &sid, &item("voltage_mv", ch, vec![1650.0]));
            assert!(
                matches!(
                    v,
                    RegistryVerdict::Accept {
                        quarantine: None,
                        ..
                    }
                ),
                "generic modeは宣言照合なし(Wave 1)なので{ch:?}を通す"
            );
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn alias_resolves_to_canonical_for_unmaterialized_declared_key() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        let cat = standard_catalog();
        enable_entry(
            conn,
            cat.find("temperature_c").unwrap(),
            &cat.catalog_version,
            "test",
        )
        .unwrap();
        define_alias(
            conn,
            "temp_old",
            "temperature_c",
            AliasKind::LocationMapping,
        )
        .unwrap();
        let v = eval(conn, &sid, &item("temp_old", None, vec![21.5]));
        assert!(
            matches!(v,
                RegistryVerdict::Accept { ref resolved_key, quarantine: None, .. }
                if resolved_key == "temperature_c"),
            "未実体化の申告はcanonicalへ写像(D6決定3(b))"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn alias_keeps_declared_key_when_series_already_materialized() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        let cat = standard_catalog();
        enable_entry(
            conn,
            cat.find("temperature_c").unwrap(),
            &cat.catalog_version,
            "test",
        )
        .unwrap();
        // 先に申告キーのままのseriesが存在する状況を作る(検疫期にunknown keyとして実体化済み)
        ensure_series(
            conn,
            &sid,
            "temp_old",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            true,
            Some("unknown_key"),
        )
        .unwrap();
        // エイリアス確立=canonical定義バインドで検疫解除される(Task 3)
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
            CHANNEL_NA,
            DEFAULT_VARIANT,
        )
        .unwrap()
        .unwrap();
        assert!(
            !meta.quarantined,
            "確立時点でseries検疫は解除済み(D6決定3(a))"
        );
        let v = eval(conn, &sid, &item("temp_old", None, vec![21.5]));
        assert!(
            matches!(v,
                RegistryVerdict::Accept { ref resolved_key, quarantine: None, .. }
                if resolved_key == "temp_old"),
            "実体化済み申告キーはseries_key不変(D6決定3(a))。検証はcanonical定義で行う"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn single_mode_normalizes_some_zero_to_channel_na() {
    // single測定の None / Some(0) が同一seriesに落ちる(正準チャネル=番兵-1)
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        let v0 = eval(conn, &sid, &item("distance_mm", Some(0), vec![100.0]));
        assert!(matches!(
            v0,
            RegistryVerdict::Accept {
                channel_index: ledger::CHANNEL_NA,
                quarantine: None,
                ..
            }
        ));
        let vn = eval(conn, &sid, &item("distance_mm", None, vec![100.0]));
        assert!(matches!(
            vn,
            RegistryVerdict::Accept {
                channel_index: ledger::CHANNEL_NA,
                quarantine: None,
                ..
            }
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn single_mode_routes_to_existing_zero_series_when_channel_na_absent() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        ensure_series(conn, &sid, "distance_mm", 0, DEFAULT_VARIANT, false, None).unwrap();

        let v = eval(conn, &sid, &item("distance_mm", None, vec![100.0]));

        assert!(matches!(
            v,
            RegistryVerdict::Accept {
                channel_index: 0,
                quarantine: None,
                ..
            }
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn single_mode_prefers_channel_na_when_zero_and_na_series_coexist_and_audits() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        ensure_series(
            conn,
            &sid,
            "distance_mm",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();
        ensure_series(conn, &sid, "distance_mm", 0, DEFAULT_VARIANT, false, None).unwrap();

        let v = eval(conn, &sid, &item("distance_mm", Some(0), vec![100.0]));

        assert!(matches!(
            v,
            RegistryVerdict::Accept {
                channel_index: CHANNEL_NA,
                quarantine: None,
                ..
            }
        ));
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='channel_form_conflict'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(events, 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn range_fallback_is_per_side_preserving_outer_shell() {
    // D6決定7外殻不変則: series個別がminのみ設定でも、max側はカタログ物理限界が生きる
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        ensure_series(
            conn,
            &sid,
            "temperature_c",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();
        conn.execute(
            "UPDATE series SET range_min = -10.0 WHERE measurement_key='temperature_c'",
            [],
        )
        .unwrap(); // range_maxはNULLのまま
        let hot = eval(conn, &sid, &item("temperature_c", None, vec![5000.0]));
        assert!(
            matches!(
                hot,
                RegistryVerdict::Accept {
                    quarantine: Some(QuarantineReason::OutOfRange),
                    ..
                }
            ),
            "max側はカタログ物理限界(1372)が生きる——外殻は消えない"
        );
        let cold = eval(conn, &sid, &item("temperature_c", None, vec![-50.0]));
        assert!(
            matches!(
                cold,
                RegistryVerdict::Accept {
                    quarantine: Some(QuarantineReason::OutOfRange),
                    ..
                }
            ),
            "min側はseries個別(-10)が適用される"
        );
        let ok = eval(conn, &sid, &item("temperature_c", None, vec![25.0]));
        assert!(matches!(
            ok,
            RegistryVerdict::Accept {
                quarantine: None,
                ..
            }
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn unknown_key_priority_beats_channel_and_range_checks() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        // 未知キー+変なchannel: UnknownKeyが優先(定義がないので他の検査は無意味)
        let v = eval(conn, &sid, &item("custom.x", Some(9), vec![1e18]));
        assert!(matches!(
            v,
            RegistryVerdict::Accept {
                quarantine: Some(QuarantineReason::UnknownKey),
                ..
            }
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn non_finite_values_are_terminally_rejected() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let verdict = eval(conn, &sid, &item("temperature_c", None, vec![v]));
            assert!(
                matches!(
                    verdict,
                    RegistryVerdict::RejectItem {
                        reason_code: ReasonCode::ValueTypeMismatch,
                        ..
                    }
                ),
                "{v} must be terminally rejected, not quarantined or accepted"
            );
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn evaluate_propagates_storage_failure_as_err() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = device(conn);
        conn.execute_batch("PRAGMA query_only = ON;").unwrap();
        // auto-enableのINSERTが失敗する → Err(RejectItemに変換されないこと=D1)
        let r = evaluate_item(conn, &sid, &item("temperature_c", None, vec![21.5]));
        assert!(r.is_err(), "storage failure must surface as Err, got {r:?}");
        conn.execute_batch("PRAGMA query_only = OFF;").unwrap();
        Ok(())
    })
    .unwrap();
}
