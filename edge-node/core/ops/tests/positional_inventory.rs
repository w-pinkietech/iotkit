use iotkit_core_ledger::{
    DeviceKind, DeviceState, NewDevice, current_generation, find_alive_by_hardware_id,
    insert_device, list_devices, positional_model_id,
};
use iotkit_core_ops::{
    Actor, ActorKind, DispatchRequest, OpError, POSITIONAL_INVENTORY_RECONCILE_OP, Tier, dispatch,
    standard_catalog,
};
use iotkit_core_storage::Migration;
use serde_json::{Value, json};

fn all_migrations() -> Vec<Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.sort_by_key(|migration| migration.version);
    all
}

fn system_request(devices: Value) -> DispatchRequest {
    DispatchRequest {
        op: POSITIONAL_INVENTORY_RECONCILE_OP.into(),
        params: json!({ "devices": devices }),
        dry_run: false,
        actor: Actor {
            actor_id: "system:iotkit-edge".into(),
            actor_kind: ActorKind::System,
            tier_ceiling: Tier::Daily,
        },
        source: Some("input_adapter_inventory".into()),
        step_up_verified: false,
        clock_trust: None,
    }
}

fn device(hardware_id: &str, user_label: &str) -> Value {
    json!({
        "hardware_id": hardware_id,
        "model_id": "test-model",
        "user_label": user_label,
    })
}

fn device_with_model(hardware_id: &str, model_id: &str, user_label: &str) -> Value {
    json!({
        "hardware_id": hardware_id,
        "model_id": model_id,
        "user_label": user_label,
    })
}

#[test]
fn system_reconcile_creates_one_atomic_batch_with_audit_and_generation() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();

    db.with_conn_sync(|conn| {
        let before = current_generation(conn).unwrap();
        let result = dispatch(
            conn,
            standard_catalog(),
            system_request(json!([
                device("input:rpi:one:i2c:0x60", "MCP9600 thermocouple"),
                device("input:rpi:one:i2c:0x44", "OPT3001 illuminance"),
            ])),
        )
        .unwrap()
        .into_public()
        .unwrap();

        assert_eq!(result["created"].as_array().unwrap().len(), 2);
        assert_eq!(result["existing"].as_array().unwrap().len(), 0);
        assert_eq!(current_generation(conn).unwrap(), before + 1);

        let rows = list_devices(conn, false).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| row.kind == DeviceKind::Positional && row.state == DeviceState::Active)
        );

        let detail: String = conn
            .query_row(
                "SELECT detail FROM ledger_events
                 WHERE kind = 'r14_op'
                 ORDER BY event_id DESC
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let audit: Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(audit["op"], POSITIONAL_INVENTORY_RECONCILE_OP);
        assert_eq!(audit["actor"], "system:iotkit-edge");
        assert_eq!(audit["actor_kind"], "system");
        assert_eq!(audit["result"], "ok");
        Ok(())
    })
    .unwrap();
}

