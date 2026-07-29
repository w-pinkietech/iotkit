use std::{
    collections::BTreeMap,
    fmt,
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{BufReader, Read},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use iotkit_core_ledger::{SystemId, series_key_of};
use iotkit_core_ops::{
    Actor, ActorKind, DispatchRequest, OpContext, OpDescriptor, OpError, Tier, dispatch,
};
use iotkit_core_publish::{
    activation::publication_allocation_high_water,
    store::{OutboxRow, select_batch},
    wire::{RecordBatch, publication_id},
};
use rusqlite::{Connection, OpenFlags, Transaction, backup::Backup, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    BackupCounts, NODE_BACKUP_FORMAT_VERSION, NodeBackupManifest, RecoveryError, SnapshotMode,
    all_edge_node_migrations,
};

const ARTIFACT_KIND: &str = "iotkit-node-backup";
const REMOVE_SNAPSHOT_DEPLOYMENT_CREDENTIALS_OP: &str =
    "recovery.snapshot.remove_deployment_credentials";
type SchemaObjectKey = (String, String, String);
type SchemaObjects = BTreeMap<SchemaObjectKey, Option<String>>;

/// A newly created offline SQLite snapshot and its derived manifest.
#[derive(Clone, PartialEq, Eq)]
pub struct SnapshotArtifact {
    pub path: PathBuf,
    pub manifest: NodeBackupManifest,
}

/// Values that can be derived and revalidated from the sanitized database alone.
#[derive(Clone, PartialEq, Eq)]
pub struct SnapshotFacts {
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub accepted_cursor: i64,
    pub allocation_high_water: i64,
    pub schema_version: u32,
    pub database_length: u64,
    pub database_sha256: String,
    pub counts: BackupCounts,
}

impl fmt::Debug for SnapshotArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotArtifact")
            .field("manifest", &self.manifest)
            .finish()
    }
}

impl fmt::Debug for SnapshotFacts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotFacts")
            .field("schema_version", &self.schema_version)
            .field("counts", &self.counts)
            .finish()
    }
}

/// Creates a point-in-time online backup, sanitizes only that copy, and derives its manifest.
pub fn create_consistent_snapshot(
    source: &Path,
    staging: &Path,
    backup_id: &str,
    now_ms: i64,
) -> Result<SnapshotArtifact, RecoveryError> {
    if backup_id.is_empty() || now_ms < 0 {
        return Err(RecoveryError::InvalidSnapshot);
    }

    let mut created = false;
    let result = (|| {
        create_new_empty(staging)?;
        created = true;
        online_backup(source, staging)?;
        sanitize_snapshot(staging)?;
        let facts = validate_snapshot(staging)?;
        let manifest = NodeBackupManifest {
            artifact_kind: ARTIFACT_KIND.into(),
            format_version: NODE_BACKUP_FORMAT_VERSION,
            backup_id: backup_id.into(),
            edge_node_id: facts.edge_node_id,
            ledger_epoch: facts.ledger_epoch,
            created_at_ms: now_ms,
            accepted_cursor: facts.accepted_cursor,
            allocation_high_water: facts.allocation_high_water,
            snapshot_mode: SnapshotMode::Online,
            shutdown_seal_id: None,
            schema_version: facts.schema_version,
            database_length: facts.database_length,
            database_sha256: facts.database_sha256,
            counts: facts.counts,
        };
        Ok(SnapshotArtifact {
            path: staging.to_path_buf(),
            manifest,
        })
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(staging);
    }
    result
}

/// Validates a self-contained sanitized SQLite snapshot and derives its database-owned facts.
pub fn validate_snapshot(path: &Path) -> Result<SnapshotFacts, RecoveryError> {
    require_self_contained_file(path)?;
    let database = (|| {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| RecoveryError::InvalidSnapshot)?;
        conn.pragma_update(None, "query_only", "ON")
            .map_err(|_| RecoveryError::InvalidSnapshot)?;
        require_canonical_schema(&conn)?;
        require_integrity(&conn)?;
        derive_database_facts(&conn)
    })()?;

    let database_length = fs::metadata(path)
        .map_err(|_| RecoveryError::Storage)?
        .len();
    if database_length == 0 {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let database_sha256 = sha256_file(path)?;
    require_self_contained_file(path)?;
    Ok(SnapshotFacts {
        edge_node_id: database.edge_node_id,
        ledger_epoch: database.ledger_epoch,
        accepted_cursor: database.accepted_cursor,
        allocation_high_water: database.allocation_high_water,
        schema_version: database.schema_version,
        database_length,
        database_sha256,
        counts: database.counts,
    })
}

pub fn recovery_descriptors() -> &'static [OpDescriptor] {
    static DESCRIPTORS: OnceLock<Vec<OpDescriptor>> = OnceLock::new();
    DESCRIPTORS
        .get_or_init(|| {
            let mut descriptors = vec![remove_deployment_credentials_descriptor()];
            descriptors.extend(crate::backup::backup_descriptors());
            descriptors
        })
        .as_slice()
}

