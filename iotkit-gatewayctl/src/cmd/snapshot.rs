use clap::{Args, Subcommand};
use iotkit_core_ledger as ledger;
use rusqlite::{
    Connection, OptionalExtension, TransactionBehavior, params_from_iter,
    types::{Value as SqlValue, ValueRef},
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

fn publish_noreplace(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let source = CString::new(source.as_os_str().as_bytes())?;
        let destination = CString::new(destination.as_os_str().as_bytes())?;
        // SAFETY: both pointers are live NUL-terminated path buffers for the duration of the call.
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::ENOSYS | libc::EINVAL)) {
            return Err(error);
        }
    }

    // Same-directory hard-link publication is atomic and fails if destination exists. Removing
    // the private temporary name afterwards leaves the published inode in place.
    std::fs::hard_link(source, destination)?;
    std::fs::remove_file(source)?;
    Ok(())
}

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;
type SchemaRow = (String, String, String, Option<String>);

const FORMAT_VERSION: i64 = 1;
// Wave 1 encrypted containers should dispatch by outer magic plus manifest.format_version.
// `readings`, `publication_log`, and `ingest_dedup` intentionally remain outside
// replacement snapshots as one custody unit. A restored pristine target therefore
// starts a fresh dedup window; unchanged retries can be accepted again under the
// newly minted ledger epoch instead of suppressing a reading absent after restore.
const SECTIONS: &[&str] = &[
    "devices",
    "series",
    "registry_entries",
    "registry_aliases",
    "legacy_sensor_type_map",
];

#[derive(Subcommand)]
pub enum SnapshotCommand {
    Export(ExportArgs),
    Restore(RestoreArgs),
    RestoreStatus(RestoreStatusArgs),
}

#[derive(Args)]
pub struct RestoreStatusArgs {
    #[arg(long)]
    pub db: PathBuf,
}

#[derive(Args)]
pub struct ExportArgs {
    pub out_path: PathBuf,
}

#[derive(Args)]
pub struct RestoreArgs {
    pub in_path: PathBuf,
    #[arg(long)]
    pub db: PathBuf,
    #[arg(long)]
    pub create: bool,
    #[arg(long)]
    pub yes: bool,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn blob_uuid_column(table: &str, column: &str) -> bool {
    matches!(
        (table, column),
        ("devices", "system_id")
            | ("devices", "parent_system_id")
            | ("devices", "superseded_by")
            | ("series", "system_id")
    )
}

fn json_value(table: &str, column: &str, value: ValueRef<'_>) -> AppResult<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(v) => Value::from(v),
        ValueRef::Real(v) => Value::from(v),
        ValueRef::Text(v) => Value::from(String::from_utf8_lossy(v).into_owned()),
        ValueRef::Blob(v) if blob_uuid_column(table, column) => {
            if v.len() != 16 {
                return Err(format!("{table}.{column} is not a 16-byte UUID blob").into());
            }
            let mut bytes = [0_u8; 16];
            bytes.copy_from_slice(v);
            Value::from(ledger::SystemId::from_bytes(bytes).to_text())
        }
        ValueRef::Blob(_) => {
            return Err(
                format!("unsupported BLOB column in plaintext snapshot: {table}.{column}").into(),
            );
        }
    })
}

fn dump_table(conn: &Connection, table: &str) -> AppResult<Value> {
    let mut stmt = conn.prepare(&format!("SELECT * FROM {table}"))?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mut query = stmt.query([])?;
    let mut rows = Vec::new();
    while let Some(row) = query.next()? {
        let mut object = Map::new();
        for (idx, name) in names.iter().enumerate() {
            object.insert(name.clone(), json_value(table, name, row.get_ref(idx)?)?);
        }
        rows.push(Value::Object(object));
    }
    Ok(Value::Array(rows))
}

fn existing_epoch(conn: &Connection) -> AppResult<String> {
    conn.query_row(
        "SELECT value FROM ledger_meta WHERE key = 'epoch'",
        [],
        |row| row.get::<_, String>(0),
    )
    .optional()?
    .ok_or_else(|| "ledger epoch does not exist".into())
}

