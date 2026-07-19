use iotkit_core_ops::{
    Actor, ActorKind, DispatchRequest, NewOperatorToken, OpContext, OpDescriptor, OpError, Tier,
    TokenKind, dispatch, issue_token, standard_catalog,
};
use iotkit_core_storage::Migration;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

fn all_migrations() -> Vec<Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    all
}

fn catalog() -> Vec<OpDescriptor> {
    vec![
        OpDescriptor {
            name: "fake.write",
            tier: Tier::Daily,
            bulk_escalates: true,
            changes_state: true,
            params_schema,
            targets,
            preconditions,
            dry_run: fake_write,
            execute: fake_write,
            secret_execute: None,
        },
        OpDescriptor {
            name: "fake.fail",
            tier: Tier::Daily,
            bulk_escalates: true,
            changes_state: true,
            params_schema,
            targets,
            preconditions,
            dry_run: fake_write,
            execute: fake_fail,
            secret_execute: None,
        },
        OpDescriptor {
            name: "fake.precondition_fail",
            tier: Tier::Daily,
            bulk_escalates: true,
            changes_state: true,
            params_schema,
            targets,
            preconditions: preconditions_write_then_fail,
            dry_run: fake_write,
            execute: fake_write,
            secret_execute: None,
        },
        OpDescriptor {
            name: "fake.dry_fail",
            tier: Tier::Daily,
            bulk_escalates: true,
            changes_state: true,
            params_schema,
            targets,
            preconditions,
            dry_run: fake_fail,
            execute: fake_write,
            secret_execute: None,
        },
        OpDescriptor {
            name: "fake.readonly",
            tier: Tier::ReadOnly,
            bulk_escalates: true,
            changes_state: false,
            params_schema,
            targets,
            preconditions,
            dry_run: fake_write,
            execute: fake_write,
            secret_execute: None,
        },
    ]
}

fn params_schema() -> Value {
    json!({ "required": ["ids"] })
}

