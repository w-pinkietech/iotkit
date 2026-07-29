use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::{RecoveryError, RecoveryStartupMode};

const CANDIDATE_COLUMNS: &[&str] = &[
    "singleton",
    "state",
    "recovery_id",
    "candidate_instance_id",
    "backup_id",
    "edge_id",
    "edge_node_id",
    "old_ledger_epoch",
    "proposed_new_epoch",
    "credential_generation",
    "handoff_schema_version",
    "installed_at_ms",
];
const ATTEMPT_COLUMNS: &[&str] = &[
    "attempt_id",
    "backup_id",
    "state",
    "reason_code",
    "artifact_name",
    "artifact_length",
    "edge_node_id",
    "ledger_epoch",
    "accepted_cursor",
    "allocation_high_water",
    "started_at_ms",
    "artifact_created_at_ms",
    "completed_at_ms",
];

/// Reads the recovery fence from an initialized database.
pub fn startup_mode(conn: &Connection) -> Result<RecoveryStartupMode, RecoveryError> {
    let candidate_table = table_exists(conn, "edge_node_recovery_candidate")?;
    let attempt_table = table_exists(conn, "edge_node_backup_attempts")?;
    let migration_23 = migration_23_applied(conn)?;
    if !candidate_table && !attempt_table && !migration_23 {
        return Ok(RecoveryStartupMode::Normal);
    }
    if !candidate_table || !attempt_table || !migration_23 {
        return Err(RecoveryError::InvalidStartupState);
    }
    if table_columns(conn, "edge_node_recovery_candidate")? != CANDIDATE_COLUMNS
        || table_columns(conn, "edge_node_backup_attempts")? != ATTEMPT_COLUMNS
        || !candidate_table_is_exact(conn)?
        || !backup_attempt_table_is_exact(conn)?
        || !candidate_trigger_is_exact(conn)?
        || !backup_attempt_trigger_is_exact(conn)?
    {
        return Err(RecoveryError::InvalidStartupState);
    }

    let rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM edge_node_recovery_candidate",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    match rows {
        0 => Ok(RecoveryStartupMode::Normal),
        1 => load_candidate(conn),
        _ => Err(RecoveryError::InvalidStartupState),
    }
}

/// Probes a database path without creating, migrating, or repairing it.
pub fn probe_startup_path(path: &Path) -> Result<RecoveryStartupMode, RecoveryError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecoveryStartupMode::Normal);
        }
        Err(_) => return Err(RecoveryError::Storage),
    };
    if metadata.len() == 0 {
        return Ok(RecoveryStartupMode::Normal);
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RecoveryError::Storage)?;
    startup_mode(&conn)
}

fn load_candidate(conn: &Connection) -> Result<RecoveryStartupMode, RecoveryError> {
    let row = conn
        .query_row(
            "SELECT state, recovery_id, candidate_instance_id, backup_id, edge_id, edge_node_id,
                old_ledger_epoch, proposed_new_epoch, credential_generation,
                handoff_schema_version, installed_at_ms
         FROM edge_node_recovery_candidate WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    let (
        state,
        recovery_id,
        candidate_instance_id,
        backup_id,
        edge_id,
        edge_node_id,
        old_ledger_epoch,
        proposed_new_epoch,
        credential_generation,
        handoff_schema_version,
        installed_at_ms,
    ) = row;
    if state != "durably_fenced_candidate"
        || !valid_identity(&recovery_id)
        || !valid_identity(&candidate_instance_id)
        || !backup_id.as_deref().is_none_or(valid_identity)
        || !valid_identity(&edge_id)
        || !valid_identity(&edge_node_id)
        || !valid_identity(&old_ledger_epoch)
        || !valid_identity(&proposed_new_epoch)
        || old_ledger_epoch == proposed_new_epoch
        || credential_generation < 0
        || handoff_schema_version != 1
        || installed_at_ms < 0
    {
        return Err(RecoveryError::InvalidStartupState);
    }
    Ok(RecoveryStartupMode::FencedCandidate {
        recovery_id,
        candidate_instance_id,
        backup_id,
        edge_id,
        old_ledger_epoch,
        proposed_new_epoch,
        credential_generation,
    })
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, RecoveryError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(RecoveryError::from)
}

fn migration_23_applied(conn: &Connection) -> Result<bool, RecoveryError> {
    if !table_exists(conn, "_schema_version")? {
        return Ok(false);
    }
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM _schema_version WHERE version = 23)",
        [],
        |row| row.get(0),
    )
    .map_err(RecoveryError::from)
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, RecoveryError> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(RecoveryError::from)?;
    statement
        .query_map([], |row| row.get(1))
        .map_err(RecoveryError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(RecoveryError::from)
}

fn candidate_table_is_exact(conn: &Connection) -> Result<bool, RecoveryError> {
    object_contains_all(
        conn,
        "table",
        "edge_node_recovery_candidate",
        &[
            "singletonintegerprimarykeycheck(singleton=1)",
            "statetextnotnullcheck(state='durably_fenced_candidate')",
            "candidate_instance_idtextnotnullunique",
            "credential_generationintegernotnullcheck(credential_generation>=0)",
            "handoff_schema_versionintegernotnullcheck(handoff_schema_version=1)",
        ],
    )
}

fn backup_attempt_table_is_exact(conn: &Connection) -> Result<bool, RecoveryError> {
    object_contains_all(
        conn,
        "table",
        "edge_node_backup_attempts",
        &[
            "attempt_idtextprimarykey",
            "backup_idtextnotnullunique",
            "statetextnotnullcheck(statein('started','success','failed'))",
            "artifact_nametextnotnullunique",
            "state='started'andreason_codeisnullandcompleted_at_msisnull",
            "state='success'andreason_code='ok'",
            "state='failed'andreason_codeisnotnullandreason_code<>'ok'",
        ],
    )
}

fn candidate_trigger_is_exact(conn: &Connection) -> Result<bool, RecoveryError> {
    object_contains_all(
        conn,
        "trigger",
        "edge_node_recovery_candidate_immutable",
        &[
            "beforeupdateonedge_node_recovery_candidate",
            "selectraise(abort,'recoverycandidateisimmutable')",
        ],
    )
}

fn backup_attempt_trigger_is_exact(conn: &Connection) -> Result<bool, RecoveryError> {
    object_contains_all(
        conn,
        "trigger",
        "edge_node_backup_attempts_forward_only",
        &[
            "beforeupdateonedge_node_backup_attempts",
            "old.state='started'andnew.statein('success','failed')",
            "selectraise(abort,'backupattempttransitionisnotallowed')",
        ],
    )
}

fn object_contains_all(
    conn: &Connection,
    object_type: &str,
    name: &str,
    required: &[&str],
) -> Result<bool, RecoveryError> {
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = ?1 AND name = ?2",
            [object_type, name],
            |row| row.get(0),
        )
        .optional()
        .map_err(RecoveryError::from)?;
    let Some(sql) = sql else {
        return Ok(false);
    };
    let normalized: String = sql
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    Ok(required
        .iter()
        .all(|fragment| normalized.contains(fragment)))
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 255 && !value.chars().any(char::is_control)
}

#[cfg(test)]
#[path = "../tests/unit/state_tests.rs"]
mod tests;
