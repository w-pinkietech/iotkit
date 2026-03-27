//! MCP9600 I2C probe and read.

use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading};
use rpi4b_transport::{I2cConfig, I2cTransport};
use bravepi_sensors::mcp9600::{self, ThermocoupleType};
use std::collections::BTreeMap;

pub fn probe_mcp9600(
    bus: &str,
    addr: u8,
    thermocouple_type: ThermocoupleType,
) -> Result<SensorIdentity, String> {
    let mut t = I2cTransport::open(bus, &I2cConfig { address: addr as u16 })
        .map_err(|e| format!("MCP9600 0x{:02x}@{}: I2C open: {}", addr, bus, e))?;

    let mut id_buf = [0u8; 2];
    t.read_register(mcp9600::REG_DEVICE_ID, &mut id_buf)
        .map_err(|e| format!("MCP9600 0x{:02x}@{}: read REG_DEVICE_ID: {}", addr, bus, e))?;

    if id_buf[0] != mcp9600::DEVICE_ID {
        return Err(format!(
            "MCP9600 0x{:02x}@{}: device ID mismatch: expected 0x{:02x}, got 0x{:02x}",
            addr, bus, mcp9600::DEVICE_ID, id_buf[0],
        ));
    }

    let config_val = mcp9600::config_value(thermocouple_type);
    t.write_register(mcp9600::REG_SENSOR_CONFIGURATION, &[config_val])
        .map_err(|e| format!("MCP9600 0x{:02x}@{}: write REG_SENSOR_CONFIGURATION: {}", addr, bus, e))?;

    let connection = ConnectionInfo {
        kind: ConnectionKind::I2c,
        parameters: BTreeMap::from([
            ("bus".into(), bus.to_string()),
            ("address".into(), format!("0x{:02x}", addr)),
        ]),
    };
    Ok(mcp9600::identity(connection))
}

pub fn read_mcp9600(bus: &str, addr: u8) -> Result<SensorReading, String> {
    let mut t = I2cTransport::open(bus, &I2cConfig { address: addr as u16 })
        .map_err(|e| format!("MCP9600 0x{:02x}@{}: I2C open: {}", addr, bus, e))?;

    let mut raw = [0u8; 2];
    t.read_register(mcp9600::REG_HOT_JUNCTION, &mut raw)
        .map_err(|e| format!("MCP9600 0x{:02x}@{}: read REG_HOT_JUNCTION: {}", addr, bus, e))?;

    Ok(mcp9600::from_i2c_raw(&raw))
}
