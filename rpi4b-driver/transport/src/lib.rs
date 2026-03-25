//! Transport layer: serial port open/read/write only.
//! No protocol knowledge. Just bytes.

use std::io;
use std::time::Duration;

pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
}

impl SerialTransport {
    /// Open /dev/ttyAMA0 at 38400 8N1, binary mode.
    pub fn open(path: &str) -> Result<Self, serialport::Error> {
        let port = serialport::new(path, 38400)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
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
