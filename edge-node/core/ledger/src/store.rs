use crate::ids::SystemId;
use iotkit_core_storage::StorageError;
use rusqlite::{Connection, OptionalExtension, params, types::Type};

#[derive(Debug)]
pub enum LedgerError {
    HardwareIdInUse(String),
    NotFound(String),
    InvalidId(String),
    InvalidModelId(String),
    InvalidReplace(String),
    UnsupportedPreReleaseSchema,
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
            Self::InvalidModelId(s) => write!(f, "invalid model_id: {s}"),
            Self::InvalidReplace(s) => write!(f, "invalid replace: {s}"),
            Self::UnsupportedPreReleaseSchema => write!(
                f,
                "unsupported pre-release Edge Node database; recreate the Edge Node database"
            ),
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
        match s {
            "positional" => Self::Positional,
            "individual" => Self::Individual,
            other => {
                tracing::warn!(
                    value = other,
                    fallback = "individual",
                    "unknown device kind in ledger row"
                );
                Self::Individual
            }
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
    pub presentation_identifier: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSeriesKey {
    pub system_id: SystemId,
    pub measurement_key: String,
    pub channel_index: i32,
    pub variant: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesListRow {
    pub series_id: i64,
    pub series_key: String,
    pub system_id: String,
    pub user_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeNodeIdentity {
    pub edge_node_id: String,
    pub ledger_epoch: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceOutcome {
    pub replaced: SystemId,
    pub old_hardware_id: String,
    pub retired_candidates: Vec<SystemId>,
}

/// チャネル正規化の一箇所(CLAUDE.md変換境界規律): channel_indexなしの番兵値と既定variant。
/// collectorとregistryの両方がこの定数を使う(重複定義禁止)。
pub const CHANNEL_NA: i32 = -1;
pub const DEFAULT_VARIANT: &str = "primary";

/// Sightings not observed within this window are stale housekeeping and dropped.
const SIGHTINGS_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000; // 30 days
/// Hard cap on unapproved sightings; the LRU (oldest last_seen) rows beyond this
/// are evicted. This is the bound that survives a unique-id flood.
const SIGHTINGS_CAP: i64 = 10_000;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn system_id_from_blob(bytes: Vec<u8>, label: &str) -> Result<SystemId, rusqlite::Error> {
    let len = bytes.len();
    let bytes: [u8; 16] = bytes.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            Type::Blob,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{label} must be a 16-byte UUID blob, got {len} bytes"),
            )),
        )
    })?;
    Ok(SystemId::from_bytes(bytes))
}

pub fn series_key_of(
    system_id: &SystemId,
    measurement_key: &str,
    channel_index: i32,
    variant: &str,
) -> String {
    let channel = if channel_index == CHANNEL_NA {
        "na".to_string()
    } else {
        channel_index.to_string()
    };
    format!(
        "{}:{}:{}:{}",
        system_id.to_text(),
        measurement_key,
        channel,
        variant
    )
}

pub fn parse_series_key(key: &str) -> Result<ParsedSeriesKey, LedgerError> {
    let parts: Vec<&str> = key.split(':').collect();
    if parts.len() != 4 {
        return Err(LedgerError::InvalidId(format!(
            "series_key must have 4 colon-separated parts: {key}"
        )));
    }
    let system_id = SystemId::from_text(parts[0])?;
    let channel_index = if parts[2] == "na" {
        CHANNEL_NA
    } else {
        parts[2]
            .parse::<i32>()
            .map_err(|_| LedgerError::InvalidId(format!("invalid series channel: {key}")))?
    };
    Ok(ParsedSeriesKey {
        system_id,
        measurement_key: parts[1].to_string(),
        channel_index,
        variant: parts[3].to_string(),
    })
}

