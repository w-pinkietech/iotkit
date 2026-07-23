use iotkit_core_ledger::{DeviceKind, DeviceState, NewDevice, SystemId};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};

use crate::device_credentials::{
    CredentialClock, DeviceAuthorityConfig, FlowWeight, SystemCredentialClock,
    SystemCredentialEntropy, abandon_device_credential_in_tx, approve_capacity_debt_in_tx,
    capacity_change_requires_approval, capacity_status, capacity_status_for_activation,
    capacity_status_for_new, change_device_flow_class_in_tx, configure_device_authority_in_tx,
    confirm_device_credential_in_tx, issue_device_credential_in_tx,
    register_device_principal_in_tx, revoke_device_credential_in_tx,
};
use crate::{
    ActorKind, DeviceCredentialDispatchResult, DeviceCredentialState, OpContext, OpDescriptor,
    OpError, SecretOpExecute, Tier,
};

use super::{required_str, target_string_array};

pub fn descriptors() -> Vec<OpDescriptor> {
    vec![
        add_descriptor(false),
        add_descriptor(true),
        lifecycle_descriptor(
            "device_credential.issue",
            issue_preconditions,
            issue_dry_run,
            Some(issue_secret_execute),
        ),
        issue_capacity_debt_descriptor(),
        lifecycle_descriptor(
            "device_credential.reissue",
            reissue_preconditions,
            reissue_dry_run,
            Some(reissue_secret_execute),
        ),
        lifecycle_descriptor(
            "device_credential.confirm",
            confirm_preconditions,
            confirm_dry_run,
            None,
        ),
        lifecycle_descriptor(
            "device_credential.abandon",
            abandon_preconditions,
            abandon_dry_run,
            None,
        ),
        lifecycle_descriptor(
            "device_credential.revoke",
            revoke_preconditions,
            revoke_dry_run,
            None,
        ),
        flow_descriptor(false),
        flow_descriptor(true),
        authority_config_descriptor(false),
        authority_config_descriptor(true),
    ]
}

fn add_descriptor(with_debt: bool) -> OpDescriptor {
    OpDescriptor {
        name: if with_debt {
            "device.add_with_credential_capacity_debt"
        } else {
            "device.add_with_credential"
        },
        tier: if with_debt {
            Tier::Construction
        } else {
            Tier::Daily
        },
        bulk_escalates: false,
        changes_state: true,
        params_schema: if with_debt {
            debt_add_schema
        } else {
            add_schema
        },
        targets: |params| {
            params
                .get("hardware_id")
                .and_then(Value::as_str)
                .map(|v| vec![v.into()])
                .unwrap_or_default()
        },
        preconditions: if with_debt {
            add_debt_preconditions
        } else {
            add_preconditions
        },
        dry_run: if with_debt {
            add_debt_dry_run
        } else {
            add_dry_run
        },
        execute: if with_debt {
            add_debt_execute
        } else {
            add_execute
        },
        secret_execute: Some(if with_debt {
            add_debt_secret_execute
        } else {
            add_secret_execute
        }),
    }
}

fn lifecycle_descriptor(
    name: &'static str,
    preconditions: fn(&Transaction<'_>, &OpContext<'_>) -> Result<(), OpError>,
    dry_run: fn(&Transaction<'_>, &OpContext<'_>) -> Result<Value, OpError>,
    secret_execute: Option<SecretOpExecute>,
) -> OpDescriptor {
    OpDescriptor {
        name,
        tier: Tier::Daily,
        bulk_escalates: false,
        changes_state: true,
        params_schema: lifecycle_schema,
        targets: |params| {
            ["principal_id", "credential_id"]
                .into_iter()
                .filter_map(|key| params.get(key).and_then(Value::as_str).map(str::to_owned))
                .collect()
        },
        preconditions,
        dry_run,
        execute: lifecycle_execute,
        secret_execute,
    }
}

