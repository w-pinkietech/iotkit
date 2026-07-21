//! 現場レジストリの書き込み層(D6決定3/4)。受理判定(R8)の唯一の参照先。
use crate::catalog::{CatalogEntry, ChannelMode, Range, ValueType};
use iotkit_core_ledger as ledger;
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug)]
pub enum RegistryError {
    /// 単一名前空間の衝突(D6決定2): キー⇔エイリアス間
    NamespaceCollision(String),
    /// alias定義の対象エントリが存在しない
    TargetNotFound(String),
    /// aliasが既に定義済み
    AliasExists(String),
    InvalidKey(String),
    Sqlite(rusqlite::Error),
    Ledger(ledger::LedgerError),
}
impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NamespaceCollision(k) => {
                write!(f, "name '{k}' collides across key/alias namespace")
            }
            Self::TargetNotFound(k) => write!(f, "alias target entry not found: {k}"),
            Self::AliasExists(a) => write!(f, "alias already defined: {a}"),
            Self::InvalidKey(k) => write!(f, "invalid measurement_key: {k}"),
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::Ledger(e) => write!(f, "ledger error: {e}"),
        }
    }
}
impl std::error::Error for RegistryError {}
impl From<rusqlite::Error> for RegistryError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}
impl From<ledger::LedgerError> for RegistryError {
    fn from(e: ledger::LedgerError) -> Self {
        Self::Ledger(e)
    }
}

