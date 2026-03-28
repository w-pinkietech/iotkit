//! MCP9600 SensorDriver implementation.

use std::collections::BTreeMap;

use bravepi_sensors::mcp9600::{self, ThermocoupleType};
use iotkit_polling_adapter_runtime::SensorDriver;
use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading};
use rpi4b_transport::{I2cConfig, I2cTransport};

pub struct Mcp9600Driver {
    pub thermocouple_type: ThermocoupleType,
}

impl SensorDriver for Mcp9600Driver {
    fn probe(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String> {
        let mut t = I2cTransport::open(bus_path, &I2cConfig { address: address as u16 })
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let mut id_buf = [0u8; 2];
        t.read_register(mcp9600::REG_DEVICE_ID, &mut id_buf)
            .map_err(|e| {
                format!(
                    "MCP9600 0x{:02x}@{}: read REG_DEVICE_ID: {}",
                    address, bus_path, e
                )
            })?;

        if id_buf[0] != mcp9600::DEVICE_ID {
            return Err(format!(
                "MCP9600 0x{:02x}@{}: device ID mismatch: expected 0x{:02x}, got 0x{:02x}",
                address,
                bus_path,
                mcp9600::DEVICE_ID,
                id_buf[0],
            ));
        }

        let config_val = mcp9600::config_value(self.thermocouple_type);
        t.write_register(mcp9600::REG_SENSOR_CONFIGURATION, &[config_val])
            .map_err(|e| {
                format!(
                    "MCP9600 0x{:02x}@{}: write REG_SENSOR_CONFIGURATION: {}",
                    address, bus_path, e
                )
            })?;

        let connection = ConnectionInfo {
            kind: ConnectionKind::I2c,
            parameters: BTreeMap::from([
                ("bus".into(), bus_path.to_string()),
                ("address".into(), format!("0x{:02x}", address)),
            ]),
        };
        Ok(mcp9600::identity(connection))
    }

    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String> {
        let mut t = I2cTransport::open(bus_path, &I2cConfig { address: address as u16 })
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let mut raw = [0u8; 2];
        t.read_register(mcp9600::REG_HOT_JUNCTION, &mut raw)
            .map_err(|e| {
                format!(
                    "MCP9600 0x{:02x}@{}: read REG_HOT_JUNCTION: {}",
                    address, bus_path, e
                )
            })?;

        Ok(mcp9600::from_i2c_raw(&raw))
    }

    fn ic_name(&self) -> &'static str {
        "mcp9600"
    }
}
