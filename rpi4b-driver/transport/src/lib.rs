//! Transport layer: serial / I2C / GPIO.
//! No protocol knowledge. Just bytes and pin states.

use std::io;
use std::time::Duration;

pub use serialport::{DataBits, Parity, StopBits};
use i2cdev::core::I2CDevice;
use i2cdev::linux::LinuxI2CDevice;
use rppal::gpio::{Gpio, InputPin, Level, OutputPin};

/// シリアルポートの設定。
#[derive(Debug, Clone)]
pub struct SerialConfig {
    pub baud_rate: u32,
    pub data_bits: DataBits,
    pub parity: Parity,
    pub stop_bits: StopBits,
}


pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
}

impl SerialTransport {
    /// 指定した設定でシリアルポートを開く。
    pub fn open(path: &str, config: &SerialConfig) -> Result<Self, serialport::Error> {
        let port = serialport::new(path, config.baud_rate)
            .data_bits(config.data_bits)
            .parity(config.parity)
            .stop_bits(config.stop_bits)
            .timeout(Duration::from_secs(1))
            .open()?;

        Ok(Self { port })
    }

    /// Read bytes from the serial port. Returns number of bytes read.
    pub fn read(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        self.port
            .set_timeout(timeout)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        match self.port.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// Write bytes to the serial port.
    pub fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.port.write(buf)
    }
}

// --- I2C Transport ---

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
        let dev = LinuxI2CDevice::new(bus, config.address)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(Self { dev })
    }

    /// レジスタからバイト列を読み取る。
    pub fn read_register(&mut self, reg: u8, buf: &mut [u8]) -> io::Result<()> {
        self.dev.write(&[reg])
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        self.dev.read(buf)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(())
    }

    /// レジスタにバイト列を書き込む。
    pub fn write_register(&mut self, reg: u8, data: &[u8]) -> io::Result<()> {
        let mut buf = Vec::with_capacity(1 + data.len());
        buf.push(reg);
        buf.extend_from_slice(data);
        self.dev.write(&buf)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(())
    }

    /// レジスタ指定なしの生読み取り。
    pub fn read_raw(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.dev.read(buf)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(())
    }

    /// レジスタ指定なしの生書き込み。
    pub fn write_raw(&mut self, data: &[u8]) -> io::Result<()> {
        self.dev.write(data)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(())
    }
}

// --- GPIO Transport ---

/// GPIO 入力ピン。BCM 番号で指定。
pub struct GpioInput {
    pin: InputPin,
}

impl GpioInput {
    /// BCM ピン番号で入力ピンを開く。
    pub fn open(bcm_pin: u8, pull: GpioPull) -> io::Result<Self> {
        let gpio = Gpio::new()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let pin = gpio.get(bcm_pin)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let pin = match pull {
            GpioPull::Up => pin.into_input_pullup(),
            GpioPull::Down => pin.into_input_pulldown(),
            GpioPull::None => pin.into_input(),
        };
        Ok(Self { pin })
    }

    /// 現在のピン状態を読む。true = High。
    pub fn read(&self) -> bool {
        self.pin.read() == Level::High
    }
}

/// GPIO 出力ピン。BCM 番号で指定。
pub struct GpioOutput {
    pin: OutputPin,
}

impl GpioOutput {
    /// BCM ピン番号で出力ピンを開く。
    pub fn open(bcm_pin: u8) -> io::Result<Self> {
        let gpio = Gpio::new()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let pin = gpio.get(bcm_pin)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            .into_output();
        Ok(Self { pin })
    }

    /// ピン状態を設定する。true = High。
    pub fn write(&mut self, high: bool) {
        if high {
            self.pin.set_high();
        } else {
            self.pin.set_low();
        }
    }
}

/// プルアップ/プルダウン設定。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GpioPull {
    Up,
    Down,
    None,
}
