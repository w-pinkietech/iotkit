//! OPT3001 SensorDriver implementation.

use std::collections::BTreeMap;
use std::sync::Arc;

use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading};
use iotkit_polling_adapter_runtime::SensorDriver;
use iotkit_sensor_drivers::opt3001;
use rpi4b_transport::i2c::I2cDeviceFactory;

pub struct Opt3001Driver {
    pub device_factory: Arc<dyn I2cDeviceFactory>,
}

impl SensorDriver for Opt3001Driver {
    fn detect(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String> {
        let mut device = self
            .device_factory
            .open(bus_path, address as u16)
            .map_err(|e| format!("OPT3001 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let mut id_buf = [0u8; 2];
        device
            .read_register(opt3001::REG_DEVICE_ID, &mut id_buf)
            .map_err(|e| {
                format!(
                    "OPT3001 0x{:02x}@{}: read REG_DEVICE_ID: {}",
                    address, bus_path, e
                )
            })?;

        let device_id = u16::from_be_bytes(id_buf);
        if device_id != opt3001::DEVICE_ID {
            return Err(format!(
                "OPT3001 0x{:02x}@{}: device ID mismatch: expected 0x{:04x}, got 0x{:04x}",
                address,
                bus_path,
                opt3001::DEVICE_ID,
                device_id,
            ));
        }

        let connection = ConnectionInfo {
            kind: ConnectionKind::I2c,
            parameters: BTreeMap::from([
                ("bus".into(), bus_path.to_string()),
                ("address".into(), format!("0x{:02x}", address)),
            ]),
        };
        Ok(opt3001::identity(connection))
    }

    fn init(&self, bus_path: &str, address: u8) -> Result<(), String> {
        let mut device = self
            .device_factory
            .open(bus_path, address as u16)
            .map_err(|e| format!("OPT3001 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let config_bytes = opt3001::INIT_CONFIG.to_le_bytes();
        device
            .write_register(opt3001::REG_CONFIG, &config_bytes)
            .map_err(|e| {
                format!(
                    "OPT3001 0x{:02x}@{}: write REG_CONFIG: {}",
                    address, bus_path, e
                )
            })?;

        Ok(())
    }

    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String> {
        let mut device = self
            .device_factory
            .open(bus_path, address as u16)
            .map_err(|e| format!("OPT3001 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let mut raw = [0u8; 2];
        device
            .read_register(opt3001::REG_RESULT, &mut raw)
            .map_err(|e| {
                format!(
                    "OPT3001 0x{:02x}@{}: read REG_RESULT: {}",
                    address, bus_path, e
                )
            })?;

        // Normalize to SMBus byte-swapped u16 for existing parser.
        let swapped = u16::from_le_bytes(raw);
        Ok(opt3001::from_i2c_raw(swapped))
    }

    fn ic_name(&self) -> &'static str {
        "opt3001"
    }

    fn validate(&self, poll_interval_ms: u64) -> Result<(), String> {
        if poll_interval_ms < 200 {
            return Err(format!(
                "poll_interval_ms {} too short for OPT3001 (minimum 200ms for conversion latency)",
                poll_interval_ms,
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/drivers/opt3001_tests.rs"]
mod tests;
