use iotkit_core_ledger::{
    DeviceKind, DeviceState, SystemId, approve_sighting, get_device, retire_device,
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};

use crate::{OpContext, OpDescriptor, OpError, Tier};

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
