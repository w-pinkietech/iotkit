use rusqlite::{OptionalExtension, params};
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde_json::{Value, json};

use crate::ingress_listener::{
    INGRESS_READY, IngressListenerMode, stage_ingress_tls_generation, validate_local_ingress_config,
};
use crate::{OpContext, OpDescriptor, OpError, Tier, fingerprint_of_pem};

use super::{required_str, required_string_array};

pub fn configure_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: "ingress.listener.configure",
        tier: Tier::Construction,
        bulk_escalates: false,
        changes_state: true,
        params_schema: || json!({"required":["enabled","bind_addr","interface","local_ingress_cidrs","mode"]}),
        targets: |_| vec!["ingress_listener".into()],
        preconditions: configure_preconditions,
        dry_run: configure_result,
        execute: configure_execute,
        secret_execute: None,
    }
}

pub fn disable_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: "ingress.listener.disable",
        tier: Tier::Construction,
        bulk_escalates: false,
        changes_state: true,
        params_schema: || json!({"required":[]}),
        targets: |_| vec!["ingress_listener".into()],
        preconditions: |_, _| Ok(()),
        dry_run: |_, _| Ok(json!({"enabled":false,"action":"disable"})),
        execute: disable_execute,
        secret_execute: None,
    }
}

pub fn rotate_tls_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: "ingress.tls.rotate",
        tier: Tier::Construction,
        bulk_escalates: false,
        changes_state: true,
        params_schema: || json!({"required":["cert_pem","key_pem"]}),
        targets: |_| vec!["ingress_tls".into()],
        preconditions: tls_preconditions,
        dry_run: tls_result,
        execute: tls_execute,
        secret_execute: None,
    }
}

fn configure_preconditions(
    tx: &rusqlite::Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<(), OpError> {
    let enabled = ctx
        .params
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| OpError::Validation("enabled must be boolean".into()))?;
    if enabled && !INGRESS_READY {
        return Err(OpError::PreconditionFailed("ingress_not_ready".into()));
    }
    let bind = required_str(ctx.params, "bind_addr")?;
    let interface = required_str(ctx.params, "interface")?;
    let cidrs = required_string_array(ctx.params, "local_ingress_cidrs")?;
    let mode = IngressListenerMode::parse(required_str(ctx.params, "mode")?)?;
    validate_local_ingress_config(bind, interface, &cidrs, mode)?;
    if mode == IngressListenerMode::Tls {
        let exists = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ingress_tls_material WHERE id=1)",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Err(OpError::PreconditionFailed("tls_material_missing".into()));
        }
    }
    Ok(())
}

fn configure_result(
    _tx: &rusqlite::Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<Value, OpError> {
    Ok(json!({"enabled":ctx.params["enabled"],"mode":ctx.params["mode"],"action":"configure"}))
}

fn configure_execute(
    tx: &rusqlite::Transaction<'_>,
    ctx: &OpContext<'_>,
) -> Result<Value, OpError> {
    let enabled = ctx.params["enabled"]
        .as_bool()
        .expect("precondition validated");
    let bind = required_str(ctx.params, "bind_addr")?;
    let interface = required_str(ctx.params, "interface")?;
    let cidrs = serde_json::to_string(&required_string_array(ctx.params, "local_ingress_cidrs")?)
        .map_err(|error| OpError::Internal(error.to_string()))?;
    let mode = IngressListenerMode::parse(required_str(ctx.params, "mode")?)?;
    let tls: Option<(i64, String)> = if mode == IngressListenerMode::Tls {
        tx.query_row(
            "SELECT generation, fingerprint FROM ingress_tls_material WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
    } else {
        None
    };
    tx.execute(
        "UPDATE ingress_listener_config SET desired_generation=desired_generation+1,
          enabled=?1, bind_addr=?2, interface=?3, local_ingress_cidrs=?4, mode=?5,
          desired_tls_generation=?6, desired_tls_fingerprint=?7, last_error=NULL,
          last_action='configured' WHERE id=1",
        params![
            enabled,
            bind,
            interface,
            cidrs,
            mode.as_str(),
            tls.as_ref().map(|v| v.0),
            tls.map(|v| v.1)
        ],
    )?;
    configure_result(tx, ctx)
}

fn disable_execute(tx: &rusqlite::Transaction<'_>, _ctx: &OpContext<'_>) -> Result<Value, OpError> {
    tx.execute(
        "UPDATE ingress_listener_config SET desired_generation=desired_generation+1,
          enabled=0, last_error=NULL, last_action='disabled' WHERE id=1",
        [],
    )?;
    Ok(json!({"enabled":false,"action":"disable"}))
}

fn tls_preconditions(_tx: &rusqlite::Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    validated_tls(ctx.params).map(|_| ())
}

fn tls_result(_tx: &rusqlite::Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let fingerprint = validated_tls(ctx.params)?;
    Ok(json!({"fingerprint":fingerprint,"action":"rotate_tls"}))
}

fn tls_execute(tx: &rusqlite::Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let fingerprint = validated_tls(ctx.params)?;
    let secret_dir = ctx
        .secret_dir
        .ok_or_else(|| OpError::PreconditionFailed("tls_custody_unavailable".into()))?;
    let generation: i64 = tx.query_row(
        "SELECT COALESCE(MAX(generation),0)+1 FROM ingress_tls_material",
        [],
        |row| row.get(0),
    )?;
    stage_ingress_tls_generation(
        secret_dir,
        generation,
        required_str(ctx.params, "cert_pem")?.as_bytes(),
        required_str(ctx.params, "key_pem")?.as_bytes(),
    )?;
    tx.execute(
        "INSERT INTO ingress_tls_material (id,generation,fingerprint,approved_at,approved_by)
         VALUES (1,?1,?2,unixepoch('subsec')*1000,?3)
         ON CONFLICT(id) DO UPDATE SET generation=excluded.generation,fingerprint=excluded.fingerprint,
           approved_at=excluded.approved_at,
           approved_by=excluded.approved_by",
        params![generation, fingerprint, ctx.actor_id],
    )?;
    tx.execute(
        "UPDATE ingress_listener_config SET desired_generation=desired_generation+1,
          desired_tls_generation=?1, desired_tls_fingerprint=?2, last_error=NULL,
          last_action='tls_rotated' WHERE id=1",
        params![generation, fingerprint],
    )?;
    Ok(json!({"generation":generation,"fingerprint":fingerprint,"action":"rotate_tls"}))
}

fn validated_tls(params: &Value) -> Result<String, OpError> {
    let cert = required_str(params, "cert_pem")?;
    let key = required_str(params, "key_pem")?;
    if cert.len() > 1024 * 1024 || key.len() > 1024 * 1024 {
        return Err(OpError::Validation("tls_material_too_large".into()));
    }
    let certs = CertificateDer::pem_slice_iter(cert.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| OpError::Validation("corrupt_tls_certificate".into()))?;
    if certs.is_empty() {
        return Err(OpError::Validation("corrupt_tls_certificate".into()));
    }
    let key_der = PrivateKeyDer::from_pem_slice(key.as_bytes())
        .map_err(|_| OpError::Validation("corrupt_tls_private_key".into()))?;
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key_der)
        .map_err(|_| OpError::Validation("tls_pair_mismatch".into()))?;
    let fingerprint = fingerprint_of_pem(cert).map_err(OpError::from)?;
    Ok(fingerprint)
}
