use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::{RecoveryError, RecoveryStartupMode};

const RECOVERY_SCHEMA: &[(&str, &str, &str)] = &[
    (
        "table",
        "edge_node_recovery_candidate",
        "createtableedge_node_recovery_candidate(singletonintegerprimarykeycheck(singleton=1),statetextnotnullcheck(state='durably_fenced_candidate'),recovery_idtextnotnull,candidate_instance_idtextnotnullunique,backup_idtext,source_database_lengthinteger,source_database_sha256text,edge_idtextnotnull,edge_node_idtextnotnull,old_ledger_epochtextnotnull,proposed_new_epochtextnotnull,credential_generationintegernotnullcheck(credential_generation>=0),handoff_schema_versionintegernotnullcheck(handoff_schema_version=1),installed_at_msintegernotnull,check((backup_idisnullandsource_database_lengthisnullandsource_database_sha256isnull)or(backup_idisnotnullandsource_database_lengthisnotnullandsource_database_length>=0andsource_database_sha256isnotnullandlength(source_database_sha256)=64andsource_database_sha256notglob'*[^0-9a-f]*')))",
    ),
    (
        "trigger",
        "edge_node_recovery_candidate_immutable",
        "createtriggeredge_node_recovery_candidate_immutablebeforeupdateonedge_node_recovery_candidatebeginselectraise(abort,'recoverycandidateisimmutable');end",
    ),
    (
        "table",
        "edge_node_backup_attempts",
        "createtableedge_node_backup_attempts(attempt_idtextprimarykey,backup_idtextnotnullunique,statetextnotnullcheck(statein('started','success','failed')),reason_codetext,artifact_nametextnotnullunique,artifact_lengthinteger,edge_node_idtextnotnull,ledger_epochtext,accepted_cursorinteger,allocation_high_waterinteger,started_at_msintegernotnull,artifact_created_at_msinteger,completed_at_msinteger,check((state='started'andreason_codeisnullandcompleted_at_msisnull)or(state='success'andreason_code='ok'andartifact_lengthisnotnullandledger_epochisnotnullandaccepted_cursorisnotnullandallocation_high_waterisnotnullandartifact_created_at_msisnotnullandcompleted_at_msisnotnull)or(state='failed'andreason_codeisnotnullandreason_code<>'ok'andcompleted_at_msisnotnull)))",
    ),
    (
        "trigger",
        "edge_node_backup_attempts_forward_only",
        "createtriggeredge_node_backup_attempts_forward_onlybeforeupdateonedge_node_backup_attemptswhenold.state<>'started'ornew.statenotin('success','failed')ornew.attempt_id<>old.attempt_idornew.backup_id<>old.backup_idornew.artifact_name<>old.artifact_nameornew.edge_node_id<>old.edge_node_idornew.started_at_ms<>old.started_at_msor(new.state='failed'and(new.artifact_lengthisnotnullornew.ledger_epochisnotnullornew.accepted_cursorisnotnullornew.allocation_high_waterisnotnullornew.artifact_created_at_msisnotnull))beginselectraise(abort,'backupattempttransitionisnotallowed');end",
    ),
    (
        "trigger",
        "edge_node_backup_attempts_insert_state",
        "createtriggeredge_node_backup_attempts_insert_statebeforeinsertonedge_node_backup_attemptswhennew.statenotin('started','failed')or(new.state='started'and(new.artifact_lengthisnotnullornew.ledger_epochisnotnullornew.accepted_cursorisnotnullornew.allocation_high_waterisnotnullornew.artifact_created_at_msisnotnull))or(new.state='failed'and(new.artifact_lengthisnotnullornew.ledger_epochisnotnullornew.accepted_cursorisnotnullornew.allocation_high_waterisnotnullornew.artifact_created_at_msisnotnull))beginselectraise(abort,'backupattemptcreationisnotallowed');end",
    ),
    (
        "trigger",
        "edge_node_backup_attempts_immutable",
        "createtriggeredge_node_backup_attempts_immutablebeforedeleteonedge_node_backup_attemptsbeginselectraise(abort,'backupattemptisimmutable');end",
    ),
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
    if !recovery_schema_is_exact(conn)? || !backup_attempts_are_valid(conn)? {
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
            "SELECT state, recovery_id, candidate_instance_id, backup_id,
                source_database_length, source_database_sha256, edge_id, edge_node_id,
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
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, i64>(12)?,
                ))
            },
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    let (
        state,
        recovery_id,
        candidate_instance_id,
        backup_id,
        source_database_length,
        source_database_sha256,
        edge_id,
        edge_node_id,
        old_ledger_epoch,
        proposed_new_epoch,
        credential_generation,
        handoff_schema_version,
        installed_at_ms,
    ) = row;
    let provenance_valid = match (
        backup_id.as_deref(),
        source_database_length,
        source_database_sha256.as_deref(),
    ) {
        (None, None, None) => true,
        (Some(_), Some(length), Some(digest)) => length >= 0 && valid_digest(digest),
        _ => false,
    };
    if state != "durably_fenced_candidate"
        || !valid_identity(&recovery_id)
        || !valid_identity(&candidate_instance_id)
        || !backup_id.as_deref().is_none_or(valid_identity)
        || !provenance_valid
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

