//! UART serial transport.

use std::io;
use std::time::Duration;

pub use serialport::{DataBits, Parity, StopBits};

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

    /// The I/O error kind behind an open failure, so callers can classify it
    /// (missing device node, permission, busy) without depending on serialport.
    pub fn open_error_kind(error: &serialport::Error) -> io::ErrorKind {
        match error.kind() {
            serialport::ErrorKind::NoDevice => io::ErrorKind::NotFound,
            serialport::ErrorKind::Io(kind) => kind,
            serialport::ErrorKind::InvalidInput => io::ErrorKind::InvalidInput,
            serialport::ErrorKind::Unknown => io::ErrorKind::Other,
        }
    }

    /// Read bytes from the serial port. Returns number of bytes read.
    pub fn read(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        self.port.set_timeout(timeout).map_err(io::Error::other)?;

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

    /// Write all bytes to the serial port, retrying on partial writes.
    pub fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        use std::io::Write;
        self.port.write_all(buf)
    }
}
