//! Typed operations over device-local pipeline definitions (#232 child issue 3).
//!
//! The Console and `nodectl pipeline` mutate definitions only through these
//! operations. Each execute runs inside the dispatcher's transaction, so the
//! definition, the series state, and the outbox rows commit together.

use iotkit_core_pipeline::{EngineError, PipelineDefinition, PipelineEngine, SeriesStart};
use iotkit_core_types::PipelineId;
use rusqlite::Transaction;
use serde_json::{Value, json};

use crate::{OpContext, OpDescriptor, OpError, Tier};

use super::required_str;

pub const CREATE: &str = "pipeline.create";
pub const UPDATE: &str = "pipeline.update";
pub const DELETE: &str = "pipeline.delete";
pub const RESET: &str = "pipeline.reset";
pub const IMPORT: &str = "pipeline.import";

pub fn descriptors() -> Vec<OpDescriptor> {
    vec![
        OpDescriptor {
            name: CREATE,
            tier: Tier::Daily,
            bulk_escalates: false,
            changes_state: true,
            params_schema: || json!({ "required": ["definition"] }),
            targets: definition_target,
            preconditions: create_preconditions,
            dry_run: create_dry_run,
            execute: create_execute,
            secret_execute: None,
        },
        OpDescriptor {
            name: UPDATE,
            tier: Tier::Daily,
            bulk_escalates: false,
            changes_state: true,
            params_schema: || json!({ "required": ["definition"] }),
            targets: definition_target,
            preconditions: update_preconditions,
            dry_run: update_dry_run,
            execute: update_execute,
            secret_execute: None,
        },
        OpDescriptor {
            name: DELETE,
            tier: Tier::Daily,
            bulk_escalates: false,
            changes_state: true,
            params_schema: || json!({ "required": ["id"] }),
            targets: id_target,
            preconditions: existing_preconditions,
            dry_run: |_, ctx| Ok(json!({ "would": "delete", "id": required_id(ctx.params)? })),
            execute: delete_execute,
            secret_execute: None,
        },
        OpDescriptor {
            name: RESET,
            tier: Tier::Daily,
            bulk_escalates: false,
            changes_state: true,
            params_schema: || json!({ "required": ["id"] }),
            targets: id_target,
            preconditions: existing_preconditions,
            dry_run: |_, ctx| Ok(json!({ "would": "reset", "id": required_id(ctx.params)? })),
            execute: reset_execute,
            secret_execute: None,
        },
        OpDescriptor {
            name: IMPORT,
            tier: Tier::Construction,
            bulk_escalates: false,
            changes_state: true,
            params_schema: || json!({ "required": ["pipelines"] }),
            targets: |params| {
                definitions(params)
                    .map(|definitions| definitions.iter().map(|d| d.id.to_string()).collect())
                    .unwrap_or_default()
            },
            preconditions: import_preconditions,
            dry_run: |_, ctx| {
                let definitions = definitions(ctx.params)?;
                Ok(json!({
                    "would": "replace_all_and_restart_every_series",
                    "pipelines": definitions.iter().map(|d| d.id.to_string()).collect::<Vec<_>>(),
                }))
            },
            execute: import_execute,
            secret_execute: None,
        },
    ]
}

fn engine(tx: &Transaction<'_>) -> Result<PipelineEngine, OpError> {
    PipelineEngine::load(tx)
        .map_err(|error| OpError::Internal(error.to_string()))?
        .ok_or_else(|| {
            OpError::PreconditionFailed(
                "edge-node-id is not recorded yet; start iotkit-edge-node once with this database"
                    .into(),
            )
        })
}

fn definition(params: &Value) -> Result<PipelineDefinition, OpError> {
    let definition = params
        .get("definition")
        .ok_or_else(|| OpError::Validation("definition must be an object".into()))?;
    let definition: PipelineDefinition = serde_json::from_value(definition.clone())
        .map_err(|error| OpError::Validation(format!("definition: {error}")))?;
    definition
        .validate()
        .map_err(|error| OpError::Validation(error.to_string()))?;
    Ok(definition)
}

fn definitions(params: &Value) -> Result<Vec<PipelineDefinition>, OpError> {
    let pipelines = params
        .get("pipelines")
        .ok_or_else(|| OpError::Validation("pipelines must be an array".into()))?;
    let definitions: Vec<PipelineDefinition> = serde_json::from_value(pipelines.clone())
        .map_err(|error| OpError::Validation(format!("pipelines: {error}")))?;
    for definition in &definitions {
        definition
            .validate()
            .map_err(|error| OpError::Validation(error.to_string()))?;
    }
    Ok(definitions)
}