fn row_to_device(row: &rusqlite::Row<'_>) -> Result<DeviceRow, rusqlite::Error> {
    let sid: Vec<u8> = row.get(0)?;
    let parent: Option<Vec<u8>> = row.get(3)?;
    Ok(DeviceRow {
        system_id: system_id_from_blob(sid, "system_id")?,
        hardware_id: row.get(1)?,
        user_label: row.get(2)?,
        parent: parent
            .map(|p| system_id_from_blob(p, "parent id"))
            .transpose()?,
        kind: DeviceKind::from_db(&row.get::<_, String>(4)?),
        state: DeviceState::from_db(&row.get::<_, String>(5)?),
        declaration_version: row.get(6)?,
        presentation_identifier: row.get(7)?,
    })
}

const DEVICE_COLS: &str = "system_id, hardware_id, user_label, parent_system_id, kind, state, declaration_version, presentation_identifier";

pub fn descriptor_revision(conn: &Connection) -> Result<u64, LedgerError> {
    let value = conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'descriptor_revision'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| LedgerError::NotFound("descriptor_revision".into()))?;
    value.parse::<u64>().map_err(|_| {
        LedgerError::InvalidId("descriptor_revision is not an unsigned integer".into())
    })
}

pub fn set_presentation_identifier(
    conn: &Connection,
    system_id: &SystemId,
    identifier: Option<&str>,
) -> Result<(), LedgerError> {
    if let Some(value) = identifier
        && (value.is_empty() || value.len() > 64 || value.chars().any(char::is_control))
    {
        return Err(LedgerError::InvalidId(
            "presentation_identifier must be 1-64 UTF-8 bytes without control characters".into(),
        ));
    }
    let changed = conn.execute(
        "UPDATE devices
         SET presentation_identifier = ?1
         WHERE system_id = ?2 AND presentation_identifier IS NOT ?1",
        params![identifier, system_id.as_bytes().to_vec()],
    )?;
    if changed == 0 && get_device(conn, system_id)?.is_none() {
        return Err(LedgerError::NotFound(format!(
            "device {}",
            system_id.to_text()
        )));
    }
    Ok(())
}

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

pub fn list_devices(
    conn: &Connection,
    include_retired: bool,
) -> Result<Vec<DeviceRow>, LedgerError> {
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

pub fn get_device(
    conn: &Connection,
    system_id: &SystemId,
) -> Result<Option<DeviceRow>, LedgerError> {
    conn.query_row(
        &format!("SELECT {DEVICE_COLS} FROM devices WHERE system_id = ?1"),
        params![system_id.as_bytes().to_vec()],
        row_to_device,
    )
    .optional()
    .map_err(LedgerError::from)
}

pub fn positional_model_id(
    conn: &Connection,
    system_id: &SystemId,
) -> Result<Option<String>, LedgerError> {
    conn.query_row(
        "SELECT model_id FROM positional_device_models WHERE system_id = ?1",
        params![system_id.as_bytes().to_vec()],
        |row| row.get(0),
    )
    .optional()
    .map_err(LedgerError::from)
}

pub fn bind_positional_model(
    conn: &Connection,
    system_id: &SystemId,
    model_id: &str,
) -> Result<(), LedgerError> {
    if !is_valid_model_id(model_id) {
        return Err(LedgerError::InvalidModelId(model_id.into()));
    }
    conn.execute(
        "INSERT INTO positional_device_models(system_id, model_id) VALUES (?1, ?2)",
        params![system_id.as_bytes().to_vec(), model_id],
    )?;
    Ok(())
}

pub fn is_valid_model_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 || !bytes[0].is_ascii_lowercase() {
        return false;
    }
    let mut after_separator = false;
    for byte in &bytes[1..] {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            after_separator = false;
        } else if matches!(byte, b'-' | b'_' | b'.') && !after_separator {
            after_separator = true;
        } else {
            return false;
        }
    }
    !after_separator
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

