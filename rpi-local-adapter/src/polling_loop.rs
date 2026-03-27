//! Polling loop internals: state management, outcome processing, and the async loop.

use iotkit_core_types::{AdapterEvent, DeviceKey, SensorIdentity, SensorReading};

use crate::config::{sensor_ic_name, SensorTarget};

/// Per-target discovery state.
#[derive(Debug, Clone)]
pub(crate) enum TargetState {
    /// Not yet discovered (DeviceDiscovered not sent).
    Pending,
    /// Discovery complete; holds the DeviceKey for event generation.
    Active(DeviceKey),
}

/// Result of a single target's probe or read within a spawn_blocking cycle.
#[derive(Debug)]
pub(crate) enum PollOutcome {
    Discovered {
        target_index: usize,
        key: DeviceKey,
        identity: SensorIdentity,
    },
    Reading {
        key: DeviceKey,
        reading: SensorReading,
    },
    ReadError {
        key: DeviceKey,
        message: String,
    },
    ProbeFailed {
        target_index: usize,
        message: String,
    },
}

/// Builds a DeviceKey from a SensorTarget.
pub(crate) fn device_key_for(target: &SensorTarget) -> DeviceKey {
    DeviceKey::new(format!(
        "i2c:0x{:02x}:{}",
        target.address,
        sensor_ic_name(&target.kind),
    ))
}

/// Pure function: applies poll outcomes to target states, returns events to send.
///
/// Rules:
/// - Discovered → state becomes Active, emit DeviceDiscovered
/// - Reading → emit SensorData (state unchanged)
/// - ReadError → emit AdapterError (state stays Active)
/// - ProbeFailed → log only (state stays Pending)
pub(crate) fn apply_outcomes(
    outcomes: Vec<PollOutcome>,
    states: &mut [TargetState],
) -> Vec<AdapterEvent> {
    let mut events = Vec::new();

    for outcome in outcomes {
        match outcome {
            PollOutcome::Discovered { target_index, key, identity } => {
                states[target_index] = TargetState::Active(key.clone());
                events.push(AdapterEvent::DeviceDiscovered {
                    device_key: key,
                    identity,
                });
            }
            PollOutcome::Reading { key, reading } => {
                events.push(AdapterEvent::SensorData {
                    device_key: key,
                    reading,
                    rssi: None,
                    battery_pct: None,
                });
            }
            PollOutcome::ReadError { key, message } => {
                events.push(AdapterEvent::AdapterError {
                    device_key: Some(key),
                    error: message,
                });
            }
            PollOutcome::ProbeFailed { target_index: _, message } => {
                tracing::warn!(error = %message, "Probe failed (no event)");
            }
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorType};
    use std::collections::BTreeMap;

    fn test_identity() -> SensorIdentity {
        SensorIdentity {
            manufacturer: "Test".into(),
            ic_part_number: "MCP9600".into(),
            sensor_type: SensorType::Temperature,
            connection: ConnectionInfo {
                kind: ConnectionKind::I2c,
                parameters: BTreeMap::new(),
            },
        }
    }

    fn test_reading() -> SensorReading {
        SensorReading::new(SensorType::Temperature, vec![22.5], vec!["celsius"])
    }

    #[test]
    fn probe_success_transitions_to_active_and_emits_discovered() {
        let mut states = vec![TargetState::Pending];
        let key = DeviceKey::new("i2c:0x60:mcp9600");
        let outcomes = vec![PollOutcome::Discovered {
            target_index: 0,
            key: key.clone(),
            identity: test_identity(),
        }];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Active(_)));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AdapterEvent::DeviceDiscovered { .. }));
    }

    #[test]
    fn read_success_emits_sensor_data() {
        let key = DeviceKey::new("i2c:0x60:mcp9600");
        let mut states = vec![TargetState::Active(key.clone())];
        let outcomes = vec![PollOutcome::Reading {
            key: key.clone(),
            reading: test_reading(),
        }];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Active(_)));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AdapterEvent::SensorData { .. }));
    }

    #[test]
    fn read_failure_keeps_active_state_and_emits_error() {
        let key = DeviceKey::new("i2c:0x60:mcp9600");
        let mut states = vec![TargetState::Active(key.clone())];
        let outcomes = vec![PollOutcome::ReadError {
            key: key.clone(),
            message: "I/O error".into(),
        }];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Active(_)));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AdapterEvent::AdapterError { .. }));
    }

    #[test]
    fn probe_failure_emits_no_event() {
        let mut states = vec![TargetState::Pending];
        let outcomes = vec![PollOutcome::ProbeFailed {
            target_index: 0,
            message: "device not found".into(),
        }];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Pending));
        assert!(events.is_empty());
    }

    #[test]
    fn discovered_only_emits_discovered_no_read_in_same_cycle() {
        let mut states = vec![TargetState::Pending];
        let key = DeviceKey::new("i2c:0x60:mcp9600");
        let outcomes = vec![PollOutcome::Discovered {
            target_index: 0,
            key: key.clone(),
            identity: test_identity(),
        }];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Active(_)));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AdapterEvent::DeviceDiscovered { .. }));
    }

    #[test]
    fn multiple_targets_independent() {
        let key_a = DeviceKey::new("i2c:0x60:mcp9600");
        let mut states = vec![
            TargetState::Active(key_a.clone()),
            TargetState::Pending,
        ];
        let outcomes = vec![
            PollOutcome::Reading {
                key: key_a.clone(),
                reading: test_reading(),
            },
            PollOutcome::ProbeFailed {
                target_index: 1,
                message: "not found".into(),
            },
        ];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Active(_)));
        assert!(matches!(states[1], TargetState::Pending));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AdapterEvent::SensorData { .. }));
    }
}
