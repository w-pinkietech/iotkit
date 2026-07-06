//! RPi4B transport layer: serial / I2C / GPIO / SPI / PWM / USB.
//! No protocol knowledge. Just bytes and pin states.

pub mod gpio;
pub mod i2c;
pub mod pwm;
pub mod serial;
pub mod spi;
pub mod usb;

// re-export for convenience
pub use gpio::{GpioInput, GpioOutput, GpioPull};
pub use i2c::{I2cConfig, I2cTransport};
pub use pwm::{PwmChannel, PwmConfig, PwmOutput, PwmPolarity};
pub use serial::{DataBits, Parity, SerialConfig, SerialTransport, StopBits};
pub use spi::{SpiBus, SpiConfig, SpiMode, SpiSlaveSelect, SpiTransport};
pub use usb::{UsbSerialInfo, UsbSerialTransport, list_usb_serial_devices};
