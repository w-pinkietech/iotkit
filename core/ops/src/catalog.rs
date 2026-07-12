use std::sync::Arc;
use std::{fmt, ops::Deref};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{Value, json};

use crate::{Actor, ActorKind, OpsError, Tier};

pub struct OpContext<'a> {
    pub params: &'a Value,
    pub actor_id: &'a str,
    pub source: Option<&'a str>,
    pub clock_trust: Option<&'a crate::ClockTrust>,
    pub actor_kind: ActorKind,
    pub dry_run: bool,
    /// Product-owned directory for durable secret custody. Generic dispatches deliberately
    /// provide none, so an operation cannot silently fall back to database secret storage.
    pub secret_dir: Option<&'a std::path::Path>,
}

pub type SecretOpExecute =
    fn(&Transaction<'_>, &OpContext<'_>) -> Result<DeviceCredentialDispatchResult, OpError>;

pub struct OpDescriptor {
    pub name: &'static str,
    pub tier: Tier,
    pub bulk_escalates: bool,
    pub changes_state: bool,
    pub params_schema: fn() -> Value,
    pub targets: fn(&Value) -> Vec<String>,
    pub preconditions: fn(&Transaction<'_>, &OpContext<'_>) -> Result<(), OpError>,
    pub dry_run: fn(&Transaction<'_>, &OpContext<'_>) -> Result<Value, OpError>,
    pub execute: fn(&Transaction<'_>, &OpContext<'_>) -> Result<Value, OpError>,
    pub secret_execute: Option<SecretOpExecute>,
}

pub struct DeviceCredentialDispatchResult {
    metadata: Value,
    presentation: crate::DeviceCredentialPresentation,
}

impl DeviceCredentialDispatchResult {
    pub fn new(metadata: Value, presentation: crate::DeviceCredentialPresentation) -> Self {
        Self {
            metadata,
            presentation,
        }
    }

    pub fn metadata(&self) -> &Value {
        &self.metadata
    }

    pub fn consume(self) -> (Value, zeroize::Zeroizing<String>) {
        (self.metadata, self.presentation.consume())
    }
}

impl fmt::Debug for DeviceCredentialDispatchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCredentialDispatchResult")
            .field("metadata", &self.metadata)
            .field("presentation", &"[REDACTED]")
            .finish()
    }
}

pub enum DispatchResult {
    Public(Value),
    DeviceCredential(DeviceCredentialDispatchResult),
}

impl DispatchResult {
    pub fn into_public(self) -> Result<Value, OpError> {
        match self {
            Self::Public(value) => Ok(value),
            Self::DeviceCredential(_) => Err(OpError::Forbidden(
                "authorized_presentation_required".into(),
            )),
        }
    }

    pub fn metadata(&self) -> &Value {
        match self {
            Self::Public(value) => value,
            Self::DeviceCredential(secret) => secret.metadata(),
        }
    }
}

impl fmt::Debug for DispatchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public(value) => f.debug_tuple("Public").field(value).finish(),
            Self::DeviceCredential(secret) => secret.fmt(f),
        }
    }
}

impl Deref for DispatchResult {
    type Target = Value;
    fn deref(&self) -> &Self::Target {
        self.metadata()
    }
}

impl PartialEq for DispatchResult {
    fn eq(&self, other: &Self) -> bool {
        self.metadata() == other.metadata()
    }
}

impl PartialEq<Value> for DispatchResult {
    fn eq(&self, other: &Value) -> bool {
        self.metadata() == other
    }
}

