//! OPT3001 I2C probe and read.

use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading};
use rpi4b_transport::{I2cConfig, I2cTransport};
use bravepi_sensors::opt3001;
use std::collections::BTreeMap;

pub fn probe_opt3001(bus: &str, addr: u8) -> Result<SensorIdentity, String> {
    let mut t = I2cTransport::open(bus, &I2cConfig { address: addr as u16 })
        .map_err(|e| format!("I2C open 0x{:02x}: {}", addr, e))?;

    let mut id_buf = [0u8; 2];
    t.read_register(opt3001::REG_DEVICE_ID, &mut id_buf)
        .map_err(|e| format!("read REG_DEVICE_ID: {}", e))?;

    let device_id = u16::from_be_bytes(id_buf);
    if device_id != opt3001::DEVICE_ID {
        return Err(format!(
            "OPT3001 device ID mismatch: expected 0x{:04x}, got 0x{:04x}",
            opt3001::DEVICE_ID, device_id,
        ));
    }

    // Write init config. Legacy Python uses smbus2 write_word_data which sends LSB first.
    // Our raw transport needs explicit LE byte order to match.
    let config_bytes = opt3001::INIT_CONFIG.to_le_bytes();
    t.write_register(opt3001::REG_CONFIG, &config_bytes)
        .map_err(|e| format!("write REG_CONFIG: {}", e))?;

    let connection = ConnectionInfo {
        kind: ConnectionKind::I2c,
        parameters: BTreeMap::from([
            ("bus".into(), bus.to_string()),
            ("address".into(), format!("0x{:02x}", addr)),
        ]),
    };
    Ok(opt3001::identity(connection))
}

pub fn read_opt3001(bus: &str, addr: u8) -> Result<SensorReading, String> {
    let mut t = I2cTransport::open(bus, &I2cConfig { address: addr as u16 })
        .map_err(|e| format!("I2C open 0x{:02x}: {}", addr, e))?;

    let mut raw = [0u8; 2];
    t.read_register(opt3001::REG_RESULT, &mut raw)
        .map_err(|e| format!("read REG_RESULT: {}", e))?;

    // Normalize to SMBus byte-swapped u16 for existing parser.
    let swapped = u16::from_le_bytes(raw);
    Ok(opt3001::from_i2c_raw(swapped))
}
