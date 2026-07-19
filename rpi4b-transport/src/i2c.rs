//! I2C transport.

use std::io;

use i2cdev::core::{I2CDevice as LinuxDeviceOps, I2CMessage, I2CTransfer};
use i2cdev::linux::{LinuxI2CDevice, LinuxI2CMessage};

/// I2C バスの設定。
#[derive(Debug, Clone)]
pub struct I2cConfig {
    pub address: u16,
}

pub struct I2cTransport {
    dev: LinuxI2CDevice,
}

fn require_exact_transfer_count(actual: u32, expected: usize) -> io::Result<()> {
    if actual == expected as u32 {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!("I2C transfer completed {actual} of {expected} messages"),
    ))
}

/// Narrow device-level I2C boundary used by sensor drivers.
///
/// `write_read` is one combined transaction and therefore supports devices
/// that require a repeated START between register selection and data read.
pub trait I2cDevice: Send {
    fn read(&mut self, data: &mut [u8]) -> io::Result<()>;
    fn write(&mut self, data: &[u8]) -> io::Result<()>;
    fn write_read(&mut self, write: &[u8], read: &mut [u8]) -> io::Result<()>;

    fn read_register(&mut self, register: u8, data: &mut [u8]) -> io::Result<()> {
        self.write_read(&[register], data)
    }

    fn write_register(&mut self, register: u8, data: &[u8]) -> io::Result<()> {
        let mut bytes = Vec::with_capacity(1 + data.len());
        bytes.push(register);
        bytes.extend_from_slice(data);
        self.write(&bytes)
    }
}

/// Opens one addressed I2C device on a Linux bus path.
pub trait I2cDeviceFactory: Send + Sync {
    fn open(&self, bus: &str, address: u16) -> io::Result<Box<dyn I2cDevice>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxI2cDeviceFactory;

impl I2cDeviceFactory for LinuxI2cDeviceFactory {
    fn open(&self, bus: &str, address: u16) -> io::Result<Box<dyn I2cDevice>> {
        Ok(Box::new(I2cTransport::open(bus, &I2cConfig { address })?))
    }
}

impl I2cTransport {
    /// I2C バスを開く（例: /dev/i2c-1）。
    pub fn open(bus: &str, config: &I2cConfig) -> io::Result<Self> {
        let dev = LinuxI2CDevice::new(bus, config.address).map_err(io::Error::other)?;
        Ok(Self { dev })
    }

    /// レジスタからバイト列を読み取る。
    pub fn read_register(&mut self, reg: u8, buf: &mut [u8]) -> io::Result<()> {
        I2cDevice::read_register(self, reg, buf)
    }

    /// レジスタにバイト列を書き込む。
    pub fn write_register(&mut self, reg: u8, data: &[u8]) -> io::Result<()> {
        I2cDevice::write_register(self, reg, data)
    }

    /// レジスタ指定なしの生読み取り。
    pub fn read_raw(&mut self, buf: &mut [u8]) -> io::Result<()> {
        I2cDevice::read(self, buf)
    }

    /// レジスタ指定なしの生書き込み。
    pub fn write_raw(&mut self, data: &[u8]) -> io::Result<()> {
        I2cDevice::write(self, data)
    }
}

impl I2cDevice for I2cTransport {
    fn read(&mut self, data: &mut [u8]) -> io::Result<()> {
        self.dev.read(data).map_err(io::Error::other)
    }

    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let mut messages = [LinuxI2CMessage::write(data)];
        let expected = messages.len();
        let completed = self.dev.transfer(&mut messages).map_err(io::Error::other)?;
        require_exact_transfer_count(completed, expected)
    }

    fn write_read(&mut self, write: &[u8], read: &mut [u8]) -> io::Result<()> {
        let mut messages = [LinuxI2CMessage::write(write), LinuxI2CMessage::read(read)];
        let expected = messages.len();
        let completed = self.dev.transfer(&mut messages).map_err(io::Error::other)?;
        require_exact_transfer_count(completed, expected)
    }
}

#[cfg(test)]
mod tests {
    use super::{I2cDevice, require_exact_transfer_count};
    use std::io;

    #[derive(Debug, PartialEq, Eq)]
    enum Transaction {
        Read(usize),
        Write(Vec<u8>),
        WriteRead(Vec<u8>, usize),
    }

    #[derive(Default)]
    struct RecordingDevice {
        transactions: Vec<Transaction>,
    }

    impl I2cDevice for RecordingDevice {
        fn read(&mut self, data: &mut [u8]) -> io::Result<()> {
            self.transactions.push(Transaction::Read(data.len()));
            data.fill(0);
            Ok(())
        }

        fn write(&mut self, data: &[u8]) -> io::Result<()> {
            self.transactions.push(Transaction::Write(data.to_vec()));
            Ok(())
        }

        fn write_read(&mut self, write: &[u8], read: &mut [u8]) -> io::Result<()> {
            self.transactions
                .push(Transaction::WriteRead(write.to_vec(), read.len()));
            read.copy_from_slice(&[0x12, 0x34]);
            Ok(())
        }
    }

    #[test]
    fn register_read_uses_one_combined_transaction() {
        let mut device = RecordingDevice::default();
        let mut data = [0_u8; 2];

        device.read_register(0x0f, &mut data).unwrap();

        assert_eq!(data, [0x12, 0x34]);
        assert_eq!(device.transactions, [Transaction::WriteRead(vec![0x0f], 2)]);
    }

    #[test]
    fn register_write_is_one_raw_write() {
        let mut device = RecordingDevice::default();

        device.write_register(0x05, &[0xaa, 0xbb]).unwrap();

        assert_eq!(
            device.transactions,
            [Transaction::Write(vec![0x05, 0xaa, 0xbb])]
        );
    }

    #[test]
    fn partial_combined_transfer_is_an_error() {
        let error = require_exact_transfer_count(1, 2).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn partial_single_transfer_is_an_error() {
        let error = require_exact_transfer_count(0, 1).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }
}
