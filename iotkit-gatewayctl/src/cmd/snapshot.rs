use clap::{Args, Subcommand};
use iotkit_core_ledger as ledger;
use rusqlite::{
    params_from_iter,
    types::{Value as SqlValue, ValueRef},
    Connection, OptionalExtension, TransactionBehavior,
};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::PathBuf;

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

const FORMAT_VERSION: i64 = 1;
// Wave 1 encrypted containers should dispatch by outer magic plus manifest.format_version.
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
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Deferred)?;
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
    std::fs::write(
        args.out_path,
        serde_json::to_vec_pretty(&Value::Object(root))?,
    )?;
    tx.commit()?;
    Ok(())
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

pub fn run_restore(conn: &Connection, args: RestoreArgs) -> AppResult<()> {
    confirm_restore(&args)?;
    let snapshot: Value = serde_json::from_slice(&std::fs::read(&args.in_path)?)?;
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
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    tx.execute("PRAGMA defer_foreign_keys = ON", [])?;
    for table in SECTIONS {
        let existing_rows: i64 =
            tx.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
        if existing_rows > 0 {
            return Err(format!("restore target is not empty: {table} table has rows").into());
        }
    }
    for table in SECTIONS {
        restore_table(&tx, table, section_rows(&snapshot, table)?)?;
    }
    ledger::renew_epoch(&tx)?;
    ledger::bump_generation(&tx)?;
    tx.commit()?;
    Ok(())
}
