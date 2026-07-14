use iotkit_core_ledger::{SystemId, series_key_of};
use rusqlite::params;

const SCHEMA_VERSION: u32 = 1;

#[derive(serde::Serialize)]
pub struct MeasurementRecord {
    pub family: &'static str,
    pub schema_version: u32,
    pub epoch: String,
    pub pub_seq: i64,
    pub series_key: String,
    pub values: Vec<f64>,
    pub event_time: i64,
    pub event_time_source: String,
    pub time_source: String,
    pub time_quality: String,
    pub received_at: i64,
    pub device_time: Option<i64>,
}

#[derive(serde::Serialize)]
pub struct AnnotationRecord {
    pub family: &'static str,
    pub schema_version: u32,
    pub epoch: String,
    pub pub_seq: i64,
    pub subtype: String,
    pub prior_epoch: String,
}

pub fn materialize_batch(
    conn: &rusqlite::Connection,
    rows: &[iotkit_core_publish::store::OutboxRow],
) -> Result<Vec<serde_json::Value>, String> {
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let v = match row.kind.as_str() {
            "measurement" => materialize_measurement(conn, row)?,
            "annotation" => materialize_annotation(row)?,
            _ => return Err(format!("unknown outbox kind: {}", row.kind)),
        };
        records.push(v);
    }
    Ok(records)
}