fn issue_capacity_debt_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: "device_credential.issue_capacity_debt",
        tier: Tier::Construction,
        bulk_escalates: false,
        changes_state: true,
        params_schema: debt_lifecycle_schema,
        targets: |params| {
            params
                .get("principal_id")
                .and_then(Value::as_str)
                .map(|value| vec![value.to_owned()])
                .unwrap_or_default()
        },
        preconditions: issue_debt_preconditions,
        dry_run: issue_debt_dry_run,
        execute: lifecycle_execute,
        secret_execute: Some(issue_debt_secret_execute),
    }
}

fn flow_descriptor(with_debt: bool) -> OpDescriptor {
    OpDescriptor {
        name: if with_debt {
            "device.flow_class_change_capacity_debt"
        } else {
            "device.flow_class_change"
        },
        tier: if with_debt {
            Tier::Construction
        } else {
            Tier::Daily
        },
        bulk_escalates: false,
        changes_state: true,
        params_schema: if with_debt {
            debt_flow_schema
        } else {
            flow_schema
        },
        targets: |params| target_string_array(params, "principal_ids"),
        preconditions: if with_debt {
            flow_debt_preconditions
        } else {
            flow_preconditions
        },
        dry_run: if with_debt {
            flow_debt_dry_run
        } else {
            flow_dry_run
        },
        execute: if with_debt {
            flow_debt_execute
        } else {
            flow_execute
        },
        secret_execute: None,
    }
}

fn authority_config_descriptor(with_debt: bool) -> OpDescriptor {
    OpDescriptor {
        name: if with_debt {
            "device.authority_configure_capacity_debt"
        } else {
            "device.authority_configure"
        },
        tier: Tier::Construction,
        bulk_escalates: false,
        changes_state: true,
        params_schema: if with_debt {
            debt_authority_config_schema
        } else {
            authority_config_schema
        },
        targets: |_| Vec::new(),
        preconditions: if with_debt {
            authority_config_debt_preconditions
        } else {
            authority_config_preconditions
        },
        dry_run: if with_debt {
            authority_config_debt_dry_run
        } else {
            authority_config_dry_run
        },
        execute: if with_debt {
            authority_config_debt_execute
        } else {
            authority_config_execute
        },
        secret_execute: None,
    }
}

fn human_only(_tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    match ctx.actor_kind {
        ActorKind::Human | ActorKind::LocalCli => Ok(()),
        ActorKind::Ai | ActorKind::System => {
            Err(OpError::Forbidden("human_authority_required".into()))
        }
    }
}

fn add_schema() -> Value {
    json!({ "required": ["hardware_id", "flow_class", "reason_code"],
        "optional": ["label", "scope_system_ids"] })
}
fn debt_add_schema() -> Value {
    debt_schema(&["hardware_id", "flow_class", "reason_code"])
}
fn lifecycle_schema() -> Value {
    json!({ "required": ["principal_id", "reason_code"], "optional": ["credential_id"] })
}
fn debt_lifecycle_schema() -> Value {
    debt_schema(&["principal_id", "reason_code"])
}
fn authority_config_schema() -> Value {
    json!({ "required": ["low_steady_units", "low_burst_units", "default_steady_units",
        "default_burst_units", "high_steady_units", "high_burst_units",
        "capacity_steady_units", "capacity_burst_units", "stale_after_ms"] })
}
fn debt_authority_config_schema() -> Value {
    debt_schema(&[
        "low_steady_units",
        "low_burst_units",
        "default_steady_units",
        "default_burst_units",
        "high_steady_units",
        "high_burst_units",
        "capacity_steady_units",
        "capacity_burst_units",
        "stale_after_ms",
    ])
}
fn flow_schema() -> Value {
    json!({ "required": ["principal_ids", "flow_class"] })
}
fn debt_flow_schema() -> Value {
    debt_schema(&["principal_ids", "flow_class"])
}

fn debt_schema(base: &[&str]) -> Value {
    let required = base.iter().map(|value| json!(value)).collect::<Vec<_>>();
    json!({"required": required, "optional": ["label", "scope_system_ids",
        "credential_id", "expected_required_steady_units", "expected_required_burst_units",
        "expected_capacity_steady_units", "expected_capacity_burst_units",
        "expected_authority_generation"]})
}