fn recovery_schema_is_exact(conn: &Connection) -> Result<bool, RecoveryError> {
    let object_count: usize = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema
             WHERE type IN ('table', 'trigger')
               AND (name LIKE 'edge_node_recovery_%' OR name LIKE 'edge_node_backup_attempts%')",
            [],
            |row| row.get(0),
        )
        .map_err(RecoveryError::from)?;
    if object_count != RECOVERY_SCHEMA.len() {
        return Ok(false);
    }
    RECOVERY_SCHEMA
        .iter()
        .try_fold(true, |matches, (object_type, name, expected)| {
            if !matches {
                return Ok(false);
            }
            let actual: Option<String> = conn
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type = ?1 AND name = ?2",
                    [object_type, name],
                    |row| row.get(0),
                )
                .optional()
                .map_err(RecoveryError::from)?;
            Ok(actual.is_some_and(|sql| normalize_sql(&sql) == *expected))
        })
}

fn backup_attempts_are_valid(conn: &Connection) -> Result<bool, RecoveryError> {
    let mut statement = conn
        .prepare(
            "SELECT attempt_id, backup_id, state, reason_code, artifact_name, artifact_length,
                    edge_node_id, ledger_epoch, accepted_cursor, allocation_high_water,
                    started_at_ms, artifact_created_at_ms, completed_at_ms
             FROM edge_node_backup_attempts",
        )
        .map_err(RecoveryError::from)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, Option<i64>>(12)?,
            ))
        })
        .map_err(RecoveryError::from)?;
    for row in rows {
        let (
            attempt_id,
            backup_id,
            state,
            reason_code,
            artifact_name,
            artifact_length,
            edge_node_id,
            ledger_epoch,
            accepted_cursor,
            allocation_high_water,
            started_at_ms,
            artifact_created_at_ms,
            completed_at_ms,
        ) = row.map_err(RecoveryError::from)?;
        if !valid_identity(&attempt_id)
            || !valid_identity(&backup_id)
            || !valid_identity(&artifact_name)
            || !valid_identity(&edge_node_id)
            || started_at_ms < 0
        {
            return Ok(false);
        }
        let terminal_fields_are_empty = artifact_length.is_none()
            && ledger_epoch.is_none()
            && accepted_cursor.is_none()
            && allocation_high_water.is_none()
            && artifact_created_at_ms.is_none();
        let valid = match state.as_str() {
            "started" => {
                reason_code.is_none() && completed_at_ms.is_none() && terminal_fields_are_empty
            }
            "success" => {
                reason_code.as_deref() == Some("ok")
                    && artifact_length.is_some_and(|value| value >= 0)
                    && ledger_epoch.as_deref().is_some_and(valid_identity)
                    && accepted_cursor.is_some_and(|value| value >= 0)
                    && allocation_high_water.is_some_and(|value| value >= 0)
                    && artifact_created_at_ms.is_some_and(|value| value >= started_at_ms)
                    && completed_at_ms.is_some_and(|value| value >= started_at_ms)
            }
            "failed" => {
                reason_code
                    .as_deref()
                    .is_some_and(crate::backup::valid_backup_failure_reason)
                    && terminal_fields_are_empty
                    && completed_at_ms.is_some_and(|value| value >= started_at_ms)
            }
            _ => false,
        };
        if !valid {
            return Ok(false);
        }
    }
    Ok(true)
}

fn normalize_sql(sql: &str) -> String {
    sql.chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 255 && !value.chars().any(char::is_control)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
#[path = "../tests/unit/state_tests.rs"]
mod tests;
