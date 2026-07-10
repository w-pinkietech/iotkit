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
mod tests {
    use super::*;
    use iotkit_core_types::SensorType;

    #[test]
    fn all_known_raw_codes_resolve() {
        let expected = [
            (257, "contact_input"),
            (258, "contact_output"),
            (259, "adc"),
            (260, "ranging"),
            (261, "temperature"),
            (262, "acceleration"),
            (263, "differential_pressure"),
            (264, "illuminance"),
        ];
        for (raw, suffix) in expected {
            let handler =
                lookup_handler(raw).unwrap_or_else(|| panic!("raw {} should resolve", raw));
            assert_eq!(handler.key_suffix, suffix, "raw {} suffix mismatch", raw);
        }
    }

    #[test]
    fn unknown_raw_code_returns_none() {
        assert!(lookup_handler(0).is_none());
        assert!(lookup_handler(9999).is_none());
    }

    #[test]
    fn handler_sensor_types_are_correct() {
        assert_eq!(
            lookup_handler(261).unwrap().sensor_type,
            SensorType::Temperature
        );
        assert_eq!(
            lookup_handler(257).unwrap().sensor_type,
            SensorType::ContactInput
        );
        assert_eq!(
            lookup_handler(258).unwrap().sensor_type,
            SensorType::ContactOutput
        );
    }

    #[test]
    fn no_duplicate_raw_codes() {
        let mut seen = std::collections::HashSet::new();
        for entry in REGISTRY.iter() {
            assert!(
                seen.insert(entry.raw_sensor_type),
                "duplicate raw code: {}",
                entry.raw_sensor_type,
            );
        }
    }
}
