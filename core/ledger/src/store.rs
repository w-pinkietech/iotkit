use crate::ids::SystemId;
use iotkit_core_storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

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
            Self::HardwareIdInUse(h) => write!(f, "hardware_id already in use by a live entry: {h}"),
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
pub struct NewDevice {
    pub hardware_id: String,
    pub user_label: Option<String>,
    pub parent: Option<SystemId>,
    pub kind: DeviceKind,
    pub initial_state: DeviceState,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn audit(
    conn: &Connection,
    kind: &str,
    system_id: Option<&SystemId>,
    detail: &str,
) -> Result<(), LedgerError> {
    conn.execute(
        "INSERT INTO ledger_events (at, kind, system_id, detail) VALUES (?1, ?2, ?3, ?4)",
        params![now_ms(), kind, system_id.map(|s| s.as_bytes().to_vec()), detail],
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
    audit(conn, "device_registered", Some(&sid), &new.hardware_id)?;
    Ok(sid)
}

pub fn find_alive_by_hardware_id(
    conn: &Connection,
    hardware_id: &str,
) -> Result<Option<DeviceRow>, LedgerError> {
    conn.query_row(
        "SELECT system_id, hardware_id, user_label, parent_system_id, kind, state, declaration_version
         FROM devices WHERE hardware_id = ?1 AND state != 'retired'",
        params![hardware_id],
        |row| {
            let sid: Vec<u8> = row.get(0)?;
            let parent: Option<Vec<u8>> = row.get(3)?;
            Ok(DeviceRow {
                system_id: SystemId::from_bytes(sid.try_into().expect("16-byte system_id")),
                hardware_id: row.get(1)?,
                user_label: row.get(2)?,
                parent: parent.map(|p| SystemId::from_bytes(p.try_into().expect("16-byte parent id"))),
                kind: DeviceKind::from_db(&row.get::<_, String>(4)?),
                state: DeviceState::from_db(&row.get::<_, String>(5)?),
                declaration_version: row.get(6)?,
            })
        },
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
) -> Result<i64, LedgerError> {
    if let Some(id) = conn
        .query_row(
            "SELECT series_id FROM series
         WHERE system_id = ?1 AND measurement_key = ?2 AND channel_index = ?3 AND variant = ?4",
            params![system_id.as_bytes().to_vec(), measurement_key, channel_index, variant],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO series (system_id, measurement_key, channel_index, variant, quarantined, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![system_id.as_bytes().to_vec(), measurement_key, channel_index, variant, quarantined as i32, now_ms()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn record_sighting(conn: &Connection, hardware_id: &str, source: &str) -> Result<(), LedgerError> {
    conn.execute(
        "INSERT INTO sightings (hardware_id, source, first_seen, last_seen, observations)
         VALUES (?1, ?2, ?3, ?3, 1)
         ON CONFLICT(hardware_id) DO UPDATE SET last_seen = ?3, observations = observations + 1",
        params![hardware_id, source, now_ms()],
    )?;
    Ok(())
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
    conn.execute("DELETE FROM sightings WHERE hardware_id = ?1", params![hardware_id])?;
    audit(conn, "sighting_approved", Some(&sid), hardware_id)?;
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
    audit(conn, "device_activated", Some(system_id), "")?;
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
    conn.execute("INSERT INTO ledger_meta (key, value) VALUES ('epoch', ?1)", params![epoch])?;
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
            let sid = insert_device(conn, &NewDevice {
                hardware_id: "ble:00000000000000ab".into(),
                user_label: Some("炉1温度".into()),
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            }).unwrap();
            let row = find_alive_by_hardware_id(conn, "ble:00000000000000ab").unwrap().unwrap();
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
            assert!(matches!(insert_device(conn, &nd), Err(LedgerError::HardwareIdInUse(_))));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn ensure_series_is_idempotent_and_monotonic() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(conn, &NewDevice {
                hardware_id: "ble:cc".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            }).unwrap();
            let s1 = ensure_series(conn, &sid, "temperature_c", -1, "primary", false).unwrap();
            let s2 = ensure_series(conn, &sid, "temperature_c", -1, "primary", false).unwrap();
            let s3 = ensure_series(conn, &sid, "voltage_mv", 0, "primary", false).unwrap();
            assert_eq!(s1, s2);
            assert!(s3 > s1);
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
            let sid = approve_sighting(conn, "ble:ff", Some("新センサー"), DeviceKind::Individual).unwrap();
            let row = find_alive_by_hardware_id(conn, "ble:ff").unwrap().unwrap();
            assert_eq!(row.system_id, sid);
            assert_eq!(row.state, DeviceState::Quarantined);
            activate_device(conn, &sid).unwrap();
            assert_eq!(
                find_alive_by_hardware_id(conn, "ble:ff").unwrap().unwrap().state,
                DeviceState::Active
            );
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
}
