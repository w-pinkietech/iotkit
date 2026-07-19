//! Edge-private compile-time factory catalog for official input adapter packages.
//!
//! Adapter-specific config parsing, validation, inventory, and startup stay behind this
//! composition-root boundary. The rest of Edge only handles a validated, type-erased instance.

use std::any::Any;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use iotkit_input_adapter_host_api::{
    AdapterInstanceId, AdapterStartContext, ConfiguredSource, InputAdapterTypeDescriptor,
    RunningInputAdapter,
};

use crate::config::RawInputAdapterInstance;

type ErasedConfig = Arc<dyn Any + Send + Sync>;

struct InputAdapterFactory {
    descriptor: fn() -> InputAdapterTypeDescriptor,
    parse_and_validate: fn(&RawInputAdapterInstance) -> Result<ErasedConfig, String>,
    start: fn(AdapterStartContext, &dyn Any) -> Result<RunningInputAdapter, String>,
    positional_inventory: fn(&ConfiguredSource, &dyn Any) -> Vec<PositionalInventoryItem>,
}

static BRAVEPI_FACTORY: InputAdapterFactory = InputAdapterFactory {
    descriptor: bravepi_mainboard_adapter::task::descriptor,
    parse_and_validate: parse_bravepi,
    start: start_bravepi,
    positional_inventory: no_positional_inventory,
};

static RPI_LOCAL_FACTORY: InputAdapterFactory = InputAdapterFactory {
    descriptor: rpi_local_adapter::descriptor,
    parse_and_validate: parse_rpi_local,
    start: start_rpi_local,
    positional_inventory: rpi_local_inventory,
};

fn catalog() -> [&'static InputAdapterFactory; 2] {
    [&BRAVEPI_FACTORY, &RPI_LOCAL_FACTORY]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionalInventoryItem {
    pub hardware_id: String,
    pub label: String,
}

#[derive(Clone)]
pub struct PreparedInputAdapter {
    instance_id: AdapterInstanceId,
    source: ConfiguredSource,
    factory: &'static InputAdapterFactory,
    config: ErasedConfig,
}

impl fmt::Debug for PreparedInputAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedInputAdapter")
            .field("instance_id", &self.instance_id)
            .field("source", &self.source)
            .field("adapter_type", &self.adapter_type())
            .finish_non_exhaustive()
    }
}

impl PreparedInputAdapter {
    pub fn instance_id(&self) -> &AdapterInstanceId {
        &self.instance_id
    }

    pub fn source(&self) -> &ConfiguredSource {
        &self.source
    }

    pub fn adapter_type(&self) -> String {
        ((self.factory.descriptor)())
            .adapter_type_id
            .as_str()
            .to_owned()
    }

    pub fn positional_inventory(&self) -> Vec<PositionalInventoryItem> {
        (self.factory.positional_inventory)(&self.source, self.config.as_ref())
    }

    pub fn start(&self, context: AdapterStartContext) -> Result<RunningInputAdapter, String> {
        (self.factory.start)(context, self.config.as_ref())
    }
}

pub(crate) fn resolve_instance(
    raw_id: String,
    raw: RawInputAdapterInstance,
) -> Result<Option<PreparedInputAdapter>, String> {
    if !raw.enabled.unwrap_or(true) {
        return Ok(None);
    }
    let instance_id = AdapterInstanceId::new(raw_id).map_err(|error| error.to_string())?;
    let source = ConfiguredSource::new(raw.source.clone()).map_err(|error| error.to_string())?;
    let factory = catalog()
        .into_iter()
        .find(|factory| {
            ((factory.descriptor)()).adapter_type_id.as_str() == raw.adapter_type.as_str()
        })
        .ok_or_else(|| {
            format!(
                "adapter {} has unknown type {:?}",
                instance_id, raw.adapter_type
            )
        })?;
    let descriptor = (factory.descriptor)();
    if raw.config_schema_version != descriptor.config_schema_version {
        return Err(format!(
            "adapter {} config_schema_version must be {}",
            instance_id, descriptor.config_schema_version
        ));
    }
    let config = (factory.parse_and_validate)(&raw)
        .map_err(|error| format!("adapter {instance_id}: {error}"))?;
    Ok(Some(PreparedInputAdapter {
        instance_id,
        source,
        factory,
        config,
    }))
}

pub fn validate_catalog() -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for factory in catalog() {
        let descriptor = (factory.descriptor)();
        if !ids.insert(descriptor.adapter_type_id.as_str().to_owned()) {
            return Err(format!(
                "duplicate input adapter type {}",
                descriptor.adapter_type_id
            ));
        }
        if descriptor.adapter_api_major != 1 {
            return Err(format!(
                "input adapter {} uses unsupported API major {}",
                descriptor.adapter_type_id, descriptor.adapter_api_major
            ));
        }
    }
    Ok(())
}

