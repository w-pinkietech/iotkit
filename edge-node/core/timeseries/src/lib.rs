//! iotkit-core-timeseries: readings persistence.

mod error;
pub mod query;

pub use error::TimeseriesError;

use iotkit_core_storage::Migration;

/// Timeseries migrations. Append to core/storage MIGRATIONS when assembling.
/// (Edge側でv3=ledgerを間に挟んで連結する。versionは昇順検証があるため
/// 1, 3, 4, 5, 7, 8 の順に並べて渡す必要がある。)
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 4,
        label: "readings_v3",
        sql: include_str!("../migrations/0004_readings_v3.sql"),
    },
    Migration {
        version: 7,
        label: "drop_sensor_readings",
        sql: include_str!("../migrations/0007_drop_sensor_readings.sql"),
    },
    Migration {
        version: 8,
        label: "event_time",
        sql: include_str!("../migrations/0008_event_time.sql"),
    },
    Migration {
        version: 17,
        label: "bounded_ingest_state",
        sql: include_str!("../migrations/0017_bounded_ingest_state.sql"),
    },
];

/// A new reading to be inserted into the v3 `readings` table.
/// Wave 0: `time_quality` is not settable here -- it defaults to 'unsynced' at
/// the schema level (D3 boundary: NTP state evaluation is Wave 1, the column
/// exists from day one but the value is fixed for now).
pub struct NewReading {
    pub series_id: i64,
    pub received_at_ms: i64,
    pub device_time_ms: Option<i64>,
    pub time_source: String,
    pub values: Vec<f64>,
    pub rssi: Option<i16>,
    pub battery_pct: Option<u8>,
    pub quarantined: bool,
}

/// Bound on staged_readings rows retained per hardware_id (oldest purged past this).
pub const STAGED_READINGS_CAP_PER_HW: i64 = 1000;

pub const FUTURE_TOLERANCE_MS: i64 = 300_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DedupLimits {
    global_rows: i64,
    principal_rows: i64,
    max_age_ms: i64,
}

impl DedupLimits {
    pub fn new(
        global_rows: i64,
        principal_rows: i64,
        max_age_ms: i64,
    ) -> Result<Self, TimeseriesError> {
        if global_rows <= 0
            || principal_rows <= 0
            || principal_rows > global_rows
            || max_age_ms <= 0
        {
            return Err(TimeseriesError::Limit(
                "dedup limits must be finite, positive, and ordered".into(),
            ));
        }
        Ok(Self {
            global_rows,
            principal_rows,
            max_age_ms,
        })
    }
}

