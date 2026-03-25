//! RPi4B transport layer: serial / I2C / GPIO.
//! No protocol knowledge. Just bytes and pin states.

pub mod serial;
pub mod i2c;
pub mod gpio;

// re-export for convenience
pub use serial::{SerialConfig, SerialTransport, DataBits, Parity, StopBits};
pub use i2c::{I2cConfig, I2cTransport};
pub use gpio::{GpioInput, GpioOutput, GpioPull};