fn materialize_measurement(
    conn: &rusqlite::Connection,
    row: &iotkit_core_publish::store::OutboxRow,
) -> Result<serde_json::Value, String> {
    let reading_seq = row
        .reading_seq
        .ok_or_else(|| "measurement missing reading_seq".to_string())?;
    let (
        series_id,
        event_time,
        event_time_source,
        received_at,
        device_time,
        time_source,
        time_quality,
        values_json,
    ): (i64, i64, String, i64, Option<i64>, String, String, String) = conn
        .query_row(
            "SELECT series_id, event_time, event_time_source, received_at, device_time, time_source, time_quality, values_json
             FROM readings
             WHERE seq = ?1",
            params![reading_seq],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;

    let (system_id_blob, measurement_key, channel_index, variant): (Vec<u8>, String, i32, String) =
        conn.query_row(
            "SELECT system_id, measurement_key, channel_index, variant
             FROM series
             WHERE series_id = ?1",
            params![series_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|e| e.to_string())?;
    let system_id_bytes: [u8; 16] = system_id_blob
        .try_into()
        .map_err(|_| "system_id blob len".to_string())?;
    let system_id = SystemId::from_bytes(system_id_bytes);
    let values = serde_json::from_str::<Vec<f64>>(&values_json).map_err(|e| e.to_string())?;
    let series_key = series_key_of(&system_id, &measurement_key, channel_index, &variant);
    let rec = MeasurementRecord {
        family: "measurement",
        schema_version: SCHEMA_VERSION,
        epoch: row.epoch.clone(),
        pub_seq: row.pub_seq,
        series_key,
        values,
        event_time,
        event_time_source,
        time_source,
        time_quality,
        received_at,
        device_time,
    };
    serde_json::to_value(&rec).map_err(|e| e.to_string())
}

fn materialize_annotation(
    row: &iotkit_core_publish::store::OutboxRow,
) -> Result<serde_json::Value, String> {
    let annotation_json = row
        .annotation_json
        .as_deref()
        .ok_or_else(|| "annotation missing annotation_json".to_string())?;
    let payload: serde_json::Value =
        serde_json::from_str(annotation_json).map_err(|e| e.to_string())?;
    let subtype = row
        .subtype
        .clone()
        .ok_or_else(|| "annotation missing subtype".to_string())?;
    let prior_epoch = payload
        .get("prior_epoch")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "annotation missing prior_epoch".to_string())?
        .to_string();
    let rec = AnnotationRecord {
        family: "annotation",
        schema_version: SCHEMA_VERSION,
        epoch: row.epoch.clone(),
        pub_seq: row.pub_seq,
        subtype,
        prior_epoch,
    };
    serde_json::to_value(&rec).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_timeseries::NewReading;

    fn test_db() -> iotkit_core_storage::DbHandle {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
        all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
        all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
        all.sort_by_key(|m| m.version);
        iotkit_core_storage::init_db_memory(&all).unwrap()
    }

    fn outbox_row(kind: &str) -> iotkit_core_publish::store::OutboxRow {
        iotkit_core_publish::store::OutboxRow {
            pub_seq: 1,
            epoch: "E".into(),
            kind: kind.into(),
            subtype: None,
            reading_seq: None,
            annotation_json: None,
        }
    }

    #[test]
    fn series_key_renders_na_channel() {
        let sid = iotkit_core_ledger::SystemId::from_bytes([0x01u8; 16]);
        let k = series_key_of(
            &sid,
            "temperature",
            iotkit_core_ledger::CHANNEL_NA,
            "primary",
        );
        assert!(k.ends_with(":temperature:na:primary"));
        assert!(k.starts_with(&sid.to_text()));
    }

    #[test]
    fn measurement_record_has_all_spec7_fields_and_no_seq() {
        let r = MeasurementRecord {
            family: "measurement",
            schema_version: 1,
            epoch: "E".into(),
            pub_seq: 5,
            series_key: "s".into(),
            values: vec![1.0],
            event_time: 10,
            event_time_source: "device".into(),
            time_source: "device_ntp".into(),
            time_quality: "unsynced".into(),
            received_at: 9,
            device_time: Some(8),
        };
        let v = serde_json::to_value(&r).unwrap();
        for f in [
            "family",
            "schema_version",
            "epoch",
            "pub_seq",
            "series_key",
            "values",
            "event_time",
            "event_time_source",
            "time_source",
            "time_quality",
            "received_at",
            "device_time",
        ] {
            assert!(v.get(f).is_some(), "missing {f}");
        }
        assert!(v.get("seq").is_none(), "readings.seq を出口に出さない");
    }

    #[test]
    fn materialize_batch_builds_measurement_and_annotation_records() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "hw:record-test".into(),
                    user_label: None,
                    parent: None,
                    kind: iotkit_core_ledger::DeviceKind::Individual,
                    initial_state: iotkit_core_ledger::DeviceState::Active,
                },
            )
            .unwrap();
            let series_id = iotkit_core_ledger::ensure_series(
                conn,
                &sid,
                "temperature_c",
                iotkit_core_ledger::CHANNEL_NA,
                iotkit_core_ledger::DEFAULT_VARIANT,
                false,
                None,
            )
            .unwrap();
            let reading_seq = iotkit_core_timeseries::insert_reading_v3(
                conn,
                &NewReading {
                    series_id,
                    received_at_ms: 100,
                    device_time_ms: Some(90),
                    time_source: "device_ntp".into(),
                    values: vec![21.5],
                    rssi: None,
                    battery_pct: None,
                    quarantined: false,
                },
            )
            .unwrap();
            let pub_seq =
                iotkit_core_publish::store::enqueue_measurement(conn, "E", reading_seq, 110)
                    .unwrap();
            let batch = iotkit_core_publish::store::select_batch(conn, "E", 0, 10).unwrap();

            let records = materialize_batch(conn, &batch).unwrap();
            assert_eq!(records.len(), 1);
            let v = &records[0];
            assert_eq!(
                v.get("family").and_then(|v| v.as_str()),
                Some("measurement")
            );
            assert_eq!(v.get("pub_seq").and_then(|v| v.as_i64()), Some(pub_seq));
            assert_eq!(
                v.get("series_key").and_then(|v| v.as_str()),
                Some(
                    series_key_of(
                        &sid,
                        "temperature_c",
                        iotkit_core_ledger::CHANNEL_NA,
                        iotkit_core_ledger::DEFAULT_VARIANT,
                    )
                    .as_str()
                )
            );
            assert_eq!(v.get("values"), Some(&serde_json::json!([21.5])));
            assert!(v.get("seq").is_none(), "readings.seq を出口に出さない");

            let annotation = iotkit_core_publish::store::OutboxRow {
                pub_seq: 3,
                epoch: "NEW".into(),
                kind: "annotation".into(),
                subtype: Some("epoch_start".into()),
                reading_seq: None,
                annotation_json: Some(r#"{"prior_epoch":"OLD"}"#.into()),
            };
            let annotations = materialize_batch(conn, &[annotation]).unwrap();
            assert_eq!(annotations.len(), 1);
            let a = &annotations[0];
            assert_eq!(a.get("family").and_then(|v| v.as_str()), Some("annotation"));
            assert_eq!(a.get("schema_version").and_then(|v| v.as_u64()), Some(1));
            assert_eq!(a.get("epoch").and_then(|v| v.as_str()), Some("NEW"));
            assert_eq!(a.get("pub_seq").and_then(|v| v.as_i64()), Some(3));
            assert_eq!(
                a.get("subtype").and_then(|v| v.as_str()),
                Some("epoch_start")
            );
            assert_eq!(a.get("prior_epoch").and_then(|v| v.as_str()), Some("OLD"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn materialize_batch_rejects_unknown_kind() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let row = outbox_row("bogus");
            assert!(materialize_batch(conn, &[row]).is_err());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn materialize_batch_rejects_measurement_without_reading_seq() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let row = outbox_row("measurement");
            assert!(materialize_batch(conn, &[row]).is_err());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn materialize_batch_rejects_annotation_without_json() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let mut row = outbox_row("annotation");
            row.subtype = Some("epoch_start".into());
            assert!(materialize_batch(conn, &[row]).is_err());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn materialize_batch_rejects_annotation_json_that_is_not_object() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let mut row = outbox_row("annotation");
            row.subtype = Some("epoch_start".into());
            row.annotation_json = Some("[]".into());
            assert!(materialize_batch(conn, &[row]).is_err());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn materialize_batch_rejects_annotation_without_subtype() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let mut row = outbox_row("annotation");
            row.annotation_json = Some(r#"{"prior_epoch":"OLD"}"#.into());
            assert!(materialize_batch(conn, &[row]).is_err());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn materialize_batch_rejects_annotation_without_prior_epoch() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let mut row = outbox_row("annotation");
            row.subtype = Some("epoch_start".into());
            row.annotation_json = Some("{}".into());
            assert!(materialize_batch(conn, &[row]).is_err());
            Ok(())
        })
        .unwrap();
    }
}
