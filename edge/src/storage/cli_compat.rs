use sqlx::Row;
use uuid::Uuid;

use super::{
    CliRawRecordRow, CliRouteDraft, CliRouteStatusRow, CliSemanticEventRow, CliSemanticRevisionRow,
    Storage, StorageError, StorageInner,
};

impl Storage {
    pub async fn list_cli_raw_records(
        &self,
        limit: usize,
    ) -> Result<Vec<CliRawRecordRow>, StorageError> {
        if !(1..=10_000).contains(&limit) {
            return Err(StorageError::InvalidRecord(
                "raw record query limit must be between 1 and 10000".into(),
            ));
        }
        let limit = i64::try_from(limit)
            .map_err(|_| StorageError::InvalidRecord("raw record query limit is invalid".into()))?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT edge_node_id,ledger_epoch,pub_seq,publication_id,record_json,received_at \
                     FROM raw_records ORDER BY received_at DESC,edge_node_id,ledger_epoch,pub_seq DESC \
                     LIMIT ?",
                )
                .bind(limit)
                .fetch_all(pool)
                .await?;
                rows.into_iter()
                    .map(|row| {
                        Ok(CliRawRecordRow {
                            edge_node_id: row.try_get("edge_node_id")?,
                            ledger_epoch: row.try_get("ledger_epoch")?,
                            pub_seq: row.try_get("pub_seq")?,
                            publication_id: row.try_get("publication_id")?,
                            record_json: row.try_get("record_json")?,
                            received_at: row.try_get("received_at")?,
                        })
                    })
                    .collect()
            }
            StorageInner::Postgres { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT edge_node_id,ledger_epoch,pub_seq,publication_id,record_json,received_at \
                     FROM raw_records ORDER BY received_at DESC,edge_node_id,ledger_epoch,pub_seq DESC \
                     LIMIT $1",
                )
                .bind(limit)
                .fetch_all(pool)
                .await?;
                rows.into_iter()
                    .map(|row| {
                        Ok(CliRawRecordRow {
                            edge_node_id: row.try_get("edge_node_id")?,
                            ledger_epoch: row.try_get("ledger_epoch")?,
                            pub_seq: row.try_get("pub_seq")?,
                            publication_id: row.try_get("publication_id")?,
                            record_json: row.try_get("record_json")?,
                            received_at: row.try_get("received_at")?,
                        })
                    })
                    .collect()
            }
        }
    }

    pub async fn list_cli_semantic_revisions(
        &self,
    ) -> Result<Vec<CliSemanticRevisionRow>, StorageError> {
        const SELECT: &str = "SELECT rule.rule_id,revision.revision,signal.edge_node_id,signal.series_key,\
             revision.spec_json,(rule.active AND revision.revision=rule.revision) AS active,\
             revision.created_at FROM semantic_rules AS rule \
             JOIN semantic_signals AS signal ON signal.signal_ref=rule.signal_ref \
             JOIN semantic_rule_revisions AS revision ON revision.rule_id=rule.rule_id \
             WHERE rule.display_name='production_pulse' AND rule.kind='cumulative_counter' \
             ORDER BY rule.rule_id,revision.revision";
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let rows = sqlx::query(SELECT).fetch_all(pool).await?;
                rows.into_iter()
                    .map(|row| {
                        let encoded: Vec<u8> = row.try_get("spec_json")?;
                        Ok(CliSemanticRevisionRow {
                            rule_id: row.try_get("rule_id")?,
                            revision: row.try_get("revision")?,
                            edge_node_id: row.try_get("edge_node_id")?,
                            series_key: row.try_get("series_key")?,
                            spec: serde_json::from_slice(&encoded).map_err(|error| {
                                StorageError::InvalidSemantic(error.to_string())
                            })?,
                            active: row.try_get::<i64, _>("active")? != 0,
                            created_at: row.try_get("created_at")?,
                        })
                    })
                    .collect()
            }
            StorageInner::Postgres { pool, .. } => {
                let rows = sqlx::query(SELECT).fetch_all(pool).await?;
                rows.into_iter()
                    .map(|row| {
                        let encoded: serde_json::Value = row.try_get("spec_json")?;
                        Ok(CliSemanticRevisionRow {
                            rule_id: row.try_get("rule_id")?,
                            revision: row.try_get("revision")?,
                            edge_node_id: row.try_get("edge_node_id")?,
                            series_key: row.try_get("series_key")?,
                            spec: serde_json::from_value(encoded).map_err(|error| {
                                StorageError::InvalidSemantic(error.to_string())
                            })?,
                            active: row.try_get("active")?,
                            created_at: row.try_get("created_at")?,
                        })
                    })
                    .collect()
            }
        }
    }

    pub async fn list_cli_semantic_events(
        &self,
        limit: usize,
    ) -> Result<Vec<CliSemanticEventRow>, StorageError> {
        if !(1..=10_000).contains(&limit) {
            return Err(StorageError::InvalidSemantic(
                "semantic event query limit must be between 1 and 10000".into(),
            ));
        }
        let limit = i64::try_from(limit).map_err(|_| {
            StorageError::InvalidSemantic("semantic event query limit is invalid".into())
        })?;
        let select = "SELECT observation.observation_id,observation.rule_id,observation.revision,\
             observation.sequence,observation.edge_node_id,observation.ledger_epoch,\
             observation.source_pub_seq,signal.series_key,observation.observed_at,\
             observation.created_at FROM semantic_observations AS observation \
             JOIN semantic_rules AS rule ON rule.rule_id=observation.rule_id \
             JOIN semantic_signals AS signal ON signal.signal_ref=rule.signal_ref \
             WHERE rule.display_name='production_pulse' AND rule.kind='cumulative_counter' \
             ORDER BY observation.observation_row_id LIMIT ";
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let rows = sqlx::query(&format!("{select}?"))
                    .bind(limit)
                    .fetch_all(pool)
                    .await?;
                rows.into_iter().map(decode_event).collect()
            }
            StorageInner::Postgres { pool, .. } => {
                let rows = sqlx::query(&format!("{select}$1"))
                    .bind(limit)
                    .fetch_all(pool)
                    .await?;
                rows.into_iter().map(decode_event).collect()
            }
        }
    }

    pub async fn add_cli_output_route(
        &self,
        draft: &CliRouteDraft,
        now: i64,
    ) -> Result<CliRouteStatusRow, StorageError> {
        if now < 0 {
            return Err(StorageError::InvalidOutput(
                "route timestamp must be non-negative".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let config = serde_json::to_vec(&draft.config)
                    .map_err(|error| StorageError::InvalidOutput(error.to_string()))?;
                if let Some(route_id) = sqlx::query_scalar::<_, String>(
                    "SELECT route.route_id FROM output_routes AS route \
                     JOIN semantic_rules AS rule ON rule.rule_id=route.rule_id \
                     WHERE route.rule_id=? AND route.adapter_id=? \
                     AND CAST(route.config_json AS TEXT)=CAST(? AS TEXT) \
                     AND route.active=1 AND rule.active=1",
                )
                .bind(&draft.rule_id)
                .bind(&draft.adapter_id)
                .bind(&config)
                .fetch_optional(&mut *tx)
                .await?
                {
                    insert_cli_audit_sqlite(&mut tx, now, "route_add", &route_id).await?;
                    tx.commit().await?;
                    return self.cli_route_status(&route_id).await;
                }
                ensure_compatible_rule_sqlite(&mut tx, &draft.rule_id).await?;
                let profile_id = ensure_cli_profile_sqlite(&mut tx, &draft.adapter_id, now).await?;
                let binding_id =
                    ensure_cli_binding_sqlite(&mut tx, &profile_id, &draft.rule_id, now).await?;
                let boundary: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(observation_row_id),0) FROM semantic_observations",
                )
                .fetch_one(&mut *tx)
                .await?;
                let route_id = format!("route_{}", Uuid::new_v4());
                sqlx::query(
                    "INSERT INTO output_routes(route_id,binding_id,rule_id,adapter_id,\
                     config_schema_version,config_json,start_after_observation_row_id,active,\
                     lifecycle_state,created_at) VALUES(?,?,?,?,?,?,?,1,'active',?)",
                )
                .bind(&route_id)
                .bind(&binding_id)
                .bind(&draft.rule_id)
                .bind(&draft.adapter_id)
                .bind(draft.config_schema_version)
                .bind(&config)
                .bind(boundary)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                insert_cli_audit_sqlite(&mut tx, now, "route_add", &route_id).await?;
                tx.commit().await?;
                self.cli_route_status(&route_id).await
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                ensure_compatible_rule_postgres(&mut tx, &draft.rule_id).await?;
                if let Some(route_id) = sqlx::query_scalar::<_, String>(
                    "SELECT route.route_id FROM output_routes AS route \
                     JOIN semantic_rules AS rule ON rule.rule_id=route.rule_id \
                     WHERE route.rule_id=$1 AND route.adapter_id=$2 AND route.config_json=$3 \
                     AND route.active=TRUE AND rule.active=TRUE FOR UPDATE OF route",
                )
                .bind(&draft.rule_id)
                .bind(&draft.adapter_id)
                .bind(&draft.config)
                .fetch_optional(&mut *tx)
                .await?
                {
                    insert_cli_audit_postgres(&mut tx, now, "route_add", &route_id).await?;
                    tx.commit().await?;
                    return self.cli_route_status(&route_id).await;
                }
                let profile_id =
                    ensure_cli_profile_postgres(&mut tx, &draft.adapter_id, now).await?;
                let binding_id =
                    ensure_cli_binding_postgres(&mut tx, &profile_id, &draft.rule_id, now).await?;
                let boundary: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(observation_row_id),0) FROM semantic_observations",
                )
                .fetch_one(&mut *tx)
                .await?;
                let route_id = format!("route_{}", Uuid::new_v4());
                sqlx::query(
                    "INSERT INTO output_routes(route_id,binding_id,rule_id,adapter_id,\
                     config_schema_version,config_json,start_after_observation_row_id,active,\
                     lifecycle_state,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,TRUE,'active',$8)",
                )
                .bind(&route_id)
                .bind(&binding_id)
                .bind(&draft.rule_id)
                .bind(&draft.adapter_id)
                .bind(draft.config_schema_version)
                .bind(&draft.config)
                .bind(boundary)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                insert_cli_audit_postgres(&mut tx, now, "route_add", &route_id).await?;
                tx.commit().await?;
                self.cli_route_status(&route_id).await
            }
        }
    }

    pub async fn list_cli_route_statuses(&self) -> Result<Vec<CliRouteStatusRow>, StorageError> {
        let select = "SELECT route.route_id,route.rule_id,route.config_json,\
             route.start_after_observation_row_id,route.active,route.created_at,\
             COUNT(outbox.export_id) FILTER (WHERE outbox.published_at IS NULL) AS pending_count,\
             COUNT(outbox.export_id) FILTER (WHERE outbox.published_at IS NOT NULL) AS published_count,\
             MIN(outbox.created_at) FILTER (WHERE outbox.published_at IS NULL) AS oldest_pending_at \
             FROM output_routes AS route JOIN semantic_rules AS rule ON rule.rule_id=route.rule_id \
             LEFT JOIN output_outbox AS outbox ON outbox.route_id=route.route_id \
             WHERE route.adapter_id='iotkit.mqtt-json.v1' AND route.route_id LIKE 'route_%' \
             AND rule.display_name='production_pulse' AND rule.kind='cumulative_counter' \
             GROUP BY route.route_id,route.rule_id,route.config_json,\
             route.start_after_observation_row_id,route.active,route.created_at \
             ORDER BY route.route_id";
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let rows = sqlx::query(select).fetch_all(pool).await?;
                rows.into_iter().map(decode_route_sqlite).collect()
            }
            StorageInner::Postgres { pool, .. } => {
                let rows = sqlx::query(select).fetch_all(pool).await?;
                rows.into_iter().map(decode_route_postgres).collect()
            }
        }
    }

    async fn cli_route_status(&self, route_id: &str) -> Result<CliRouteStatusRow, StorageError> {
        self.list_cli_route_statuses()
            .await?
            .into_iter()
            .find(|route| route.route_id == route_id)
            .ok_or(StorageError::SemanticNotFound)
    }
}

