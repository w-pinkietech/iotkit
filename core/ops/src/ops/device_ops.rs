use iotkit_core_ledger::{
    DeviceKind, DeviceState, NewDevice, SystemId, approve_sighting, find_alive_by_hardware_id,
    get_device, insert_device, retire_device,
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;

use crate::{ActorKind, OpContext, OpDescriptor, OpError, Tier};

use super::{required_string_array, target_string_array};

pub fn pin_sighting_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: "device.sighting_pin",
        tier: Tier::Daily,
        bulk_escalates: false,
        changes_state: true,
        params_schema: pin_schema,
        targets: |_| vec!["staging_sighting".into()],
        preconditions: pin_preconditions,
        dry_run: pin_dry_run,
        execute: pin_execute,
        secret_execute: None,
    }
}

fn pin_schema() -> Value {
    json!({ "required": ["principal_id", "staging_subject", "pinned"] })
}

fn pin_params<'a>(ctx: &'a OpContext<'_>) -> Result<(&'a str, &'a str, bool), OpError> {
    let principal = ctx
        .params
        .get("principal_id")
        .and_then(Value::as_str)
        .ok_or_else(|| OpError::Validation("principal_id must be a string".into()))?;
    let subject = ctx
        .params
        .get("staging_subject")
        .and_then(Value::as_str)
        .ok_or_else(|| OpError::Validation("staging_subject must be a string".into()))?;
    let pinned = ctx
        .params
        .get("pinned")
        .and_then(Value::as_bool)
        .ok_or_else(|| OpError::Validation("pinned must be boolean".into()))?;
    Ok((principal, subject, pinned))
}

fn pin_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    let (principal, subject, pinned) = pin_params(ctx)?;
    iotkit_core_timeseries::validate_sighting_pin(
        tx,
        principal,
        subject,
        pinned,
        iotkit_core_timeseries::StagingLimits::default(),
    )
    .map_err(|error| OpError::PreconditionFailed(error.to_string()))
}

fn pin_dry_run(_tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let (_, _, pinned) = pin_params(ctx)?;
    Ok(json!({ "would": "set_sighting_pin", "pinned": pinned }))
}

fn pin_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let (principal, subject, pinned) = pin_params(ctx)?;
    iotkit_core_timeseries::set_sighting_pin(
        tx,
        principal,
        subject,
        pinned,
        iotkit_core_timeseries::StagingLimits::default(),
    )
    .map_err(|error| OpError::PreconditionFailed(error.to_string()))?;
    Ok(json!({ "pinned": pinned }))
}

pub fn approve_sighting_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: "device.approve_sighting",
        tier: Tier::Daily,
        bulk_escalates: true,
        changes_state: true,
        params_schema: hardware_schema,
        targets: hardware_targets,
        preconditions: approve_preconditions,
        dry_run: approve_dry_run,
        execute: approve_execute,
        secret_execute: None,
    }
}

pub fn retire_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: "device.retire",
        tier: Tier::Daily,
        bulk_escalates: true,
        changes_state: true,
        params_schema: system_schema,
        targets: system_targets,
        preconditions: retire_preconditions,
        dry_run: retire_dry_run,
        execute: retire_execute,
        secret_execute: None,
    }
}

pub fn reconcile_positional_inventory_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: crate::POSITIONAL_INVENTORY_RECONCILE_OP,
        tier: Tier::Daily,
        bulk_escalates: false,
        changes_state: true,
        params_schema: positional_inventory_schema,
        targets: positional_inventory_targets,
        preconditions: positional_inventory_preconditions,
        dry_run: positional_inventory_dry_run,
        execute: positional_inventory_execute,
        secret_execute: None,
    }
}

#[derive(Debug)]
struct PositionalInventoryIntent {
    hardware_id: String,
    user_label: Option<String>,
}

fn positional_inventory_schema() -> Value {
    json!({ "required": ["devices"] })
}

fn positional_inventory_targets(params: &Value) -> Vec<String> {
    params
        .get("devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|device| device.get("hardware_id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn positional_inventory_intents(
    ctx: &OpContext<'_>,
) -> Result<Vec<PositionalInventoryIntent>, OpError> {
    let devices = ctx
        .params
        .get("devices")
        .and_then(Value::as_array)
        .ok_or_else(|| OpError::Validation("devices must be an array".into()))?;
    if devices.is_empty() {
        return Err(OpError::Validation(
            "positional inventory must not be empty".into(),
        ));
    }

    let mut seen = BTreeSet::new();
    devices
        .iter()
        .map(|device| {
            let device = device
                .as_object()
                .ok_or_else(|| OpError::Validation("devices entries must be objects".into()))?;
            if device
                .keys()
                .any(|key| !matches!(key.as_str(), "hardware_id" | "user_label"))
            {
                return Err(OpError::Validation(
                    "undeclared positional inventory field".into(),
                ));
            }
            let hardware_id = device
                .get("hardware_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    OpError::Validation("devices hardware_id must be a non-empty string".into())
                })?
                .to_owned();
            if !seen.insert(hardware_id.clone()) {
                return Err(OpError::Validation(
                    "duplicate positional inventory hardware_id".into(),
                ));
            }
            let user_label = match device.get("user_label") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) => Some(value.clone()),
                Some(_) => {
                    return Err(OpError::Validation(
                        "devices user_label must be a string or null".into(),
                    ));
                }
            };
            Ok(PositionalInventoryIntent {
                hardware_id,
                user_label,
            })
        })
        .collect()
}

