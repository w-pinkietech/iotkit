use iotkit_core_ops::{
    Actor, ActorKind, DispatchRequest, OpError, Tier, dispatch, standard_catalog,
};
use iotkit_core_publish::store::{TargetRow, select_batch, target_insert};
use serde_json::json;

fn migrations() -> Vec<iotkit_core_storage::Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.sort_by_key(|migration| migration.version);
    all
}

fn request(dry_run: bool) -> DispatchRequest {
    DispatchRequest {
        op: "exit.commissioning_smoke.enqueue".into(),
        params: json!({}),
        dry_run,
        actor: Actor {
            actor_id: "local_cli".into(),
            actor_kind: ActorKind::LocalCli,
            tier_ceiling: Tier::Construction,
        },
        source: Some("local_cli".into()),
        step_up_verified: false,
        clock_trust: None,
    }
}

#[test]
fn commissioning_smoke_requires_initialized_mqtt_edge_target() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    db.with_conn_sync(|conn| {
        iotkit_core_ledger::ledger_epoch(conn).unwrap();
        let error = dispatch(conn, standard_catalog(), request(false)).unwrap_err();
        assert!(matches!(error, OpError::PreconditionFailed(_)));
        assert!(
            select_batch(
                conn,
                &iotkit_core_ledger::ledger_epoch(conn).unwrap(),
                0,
                10
            )
            .unwrap()
            .is_empty()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn commissioning_smoke_dispatch_enqueues_audited_test_record() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
        target_insert(
            conn,
            &TargetRow {
                target_id: "edge".into(),
                endpoint_url: "mqtt://broker:1883".into(),
                credential_token: String::new(),
                archive_responsible: true,
                schema_version: 1,
                cursor_epoch: None,
                cursor_pub_seq: 0,
            },
            1,
        )
        .unwrap();

        let dry_run = dispatch(conn, standard_catalog(), request(true)).unwrap();
        assert_eq!(dry_run["would"], "enqueue_commissioning_smoke");
        assert!(select_batch(conn, &epoch, 0, 10).unwrap().is_empty());

        let result = dispatch(conn, standard_catalog(), request(false)).unwrap();
        let test_id = result["test_id"].as_str().unwrap();
        assert!(test_id.starts_with("smoke-"));
        assert_eq!(result["target_id"], "edge");
        assert_eq!(result["ledger_epoch"], epoch);
        assert!(result["pub_seq"].as_i64().unwrap() > 0);

        let rows = select_batch(conn, &epoch, 0, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "commissioning_smoke");
        assert!(rows[0].annotation_json.as_deref().unwrap().contains(test_id));
        let audits: i64 = conn
            .query_row(
                "SELECT count(*) FROM ledger_events WHERE kind='r14_op' AND detail LIKE '%exit.commissioning_smoke.enqueue%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 2, "dry-run and execute are both audited");
        Ok(())
    })
    .unwrap();
}

#[test]
fn commissioning_smoke_rejects_a_discovery_only_edge() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
        iotkit_core_publish::activation::install_edge_target(
            conn,
            &TargetRow {
                target_id: "edge".into(),
                endpoint_url: "mqtts://broker.example.test:8883".into(),
                credential_token: String::new(),
                archive_responsible: true,
                schema_version: 1,
                cursor_epoch: None,
                cursor_pub_seq: 0,
            },
            1,
        )
        .unwrap();

        assert!(dispatch(conn, standard_catalog(), request(false)).is_err());
        assert!(select_batch(conn, &epoch, 0, 10).unwrap().is_empty());
        Ok(())
    })
    .unwrap();
}