async fn ensure_compatible_rule_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    rule_id: &str,
) -> Result<(), StorageError> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM semantic_rules \
         WHERE rule_id=? AND display_name='production_pulse' \
         AND kind='cumulative_counter' AND active=1",
    )
    .bind(rule_id)
    .fetch_one(&mut **tx)
    .await?;
    if exists == 1 {
        Ok(())
    } else {
        Err(StorageError::SemanticNotFound)
    }
}

async fn ensure_compatible_rule_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rule_id: &str,
) -> Result<(), StorageError> {
    let exists: Option<String> = sqlx::query_scalar(
        "SELECT rule_id FROM semantic_rules \
         WHERE rule_id=$1 AND display_name='production_pulse' \
         AND kind='cumulative_counter' AND active=TRUE FOR UPDATE",
    )
    .bind(rule_id)
    .fetch_one(&mut **tx)
    .await?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(StorageError::SemanticNotFound)
    }
}

async fn ensure_cli_profile_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    adapter_id: &str,
    now: i64,
) -> Result<String, StorageError> {
    if let Some(id) = sqlx::query_scalar(
        "SELECT profile_id FROM export_profiles \
         WHERE adapter_id=? AND state IN ('preparing','active','draining')",
    )
    .bind(adapter_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok(id);
    }
    let id = format!("exp_{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO export_profiles(profile_id,display_name,adapter_id,adapter_schema_version,\
         setup_json,state,revision,created_at) VALUES(?,'Legacy MQTT routes',?,1,'{}','active',1,?)",
    )
    .bind(&id)
    .bind(adapter_id)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

async fn ensure_cli_profile_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    adapter_id: &str,
    now: i64,
) -> Result<String, StorageError> {
    if let Some(id) = sqlx::query_scalar(
        "SELECT profile_id FROM export_profiles \
         WHERE adapter_id=$1 AND state IN ('preparing','active','draining') FOR UPDATE",
    )
    .bind(adapter_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok(id);
    }
    let id = format!("exp_{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO export_profiles(profile_id,display_name,adapter_id,adapter_schema_version,\
         setup_json,state,revision,created_at) \
         VALUES($1,'Legacy MQTT routes',$2,1,'{}'::jsonb,'active',1,$3)",
    )
    .bind(&id)
    .bind(adapter_id)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

async fn ensure_cli_binding_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    profile_id: &str,
    rule_id: &str,
    now: i64,
) -> Result<String, StorageError> {
    if let Some(id) = sqlx::query_scalar(
        "SELECT binding_id FROM output_bindings WHERE profile_id=? AND rule_id=?",
    )
    .bind(profile_id)
    .bind(rule_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok(id);
    }
    let id = format!("bind_{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO output_bindings(binding_id,profile_id,rule_id,mode,state,revision,\
         created_at,activated_at) VALUES(?,?,?,'observation','active',1,?,?)",
    )
    .bind(&id)
    .bind(profile_id)
    .bind(rule_id)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO output_binding_starts(binding_id,ledger_epoch,start_after_pub_seq) \
         SELECT ?,cursor.ledger_epoch,cursor.accepted_through FROM semantic_rules AS rule \
         JOIN semantic_signals AS signal ON signal.signal_ref=rule.signal_ref \
         JOIN accepted_cursors AS cursor ON cursor.edge_node_id=signal.edge_node_id \
         WHERE rule.rule_id=?",
    )
    .bind(&id)
    .bind(rule_id)
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