pub struct DispatchRequest {
    pub op: String,
    pub params: Value,
    pub dry_run: bool,
    pub actor: Actor,
    pub source: Option<String>,
    pub step_up_verified: bool,
    pub clock_trust: Option<Arc<crate::ClockTrust>>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OpError {
    #[error("op not found")]
    NotFound,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("step-up required")]
    StepUpRequired,
    #[error("precondition failed: {0}")]
    PreconditionFailed(String),
    #[error("validation: {0}")]
    Validation(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl From<rusqlite::Error> for OpError {
    fn from(value: rusqlite::Error) -> Self {
        let message = value.to_string();
        if message.contains("capacity_math_overflow") {
            Self::Validation("capacity_math_overflow".into())
        } else {
            Self::Internal(message)
        }
    }
}

impl From<iotkit_core_storage::StorageError> for OpError {
    fn from(value: iotkit_core_storage::StorageError) -> Self {
        Self::Internal(value.to_string())
    }
}

impl From<iotkit_core_ledger::LedgerError> for OpError {
    fn from(value: iotkit_core_ledger::LedgerError) -> Self {
        match value {
            iotkit_core_ledger::LedgerError::NotFound(_) => Self::NotFound,
            iotkit_core_ledger::LedgerError::InvalidId(_) => Self::Validation(value.to_string()),
            iotkit_core_ledger::LedgerError::HardwareIdInUse(_)
            | iotkit_core_ledger::LedgerError::InvalidReplace(_) => {
                Self::PreconditionFailed(value.to_string())
            }
            iotkit_core_ledger::LedgerError::Storage(_)
            | iotkit_core_ledger::LedgerError::Sqlite(_) => Self::Internal(value.to_string()),
        }
    }
}

impl From<iotkit_core_registry::RegistryError> for OpError {
    fn from(value: iotkit_core_registry::RegistryError) -> Self {
        match value {
            iotkit_core_registry::RegistryError::TargetNotFound(_) => Self::NotFound,
            iotkit_core_registry::RegistryError::NamespaceCollision(_)
            | iotkit_core_registry::RegistryError::AliasExists(_) => {
                Self::PreconditionFailed(value.to_string())
            }
            iotkit_core_registry::RegistryError::InvalidKey(_) => {
                Self::Validation(value.to_string())
            }
            iotkit_core_registry::RegistryError::Ledger(e) => e.into(),
            iotkit_core_registry::RegistryError::Sqlite(_) => Self::Internal(value.to_string()),
        }
    }
}

impl From<OpsError> for OpError {
    fn from(value: OpsError) -> Self {
        match value {
            OpsError::NotFound => Self::NotFound,
            OpsError::Conflict => Self::PreconditionFailed(value.to_string()),
            OpsError::Forbidden => Self::Forbidden(value.to_string()),
            OpsError::Validation(code) if code == "capacity_math_overflow" => {
                Self::Validation(code)
            }
            OpsError::Validation(code) => Self::Validation(format!("validation: {code}")),
            OpsError::ClockUntrusted => Self::PreconditionFailed(value.to_string()),
            OpsError::Ledger(e) => e.into(),
            OpsError::Sqlite(error) if error.to_string().contains("capacity_math_overflow") => {
                Self::Validation("capacity_math_overflow".into())
            }
            OpsError::Storage(_)
            | OpsError::CredentialHash
            | OpsError::Random
            | OpsError::Io(_) => Self::Internal(value.to_string()),
            OpsError::Sqlite(_) => Self::Internal(value.to_string()),
        }
    }
}

pub fn dispatch(
    conn: &Connection,
    catalog: &[OpDescriptor],
    req: DispatchRequest,
) -> Result<DispatchResult, OpError> {
    dispatch_with_secret_dir(conn, catalog, req, None)
}

pub fn dispatch_with_secret_dir(
    conn: &Connection,
    catalog: &[OpDescriptor],
    req: DispatchRequest,
    secret_dir: Option<&std::path::Path>,
) -> Result<DispatchResult, OpError> {
    let Some(descriptor) = catalog.iter().find(|op| op.name == req.op) else {
        return Err(OpError::NotFound);
    };
    let settles_tls_custody = req.op == "ingress.tls.rotate" && !req.dry_run;
    let reconcile_tls_custody = || -> Result<(), OpError> {
        if settles_tls_custody && let Some(secret_dir) = secret_dir {
            crate::reconcile_ingress_tls_custody(conn, secret_dir)
                .map_err(|_| OpError::Internal("tls_custody_io".into()))?;
        }
        Ok(())
    };
    // Recover a prior crash window before choosing the next generation. This removes
    // unreferenced material and promotes only state whose R14 transaction is already visible.
    reconcile_tls_custody()?;

    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|e| OpError::Internal(e.to_string()))?;
    if descriptor.tier == Tier::ReadOnly {
        return Err(OpError::Internal("invalid op tier: read_only".to_string()));
    }
    debug_assert_ne!(descriptor.tier, Tier::ReadOnly);

    let targets = (descriptor.targets)(&req.params);
    let effective_tier = if targets.len() > 1 && descriptor.bulk_escalates {
        match descriptor.tier {
            Tier::ReadOnly => Tier::Routine,
            Tier::Routine => Tier::Daily,
            Tier::Daily => Tier::Construction,
            Tier::Construction => Tier::Construction,
        }
    } else {
        descriptor.tier
    };
    let ctx = OpContext {
        params: &req.params,
        actor_id: &req.actor.actor_id,
        source: req.source.as_deref(),
        clock_trust: req.clock_trust.as_deref(),
        actor_kind: req.actor.actor_kind,
        dry_run: req.dry_run,
        secret_dir,
    };

    let result = (|| -> Result<DispatchResult, OpError> {
        if matches!(req.actor.actor_kind, ActorKind::Human | ActorKind::Ai) {
            let token_expiry = tx
                .query_row(
                    "SELECT expires_at FROM operator_tokens
                     WHERE token_id = ?1
                       AND revoked_at IS NULL
                       AND auth_generation = (
                           SELECT auth_generation FROM auth_state WHERE id = 1
                       )",
                    [req.actor.actor_id.as_str()],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()
                .map_err(|e| OpError::Internal(e.to_string()))?
                .flatten();
            let token_exists = tx
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM operator_tokens
                       WHERE token_id = ?1
                         AND revoked_at IS NULL
                         AND auth_generation = (
                           SELECT auth_generation FROM auth_state WHERE id = 1
                         )
                     )",
                    [req.actor.actor_id.as_str()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|e| OpError::Internal(e.to_string()))?;
            if !token_exists {
                return Err(OpError::Forbidden("token_revoked".to_string()));
            }
            if let Some(expires_at) = token_expiry {
                let clock_trust = req
                    .clock_trust
                    .as_deref()
                    .ok_or_else(|| OpError::Forbidden("clock_untrusted".to_string()))?;
                let now =
                    clock_trust
                        .trusted_now_and_advance(&tx)
                        .map_err(|error| match error {
                            crate::ClockTrustError::Untrusted => {
                                OpError::Forbidden("clock_untrusted".to_string())
                            }
                            crate::ClockTrustError::Ops(error) => {
                                OpError::Internal(error.to_string())
                            }
                        })?;
                if expires_at <= now {
                    return Err(OpError::Forbidden("token_revoked".to_string()));
                }
            }
        }

        if req.actor.tier_ceiling < effective_tier {
            return Err(OpError::Forbidden("tier".to_string()));
        }

        if effective_tier == Tier::Construction && !req.step_up_verified {
            return Err(OpError::StepUpRequired);
        }

        if descriptor.secret_execute.is_some()
            && !req.dry_run
            && req.actor.actor_kind != ActorKind::LocalCli
        {
            return Err(OpError::Forbidden(
                "authorized_presentation_required".into(),
            ));
        }

        validate_params((descriptor.params_schema)(), &req.params)?;
        tx.execute_batch("SAVEPOINT op")
            .map_err(|e| OpError::Internal(e.to_string()))?;

        let op_result = (|| -> Result<DispatchResult, OpError> {
            (descriptor.preconditions)(&tx, &ctx)?;
            if req.dry_run {
                return (descriptor.dry_run)(&tx, &ctx).map(DispatchResult::Public);
            }

            let value = match descriptor.secret_execute {
                Some(execute) => DispatchResult::DeviceCredential(execute(&tx, &ctx)?),
                None => DispatchResult::Public((descriptor.execute)(&tx, &ctx)?),
            };
            if descriptor.changes_state {
                iotkit_core_ledger::bump_generation(&tx)
                    .map_err(|e| OpError::Internal(e.to_string()))?;
            }
            Ok(value)
        })();

        if req.dry_run || op_result.is_err() {
            let cleanup_result = tx
                .execute_batch("ROLLBACK TO op; RELEASE op")
                .map_err(|e| OpError::Internal(e.to_string()));
            return match (op_result, cleanup_result) {
                (Ok(value), Ok(())) => Ok(value),
                (Err(err), Ok(())) => Err(err),
                (_, Err(err)) => Err(err),
            };
        }

        tx.execute_batch("RELEASE op")
            .map_err(|e| OpError::Internal(e.to_string()))?;
        op_result
    })();

    let detail = json!({
        "op": req.op,
        "actor": req.actor.actor_id,
        "actor_kind": actor_kind_str(req.actor.actor_kind),
        "tier": descriptor.tier.as_str(),
        "effective_tier": effective_tier.as_str(),
        "dry_run": req.dry_run,
        "params": redact_audit_value(&req.params),
        "result": audit_result(&result),
        "targets": targets,
        "source": req.source,
    });
    if let Err(error) = iotkit_core_ledger::record_event(&tx, "r14_op", None, &detail.to_string()) {
        drop(tx);
        reconcile_tls_custody()?;
        return Err(OpError::Internal(error.to_string()));
    }
    if let Err(error) = tx.commit() {
        reconcile_tls_custody()?;
        return Err(OpError::Internal(error.to_string()));
    }
    // Promotion is intentionally after audit + SQLite commit. On an operation error this same
    // pass removes any unreferenced staging; on success it publishes the now-proven generation.
    reconcile_tls_custody()?;

    result
}

fn redact_audit_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase();
                    let sensitive_key = [
                        "plaintext",
                        "token",
                        "credential",
                        "passphrase",
                        "private",
                        "secret",
                        "pem",
                    ]
                    .iter()
                    .any(|marker| normalized.contains(marker))
                        || (normalized.ends_with("key")
                            && !matches!(normalized.as_str(), "key" | "measurement_key"));
                    let value = if sensitive_key {
                        Value::String("[REDACTED]".into())
                    } else {
                        redact_audit_value(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_audit_value).collect()),
        Value::String(value)
            if value.contains("ikd_")
                || value.contains("iko_")
                || value.contains("-----BEGIN ")
                || value.contains("PRIVATE KEY-----") =>
        {
            Value::String("[REDACTED]".into())
        }
        _ => value.clone(),
    }
}