fn parse_bravepi(raw: &RawInputAdapterInstance) -> Result<ErasedConfig, String> {
    if raw.bus_path.is_some() || raw.poll_interval_ms.is_some() {
        return Err("has rpi-local-only fields".into());
    }
    let port = raw
        .port
        .clone()
        .ok_or_else(|| "requires port".to_string())?;
    if port.is_empty() {
        return Err("port must not be empty".into());
    }
    Ok(Arc::new(port))
}

fn start_bravepi(
    context: AdapterStartContext,
    config: &dyn Any,
) -> Result<RunningInputAdapter, String> {
    let port = config
        .downcast_ref::<String>()
        .expect("BravePI factory owns validated String config");
    bravepi_mainboard_adapter::task::start_host(context, port.clone())
        .map_err(|error| format!("failed to start BravePI adapter: {error}"))
}

fn no_positional_inventory(
    _source: &ConfiguredSource,
    _config: &dyn Any,
) -> Vec<PositionalInventoryItem> {
    Vec::new()
}

fn rpi_local_targets() -> Vec<rpi_local_adapter::RpiLocalTarget> {
    rpi_local_adapter::built_in_targets()
}

fn parse_rpi_local(raw: &RawInputAdapterInstance) -> Result<ErasedConfig, String> {
    if raw.port.is_some() {
        return Err("has bravepi-mainboard-only fields".into());
    }
    let bus_path = raw
        .bus_path
        .clone()
        .ok_or_else(|| "requires bus_path".to_string())?;
    let poll_interval_ms = raw
        .poll_interval_ms
        .ok_or_else(|| "requires poll_interval_ms".to_string())?;
    let config = rpi_local_adapter::RpiLocalConfig {
        bus_path,
        poll_interval_ms,
        targets: rpi_local_targets(),
    };
    rpi_local_adapter::validate(&config)
        .map_err(|error| format!("invalid rpi-local config: {error}"))?;
    Ok(Arc::new(config))
}

fn start_rpi_local(
    context: AdapterStartContext,
    config: &dyn Any,
) -> Result<RunningInputAdapter, String> {
    let config = config
        .downcast_ref::<rpi_local_adapter::RpiLocalConfig>()
        .expect("RPi local factory owns validated RpiLocalConfig")
        .clone();
    rpi_local_adapter::start_host(context, config)
        .map_err(|error| format!("failed to start RPi local adapter: {error}"))
}

fn rpi_local_inventory(
    source: &ConfiguredSource,
    config: &dyn Any,
) -> Vec<PositionalInventoryItem> {
    let config = config
        .downcast_ref::<rpi_local_adapter::RpiLocalConfig>()
        .expect("RPi local factory owns validated RpiLocalConfig");
    rpi_local_adapter::positional_inventory(config)
        .into_iter()
        .map(|device| PositionalInventoryItem {
            hardware_id: format!("{}:{}", source.as_str(), device.locator),
            label: device.label,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(adapter_type: &str) -> RawInputAdapterInstance {
        RawInputAdapterInstance {
            adapter_type: adapter_type.into(),
            enabled: Some(true),
            config_schema_version: 1,
            source: "input:test:line_a".into(),
            port: None,
            bus_path: None,
            poll_interval_ms: None,
        }
    }

    #[test]
    fn built_in_catalog_is_unique_and_uses_host_api_v1() {
        validate_catalog().unwrap();
        let ids: Vec<_> = catalog()
            .into_iter()
            .map(|factory| ((factory.descriptor)()).adapter_type_id.as_str().to_owned())
            .collect();
        assert_eq!(ids, ["bravepi-mainboard", "rpi-local"]);
    }

    #[test]
    fn rpi_factory_validates_driver_limits_before_returning_prepared_instance() {
        let mut raw = raw("rpi-local");
        raw.bus_path = Some("/dev/i2c-1".into());
        raw.poll_interval_ms = Some(50);
        let error = resolve_instance("line_a".into(), raw).unwrap_err();
        assert!(error.contains("poll_interval_ms"));
    }

    #[test]
    fn rpi_inventory_and_runtime_share_the_same_validated_targets() {
        let mut raw = raw("rpi-local");
        raw.bus_path = Some("/dev/i2c-1".into());
        raw.poll_interval_ms = Some(1_000);
        let prepared = resolve_instance("line_a".into(), raw).unwrap().unwrap();
        let inventory = prepared.positional_inventory();
        assert_eq!(inventory.len(), 2);
        assert_eq!(inventory[0].hardware_id, "input:test:line_a:i2c:0x60");
        assert_eq!(inventory[1].hardware_id, "input:test:line_a:i2c:0x44");
    }
}