async fn ensure_cli_binding_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile_id: &str,
    rule_id: &str,
    now: i64,
) -> Result<String, StorageError> {
    if let Some(id) = sqlx::query_scalar(
        "SELECT binding_id FROM output_bindings WHERE profile_id=$1 AND rule_id=$2 FOR UPDATE",
    )
    .bind(profile_id)
    .bind(rule_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok(id);
    }
    let id = format!("bind_{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO output_bindings(binding_id,profile_id,rule_id,mode,state,revision,\
         created_at,activated_at) VALUES($1,$2,$3,'observation','active',1,$4,$4)",
    )
    .bind(&id)
    .bind(profile_id)
    .bind(rule_id)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO output_binding_starts(binding_id,ledger_epoch,start_after_pub_seq) \
         SELECT $1,cursor.ledger_epoch,cursor.accepted_through FROM semantic_rules AS rule \
         JOIN semantic_signals AS signal ON signal.signal_ref=rule.signal_ref \
         JOIN accepted_cursors AS cursor ON cursor.edge_node_id=signal.edge_node_id \
         WHERE rule.rule_id=$2",
    )
    .bind(&id)
    .bind(rule_id)
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

async fn insert_cli_audit_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    now: i64,
    operation: &str,
    resource: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO audit_events(occurred_at,actor_class,actor_ref,operation,resource_ref,\
         outcome,summary_json) VALUES(?,'local_cli','local-cli',?,?,'success','{}')",
    )
    .bind(now)
    .bind(operation)
    .bind(resource)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_cli_audit_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    now: i64,
    operation: &str,
    resource: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO audit_events(occurred_at,actor_class,actor_ref,operation,resource_ref,\
         outcome,summary_json) VALUES($1,'local_cli','local-cli',$2,$3,'success','{}'::jsonb)",
    )
    .bind(now)
    .bind(operation)
    .bind(resource)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn decode_route_sqlite(row: sqlx::sqlite::SqliteRow) -> Result<CliRouteStatusRow, StorageError> {
    let config: Vec<u8> = row.try_get("config_json")?;
    Ok(CliRouteStatusRow {
        route_id: row.try_get("route_id")?,
        rule_id: row.try_get("rule_id")?,
        config: serde_json::from_slice(&config)
            .map_err(|error| StorageError::InvalidOutput(error.to_string()))?,
        start_after_observation_row_id: row.try_get("start_after_observation_row_id")?,
        active: row.try_get::<i64, _>("active")? != 0,
        created_at: row.try_get("created_at")?,
        pending_count: row.try_get("pending_count")?,
        published_count: row.try_get("published_count")?,
        oldest_pending_at: row.try_get("oldest_pending_at")?,
    })
}