fn approval_preview(tx: &Transaction<'_>, status: crate::CapacityStatus) -> Result<Value, OpError> {
    Ok(json!({
        "required_steady_units": status.required_steady_units,
        "required_burst_units": status.required_burst_units,
        "capacity_steady_units": status.capacity_steady_units,
        "capacity_burst_units": status.capacity_burst_units,
        "authority_generation": crate::device_auth_generation(tx)?,
    }))
}

fn validate_capacity_approval(
    tx: &Transaction<'_>,
    params: &Value,
    status: crate::CapacityStatus,
) -> Result<(), OpError> {
    let matches = [
        (
            "expected_required_steady_units",
            status.required_steady_units,
        ),
        ("expected_required_burst_units", status.required_burst_units),
        (
            "expected_capacity_steady_units",
            status.capacity_steady_units,
        ),
        ("expected_capacity_burst_units", status.capacity_burst_units),
        (
            "expected_authority_generation",
            crate::device_auth_generation(tx)?,
        ),
    ]
    .into_iter()
    .all(|(key, expected)| params.get(key).and_then(Value::as_i64) == Some(expected));
    if !matches {
        return Err(OpError::PreconditionFailed(
            "capacity_approval_stale".into(),
        ));
    }
    Ok(())
}

fn add_common_preconditions(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    allow_debt: bool,
) -> Result<(), OpError> {
    human_only(tx, ctx)?;
    let hardware_id = required_str(ctx.params, "hardware_id")?;
    if hardware_id.is_empty() {
        return Err(OpError::Validation("hardware_id must not be empty".into()));
    }
    require_reason(ctx.params, "device_commissioning")?;
    let status = capacity_status_for_new(tx, required_str(ctx.params, "flow_class")?)?;
    if !allow_debt && capacity_change_requires_approval(tx, status)? {
        return Err(OpError::PreconditionFailed("capacity_exceeded".into()));
    }
    Ok(())
}
fn add_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    add_common_preconditions(tx, ctx, false)
}
fn add_debt_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    add_common_preconditions(tx, ctx, true)?;
    let status = capacity_status_for_new(tx, required_str(ctx.params, "flow_class")?)?;
    if ctx.dry_run || !status.exceeds() {
        Ok(())
    } else {
        validate_capacity_approval(tx, ctx.params, status)
    }
}
fn add_dry_run(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    add_common_dry_run(tx, ctx, false)
}
fn add_debt_dry_run(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    add_common_dry_run(tx, ctx, true)
}
fn add_common_dry_run(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    debt: bool,
) -> Result<Value, OpError> {
    let status = capacity_status_for_new(tx, required_str(ctx.params, "flow_class")?)?;
    let mut value = approval_preview(tx, status)?;
    let object = value.as_object_mut().expect("preview object");
    object.insert("would".into(), json!("add_device_and_issue_credential"));
    object.insert("capacity_debt".into(), json!(debt && status.exceeds()));
    Ok(value)
}
fn add_execute(_tx: &Transaction<'_>, _ctx: &OpContext<'_>) -> Result<Value, OpError> {
    Err(OpError::Internal("secret executor required".into()))
}
fn add_debt_execute(_tx: &Transaction<'_>, _ctx: &OpContext<'_>) -> Result<Value, OpError> {
    Err(OpError::Internal("secret executor required".into()))
}
fn add_secret_execute(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<DeviceCredentialDispatchResult, OpError> {
    add_common_secret_execute(tx, ctx, false)
}

fn add_debt_secret_execute(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<DeviceCredentialDispatchResult, OpError> {
    add_common_secret_execute(tx, ctx, true)
}

fn add_common_secret_execute(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    debt: bool,
) -> Result<DeviceCredentialDispatchResult, OpError> {
    let flow_class = required_str(ctx.params, "flow_class")?;
    let status = capacity_status_for_new(tx, flow_class)?;
    let sid = iotkit_core_ledger::insert_device(
        tx,
        &NewDevice {
            hardware_id: required_str(ctx.params, "hardware_id")?.into(),
            user_label: ctx
                .params
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_owned),
            parent: None,
            kind: DeviceKind::Individual,
            initial_state: DeviceState::Quarantined,
        },
    )?;
    let principal_id = format!("dev_{}", sid.to_text());
    let mut scopes = vec![sid];
    if let Some(extra) = ctx.params.get("scope_system_ids").and_then(Value::as_array) {
        for value in extra {
            let text = value.as_str().ok_or_else(|| {
                OpError::Validation("scope_system_ids entries must be strings".into())
            })?;
            let scope = SystemId::from_text(text)?;
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
    }
    let now = now_ms();
    register_device_principal_in_tx(tx, &principal_id, &sid, &scopes, flow_class, now)?;
    if debt && status.exceeds() {
        approve_capacity_debt_in_tx(tx, status, ctx.actor_id, "device_add", now)?;
    }
    let (credential_id, plaintext) = issue_device_credential_in_tx(
        tx,
        &principal_id,
        DeviceCredentialState::Current,
        require_reason(ctx.params, "device_commissioning")?,
        &mut SystemCredentialEntropy,
        &SystemCredentialClock,
    )?;
    Ok(DeviceCredentialDispatchResult::new(
        json!({"system_id":sid.to_text(),"principal_id":principal_id,"credential_id":credential_id}),
        plaintext,
    ))
}

fn require_reason<'a>(params: &'a Value, expected: &str) -> Result<&'a str, OpError> {
    let reason = required_str(params, "reason_code")?;
    crate::CredentialReasonCode::parse(reason)?;
    if reason != expected {
        return Err(OpError::Validation(format!(
            "reason_code must be {expected}"
        )));
    }
    Ok(reason)
}
fn principal_exists(tx: &Transaction<'_>, principal_id: &str) -> Result<bool, OpError> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM device_ingest_principals p
         JOIN devices d ON d.system_id=p.device_system_id AND d.state!='retired'
         WHERE p.principal_id = ?1
           AND EXISTS (
             SELECT 1 FROM device_principal_scopes s
             JOIN devices sd ON sd.system_id=s.system_id AND sd.state!='retired'
             WHERE s.principal_id=p.principal_id
           ))",
        [principal_id],
        |r| r.get(0),
    )?)
}
fn live_state(
    tx: &Transaction<'_>,
    principal_id: &str,
    state: &str,
) -> Result<Option<String>, OpError> {
    Ok(tx
        .query_row(
            "SELECT credential_id FROM device_credentials WHERE principal_id = ?1 AND state = ?2",
            params![principal_id, state],
            |r| r.get(0),
        )
        .optional()?)
}
fn issue_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    issue_common_preconditions(tx, ctx, false)
}
fn issue_debt_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    issue_common_preconditions(tx, ctx, true)
}
fn issue_common_preconditions(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    allow_debt: bool,
) -> Result<(), OpError> {
    human_only(tx, ctx)?;
    require_reason(ctx.params, "manual_issue")?;
    let p = required_str(ctx.params, "principal_id")?;
    if !principal_exists(tx, p)? {
        return Err(OpError::NotFound);
    }
    if live_state(tx, p, "current")?.is_some() || live_state(tx, p, "pending")?.is_some() {
        return Err(OpError::PreconditionFailed("live_credential_exists".into()));
    }
    let status = capacity_status_for_activation(tx, p)?;
    if allow_debt {
        if !ctx.dry_run && status.exceeds() {
            validate_capacity_approval(tx, ctx.params, status)?;
        }
    } else if capacity_change_requires_approval(tx, status)? {
        return Err(OpError::PreconditionFailed("capacity_exceeded".into()));
    }
    Ok(())
}
fn reissue_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    human_only(tx, ctx)?;
    require_reason(ctx.params, "credential_reissue")?;
    let p = required_str(ctx.params, "principal_id")?;
    if live_state(tx, p, "current")?.is_none() {
        return Err(OpError::PreconditionFailed(
            "current_credential_required".into(),
        ));
    }
    if live_state(tx, p, "pending")?.is_some() {
        return Err(OpError::PreconditionFailed(
            "pending_credential_exists".into(),
        ));
    }
    Ok(())
}
fn issue_dry_run(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    issue_common_dry_run(tx, ctx, false)
}
fn issue_debt_dry_run(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    issue_common_dry_run(tx, ctx, true)
}
fn issue_common_dry_run(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    debt: bool,
) -> Result<Value, OpError> {
    let principal_id = required_str(ctx.params, "principal_id")?;
    let status = capacity_status_for_activation(tx, principal_id)?;
    let mut value = approval_preview(tx, status)?;
    let object = value.as_object_mut().expect("preview object");
    object.insert("would".into(), json!("issue_device_credential"));
    object.insert("principal_id".into(), json!(principal_id));
    object.insert("capacity_debt".into(), json!(debt && status.exceeds()));
    Ok(value)
}
fn reissue_dry_run(_tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    Ok(
        json!({"would":"issue_pending_device_credential","principal_id":required_str(ctx.params,"principal_id")?}),
    )
}
fn lifecycle_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    match required_str(ctx.params, "reason_code")? {
        "credential_confirmed" => confirm_execute(tx, ctx),
        "pending_abandoned" => abandon_execute(tx, ctx),
        "operator_revoked" => revoke_execute(tx, ctx),
        _ => Err(OpError::Internal("secret executor required".into())),
    }
}
fn issue_secret_execute(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<DeviceCredentialDispatchResult, OpError> {
    issue_state(tx, ctx, DeviceCredentialState::Current, false)
}
fn issue_debt_secret_execute(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<DeviceCredentialDispatchResult, OpError> {
    issue_state(tx, ctx, DeviceCredentialState::Current, true)
}
fn reissue_secret_execute(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<DeviceCredentialDispatchResult, OpError> {
    issue_state(tx, ctx, DeviceCredentialState::Pending, false)
}
fn issue_state(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    state: DeviceCredentialState,
    approve_debt: bool,
) -> Result<DeviceCredentialDispatchResult, OpError> {
    let p = required_str(ctx.params, "principal_id")?;
    if approve_debt {
        let status = capacity_status_for_activation(tx, p)?;
        if status.exceeds() {
            approve_capacity_debt_in_tx(tx, status, ctx.actor_id, "credential_issue", now_ms())?;
        }
    }
    let (id, secret) = issue_device_credential_in_tx(
        tx,
        p,
        state,
        required_str(ctx.params, "reason_code")?,
        &mut SystemCredentialEntropy,
        &SystemCredentialClock,
    )?;
    Ok(DeviceCredentialDispatchResult::new(
        json!({"principal_id":p,"credential_id":id,"state":state.as_str()}),
        secret,
    ))
}

fn credential_preconditions(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    required_state: Option<&str>,
    must_be_proven: bool,
) -> Result<(), OpError> {
    human_only(tx, ctx)?;
    let expected = match required_state {
        Some("pending") if must_be_proven => "credential_confirmed",
        Some("pending") => "pending_abandoned",
        _ => "operator_revoked",
    };
    require_reason(ctx.params, expected)?;
    let p = required_str(ctx.params, "principal_id")?;
    let id = required_str(ctx.params, "credential_id")?;
    let row:Option<(String,Option<i64>)>=tx.query_row("SELECT state, proven_at FROM device_credentials WHERE principal_id=?1 AND credential_id=?2",
        params![p,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((state, proven)) = row else {
        return Err(OpError::NotFound);
    };
    if required_state.is_some_and(|s| s != state) || (must_be_proven && proven.is_none()) {
        return Err(OpError::PreconditionFailed(
            "credential_state_conflict".into(),
        ));
    }
    Ok(())
}
fn confirm_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    credential_preconditions(tx, ctx, Some("pending"), true)
}
fn abandon_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    credential_preconditions(tx, ctx, Some("pending"), false)
}
fn revoke_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    credential_preconditions(tx, ctx, None, false)
}
fn lifecycle_dry(ctx: &OpContext<'_>, action: &str) -> Result<Value, OpError> {
    Ok(
        json!({"would":action,"principal_id":required_str(ctx.params,"principal_id")?,"credential_id":required_str(ctx.params,"credential_id")?}),
    )
}
fn confirm_dry_run(_: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    lifecycle_dry(ctx, "confirm_device_credential")
}
fn abandon_dry_run(_: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    lifecycle_dry(ctx, "abandon_device_credential")
}
fn revoke_dry_run(_: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    lifecycle_dry(ctx, "revoke_device_credential")
}
fn confirm_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    confirm_device_credential_in_tx(
        tx,
        required_str(ctx.params, "principal_id")?,
        required_str(ctx.params, "credential_id")?,
        required_str(ctx.params, "reason_code")?,
        now_ms(),
    )?;
    Ok(json!({"confirmed":required_str(ctx.params,"credential_id")?}))
}
fn abandon_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    abandon_device_credential_in_tx(
        tx,
        required_str(ctx.params, "principal_id")?,
        required_str(ctx.params, "credential_id")?,
        required_str(ctx.params, "reason_code")?,
        now_ms(),
    )?;
    Ok(json!({"abandoned":required_str(ctx.params,"credential_id")?}))
}
fn revoke_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    revoke_device_credential_in_tx(
        tx,
        required_str(ctx.params, "principal_id")?,
        required_str(ctx.params, "credential_id")?,
        required_str(ctx.params, "reason_code")?,
        now_ms(),
    )?;
    Ok(json!({"revoked":required_str(ctx.params,"credential_id")?}))
}

