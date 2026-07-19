use iotkit_core_types::{SensorReading, SensorType};
use iotkit_polling_adapter_runtime::SensorDriver;
use rpi4b_transport::i2c::I2cDeviceFactory;
use std::sync::Arc;

use crate::{RpiLocalTarget, ThermocoupleType};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeasurementProjection {
    pub measurement_key: &'static str,
    pub channel_index: Option<u16>,
    pub values: Vec<f64>,
}

impl RpiLocalTarget {
    pub fn device_model_id(&self) -> &'static str {
        match self {
            Self::MCP9600 { .. } => "mcp9600",
            Self::OPT3001 { .. } => "opt3001",
        }
    }

    pub fn address(&self) -> u8 {
        match self {
            Self::MCP9600 { address, .. } | Self::OPT3001 { address } => *address,
        }
    }

    pub fn inventory_label(&self) -> &'static str {
        match self {
            Self::MCP9600 { .. } => "MCP9600 thermocouple",
            Self::OPT3001 { .. } => "OPT3001 illuminance",
        }
    }

    pub(crate) fn build_driver(
        &self,
        device_factory: Arc<dyn I2cDeviceFactory>,
    ) -> Arc<dyn SensorDriver> {
        match self {
            Self::MCP9600 {
                thermocouple_type, ..
            } => Arc::new(crate::drivers::mcp9600::Mcp9600Driver {
                thermocouple_type: *thermocouple_type,
                device_factory,
            }),
            Self::OPT3001 { .. } => {
                Arc::new(crate::drivers::opt3001::Opt3001Driver { device_factory })
            }
        }
    }

    pub(crate) fn matches_device_key(&self, device_key: &str) -> bool {
        device_key == format!("i2c:0x{:02x}:{}", self.address(), self.device_model_id())
    }

    pub(crate) fn project(&self, reading: &SensorReading) -> Result<MeasurementProjection, String> {
        let (expected_type, expected_unit, measurement_key) = match self {
            Self::MCP9600 { .. } => (SensorType::Temperature, "celsius", "temperature_c"),
            Self::OPT3001 { .. } => (SensorType::Illuminance, "lux", "illuminance_lux"),
        };
        if reading.sensor_type != expected_type {
            return Err(format!(
                "{} produced unexpected sensor type {}",
                self.device_model_id(),
                reading.sensor_type
            ));
        }
        if reading.values.len() != 1 {
            return Err(format!(
                "{} produced {} values; expected 1",
                self.device_model_id(),
                reading.values.len()
            ));
        }
        if reading.labels.as_slice() != [expected_unit] {
            return Err(format!(
                "{} produced unexpected units {:?}; expected {:?}",
                self.device_model_id(),
                reading.labels,
                [expected_unit]
            ));
        }
        Ok(MeasurementProjection {
            measurement_key,
            channel_index: None,
            values: reading.values.clone(),
        })
    }
}

pub fn built_in_targets() -> Vec<RpiLocalTarget> {
    vec![
        RpiLocalTarget::MCP9600 {
            address: 0x60,
            thermocouple_type: ThermocoupleType::K,
        },
        RpiLocalTarget::OPT3001 { address: 0x44 },
    ]
}
