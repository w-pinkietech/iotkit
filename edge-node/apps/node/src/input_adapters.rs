//! Edge Node-private compile-time factory catalog for official input adapter packages.
//!
//! Adapter-specific config parsing, validation, inventory, and startup stay behind this
//! composition-root boundary. The rest of Edge Node only handles a validated, type-erased instance.

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

/// Why an adapter instance did not start. `io_kind` is kept so the status
/// topic can report `interface-open-failed` with the contract's `reason`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterStartError {
    pub io_kind: Option<std::io::ErrorKind>,
    pub message: String,
}

impl fmt::Display for AdapterStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AdapterStartError {}

impl AdapterStartError {
    fn io(prefix: &str, error: std::io::Error) -> Self {
        Self {
            io_kind: Some(error.kind()),
            message: format!("{prefix}: {error}"),
        }
    }

    fn other(prefix: &str, error: impl fmt::Display) -> Self {
        Self {
            io_kind: None,
            message: format!("{prefix}: {error}"),
        }
    }
}

struct InputAdapterFactory {
    descriptor: fn() -> InputAdapterTypeDescriptor,
    parse_and_validate: fn(&RawInputAdapterInstance) -> Result<ErasedConfig, String>,
    start: fn(AdapterStartContext, &dyn Any) -> Result<RunningInputAdapter, AdapterStartError>,
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

static TRIAL_SAMPLE_FACTORY: InputAdapterFactory = InputAdapterFactory {
    descriptor: trial_sample_adapter::descriptor,
    parse_and_validate: parse_trial_sample,
    start: start_trial_sample,
    positional_inventory: trial_sample_inventory,
};

fn catalog() -> [&'static InputAdapterFactory; 3] {
    [&BRAVEPI_FACTORY, &RPI_LOCAL_FACTORY, &TRIAL_SAMPLE_FACTORY]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionalInventoryItem {
    pub hardware_id: String,
    pub model_id: String,
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

    pub fn start(
        &self,
        context: AdapterStartContext,
    ) -> Result<RunningInputAdapter, AdapterStartError> {
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
    if raw.bus_path.is_some() || raw.poll_interval_ms.is_some() || raw.devices.is_some() {
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
) -> Result<RunningInputAdapter, AdapterStartError> {
    let port = config
        .downcast_ref::<String>()
        .expect("BravePI factory owns validated String config");
    bravepi_mainboard_adapter::task::start_host(context, port.clone())
        .map_err(|error| AdapterStartError::io("failed to start BravePI adapter", error))
}

fn no_positional_inventory(
    _source: &ConfiguredSource,
    _config: &dyn Any,
) -> Vec<PositionalInventoryItem> {
    Vec::new()
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
    let targets = match &raw.devices {
        None => rpi_local_adapter::built_in_targets(),
        Some(devices) => devices
            .iter()
            .map(|device| {
                let settings = device
                    .settings
                    .iter()
                    .map(|(name, value)| (name.clone(), value.to_host_value()))
                    .collect();
                rpi_local_adapter::configured_target(&device.model, device.address, &settings)
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    let config = rpi_local_adapter::RpiLocalConfig {
        bus_path,
        poll_interval_ms,
        targets,
    };
    rpi_local_adapter::validate(&config)
        .map_err(|error| format!("invalid rpi-local config: {error}"))?;
    Ok(Arc::new(config))
}

fn start_rpi_local(
    context: AdapterStartContext,
    config: &dyn Any,
) -> Result<RunningInputAdapter, AdapterStartError> {
    let config = config
        .downcast_ref::<rpi_local_adapter::RpiLocalConfig>()
        .expect("RPi local factory owns validated RpiLocalConfig")
        .clone();
    rpi_local_adapter::start_host(context, config)
        .map_err(|error| AdapterStartError::io("failed to start RPi local adapter", error))
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
            model_id: device.model_id,
            label: device.label,
        })
        .collect()
}

fn parse_trial_sample(raw: &RawInputAdapterInstance) -> Result<ErasedConfig, String> {
    if std::env::var_os(trial_sample_adapter::ENABLE_ENV).as_deref()
        != Some(std::ffi::OsStr::new("1"))
    {
        return Err(format!(
            "requires {}=1 (trial profile only; refuse field enablement)",
            trial_sample_adapter::ENABLE_ENV
        ));
    }
    if raw.port.is_some() || raw.bus_path.is_some() || raw.devices.is_some() {
        return Err("has non trial-sample-only fields".into());
    }
    let config = trial_sample_adapter::TrialSampleConfig {
        poll_interval_ms: raw
            .poll_interval_ms
            .ok_or_else(|| "requires poll_interval_ms".to_string())?,
    };
    trial_sample_adapter::validate(config)
        .map_err(|error| format!("invalid trial-sample config: {error}"))?;
    Ok(Arc::new(config))
}

fn start_trial_sample(
    context: AdapterStartContext,
    config: &dyn Any,
) -> Result<RunningInputAdapter, AdapterStartError> {
    let config = *config
        .downcast_ref::<trial_sample_adapter::TrialSampleConfig>()
        .expect("trial sample factory owns validated TrialSampleConfig");
    trial_sample_adapter::start_host(context, config)
        .map_err(|error| AdapterStartError::other("failed to start trial sample adapter", error))
}

fn trial_sample_inventory(
    source: &ConfiguredSource,
    _config: &dyn Any,
) -> Vec<PositionalInventoryItem> {
    trial_sample_adapter::inventory_items(source.as_str())
        .into_iter()
        .map(|item| PositionalInventoryItem {
            hardware_id: item.hardware_id,
            model_id: item.model_id,
            label: item.label,
        })
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/input_adapters_tests.rs"]
mod tests;