#[derive(Debug, Clone)]
pub struct EntryRow {
    pub measurement_key: String,
    pub origin: String,
    pub catalog_version: Option<String>,
    pub entry_revision: String,
    pub unit_ucum: Option<String>,
    pub unit_display: Option<String>,
    pub value_type: ValueType,
    pub semantic_class: String,
    pub channel_mode: ChannelMode,
    pub channel_roles: Vec<String>,
    pub physical_min: Option<f64>,
    pub physical_max: Option<f64>,
    pub local_min: Option<f64>,
    pub local_max: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct AliasRow {
    pub alias: String,
    pub measurement_key: String,
    pub alias_kind: String,
}

#[derive(Debug, Clone)]
pub struct CustomEntrySpec {
    pub measurement_key: String,
    pub unit_ucum: Option<String>,
    pub unit_display: Option<String>,
    pub value_type: ValueType,
    pub semantic_class: String,
    pub channel_mode: ChannelMode,
    pub channel_roles: Vec<String>,
    pub physical_min: Option<f64>,
    pub physical_max: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum Resolution {
    Entry(EntryRow),
    Alias {
        canonical: EntryRow,
        alias_kind: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasKind {
    Rename,
    LocationMapping,
}
impl AliasKind {
    fn as_db(&self) -> &'static str {
        match self {
            Self::Rename => "rename",
            Self::LocationMapping => "location_mapping",
        }
    }
}

/// D6決定11のlegacy_sensor_type対応表(移行シム。カタログの一部ではない)。
/// 262はacceleration_mg側のみ(スペクトログラム側の分解はアダプタ写像=決定3)。
pub const LEGACY_SENSOR_MAP: &[(u16, &str)] = &[
    (257, "contact_state"),
    (258, "contact_output_state"),
    (259, "voltage_mv"),
    (260, "distance_mm"),
    (261, "temperature_c"),
    (262, "acceleration_mg"),
    (263, "differential_pressure_pa"),
    (264, "illuminance_lux"),
    (294, "contact_state"),
    (295, "contact_state"),
    (296, "contact_output_state"),
    (297, "temperature_c"),
    (298, "current_ma"),
    (299, "voltage_mv"),
];

pub fn validate_measurement_key(key: &str) -> Result<(), RegistryError> {
    iotkit_ingest_contract::validate_measurement_key(key)
        .map_err(|e| RegistryError::InvalidKey(format!("{key}: {e}")))
}

pub fn validate_custom_entry_spec(spec: &CustomEntrySpec) -> Result<(), RegistryError> {
    if !spec.measurement_key.starts_with("custom.") {
        return Err(RegistryError::InvalidKey(spec.measurement_key.clone()));
    }
    validate_measurement_key(&spec.measurement_key)?;
    if spec.channel_mode == ChannelMode::Fixed && spec.channel_roles.is_empty() {
        return Err(RegistryError::InvalidKey(format!(
            "{}: fixed channel_mode requires channel_roles",
            spec.measurement_key
        )));
    }
    if spec.channel_mode != ChannelMode::Fixed && !spec.channel_roles.is_empty() {
        return Err(RegistryError::InvalidKey(format!(
            "{}: channel_roles only allowed for fixed mode",
            spec.measurement_key
        )));
    }
    custom_physical_range(spec)?;
    Ok(())
}

fn custom_physical_range(spec: &CustomEntrySpec) -> Result<Option<Range>, RegistryError> {
    let physical_range = match (spec.physical_min, spec.physical_max) {
        (Some(min), Some(max)) if min.partial_cmp(&max) == Some(std::cmp::Ordering::Less) => {
            Some(Range { min, max })
        }
        (None, None) => None,
        (Some(_), Some(_)) => {
            return Err(RegistryError::InvalidKey(format!(
                "{}: physical_min must be less than physical_max",
                spec.measurement_key
            )));
        }
        _ => {
            return Err(RegistryError::InvalidKey(format!(
                "{}: physical_min and physical_max must be supplied together",
                spec.measurement_key
            )));
        }
    };
    if spec.value_type == ValueType::Record && physical_range.is_some() {
        return Err(RegistryError::InvalidKey(format!(
            "{}: record type cannot carry physical range",
            spec.measurement_key
        )));
    }
    Ok(physical_range)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn alias_exists(conn: &Connection, name: &str) -> Result<bool, RegistryError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM registry_aliases WHERE alias = ?1",
        params![name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> Result<EntryRow, rusqlite::Error> {
    let roles_json: Option<String> = row.get(9)?;
    let channel_roles: Vec<String> = roles_json
        .as_deref()
        .map(|j| match serde_json::from_str(j) {
            Ok(roles) => roles,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    channel_roles_json = j,
                    "invalid channel_roles_json in registry row; using empty roles"
                );
                Vec::new()
            }
        })
        .unwrap_or_default();
    Ok(EntryRow {
        measurement_key: row.get(0)?,
        origin: row.get(1)?,
        catalog_version: row.get(2)?,
        entry_revision: row.get(3)?,
        unit_ucum: row.get(4)?,
        unit_display: row.get(5)?,
        value_type: ValueType::from_db(&row.get::<_, String>(6)?),
        semantic_class: row.get(7)?,
        channel_mode: ChannelMode::from_db(&row.get::<_, String>(8)?),
        channel_roles,
        physical_min: row.get(10)?,
        physical_max: row.get(11)?,
        local_min: row.get(12)?,
        local_max: row.get(13)?,
    })
}

const ENTRY_COLS: &str = "measurement_key, origin, catalog_version, entry_revision, unit_ucum, \
     unit_display, value_type, semantic_class, channel_mode, channel_roles_json, \
     physical_min, physical_max, local_min, local_max";

pub fn get_entry(conn: &Connection, key: &str) -> Result<Option<EntryRow>, RegistryError> {
    conn.query_row(
        &format!("SELECT {ENTRY_COLS} FROM registry_entries WHERE measurement_key = ?1"),
        params![key],
        row_to_entry,
    )
    .optional()
    .map_err(RegistryError::from)
}

pub fn list_entries(conn: &Connection) -> Result<Vec<EntryRow>, RegistryError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ENTRY_COLS} FROM registry_entries ORDER BY measurement_key ASC"
    ))?;
    stmt.query_map([], row_to_entry)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(RegistryError::from)
}

