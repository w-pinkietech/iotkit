//! MCP9600 SensorDriver implementation.

use std::collections::BTreeMap;
use std::sync::Arc;

use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading};
use iotkit_polling_adapter_runtime::SensorDriver;
use iotkit_sensor_drivers::mcp9600::{self, ThermocoupleType};
use rpi4b_transport::i2c::I2cDeviceFactory;

pub struct Mcp9600Driver {
    pub thermocouple_type: ThermocoupleType,
    pub device_factory: Arc<dyn I2cDeviceFactory>,
}

impl SensorDriver for Mcp9600Driver {
    fn detect(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String> {
        let mut device = self
            .device_factory
            .open(bus_path, address as u16)
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let mut id_buf = [0u8; 2];
        device
            .read_register(mcp9600::REG_DEVICE_ID, &mut id_buf)
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

        let connection = ConnectionInfo {
            kind: ConnectionKind::I2c,
            parameters: BTreeMap::from([
                ("bus".into(), bus_path.to_string()),
                ("address".into(), format!("0x{:02x}", address)),
            ]),
        };
        Ok(mcp9600::identity(connection))
    }

    fn init(&self, bus_path: &str, address: u8) -> Result<(), String> {
        let mut device = self
            .device_factory
            .open(bus_path, address as u16)
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let config_val = mcp9600::config_value(self.thermocouple_type);
        device
            .write_register(mcp9600::REG_SENSOR_CONFIGURATION, &[config_val])
            .map_err(|e| {
                format!(
                    "MCP9600 0x{:02x}@{}: write REG_SENSOR_CONFIGURATION: {}",
                    address, bus_path, e
                )
            })?;

        Ok(())
    }

    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String> {
        let mut device = self
            .device_factory
            .open(bus_path, address as u16)
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let mut raw = [0u8; 2];
        device
            .read_register(mcp9600::REG_HOT_JUNCTION, &mut raw)
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

#[cfg(test)]
mod tests {
    use super::*;
    use rpi4b_transport::i2c::{I2cDevice, I2cDeviceFactory};
    use std::collections::VecDeque;
    use std::io;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Operation {
        Open(String, u16),
        Write(Vec<u8>),
        WriteRead(Vec<u8>, usize),
    }

    #[derive(Default)]
    struct State {
        operations: Vec<Operation>,
        responses: VecDeque<Vec<u8>>,
    }

    #[derive(Clone, Default)]
    struct RecordingFactory {
        state: Arc<Mutex<State>>,
    }

    struct RecordingDevice {
        state: Arc<Mutex<State>>,
    }

    impl I2cDevice for RecordingDevice {
        fn read(&mut self, _data: &mut [u8]) -> io::Result<()> {
            unreachable!("MCP9600 driver uses combined register reads")
        }

        fn write(&mut self, data: &[u8]) -> io::Result<()> {
            self.state
                .lock()
                .unwrap()
                .operations
                .push(Operation::Write(data.to_vec()));
            Ok(())
        }

        fn write_read(&mut self, write: &[u8], read: &mut [u8]) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            state
                .operations
                .push(Operation::WriteRead(write.to_vec(), read.len()));
            let response = state.responses.pop_front().expect("queued response");
            read.copy_from_slice(&response);
            Ok(())
        }
    }

    impl I2cDeviceFactory for RecordingFactory {
        fn open(&self, bus: &str, address: u16) -> io::Result<Box<dyn I2cDevice>> {
            self.state
                .lock()
                .unwrap()
                .operations
                .push(Operation::Open(bus.to_string(), address));
            Ok(Box::new(RecordingDevice {
                state: Arc::clone(&self.state),
            }))
        }
    }

    #[test]
    fn detect_uses_injected_factory_and_combined_register_read() {
        let factory = RecordingFactory::default();
        factory
            .state
            .lock()
            .unwrap()
            .responses
            .push_back(vec![mcp9600::DEVICE_ID, 0]);
        let driver = Mcp9600Driver {
            thermocouple_type: ThermocoupleType::K,
            device_factory: Arc::new(factory.clone()),
        };

        driver.detect("/dev/i2c-test", 0x60).unwrap();

        assert_eq!(
            factory.state.lock().unwrap().operations,
            [
                Operation::Open("/dev/i2c-test".into(), 0x60),
                Operation::WriteRead(vec![mcp9600::REG_DEVICE_ID], 2),
            ]
        );
    }

    #[test]
    fn init_writes_the_exact_wire_bytes() {
        let factory = RecordingFactory::default();
        let driver = Mcp9600Driver {
            thermocouple_type: ThermocoupleType::K,
            device_factory: Arc::new(factory.clone()),
        };

        driver.init("/dev/i2c-test", 0x60).unwrap();

        assert_eq!(
            factory.state.lock().unwrap().operations,
            [
                Operation::Open("/dev/i2c-test".into(), 0x60),
                Operation::Write(vec![
                    mcp9600::REG_SENSOR_CONFIGURATION,
                    mcp9600::config_value(ThermocoupleType::K),
                ]),
            ]
        );
    }
}
