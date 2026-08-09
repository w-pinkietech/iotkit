use std::{collections::BTreeMap, path::Path};

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use sqlx::{
    Row, ValueRef,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow},
};

use super::{OperationBackend, Storage, StorageError, StorageProfile, acquire_sqlite_guard};

const TABLES: &[&str] = &[
    "edge_meta",
    "edge_descriptor_state",
    "descriptor_devices",
    "descriptor_signals",
    "inventory_devices",
    "inventory_signals",
    "device_profiles",
    "signal_profiles",
    "edge_node_activations",
    "activation_command_outbox",
    "edge_node_recovery_cases",
    "recovery_command_outbox",
    "raw_records",
    "accepted_cursors",
    "edge_node_status",
    "edge_backup_events",
    "edge_backup_cursors",
    "edge_restore_events",
    "edge_restore_cursor_checks",
    "edge_storage_samples",
    "edge_accounts",
    "edge_sessions",
    "audit_events",
    "semantic_signals",
    "semantic_rules",
    "semantic_rule_revisions",
    "semantic_rule_starts",
    "semantic_rule_ends",
    "semantic_calibration_revisions",
    "semantic_calibration_starts",
    "semantic_rule_runtime",
    "semantic_projection_queue",
    "semantic_projection_receipts",
    "semantic_observations",
    "semantic_projection_failures",
    "semantic_counter_resets",
    "semantic_counter_reset_boundaries",
    "export_profiles",
    "output_identities",
    "output_bindings",
    "output_binding_starts",
    "output_binding_ends",
    "output_routes",
    "output_outbox",
    "output_route_attempts",
];
const RAW_RECORDS_PREVIEW_INDEX: &str = "ix_raw_records_preview_signal_received";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationCursor {
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub accepted_through: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageMigrationReport {
    pub source_profile: &'static str,
    pub target_profile: &'static str,
    pub edge_id: String,
    pub schema_version: i64,
    pub table_counts: BTreeMap<String, i64>,
    pub cursors: Vec<MigrationCursor>,
    pub content_digest: String,
    pub completed: bool,
}

struct ColumnKind {
    name: String,
    udt: String,
}

pub async fn migrate_sqlite_to_postgres(
    source_path: &Path,
    postgres_dsn: &str,
) -> Result<StorageMigrationReport, StorageError> {
    if !source_path.is_file() {
        return Err(StorageError::ProfileMigration(
            "--from-sqlite must name an existing Edge database".into(),
        ));
    }
    let _source_guard = acquire_sqlite_guard(source_path)?;
    let source = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(source_path)
                .read_only(true),
        )
        .await?;
    validate_source_schema(&source).await?;

    let schema_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version),0) FROM _sqlx_migrations WHERE success=TRUE",
    )
    .fetch_one(&source)
    .await?;
    if !migratable_source_schema_version(schema_version) {
        return Err(StorageError::ProfileMigration(format!(
            "SQLite migration source schema is {schema_version}, want 12; start current IoTKit Edge \
             against SQLite to complete its schema upgrade before offline migration"
        )));
    }
    validate_source_preview_index(&source).await?;
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
        .fetch_one(&source)
        .await
        .map_err(|_| {
            StorageError::ProfileMigration(
                "SQLite migration source has no Rust Edge identity".into(),
            )
        })?;

    let target = Storage::connect(StorageProfile::Postgres {
        dsn: postgres_dsn.into(),
    })
    .await?;
    let OperationBackend::Postgres { pool, .. } = target.operation_backend() else {
        unreachable!("constructed PostgreSQL storage")
    };
    let mut tx = pool.begin().await?;
    validate_target_preview_index(&mut tx).await?;
    for table in TABLES {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM \"{table}\""))
            .fetch_one(&mut *tx)
            .await?;
        if count != 0 {
            return Err(StorageError::ProfileMigration(
                "PostgreSQL migration target is not empty".into(),
            ));
        }
    }

    let mut table_counts = BTreeMap::new();
    let mut source_hashes = Vec::new();
    let mut source_table_digests = BTreeMap::new();
    for table in TABLES {
        let columns = postgres_columns(&mut tx, table).await?;
        validate_source_columns(&source, table, &columns).await?;
        let mut table_hashes = Vec::new();
        let count = copy_table(&source, &mut tx, table, &columns, &mut table_hashes).await?;
        source_hashes.extend(table_hashes.iter().copied());
        source_table_digests.insert(*table, digest_hashes(table_hashes));
        table_counts.insert((*table).into(), count);
    }
    reset_sequences(&mut tx).await?;

    let mut target_hashes = Vec::new();
    let mut target_table_digests = BTreeMap::new();
    for table in TABLES {
        let expected = table_counts[*table];
        let actual: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM \"{table}\""))
            .fetch_one(&mut *tx)
            .await?;
        if actual != expected {
            return Err(StorageError::ProfileMigration(format!(
                "migration row count mismatch for {table}"
            )));
        }
        let mut table_hashes = Vec::new();
        hash_target_table(&mut tx, table, &mut table_hashes).await?;
        target_table_digests.insert(*table, digest_hashes(table_hashes.clone()));
        target_hashes.extend(table_hashes);
    }
    verify_table_digests(&source_table_digests, &target_table_digests)?;
    let source_digest = digest_hashes(source_hashes);
    let target_digest = digest_hashes(target_hashes);
    if source_digest != target_digest {
        return Err(StorageError::ProfileMigration(
            "migration content digest mismatch".into(),
        ));
    }
    let cursors = sqlx::query(
        "SELECT edge_node_id,ledger_epoch,accepted_through FROM accepted_cursors \
         ORDER BY edge_node_id,ledger_epoch",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|row| {
        Ok(MigrationCursor {
            edge_node_id: row.try_get("edge_node_id")?,
            ledger_epoch: row.try_get("ledger_epoch")?,
            accepted_through: row.try_get("accepted_through")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let target_edge_id: String =
        sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
            .fetch_one(&mut *tx)
            .await?;
    if target_edge_id != edge_id {
        return Err(StorageError::ProfileMigration(
            "migration Edge identity mismatch".into(),
        ));
    }
    tx.commit().await?;
    Ok(StorageMigrationReport {
        source_profile: "embedded",
        target_profile: "postgres",
        edge_id,
        schema_version,
        table_counts,
        cursors,
        content_digest: source_digest,
        completed: true,
    })
}

fn migratable_source_schema_version(schema_version: i64) -> bool {
    schema_version == 12
}

async fn validate_source_schema(pool: &sqlx::SqlitePool) -> Result<(), StorageError> {
    let migration_table: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await?;
    let go_table: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='semantic_mappings'",
    )
    .fetch_one(pool)
    .await?;
    if migration_table != 1 || go_table != 0 {
        return Err(StorageError::UnsupportedLegacySchema);
    }
    let actual = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master WHERE type='table' \
         AND name NOT LIKE 'sqlite_%' AND name<>'_sqlx_migrations' ORDER BY name",
    )
    .fetch_all(pool)
    .await?;
    let mut expected = TABLES.iter().map(ToString::to_string).collect::<Vec<_>>();
    expected.sort();
    if actual != expected {
        let missing = expected
            .iter()
            .filter(|table| !actual.contains(table))
            .cloned()
            .collect::<Vec<_>>();
        let unexpected = actual
            .iter()
            .filter(|table| !expected.contains(table))
            .cloned()
            .collect::<Vec<_>>();
        return Err(StorageError::ProfileMigration(format!(
            "SQLite migration source is not the exact current Rust schema \
                 (missing: {}; unexpected: {}); start current IoTKit Edge to complete its schema upgrade before offline migration",
            missing.join(","),
            unexpected.join(",")
        )));
    }
    Ok(())
}

async fn validate_source_preview_index(pool: &sqlx::SqlitePool) -> Result<(), StorageError> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND tbl_name='raw_records' \
         AND name=?",
    )
    .bind(RAW_RECORDS_PREVIEW_INDEX)
    .fetch_one(pool)
    .await?;
    if exists != 1 {
        return Err(StorageError::ProfileMigration(
            "SQLite migration source is missing the schema-v11 raw preview index; start current \
             IoTKit Edge to complete its schema upgrade before offline migration"
                .into(),
        ));
    }
    Ok(())
}

