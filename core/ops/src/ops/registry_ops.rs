use iotkit_core_registry::{
    AliasKind, ChannelMode, CustomEntrySpec, ValueType, define_alias, define_custom_entry,
    find_resolution, get_entry, list_aliases, validate_custom_entry_spec, validate_measurement_key,
};
use rusqlite::Transaction;
use serde_json::{Value, json};

use crate::{OpContext, OpDescriptor, OpError, Tier};

use super::{required_object, required_str};

pub fn resolve_unknown_key_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: "registry.resolve_unknown_key",
        tier: Tier::Daily,
        bulk_escalates: false,
        changes_state: true,
        params_schema,
        targets,
        preconditions,
        dry_run,
        execute,
        secret_execute: None,
    }
}

fn params_schema() -> Value {
    json!({ "required": ["key", "resolution"] })
}

fn targets(params: &Value) -> Vec<String> {
    params
        .get("key")
        .and_then(Value::as_str)
        .map(|key| vec![key.to_string()])
        .unwrap_or_default()
}

fn preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    let key = required_str(ctx.params, "key")?;
    validate_measurement_key(key)?;
    if find_resolution(tx, key)?.is_some() {
        return Err(OpError::PreconditionFailed(format!(
            "key already resolved: {key}"
        )));
    }
    match resolution(ctx.params)? {
        ResolutionSpec::Alias { target } => {
            validate_measurement_key(&target)?;
            get_entry(tx, &target)?.ok_or(OpError::NotFound)?;
        }
        ResolutionSpec::Custom { spec } => {
            validate_custom_entry_spec(&spec)?;
            if get_entry(tx, &spec.measurement_key)?.is_some()
                || list_aliases(tx)?
                    .iter()
                    .any(|alias| alias.alias == spec.measurement_key)
            {
                return Err(OpError::PreconditionFailed(format!(
                    "custom entry collides: {}",
                    spec.measurement_key
                )));
            }
        }
    }
    Ok(())
}

fn dry_run(_tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let key = required_str(ctx.params, "key")?;
    match resolution(ctx.params)? {
        ResolutionSpec::Alias { target } => Ok(json!({
            "would": "define_alias",
            "key": key,
            "target": target,
        })),
        ResolutionSpec::Custom { spec } => Ok(json!({
            "would": "define_custom_entry_and_alias",
            "key": key,
            "measurement_key": spec.measurement_key,
        })),
    }
}

fn execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let key = required_str(ctx.params, "key")?;
    match resolution(ctx.params)? {
        ResolutionSpec::Alias { target } => {
            define_alias(tx, key, &target, AliasKind::SiteMapping)?;
            Ok(json!({
                "alias": key,
                "target": target,
            }))
        }
        ResolutionSpec::Custom { spec } => {
            let entry = define_custom_entry(tx, &spec)?;
            define_alias(tx, key, &entry.measurement_key, AliasKind::SiteMapping)?;
            Ok(json!({
                "alias": key,
                "target": entry.measurement_key,
                "origin": entry.origin,
            }))
        }
    }
}

enum ResolutionSpec {
    Alias { target: String },
    Custom { spec: CustomEntrySpec },
}

fn resolution(params: &Value) -> Result<ResolutionSpec, OpError> {
    let resolution = required_object(params, "resolution")?;
    reject_undeclared(resolution, &["alias_to", "custom"])?;
    match (resolution.get("alias_to"), resolution.get("custom")) {
        (Some(_), Some(_)) => Err(OpError::Validation(
            "resolution must contain exactly one of alias_to or custom".to_string(),
        )),
        (None, None) => Err(OpError::Validation(
            "resolution must contain exactly one of alias_to or custom".to_string(),
        )),
        (Some(target), None) => {
            let target = target.as_str().ok_or_else(|| {
                OpError::Validation("resolution.alias_to must be a string".to_string())
            })?;
            Ok(ResolutionSpec::Alias {
                target: target.to_string(),
            })
        }
        (None, Some(custom)) => {
            let custom = custom.as_object().ok_or_else(|| {
                OpError::Validation("resolution.custom must be an object".to_string())
            })?;
            Ok(ResolutionSpec::Custom {
                spec: custom_spec(custom)?,
            })
        }
    }
}

fn custom_spec(custom: &serde_json::Map<String, Value>) -> Result<CustomEntrySpec, OpError> {
    reject_undeclared(
        custom,
        &[
            "measurement_key",
            "unit_ucum",
            "unit_display",
            "value_type",
            "semantic_class",
            "channel_mode",
            "channel_roles",
            "physical_min",
            "physical_max",
        ],
    )?;
    let value_type = match required_field(custom, "value_type")? {
        "float" => ValueType::Float,
        "int" => ValueType::Int,
        "bool" => ValueType::Bool,
        "record" => ValueType::Record,
        other => return Err(OpError::Validation(format!("unknown value_type: {other}"))),
    };
    let channel_mode = match required_field(custom, "channel_mode")? {
        "single" => ChannelMode::Single,
        "generic" => ChannelMode::Generic,
        "fixed" => ChannelMode::Fixed,
        other => {
            return Err(OpError::Validation(format!(
                "unknown channel_mode: {other}"
            )));
        }
    };
    Ok(CustomEntrySpec {
        measurement_key: required_field(custom, "measurement_key")?.to_string(),
        unit_ucum: optional_string(custom, "unit_ucum")?,
        unit_display: optional_string(custom, "unit_display")?,
        value_type,
        semantic_class: required_field(custom, "semantic_class")?.to_string(),
        channel_mode,
        channel_roles: optional_string_array(custom, "channel_roles")?,
        physical_min: optional_f64(custom, "physical_min")?,
        physical_max: optional_f64(custom, "physical_max")?,
    })
}

fn reject_undeclared(
    object: &serde_json::Map<String, Value>,
    allowed: &[&str],
) -> Result<(), OpError> {
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(OpError::Validation("undeclared operation parameter".into()));
    }
    Ok(())
}

fn required_field<'a>(
    custom: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, OpError> {
    custom
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| OpError::Validation(format!("custom.{key} must be a string")))
}

fn optional_string(
    custom: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, OpError> {
    custom
        .get(key)
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| OpError::Validation(format!("custom.{key} must be a string")))
        })
        .transpose()
}

fn optional_string_array(
    custom: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, OpError> {
    let Some(value) = custom.get(key) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    value
        .as_array()
        .ok_or_else(|| OpError::Validation(format!("custom.{key} must be an array")))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| OpError::Validation(format!("custom.{key} entries must be strings")))
        })
        .collect()
}

fn optional_f64(
    custom: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<f64>, OpError> {
    custom
        .get(key)
        .map(|value| {
            if value.is_null() {
                Ok(None)
            } else {
                value
                    .as_f64()
                    .map(Some)
                    .ok_or_else(|| OpError::Validation(format!("custom.{key} must be a number")))
            }
        })
        .transpose()
        .map(Option::flatten)
}