fn create_new_empty(path: &Path) -> Result<(), RecoveryError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path).map(|_| ()).map_err(RecoveryError::from)
}

fn online_backup(source: &Path, staging: &Path) -> Result<(), RecoveryError> {
    let source_conn = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RecoveryError::Storage)?;
    source_conn
        .execute_batch("BEGIN DEFERRED")
        .map_err(|_| RecoveryError::Storage)?;
    source_conn
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|_| RecoveryError::Storage)?;
    let backup_result = (|| {
        let mut destination =
            Connection::open_with_flags(staging, OpenFlags::SQLITE_OPEN_READ_WRITE)
                .map_err(|_| RecoveryError::Storage)?;
        let backup =
            Backup::new(&source_conn, &mut destination).map_err(|_| RecoveryError::Storage)?;
        backup
            .run_to_completion(16, Duration::from_millis(1), None)
            .map_err(|_| RecoveryError::Storage)?;
        drop(backup);
        drop(destination);
        Ok(())
    })();
    let rollback_result = source_conn
        .execute_batch("ROLLBACK")
        .map_err(|_| RecoveryError::Storage);
    match (backup_result, rollback_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

fn sanitize_snapshot(path: &Path) -> Result<(), RecoveryError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    require_canonical_schema(&conn)?;
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    if !journal_mode.eq_ignore_ascii_case("delete") {
        return Err(RecoveryError::InvalidSnapshot);
    }
    conn.pragma_update(None, "secure_delete", "ON")
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    dispatch(
        &conn,
        recovery_descriptors(),
        DispatchRequest {
            op: REMOVE_SNAPSHOT_DEPLOYMENT_CREDENTIALS_OP.into(),
            params: json!({"credential_token": "[REDACTED]"}),
            dry_run: false,
            actor: Actor {
                actor_id: "recovery-snapshot".into(),
                actor_kind: ActorKind::LocalCli,
                tier_ceiling: Tier::Construction,
            },
            source: None,
            step_up_verified: true,
            clock_trust: None,
        },
    )
    .map_err(|_| RecoveryError::InvalidSnapshot)?;
    conn.execute_batch("VACUUM")
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    Ok(())
}

fn remove_deployment_credentials_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: REMOVE_SNAPSHOT_DEPLOYMENT_CREDENTIALS_OP,
        tier: Tier::Construction,
        bulk_escalates: false,
        changes_state: true,
        params_schema: || json!({"required": ["credential_token"]}),
        targets: |_| Vec::new(),
        preconditions: remove_credentials_preconditions,
        dry_run: remove_credentials_dry_run,
        execute: remove_credentials_execute,
        secret_execute: None,
    }
}

fn remove_credentials_preconditions(
    _tx: &Transaction<'_>,
    context: &OpContext<'_>,
) -> Result<(), OpError> {
    if context
        .params
        .get("credential_token")
        .and_then(Value::as_str)
        != Some("[REDACTED]")
    {
        return Err(OpError::Validation(
            "snapshot credential parameter must be redacted".into(),
        ));
    }
    Ok(())
}

fn remove_credentials_dry_run(
    _tx: &Transaction<'_>,
    _context: &OpContext<'_>,
) -> Result<Value, OpError> {
    Ok(json!({"would": "remove_deployment_credentials"}))
}

fn remove_credentials_execute(
    tx: &Transaction<'_>,
    _context: &OpContext<'_>,
) -> Result<Value, OpError> {
    tx.execute(
        "UPDATE target_registry SET credential_token='' WHERE credential_token<>''",
        [],
    )?;
    Ok(json!({"sanitized": "deployment_credentials"}))
}

struct DatabaseFacts {
    edge_node_id: String,
    ledger_epoch: String,
    accepted_cursor: i64,
    allocation_high_water: i64,
    schema_version: u32,
    counts: BackupCounts,
}