impl Default for DedupLimits {
    fn default() -> Self {
        Self::new(100_000, 10_000, 72 * 60 * 60 * 1000).expect("finite defaults")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DedupHealth {
    pub rows: u64,
    pub max_principal_rows: u64,
    pub oldest_age_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagingLimits {
    global_rows: i64,
    principal_rows: i64,
    global_bytes: i64,
    principal_bytes: i64,
    max_age_ms: i64,
    reserve_rows: i64,
    reserve_bytes: i64,
}

impl StagingLimits {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        global_rows: i64,
        principal_rows: i64,
        global_bytes: i64,
        principal_bytes: i64,
        max_age_ms: i64,
        reserve_rows: i64,
        reserve_bytes: i64,
    ) -> Result<Self, TimeseriesError> {
        if global_rows <= 0
            || principal_rows <= 0
            || principal_rows > global_rows
            || global_bytes <= 0
            || principal_bytes <= 0
            || principal_bytes > global_bytes
            || max_age_ms <= 0
            || reserve_rows <= 0
            || reserve_rows >= principal_rows
            || reserve_bytes <= 0
            || reserve_bytes >= principal_bytes
        {
            return Err(TimeseriesError::Limit(
                "staging limits and maximum-envelope reserve must be finite, positive, and ordered"
                    .into(),
            ));
        }
        Ok(Self {
            global_rows,
            principal_rows,
            global_bytes,
            principal_bytes,
            max_age_ms,
            reserve_rows,
            reserve_bytes,
        })
    }
}

impl Default for StagingLimits {
    fn default() -> Self {
        Self::new(
            10_000,
            1_000,
            64 * 1024 * 1024,
            8 * 1024 * 1024,
            30 * 24 * 60 * 60 * 1000,
            256,
            64 * 1024,
        )
        .expect("finite defaults")
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StagingAdmission {
    pub expired_subjects: u64,
    pub evicted_subjects: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagedSighting<'a> {
    pub hardware_id: &'a str,
    pub payload_json: &'a str,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StagingHealth {
    pub rows: u64,
    pub bytes: u64,
    pub pinned_rows: u64,
    pub pinned_bytes: u64,
    pub principals: u64,
}

/// Attempt to claim `(stable_principal_id, envelope_id)` in `ingest_dedup`.
///
/// Credential/auth epochs deliberately do not participate. Replacement restore
/// does not carry readings/outbox and therefore does not carry these claims:
/// its fresh target resets the dedup window, so an unchanged post-restore retry
/// may be accepted again under the replacement's new ledger epoch.
/// Returns `true` if this is the first claim (proceed with ingest),
/// `false` if already claimed (duplicate -- D1 dedup key is sender-scoped).
pub fn try_claim_envelope(
    conn: &rusqlite::Connection,
    stable_principal_id: &str,
    envelope_id: &str,
) -> Result<bool, TimeseriesError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    try_claim_envelope_bounded_at(
        conn,
        stable_principal_id,
        envelope_id,
        now,
        DedupLimits::default(),
    )
}

pub fn try_claim_envelope_bounded_at(
    conn: &rusqlite::Connection,
    stable_principal_id: &str,
    envelope_id: &str,
    now_ms: i64,
    limits: DedupLimits,
) -> Result<bool, TimeseriesError> {
    let cutoff = now_ms.saturating_sub(limits.max_age_ms);
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM ingest_dedup WHERE sender_id=?1 AND envelope_id=?2 AND received_at >= ?3)",
        rusqlite::params![stable_principal_id, envelope_id, cutoff],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(false);
    }
    let refreshed = conn.execute(
        "UPDATE ingest_dedup SET received_at=?3 WHERE sender_id=?1 AND envelope_id=?2 AND received_at < ?3",
        rusqlite::params![stable_principal_id, envelope_id, now_ms],
    )?;
    if refreshed == 1 {
        return Ok(true);
    }
    loop {
        let principal_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ingest_dedup WHERE sender_id=?1",
            [stable_principal_id],
            |row| row.get(0),
        )?;
        let global_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |row| row.get(0))?;
        if principal_count < limits.principal_rows && global_count < limits.global_rows {
            break;
        }
        if principal_count >= limits.principal_rows {
            conn.execute(
                "DELETE FROM ingest_dedup WHERE rowid=(SELECT rowid FROM ingest_dedup WHERE sender_id=?1 ORDER BY received_at, rowid LIMIT 1)",
                [stable_principal_id],
            )?;
        } else {
            conn.execute(
                "DELETE FROM ingest_dedup WHERE rowid=(SELECT rowid FROM ingest_dedup ORDER BY received_at, rowid LIMIT 1)",
                [],
            )?;
        }
    }
    conn.execute(
        "INSERT INTO ingest_dedup (sender_id, envelope_id, received_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![stable_principal_id, envelope_id, now_ms],
    )?;
    Ok(true)
}