fn required_id(params: &Value) -> Result<PipelineId, OpError> {
    required_str(params, "id")?
        .parse()
        .map_err(|error| OpError::Validation(format!("id {error}")))
}

fn definition_target(params: &Value) -> Vec<String> {
    params
        .get("definition")
        .and_then(|definition| definition.get("id"))
        .and_then(Value::as_str)
        .map(|id| vec![id.to_string()])
        .unwrap_or_default()
}

fn id_target(params: &Value) -> Vec<String> {
    params
        .get("id")
        .and_then(Value::as_str)
        .map(|id| vec![id.to_string()])
        .unwrap_or_default()
}

fn engine_error(error: EngineError) -> OpError {
    match error {
        EngineError::Validation(error) => OpError::Validation(error.to_string()),
        EngineError::AlreadyExists(_) | EngineError::NotFound(_) | EngineError::KindChanged => {
            OpError::PreconditionFailed(error.to_string())
        }
        other => OpError::Internal(other.to_string()),
    }
}

fn series_json(start: &SeriesStart) -> Value {
    json!({
        "id": start.pipeline_id.to_string(),
        "series_id": start.series_id,
        "published_sequence": start.published.as_ref().map(|o| o.sequence),
    })
}

fn create_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    let definition = definition(ctx.params)?;
    engine(tx)?;
    if iotkit_core_pipeline::store::get_definition(tx, &definition.id)
        .map_err(|error| OpError::Internal(error.to_string()))?
        .is_some()
    {
        return Err(OpError::PreconditionFailed(format!(
            "pipeline {} already exists",
            definition.id
        )));
    }
    Ok(())
}

fn create_dry_run(_tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let definition = definition(ctx.params)?;
    Ok(json!({
        "would": "create_and_start_series",
        "id": definition.id.to_string(),
        "kind": definition.kind.key(),
    }))
}

fn create_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let definition = definition(ctx.params)?;
    let start = engine(tx)?
        .create(tx, &definition, now_ms())
        .map_err(engine_error)?;
    Ok(series_json(&start))
}

fn update_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    let definition = definition(ctx.params)?;
    engine(tx)?;
    let existing = iotkit_core_pipeline::store::get_definition(tx, &definition.id)
        .map_err(|error| OpError::Internal(error.to_string()))?
        .ok_or(OpError::NotFound)?;
    if existing.kind != definition.kind {
        return Err(OpError::PreconditionFailed(
            EngineError::KindChanged.to_string(),
        ));
    }
    Ok(())
}

fn update_dry_run(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let definition = definition(ctx.params)?;
    let existing = iotkit_core_pipeline::store::get_definition(tx, &definition.id)
        .map_err(|error| OpError::Internal(error.to_string()))?
        .ok_or(OpError::NotFound)?;
    Ok(json!({
        "would": if existing.structural_hash() == definition.structural_hash() {
            "update_keeping_series"
        } else {
            "update_and_start_new_series"
        },
        "id": definition.id.to_string(),
    }))
}

fn update_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let definition = definition(ctx.params)?;
    let started = engine(tx)?
        .update(tx, &definition, now_ms())
        .map_err(engine_error)?;
    Ok(json!({
        "id": definition.id.to_string(),
        "new_series": started.as_ref().map(series_json),
    }))
}

fn existing_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    let id = required_id(ctx.params)?;
    engine(tx)?;
    iotkit_core_pipeline::store::get_definition(tx, &id)
        .map_err(|error| OpError::Internal(error.to_string()))?
        .ok_or(OpError::NotFound)?;
    Ok(())
}

fn delete_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let id = required_id(ctx.params)?;
    engine(tx)?
        .delete(tx, &id, now_ms())
        .map_err(engine_error)?;
    Ok(json!({ "id": id.to_string(), "deleted": true }))
}

fn reset_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let id = required_id(ctx.params)?;
    let start = engine(tx)?.reset(tx, &id, now_ms()).map_err(engine_error)?;
    Ok(series_json(&start))
}

fn import_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    definitions(ctx.params)?;
    engine(tx)?;
    Ok(())
}

fn import_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let definitions = definitions(ctx.params)?;
    let started = engine(tx)?
        .import(tx, &definitions, now_ms())
        .map_err(engine_error)?;
    Ok(json!({
        "imported": started.len(),
        "series": started.iter().map(series_json).collect::<Vec<_>>(),
    }))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}