fn decode_route_postgres(row: sqlx::postgres::PgRow) -> Result<CliRouteStatusRow, StorageError> {
    Ok(CliRouteStatusRow {
        route_id: row.try_get("route_id")?,
        rule_id: row.try_get("rule_id")?,
        config: row.try_get("config_json")?,
        start_after_observation_row_id: row.try_get("start_after_observation_row_id")?,
        active: row.try_get("active")?,
        created_at: row.try_get("created_at")?,
        pending_count: row.try_get("pending_count")?,
        published_count: row.try_get("published_count")?,
        oldest_pending_at: row.try_get("oldest_pending_at")?,
    })
}

fn decode_event<R>(row: R) -> Result<CliSemanticEventRow, StorageError>
where
    R: Row,
    for<'column> &'column str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
    i64: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    Ok(CliSemanticEventRow {
        event_id: row.try_get("observation_id")?,
        rule_id: row.try_get("rule_id")?,
        mapping_revision: row.try_get("revision")?,
        event_sequence: row.try_get("sequence")?,
        edge_node_id: row.try_get("edge_node_id")?,
        ledger_epoch: row.try_get("ledger_epoch")?,
        source_pub_seq: row.try_get("source_pub_seq")?,
        source_series_key: row.try_get("series_key")?,
        occurred_at: row.try_get("observed_at")?,
        created_at: row.try_get("created_at")?,
    })
}
