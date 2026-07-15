use iotkit_core_ledger::{
    DeviceKind, DeviceState, NewDevice, SystemId, get_device, insert_device, record_sighting,
};
use iotkit_core_ops::{
    Actor, ActorKind, DispatchRequest, OpError, Tier, dispatch, standard_catalog,
};
use iotkit_core_storage::Migration;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

fn all_migrations() -> Vec<Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    all
}

fn actor(kind: ActorKind, ceiling: Tier) -> Actor {
    Actor {
        actor_id: match kind {
            ActorKind::Human => "tok_human".to_string(),
            ActorKind::Ai => "tok_ai".to_string(),
            ActorKind::LocalCli => "local_cli".to_string(),
        },
        actor_kind: kind,
        tier_ceiling: ceiling,
    }
}

fn request(op: &str, params: Value, dry_run: bool, actor: Actor, step_up: bool) -> DispatchRequest {
    DispatchRequest {
        op: op.to_string(),
        params,
        dry_run,
        actor,
        source: Some("test-suite".to_string()),
        step_up_verified: step_up,
        clock_trust: None,
    }
}

fn enable_temperature(conn: &Connection) {
    let catalog = iotkit_core_registry::standard_catalog();
    iotkit_core_registry::enable_entry(
        conn,
        catalog.find("temperature_c").unwrap(),
        &catalog.catalog_version,
        "test",
    )
    .unwrap();
}

fn active_device(conn: &Connection, hw: &str) -> SystemId {
    insert_device(
        conn,
        &NewDevice {
            hardware_id: hw.to_string(),
            user_label: None,
            parent: None,
            kind: DeviceKind::Individual,
            initial_state: DeviceState::Active,
        },
    )
    .unwrap()
}

fn token_id(conn: &Connection, name: &str) -> String {
    let out = dispatch(
        conn,
        standard_catalog(),
        request(
            "operator_token.issue",
            json!({
                "name": name,
                "kind": "human",
                "tier_ceiling": "routine",
                "expires_at": null,
            }),
            false,
            actor(ActorKind::LocalCli, Tier::Construction),
            true,
        ),
    )
    .unwrap();
    out["token_id"].as_str().unwrap().to_string()
}

