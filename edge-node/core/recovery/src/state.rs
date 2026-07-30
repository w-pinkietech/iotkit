use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::{RecoveryError, RecoveryStartupMode};

const RECOVERY_SCHEMA: &[(&str, &str, &str)] = &[
    (
        "table",
        "edge_node_recovery_candidate",
        "createtableedge_node_recovery_candidate(singletonintegerprimarykeycheck(singleton=1),statetextnotnullcheck(state='durably_fenced_candidate'),recovery_idtextnotnull,candidate_instance_idtextnotnullunique,backup_idtext,source_database_lengthinteger,source_database_sha256text,artifact_lengthinteger,artifact_sha256text,edge_idtextnotnull,edge_node_idtextnotnull,old_ledger_epochtextnotnull,proposed_new_epochtextnotnull,credential_generationintegernotnullcheck(credential_generation>=0),handoff_schema_versionintegernotnullcheck(handoff_schema_version=1),installed_at_msintegernotnull,check((backup_idisnullandsource_database_lengthisnullandsource_database_sha256isnullandartifact_lengthisnullandartifact_sha256isnull)or(backup_idisnotnullandsource_database_lengthisnotnullandsource_database_length>=0andsource_database_sha256isnotnullandlength(source_database_sha256)=64andsource_database_sha256notglob'*[^0-9a-f]*'andartifact_lengthisnotnullandartifact_length>=0andartifact_sha256isnotnullandlength(artifact_sha256)=64andartifact_sha256notglob'*[^0-9a-f]*')))",
    ),
    (
        "trigger",
        "edge_node_recovery_candidate_immutable",
        "createtriggeredge_node_recovery_candidate_immutablebeforeupdateonedge_node_recovery_candidatebeginselectraise(abort,'recoverycandidateisimmutable');end",
    ),
    (
        "trigger",
        "edge_node_recovery_candidate_immutable_delete",
        "createtriggeredge_node_recovery_candidate_immutable_deletebeforedeleteonedge_node_recovery_candidatebeginselectraise(abort,'recoverycandidateisimmutable');end",
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
    (
        "table",
        "edge_node_recovery_activation",
        "createtableedge_node_recovery_activation(singletonintegerprimarykeycheck(singleton=1),statetextnotnullcheck(statein('applied','completed')),request_jsontextnotnullcheck(json_valid(request_json)),result_jsontextnotnullcheck(json_valid(result_json)),completion_jsontextcheck(completion_jsonisnullorjson_valid(completion_json)),applied_at_msintegernotnullcheck(applied_at_ms>=0),completed_at_msintegercheck(completed_at_msisnullorcompleted_at_ms>=0),check((state='applied'andcompletion_jsonisnullandcompleted_at_msisnull)or(state='completed'andcompletion_jsonisnotnullandcompleted_at_msisnotnull)))",
    ),
    (
        "trigger",
        "edge_node_recovery_activation_forward_only",
        "createtriggeredge_node_recovery_activation_forward_onlybeforeupdateonedge_node_recovery_activationwhenold.state<>'applied'ornew.state<>'completed'ornew.singleton<>old.singletonornew.request_json<>old.request_jsonornew.result_json<>old.result_jsonornew.applied_at_ms<>old.applied_at_msornew.completion_jsonisnullornew.completed_at_msisnullbeginselectraise(abort,'recoveryactivationtransitionisnotallowed');end",
    ),
    (
        "trigger",
        "edge_node_recovery_activation_insert_state",
        "createtriggeredge_node_recovery_activation_insert_statebeforeinsertonedge_node_recovery_activationwhennew.state<>'applied'ornew.completion_jsonisnotnullornew.completed_at_msisnotnullbeginselectraise(abort,'recoveryactivationmustbeginapplied');end",
    ),
    (
        "trigger",
        "edge_node_recovery_activation_immutable_delete",
        "createtriggeredge_node_recovery_activation_immutable_deletebeforedeleteonedge_node_recovery_activationbeginselectraise(abort,'recoveryactivationisimmutable');end",
    ),
];

/// Reads the recovery fence from an initialized database.
pub fn startup_mode(conn: &Connection) -> Result<RecoveryStartupMode, RecoveryError> {
    let candidate_table = table_exists(conn, "edge_node_recovery_candidate")?;
    let attempt_table = table_exists(conn, "edge_node_backup_attempts")?;
    let activation_table = table_exists(conn, "edge_node_recovery_activation")?;
    let migrations = recovery_migrations_applied(conn)?;
    if !candidate_table && !attempt_table && !activation_table && !migrations {
        return Ok(RecoveryStartupMode::Normal);
    }
    if !candidate_table || !attempt_table || !activation_table || !migrations {
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
        0 => {
            let activation_rows: i64 = conn
                .query_row(
                    "SELECT count(*) FROM edge_node_recovery_activation",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| RecoveryError::InvalidStartupState)?;
            if activation_rows == 0 {
                Ok(RecoveryStartupMode::Normal)
            } else {
                Err(RecoveryError::InvalidStartupState)
            }
        }
        1 => load_recovery_mode(conn),
        _ => Err(RecoveryError::InvalidStartupState),
    }
}

fn load_recovery_mode(conn: &Connection) -> Result<RecoveryStartupMode, RecoveryError> {
    let candidate = load_candidate(conn)?;
    let activation_rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM edge_node_recovery_activation",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    if activation_rows == 0 {
        return Ok(candidate);
    }
    if activation_rows != 1 {
        return Err(RecoveryError::InvalidStartupState);
    }
    let RecoveryStartupMode::FencedCandidate {
        recovery_id,
        candidate_instance_id,
        backup_id,
        edge_id,
        old_ledger_epoch,
        proposed_new_epoch,
        credential_generation,
    } = candidate
    else {
        return Err(RecoveryError::InvalidStartupState);
    };
    let Some(backup_id) = backup_id else {
        return Err(RecoveryError::InvalidStartupState);
    };
    let (edge_node_id, state, request_json, result_json, completion_json): (
        String,
        String,
        String,
        String,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT candidate.edge_node_id,activation.state,activation.request_json,
                    activation.result_json,activation.completion_json
             FROM edge_node_recovery_candidate AS candidate
             CROSS JOIN edge_node_recovery_activation AS activation
             WHERE candidate.singleton=1 AND activation.singleton=1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    let request = crate::RecoveryActivationRequest::decode(request_json.as_bytes())
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    let result = crate::RecoveryActivationResult::decode(result_json.as_bytes())
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    result
        .validate_for(&request)
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    if request.recovery_id != recovery_id
        || request.candidate_instance_id != candidate_instance_id
        || request.backup_id != backup_id
        || request.edge_id != edge_id
        || request.edge_node_id != edge_node_id
        || request.old_ledger_epoch != old_ledger_epoch
        || request.new_ledger_epoch != proposed_new_epoch
        || request.broker_credential_generation != credential_generation
    {
        return Err(RecoveryError::InvalidStartupState);
    }
    validate_activated_database(conn, &request, &result, state.as_str() == "applied")?;
    match (state.as_str(), completion_json) {
        ("applied", None) => Ok(RecoveryStartupMode::AwaitingCompletion {
            recovery_id,
            candidate_instance_id,
            new_ledger_epoch: proposed_new_epoch,
        }),
        ("completed", Some(completion_json)) => {
            let completion = crate::RecoveryCompletion::decode(completion_json.as_bytes())
                .map_err(|_| RecoveryError::InvalidStartupState)?;
            completion
                .validate_for(&request)
                .map_err(|_| RecoveryError::InvalidStartupState)?;
            Ok(RecoveryStartupMode::Recovered {
                recovery_id,
                candidate_instance_id,
                new_ledger_epoch: proposed_new_epoch,
            })
        }
        _ => Err(RecoveryError::InvalidStartupState),
    }
}

fn validate_activated_database(
    conn: &Connection,
    request: &crate::RecoveryActivationRequest,
    result: &crate::RecoveryActivationResult,
    awaiting_completion: bool,
) -> Result<(), RecoveryError> {
    let ledger_epoch: String = conn
        .query_row(
            "SELECT value FROM ledger_meta WHERE key='epoch'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    let device_generation: i64 = conn
        .query_row(
            "SELECT device_credential_generation FROM auth_state WHERE id=1",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    let (target_epoch, target_cursor): (Option<String>, i64) = conn
        .query_row(
            "SELECT cursor_epoch,cursor_pub_seq FROM target_registry",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    let (activation_state, activation_edge, activation_epoch): (
        String,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT state,edge_id,ledger_epoch FROM edge_node_activation WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    let foreign_rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM publication_log WHERE epoch<>?1",
            [request.new_ledger_epoch.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::InvalidStartupState)?;
    let device_generation_is_valid = if awaiting_completion {
        device_generation == request.device_auth_generation
    } else {
        device_generation >= request.device_auth_generation
    };
    if ledger_epoch != request.new_ledger_epoch
        || !device_generation_is_valid
        || target_epoch.as_deref() != Some(request.new_ledger_epoch.as_str())
        || target_cursor < 0
        || activation_state != "active"
        || activation_edge.as_deref() != Some(request.edge_id.as_str())
        || activation_epoch.as_deref() != Some(request.new_ledger_epoch.as_str())
        || foreign_rows != 0
    {
        return Err(RecoveryError::InvalidStartupState);
    }
    if awaiting_completion {
        let (count, minimum, maximum): (i64, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT count(*),min(pub_seq),max(pub_seq) FROM publication_log",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| RecoveryError::InvalidStartupState)?;
        if target_cursor != 0
            || count != result.last_new_publication_seq
            || count != result.replayed_records.saturating_add(1)
            || minimum != Some(1)
            || maximum != Some(count)
        {
            return Err(RecoveryError::InvalidStartupState);
        }
    }
    Ok(())
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
                source_database_length, source_database_sha256, artifact_length, artifact_sha256,
                edge_id, edge_node_id,
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
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, i64>(14)?,
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
        artifact_length,
        artifact_sha256,
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
        artifact_length,
        artifact_sha256.as_deref(),
    ) {
        (None, None, None, None, None) => true,
        (
            Some(_),
            Some(source_length),
            Some(source_digest),
            Some(artifact_length),
            Some(artifact_digest),
        ) => {
            source_length >= 0
                && valid_digest(source_digest)
                && artifact_length >= 0
                && valid_digest(artifact_digest)
        }
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

fn recovery_migrations_applied(conn: &Connection) -> Result<bool, RecoveryError> {
    if !table_exists(conn, "_schema_version")? {
        return Ok(false);
    }
    conn.query_row(
        "SELECT
             EXISTS(SELECT 1 FROM _schema_version WHERE version = 23)
             AND EXISTS(SELECT 1 FROM _schema_version WHERE version = 24)",
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
    crate::model::valid_recovery_id(value)
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
