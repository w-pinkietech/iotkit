//! GPIO transport.

use std::io;

use rppal::gpio::{Gpio, InputPin, Level, OutputPin};

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
