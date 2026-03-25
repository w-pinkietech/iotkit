//! USB serial transport.
//!
//! USB シリアルデバイス（/dev/ttyUSB*, /dev/ttyACM*）用。
//! 内部的には SerialTransport と同じだが、デバイス列挙機能を持つ。

use std::io;
use std::time::Duration;

use crate::serial::{SerialConfig, SerialTransport};

/// 接続中の USB シリアルデバイス情報。
#[derive(Debug, Clone)]
pub struct UsbSerialInfo {
    pub port_name: String,
    pub vid: u16,
    pub pid: u16,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
}

/// 接続中の USB シリアルデバイスを列挙する。
pub fn list_usb_serial_devices() -> io::Result<Vec<UsbSerialInfo>> {
    let ports = serialport::available_ports()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let mut devices = Vec::new();
    for port in ports {
        if let serialport::SerialPortType::UsbPort(usb) = port.port_type {
            devices.push(UsbSerialInfo {
                port_name: port.port_name,
                vid: usb.vid,
                pid: usb.pid,
                serial_number: usb.serial_number,
                manufacturer: usb.manufacturer,
                product: usb.product,
            });
        }
    }
    Ok(devices)
}

/// USB シリアルデバイスを開く。
/// SerialTransport のラッパー。
pub struct UsbSerialTransport {
    inner: SerialTransport,
}

impl UsbSerialTransport {
    /// デバイスパス（例: /dev/ttyUSB0）と設定でオープン。
    pub fn open(path: &str, config: &SerialConfig) -> io::Result<Self> {
        let inner = SerialTransport::open(path, config)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(Self { inner })
    }

    /// VID/PID でデバイスを探してオープン。
    pub fn open_by_vid_pid(vid: u16, pid: u16, config: &SerialConfig) -> io::Result<Self> {
        let devices = list_usb_serial_devices()?;
        let device = devices.iter()
            .find(|d| d.vid == vid && d.pid == pid)
            .ok_or_else(|| io::Error::new(
                io::ErrorKind::NotFound,
                format!("USB device {:04x}:{:04x} not found", vid, pid),
            ))?;
        Self::open(&device.port_name, config)
    }

    pub fn read(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        self.inner.read(buf, timeout)
    }

    pub fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }
}
