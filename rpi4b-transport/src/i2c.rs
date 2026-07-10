//! I2C transport.

use std::io;

use i2cdev::core::I2CDevice;
use i2cdev::linux::LinuxI2CDevice;

/// I2C バスの設定。
#[derive(Debug, Clone)]
pub struct I2cConfig {
    pub address: u16,
}

pub struct I2cTransport {
    dev: LinuxI2CDevice,
}

impl I2cTransport {
    /// I2C バスを開く（例: /dev/i2c-1）。
    pub fn open(bus: &str, config: &I2cConfig) -> io::Result<Self> {
        let dev = LinuxI2CDevice::new(bus, config.address).map_err(io::Error::other)?;
        Ok(Self { dev })
    }

    /// レジスタからバイト列を読み取る。
    pub fn read_register(&mut self, reg: u8, buf: &mut [u8]) -> io::Result<()> {
        self.dev.write(&[reg]).map_err(io::Error::other)?;
        self.dev.read(buf).map_err(io::Error::other)?;
        Ok(())
    }

    /// レジスタにバイト列を書き込む。
    pub fn write_register(&mut self, reg: u8, data: &[u8]) -> io::Result<()> {
        let mut buf = Vec::with_capacity(1 + data.len());
        buf.push(reg);
        buf.extend_from_slice(data);
        self.dev.write(&buf).map_err(io::Error::other)?;
        Ok(())
    }

    /// レジスタ指定なしの生読み取り。
    pub fn read_raw(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.dev.read(buf).map_err(io::Error::other)?;
        Ok(())
    }

    /// レジスタ指定なしの生書き込み。
    pub fn write_raw(&mut self, data: &[u8]) -> io::Result<()> {
        self.dev.write(data).map_err(io::Error::other)?;
        Ok(())
    }
}