fn flow_common_preconditions(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    allow_debt: bool,
) -> Result<(), OpError> {
    human_only(tx, ctx)?;
    let ids = target_string_array(ctx.params, "principal_ids");
    if ids.len() != 1 {
        return Err(OpError::Validation(
            "exactly one principal_id is required".into(),
        ));
    }
    let status = capacity_status(tx, Some((&ids[0], required_str(ctx.params, "flow_class")?)))?;
    if !allow_debt && capacity_change_requires_approval(tx, status)? {
        return Err(OpError::PreconditionFailed("capacity_exceeded".into()));
    }
    Ok(())
}
fn flow_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    flow_common_preconditions(tx, ctx, false)
}
fn flow_debt_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    flow_common_preconditions(tx, ctx, true)?;
    let ids = target_string_array(ctx.params, "principal_ids");
    let status = capacity_status(tx, Some((&ids[0], required_str(ctx.params, "flow_class")?)))?;
    if ctx.dry_run || !status.exceeds() {
        Ok(())
    } else {
        validate_capacity_approval(tx, ctx.params, status)
    }
}
fn flow_common_dry(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    debt: bool,
) -> Result<Value, OpError> {
    let ids = target_string_array(ctx.params, "principal_ids");
    let status = capacity_status(tx, Some((&ids[0], required_str(ctx.params, "flow_class")?)))?;
    let mut value = approval_preview(tx, status)?;
    let object = value.as_object_mut().expect("preview object");
    object.insert("would".into(), json!("change_flow_class"));
    object.insert("capacity_debt".into(), json!(debt && status.exceeds()));
    Ok(value)
}
fn flow_dry_run(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    flow_common_dry(tx, ctx, false)
}
fn flow_debt_dry_run(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    flow_common_dry(tx, ctx, true)
}
fn flow_common_execute(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    debt: bool,
) -> Result<Value, OpError> {
    let id = &target_string_array(ctx.params, "principal_ids")[0];
    let status = change_device_flow_class_in_tx(
        tx,
        id,
        required_str(ctx.params, "flow_class")?,
        debt,
        ctx.actor_id,
        now_ms(),
    )?;
    Ok(
        json!({"principal_id":id,"flow_class":required_str(ctx.params,"flow_class")?,"capacity_debt":debt&&status.exceeds()}),
    )
}