#[test]
fn repeated_reconcile_reuses_the_existing_system_id() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    let hardware_id = "input:rpi:one:i2c:0x60";

    db.with_conn_sync(|conn| {
        let first = dispatch(
            conn,
            standard_catalog(),
            system_request(json!([device(hardware_id, "original label")])),
        )
        .unwrap()
        .into_public()
        .unwrap();
        let first_id = first["created"][0]["system_id"]
            .as_str()
            .unwrap()
            .to_owned();

        let second = dispatch(
            conn,
            standard_catalog(),
            system_request(json!([device(hardware_id, "new default label")])),
        )
        .unwrap()
        .into_public()
        .unwrap();

        assert_eq!(second["created"].as_array().unwrap().len(), 0);
        assert_eq!(second["existing"][0]["system_id"], first_id);
        let rows = list_devices(conn, false).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].system_id.to_text(), first_id);
        assert_eq!(rows[0].user_label.as_deref(), Some("original label"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn positional_inventory_rejects_non_canonical_model_before_writing() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    let hardware_id = "input:rpi:one:i2c:0x60";

    db.with_conn_sync(|conn| {
        let error = dispatch(
            conn,
            standard_catalog(),
            system_request(json!([device_with_model(
                hardware_id,
                "Model ID",
                "invalid model",
            )])),
        )
        .unwrap_err();
        assert!(matches!(error, OpError::Validation(_)));
        assert!(
            find_alive_by_hardware_id(conn, hardware_id)
                .unwrap()
                .is_none()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn positional_inventory_rejects_a_model_change_at_the_same_locator_atomically() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    let hardware_id = "input:rpi:one:i2c:0x60";
    let would_be_new = "input:rpi:one:i2c:0x44";

    db.with_conn_sync(|conn| {
        dispatch(
            conn,
            standard_catalog(),
            system_request(json!([device_with_model(
                hardware_id,
                "mcp9600",
                "MCP9600 thermocouple",
            )])),
        )
        .unwrap()
        .into_public()
        .unwrap();

        let repeated = dispatch(
            conn,
            standard_catalog(),
            system_request(json!([device_with_model(
                hardware_id,
                "mcp9600",
                "MCP9600 thermocouple",
            )])),
        )
        .unwrap()
        .into_public()
        .unwrap();
        assert_eq!(repeated["existing"].as_array().unwrap().len(), 1);

        let error = dispatch(
            conn,
            standard_catalog(),
            system_request(json!([
                device_with_model(would_be_new, "opt3001", "must not be inserted"),
                device_with_model(hardware_id, "opt3001", "wrong replacement"),
            ])),
        )
        .unwrap_err();
        assert!(
            matches!(error, OpError::PreconditionFailed(ref code) if code == "positional_inventory_model_conflict")
        );
        assert!(
            find_alive_by_hardware_id(conn, would_be_new)
                .unwrap()
                .is_none()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn first_reconcile_binds_the_model_to_a_preexisting_positional_device() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    let hardware_id = "input:rpi:one:i2c:0x60";

    db.with_conn_sync(|conn| {
        let system_id = insert_device(
            conn,
            &NewDevice {
                hardware_id: hardware_id.into(),
                user_label: Some("legacy positional device".into()),
                parent: None,
                kind: DeviceKind::Positional,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        assert_eq!(positional_model_id(conn, &system_id).unwrap(), None);

        dispatch(
            conn,
            standard_catalog(),
            system_request(json!([device_with_model(
                hardware_id,
                "mcp9600",
                "MCP9600 thermocouple",
            )])),
        )
        .unwrap()
        .into_public()
        .unwrap();

        assert_eq!(
            positional_model_id(conn, &system_id).unwrap().as_deref(),
            Some("mcp9600")
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn omitted_inventory_entry_is_not_silently_retired_or_reused() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    let retained = "input:rpi:one:i2c:0x60";
    let omitted = "input:rpi:one:i2c:0x44";

    db.with_conn_sync(|conn| {
        dispatch(
            conn,
            standard_catalog(),
            system_request(json!([
                device_with_model(retained, "mcp9600", "MCP9600 thermocouple"),
                device_with_model(omitted, "opt3001", "OPT3001 illuminance"),
            ])),
        )
        .unwrap()
        .into_public()
        .unwrap();
        let omitted_system_id = find_alive_by_hardware_id(conn, omitted)
            .unwrap()
            .unwrap()
            .system_id;

        dispatch(
            conn,
            standard_catalog(),
            system_request(json!([device_with_model(
                retained,
                "mcp9600",
                "MCP9600 thermocouple",
            )])),
        )
        .unwrap()
        .into_public()
        .unwrap();

        let omitted_after = find_alive_by_hardware_id(conn, omitted).unwrap().unwrap();
        assert_eq!(omitted_after.system_id, omitted_system_id);
        assert_eq!(omitted_after.state, DeviceState::Active);
        assert_eq!(
            positional_model_id(conn, &omitted_system_id)
                .unwrap()
                .as_deref(),
            Some("opt3001")
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn kind_conflict_rejects_the_whole_batch_without_partial_inserts() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    let conflict_id = "input:rpi:one:i2c:0x60";
    let would_be_new = "input:rpi:one:i2c:0x44";

    db.with_conn_sync(|conn| {
        insert_device(
            conn,
            &NewDevice {
                hardware_id: conflict_id.into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        let generation_before = current_generation(conn).unwrap();

        let error = dispatch(
            conn,
            standard_catalog(),
            system_request(json!([
                device(would_be_new, "must not be inserted"),
                device(conflict_id, "conflicting kind"),
            ])),
        )
        .unwrap_err();

        assert!(
            matches!(error, OpError::PreconditionFailed(ref code) if code == "positional_inventory_kind_conflict")
        );
        assert!(
            find_alive_by_hardware_id(conn, would_be_new)
                .unwrap()
                .is_none()
        );
        assert_eq!(current_generation(conn).unwrap(), generation_before);

        let detail: String = conn
            .query_row(
                "SELECT detail FROM ledger_events
                 WHERE kind = 'r14_op'
                 ORDER BY event_id DESC
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let audit: Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(audit["result"], "error:precondition_failed");
        Ok(())
    })
    .unwrap();
}

#[test]
fn positional_inventory_operation_rejects_non_system_actor() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();

    db.with_conn_sync(|conn| {
        let mut request = system_request(json!([device("input:rpi:one:i2c:0x60", "sensor")]));
        request.actor = Actor {
            actor_id: "local_cli".into(),
            actor_kind: ActorKind::LocalCli,
            tier_ceiling: Tier::Construction,
        };
        request.step_up_verified = true;

        assert!(matches!(
            dispatch(conn, standard_catalog(), request),
            Err(OpError::Forbidden(ref code)) if code == "system_actor_required"
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn system_actor_cannot_dispatch_unrelated_operations() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();

    db.with_conn_sync(|conn| {
        let mut request = system_request(json!([]));
        request.op = "device.retire".into();
        request.params = json!({ "system_ids": [] });

        assert!(matches!(
            dispatch(conn, standard_catalog(), request),
            Err(OpError::Forbidden(ref code)) if code == "system_actor_scope"
        ));
        Ok(())
    })
    .unwrap();
}