pub fn run_export(conn: &Connection, args: ExportArgs) -> AppResult<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if iotkit_core_ops::replacement_backup_health(&tx)?.replacement_backup_unavailable {
        return Err(format!(
            "replacement backup unavailable: legacy plaintext snapshot export is refused while device credentials exist. {}",
            iotkit_core_ops::device_credentials::REPLACEMENT_BACKUP_ACTION
        ).into());
    }
    if let Some(ready_path) = std::env::var_os("IOTKIT_TEST_EXPORT_READY_FILE") {
        std::fs::write(&ready_path, b"ready")?;
        let continue_path = std::env::var_os("IOTKIT_TEST_EXPORT_CONTINUE_FILE")
            .ok_or("IOTKIT_TEST_EXPORT_CONTINUE_FILE is required with ready file")?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !std::path::Path::new(&continue_path).exists() {
            if std::time::Instant::now() >= deadline {
                return Err("timed out waiting for export concurrency test continuation".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    let epoch = existing_epoch(&tx)?;
    let mut root = Map::new();
    root.insert(
        "manifest".to_string(),
        serde_json::json!({
            "format_version": FORMAT_VERSION,
            "created_at": now_ms(),
            "epoch": epoch,
            "sections": SECTIONS
        }),
    );
    for table in SECTIONS {
        root.insert((*table).to_string(), dump_table(&tx, table)?);
    }
    root.insert("secrets".to_string(), Value::Null);
    root.insert("calibration".to_string(), Value::Null);
    root.insert("desired_config".to_string(), Value::Null);
    let bytes = serde_json::to_vec_pretty(&Value::Object(root))?;
    if args.out_path.exists() {
        return Err(format!(
            "snapshot output already exists: {}",
            args.out_path.display()
        )
        .into());
    }
    let parent = args
        .out_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let file_name = args
        .out_path
        .file_name()
        .ok_or("snapshot output requires a file name")?
        .to_string_lossy();
    let temp_path = parent.join(format!(".{file_name}.tmp.{}", std::process::id()));
    let result = (|| -> AppResult<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        if std::env::var_os("IOTKIT_TEST_FAIL_EXPORT_BEFORE_RENAME").is_some() {
            return Err("injected snapshot export failure before rename".into());
        }
        if let Some(ready_path) = std::env::var_os("IOTKIT_TEST_EXPORT_PUBLISH_READY_FILE") {
            std::fs::write(&ready_path, b"ready")?;
            let continue_path = std::env::var_os("IOTKIT_TEST_EXPORT_PUBLISH_CONTINUE_FILE")
                .ok_or("IOTKIT_TEST_EXPORT_PUBLISH_CONTINUE_FILE is required with ready file")?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !std::path::Path::new(&continue_path).exists() {
                if std::time::Instant::now() >= deadline {
                    return Err("timed out waiting for export publication continuation".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        publish_noreplace(&temp_path, &args.out_path)?;
        std::fs::File::open(parent)?.sync_all()?;
        tx.commit()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

fn section_rows<'a>(snapshot: &'a Value, table: &str) -> AppResult<&'a Vec<Value>> {
    snapshot
        .get(table)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("snapshot section must be an array: {table}").into())
}

fn sqlite_value(table: &str, column: &str, value: &Value) -> AppResult<SqlValue> {
    Ok(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(v) => SqlValue::Integer(i64::from(*v)),
        Value::Number(v) if v.is_i64() => SqlValue::Integer(v.as_i64().unwrap()),
        Value::Number(v) if v.is_u64() => SqlValue::Integer(
            i64::try_from(v.as_u64().unwrap())
                .map_err(|_| format!("{table}.{column} integer is out of range"))?,
        ),
        Value::Number(v) => SqlValue::Real(
            v.as_f64()
                .ok_or_else(|| format!("{table}.{column} is not a finite number"))?,
        ),
        Value::String(v) if blob_uuid_column(table, column) => {
            SqlValue::Blob(ledger::SystemId::from_text(v)?.as_bytes().to_vec())
        }
        Value::String(v) => SqlValue::Text(v.clone()),
        Value::Array(_) | Value::Object(_) => {
            return Err(format!("{table}.{column} must be scalar JSON").into());
        }
    })
}

fn quote_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn table_columns(conn: &Connection, table: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "PRAGMA table_info({})",
        quote_sqlite_identifier(table)
    ))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if columns.is_empty() {
        return Err(format!("restore target table has no columns: {table}").into());
    }
    Ok(columns)
}

fn restore_table(conn: &Connection, table: &str, rows: &[Value]) -> AppResult<()> {
    let schema_columns = table_columns(conn, table)?;
    let allowed_columns: HashSet<&str> = schema_columns.iter().map(String::as_str).collect();
    let quoted_table = quote_sqlite_identifier(table);
    for row in rows {
        let object = row
            .as_object()
            .ok_or_else(|| format!("snapshot row must be an object: {table}"))?;
        for column in object.keys() {
            if !allowed_columns.contains(column.as_str()) {
                return Err(format!("unknown snapshot column: {table}.{column}").into());
            }
        }
        let columns: Vec<&str> = schema_columns
            .iter()
            .map(String::as_str)
            .filter(|column| object.contains_key(*column))
            .collect();
        let placeholders = std::iter::repeat_n("?", columns.len())
            .collect::<Vec<_>>()
            .join(", ");
        let values = columns
            .iter()
            .map(|column| sqlite_value(table, column, &object[*column]))
            .collect::<AppResult<Vec<_>>>()?;
        let quoted_columns = columns
            .iter()
            .map(|column| quote_sqlite_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("INSERT INTO {quoted_table} ({quoted_columns}) VALUES ({placeholders})");
        conn.execute(&sql, params_from_iter(values.iter()))?;
    }
    Ok(())
}

fn confirm_restore(args: &RestoreArgs) -> AppResult<()> {
    if args.yes {
        return Ok(());
    }
    eprintln!(
        "Restore snapshot {} into {}? Type 'restore' to continue:",
        args.in_path.display(),
        args.db.display()
    );
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    if line.trim() == "restore" {
        Ok(())
    } else {
        Err("restore aborted".into())
    }
}

fn require_exhaustively_pristine_target(conn: &Connection) -> AppResult<()> {
    require_canonical_schema(conn)?;
    let sequence_rows: i64 =
        conn.query_row("SELECT COUNT(*) FROM sqlite_sequence", [], |row| row.get(0))?;
    if sequence_rows != 0 {
        return Err("restore target is not empty: sqlite sequence state exists".into());
    }
    let auth_state_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM auth_state
         WHERE id = 1
           AND auth_generation = 0
           AND device_credential_generation = 0
           AND recovery_required = 0
           AND ownership_ever_established = 0
           AND clock_floor_ms = 0
           AND clock_evidence_source IS NULL
           AND clock_evidence_at_ms IS NULL
           AND manual_evidence_seq = 0",
        [],
        |row| row.get(0),
    )?;
    if auth_state_rows != 1 {
        return Err(
            "restore target is not empty: auth_state contains authority or recovery state".into(),
        );
    }
    let canonical_flow_classes: i64 = conn.query_row(
        "SELECT COUNT(*) FROM device_flow_classes
         WHERE flow_class IN ('low', 'default', 'high')
           AND steady_units = 1 AND burst_units = 1",
        [],
        |row| row.get(0),
    )?;
    let flow_class_rows: i64 =
        conn.query_row("SELECT COUNT(*) FROM device_flow_classes", [], |row| {
            row.get(0)
        })?;
    if canonical_flow_classes != 3 || flow_class_rows != 3 {
        return Err(
            "restore target is not empty: device_flow_classes contains non-pristine configuration"
                .into(),
        );
    }
    let canonical_capacity: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM device_capacity
         WHERE id = 1 AND steady_units = 1 AND burst_units = 1 AND stale_after_ms = 1)",
        [],
        |row| row.get(0),
    )?;
    let capacity_rows: i64 =
        conn.query_row("SELECT COUNT(*) FROM device_capacity", [], |row| row.get(0))?;
    if !canonical_capacity || capacity_rows != 1 {
        return Err(
            "restore target is not empty: device_capacity contains non-pristine configuration"
                .into(),
        );
    }

    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_schema
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
           AND name NOT IN ('_schema_version', 'auth_state', 'device_flow_classes', 'device_capacity')
         ORDER BY name",
    )?;
    let tables = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for table in tables {
        let count: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM {}", quote_sqlite_identifier(&table)),
            [],
            |row| row.get(0),
        )?;
        if count != 0 {
            return Err(format!("restore target is not empty: {table} table has rows").into());
        }
    }
    Ok(())
}

