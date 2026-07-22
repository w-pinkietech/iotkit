//! BravePI raw sensor_type → SensorHandler の対応表。
//! BravePI プロトコル固有の番号体系はこのモジュールに閉じる。

use iotkit_sensor_drivers::SensorHandler;

struct RegistryEntry {
    raw_sensor_type: u16,
    handler: &'static SensorHandler,
}

static REGISTRY: &[RegistryEntry] = &[
    RegistryEntry {
        raw_sensor_type: 257,
        handler: &iotkit_sensor_drivers::contact::CONTACT_INPUT,
    },
    RegistryEntry {
        raw_sensor_type: 258,
        handler: &iotkit_sensor_drivers::contact::CONTACT_OUTPUT,
    },
    RegistryEntry {
        raw_sensor_type: 259,
        handler: &iotkit_sensor_drivers::mcp3427::HANDLER,
    },
    RegistryEntry {
        raw_sensor_type: 260,
        handler: &iotkit_sensor_drivers::vl53l1x::HANDLER,
    },
    RegistryEntry {
        raw_sensor_type: 261,
        handler: &iotkit_sensor_drivers::mcp9600::HANDLER,
    },
    RegistryEntry {
        raw_sensor_type: 262,
        handler: &iotkit_sensor_drivers::lis2duxs12::HANDLER,
    },
    RegistryEntry {
        raw_sensor_type: 263,
        handler: &iotkit_sensor_drivers::sdp810::HANDLER,
    },
    RegistryEntry {
        raw_sensor_type: 264,
        handler: &iotkit_sensor_drivers::opt3001::HANDLER,
    },
];

pub(crate) fn lookup_handler(raw: u16) -> Option<&'static SensorHandler> {
    REGISTRY
        .iter()
        .find(|e| e.raw_sensor_type == raw)
        .map(|e| e.handler)
}

#[cfg(test)]
#[path = "../tests/unit/registry_tests.rs"]
mod tests;