async fn validate_target_preview_index(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), StorageError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_indexes WHERE schemaname=current_schema() \
         AND tablename='raw_records' AND indexname=$1)",
    )
    .bind(RAW_RECORDS_PREVIEW_INDEX)
    .fetch_one(&mut **tx)
    .await?;
    if !exists {
        return Err(StorageError::ProfileMigration(
            "PostgreSQL migration target is missing the schema-v11 raw preview index".into(),
        ));
    }
    Ok(())
}

async fn postgres_columns(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &str,
) -> Result<Vec<ColumnKind>, StorageError> {
    sqlx::query(
        "SELECT column_name,udt_name FROM information_schema.columns \
         WHERE table_schema=current_schema() AND table_name=$1 ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|row| {
        Ok(ColumnKind {
            name: row.try_get("column_name")?,
            udt: row.try_get("udt_name")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()
    .map_err(Into::into)
}

async fn validate_source_columns(
    pool: &sqlx::SqlitePool,
    table: &str,
    columns: &[ColumnKind],
) -> Result<(), StorageError> {
    let mut source = sqlx::query(&format!("PRAGMA table_info(\"{table}\")"))
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row.try_get::<String, _>("name"))
        .collect::<Result<Vec<_>, _>>()?;
    let mut target = columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    source.sort();
    target.sort();
    if source != target {
        return Err(StorageError::ProfileMigration(format!(
            "SQLite and PostgreSQL schema differ for {table}"
        )));
    }
    Ok(())
}

async fn copy_table(
    source: &sqlx::SqlitePool,
    target: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &str,
    columns: &[ColumnKind],
    hashes: &mut Vec<[u8; 32]>,
) -> Result<i64, StorageError> {
    let mut offset = 0_i64;
    loop {
        let rows = sqlx::query(&format!("SELECT * FROM \"{table}\" LIMIT 500 OFFSET ?"))
            .bind(offset)
            .fetch_all(source)
            .await?;
        if rows.is_empty() {
            break;
        }
        let values = rows
            .iter()
            .map(|row| sqlite_json_row(row, columns))
            .collect::<Result<Vec<_>, _>>()?;
        hashes.extend(values.iter().map(row_hash));
        let identity_override = matches!(table, "audit_events" | "semantic_observations")
            .then_some(" OVERRIDING SYSTEM VALUE")
            .unwrap_or("");
        sqlx::query(&format!(
            "INSERT INTO \"{table}\"{identity_override} SELECT * FROM \
             jsonb_populate_recordset(NULL::\"{table}\", $1)"
        ))
        .bind(Value::Array(values))
        .execute(&mut **target)
        .await?;
        offset += i64::try_from(rows.len())
            .map_err(|_| StorageError::ProfileMigration("row count overflow".into()))?;
    }
    Ok(offset)
}

fn sqlite_json_row(row: &SqliteRow, columns: &[ColumnKind]) -> Result<Value, StorageError> {
    let mut value = Map::new();
    for column in columns {
        let name = column.name.as_str();
        let raw = row.try_get_raw(name)?;
        let field = if raw.is_null() {
            Value::Null
        } else {
            match column.udt.as_str() {
                "int2" | "int4" | "int8" => Value::from(row.try_get::<i64, _>(name)?),
                "float4" | "float8" => {
                    let number = row.try_get::<f64, _>(name)?;
                    sqlite_float_json(number)?
                }
                "bool" => Value::Bool(row.try_get::<i64, _>(name)? != 0),
                "bytea" => {
                    let bytes = row.try_get::<Vec<u8>, _>(name)?;
                    Value::String(format!("\\x{}", encode_hex(&bytes)))
                }
                "jsonb" => {
                    let bytes = row.try_get::<Vec<u8>, _>(name)?;
                    serde_json::from_slice(&bytes).map_err(|error| {
                        StorageError::ProfileMigration(format!(
                            "invalid stored JSON in {}: {error}",
                            column.name
                        ))
                    })?
                }
                "text" | "varchar" => Value::String(row.try_get(name)?),
                other => {
                    return Err(StorageError::ProfileMigration(format!(
                        "unsupported PostgreSQL type {other}"
                    )));
                }
            }
        };
        value.insert(column.name.clone(), field);
    }
    Ok(Value::Object(value))
}

fn sqlite_float_json(number: f64) -> Result<Value, StorageError> {
    if !number.is_finite() {
        return Err(StorageError::ProfileMigration(
            "SQLite contains a non-finite number".into(),
        ));
    }
    if number == 0.0 {
        return Ok(Value::from(0));
    }
    let i64_upper_exclusive = -(i64::MIN as f64);
    if number.fract() == 0.0 && number >= i64::MIN as f64 && number < i64_upper_exclusive {
        return Ok(Value::from(number as i64));
    }
    Ok(Value::Number(
        serde_json::Number::from_f64(number).expect("finite float encodes"),
    ))
}

async fn hash_target_table(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &str,
    hashes: &mut Vec<[u8; 32]>,
) -> Result<(), StorageError> {
    let mut offset = 0_i64;
    loop {
        let rows = sqlx::query(&format!(
            "SELECT to_jsonb(item) AS value FROM \"{table}\" AS item LIMIT 500 OFFSET $1"
        ))
        .bind(offset)
        .fetch_all(&mut **tx)
        .await?;
        if rows.is_empty() {
            break;
        }
        for row in &rows {
            hashes.push(row_hash(&row.try_get::<Value, _>("value")?));
        }
        offset += i64::try_from(rows.len())
            .map_err(|_| StorageError::ProfileMigration("row count overflow".into()))?;
    }
    Ok(())
}

fn row_hash(value: &Value) -> [u8; 32] {
    Sha256::digest(serde_json::to_vec(value).expect("JSON value encodes")).into()
}

fn digest_hashes(mut hashes: Vec<[u8; 32]>) -> String {
    hashes.sort_unstable();
    let mut digest = Sha256::new();
    for hash in hashes {
        digest.update(hash);
    }
    encode_hex(&digest.finalize())
}

fn verify_table_digests(
    source: &BTreeMap<&str, String>,
    target: &BTreeMap<&str, String>,
) -> Result<(), StorageError> {
    for (table, source_digest) in source {
        if target.get(table) != Some(source_digest) {
            return Err(StorageError::ProfileMigration(format!(
                "migration content digest mismatch for {table}"
            )));
        }
    }
    Ok(())
}

async fn reset_sequences(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), StorageError> {
    for (table, column) in [
        ("audit_events", "audit_row_id"),
        ("semantic_observations", "observation_row_id"),
    ] {
        sqlx::query(&format!(
            "SELECT setval(pg_get_serial_sequence('{table}','{column}'),\
             COALESCE(MAX(\"{column}\"),1),COUNT(*)>0) FROM \"{table}\""
        ))
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
#[path = "../../tests/unit/storage_migrate_tests.rs"]
mod tests;