fn targets(params: &Value) -> Vec<String> {
    params
        .get("ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn preconditions(_tx: &rusqlite::Transaction<'_>, _ctx: &OpContext<'_>) -> Result<(), OpError> {
    Ok(())
}

fn preconditions_write_then_fail(
    tx: &rusqlite::Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<(), OpError> {
    let id = targets(ctx.params)
        .into_iter()
        .next()
        .ok_or_else(|| OpError::Validation("ids required".to_string()))?;
    insert_registry_marker(tx, &id)?;
    Err(OpError::PreconditionFailed("blocked".to_string()))
}

fn fake_write(tx: &rusqlite::Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let id = targets(ctx.params)
        .into_iter()
        .next()
        .ok_or_else(|| OpError::Validation("ids required".to_string()))?;
    insert_registry_marker(tx, &id)?;
    Ok(json!({
        "wrote": id,
        "actor_id": ctx.actor_id,
        "source": ctx.source,
    }))
}

fn fake_fail(tx: &rusqlite::Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let id = targets(ctx.params)
        .into_iter()
        .next()
        .ok_or_else(|| OpError::Validation("ids required".to_string()))?;
    insert_registry_marker(tx, &id)?;
    Err(OpError::Internal("boom".to_string()))
}

fn insert_registry_marker(conn: &Connection, id: &str) -> Result<(), OpError> {
    conn.execute(
        "INSERT INTO registry_entries (
            measurement_key, origin, catalog_version, entry_revision,
            unit_ucum, unit_display, value_type, semantic_class,
            channel_mode, channel_roles_json, physical_min, physical_max,
            site_min, site_max, enabled_at
         ) VALUES (?1, 'custom', NULL, 'fake-revision', NULL, NULL, 'float', 'test',
            'single', NULL, NULL, NULL, NULL, NULL, 1)",
        params![format!("fake.{id}")],
    )
    .map_err(|e| OpError::Internal(e.to_string()))?;
    Ok(())
}

fn actor(kind: ActorKind, ceiling: Tier) -> Actor {
    Actor {
        actor_id: match kind {
            ActorKind::Human => "tok_human".to_string(),
            ActorKind::Ai => "tok_ai".to_string(),
            ActorKind::LocalCli => "local_cli".to_string(),
            ActorKind::System => "system:test".to_string(),
        },
        actor_kind: kind,
        tier_ceiling: ceiling,
    }
}

fn request(
    op: &str,
    ids: &[&str],
    dry_run: bool,
    actor: Actor,
    step_up_verified: bool,
) -> DispatchRequest {
    DispatchRequest {
        op: op.to_string(),
        params: json!({ "ids": ids }),
        dry_run,
        actor,
        source: Some("test".to_string()),
        step_up_verified,
        clock_trust: None,
    }
}

fn standard_request(
    op: &str,
    params: Value,
    dry_run: bool,
    actor: Actor,
    step_up_verified: bool,
) -> DispatchRequest {
    DispatchRequest {
        op: op.to_string(),
        params,
        dry_run,
        actor,
        source: Some("test".to_string()),
        step_up_verified,
        clock_trust: None,
    }
}

fn human_token_actor(conn: &Connection) -> Actor {
    let issued = issue_token(
        conn,
        &NewOperatorToken {
            name: "human".to_string(),
            kind: TokenKind::Human,
            ceiling: Tier::Daily,
            is_session: false,
            expires_at: None,
        },
        "test",
        Some("test"),
        None,
    )
    .unwrap();
    Actor {
        actor_id: issued.token_id,
        actor_kind: ActorKind::Human,
        tier_ceiling: Tier::Daily,
    }
}

fn registry_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM registry_entries WHERE measurement_key LIKE 'fake.%'",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

fn r14_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM ledger_events WHERE kind = 'r14_op'",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

fn latest_r14(conn: &Connection) -> Value {
    let detail: String = conn
        .query_row(
            "SELECT detail FROM ledger_events WHERE kind = 'r14_op' ORDER BY event_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&detail).unwrap()
}

#[test]
fn dry_run_rolls_back_target_write_but_records_audit() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let before = registry_count(conn);

        let out = dispatch(
            conn,
            &catalog(),
            request(
                "fake.write",
                &["dry"],
                true,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap();

        assert_eq!(
            out,
            json!({ "wrote": "dry", "actor_id": "local_cli", "source": "test" })
        );
        assert_eq!(registry_count(conn), before);
        assert_eq!(r14_count(conn), 1);
        let detail = latest_r14(conn);
        assert_eq!(detail["dry_run"], true);
        assert_eq!(detail["result"], "ok");
        Ok(())
    })
    .unwrap();
}

#[test]
fn execute_success_commits_target_write_audit_and_generation_bump() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let before = registry_count(conn);
        let generation_before = iotkit_core_ledger::current_generation(conn).unwrap();

        let out = dispatch(
            conn,
            &catalog(),
            request(
                "fake.write",
                &["ok"],
                false,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap();

        assert_eq!(
            out,
            json!({ "wrote": "ok", "actor_id": "local_cli", "source": "test" })
        );
        assert_eq!(registry_count(conn), before + 1);
        assert_eq!(
            iotkit_core_ledger::current_generation(conn).unwrap(),
            generation_before + 1
        );
        let detail = latest_r14(conn);
        assert_eq!(detail["result"], "ok");
        assert_eq!(detail["targets"], json!(["ok"]));
        Ok(())
    })
    .unwrap();
}

#[test]
fn execute_failure_rolls_back_target_write_but_keeps_failure_audit() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let before = registry_count(conn);

        let err = dispatch(
            conn,
            &catalog(),
            request(
                "fake.fail",
                &["fail"],
                false,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap_err();

        assert!(matches!(err, OpError::Internal(message) if message == "boom"));
        assert_eq!(registry_count(conn), before);
        let detail = latest_r14(conn);
        assert_eq!(detail["result"], "error:internal");
        assert_eq!(detail["targets"], json!(["fail"]));
        Ok(())
    })
    .unwrap();
}

