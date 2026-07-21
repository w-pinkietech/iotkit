use std::time::{SystemTime, UNIX_EPOCH};

use iotkit_core_publish::store::{enqueue_commissioning_smoke, target_get};
use rusqlite::Transaction;
use serde_json::{Value, json};

use crate::{OpContext, OpDescriptor, OpError, Tier};

const EDGE_TARGET_ID: &str = "edge";

pub fn enqueue_smoke_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: "exit.commissioning_smoke.enqueue",
        tier: Tier::Daily,
        bulk_escalates: false,
        changes_state: true,
        params_schema: || json!({"required": []}),
        targets: |_| vec![EDGE_TARGET_ID.into()],
        preconditions,
        dry_run,
        execute,
        secret_execute: None,
    }
}

fn preconditions(tx: &Transaction<'_>, _ctx: &OpContext<'_>) -> Result<(), OpError> {
    if !iotkit_core_publish::activation::publication_admitted(tx).map_err(publish_error)? {
        return Err(OpError::PreconditionFailed(
            "Edge Node activation is required before commissioning smoke".into(),
        ));
    }
    let target = target_get(tx).map_err(publish_error)?;
    match target {
        Some(target)
            if target.target_id == EDGE_TARGET_ID
                && target.archive_responsible
                && target.credential_token.is_empty() =>
        {
            Ok(())
        }
        Some(_) => Err(OpError::PreconditionFailed(
            "configured exit target is not the MQTT IoTKit Edge target".into(),
        )),
        None => Err(OpError::PreconditionFailed(
            "MQTT IoTKit Edge target is not initialized; start Edge Node with MQTT exit enabled first".into(),
        )),
    }
}

fn dry_run(tx: &Transaction<'_>, _ctx: &OpContext<'_>) -> Result<Value, OpError> {
    Ok(json!({
        "would": "enqueue_commissioning_smoke",
        "target_id": EDGE_TARGET_ID,
        "ledger_epoch": iotkit_core_ledger::ledger_epoch(tx)?,
    }))
}

fn execute(tx: &Transaction<'_>, _ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let ledger_epoch = iotkit_core_ledger::ledger_epoch(tx)?;
    let test_id = new_test_id()?;
    let pub_seq = enqueue_commissioning_smoke(tx, &ledger_epoch, &test_id, now_ms())
        .map_err(publish_error)?;
    Ok(json!({
        "test_id": test_id,
        "target_id": EDGE_TARGET_ID,
        "ledger_epoch": ledger_epoch,
        "pub_seq": pub_seq,
    }))
}

fn new_test_id() -> Result<String, OpError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| OpError::Internal("commissioning smoke random generation failed".into()))?;
    let mut encoded = String::with_capacity(38);
    encoded.push_str("smoke-");
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}")
            .map_err(|_| OpError::Internal("commissioning smoke ID encoding failed".into()))?;
    }
    Ok(encoded)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn publish_error(error: iotkit_core_publish::PublishError) -> OpError {
    match error {
        iotkit_core_publish::PublishError::Invalid(message) => OpError::Validation(message),
        other => OpError::Internal(other.to_string()),
    }
}