fn positional_inventory_preconditions(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<(), OpError> {
    if ctx.actor_kind != ActorKind::System {
        return Err(OpError::Forbidden("system_actor_required".into()));
    }
    for intent in positional_inventory_intents(ctx)? {
        if let Some(existing) = find_alive_by_hardware_id(tx, &intent.hardware_id)?
            && existing.kind != DeviceKind::Positional
        {
            return Err(OpError::PreconditionFailed(
                "positional_inventory_kind_conflict".into(),
            ));
        }
    }
    Ok(())
}

fn positional_inventory_dry_run(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<Value, OpError> {
    let mut create = Vec::new();
    let mut existing = Vec::new();
    for intent in positional_inventory_intents(ctx)? {
        match find_alive_by_hardware_id(tx, &intent.hardware_id)? {
            Some(row) => existing.push(json!({
                "hardware_id": intent.hardware_id,
                "system_id": row.system_id.to_text(),
            })),
            None => create.push(intent.hardware_id),
        }
    }
    Ok(json!({
        "would": "reconcile_positional_inventory",
        "create": create,
        "existing": existing,
    }))
}

fn positional_inventory_execute(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<Value, OpError> {
    let mut created = Vec::new();
    let mut existing = Vec::new();
    for intent in positional_inventory_intents(ctx)? {
        if let Some(row) = find_alive_by_hardware_id(tx, &intent.hardware_id)? {
            existing.push(json!({
                "hardware_id": intent.hardware_id,
                "system_id": row.system_id.to_text(),
            }));
            continue;
        }
        let system_id = insert_device(
            tx,
            &NewDevice {
                hardware_id: intent.hardware_id.clone(),
                user_label: intent.user_label,
                parent: None,
                kind: DeviceKind::Positional,
                initial_state: DeviceState::Active,
            },
        )?;
        created.push(json!({
            "hardware_id": intent.hardware_id,
            "system_id": system_id.to_text(),
        }));
    }
    Ok(json!({
        "created": created,
        "existing": existing,
    }))
}

fn hardware_schema() -> Value {
    json!({ "required": ["hardware_ids"] })
}

fn system_schema() -> Value {
    json!({ "required": ["system_ids"] })
}

fn hardware_targets(params: &Value) -> Vec<String> {
    target_string_array(params, "hardware_ids")
}

fn system_targets(params: &Value) -> Vec<String> {
    target_string_array(params, "system_ids")
}

fn approve_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    let hardware_ids = required_string_array(ctx.params, "hardware_ids")?;
    if hardware_ids.is_empty() {
        return Err(OpError::Validation("empty targets".to_string()));
    }
    for hw in hardware_ids {
        let exists = tx
            .query_row(
                "SELECT 1 FROM sightings WHERE hardware_id = ?1",
                params![hw],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(OpError::NotFound);
        }
    }
    Ok(())
}

fn approve_dry_run(_tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let hardware_ids = required_string_array(ctx.params, "hardware_ids")?;
    Ok(json!({
        "would": "approve_sighting_as_quarantined",
        "count": hardware_ids.len(),
        "hardware_ids": hardware_ids,
    }))
}

fn approve_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let mut approved = Vec::new();
    for hw in required_string_array(ctx.params, "hardware_ids")? {
        let sid = approve_sighting(tx, &hw, None, DeviceKind::Individual)?;
        approved.push(json!({
            "hardware_id": hw,
            "system_id": sid.to_text(),
        }));
    }
    Ok(json!({ "approved": approved }))
}

fn retire_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    for sid in system_ids(ctx)? {
        let row = get_device(tx, &sid)?.ok_or(OpError::NotFound)?;
        if row.state == DeviceState::Retired {
            return Err(OpError::NotFound);
        }
    }
    Ok(())
}

fn retire_dry_run(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let mut devices = Vec::new();
    for sid in system_ids(ctx)? {
        let row = get_device(tx, &sid)?.ok_or(OpError::NotFound)?;
        devices.push(json!({
            "system_id": sid.to_text(),
            "state": row.state.to_db_for_op(),
            "hardware_id": row.hardware_id,
        }));
    }
    Ok(json!({ "would": "retire_device", "devices": devices }))
}

fn retire_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let mut retired = Vec::new();
    for sid in system_ids(ctx)? {
        retire_device(tx, &sid)?;
        retired.push(sid.to_text());
    }
    crate::device_credentials::recover_capacity_debt_if_possible_in_tx(tx, now_ms())?;
    Ok(json!({ "retired": retired }))
}

fn now_ms() -> i64 {
    use crate::device_credentials::CredentialClock;
    crate::device_credentials::SystemCredentialClock.now_ms()
}

fn system_ids(ctx: &OpContext<'_>) -> Result<Vec<SystemId>, OpError> {
    let system_ids = required_string_array(ctx.params, "system_ids")?;
    if system_ids.is_empty() {
        return Err(OpError::Validation("empty targets".to_string()));
    }
    system_ids
        .into_iter()
        .map(|sid| SystemId::from_text(&sid).map_err(OpError::from))
        .collect()
}

trait DeviceStateForOp {
    fn to_db_for_op(self) -> &'static str;
}

impl DeviceStateForOp for DeviceState {
    fn to_db_for_op(self) -> &'static str {
        match self {
            DeviceState::Quarantined => "quarantined",
            DeviceState::Active => "active",
            DeviceState::Retired => "retired",
        }
    }
}