pub fn dedup_health(
    conn: &rusqlite::Connection,
    limits: DedupLimits,
) -> Result<DedupHealth, TimeseriesError> {
    let (rows, oldest): (i64, Option<i64>) = conn.query_row(
        "SELECT COUNT(*), MIN(received_at) FROM ingest_dedup",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let max_principal_rows: i64 = conn.query_row(
        "SELECT COALESCE(MAX(n),0) FROM (SELECT COUNT(*) n FROM ingest_dedup GROUP BY sender_id)",
        [],
        |row| row.get(0),
    )?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok(DedupHealth {
        rows: rows.max(0) as u64,
        max_principal_rows: max_principal_rows.max(0) as u64,
        oldest_age_ms: oldest.map_or(0, |at| {
            now.saturating_sub(at).min(limits.max_age_ms).max(0) as u64
        }),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DedupMaintenanceHealth {
    pub degraded: bool,
    pub episode_started_at: Option<i64>,
    pub last_failure_at: Option<i64>,
    pub last_success_at: Option<i64>,
}

pub fn dedup_maintenance_health(
    conn: &rusqlite::Connection,
) -> Result<DedupMaintenanceHealth, TimeseriesError> {
    Ok(conn.query_row(
        "SELECT degraded, episode_started_at, last_failure_at, last_success_at FROM ingest_dedup_maintenance WHERE id=1",
        [],
        |row| {
            Ok(DedupMaintenanceHealth {
                degraded: row.get(0)?,
                episode_started_at: row.get(1)?,
                last_failure_at: row.get(2)?,
                last_success_at: row.get(3)?,
            })
        },
    )?)
}

pub fn mark_dedup_purge_failed(
    conn: &rusqlite::Connection,
    now_ms: i64,
) -> Result<bool, TimeseriesError> {
    let was_degraded: bool = conn.query_row(
        "SELECT degraded FROM ingest_dedup_maintenance WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE ingest_dedup_maintenance SET degraded=1, episode_started_at=CASE WHEN degraded=0 THEN ?1 ELSE episode_started_at END, last_failure_at=?1 WHERE id=1",
        [now_ms],
    )?;
    Ok(!was_degraded)
}

pub fn mark_dedup_purge_recovered(
    conn: &rusqlite::Connection,
    now_ms: i64,
) -> Result<bool, TimeseriesError> {
    let was_degraded: bool = conn.query_row(
        "SELECT degraded FROM ingest_dedup_maintenance WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE ingest_dedup_maintenance SET degraded=0, episode_started_at=NULL, last_success_at=?1 WHERE id=1",
        [now_ms],
    )?;
    Ok(was_degraded)
}

/// Insert a reading into the v3 `readings` table. Returns the monotonic `seq`.
/// Unlike v2, identical (series, time, values) tuples are NOT deduplicated --
/// dedup happens once, upstream, via `try_claim_envelope` on (sender, envelope_id).
pub fn insert_reading_v3(
    conn: &rusqlite::Connection,
    r: &NewReading,
) -> Result<i64, TimeseriesError> {
    for v in &r.values {
        if !v.is_finite() {
            return Err(TimeseriesError::InvalidReading(format!(
                "non-finite value {v}"
            )));
        }
    }
    let values_json = serde_json::to_string(&r.values)
        .map_err(|e| TimeseriesError::InvalidReading(e.to_string()))?;
    let (event_time, event_time_source) =
        derive_event_time(r.received_at_ms, r.device_time_ms, &r.time_source);
    conn.execute(
        "INSERT INTO readings (series_id, received_at, device_time, time_source, event_time, event_time_source, values_json, rssi, battery_pct, quarantined)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            r.series_id, r.received_at_ms, r.device_time_ms, r.time_source,
            event_time, event_time_source, values_json, r.rssi, r.battery_pct,
            r.quarantined as i32
        ],
    ).map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    Ok(conn.last_insert_rowid())
}

/// D7決定3: event_time導出。device_time採用はtime_sourceがデバイス由来
/// (device_ntp/device_rtc)またはedge_node_adjusted(age_ms復元)のときのみ。
/// time_source=edge_node(デバイス時刻なしの申告)はdevice_time_msがあっても信頼しない(矛盾入力)。
fn derive_event_time(
    received_at_ms: i64,
    device_time_ms: Option<i64>,
    time_source: &str,
) -> (i64, &'static str) {
    let label = match time_source {
        "device_ntp" | "device_rtc" => "device",
        "edge_node_adjusted" => "edge_node_adjusted",
        _ => return (received_at_ms, "received_at"),
    };
    match device_time_ms {
        Some(dt) if dt <= received_at_ms + FUTURE_TOLERANCE_MS => (dt, label),
        _ => (received_at_ms, "received_at"),
    }
}

/// Append a row to `staged_readings` (D5 path A: witnessed-but-not-yet-approved
/// device data). Bounded per hardware_id -- oldest rows beyond
/// `STAGED_READINGS_CAP_PER_HW` are purged after each insert.
pub fn insert_staged_reading(
    conn: &rusqlite::Connection,
    hardware_id: &str,
    received_at_ms: i64,
    payload_json: &str,
) -> Result<(), TimeseriesError> {
    conn.execute(
        "INSERT INTO staged_readings (hardware_id, received_at, payload_json) VALUES (?1, ?2, ?3)",
        rusqlite::params![hardware_id, received_at_ms, payload_json],
    )
    .map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    conn.execute(
        "DELETE FROM staged_readings WHERE hardware_id = ?1 AND id NOT IN (
            SELECT id FROM staged_readings WHERE hardware_id = ?1 ORDER BY id DESC LIMIT ?2)",
        rusqlite::params![hardware_id, STAGED_READINGS_CAP_PER_HW],
    )
    .map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    Ok(())
}

pub fn stage_sighting_at(
    conn: &rusqlite::Connection,
    principal_id: &str,
    hardware_id: &str,
    received_at_ms: i64,
    payload_json: &str,
    limits: StagingLimits,
) -> Result<StagingAdmission, TimeseriesError> {
    stage_sightings_at(
        conn,
        principal_id,
        received_at_ms,
        &[StagedSighting {
            hardware_id,
            payload_json,
        }],
        limits,
    )
}

pub fn stage_sightings_at(
    conn: &rusqlite::Connection,
    principal_id: &str,
    received_at_ms: i64,
    sightings: &[StagedSighting<'_>],
    limits: StagingLimits,
) -> Result<StagingAdmission, TimeseriesError> {
    let staged_rows = i64::try_from(sightings.len())
        .map_err(|_| TimeseriesError::Limit("staging row count is not representable".into()))?;
    let payload_bytes = sightings
        .iter()
        .map(|sighting| {
            i64::try_from(sighting.payload_json.len()).map_err(|_| {
                TimeseriesError::Limit("staging payload size is not representable".into())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let staged_bytes = payload_bytes.iter().try_fold(0_i64, |total, bytes| {
        total
            .checked_add(*bytes)
            .ok_or_else(|| TimeseriesError::Limit("staging envelope byte cost overflowed".into()))
    })?;
    if staged_rows > limits.reserve_rows
        || staged_bytes > limits.reserve_bytes
        || staged_rows > limits.principal_rows
        || staged_bytes > limits.principal_bytes
    {
        return Err(TimeseriesError::Limit(
            "staging envelope exceeds maximum-envelope reserve".into(),
        ));
    }
    if sightings.is_empty() {
        return Ok(StagingAdmission::default());
    }

    let mut protected_subjects = std::collections::HashSet::with_capacity(sightings.len());
    let mut subject_pins = Vec::with_capacity(sightings.len());
    let mut inherited_pinned_rows = 0_i64;
    let mut inherited_pinned_bytes = 0_i64;
    for (sighting, bytes) in sightings.iter().zip(&payload_bytes) {
        protected_subjects.insert((principal_id.to_string(), sighting.hardware_id.to_string()));
        let subject_pinned: bool = conn.query_row(
            "SELECT COALESCE(MAX(pinned),0) FROM staged_readings WHERE principal_id=?1 AND hardware_id=?2",
            rusqlite::params![principal_id, sighting.hardware_id],
            |row| row.get(0),
        )?;
        subject_pins.push(subject_pinned);
        if subject_pinned {
            inherited_pinned_rows = inherited_pinned_rows.checked_add(1).ok_or_else(|| {
                TimeseriesError::Limit("staging inherited pin row cost overflowed".into())
            })?;
            inherited_pinned_bytes =
                inherited_pinned_bytes.checked_add(*bytes).ok_or_else(|| {
                    TimeseriesError::Limit("staging inherited pin byte cost overflowed".into())
                })?;
        }
    }
    validate_inherited_pin_capacity(
        conn,
        principal_id,
        inherited_pinned_rows,
        inherited_pinned_bytes,
        limits,
    )?;

    let cutoff = received_at_ms.saturating_sub(limits.max_age_ms);
    let expired = conn
        .prepare(
            "SELECT principal_id, hardware_id FROM staged_readings
             GROUP BY principal_id, hardware_id
             HAVING MAX(pinned)=0 AND MIN(received_at) < ?1",
        )?
        .query_map([cutoff], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    conn.execute(
        "DELETE FROM staged_readings
         WHERE received_at < ?1
           AND NOT EXISTS (
               SELECT 1 FROM staged_readings pinned_subject
               WHERE pinned_subject.principal_id=staged_readings.principal_id
                 AND pinned_subject.hardware_id=staged_readings.hardware_id
                 AND pinned_subject.pinned=1
           )",
        [cutoff],
    )?;
    let mut outcome = StagingAdmission {
        expired_subjects: expired.len() as u64,
        evicted_subjects: 0,
    };
    let mut removed_subjects = expired.clone();
    loop {
        let global = staging_health(conn, limits)?;
        let (principal_rows, principal_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(payload_bytes),0) FROM staged_readings WHERE principal_id=?1",
            [principal_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let global_rows = i64::try_from(global.rows)
            .map_err(|_| TimeseriesError::Limit("staging global row count overflowed".into()))?;
        let global_bytes = i64::try_from(global.bytes)
            .map_err(|_| TimeseriesError::Limit("staging global byte count overflowed".into()))?;
        let principal_over = principal_rows.saturating_add(staged_rows) > limits.principal_rows
            || principal_bytes.saturating_add(staged_bytes) > limits.principal_bytes;
        let global_over = global_rows.saturating_add(staged_rows) > limits.global_rows
            || global_bytes.saturating_add(staged_bytes) > limits.global_bytes;
        if !principal_over && !global_over {
            break;
        }
        let victim = oldest_evictable_staging_subject(
            conn,
            principal_over.then_some(principal_id),
            &protected_subjects,
        )?;
        let Some((victim_principal, victim_hardware)) = victim else {
            return Err(TimeseriesError::Limit(
                "staging capacity is protected by pin reserve".into(),
            ));
        };
        conn.execute(
            "DELETE FROM staged_readings WHERE principal_id=?1 AND hardware_id=?2",
            rusqlite::params![victim_principal, victim_hardware],
        )?;
        removed_subjects.push((victim_principal, victim_hardware));
        outcome.evicted_subjects = outcome.evicted_subjects.saturating_add(1);
    }
    for ((sighting, bytes), subject_pinned) in sightings.iter().zip(payload_bytes).zip(subject_pins)
    {
        conn.execute(
            "INSERT INTO staged_readings (hardware_id, received_at, payload_json, principal_id, payload_bytes, pinned) VALUES (?1,?2,?3,?4,?5,?6)",
            rusqlite::params![sighting.hardware_id, received_at_ms, sighting.payload_json, principal_id, bytes, subject_pinned],
        )?;
    }
    for (removed_principal, removed_hardware) in &removed_subjects {
        delete_sighting_if_unstaged(conn, removed_principal, removed_hardware)?;
    }
    Ok(outcome)
}

fn validate_inherited_pin_capacity(
    conn: &rusqlite::Connection,
    principal_id: &str,
    staged_rows: i64,
    staged_bytes: i64,
    limits: StagingLimits,
) -> Result<(), TimeseriesError> {
    if staged_rows == 0 {
        return Ok(());
    }
    let health = staging_health(conn, limits)?;
    let (principal_pinned_rows, principal_pinned_bytes): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(payload_bytes),0) FROM staged_readings WHERE principal_id=?1 AND pinned=1",
        [principal_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if (health.pinned_rows as i64).saturating_add(staged_rows)
        > limits.global_rows - limits.reserve_rows
        || principal_pinned_rows.saturating_add(staged_rows)
            > limits.principal_rows - limits.reserve_rows
        || (health.pinned_bytes as i64).saturating_add(staged_bytes)
            > limits.global_bytes - limits.reserve_bytes
        || principal_pinned_bytes.saturating_add(staged_bytes)
            > limits.principal_bytes - limits.reserve_bytes
    {
        return Err(TimeseriesError::Limit(
            "pinned subject growth would consume maximum-envelope evictable reserve".into(),
        ));
    }
    Ok(())
}

fn oldest_evictable_staging_subject(
    conn: &rusqlite::Connection,
    principal_id: Option<&str>,
    protected_subjects: &std::collections::HashSet<(String, String)>,
) -> Result<Option<(String, String)>, TimeseriesError> {
    let sql = if principal_id.is_some() {
        "SELECT principal_id, hardware_id FROM staged_readings WHERE principal_id=?1 GROUP BY principal_id, hardware_id HAVING MAX(pinned)=0 ORDER BY MIN(received_at), MIN(id)"
    } else {
        "SELECT principal_id, hardware_id FROM staged_readings GROUP BY principal_id, hardware_id HAVING MAX(pinned)=0 ORDER BY MIN(received_at), MIN(id)"
    };
    let mut statement = conn.prepare(sql)?;
    let candidates = if let Some(principal_id) = principal_id {
        statement
            .query_map([principal_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<(String, String)>, _>>()?
    } else {
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<(String, String)>, _>>()?
    };
    Ok(candidates
        .into_iter()
        .find(|subject| !protected_subjects.contains(subject)))
}

fn delete_sighting_if_unstaged(
    conn: &rusqlite::Connection,
    principal_id: &str,
    hardware_id: &str,
) -> Result<(), TimeseriesError> {
    if principal_id == "legacy:unknown" {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM sightings
         WHERE hardware_id=?1 AND source=?2
           AND NOT EXISTS (
               SELECT 1 FROM staged_readings remaining
               WHERE remaining.hardware_id=?1
           )",
        rusqlite::params![hardware_id, principal_id],
    )?;
    Ok(())
}

pub fn set_sighting_pin(
    conn: &rusqlite::Connection,
    principal_id: &str,
    hardware_id: &str,
    pinned: bool,
    limits: StagingLimits,
) -> Result<(), TimeseriesError> {
    validate_sighting_pin(conn, principal_id, hardware_id, pinned, limits)?;
    conn.execute(
        "UPDATE staged_readings SET pinned=?3 WHERE principal_id=?1 AND hardware_id=?2",
        rusqlite::params![principal_id, hardware_id, pinned],
    )?;
    Ok(())
}

pub fn validate_sighting_pin(
    conn: &rusqlite::Connection,
    principal_id: &str,
    hardware_id: &str,
    pinned: bool,
    limits: StagingLimits,
) -> Result<(), TimeseriesError> {
    let (rows, bytes, currently_pinned): (i64, i64, bool) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(payload_bytes),0), COALESCE(MAX(pinned),0) FROM staged_readings WHERE principal_id=?1 AND hardware_id=?2",
        rusqlite::params![principal_id, hardware_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if rows == 0 {
        return Err(TimeseriesError::Limit(
            "staging subject does not exist".into(),
        ));
    }
    if pinned && !currently_pinned {
        let health = staging_health(conn, limits)?;
        let (principal_pinned_rows, principal_pinned_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(payload_bytes),0) FROM staged_readings WHERE principal_id=?1 AND pinned=1",
            [principal_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if (health.pinned_rows as i64).saturating_add(rows)
            > limits.global_rows - limits.reserve_rows
            || principal_pinned_rows.saturating_add(rows)
                > limits.principal_rows - limits.reserve_rows
            || (health.pinned_bytes as i64).saturating_add(bytes)
                > limits.global_bytes - limits.reserve_bytes
            || principal_pinned_bytes.saturating_add(bytes)
                > limits.principal_bytes - limits.reserve_bytes
        {
            return Err(TimeseriesError::Limit(
                "pin would consume maximum-envelope evictable reserve".into(),
            ));
        }
    }
    Ok(())
}

pub fn staging_subject_exists(
    conn: &rusqlite::Connection,
    principal_id: &str,
    hardware_id: &str,
) -> Result<bool, TimeseriesError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM staged_readings WHERE principal_id=?1 AND hardware_id=?2)",
        rusqlite::params![principal_id, hardware_id],
        |row| row.get(0),
    )?)
}

pub fn staging_health(
    conn: &rusqlite::Connection,
    _limits: StagingLimits,
) -> Result<StagingHealth, TimeseriesError> {
    let (rows, bytes, pinned_rows, pinned_bytes, principals): (i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(payload_bytes),0), COALESCE(SUM(pinned),0), COALESCE(SUM(CASE WHEN pinned=1 THEN payload_bytes ELSE 0 END),0), COUNT(DISTINCT principal_id) FROM staged_readings",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
    Ok(StagingHealth {
        rows: rows.max(0) as u64,
        bytes: bytes.max(0) as u64,
        pinned_rows: pinned_rows.max(0) as u64,
        pinned_bytes: pinned_bytes.max(0) as u64,
        principals: principals.max(0) as u64,
    })
}

/// Delete `ingest_dedup` rows older than `cutoff_ms` (TTL 72h enforcement).
/// Returns the number of rows deleted.
pub fn purge_dedup_before(
    conn: &rusqlite::Connection,
    cutoff_ms: i64,
) -> Result<u64, TimeseriesError> {
    let n = conn
        .execute(
            "DELETE FROM ingest_dedup WHERE received_at < ?1",
            rusqlite::params![cutoff_ms],
        )
        .map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    Ok(n as u64)
}

#[cfg(test)]
#[path = "../tests/unit/v3_tests.rs"]
mod v3_tests;