pub fn list_aliases(conn: &Connection) -> Result<Vec<AliasRow>, RegistryError> {
    let mut stmt = conn.prepare(
        "SELECT alias, measurement_key, alias_kind
         FROM registry_aliases ORDER BY alias ASC",
    )?;
    stmt.query_map([], |row| {
        Ok(AliasRow {
            alias: row.get(0)?,
            measurement_key: row.get(1)?,
            alias_kind: row.get(2)?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(RegistryError::from)
}

pub fn enable_entry(
    conn: &Connection,
    entry: &CatalogEntry,
    catalog_version: &str,
    trigger: &str,
) -> Result<EntryRow, RegistryError> {
    if let Some(existing) = get_entry(conn, &entry.key)? {
        return Ok(existing); // 冪等(copy-on-enableは初回のみ)
    }
    if alias_exists(conn, &entry.key)? {
        return Err(RegistryError::NamespaceCollision(entry.key.clone()));
    }
    let revision = entry.revision();
    let roles_json = if entry.channel_roles.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&entry.channel_roles).expect("string vec serializes"))
    };
    conn.execute(
        "INSERT INTO registry_entries (measurement_key, origin, catalog_version, entry_revision,
            unit_ucum, unit_display, value_type, semantic_class, channel_mode, channel_roles_json,
            physical_min, physical_max, local_min, local_max, enabled_at)
         VALUES (?1, 'catalog', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, ?12)",
        params![
            entry.key,
            catalog_version,
            revision,
            entry.unit_ucum,
            entry.unit_display,
            entry.value_type.as_db(),
            entry.semantic_class,
            entry.channel_mode.as_db(),
            roles_json,
            entry.physical_range.map(|r| r.min),
            entry.physical_range.map(|r| r.max),
            now_ms(),
        ],
    )?;
    let detail = serde_json::json!({
        "key": entry.key, "revision": revision,
        "catalog_version": catalog_version, "trigger": trigger,
    })
    .to_string();
    ledger::record_event(conn, "registry_entry_enabled", None, &detail)?;
    get_entry(conn, &entry.key)?
        .ok_or_else(|| RegistryError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
}

pub fn define_custom_entry(
    conn: &Connection,
    spec: &CustomEntrySpec,
) -> Result<EntryRow, RegistryError> {
    validate_custom_entry_spec(spec)?;
    if get_entry(conn, &spec.measurement_key)?.is_some() {
        return Err(RegistryError::NamespaceCollision(
            spec.measurement_key.clone(),
        ));
    }
    if alias_exists(conn, &spec.measurement_key)? {
        return Err(RegistryError::NamespaceCollision(
            spec.measurement_key.clone(),
        ));
    }
    let physical_range = custom_physical_range(spec)?;

    let entry = CatalogEntry {
        key: spec.measurement_key.clone(),
        unit_ucum: spec.unit_ucum.clone(),
        unit_display: spec.unit_display.clone(),
        value_type: spec.value_type,
        semantic_class: spec.semantic_class.clone(),
        channel_mode: spec.channel_mode,
        channel_roles: spec.channel_roles.clone(),
        physical_range,
    };
    let revision = entry.revision();
    let roles_json = if entry.channel_roles.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&entry.channel_roles).expect("string vec serializes"))
    };
    conn.execute(
        "INSERT INTO registry_entries (measurement_key, origin, catalog_version, entry_revision,
            unit_ucum, unit_display, value_type, semantic_class, channel_mode, channel_roles_json,
            physical_min, physical_max, local_min, local_max, enabled_at)
         VALUES (?1, 'custom', NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, ?11)",
        params![
            entry.key,
            revision,
            entry.unit_ucum,
            entry.unit_display,
            entry.value_type.as_db(),
            entry.semantic_class,
            entry.channel_mode.as_db(),
            roles_json,
            entry.physical_range.map(|r| r.min),
            entry.physical_range.map(|r| r.max),
            now_ms(),
        ],
    )?;
    let detail = serde_json::json!({
        "key": entry.key,
        "revision": revision,
        "origin": "custom",
        "catalog_version": serde_json::Value::Null,
    })
    .to_string();
    ledger::record_event(conn, "registry_entry_enabled", None, &detail)?;
    get_entry(conn, &spec.measurement_key)?
        .ok_or_else(|| RegistryError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
}