fn required_positive_i64(params: &Value, key: &str) -> Result<i64, OpError> {
    let value = params
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| OpError::Validation(format!("{key} must be an integer")))?;
    if value <= 0 {
        return Err(OpError::Validation(format!("{key} must be positive")));
    }
    Ok(value)
}

fn authority_config(params: &Value) -> Result<DeviceAuthorityConfig, OpError> {
    Ok(DeviceAuthorityConfig {
        low: FlowWeight {
            steady_units: required_positive_i64(params, "low_steady_units")?,
            burst_units: required_positive_i64(params, "low_burst_units")?,
        },
        default: FlowWeight {
            steady_units: required_positive_i64(params, "default_steady_units")?,
            burst_units: required_positive_i64(params, "default_burst_units")?,
        },
        high: FlowWeight {
            steady_units: required_positive_i64(params, "high_steady_units")?,
            burst_units: required_positive_i64(params, "high_burst_units")?,
        },
        capacity: FlowWeight {
            steady_units: required_positive_i64(params, "capacity_steady_units")?,
            burst_units: required_positive_i64(params, "capacity_burst_units")?,
        },
        stale_after_ms: required_positive_i64(params, "stale_after_ms")?,
    })
}

fn authority_config_status(
    tx: &Transaction<'_>,
    config: DeviceAuthorityConfig,
) -> Result<crate::CapacityStatus, OpError> {
    let (low, default, high): (i64, i64, i64) = tx.query_row(
        "SELECT
           COALESCE(SUM(CASE WHEN p.flow_class='low' THEN 1 ELSE 0 END),0),
           COALESCE(SUM(CASE WHEN p.flow_class='default' THEN 1 ELSE 0 END),0),
           COALESCE(SUM(CASE WHEN p.flow_class='high' THEN 1 ELSE 0 END),0)
         FROM live_device_ingest_principals p",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let weighted_total = |values: [(i64, i64); 3]| -> Result<i64, OpError> {
        values
            .into_iter()
            .try_fold(0_i64, |total, (count, weight)| {
                count
                    .checked_mul(weight)
                    .and_then(|value| total.checked_add(value))
                    .ok_or_else(|| OpError::Validation("capacity_math_overflow".into()))
            })
    };
    Ok(crate::CapacityStatus {
        required_steady_units: weighted_total([
            (low, config.low.steady_units),
            (default, config.default.steady_units),
            (high, config.high.steady_units),
        ])?,
        required_burst_units: weighted_total([
            (low, config.low.burst_units),
            (default, config.default.burst_units),
            (high, config.high.burst_units),
        ])?,
        capacity_steady_units: config.capacity.steady_units,
        capacity_burst_units: config.capacity.burst_units,
    })
}

