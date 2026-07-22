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
#[path = "../tests/unit/i2c_tests.rs"]
mod tests;
