//! SPI transport.

use std::io;

use rppal::spi::{Bus, Mode, SlaveSelect, Spi};

/// SPI の設定。
#[derive(Debug, Clone)]
pub struct SpiConfig {
    pub bus: SpiBus,
    pub slave_select: SpiSlaveSelect,
    pub clock_speed_hz: u32,
    pub mode: SpiMode,
}

/// SPI バス。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpiBus {
    Spi0,
    Spi1,
}

/// スレーブセレクト (CS)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpiSlaveSelect {
    Ss0,
    Ss1,
    Ss2,
}

/// SPI モード (CPOL/CPHA)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpiMode {
    Mode0,
    Mode1,
    Mode2,
    Mode3,
}

pub struct SpiTransport {
    spi: Spi,
}

impl SpiTransport {
    /// SPI バスを開く。
    pub fn open(config: &SpiConfig) -> io::Result<Self> {
        let bus = match config.bus {
            SpiBus::Spi0 => Bus::Spi0,
            SpiBus::Spi1 => Bus::Spi1,
        };
        let ss = match config.slave_select {
            SpiSlaveSelect::Ss0 => SlaveSelect::Ss0,
            SpiSlaveSelect::Ss1 => SlaveSelect::Ss1,
            SpiSlaveSelect::Ss2 => SlaveSelect::Ss2,
        };
        let mode = match config.mode {
            SpiMode::Mode0 => Mode::Mode0,
            SpiMode::Mode1 => Mode::Mode1,
            SpiMode::Mode2 => Mode::Mode2,
            SpiMode::Mode3 => Mode::Mode3,
        };
        let spi = Spi::new(bus, ss, config.clock_speed_hz, mode)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(Self { spi })
    }

    /// 読み取り。
    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.spi.read(buf)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    /// 書き込み。
    pub fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.spi.write(data)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    /// 同時送受信 (full-duplex)。
    pub fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> io::Result<usize> {
        self.spi.transfer(rx, tx)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}