fn matrix_params(conn: &Connection, op: &str, bulk: bool) -> Value {
    match (op, bulk) {
        ("registry.resolve_unknown_key", false) => {
            enable_temperature(conn);
            json!({"key":"temp","resolution":{"alias_to":"temperature_c"}})
        }
        ("exit.commissioning_smoke.enqueue", false) => {
            iotkit_core_publish::store::target_insert(
                conn,
                &iotkit_core_publish::store::TargetRow {
                    target_id: "site".into(),
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
            json!({})
        }
        ("device.approve_sighting", false) => {
            record_sighting(conn, "rpi-local:default:i2c:0x60", "test").unwrap();
            json!({"hardware_ids":["rpi-local:default:i2c:0x60"]})
        }
        ("device.approve_sighting", true) => {
            record_sighting(conn, "rpi-local:default:i2c:0x60", "test").unwrap();
            record_sighting(conn, "rpi-local:default:i2c:0x61", "test").unwrap();
            json!({"hardware_ids":["rpi-local:default:i2c:0x60","rpi-local:default:i2c:0x61"]})
        }
        ("device.retire", false) => {
            let sid = active_device(conn, "rpi-local:default:i2c:0x62");
            json!({"system_ids":[sid.to_text()]})
        }
        ("device.retire", true) => {
            let a = active_device(conn, "rpi-local:default:i2c:0x62");
            let b = active_device(conn, "rpi-local:default:i2c:0x63");
            json!({"system_ids":[a.to_text(), b.to_text()]})
        }
        ("operator_token.issue", false) => {
            json!({"name":"ai-harness","kind":"ai","tier_ceiling":"routine","expires_at":null})
        }
        ("operator_token.revoke", false) => {
            let id = token_id(conn, "revoke-one");
            json!({"token_ids":[id]})
        }
        ("operator_token.revoke", true) => {
            let a = token_id(conn, "revoke-a");
            let b = token_id(conn, "revoke-b");
            json!({"token_ids":[a, b]})
        }
        other => panic!("unsupported matrix case: {other:?}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expected {
    Ok,
    Forbidden,
    StepUpRequired,
}

const MATRIX_CASES: &[(&str, bool, Tier)] = &[
    ("registry.resolve_unknown_key", false, Tier::Daily),
    ("exit.commissioning_smoke.enqueue", false, Tier::Daily),
    ("device.approve_sighting", false, Tier::Daily),
    ("device.approve_sighting", true, Tier::Construction),
    ("device.retire", false, Tier::Daily),
    ("device.retire", true, Tier::Construction),
    ("operator_token.issue", false, Tier::Construction),
    ("operator_token.revoke", false, Tier::Daily),
    ("operator_token.revoke", true, Tier::Construction),
];

const CEILINGS: &[Tier] = &[
    Tier::ReadOnly,
    Tier::Routine,
    Tier::Daily,
    Tier::Construction,
];

fn expected(ceiling: Tier, effective_tier: Tier, step_up: bool) -> Expected {
    if ceiling < effective_tier {
        return Expected::Forbidden;
    }
    if effective_tier == Tier::Construction && !step_up {
        return Expected::StepUpRequired;
    }
    Expected::Ok
}

#[test]
fn standard_catalog_enforces_tier_matrix_for_single_and_bulk_shapes() {
    for &(op, bulk, effective_tier) in MATRIX_CASES {
        for &ceiling in CEILINGS {
            for step_up in [false, true] {
                let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
                db.with_conn_sync(|conn| {
                    let params = matrix_params(conn, op, bulk);
                    let result = dispatch(
                        conn,
                        standard_catalog(),
                        request(
                            op,
                            params,
                            true,
                            actor(ActorKind::LocalCli, ceiling),
                            step_up,
                        ),
                    );
                    match expected(ceiling, effective_tier, step_up) {
                        Expected::Ok => {
                            let value = result.unwrap();
                            if op == "operator_token.issue" {
                                assert!(value.get("plaintext").is_none());
                            }
                        }
                        Expected::Forbidden => {
                            assert!(matches!(result, Err(OpError::Forbidden(_))));
                        }
                        Expected::StepUpRequired => {
                            assert!(matches!(result, Err(OpError::StepUpRequired)));
                        }
                    }
                    Ok(())
                })
                .unwrap();
            }
        }
    }
}

#[test]
fn resolve_unknown_key_alias_branch_defines_site_mapping_alias() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        enable_temperature(conn);
        dispatch(
            conn,
            standard_catalog(),
            request(
                "registry.resolve_unknown_key",
                json!({"key":"temp","resolution":{"alias_to":"temperature_c"}}),
                false,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap();

        let target: String = conn
            .query_row(
                "SELECT measurement_key FROM registry_aliases WHERE alias = 'temp'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(target, "temperature_c");
        Ok(())
    })
    .unwrap();
}

#[test]
fn resolve_unknown_key_custom_branch_defines_custom_entry_and_alias() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        dispatch(
            conn,
            standard_catalog(),
            request(
                "registry.resolve_unknown_key",
                json!({
                    "key":"tank_temp",
                    "resolution":{"custom":{
                        "measurement_key":"custom.tank_temp",
                        "unit_ucum":"Cel",
                        "unit_display":"C",
                        "value_type":"float",
                        "semantic_class":"sensor",
                        "channel_mode":"single",
                        "channel_roles":[],
                        "physical_min":null,
                        "physical_max":null
                    }}
                }),
                false,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap();

        let origin: String = conn
            .query_row(
                "SELECT origin FROM registry_entries WHERE measurement_key = 'custom.tank_temp'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let target: String = conn
            .query_row(
                "SELECT measurement_key FROM registry_aliases WHERE alias = 'tank_temp'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(origin, "custom");
        assert_eq!(target, "custom.tank_temp");
        Ok(())
    })
    .unwrap();
}

#[test]
fn resolve_unknown_key_rejects_ambiguous_resolution_shape() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        enable_temperature(conn);
        let both = dispatch(
            conn,
            standard_catalog(),
            request(
                "registry.resolve_unknown_key",
                json!({
                    "key":"temp",
                    "resolution":{
                        "alias_to":"temperature_c",
                        "custom":{
                            "measurement_key":"custom.temp",
                            "unit_ucum":"Cel",
                            "unit_display":"C",
                            "value_type":"float",
                            "semantic_class":"sensor",
                            "channel_mode":"single"
                        }
                    }
                }),
                true,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        );
        assert!(matches!(both, Err(OpError::Validation(_))));

        let neither = dispatch(
            conn,
            standard_catalog(),
            request(
                "registry.resolve_unknown_key",
                json!({"key":"temp","resolution":{}}),
                true,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        );
        assert!(matches!(neither, Err(OpError::Validation(_))));
        Ok(())
    })
    .unwrap();
}

#[test]
fn resolve_unknown_key_dry_run_validates_custom_specs_like_execute() {
    let invalid_cases = [
        json!({
            "key":"tank_temp",
            "resolution":{"custom":{
                "measurement_key":"tank_temp",
                "value_type":"float",
                "semantic_class":"sensor",
                "channel_mode":"single"
            }}
        }),
        json!({
            "key":"tank_temp",
            "resolution":{"custom":{
                "measurement_key":"custom.tank_temp",
                "value_type":"float",
                "semantic_class":"sensor",
                "channel_mode":"single",
                "physical_min":0.0
            }}
        }),
        json!({
            "key":"vector",
            "resolution":{"custom":{
                "measurement_key":"custom.vector",
                "value_type":"float",
                "semantic_class":"sensor",
                "channel_mode":"fixed",
                "channel_roles":[]
            }}
        }),
        json!({
            "key":"Bad:Alias",
            "resolution":{"custom":{
                "measurement_key":"custom.tank_temp",
                "value_type":"float",
                "semantic_class":"sensor",
                "channel_mode":"single"
            }}
        }),
    ];

    for params in invalid_cases {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let result = dispatch(
                conn,
                standard_catalog(),
                request(
                    "registry.resolve_unknown_key",
                    params.clone(),
                    true,
                    actor(ActorKind::LocalCli, Tier::Daily),
                    false,
                ),
            );
            assert!(
                matches!(
                    result,
                    Err(OpError::Validation(_) | OpError::PreconditionFailed(_))
                ),
                "expected validation/precondition failure for {params:?}, got {result:?}"
            );
            Ok(())
        })
        .unwrap();
    }
}

#[test]
fn resolve_unknown_key_dry_run_validates_alias_keys_like_execute() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        enable_temperature(conn);
        let bad_alias = dispatch(
            conn,
            standard_catalog(),
            request(
                "registry.resolve_unknown_key",
                json!({"key":"Bad:Alias","resolution":{"alias_to":"temperature_c"}}),
                true,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        );
        assert!(matches!(bad_alias, Err(OpError::Validation(_))));

        let bad_target = dispatch(
            conn,
            standard_catalog(),
            request(
                "registry.resolve_unknown_key",
                json!({"key":"temp","resolution":{"alias_to":"Bad:Target"}}),
                true,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        );
        assert!(matches!(bad_target, Err(OpError::Validation(_))));
        Ok(())
    })
    .unwrap();
}

#[test]
fn approve_sighting_keeps_device_quarantined_and_keeps_domain_audit() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        record_sighting(conn, "rpi-local:default:i2c:0x60", "test").unwrap();
        let out = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.approve_sighting",
                json!({"hardware_ids":["rpi-local:default:i2c:0x60"]}),
                false,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap();
        let sid = SystemId::from_text(out["approved"][0]["system_id"].as_str().unwrap()).unwrap();
        let row = get_device(conn, &sid).unwrap().unwrap();
        assert_eq!(row.state, DeviceState::Quarantined);

        let sighting_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind = 'sighting_approved'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let activated_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind = 'device_activated'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let r14_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind = 'r14_op'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sighting_events, 1);
        assert_eq!(activated_events, 0);
        assert_eq!(r14_events, 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn approve_sighting_dry_run_reports_quarantine_allocation() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        record_sighting(conn, "rpi-local:default:i2c:0x60", "test").unwrap();
        let out = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.approve_sighting",
                json!({"hardware_ids":["rpi-local:default:i2c:0x60"]}),
                true,
                actor(ActorKind::LocalCli, Tier::Daily),
                false,
            ),
        )
        .unwrap();
        assert_eq!(out["would"], "approve_sighting_as_quarantined");
        Ok(())
    })
    .unwrap();
}