pub fn find_series_by_key(conn: &Connection, key: &str) -> Result<Option<i64>, LedgerError> {
    let parsed = parse_series_key(key)?;
    conn.query_row(
        "SELECT series_id FROM series
         WHERE system_id = ?1 AND measurement_key = ?2 AND channel_index = ?3 AND variant = ?4",
        params![
            parsed.system_id.as_bytes().to_vec(),
            parsed.measurement_key,
            parsed.channel_index,
            parsed.variant
        ],
        |row| row.get(0),
    )
    .optional()
    .map_err(LedgerError::from)
}

pub fn list_series(conn: &Connection) -> Result<Vec<SeriesListRow>, LedgerError> {
    let mut stmt = conn.prepare(
        "SELECT s.series_id, s.system_id, s.measurement_key, s.channel_index, s.variant,
                d.user_label
         FROM series s
         LEFT JOIN devices d ON d.system_id = s.system_id
         ORDER BY s.series_id ASC",
    )?;
    stmt.query_map([], |row| {
        let sid = system_id_from_blob(row.get(1)?, "series.system_id")?;
        let measurement_key: String = row.get(2)?;
        let channel_index: i32 = row.get(3)?;
        let variant: String = row.get(4)?;
        Ok(SeriesListRow {
            series_id: row.get(0)?,
            series_key: series_key_of(&sid, &measurement_key, channel_index, &variant),
            system_id: sid.to_text(),
            user_label: row.get(5)?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
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
            system_id: system_id_from_blob(sid, "series.system_id")?,
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
/// `quarantine_reason` が一致し、canonicalのchannel定義にも適合するseriesのみ。
pub fn release_series_quarantine_for_key_checked(
    conn: &Connection,
    measurement_key: &str,
    reason: &str,
    channel_ok: &dyn Fn(i32) -> bool,
) -> Result<(Vec<i64>, Vec<i64>), LedgerError> {
    let mut stmt = conn.prepare(
        "SELECT series_id, channel_index FROM series
         WHERE measurement_key = ?1 AND quarantined = 1 AND quarantine_reason = ?2",
    )?;
    let rows: Vec<(i64, i32)> = stmt
        .query_map(params![measurement_key, reason], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<Result<_, _>>()?;
    let mut released = Vec::new();
    let mut mismatch = Vec::new();
    for (series_id, channel_index) in rows {
        if channel_ok(channel_index) {
            released.push(series_id);
        } else {
            mismatch.push(series_id);
        }
    }
    for series_id in &released {
        conn.execute(
            "UPDATE series SET quarantined = 0, quarantine_reason = NULL
             WHERE series_id = ?1",
            params![series_id],
        )?;
    }
    for series_id in &mismatch {
        conn.execute(
            "UPDATE series SET quarantine_reason = 'undeclared_channel'
             WHERE series_id = ?1",
            params![series_id],
        )?;
    }
    Ok((released, mismatch))
}

pub fn set_calibration_review(
    conn: &Connection,
    system_id: &SystemId,
    flag: bool,
) -> Result<usize, LedgerError> {
    conn.execute(
        "UPDATE series SET calibration_review = ?1 WHERE system_id = ?2",
        params![flag as i32, system_id.as_bytes().to_vec()],
    )
    .map_err(LedgerError::from)
}

pub fn replace_hardware(
    conn: &Connection,
    system_id: &SystemId,
    new_hardware_id: &str,
) -> Result<ReplaceOutcome, LedgerError> {
    let target = get_device(conn, system_id)?
        .filter(|row| row.state != DeviceState::Retired)
        .ok_or_else(|| {
            LedgerError::NotFound(format!("non-retired device {}", system_id.to_text()))
        })?;
    if target.hardware_id == new_hardware_id {
        return Err(LedgerError::InvalidReplace(format!(
            "new hardware_id is the same as current hardware_id: {new_hardware_id}"
        )));
    }
    let old_hardware_id = target.hardware_id;
    let now = now_ms();
    let mut retired_candidates = Vec::new();

    if let Some(candidate) = find_alive_by_hardware_id(conn, new_hardware_id)?
        && candidate.system_id != *system_id
    {
        conn.execute(
            "UPDATE devices SET state = 'retired', retired_at = ?1, superseded_by = ?2
             WHERE system_id = ?3 AND state != 'retired'",
            params![
                now,
                system_id.as_bytes().to_vec(),
                candidate.system_id.as_bytes().to_vec()
            ],
        )?;
        retired_candidates.push(candidate.system_id);
    }

    conn.execute(
        "UPDATE devices SET hardware_id = ?1 WHERE system_id = ?2 AND state != 'retired'",
        params![new_hardware_id, system_id.as_bytes().to_vec()],
    )?;
    set_calibration_review(conn, system_id, true)?;
    let detail = serde_json::json!({
        "old_hw": old_hardware_id,
        "new_hw": new_hardware_id,
        "at": now,
    });
    record_event(
        conn,
        "hardware_replaced",
        Some(system_id),
        &detail.to_string(),
    )?;
    Ok(ReplaceOutcome {
        replaced: *system_id,
        old_hardware_id,
        retired_candidates,
    })
}

pub fn record_sighting(
    conn: &Connection,
    hardware_id: &str,
    source: &str,
) -> Result<(), LedgerError> {
    conn.execute(
        "INSERT INTO sightings (hardware_id, source, first_seen, last_seen, observations)
         VALUES (?1, ?2, ?3, ?3, 1)
         ON CONFLICT(hardware_id) DO UPDATE SET source = excluded.source, last_seen = ?3, observations = sightings.observations + 1",
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

/// Bound the unapproved-sightings table so untrusted senders (R2) cannot grow it
/// without limit. Applied inside the caller's transaction, in order:
///   1. TTL — drop sightings whose `last_seen` is older than `SIGHTINGS_TTL_MS`
///      (a live device re-sights on its next contact).
///   2. Cap — after each retention pass, keep only the `SIGHTINGS_CAP`
///      most-recently-seen rows (LRU by `last_seen`); the rest are evicted. This
///      bounds the table at steady state; peak growth within a retention interval
///      is bounded by R2's admission control (rate/volume limits at ingress),
///      which is the first line of defense.
///
/// Approval still deletes its own row separately; this is pure housekeeping.
///
/// Returns the number of rows deleted.
pub fn purge_sightings(conn: &Connection, now: i64) -> Result<u64, LedgerError> {
    let cutoff = now.saturating_sub(SIGHTINGS_TTL_MS);
    let staged_table_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='staged_readings')",
        [],
        |row| row.get(0),
    )?;
    let staged_guard = if staged_table_exists {
        " AND NOT EXISTS (
            SELECT 1 FROM staged_readings
            WHERE staged_readings.hardware_id=sightings.hardware_id
          )"
    } else {
        ""
    };
    let by_ttl = conn.execute(
        &format!("DELETE FROM sightings WHERE last_seen < ?1{staged_guard}"),
        params![cutoff],
    )?;
    let by_cap = conn.execute(
        &format!(
            "DELETE FROM sightings WHERE hardware_id IN (
                 SELECT hardware_id FROM sightings
                 ORDER BY last_seen DESC, hardware_id ASC
                 LIMIT -1 OFFSET ?1
             ){staged_guard}"
        ),
        params![SIGHTINGS_CAP],
    )?;
    Ok((by_ttl + by_cap) as u64)
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
            system_id: sid
                .map(|b| system_id_from_blob(b, "event.system_id"))
                .transpose()?,
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

/// Activate quarantined devices whose quarantine TTL has elapsed.
pub fn expire_quarantined_devices(
    conn: &Connection,
    ttl_ms: i64,
) -> Result<Vec<SystemId>, LedgerError> {
    let cutoff = now_ms().saturating_sub(ttl_ms);
    let mut stmt = conn.prepare(
        "SELECT system_id FROM devices
         WHERE state = 'quarantined' AND created_at < ?1
         ORDER BY created_at ASC, hardware_id ASC",
    )?;
    let expired: Vec<SystemId> = stmt
        .query_map(params![cutoff], |row| {
            let sid: Vec<u8> = row.get(0)?;
            system_id_from_blob(sid, "expired device system_id")
        })?
        .collect::<Result<_, _>>()?;
    drop(stmt);

    for sid in &expired {
        conn.execute(
            "UPDATE devices SET state = 'active'
             WHERE system_id = ?1 AND state = 'quarantined'",
            params![sid.as_bytes().to_vec()],
        )?;
        let detail = serde_json::json!({ "ttl_ms": ttl_ms });
        record_event(conn, "quarantine_expired", Some(sid), &detail.to_string())?;
    }

    Ok(expired)
}

/// retire(墓標): 行は消さない。system_id再利用は永久禁止(D5決定4)。
pub fn retire_device(conn: &Connection, system_id: &SystemId) -> Result<(), LedgerError> {
    let n = conn.execute(
        "UPDATE devices SET state = 'retired', retired_at = ?1
         WHERE system_id = ?2 AND state != 'retired'",
        params![now_ms(), system_id.as_bytes().to_vec()],
    )?;
    if n == 0 {
        return Err(LedgerError::NotFound(format!(
            "non-retired device {}",
            system_id.to_text()
        )));
    }
    record_event(conn, "device_retired", Some(system_id), "")?;
    Ok(())
}

/// D5決定3 generation counter共有: 台帳変異の世代番号。CLI変異Txの最終手順で必ず呼ぶ。
pub fn bump_generation(conn: &Connection) -> Result<i64, LedgerError> {
    conn.execute(
        "INSERT INTO ledger_meta (key, value) VALUES ('generation', '1')
         ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
        [],
    )?;
    current_generation(conn)
}

pub fn current_generation(conn: &Connection) -> Result<i64, LedgerError> {
    conn.query_row(
        "SELECT CAST(value AS INTEGER) FROM ledger_meta WHERE key = 'generation'",
        [],
        |row| row.get(0),
    )
    .optional()
    .map(|v| v.unwrap_or(0))
    .map_err(LedgerError::from)
}

/// Edge Node機体identity。初回にUUIDv7を生成し、ledger epochとは独立して永続化する。
pub fn edge_node_id(conn: &Connection) -> Result<String, LedgerError> {
    if conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'gateway_identity'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        return Err(LedgerError::UnsupportedPreReleaseSchema);
    }

    if let Some(identity) = conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'edge_node_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(identity);
    }

    let candidate = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT OR IGNORE INTO ledger_meta (key, value) VALUES ('edge_node_id', ?1)",
        params![candidate],
    )?;
    conn.query_row(
        "SELECT value FROM ledger_meta WHERE key = 'edge_node_id'",
        [],
        |row| row.get(0),
    )
    .map_err(LedgerError::from)
}

/// Return an initialized Edge Node identity without generating or changing either value.
pub fn load_edge_node_identity(conn: &Connection) -> Result<EdgeNodeIdentity, LedgerError> {
    if conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'gateway_identity'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        return Err(LedgerError::UnsupportedPreReleaseSchema);
    }
    let edge_node_id = conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'edge_node_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| LedgerError::NotFound("edge_node_id".into()))?;
    let ledger_epoch = conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'epoch'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| LedgerError::NotFound("ledger_epoch".into()))?;
    Ok(EdgeNodeIdentity {
        edge_node_id,
        ledger_epoch,
    })
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

/// 復元後の新世代化: 格納済みepochを新UUIDv7で置換し、旧値を監査イベントに記録する。
pub fn renew_epoch(conn: &Connection) -> Result<String, LedgerError> {
    let old_epoch = conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'epoch'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let new_epoch = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO ledger_meta (key, value) VALUES ('epoch', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![new_epoch],
    )?;
    let detail = serde_json::json!({ "old_epoch": old_epoch });
    record_event(conn, "epoch_renewed", None, &detail.to_string())?;
    Ok(new_epoch)
}

#[cfg(test)]
#[path = "../tests/unit/store_tests.rs"]
mod tests;
