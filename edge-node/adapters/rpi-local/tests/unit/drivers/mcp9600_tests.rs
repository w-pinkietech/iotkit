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
