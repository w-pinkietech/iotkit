use std::sync::OnceLock;

use serde_json::Value;

use crate::{OpDescriptor, OpError};

mod commissioning_ops;
mod credential_ops;
mod device_ops;
mod ingress_listener_ops;
mod registry_ops;
mod token_ops;

pub fn standard_catalog() -> &'static [OpDescriptor] {
    static CATALOG: OnceLock<Vec<OpDescriptor>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let mut catalog = vec![
                registry_ops::resolve_unknown_key_descriptor(),
                commissioning_ops::enqueue_smoke_descriptor(),
                device_ops::approve_sighting_descriptor(),
                device_ops::pin_sighting_descriptor(),
                device_ops::retire_descriptor(),
                token_ops::issue_descriptor(),
                token_ops::revoke_descriptor(),
                ingress_listener_ops::configure_descriptor(),
                ingress_listener_ops::disable_descriptor(),
                ingress_listener_ops::rotate_tls_descriptor(),
            ];
            catalog.extend(credential_ops::descriptors());
            debug_assert!(catalog.iter().all(|op| op.tier != crate::Tier::ReadOnly));
            catalog
        })
        .as_slice()
}

fn required_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, OpError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| OpError::Validation(format!("{key} must be a string")))
}

fn required_string_array(params: &Value, key: &str) -> Result<Vec<String>, OpError> {
    let values = params
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| OpError::Validation(format!("{key} must be an array")))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| OpError::Validation(format!("{key} entries must be strings")))
        })
        .collect()
}

fn target_string_array(params: &Value, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn required_object<'a>(
    params: &'a Value,
    key: &str,
) -> Result<&'a serde_json::Map<String, Value>, OpError> {
    params
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| OpError::Validation(format!("{key} must be an object")))
}
