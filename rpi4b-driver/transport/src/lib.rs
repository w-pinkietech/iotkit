//! RPi4B transport layer: serial / I2C / GPIO / SPI / PWM / USB.
//! No protocol knowledge. Just bytes and pin states.

pub mod serial;
pub mod i2c;
pub mod gpio;
pub mod spi;
pub mod pwm;
pub mod usb;

// re-export for convenience
pub use serial::{SerialConfig, SerialTransport, DataBits, Parity, StopBits};
pub use i2c::{I2cConfig, I2cTransport};
pub use gpio::{GpioInput, GpioOutput, GpioPull};
pub use spi::{SpiConfig, SpiTransport, SpiBus, SpiSlaveSelect, SpiMode};
pub use pwm::{PwmConfig, PwmOutput, PwmChannel, PwmPolarity};
pub use usb::{UsbSerialTransport, UsbSerialInfo, list_usb_serial_devices};
