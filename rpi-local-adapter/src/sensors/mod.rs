//! Per-sensor I2C probe and read dispatch.

pub mod mcp9600;
pub mod opt3001;

use iotkit_core_types::{SensorIdentity, SensorReading};
use crate::config::SensorKind;

pub fn probe(kind: &SensorKind, bus: &str, addr: u8) -> Result<SensorIdentity, String> {
    match kind {
        SensorKind::MCP9600 { thermocouple_type } => {
            mcp9600::probe_mcp9600(bus, addr, *thermocouple_type)
        }
        SensorKind::OPT3001 => opt3001::probe_opt3001(bus, addr),
    }
}

pub fn read(kind: &SensorKind, bus: &str, addr: u8) -> Result<SensorReading, String> {
    match kind {
        SensorKind::MCP9600 { .. } => mcp9600::read_mcp9600(bus, addr),
        SensorKind::OPT3001 => opt3001::read_opt3001(bus, addr),
    }
}
