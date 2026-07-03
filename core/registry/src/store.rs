//! 現場レジストリの書き込み層(D6決定3/4)。受理判定(R8)の唯一の参照先。
use crate::catalog::{CatalogEntry, ChannelMode, ValueType};
use iotkit_core_ledger as ledger;
use rusqlite::{params, Connection, OptionalExtension};

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
    pub site_min: Option<f64>,
    pub site_max: Option<f64>,
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
    SiteMapping,
}
impl AliasKind {
    fn as_db(&self) -> &'static str {
        match self {
            Self::Rename => "rename",
            Self::SiteMapping => "site_mapping",
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
        .map(|j| serde_json::from_str(j).unwrap_or_default())
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
        site_min: row.get(12)?,
        site_max: row.get(13)?,
    })
}

const ENTRY_COLS: &str = "measurement_key, origin, catalog_version, entry_revision, unit_ucum, \
     unit_display, value_type, semantic_class, channel_mode, channel_roles_json, \
     physical_min, physical_max, site_min, site_max";

pub fn get_entry(conn: &Connection, key: &str) -> Result<Option<EntryRow>, RegistryError> {
    conn.query_row(
        &format!("SELECT {ENTRY_COLS} FROM registry_entries WHERE measurement_key = ?1"),
        params![key],
        row_to_entry,
    )
    .optional()
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
            physical_min, physical_max, site_min, site_max, enabled_at)
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
    iotkit_ingest_contract::validate_measurement_key(alias)
        .map_err(|e| RegistryError::InvalidKey(format!("{alias}: {e}")))?;
    if get_entry(conn, alias)?.is_some() {
        return Err(RegistryError::NamespaceCollision(alias.to_string()));
    }
    if alias_exists(conn, alias)? {
        return Err(RegistryError::AliasExists(alias.to_string()));
    }
    if get_entry(conn, target_key)?.is_none() {
        return Err(RegistryError::TargetNotFound(target_key.to_string()));
    }
    conn.execute(
        "INSERT INTO registry_aliases (alias, measurement_key, alias_kind, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![alias, target_key, kind.as_db(), now_ms()],
    )?;
    // D6決定3(a): 申告キー(=alias名)のまま実体化済みのunknown_key検疫seriesへcanonical定義が
    // バインドされた → series級検疫を解除(series_keyは不変=履歴を切らない)。
    // undeclared_channel等の検疫はエイリアスでは解決しないため対象外。
    let released = ledger::release_series_quarantine_for_key(conn, alias, "unknown_key")?;
    if !released.is_empty() {
        let detail = serde_json::json!({
            "alias": alias, "canonical": target_key, "series_ids": released,
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
            assert_eq!(row.site_min, None, "現場既定はWave 0では未設定");
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
            define_alias(conn, "temp_old", "temperature_c", AliasKind::SiteMapping).unwrap();
            match find_resolution(conn, "temp_old").unwrap().unwrap() {
                Resolution::Alias {
                    canonical,
                    alias_kind,
                } => {
                    assert_eq!(canonical.measurement_key, "temperature_c");
                    assert_eq!(alias_kind, "site_mapping");
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
                    AliasKind::SiteMapping
                ),
                Err(RegistryError::NamespaceCollision(_))
            ));
            // 逆方向: 既存aliasと同名のエントリ有効化 → 拒否(D6決定3の衝突検査)
            define_alias(conn, "voltage_mv", "temperature_c", AliasKind::SiteMapping).unwrap();
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
            define_alias(conn, "temp_old", "temperature_c", AliasKind::SiteMapping).unwrap();
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
}