fn authority_config_preconditions(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<(), OpError> {
    human_only(tx, ctx)?;
    let status = authority_config_status(tx, authority_config(ctx.params)?.validate()?)?;
    if capacity_change_requires_approval(tx, status)? {
        return Err(OpError::PreconditionFailed("capacity_exceeded".into()));
    }
    Ok(())
}

fn authority_config_debt_preconditions(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<(), OpError> {
    human_only(tx, ctx)?;
    let status = authority_config_status(tx, authority_config(ctx.params)?.validate()?)?;
    if ctx.dry_run || !status.exceeds() {
        Ok(())
    } else {
        validate_capacity_approval(tx, ctx.params, status)
    }
}

fn authority_config_dry_run(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    authority_config_common_dry_run(tx, ctx, false)
}
fn authority_config_debt_dry_run(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<Value, OpError> {
    authority_config_common_dry_run(tx, ctx, true)
}
fn authority_config_common_dry_run(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    debt: bool,
) -> Result<Value, OpError> {
    let config = authority_config(ctx.params)?;
    let status = authority_config_status(tx, config.validate()?)?;
    let mut value = approval_preview(tx, status)?;
    let object = value.as_object_mut().expect("preview object");
    object.insert("would".into(), json!("configure_device_authority"));
    object.insert("capacity_debt".into(), json!(debt && status.exceeds()));
    object.insert("config".into(), ctx.params.clone());
    Ok(value)
}

fn authority_config_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    authority_config_common_execute(tx, ctx, false)
}
fn authority_config_debt_execute(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<Value, OpError> {
    authority_config_common_execute(tx, ctx, true)
}
fn authority_config_common_execute(
    tx: &Transaction<'_>,
    ctx: &OpContext<'_>,
    approve_debt: bool,
) -> Result<Value, OpError> {
    let config = authority_config(ctx.params)?;
    configure_device_authority_in_tx(tx, config, approve_debt, ctx.actor_id, now_ms())?;
    Ok(json!({"configured":true}))
}
fn flow_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    flow_common_execute(tx, ctx, false)
}
fn flow_debt_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    flow_common_execute(tx, ctx, true)
}

fn now_ms() -> i64 {
    SystemCredentialClock.now_ms()
}
