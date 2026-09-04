use iotkit_core_ops::{
    Actor, ActorKind, DispatchRequest, OpError, Tier, dispatch, standard_catalog,
};
use iotkit_core_pipeline::{PipelineEngine, outbox, store};
use iotkit_core_storage::Migration;
use rusqlite::Connection;
use serde_json::{Value, json};

fn all_migrations() -> Vec<Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.extend_from_slice(iotkit_core_pipeline::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    all
}

fn local_cli() -> Actor {
    Actor {
        actor_id: "local_cli".to_string(),
        actor_kind: ActorKind::LocalCli,
        tier_ceiling: Tier::Construction,
    }
}

fn run(conn: &Connection, op: &str, params: Value) -> Result<Value, OpError> {
    run_with_step_up(conn, op, params, false)
}

fn run_with_step_up(
    conn: &Connection,
    op: &str,
    params: Value,
    step_up_verified: bool,
) -> Result<Value, OpError> {
    dispatch(
        conn,
        standard_catalog(),
        DispatchRequest {
            op: op.to_string(),
            params,
            dry_run: false,
            actor: local_cli(),
            source: Some("test".to_string()),
            step_up_verified,
            clock_trust: None,
        },
    )
    .and_then(|result| result.into_public())
}

fn count_definition(id: &str) -> Value {
    json!({
        "id": id,
        "kind": "accumulated-count",
        "input": { "adapter": "trial_sample", "measurement_key": "contact_state" },
        "trigger": "on-transition",
        "detector": { "mode": "high-active", "rise_threshold": 0.5, "fall_threshold": 0.5 },
    })
}

fn recorded_node(conn: &Connection) {
    PipelineEngine::new("rpi1".parse().unwrap())
        .reconcile(conn, 0)
        .unwrap();
}

#[test]
fn pipeline_operations_require_a_recorded_edge_node_id() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let error = run(
            conn,
            "pipeline.create",
            json!({ "definition": count_definition("press-01") }),
        )
        .unwrap_err();
        assert!(matches!(error, OpError::PreconditionFailed(message) if message.contains("edge-node-id")));
        assert!(store::list_definitions(conn).unwrap().is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn create_update_reset_and_delete_go_through_the_dispatcher() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        recorded_node(conn);

        let created = run(
            conn,
            "pipeline.create",
            json!({ "definition": count_definition("press-01") }),
        )
        .unwrap();
        assert_eq!(created["id"], "press-01");
        assert_eq!(created["published_sequence"], 1);
        let first_series = created["series_id"].as_str().unwrap().to_owned();
        assert_eq!(outbox::count(conn).unwrap(), 1);

        let duplicate = run(
            conn,
            "pipeline.create",
            json!({ "definition": count_definition("press-01") }),
        )
        .unwrap_err();
        assert!(matches!(duplicate, OpError::PreconditionFailed(_)));

        let mut tuned = count_definition("press-01");
        tuned["display_name"] = json!("Press 01");
        let updated = run(conn, "pipeline.update", json!({ "definition": tuned })).unwrap();
        assert!(updated["new_series"].is_null(), "tuning keeps the series");

        let mut restructured = count_definition("press-01");
        restructured["input"]["channel_index"] = json!(1);
        let updated = run(
            conn,
            "pipeline.update",
            json!({ "definition": restructured }),
        )
        .unwrap();
        let second_series = updated["new_series"]["series_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(second_series, first_series);

        let mut other_kind = count_definition("press-01");
        other_kind["kind"] = json!("state");
        other_kind.as_object_mut().unwrap().remove("trigger");
        let error = run(conn, "pipeline.update", json!({ "definition": other_kind })).unwrap_err();
        assert!(matches!(error, OpError::PreconditionFailed(message) if message.contains("kind")));

        let reset = run(conn, "pipeline.reset", json!({ "id": "press-01" })).unwrap();
        assert_ne!(reset["series_id"], second_series);
        assert_eq!(reset["published_sequence"], 1);

        let deleted = run(conn, "pipeline.delete", json!({ "id": "press-01" })).unwrap();
        assert_eq!(deleted["deleted"], true);
        assert!(store::list_definitions(conn).unwrap().is_empty());
        let last = outbox::all(conn).unwrap().pop().unwrap();
        assert!(last.payload.is_empty(), "delete clears the retained value");

        assert!(matches!(
            run(conn, "pipeline.delete", json!({ "id": "press-01" })),
            Err(OpError::NotFound)
        ));
        assert!(matches!(
            run(conn, "pipeline.reset", json!({ "id": "Press-01" })),
            Err(OpError::Validation(_))
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn invalid_definitions_are_rejected_before_any_write() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        recorded_node(conn);
        let mut missing_trigger = count_definition("press-01");
        missing_trigger.as_object_mut().unwrap().remove("trigger");
        let error = run(
            conn,
            "pipeline.create",
            json!({ "definition": missing_trigger }),
        )
        .unwrap_err();
        assert!(matches!(error, OpError::Validation(message) if message.contains("trigger")));

        let mut unknown_field = count_definition("press-01");
        unknown_field["extra"] = json!(1);
        assert!(matches!(
            run(
                conn,
                "pipeline.create",
                json!({ "definition": unknown_field })
            ),
            Err(OpError::Validation(_))
        ));
        assert!(matches!(
            run(conn, "pipeline.create", json!({})),
            Err(OpError::Validation(_))
        ));
        assert!(store::list_definitions(conn).unwrap().is_empty());
        assert_eq!(outbox::count(conn).unwrap(), 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn import_replaces_every_definition_in_one_dispatch() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        recorded_node(conn);
        run(
            conn,
            "pipeline.create",
            json!({ "definition": count_definition("press-01") }),
        )
        .unwrap();
        let params =
            json!({ "pipelines": [count_definition("press-02"), count_definition("press-03")] });
        assert!(
            matches!(
                run(conn, "pipeline.import", params.clone()),
                Err(OpError::StepUpRequired)
            ),
            "import replaces everything, so it is a construction-tier operation"
        );
        let imported = run_with_step_up(conn, "pipeline.import", params, true).unwrap();
        assert_eq!(imported["imported"], 2);
        let ids: Vec<String> = store::list_definitions(conn)
            .unwrap()
            .iter()
            .map(|d| d.id.to_string())
            .collect();
        assert_eq!(ids, vec!["press-02", "press-03"]);
        let rows = outbox::all(conn).unwrap();
        assert!(
            rows.iter()
                .any(|row| row.payload.is_empty() && row.topic.contains("/press-01/"))
        );

        let error = run_with_step_up(
            conn,
            "pipeline.import",
            json!({ "pipelines": [count_definition("press-04"), count_definition("press-04")] }),
            true,
        )
        .unwrap_err();
        assert!(matches!(error, OpError::Validation(message) if message.contains("duplicate")));
        assert_eq!(
            store::list_definitions(conn).unwrap().len(),
            2,
            "rolled back"
        );
        Ok(())
    })
    .unwrap();
}