#[test]
fn retire_bulk_failure_rolls_back_all_devices() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let active = active_device(conn, "rpi-local:default:i2c:0x62");
        let retired = active_device(conn, "rpi-local:default:i2c:0x63");
        iotkit_core_ledger::retire_device(conn, &retired).unwrap();

        let result = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.retire",
                json!({"system_ids":[active.to_text(), retired.to_text()]}),
                false,
                actor(ActorKind::LocalCli, Tier::Construction),
                true,
            ),
        );
        assert!(matches!(result, Err(OpError::NotFound)));
        assert_eq!(
            get_device(conn, &active).unwrap().unwrap().state,
            DeviceState::Active
        );
        assert_eq!(
            get_device(conn, &retired).unwrap().unwrap().state,
            DeviceState::Retired
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn ai_token_cannot_dispatch_construction_token_issue() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let issued = dispatch(
            conn,
            standard_catalog(),
            request(
                "operator_token.issue",
                json!({"name":"ai-harness","kind":"ai","tier_ceiling":"routine","expires_at":null}),
                false,
                actor(ActorKind::LocalCli, Tier::Construction),
                true,
            ),
        )
        .unwrap();
        let ai_actor = Actor {
            actor_id: issued["token_id"].as_str().unwrap().to_string(),
            actor_kind: ActorKind::Ai,
            tier_ceiling: Tier::Routine,
        };
        assert_eq!(ai_actor.actor_kind, ActorKind::Ai);
        assert_eq!(ai_actor.tier_ceiling, Tier::Routine);

        let err = dispatch(
            conn,
            standard_catalog(),
            request(
                "operator_token.issue",
                json!({"name":"nested","kind":"human","tier_ceiling":"routine","expires_at":null}),
                false,
                ai_actor,
                true,
            ),
        )
        .unwrap_err();
        assert!(matches!(err, OpError::Forbidden(_)));
        Ok(())
    })
    .unwrap();
}

