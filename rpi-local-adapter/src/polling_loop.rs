//! Polling loop internals: state management, outcome processing, and the async loop.

use iotkit_core_types::{AdapterCommand, AdapterEvent, DeviceKey, SensorIdentity, SensorReading};
use tokio::sync::mpsc;

use crate::config::{sensor_ic_name, RpiLocalConfig, SensorTarget};

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

/// Executes one poll cycle synchronously (called inside spawn_blocking).
/// For each target: Active → read, Pending → probe only (first read is next tick).
pub(crate) fn poll_cycle(
    targets: &[SensorTarget],
    states: &[TargetState],
    bus_path: &str,
) -> Vec<PollOutcome> {
    let mut outcomes = Vec::new();

    for (i, target) in targets.iter().enumerate() {
        match &states[i] {
            TargetState::Pending => {
                match crate::sensors::probe(&target.kind, bus_path, target.address) {
                    Ok(identity) => {
                        let key = device_key_for(target);
                        outcomes.push(PollOutcome::Discovered {
                            target_index: i,
                            key,
                            identity,
                        });
                        // Do NOT read in the same cycle — sensors like OPT3001
                        // need conversion latency after init. First read happens
                        // on the next poll tick.
                    }
                    Err(msg) => {
                        outcomes.push(PollOutcome::ProbeFailed {
                            target_index: i,
                            message: msg,
                        });
                    }
                }
            }
            TargetState::Active(key) => {
                match crate::sensors::read(&target.kind, bus_path, target.address) {
                    Ok(reading) => {
                        outcomes.push(PollOutcome::Reading {
                            key: key.clone(),
                            reading,
                        });
                    }
                    Err(msg) => {
                        outcomes.push(PollOutcome::ReadError {
                            key: key.clone(),
                            message: msg,
                        });
                    }
                }
            }
        }
    }

    outcomes
}

/// The main async polling loop. Runs as a spawned tokio task.
pub(crate) async fn polling_loop(
    config: RpiLocalConfig,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
) {
    let period = std::time::Duration::from_millis(config.poll_interval_ms);
    let bus_path = config.bus_path.clone();
    let targets = config.targets.clone();

    // Initialize all targets as Pending.
    let mut states: Vec<TargetState> = vec![TargetState::Pending; targets.len()];

    // Startup probe: one spawn_blocking call for all targets.
    {
        let bus = bus_path.clone();
        let tgts = targets.clone();
        let st = states.clone();
        match tokio::task::spawn_blocking(move || poll_cycle(&tgts, &st, &bus)).await {
            Ok(outcomes) => {
                let events = apply_outcomes(outcomes, &mut states);
                for event in events {
                    if event_tx.send(event).await.is_err() {
                        tracing::warn!("Event channel closed during startup probe");
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Startup probe spawn_blocking failed");
                let _ = event_tx.send(AdapterEvent::AdapterError {
                    device_key: None,
                    error: format!("startup probe failed: {}", e),
                }).await;
                return;
            }
        }
    }

    tracing::info!(
        active = states.iter().filter(|s| matches!(s, TargetState::Active(_))).count(),
        pending = states.iter().filter(|s| matches!(s, TargetState::Pending)).count(),
        "Startup probe complete, entering poll loop",
    );

    // Use interval_at to avoid immediate first tick after startup probe.
    let start = tokio::time::Instant::now() + period;
    let mut interval = tokio::time::interval_at(start, period);

    loop {
        tokio::select! {
            biased;
            cmd = command_rx.recv() => {
                match cmd {
                    Some(AdapterCommand::Shutdown) | None => {
                        tracing::info!("rpi-local-adapter shutting down");
                        return;
                    }
                    Some(AdapterCommand::DeviceCommand(dev_cmd)) => {
                        // Only report with device_key if we own that device.
                        let known = states.iter().any(|s| {
                            matches!(s, TargetState::Active(k) if k == &dev_cmd.device_key)
                        });
                        let _ = event_tx.send(AdapterEvent::AdapterError {
                            device_key: if known { Some(dev_cmd.device_key) } else { None },
                            error: "unsupported command: rpi-local-adapter v1 does not handle DeviceCommand".to_string(),
                        }).await;
                    }
                }
            }
            _ = interval.tick() => {
                let bus = bus_path.clone();
                let tgts = targets.clone();
                let st = states.clone();
                match tokio::task::spawn_blocking(move || poll_cycle(&tgts, &st, &bus)).await {
                    Ok(outcomes) => {
                        let events = apply_outcomes(outcomes, &mut states);
                        for event in events {
                            if event_tx.send(event).await.is_err() {
                                tracing::warn!("Event channel closed, exiting poll loop");
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Poll cycle spawn_blocking failed");
                        let _ = event_tx.send(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!("poll cycle failed: {}", e),
                        }).await;
                    }
                }
            }
        }
    }
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
