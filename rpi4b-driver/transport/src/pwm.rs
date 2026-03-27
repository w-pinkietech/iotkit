//! PWM transport.

use std::io;

use rppal::pwm::{Channel, Polarity, Pwm};

/// PWM の設定。
#[derive(Debug, Clone)]
pub struct PwmConfig {
    pub channel: PwmChannel,
    pub frequency_hz: f64,
    pub duty_cycle: f64,
    pub polarity: PwmPolarity,
}

/// PWM チャンネル。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PwmChannel {
    Pwm0,
    Pwm1,
}

/// PWM 極性。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PwmPolarity {
    Normal,
    Inverse,
}

pub struct PwmOutput {
    pwm: Pwm,
}

impl PwmOutput {
    /// PWM チャンネルを開いて開始する。
    pub fn open(config: &PwmConfig) -> io::Result<Self> {
        let channel = match config.channel {
            PwmChannel::Pwm0 => Channel::Pwm0,
            PwmChannel::Pwm1 => Channel::Pwm1,
        };
        let polarity = match config.polarity {
            PwmPolarity::Normal => Polarity::Normal,
            PwmPolarity::Inverse => Polarity::Inverse,
        };
        let pwm = Pwm::with_frequency(channel, config.frequency_hz, config.duty_cycle, polarity, true)
            .map_err(io::Error::other)?;
        Ok(Self { pwm })
    }

    /// デューティ比を変更する (0.0 〜 1.0)。
    pub fn set_duty_cycle(&mut self, duty: f64) -> io::Result<()> {
        self.pwm.set_duty_cycle(duty)
            .map_err(io::Error::other)
    }

    /// 周波数を変更する (Hz)。
    pub fn set_frequency(&mut self, freq_hz: f64) -> io::Result<()> {
        self.pwm.set_frequency(freq_hz, self.pwm.duty_cycle()
            .map_err(io::Error::other)?)
            .map_err(io::Error::other)
    }

    /// PWM を停止する。
    pub fn disable(&mut self) -> io::Result<()> {
        self.pwm.disable()
            .map_err(io::Error::other)
    }

    /// PWM を再開する。
    pub fn enable(&mut self) -> io::Result<()> {
        self.pwm.enable()
            .map_err(io::Error::other)
    }
}