pub fn find_resolution(
    conn: &Connection,
    declared_key: &str,
) -> Result<Option<Resolution>, RegistryError> {
    if let Some(entry) = get_entry(conn, declared_key)? {
        return Ok(Some(Resolution::Entry(entry)));
    }
    let alias: Option<(String, String)> = conn
        .query_row(
            "SELECT measurement_key, alias_kind FROM registry_aliases WHERE alias = ?1",
            params![declared_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match alias {
        Some((target, kind)) => {
            let canonical =
                get_entry(conn, &target)?.ok_or_else(|| RegistryError::TargetNotFound(target))?;
            Ok(Some(Resolution::Alias {
                canonical,
                alias_kind: kind,
            }))
        }
        None => Ok(None),
    }
}

pub fn define_alias(
    conn: &Connection,
    alias: &str,
    target_key: &str,
    kind: AliasKind,
) -> Result<(), RegistryError> {
    validate_measurement_key(alias)?;
    if get_entry(conn, alias)?.is_some() {
        return Err(RegistryError::NamespaceCollision(alias.to_string()));
    }
    if alias_exists(conn, alias)? {
        return Err(RegistryError::AliasExists(alias.to_string()));
    }
    let target = get_entry(conn, target_key)?
        .ok_or_else(|| RegistryError::TargetNotFound(target_key.to_string()))?;
    conn.execute(
        "INSERT INTO registry_aliases (alias, measurement_key, alias_kind, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![alias, target_key, kind.as_db(), now_ms()],
    )?;
    // D6決定3(a): 申告キー(=alias名)のまま実体化済みのunknown_key検疫seriesへcanonical定義が
    // バインドされた → series級検疫を解除(series_keyは不変=履歴を切らない)。
    // canonicalのchannel定義に合わないunknown_key検疫はundeclared_channelへ張り替えて維持する。
    let channel_ok = |channel_index: i32| match target.channel_mode {
        ChannelMode::Single => channel_index == ledger::CHANNEL_NA || channel_index == 0,
        ChannelMode::Fixed => {
            channel_index >= 0 && (channel_index as usize) < target.channel_roles.len()
        }
        ChannelMode::Generic => true,
    };
    let (released, mismatch) =
        ledger::release_series_quarantine_for_key_checked(conn, alias, "unknown_key", &channel_ok)?;
    if !released.is_empty() || !mismatch.is_empty() {
        let detail = serde_json::json!({
            "alias": alias, "canonical": target_key, "series_ids": released,
            "channel_mismatch_ids": mismatch,
        })
        .to_string();
        ledger::record_event(conn, "series_quarantine_released", None, &detail)?;
    }
    Ok(())
}

pub fn seed_legacy_sensor_map(conn: &Connection) -> Result<usize, RegistryError> {
    let mut inserted = 0;
    for (st, key) in LEGACY_SENSOR_MAP {
        inserted += conn.execute(
            "INSERT INTO legacy_sensor_type_map (sensor_type, measurement_key, created_at)
             VALUES (?1, ?2, ?3) ON CONFLICT(sensor_type) DO NOTHING",
            params![st, key, now_ms()],
        )?;
    }
    Ok(inserted)
}

pub fn lookup_legacy(conn: &Connection, sensor_type: u16) -> Result<Option<String>, RegistryError> {
    conn.query_row(
        "SELECT measurement_key FROM legacy_sensor_type_map WHERE sensor_type = ?1",
        params![sensor_type],
        |row| row.get(0),
    )
    .optional()
    .map_err(RegistryError::from)
}

#[cfg(test)]
mod tests {
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
}
