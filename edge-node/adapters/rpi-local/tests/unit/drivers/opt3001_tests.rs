use super::*;
use rpi4b_transport::i2c::{I2cDevice, I2cDeviceFactory};
use std::io;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct RecordingFactory {
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
}

struct RecordingDevice {
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl I2cDevice for RecordingDevice {
    fn read(&mut self, _data: &mut [u8]) -> io::Result<()> {
        unreachable!("OPT3001 driver uses combined register reads")
    }

    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.writes.lock().unwrap().push(data.to_vec());
        Ok(())
    }

    fn write_read(&mut self, _write: &[u8], _read: &mut [u8]) -> io::Result<()> {
        unreachable!("this test only exercises init")
    }
}

impl I2cDeviceFactory for RecordingFactory {
    fn open(&self, _bus: &str, _address: u16) -> io::Result<Box<dyn I2cDevice>> {
        Ok(Box::new(RecordingDevice {
            writes: Arc::clone(&self.writes),
        }))
    }
}

#[test]
fn init_writes_the_exact_existing_wire_bytes() {
    let factory = RecordingFactory::default();
    let driver = Opt3001Driver {
        device_factory: Arc::new(factory.clone()),
    };

    driver.init("/dev/i2c-test", 0x44).unwrap();

    assert_eq!(
        *factory.writes.lock().unwrap(),
        [vec![opt3001::REG_CONFIG, 0xcc, 0x10]]
    );
}
