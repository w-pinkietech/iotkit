use crate::ids::SystemId;
use iotkit_core_storage::StorageError;
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug)]
pub enum LedgerError {
    HardwareIdInUse(String),
    NotFound(String),
    InvalidId(String),
    Storage(StorageError),
    Sqlite(rusqlite::Error),
}
impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HardwareIdInUse(h) => {
                write!(f, "hardware_id already in use by a live entry: {h}")
            }
            Self::NotFound(w) => write!(f, "not found: {w}"),
            Self::InvalidId(s) => write!(f, "invalid system_id text: {s}"),
            Self::Storage(e) => write!(f, "storage error: {e}"),
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
        }
    }
}
impl std::error::Error for LedgerError {}
impl From<rusqlite::Error> for LedgerError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Individual,
    Positional,
}
impl DeviceKind {
    fn as_db(&self) -> &'static str {
        match self {
            Self::Individual => "individual",
            Self::Positional => "positional",
        }
    }
    fn from_db(s: &str) -> Self {
        if s == "positional" {
            Self::Positional
        } else {
            Self::Individual
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    Quarantined,
    Active,
    Retired,
}
impl DeviceState {
    fn as_db(&self) -> &'static str {
        match self {
            Self::Quarantined => "quarantined",
            Self::Active => "active",
            Self::Retired => "retired",
        }
    }
    fn from_db(s: &str) -> Self {
        match s {
            "active" => Self::Active,
            "retired" => Self::Retired,
            _ => Self::Quarantined,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceRow {
    pub system_id: SystemId,
    pub hardware_id: String,
    pub user_label: Option<String>,
    pub parent: Option<SystemId>,
    pub kind: DeviceKind,
    pub state: DeviceState,
    pub declaration_version: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct SeriesRow {
    pub series_id: i64,
    pub system_id: SystemId,
    pub measurement_key: String,
    pub channel_index: i32,
    pub variant: String,
    pub quarantined: bool,
    pub quarantine_reason: Option<String>,
    pub value_semantics: String,
    pub unit: Option<String>,
    pub range_min: Option<f64>,
    pub range_max: Option<f64>,
    pub calibration_review: bool,
}

#[derive(Debug, Clone)]
pub struct SightingRow {
    pub hardware_id: String,
    pub source: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub observations: i64,
}

#[derive(Debug, Clone)]
pub struct EventRow {
    pub event_id: i64,
    pub at: i64,
    pub kind: String,
    pub system_id: Option<SystemId>,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct NewDevice {
    pub hardware_id: String,
    pub user_label: Option<String>,
    pub parent: Option<SystemId>,
    pub kind: DeviceKind,
    pub initial_state: DeviceState,
}

/// チャネル正規化の一箇所(CLAUDE.md変換境界規律): channel_indexなしの番兵値と既定variant。
/// collectorとregistryの両方がこの定数を使う(重複定義禁止)。
pub const CHANNEL_NA: i32 = -1;
pub const DEFAULT_VARIANT: &str = "primary";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn system_id_from_blob(bytes: Vec<u8>, label: &str) -> SystemId {
    SystemId::from_bytes(bytes.try_into().unwrap_or_else(|_| panic!("16-byte {label}")))
}

fn row_to_device(row: &rusqlite::Row<'_>) -> Result<DeviceRow, rusqlite::Error> {
    let sid: Vec<u8> = row.get(0)?;
    let parent: Option<Vec<u8>> = row.get(3)?;
    Ok(DeviceRow {
        system_id: system_id_from_blob(sid, "system_id"),
        hardware_id: row.get(1)?,
        user_label: row.get(2)?,
        parent: parent.map(|p| system_id_from_blob(p, "parent id")),
        kind: DeviceKind::from_db(&row.get::<_, String>(4)?),
        state: DeviceState::from_db(&row.get::<_, String>(5)?),
        declaration_version: row.get(6)?,
    })
}

const DEVICE_COLS: &str =
    "system_id, hardware_id, user_label, parent_system_id, kind, state, declaration_version";

/// append-only監査イベント(R13最小下地)への行追記。registryクレート等の外部呼び出し用公開面。
pub fn record_event(
    conn: &Connection,
    kind: &str,
    system_id: Option<&SystemId>,
    detail: &str,
) -> Result<(), LedgerError> {
    conn.execute(
        "INSERT INTO ledger_events (at, kind, system_id, detail) VALUES (?1, ?2, ?3, ?4)",
        params![
            now_ms(),
            kind,
            system_id.map(|s| s.as_bytes().to_vec()),
            detail
        ],
    )?;
    Ok(())
}

pub fn insert_device(conn: &Connection, new: &NewDevice) -> Result<SystemId, LedgerError> {
    if find_alive_by_hardware_id(conn, &new.hardware_id)?.is_some() {
        return Err(LedgerError::HardwareIdInUse(new.hardware_id.clone()));
    }
    let sid = SystemId::generate();
    conn.execute(
        "INSERT INTO devices (system_id, hardware_id, user_label, parent_system_id, kind, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            sid.as_bytes().to_vec(), new.hardware_id, new.user_label,
            new.parent.map(|p| p.as_bytes().to_vec()),
            new.kind.as_db(), new.initial_state.as_db(), now_ms()
        ],
    )?;
    record_event(conn, "device_registered", Some(&sid), &new.hardware_id)?;
    Ok(sid)
}

pub fn find_alive_by_hardware_id(
    conn: &Connection,
    hardware_id: &str,
) -> Result<Option<DeviceRow>, LedgerError> {
    conn.query_row(
        &format!("SELECT {DEVICE_COLS} FROM devices WHERE hardware_id = ?1 AND state != 'retired'"),
        params![hardware_id],
        row_to_device,
    )
    .optional()
    .map_err(LedgerError::from)
}

pub fn list_devices(conn: &Connection, include_retired: bool) -> Result<Vec<DeviceRow>, LedgerError> {
    let sql = if include_retired {
        format!("SELECT {DEVICE_COLS} FROM devices ORDER BY created_at ASC, hardware_id ASC")
    } else {
        format!(
            "SELECT {DEVICE_COLS} FROM devices
             WHERE state != 'retired' ORDER BY created_at ASC, hardware_id ASC"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_map([], row_to_device)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(LedgerError::from)
}

pub fn get_device(conn: &Connection, system_id: &SystemId) -> Result<Option<DeviceRow>, LedgerError> {
    conn.query_row(
        &format!("SELECT {DEVICE_COLS} FROM devices WHERE system_id = ?1"),
        params![system_id.as_bytes().to_vec()],
        row_to_device,
    )
    .optional()
    .map_err(LedgerError::from)
}

pub fn ensure_series(
    conn: &Connection,
    system_id: &SystemId,
    measurement_key: &str,
    channel_index: i32,
    variant: &str,
    quarantined: bool,
    quarantine_reason: Option<&str>,
) -> Result<i64, LedgerError> {
    if let Some(id) = conn
        .query_row(
            "SELECT series_id FROM series
         WHERE system_id = ?1 AND measurement_key = ?2 AND channel_index = ?3 AND variant = ?4",
            params![
                system_id.as_bytes().to_vec(),
                measurement_key,
                channel_index,
                variant
            ],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO series (system_id, measurement_key, channel_index, variant, quarantined, quarantine_reason, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![system_id.as_bytes().to_vec(), measurement_key, channel_index, variant,
            quarantined as i32, quarantine_reason, now_ms()],
    )?;
    Ok(conn.last_insert_rowid())
}

#[derive(Debug, Clone)]
pub struct SeriesMeta {
    pub series_id: i64,
    pub quarantined: bool,
    pub quarantine_reason: Option<String>,
    pub range_min: Option<f64>,
    pub range_max: Option<f64>,
}

pub fn find_series_meta(
    conn: &Connection,
    system_id: &SystemId,
    measurement_key: &str,
    channel_index: i32,
    variant: &str,
) -> Result<Option<SeriesMeta>, LedgerError> {
    conn.query_row(
        "SELECT series_id, quarantined, quarantine_reason, range_min, range_max FROM series
         WHERE system_id = ?1 AND measurement_key = ?2 AND channel_index = ?3 AND variant = ?4",
        params![
            system_id.as_bytes().to_vec(),
            measurement_key,
            channel_index,
            variant
        ],
        |row| {
            Ok(SeriesMeta {
                series_id: row.get(0)?,
                quarantined: row.get::<_, i32>(1)? != 0,
                quarantine_reason: row.get(2)?,
                range_min: row.get(3)?,
                range_max: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(LedgerError::from)
}

pub fn list_series_for_device(
    conn: &Connection,
    system_id: &SystemId,
) -> Result<Vec<SeriesRow>, LedgerError> {
    let mut stmt = conn.prepare(
        "SELECT series_id, system_id, measurement_key, channel_index, variant, quarantined,
            quarantine_reason, value_semantics, unit, range_min, range_max, calibration_review
         FROM series WHERE system_id = ?1 ORDER BY series_id ASC",
    )?;
    stmt.query_map(params![system_id.as_bytes().to_vec()], |row| {
        let sid: Vec<u8> = row.get(1)?;
        Ok(SeriesRow {
            series_id: row.get(0)?,
            system_id: system_id_from_blob(sid, "series.system_id"),
            measurement_key: row.get(2)?,
            channel_index: row.get(3)?,
            variant: row.get(4)?,
            quarantined: row.get::<_, i32>(5)? != 0,
            quarantine_reason: row.get(6)?,
            value_semantics: row.get(7)?,
            unit: row.get(8)?,
            range_min: row.get(9)?,
            range_max: row.get(10)?,
            calibration_review: row.get::<_, i32>(11)? != 0,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(LedgerError::from)
}

/// D6決定3(a): 当該subjectで申告キーのseriesが(channel/variant不問で)実体化済みか。
pub fn series_exists_for_key(
    conn: &Connection,
    system_id: &SystemId,
    measurement_key: &str,
) -> Result<bool, LedgerError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM series WHERE system_id = ?1 AND measurement_key = ?2",
        params![system_id.as_bytes().to_vec(), measurement_key],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

/// エイリアス確立時の検疫解除(D6決定3(a)): 申告キーのまま実体化済みの検疫seriesに
/// canonical定義がバインドされたため、series級検疫を解く。過去の検疫行(readings)は
/// 履歴としてそのまま(保存済みデータの解釈を遡って変えない)。解除対象は
/// `quarantine_reason` が一致するseriesのみ(undeclared_channel等はエイリアスでは解決しない)。
pub fn release_series_quarantine_for_key(
    conn: &Connection,
    measurement_key: &str,
    reason: &str,
) -> Result<Vec<i64>, LedgerError> {
    let mut stmt = conn.prepare(
        "SELECT series_id FROM series
         WHERE measurement_key = ?1 AND quarantined = 1 AND quarantine_reason = ?2",
    )?;
    let ids: Vec<i64> = stmt
        .query_map(params![measurement_key, reason], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    if !ids.is_empty() {
        conn.execute(
            "UPDATE series SET quarantined = 0, quarantine_reason = NULL
             WHERE measurement_key = ?1 AND quarantined = 1 AND quarantine_reason = ?2",
            params![measurement_key, reason],
        )?;
    }
    Ok(ids)
}

pub fn record_sighting(
    conn: &Connection,
    hardware_id: &str,
    source: &str,
) -> Result<(), LedgerError> {
    conn.execute(
        "INSERT INTO sightings (hardware_id, source, first_seen, last_seen, observations)
         VALUES (?1, ?2, ?3, ?3, 1)
         ON CONFLICT(hardware_id) DO UPDATE SET last_seen = ?3, observations = observations + 1",
        params![hardware_id, source, now_ms()],
    )?;
    Ok(())
}

pub fn list_sightings(conn: &Connection) -> Result<Vec<SightingRow>, LedgerError> {
    let mut stmt = conn.prepare(
        "SELECT hardware_id, source, first_seen, last_seen, observations
         FROM sightings ORDER BY last_seen DESC, hardware_id ASC",
    )?;
    stmt.query_map([], |row| {
        Ok(SightingRow {
            hardware_id: row.get(0)?,
            source: row.get(1)?,
            first_seen: row.get(2)?,
            last_seen: row.get(3)?,
            observations: row.get(4)?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(LedgerError::from)
}

pub fn list_recent_events(conn: &Connection, limit: u32) -> Result<Vec<EventRow>, LedgerError> {
    let mut stmt = conn.prepare(
        "SELECT event_id, at, kind, system_id, detail
         FROM ledger_events ORDER BY event_id DESC LIMIT ?1",
    )?;
    stmt.query_map(params![limit], |row| {
        let sid: Option<Vec<u8>> = row.get(3)?;
        Ok(EventRow {
            event_id: row.get(0)?,
            at: row.get(1)?,
            kind: row.get(2)?,
            system_id: sid.map(|b| system_id_from_blob(b, "event.system_id")),
            detail: row.get(4)?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(LedgerError::from)
}

pub fn approve_sighting(
    conn: &Connection,
    hardware_id: &str,
    user_label: Option<&str>,
    kind: DeviceKind,
) -> Result<SystemId, LedgerError> {
    let seen: bool = conn
        .query_row(
            "SELECT 1 FROM sightings WHERE hardware_id = ?1",
            params![hardware_id],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !seen {
        return Err(LedgerError::NotFound(format!("sighting {hardware_id}")));
    }
    let sid = insert_device(
        conn,
        &NewDevice {
            hardware_id: hardware_id.to_string(),
            user_label: user_label.map(String::from),
            parent: None,
            kind,
            initial_state: DeviceState::Quarantined, // D5経路A: 承認→検疫→active
        },
    )?;
    conn.execute(
        "DELETE FROM sightings WHERE hardware_id = ?1",
        params![hardware_id],
    )?;
    record_event(conn, "sighting_approved", Some(&sid), hardware_id)?;
    Ok(sid)
}

pub fn activate_device(conn: &Connection, system_id: &SystemId) -> Result<(), LedgerError> {
    let n = conn.execute(
        "UPDATE devices SET state = 'active' WHERE system_id = ?1 AND state = 'quarantined'",
        params![system_id.as_bytes().to_vec()],
    )?;
    if n == 0 {
        return Err(LedgerError::NotFound(format!(
            "quarantined device {}",
            system_id.to_text()
        )));
    }
    record_event(conn, "device_activated", Some(system_id), "")?;
    Ok(())
}

/// 台帳エポック(D5決定3の複合カーソル (epoch, seq) の前半)。初回に生成し永続化。
pub fn ledger_epoch(conn: &Connection) -> Result<String, LedgerError> {
    if let Some(v) = conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'epoch'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(v);
    }
    let epoch = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO ledger_meta (key, value) VALUES ('epoch', ?1)",
        params![epoch],
    )?;
    Ok(epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_storage::init_db_memory;

    fn test_db() -> iotkit_core_storage::DbHandle {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(crate::MIGRATIONS);
        init_db_memory(&all).expect("in-memory db")
    }

    #[test]
    fn insert_and_resolve_device_by_hardware_id() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(
                conn,
                &NewDevice {
                    hardware_id: "ble:00000000000000ab".into(),
                    user_label: Some("炉1温度".into()),
                    parent: None,
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
            let row = find_alive_by_hardware_id(conn, "ble:00000000000000ab")
                .unwrap()
                .unwrap();
            assert_eq!(row.system_id, sid);
            assert_eq!(row.kind, DeviceKind::Individual);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn duplicate_alive_hardware_id_is_rejected() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let nd = NewDevice {
                hardware_id: "i2c:0x60".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Positional,
                initial_state: DeviceState::Active,
            };
            insert_device(conn, &nd).unwrap();
            assert!(matches!(
                insert_device(conn, &nd),
                Err(LedgerError::HardwareIdInUse(_))
            ));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn ensure_series_is_idempotent_and_monotonic() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(
                conn,
                &NewDevice {
                    hardware_id: "ble:cc".into(),
                    user_label: None,
                    parent: None,
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
            let s1 = ensure_series(
                conn,
                &sid,
                "temperature_c",
                CHANNEL_NA,
                DEFAULT_VARIANT,
                false,
                None,
            )
            .unwrap();
            let s2 = ensure_series(
                conn,
                &sid,
                "temperature_c",
                CHANNEL_NA,
                DEFAULT_VARIANT,
                false,
                None,
            )
            .unwrap();
            let s3 =
                ensure_series(conn, &sid, "voltage_mv", 0, DEFAULT_VARIANT, false, None).unwrap();
            assert_eq!(s1, s2);
            assert!(s3 > s1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn ensure_series_stores_quarantine_reason() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(
                conn,
                &NewDevice {
                    hardware_id: "ble:qr".into(),
                    user_label: None,
                    parent: None,
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
            let id = ensure_series(
                conn,
                &sid,
                "custom.mystery",
                CHANNEL_NA,
                DEFAULT_VARIANT,
                true,
                Some("unknown_key"),
            )
            .unwrap();
            let meta = find_series_meta(conn, &sid, "custom.mystery", CHANNEL_NA, DEFAULT_VARIANT)
                .unwrap()
                .unwrap();
            assert_eq!(meta.series_id, id);
            assert!(meta.quarantined);
            assert_eq!(meta.quarantine_reason.as_deref(), Some("unknown_key"));
            assert_eq!(meta.range_min, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn series_calibration_review_defaults_to_zero() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(
                conn,
                &NewDevice {
                    hardware_id: "ble:cal".into(),
                    user_label: None,
                    parent: None,
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
            let series_id = ensure_series(
                conn,
                &sid,
                "temperature_c",
                CHANNEL_NA,
                DEFAULT_VARIANT,
                false,
                None,
            )
            .unwrap();
            let calibration_review: i64 = conn
                .query_row(
                    "SELECT calibration_review FROM series WHERE series_id = ?1",
                    params![series_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(calibration_review, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn series_exists_for_key_ignores_channel_and_variant() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(
                conn,
                &NewDevice {
                    hardware_id: "ble:ex".into(),
                    user_label: None,
                    parent: None,
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
            assert!(!series_exists_for_key(conn, &sid, "temp_old").unwrap());
            ensure_series(conn, &sid, "temp_old", 2, "count", false, None).unwrap();
            assert!(series_exists_for_key(conn, &sid, "temp_old").unwrap());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn record_event_appends_to_ledger_events() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            record_event(
                conn,
                "registry_entry_enabled",
                None,
                r#"{"key":"temperature_c"}"#,
            )
            .unwrap();
            let (kind, detail): (String, String) = conn
                .query_row(
                    "SELECT kind, detail FROM ledger_events ORDER BY event_id DESC LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(kind, "registry_entry_enabled");
            assert!(detail.contains("temperature_c"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn release_series_quarantine_clears_matching_reason_only() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(
                conn,
                &NewDevice {
                    hardware_id: "ble:rel".into(),
                    user_label: None,
                    parent: None,
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
            let a = ensure_series(
                conn,
                &sid,
                "temp_old",
                CHANNEL_NA,
                DEFAULT_VARIANT,
                true,
                Some("unknown_key"),
            )
            .unwrap();
            ensure_series(
                conn,
                &sid,
                "other_key",
                CHANNEL_NA,
                DEFAULT_VARIANT,
                true,
                Some("undeclared_channel"),
            )
            .unwrap();
            let released =
                release_series_quarantine_for_key(conn, "temp_old", "unknown_key").unwrap();
            assert_eq!(released, vec![a]);
            let meta = find_series_meta(conn, &sid, "temp_old", CHANNEL_NA, DEFAULT_VARIANT)
                .unwrap()
                .unwrap();
            assert!(!meta.quarantined);
            assert_eq!(meta.quarantine_reason, None);
            // キーも理由も異なるseriesは対象外
            let other = find_series_meta(conn, &sid, "other_key", CHANNEL_NA, DEFAULT_VARIANT)
                .unwrap()
                .unwrap();
            assert!(other.quarantined);
            // 対象なしの冪等呼び出し
            assert!(
                release_series_quarantine_for_key(conn, "temp_old", "unknown_key")
                    .unwrap()
                    .is_empty()
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn find_series_meta_returns_range_override() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(
                conn,
                &NewDevice {
                    hardware_id: "ble:rng".into(),
                    user_label: None,
                    parent: None,
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
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
            // Wave 0にはseries値域の設定APIがない(R14=計画4)ため、直接SQLで個別上書きを模擬
            conn.execute(
                "UPDATE series SET range_min = -10.0, range_max = 50.0
                 WHERE system_id = ?1 AND measurement_key = 'temperature_c'",
                params![sid.as_bytes().to_vec()],
            )
            .unwrap();
            let meta = find_series_meta(conn, &sid, "temperature_c", CHANNEL_NA, DEFAULT_VARIANT)
                .unwrap()
                .unwrap();
            assert_eq!(meta.range_min, Some(-10.0));
            assert_eq!(meta.range_max, Some(50.0));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn sighting_then_approve_creates_quarantined_device() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            record_sighting(conn, "ble:ff", "bravepi-mainboard").unwrap();
            record_sighting(conn, "ble:ff", "bravepi-mainboard").unwrap();
            let sid = approve_sighting(conn, "ble:ff", Some("新センサー"), DeviceKind::Individual)
                .unwrap();
            let row = find_alive_by_hardware_id(conn, "ble:ff").unwrap().unwrap();
            assert_eq!(row.system_id, sid);
            assert_eq!(row.state, DeviceState::Quarantined);
            activate_device(conn, &sid).unwrap();
            assert_eq!(
                find_alive_by_hardware_id(conn, "ble:ff")
                    .unwrap()
                    .unwrap()
                    .state,
                DeviceState::Active
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn retired_hardware_id_becomes_reusable() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let hardware_id = "ble:retire01";
            let sid1 = insert_device(
                conn,
                &NewDevice {
                    hardware_id: hardware_id.into(),
                    user_label: None,
                    parent: None,
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();

            // retiredへ直接遷移(アプリ層のretire APIが未実装のためraw SQLで模擬)
            conn.execute(
                "UPDATE devices SET state = 'retired', retired_at = ?1 WHERE system_id = ?2",
                params![now_ms(), sid1.as_bytes().to_vec()],
            )
            .unwrap();

            // partial unique index はstate != 'retired'のみ対象のため、同一hardware_idの再登録が成功するはず
            let sid2 = insert_device(
                conn,
                &NewDevice {
                    hardware_id: hardware_id.into(),
                    user_label: None,
                    parent: None,
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
            assert_ne!(sid1, sid2);

            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM devices WHERE hardware_id = ?1",
                    params![hardware_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 2, "retired行と新規active行がDB上に共存するはず");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn db_level_unique_constraint_rejects_alive_duplicate() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let hardware_id = "ble:dbunique01";
            insert_device(conn, &NewDevice {
                hardware_id: hardware_id.into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            }).unwrap();

            // アプリ層の事前チェック(find_alive_by_hardware_id)をバイパスし、
            // DB側のpartial unique index (idx_devices_hardware_alive) 単独で
            // alive重複を弾けることを検証する。
            let other_sid = SystemId::generate();
            let err = conn
                .execute(
                    "INSERT INTO devices (system_id, hardware_id, user_label, parent_system_id, kind, state, created_at)
                     VALUES (?1, ?2, NULL, NULL, ?3, 'active', ?4)",
                    params![
                        other_sid.as_bytes().to_vec(),
                        hardware_id,
                        DeviceKind::Individual.as_db(),
                        now_ms()
                    ],
                )
                .expect_err("DB-level partial unique indexがalive重複を拒否するはず");

            match err {
                rusqlite::Error::SqliteFailure(e, _) => {
                    assert_eq!(
                        e.code,
                        rusqlite::ErrorCode::ConstraintViolation,
                        "unique制約違反であるはず: {e:?}"
                    );
                }
                other => panic!("unique制約違反(SqliteFailure)を期待したが別のエラー: {other:?}"),
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn ledger_epoch_is_generated_once_and_stable() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let e1 = ledger_epoch(conn).unwrap();
            let e2 = ledger_epoch(conn).unwrap();
            assert_eq!(e1, e2);
            assert!(!e1.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn listing_apis_return_inserted_devices_series_sightings_and_events() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let parent = insert_device(
                conn,
                &NewDevice {
                    hardware_id: "ble:parent".into(),
                    user_label: Some("parent".into()),
                    parent: None,
                    kind: DeviceKind::Positional,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
            let child = insert_device(
                conn,
                &NewDevice {
                    hardware_id: "ble:child".into(),
                    user_label: Some("child".into()),
                    parent: Some(parent),
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
            let retired = insert_device(
                conn,
                &NewDevice {
                    hardware_id: "ble:retired".into(),
                    user_label: None,
                    parent: None,
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
            conn.execute(
                "UPDATE devices SET state = 'retired' WHERE system_id = ?1",
                params![retired.as_bytes().to_vec()],
            )
            .unwrap();

            let child_row = get_device(conn, &child).unwrap().unwrap();
            assert_eq!(child_row.hardware_id, "ble:child");
            assert_eq!(child_row.parent, Some(parent));
            let parent_row = get_device(conn, &parent).unwrap().unwrap();
            assert_eq!(parent_row.hardware_id, "ble:parent");
            assert_eq!(parent_row.parent, None);

            let alive = list_devices(conn, false).unwrap();
            assert_eq!(alive.len(), 2);
            assert!(alive.iter().all(|d| d.state != DeviceState::Retired));
            let all = list_devices(conn, true).unwrap();
            assert_eq!(all.len(), 3);
            let listed_parent = all.iter().find(|d| d.system_id == parent).unwrap();
            assert_eq!(listed_parent.parent, None);

            let series_id = ensure_series(
                conn,
                &child,
                "temperature_c",
                7,
                "backup",
                true,
                Some("unknown_key"),
            )
            .unwrap();
            conn.execute(
                "UPDATE series SET unit = 'Cel', range_min = -10.0, range_max = 50.0,
                    calibration_review = 1 WHERE series_id = ?1",
                params![series_id],
            )
            .unwrap();
            let series = list_series_for_device(conn, &child).unwrap();
            assert_eq!(series.len(), 1);
            assert_eq!(series[0].series_id, series_id);
            assert_eq!(series[0].system_id, child);
            assert_eq!(series[0].measurement_key, "temperature_c");
            assert_eq!(series[0].channel_index, 7);
            assert_eq!(series[0].variant, "backup");
            assert!(series[0].quarantined);
            assert_eq!(series[0].quarantine_reason.as_deref(), Some("unknown_key"));
            assert_eq!(series[0].value_semantics, "calibrated");
            assert_eq!(series[0].unit.as_deref(), Some("Cel"));
            assert_eq!(series[0].range_min, Some(-10.0));
            assert_eq!(series[0].range_max, Some(50.0));
            assert!(series[0].calibration_review);

            record_sighting(conn, "ble:seen", "adapter-a").unwrap();
            let sightings = list_sightings(conn).unwrap();
            assert_eq!(sightings.len(), 1);
            assert_eq!(sightings[0].hardware_id, "ble:seen");
            assert_eq!(sightings[0].source, "adapter-a");
            assert_eq!(sightings[0].observations, 1);

            record_event(conn, "manual", Some(&child), r#"{"ok":true}"#).unwrap();
            record_event(conn, "system_note", None, r#"{"ok":false}"#).unwrap();
            let events = list_recent_events(conn, 2).unwrap();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].kind, "system_note");
            assert_eq!(events[0].system_id, None);
            assert_eq!(events[1].kind, "manual");
            assert_eq!(events[1].system_id, Some(child));
            Ok(())
        })
        .unwrap();
    }
}