fn validate_params(schema: Value, params: &Value) -> Result<(), OpError> {
    let Some(params) = params.as_object() else {
        return Err(OpError::Validation("params must be an object".into()));
    };
    let Some(required) = schema.get("required").and_then(Value::as_array) else {
        return Err(OpError::Internal(
            "operation schema must declare required parameters".into(),
        ));
    };
    let optional = schema
        .get("optional")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut declared = std::collections::BTreeSet::new();
    for key in required.iter().chain(optional.iter()) {
        let Some(key) = key.as_str() else {
            return Err(OpError::Internal(
                "schema parameter entries must be strings".into(),
            ));
        };
        declared.insert(key);
    }
    if params.keys().any(|key| !declared.contains(key.as_str())) {
        return Err(OpError::Validation("undeclared operation parameter".into()));
    }
    for key in required {
        let Some(key) = key.as_str() else {
            return Err(OpError::Validation(
                "schema required entries must be strings".to_string(),
            ));
        };
        if params.get(key).is_none() {
            return Err(OpError::Validation(format!(
                "missing required param: {key}"
            )));
        }
    }
    Ok(())
}

fn audit_result(result: &Result<DispatchResult, OpError>) -> String {
    match result {
        Ok(_) => "ok".to_string(),
        Err(err) => format!("error:{}", error_code(err)),
    }
}

fn error_code(err: &OpError) -> &'static str {
    match err {
        OpError::NotFound => "not_found",
        OpError::Forbidden(_) => "forbidden",
        OpError::StepUpRequired => "step_up_required",
        OpError::PreconditionFailed(_) => "precondition_failed",
        OpError::Validation(_) => "validation",
        OpError::Internal(_) => "internal",
    }
}

fn actor_kind_str(kind: ActorKind) -> &'static str {
    match kind {
        ActorKind::Human => "human",
        ActorKind::Ai => "ai",
        ActorKind::LocalCli => "local_cli",
    }
}