fn require_self_contained_file(path: &Path) -> Result<(), RecoveryError> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        match fs::metadata(PathBuf::from(sidecar)) {
            Ok(_) => return Err(RecoveryError::InvalidSnapshot),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(RecoveryError::Storage),
        }
    }
    Ok(())
}

fn derive_database_facts(conn: &Connection) -> Result<DatabaseFacts, RecoveryError> {
    let identity = iotkit_core_ledger::load_edge_node_identity(conn)
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    if !valid_topic_identity(&identity.edge_node_id) || !valid_identity(&identity.ledger_epoch) {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let activation_rows = count(conn, "edge_node_activation")?;
    if activation_rows != 1 {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let (activation_state, edge_id, activation_epoch): (String, Option<String>, Option<String>) =
        conn.query_row(
            "SELECT state, edge_id, ledger_epoch
             FROM edge_node_activation WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    if activation_state != "active"
        || !edge_id.as_deref().is_some_and(valid_identity)
        || activation_epoch.as_deref() != Some(identity.ledger_epoch.as_str())
    {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let candidate_rows = count(conn, "edge_node_recovery_candidate")?;
    if candidate_rows != 0 {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let target_rows = count(conn, "target_registry")?;
    if target_rows != 1 {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let (credential_token, schema_version, cursor_epoch, accepted_cursor): (
        String,
        i64,
        Option<String>,
        i64,
    ) = conn
        .query_row(
            "SELECT credential_token, schema_version, cursor_epoch, cursor_pub_seq
             FROM target_registry LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    if !credential_token.is_empty()
        || schema_version != 1
        || accepted_cursor < 0
        || match cursor_epoch.as_deref() {
            Some(epoch) => epoch != identity.ledger_epoch,
            None => accepted_cursor != 0,
        }
    {
        return Err(RecoveryError::InvalidSnapshot);
    }

    let allocation_high_water =
        publication_allocation_high_water(conn).map_err(|_| RecoveryError::InvalidSnapshot)?;
    if allocation_high_water < 0 || accepted_cursor > allocation_high_water {
        return Err(RecoveryError::InvalidSnapshot);
    }
    require_publication_boundaries(
        conn,
        &identity.ledger_epoch,
        accepted_cursor,
        allocation_high_water,
    )?;
    require_materializable_records(
        conn,
        &identity.edge_node_id,
        &identity.ledger_epoch,
        accepted_cursor,
        allocation_high_water,
    )?;
    let schema_version = all_edge_node_migrations()
        .last()
        .map(|migration| migration.version)
        .ok_or(RecoveryError::InvalidSnapshot)?;
    let counts = derive_counts(conn, activation_rows)?;
    Ok(DatabaseFacts {
        edge_node_id: identity.edge_node_id,
        ledger_epoch: identity.ledger_epoch,
        accepted_cursor,
        allocation_high_water,
        schema_version,
        counts,
    })
}

fn require_publication_boundaries(
    conn: &Connection,
    ledger_epoch: &str,
    accepted_cursor: i64,
    allocation_high_water: i64,
) -> Result<(), RecoveryError> {
    let conflicting_epoch_rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM publication_log WHERE epoch<>?1",
            [ledger_epoch],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    if conflicting_epoch_rows != 0 {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let maximum: Option<i64> = conn
        .query_row("SELECT max(pub_seq) FROM publication_log", [], |row| {
            row.get(0)
        })
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    if maximum.is_some_and(|value| value > allocation_high_water) {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let (rows, minimum, maximum): (i64, Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT count(*), min(pub_seq), max(pub_seq)
             FROM publication_log
             WHERE epoch=?1 AND pub_seq>?2",
            params![ledger_epoch, accepted_cursor],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    let expected = allocation_high_water
        .checked_sub(accepted_cursor)
        .ok_or(RecoveryError::InvalidSnapshot)?;
    if rows != expected
        || if expected == 0 {
            minimum.is_some() || maximum.is_some()
        } else {
            minimum != Some(accepted_cursor + 1) || maximum != Some(allocation_high_water)
        }
    {
        return Err(RecoveryError::InvalidSnapshot);
    }
    Ok(())
}

fn require_materializable_records(
    conn: &Connection,
    edge_node_id: &str,
    ledger_epoch: &str,
    accepted_cursor: i64,
    allocation_high_water: i64,
) -> Result<(), RecoveryError> {
    let mut after = accepted_cursor;
    while after < allocation_high_water {
        let rows = select_batch(conn, ledger_epoch, after, 256)
            .map_err(|_| RecoveryError::InvalidSnapshot)?;
        if rows.is_empty() {
            return Err(RecoveryError::InvalidSnapshot);
        }
        let records = rows
            .iter()
            .map(|row| materialize_record(conn, row))
            .collect::<Result<Vec<_>, _>>()?;
        let cursor_start = rows
            .first()
            .map(|row| row.pub_seq)
            .ok_or(RecoveryError::InvalidSnapshot)?;
        let cursor_end = rows
            .last()
            .map(|row| row.pub_seq)
            .ok_or(RecoveryError::InvalidSnapshot)?;
        let batch = RecordBatch {
            schema_version: 1,
            edge_node_id: edge_node_id.into(),
            ledger_epoch: ledger_epoch.into(),
            publication_id: publication_id(edge_node_id, ledger_epoch, cursor_start, cursor_end),
            cursor_start,
            cursor_end,
            records,
        };
        batch
            .validate()
            .map_err(|_| RecoveryError::InvalidSnapshot)?;
        after = cursor_end;
    }
    Ok(())
}

fn materialize_record(conn: &Connection, row: &OutboxRow) -> Result<Value, RecoveryError> {
    match row.kind.as_str() {
        "measurement" => materialize_measurement(conn, row),
        "annotation" => materialize_annotation(row),
        "commissioning_smoke" => materialize_commissioning_smoke(row),
        _ => Err(RecoveryError::InvalidSnapshot),
    }
}

fn materialize_measurement(conn: &Connection, row: &OutboxRow) -> Result<Value, RecoveryError> {
    if row.subtype.is_some() || row.annotation_json.is_some() {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let reading_seq = row.reading_seq.ok_or(RecoveryError::InvalidSnapshot)?;
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
            "SELECT series_id, event_time, event_time_source, received_at, device_time,
                    time_source, time_quality, values_json
             FROM readings WHERE seq=?1",
            [reading_seq],
            |record| {
                Ok((
                    record.get(0)?,
                    record.get(1)?,
                    record.get(2)?,
                    record.get(3)?,
                    record.get(4)?,
                    record.get(5)?,
                    record.get(6)?,
                    record.get(7)?,
                ))
            },
        )
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    let (system_id, measurement_key, channel_index, variant): (Vec<u8>, String, i32, String) = conn
        .query_row(
            "SELECT system_id, measurement_key, channel_index, variant
             FROM series WHERE series_id=?1",
            [series_id],
            |record| {
                Ok((
                    record.get(0)?,
                    record.get(1)?,
                    record.get(2)?,
                    record.get(3)?,
                ))
            },
        )
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    let system_id: [u8; 16] = system_id
        .try_into()
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    let values: Vec<f64> =
        serde_json::from_str(&values_json).map_err(|_| RecoveryError::InvalidSnapshot)?;
    Ok(json!({
        "family": "measurement",
        "schema_version": 1,
        "epoch": row.epoch,
        "pub_seq": row.pub_seq,
        "series_key": series_key_of(
            &SystemId::from_bytes(system_id),
            &measurement_key,
            channel_index,
            &variant,
        ),
        "values": values,
        "event_time": event_time,
        "event_time_source": event_time_source,
        "time_source": time_source,
        "time_quality": time_quality,
        "received_at": received_at,
        "device_time": device_time,
    }))
}

fn materialize_annotation(row: &OutboxRow) -> Result<Value, RecoveryError> {
    if row.reading_seq.is_some() || row.subtype.as_deref() != Some("epoch_start") {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let payload: Value = serde_json::from_str(
        row.annotation_json
            .as_deref()
            .ok_or(RecoveryError::InvalidSnapshot)?,
    )
    .map_err(|_| RecoveryError::InvalidSnapshot)?;
    let prior_epoch = payload
        .as_object()
        .filter(|object| object.len() == 1)
        .and_then(|object| object.get("prior_epoch"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(RecoveryError::InvalidSnapshot)?;
    Ok(json!({
        "family": "annotation",
        "schema_version": 1,
        "epoch": row.epoch,
        "pub_seq": row.pub_seq,
        "subtype": "epoch_start",
        "prior_epoch": prior_epoch,
    }))
}

fn materialize_commissioning_smoke(row: &OutboxRow) -> Result<Value, RecoveryError> {
    if row.reading_seq.is_some() || row.subtype.is_some() {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let payload: Value = serde_json::from_str(
        row.annotation_json
            .as_deref()
            .ok_or(RecoveryError::InvalidSnapshot)?,
    )
    .map_err(|_| RecoveryError::InvalidSnapshot)?;
    let test_id = payload
        .as_object()
        .filter(|object| object.len() == 1)
        .and_then(|object| object.get("test_id"))
        .and_then(Value::as_str)
        .ok_or(RecoveryError::InvalidSnapshot)?;
    iotkit_core_publish::store::validate_commissioning_smoke_test_id(test_id)
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    Ok(json!({
        "family": "commissioning_smoke",
        "schema_version": 1,
        "epoch": row.epoch,
        "pub_seq": row.pub_seq,
        "test_id": test_id,
    }))
}

fn derive_counts(conn: &Connection, activation_rows: u64) -> Result<BackupCounts, RecoveryError> {
    Ok(BackupCounts {
        devices: count(conn, "devices")?,
        series: count(conn, "series")?,
        readings: count(conn, "readings")?,
        publication_rows: count(conn, "publication_log")?,
        ingest_dedup_rows: count(conn, "ingest_dedup")?,
        staged_readings: count(conn, "staged_readings")?,
        quarantine_rows: count_where(conn, "readings", "quarantined<>0")?,
        device_principals: count(conn, "device_ingest_principals")?,
        device_credentials: count(conn, "device_credentials")?,
        activation_rows,
        ledger_events: count(conn, "ledger_events")?,
        audit_events: count_where(conn, "ledger_events", "kind='r14_op'")?,
    })
}

fn count(conn: &Connection, table: &str) -> Result<u64, RecoveryError> {
    count_where(conn, table, "1")
}

fn count_where(conn: &Connection, table: &str, predicate: &str) -> Result<u64, RecoveryError> {
    let value: i64 = conn
        .query_row(
            &format!("SELECT count(*) FROM {table} WHERE {predicate}"),
            [],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    u64::try_from(value).map_err(|_| RecoveryError::InvalidSnapshot)
}

fn require_integrity(conn: &Connection) -> Result<(), RecoveryError> {
    let quick_check = conn
        .prepare("PRAGMA quick_check")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    if quick_check.as_slice() != ["ok"] {
        return Err(RecoveryError::InvalidSnapshot);
    }
    let mut statement = conn
        .prepare("PRAGMA foreign_key_check")
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    let mut rows = statement
        .query([])
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    if rows
        .next()
        .map_err(|_| RecoveryError::InvalidSnapshot)?
        .is_some()
    {
        return Err(RecoveryError::InvalidSnapshot);
    }
    Ok(())
}

fn require_canonical_schema(conn: &Connection) -> Result<(), RecoveryError> {
    let canonical = Connection::open_in_memory().map_err(|_| RecoveryError::InvalidSnapshot)?;
    iotkit_core_storage::run_migrations(&canonical, &all_edge_node_migrations())
        .map_err(|_| RecoveryError::InvalidSnapshot)?;
    if migration_rows(conn)? != migration_rows(&canonical)?
        || schema_objects(conn)? != schema_objects(&canonical)?
    {
        return Err(RecoveryError::InvalidSnapshot);
    }
    Ok(())
}

fn migration_rows(conn: &Connection) -> Result<Vec<(u32, String)>, RecoveryError> {
    conn.prepare("SELECT version, label FROM _schema_version ORDER BY version")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(|_| RecoveryError::InvalidSnapshot)
}

fn schema_objects(conn: &Connection) -> Result<SchemaObjects, RecoveryError> {
    conn.prepare(
        "SELECT type, name, tbl_name, sql
         FROM sqlite_schema
         WHERE name NOT LIKE 'sqlite_%'
         ORDER BY type, name, tbl_name",
    )
    .and_then(|mut statement| {
        statement
            .query_map([], |row| {
                Ok((
                    (
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ),
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect()
    })
    .map_err(|_| RecoveryError::InvalidSnapshot)
}

fn sha256_file(path: &Path) -> Result<String, RecoveryError> {
    let file = File::open(path).map_err(|_| RecoveryError::Storage)?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| RecoveryError::Storage)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let mut encoded = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut encoded, "{byte:02x}").map_err(|_| RecoveryError::Storage)?;
    }
    Ok(encoded)
}

fn valid_topic_identity(value: &str) -> bool {
    valid_identity(value) && !value.contains(['/', '+', '#'])
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !value.contains(':')
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
#[path = "../tests/unit/snapshot_tests.rs"]
mod tests;