struct RollbackClock(AtomicI64);

impl iotkit_core_ops::Clock for RollbackClock {
    fn wall_time_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }

    fn monotonic_ms(&self) -> u64 {
        0
    }

    fn kernel_synchronized(&self) -> bool {
        true
    }
}

#[test]
fn middleware_auth_then_clock_rollback_is_rechecked_inside_dispatch() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let clock = Arc::new(RollbackClock(AtomicI64::new(5_000)));
        let trust = Arc::new(
            iotkit_core_ops::ClockTrust::load(
                conn,
                clock.clone(),
                Duration::from_millis(10),
                Duration::from_secs(60),
            )
            .unwrap(),
        );
        let issued = iotkit_core_ops::issue_session_token(
            conn,
            "finite",
            Tier::Construction,
            60_000,
            "test",
            None,
            &trust,
        )
        .unwrap();
        let middleware_actor =
            iotkit_core_ops::authenticate(conn, issued.plaintext.expose(), &trust)
                .unwrap()
                .unwrap();
        assert_eq!(
            iotkit_core_ops::ClockTrust::persisted_floor(conn).unwrap(),
            5_000
        );

        clock.0.store(1_000, Ordering::SeqCst);
        let result = dispatch(
            conn,
            standard_catalog(),
            DispatchRequest {
                op: "operator_token.issue".to_string(),
                params: json!({
                    "name": "must-not-exist",
                    "kind": "human",
                    "tier_ceiling": "routine",
                    "expires_at": null,
                }),
                dry_run: false,
                actor: middleware_actor,
                source: Some("test-suite".to_string()),
                step_up_verified: true,
                clock_trust: Some(trust),
            },
        );
        assert_eq!(
            result,
            Err(OpError::Forbidden("clock_untrusted".to_string()))
        );
        let created: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM operator_tokens WHERE name = 'must-not-exist'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(created, 0);
        assert_eq!(
            iotkit_core_ops::ClockTrust::persisted_floor(conn).unwrap(),
            5_000
        );
        Ok(())
    })
    .unwrap();
}