fn all_migrations() -> Vec<iotkit_core_storage::Migration> {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    migrations.sort_by_key(|migration| migration.version);
    migrations
}

fn canonical_schema_rows(conn: &Connection) -> AppResult<Vec<SchemaRow>> {
    let mut stmt = conn.prepare(
        "SELECT type, name, tbl_name, sql FROM sqlite_schema
         WHERE name NOT LIKE 'sqlite_%'
         ORDER BY type, name, tbl_name",
    )?;
    Ok(stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

fn migration_rows(conn: &Connection) -> AppResult<Vec<(i64, String)>> {
    let mut stmt = conn.prepare("SELECT version, label FROM _schema_version ORDER BY version")?;
    Ok(stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn require_canonical_schema(conn: &Connection) -> AppResult<()> {
    let canonical = Connection::open_in_memory()?;
    iotkit_core_storage::run_migrations(&canonical, &all_migrations())?;
    if canonical_schema_rows(conn)? != canonical_schema_rows(&canonical)?
        || migration_rows(conn)? != migration_rows(&canonical)?
    {
        return Err("restore target schema or migration set is not canonical".into());
    }
    Ok(())
}

pub fn run_restore(conn: &Connection, args: RestoreArgs) -> AppResult<()> {
    confirm_restore(&args)?;
    let snapshot_bytes = std::fs::read(&args.in_path)?;
    let snapshot_sha256 = format!("{:x}", Sha256::digest(&snapshot_bytes));
    let snapshot: Value = serde_json::from_slice(&snapshot_bytes)?;
    let manifest = snapshot
        .get("manifest")
        .and_then(Value::as_object)
        .ok_or("snapshot manifest must be an object")?;
    if manifest.get("format_version").and_then(Value::as_i64) != Some(FORMAT_VERSION) {
        return Err("unsupported snapshot format_version".into());
    }
    let sections = manifest
        .get("sections")
        .and_then(Value::as_array)
        .ok_or("snapshot manifest.sections must be an array")?;
    let expected_sections: Vec<Value> = SECTIONS.iter().map(|s| Value::from(*s)).collect();
    if *sections != expected_sections {
        return Err("snapshot sections do not match R22 Wave 0 format".into());
    }
    let new_auth_epoch = iotkit_core_ops::new_auth_epoch()?;
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    require_exhaustively_pristine_target(&tx)?;
    let old_ledger_generation = ledger::current_generation(&tx)?;
    let old_ledger_epoch: Option<String> = tx
        .query_row(
            "SELECT value FROM ledger_meta WHERE key = 'epoch'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let old_auth_generation = iotkit_core_ops::auth_generation(&tx)?;
    let old_auth_epoch = iotkit_core_ops::auth_epoch(&tx)?;
    tx.execute("PRAGMA defer_foreign_keys = ON", [])?;
    for table in SECTIONS {
        restore_table(&tx, table, section_rows(&snapshot, table)?)?;
    }
    let new_ledger_epoch = ledger::renew_epoch(&tx)?;
    iotkit_core_ops::enter_restored_local_recovery(&tx, &new_auth_epoch)?;
    let new_ledger_generation = ledger::bump_generation(&tx)?;
    let new_auth_generation = iotkit_core_ops::auth_generation(&tx)?;
    tx.execute(
        "INSERT INTO restore_receipts (
           id, snapshot_sha256, old_ledger_generation, new_ledger_generation,
           old_ledger_epoch, new_ledger_epoch, old_auth_generation, new_auth_generation,
           old_auth_epoch, new_auth_epoch, committed_at
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            snapshot_sha256,
            old_ledger_generation,
            new_ledger_generation,
            old_ledger_epoch,
            new_ledger_epoch,
            old_auth_generation,
            new_auth_generation,
            old_auth_epoch,
            new_auth_epoch,
            now_ms(),
        ],
    )?;
    if std::env::var_os("IOTKIT_TEST_FAIL_RESTORE_BEFORE_COMMIT").is_some() {
        return Err("injected restore failure before commit".into());
    }
    tx.commit()?;
    Ok(())
}

pub fn run_restore_status(conn: &Connection) -> AppResult<()> {
    let receipt = conn
        .query_row(
            "SELECT snapshot_sha256, old_ledger_generation, new_ledger_generation,
                    old_ledger_epoch, new_ledger_epoch, old_auth_generation,
                    new_auth_generation, old_auth_epoch, new_auth_epoch, committed_at
             FROM restore_receipts WHERE id = 1",
            [],
            |row| {
                Ok(serde_json::json!({
                    "snapshot_sha256": row.get::<_, String>(0)?,
                    "old_ledger_generation": row.get::<_, i64>(1)?,
                    "new_ledger_generation": row.get::<_, i64>(2)?,
                    "old_ledger_epoch": row.get::<_, Option<String>>(3)?,
                    "new_ledger_epoch": row.get::<_, String>(4)?,
                    "old_auth_generation": row.get::<_, i64>(5)?,
                    "new_auth_generation": row.get::<_, i64>(6)?,
                    "old_auth_epoch": row.get::<_, String>(7)?,
                    "new_auth_epoch": row.get::<_, String>(8)?,
                    "committed_at": row.get::<_, i64>(9)?,
                }))
            },
        )
        .optional()?;
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({ "restore_receipt": receipt }))?
    );
    Ok(())
}