#[test]
fn revoked_human_token_is_rechecked_during_dispatch() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let human = human_token_actor(conn);
        conn.execute(
            "UPDATE operator_tokens SET revoked_at = 1 WHERE token_id = ?1",
            params![human.actor_id.as_str()],
        )
        .unwrap();

        let err = dispatch(
            conn,
            &catalog(),
            request("fake.write", &["revoked"], false, human, false),
        )
        .unwrap_err();

        assert!(matches!(err, OpError::Forbidden(reason) if reason == "token_revoked"));
        let detail = latest_r14(conn);
        assert_eq!(detail["result"], "error:forbidden");
        assert_eq!(registry_count(conn), 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn precondition_failure_rolls_back_target_write_but_keeps_failure_audit() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let before = registry_count(conn);

        let err = dispatch(
            conn,
            &catalog(),
            request(
                "fake.precondition_fail",
                &["precondition"],
                false,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap_err();

        assert!(matches!(err, OpError::PreconditionFailed(message) if message == "blocked"));
        assert_eq!(registry_count(conn), before);
        let detail = latest_r14(conn);
        assert_eq!(detail["result"], "error:precondition_failed");
        assert_eq!(detail["targets"], json!(["precondition"]));
        Ok(())
    })
    .unwrap();
}

#[test]
fn dry_run_failure_rolls_back_target_write_but_keeps_failure_audit() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let before = registry_count(conn);

        let err = dispatch(
            conn,
            &catalog(),
            request(
                "fake.dry_fail",
                &["dry-fail"],
                true,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap_err();

        assert!(matches!(err, OpError::Internal(message) if message == "boom"));
        assert_eq!(registry_count(conn), before);
        let detail = latest_r14(conn);
        assert_eq!(detail["dry_run"], true);
        assert_eq!(detail["result"], "error:internal");
        assert_eq!(detail["targets"], json!(["dry-fail"]));
        Ok(())
    })
    .unwrap();
}

#[test]
fn read_only_descriptor_tier_is_rejected_at_runtime_without_audit() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let before_audit = r14_count(conn);
        let before_registry = registry_count(conn);

        let err = dispatch(
            conn,
            &catalog(),
            request(
                "fake.readonly",
                &["readonly"],
                false,
                actor(ActorKind::LocalCli, Tier::Construction),
                true,
            ),
        )
        .unwrap_err();

        assert!(
            matches!(err, OpError::Internal(message) if message == "invalid op tier: read_only")
        );
        assert_eq!(r14_count(conn), before_audit);
        assert_eq!(registry_count(conn), before_registry);
        Ok(())
    })
    .unwrap();
}

#[test]
fn ceiling_below_effective_tier_is_forbidden_and_audited() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let err = dispatch(
            conn,
            &catalog(),
            request(
                "fake.write",
                &["deny"],
                false,
                actor(ActorKind::LocalCli, Tier::ReadOnly),
                false,
            ),
        )
        .unwrap_err();

        assert!(matches!(err, OpError::Forbidden(reason) if reason == "tier"));
        let detail = latest_r14(conn);
        assert_eq!(detail["result"], "error:forbidden");
        assert_eq!(detail["effective_tier"], "daily");
        Ok(())
    })
    .unwrap();
}

#[test]
fn bulk_escalation_to_construction_requires_step_up() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let err = dispatch(
            conn,
            &catalog(),
            request(
                "fake.write",
                &["a", "b"],
                false,
                actor(ActorKind::LocalCli, Tier::Construction),
                false,
            ),
        )
        .unwrap_err();

        assert!(matches!(err, OpError::StepUpRequired));
        let detail = latest_r14(conn);
        assert_eq!(detail["result"], "error:step_up_required");
        assert_eq!(detail["effective_tier"], "construction");
        Ok(())
    })
    .unwrap();
}

#[test]
fn ai_token_issue_dry_run_rejects_ceiling_above_routine() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let err = dispatch(
            conn,
            standard_catalog(),
            standard_request(
                "operator_token.issue",
                json!({
                    "name": "ai-daily",
                    "kind": "ai",
                    "tier_ceiling": "daily"
                }),
                true,
                actor(ActorKind::LocalCli, Tier::Construction),
                true,
            ),
        )
        .unwrap_err();

        assert!(
            matches!(err, OpError::Validation(message) if message == "ai token tier ceiling cannot exceed routine")
        );
        let detail = latest_r14(conn);
        assert_eq!(detail["result"], "error:validation");
        Ok(())
    })
    .unwrap();
}

#[test]
fn retire_empty_targets_is_validation_error() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let err = dispatch(
            conn,
            standard_catalog(),
            standard_request(
                "device.retire",
                json!({ "system_ids": [] }),
                true,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap_err();

        assert!(matches!(err, OpError::Validation(message) if message == "empty targets"));
        let detail = latest_r14(conn);
        assert_eq!(detail["result"], "error:validation");
        Ok(())
    })
    .unwrap();
}

#[test]
fn unknown_op_returns_not_found_without_audit() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let before = r14_count(conn);

        let err = dispatch(
            conn,
            &catalog(),
            request(
                "fake.missing",
                &["missing"],
                false,
                actor(ActorKind::LocalCli, Tier::Construction),
                true,
            ),
        )
        .unwrap_err();

        assert!(matches!(err, OpError::NotFound));
        assert_eq!(r14_count(conn), before);
        Ok(())
    })
    .unwrap();
}

#[test]
fn sighting_pin_is_typed_dry_runnable_and_preserves_evictable_reserve() {
    let mut migrations = all_migrations();
    migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    migrations.sort_by_key(|migration| migration.version);
    let db = iotkit_core_storage::init_db_memory(&migrations).unwrap();
    db.with_conn_sync(|conn| {
        let limits = iotkit_core_timeseries::StagingLimits::default();
        iotkit_core_timeseries::stage_sighting_at(
            conn,
            "principal:official",
            "subject-a",
            10,
            "{}",
            limits,
        )
        .unwrap();
        let params = json!({
            "principal_id":"principal:official",
            "staging_subject":"subject-a",
            "pinned":true
        });
        let result = dispatch(
            conn,
            standard_catalog(),
            standard_request(
                "device.sighting_pin",
                params.clone(),
                true,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap();
        assert_eq!(result["would"], "set_sighting_pin");
        assert!(!conn.query_row(
            "SELECT pinned FROM staged_readings WHERE hardware_id='subject-a'",
            [],
            |row| row.get::<_, bool>(0),
        )?);

        dispatch(
            conn,
            standard_catalog(),
            standard_request(
                "device.sighting_pin",
                params,
                false,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap();
        assert!(conn.query_row(
            "SELECT pinned FROM staged_readings WHERE hardware_id='subject-a'",
            [],
            |row| row.get::<_, bool>(0),
        )?);

        conn.execute_batch(
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<743)
             INSERT INTO staged_readings
                 (hardware_id,received_at,payload_json,principal_id,payload_bytes,pinned)
             SELECT printf('pinned-%04d',x),x,'{}','principal:official',2,1 FROM n;
             INSERT INTO staged_readings
                 (hardware_id,received_at,payload_json,principal_id,payload_bytes,pinned)
             VALUES('reserve-candidate',1000,'{}','principal:official',2,0);",
        )?;
        let reserve_error = dispatch(
            conn,
            standard_catalog(),
            standard_request(
                "device.sighting_pin",
                json!({
                    "principal_id":"principal:official",
                    "staging_subject":"reserve-candidate",
                    "pinned":true
                }),
                false,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap_err();
        assert!(matches!(reserve_error, OpError::PreconditionFailed(message) if message.contains("evictable reserve")));
        Ok(())
    })
    .unwrap();
}
