use std::sync::Arc;
use tokio::sync::mpsc;

use iotkit_core_types::{
    AdapterCommand, AdapterEvent, DeviceKey, SensorIdentity, SensorReading,
};

use crate::SensorDriver;

// ── Internal failure threshold constants ─────────────────

const MAX_READ_FAILURES: u32 = 5;
const MAX_PROBE_FAILURES: u32 = 10;

// ── TargetState ──────────────────────────────────────────

pub(crate) enum TargetState {
    Pending {
        consecutive_probe_failures: u32,
        escalation_emitted: bool,
    },
    Active {
        key: DeviceKey,
        consecutive_read_failures: u32,
    },
}

impl TargetState {
    pub(crate) fn new_pending() -> Self {
        TargetState::Pending {
            consecutive_probe_failures: 0,
            escalation_emitted: false,
        }
    }
}

// ── TargetRuntime ────────────────────────────────────────

pub(crate) struct TargetRuntime {
    pub address: u8,
    pub driver: Arc<dyn SensorDriver>,
    pub key_suffix: String,
}

// ── PollOutcome ──────────────────────────────────────────

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
        target_index: usize,
        key: DeviceKey,
        message: String,
    },
    ProbeFailed {
        target_index: usize,
        message: String,
    },
}

// ── device_key_for ───────────────────────────────────────

pub(crate) fn device_key_for(address: u8, key_suffix: &str) -> DeviceKey {
    DeviceKey::new(format!("i2c:0x{address:02x}:{key_suffix}"))
}

// ── apply_outcomes ───────────────────────────────────────

pub(crate) fn apply_outcomes(
    outcomes: Vec<PollOutcome>,
    states: &mut [TargetState],
    targets: &[TargetRuntime],
) -> Vec<AdapterEvent> {
    let mut events = Vec::new();

    for outcome in outcomes {
        match outcome {
            PollOutcome::Discovered {
                target_index,
                key,
                identity,
            } => {
                states[target_index] = TargetState::Active {
                    key: key.clone(),
                    consecutive_read_failures: 0,
                };
                events.push(AdapterEvent::DeviceDiscovered {
                    device_key: key,
                    identity,
                });
            }

            PollOutcome::Reading { key, reading } => {
                // Reset read failure counter for matching Active state.
                for state in states.iter_mut() {
                    if let &mut TargetState::Active {
                        key: ref k,
                        ref mut consecutive_read_failures,
                    } = state
                    {
                        if k.as_str() == key.as_str() {
                            *consecutive_read_failures = 0;
                            break;
                        }
                    }
                }
                events.push(AdapterEvent::SensorData {
                    device_key: key,
                    reading,
                    rssi: None,
                    battery_pct: None,
                });
            }

            PollOutcome::ReadError {
                target_index,
                key,
                message,
            } => {
                if let TargetState::Active {
                    ref mut consecutive_read_failures,
                    ..
                } = states[target_index]
                {
                    *consecutive_read_failures += 1;
                    let n = *consecutive_read_failures;

                    events.push(AdapterEvent::AdapterError {
                        device_key: Some(key.clone()),
                        error: message.clone(),
                    });

                    if n >= MAX_READ_FAILURES {
                        let reason = format!(
                            "consecutive read failures ({n}): {message}"
                        );
                        tracing::info!(
                            device_key = key.as_str(),
                            "device lost: {reason}"
                        );
                        events.push(AdapterEvent::DeviceLost {
                            device_key: key,
                            reason,
                        });
                        states[target_index] = TargetState::new_pending();
                    }
                }
            }

            PollOutcome::ProbeFailed {
                target_index,
                message,
            } => {
                if let TargetState::Pending {
                    ref mut consecutive_probe_failures,
                    ref mut escalation_emitted,
                } = states[target_index]
                {
                    *consecutive_probe_failures += 1;
                    let n = *consecutive_probe_failures;

                    if n >= MAX_PROBE_FAILURES && !*escalation_emitted {
                        let addr = targets[target_index].address;
                        events.push(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!(
                                "target 0x{addr:02x} probe failed {n} consecutive times: {message}"
                            ),
                        });
                        *escalation_emitted = true;
                    } else {
                        tracing::warn!(
                            target_index,
                            "probe failed: {message}"
                        );
                    }
                }
            }
        }
    }

    events
}

// ── poll_cycle ───────────────────────────────────────────

pub(crate) fn poll_cycle(
    targets: &[TargetRuntime],
    states: &[TargetState],
    bus_path: &str,
) -> Vec<PollOutcome> {
    let mut outcomes = Vec::new();
    for (i, target) in targets.iter().enumerate() {
        match &states[i] {
            TargetState::Pending { .. } => {
                match target.driver.probe(bus_path, target.address) {
                    Ok(identity) => {
                        let key = device_key_for(target.address, &target.key_suffix);
                        outcomes.push(PollOutcome::Discovered { target_index: i, key, identity });
                    }
                    Err(msg) => {
                        outcomes.push(PollOutcome::ProbeFailed { target_index: i, message: msg });
                    }
                }
            }
            TargetState::Active { key, .. } => {
                match target.driver.read(bus_path, target.address) {
                    Ok(reading) => {
                        outcomes.push(PollOutcome::Reading { key: key.clone(), reading });
                    }
                    Err(msg) => {
                        outcomes.push(PollOutcome::ReadError { target_index: i, key: key.clone(), message: msg });
                    }
                }
            }
        }
    }
    outcomes
}

// ── Stub polling loop ────────────────────────────────────

/// Stub polling loop. Will be fleshed out in a later task.
pub(crate) async fn polling_loop(
    _bus_path: String,
    _targets: Vec<(u8, Arc<dyn SensorDriver>, Option<String>)>,
    _poll_interval_ms: u64,
    _event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
) {
    // Wait for shutdown or channel close.
    while let Some(cmd) = command_rx.recv().await {
        if matches!(cmd, AdapterCommand::Shutdown) {
            break;
        }
    }
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::{
        ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading, SensorType,
    };
    use std::collections::{BTreeMap, VecDeque};

    // ── MockDriver ───────────────────────────────────────

    struct MockDriver {
        name: &'static str,
        probe_results: std::sync::Mutex<VecDeque<Result<SensorIdentity, String>>>,
        read_results: std::sync::Mutex<VecDeque<Result<SensorReading, String>>>,
    }

    impl MockDriver {
        fn new(
            name: &'static str,
            probe_results: Vec<Result<SensorIdentity, String>>,
            read_results: Vec<Result<SensorReading, String>>,
        ) -> Self {
            MockDriver {
                name,
                probe_results: std::sync::Mutex::new(VecDeque::from(probe_results)),
                read_results: std::sync::Mutex::new(VecDeque::from(read_results)),
            }
        }
    }

    impl SensorDriver for MockDriver {
        fn probe(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
            self.probe_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("no more probe results for {}", self.name))
        }
        fn read(&self, _bus_path: &str, _address: u8) -> Result<SensorReading, String> {
            self.read_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("no more read results for {}", self.name))
        }
        fn ic_name(&self) -> &'static str {
            self.name
        }
    }

    fn make_mock_target(
        address: u8,
        suffix: &str,
        probe_results: Vec<Result<SensorIdentity, String>>,
        read_results: Vec<Result<SensorReading, String>>,
    ) -> TargetRuntime {
        TargetRuntime {
            address,
            driver: Arc::new(MockDriver::new("MOCK", probe_results, read_results)),
            key_suffix: suffix.to_string(),
        }
    }

    // ── poll_cycle tests ─────────────────────────────────

    #[test]
    fn pending_target_probe_success() {
        let identity = make_identity();
        let targets = vec![make_mock_target(
            0x40,
            "temperature",
            vec![Ok(identity.clone())],
            vec![],
        )];
        let states = vec![TargetState::new_pending()];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::Discovered { target_index, key, identity: id } => {
                assert_eq!(*target_index, 0);
                assert_eq!(key.as_str(), "i2c:0x40:temperature");
                assert_eq!(id.ic_part_number, identity.ic_part_number);
            }
            _ => panic!("expected Discovered"),
        }
    }

    #[test]
    fn pending_target_probe_failure() {
        let targets = vec![make_mock_target(
            0x40,
            "temperature",
            vec![Err("NACK".into())],
            vec![],
        )];
        let states = vec![TargetState::new_pending()];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::ProbeFailed { target_index, message } => {
                assert_eq!(*target_index, 0);
                assert_eq!(message, "NACK");
            }
            _ => panic!("expected ProbeFailed"),
        }
    }

    #[test]
    fn active_target_read_success() {
        let reading = make_reading();
        let key = device_key_for(0x40, "temperature");
        let targets = vec![make_mock_target(
            0x40,
            "temperature",
            vec![],
            vec![Ok(reading.clone())],
        )];
        let states = vec![TargetState::Active {
            key: key.clone(),
            consecutive_read_failures: 0,
        }];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::Reading { key: k, reading: r } => {
                assert_eq!(k.as_str(), "i2c:0x40:temperature");
                assert_eq!(r.values, reading.values);
            }
            _ => panic!("expected Reading"),
        }
    }

    #[test]
    fn active_target_read_failure() {
        let key = device_key_for(0x40, "temperature");
        let targets = vec![make_mock_target(
            0x40,
            "temperature",
            vec![],
            vec![Err("i/o timeout".into())],
        )];
        let states = vec![TargetState::Active {
            key: key.clone(),
            consecutive_read_failures: 0,
        }];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::ReadError { target_index, key: k, message } => {
                assert_eq!(*target_index, 0);
                assert_eq!(k.as_str(), "i2c:0x40:temperature");
                assert_eq!(message, "i/o timeout");
            }
            _ => panic!("expected ReadError"),
        }
    }

    // ── apply_outcomes tests ─────────────────────────────

    /// Minimal no-op driver for tests (apply_outcomes never calls driver methods).
    struct StubDriver;

    impl SensorDriver for StubDriver {
        fn probe(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
            unimplemented!("not called by apply_outcomes")
        }
        fn read(&self, _bus_path: &str, _address: u8) -> Result<SensorReading, String> {
            unimplemented!("not called by apply_outcomes")
        }
        fn ic_name(&self) -> &'static str {
            "STUB"
        }
    }

    fn make_target(address: u8, suffix: &str) -> TargetRuntime {
        TargetRuntime {
            address,
            driver: Arc::new(StubDriver),
            key_suffix: suffix.to_string(),
        }
    }

    fn make_identity() -> SensorIdentity {
        SensorIdentity {
            manufacturer: "Test".into(),
            ic_part_number: "STUB".into(),
            sensor_type: SensorType::Temperature,
            connection: ConnectionInfo {
                kind: ConnectionKind::I2c,
                parameters: BTreeMap::new(),
            },
        }
    }

    fn make_reading() -> SensorReading {
        SensorReading::new(SensorType::Temperature, vec![25.0], vec!["temp_c"])
    }

    #[test]
    fn probe_success_transitions_to_active_and_emits_discovered() {
        let targets = vec![make_target(0x40, "temperature")];
        let mut states = vec![TargetState::new_pending()];
        let key = device_key_for(0x40, "temperature");
        let identity = make_identity();

        let outcomes = vec![PollOutcome::Discovered {
            target_index: 0,
            key: key.clone(),
            identity: identity.clone(),
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AdapterEvent::DeviceDiscovered {
                device_key,
                identity: id,
            } => {
                assert_eq!(device_key.as_str(), "i2c:0x40:temperature");
                assert_eq!(id.ic_part_number, "STUB");
            }
            other => panic!("expected DeviceDiscovered, got {other:?}"),
        }

        match &states[0] {
            TargetState::Active {
                key: k,
                consecutive_read_failures,
            } => {
                assert_eq!(k.as_str(), "i2c:0x40:temperature");
                assert_eq!(*consecutive_read_failures, 0);
            }
            _ => panic!("expected Active state"),
        }
    }

    #[test]
    fn read_success_emits_sensor_data_and_resets_counter() {
        let targets = vec![make_target(0x40, "temperature")];
        let key = device_key_for(0x40, "temperature");
        let mut states = vec![TargetState::Active {
            key: key.clone(),
            consecutive_read_failures: 3,
        }];

        let outcomes = vec![PollOutcome::Reading {
            key: key.clone(),
            reading: make_reading(),
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AdapterEvent::SensorData {
                device_key,
                reading,
                rssi,
                battery_pct,
            } => {
                assert_eq!(device_key.as_str(), "i2c:0x40:temperature");
                assert_eq!(reading.values, vec![25.0]);
                assert_eq!(*rssi, None);
                assert_eq!(*battery_pct, None);
            }
            other => panic!("expected SensorData, got {other:?}"),
        }

        // Counter should be reset to 0.
        match &states[0] {
            TargetState::Active {
                consecutive_read_failures,
                ..
            } => assert_eq!(*consecutive_read_failures, 0),
            _ => panic!("expected Active state"),
        }
    }

    #[test]
    fn read_failure_emits_error_and_increments_counter() {
        let targets = vec![make_target(0x40, "temperature")];
        let key = device_key_for(0x40, "temperature");
        let mut states = vec![TargetState::Active {
            key: key.clone(),
            consecutive_read_failures: 0,
        }];

        let outcomes = vec![PollOutcome::ReadError {
            target_index: 0,
            key: key.clone(),
            message: "i/o timeout".into(),
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AdapterEvent::AdapterError {
                device_key,
                error,
            } => {
                assert_eq!(device_key.as_ref().unwrap().as_str(), "i2c:0x40:temperature");
                assert!(error.contains("i/o timeout"));
            }
            other => panic!("expected AdapterError, got {other:?}"),
        }

        match &states[0] {
            TargetState::Active {
                consecutive_read_failures,
                ..
            } => assert_eq!(*consecutive_read_failures, 1),
            _ => panic!("expected Active state"),
        }
    }

    #[test]
    fn read_failures_at_threshold_emits_lost_and_transitions_to_pending() {
        let targets = vec![make_target(0x40, "temperature")];
        let key = device_key_for(0x40, "temperature");
        let mut states = vec![TargetState::Active {
            key: key.clone(),
            consecutive_read_failures: MAX_READ_FAILURES - 1,
        }];

        let outcomes = vec![PollOutcome::ReadError {
            target_index: 0,
            key: key.clone(),
            message: "NACK".into(),
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        assert_eq!(events.len(), 2);

        // First: AdapterError
        assert!(matches!(&events[0], AdapterEvent::AdapterError { .. }));

        // Second: DeviceLost
        match &events[1] {
            AdapterEvent::DeviceLost { device_key, reason } => {
                assert_eq!(device_key.as_str(), "i2c:0x40:temperature");
                assert!(reason.contains("consecutive read failures (5)"));
                assert!(reason.contains("NACK"));
            }
            other => panic!("expected DeviceLost, got {other:?}"),
        }

        // State should be Pending.
        match &states[0] {
            TargetState::Pending {
                consecutive_probe_failures,
                escalation_emitted,
            } => {
                assert_eq!(*consecutive_probe_failures, 0);
                assert!(!*escalation_emitted);
            }
            _ => panic!("expected Pending state"),
        }
    }

    #[test]
    fn probe_failure_below_threshold_emits_no_event() {
        let targets = vec![make_target(0x40, "temperature")];
        let mut states = vec![TargetState::new_pending()];

        let outcomes = vec![PollOutcome::ProbeFailed {
            target_index: 0,
            message: "NACK".into(),
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        assert!(events.is_empty());

        match &states[0] {
            TargetState::Pending {
                consecutive_probe_failures,
                escalation_emitted,
            } => {
                assert_eq!(*consecutive_probe_failures, 1);
                assert!(!*escalation_emitted);
            }
            _ => panic!("expected Pending state"),
        }
    }

    #[test]
    fn probe_failure_at_threshold_emits_one_error() {
        let targets = vec![make_target(0x40, "temperature")];
        let mut states = vec![TargetState::Pending {
            consecutive_probe_failures: MAX_PROBE_FAILURES - 1,
            escalation_emitted: false,
        }];

        let outcomes = vec![PollOutcome::ProbeFailed {
            target_index: 0,
            message: "device not found".into(),
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AdapterEvent::AdapterError {
                device_key,
                error,
            } => {
                assert!(device_key.is_none());
                assert!(error.contains("target 0x40 probe failed 10 consecutive times"));
                assert!(error.contains("device not found"));
            }
            other => panic!("expected AdapterError, got {other:?}"),
        }

        // escalation_emitted should now be true
        match &states[0] {
            TargetState::Pending {
                escalation_emitted, ..
            } => assert!(*escalation_emitted),
            _ => panic!("expected Pending state"),
        }

        // A second probe failure should NOT emit another event.
        let outcomes2 = vec![PollOutcome::ProbeFailed {
            target_index: 0,
            message: "device not found".into(),
        }];
        let events2 = apply_outcomes(outcomes2, &mut states, &targets);
        assert!(events2.is_empty());
    }

    #[test]
    fn probe_success_after_escalation_resets_counter_and_flag() {
        let targets = vec![make_target(0x40, "temperature")];
        let mut states = vec![TargetState::Pending {
            consecutive_probe_failures: 15,
            escalation_emitted: true,
        }];

        let key = device_key_for(0x40, "temperature");
        let outcomes = vec![PollOutcome::Discovered {
            target_index: 0,
            key: key.clone(),
            identity: make_identity(),
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AdapterEvent::DeviceDiscovered { .. }));

        // State should be Active with zero failures.
        match &states[0] {
            TargetState::Active {
                consecutive_read_failures,
                ..
            } => assert_eq!(*consecutive_read_failures, 0),
            _ => panic!("expected Active state"),
        }
    }

    #[test]
    fn discovered_only_no_same_cycle_read() {
        let targets = vec![make_target(0x40, "temperature")];
        let mut states = vec![TargetState::new_pending()];
        let key = device_key_for(0x40, "temperature");

        let outcomes = vec![PollOutcome::Discovered {
            target_index: 0,
            key,
            identity: make_identity(),
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        // Only DeviceDiscovered, no SensorData.
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AdapterEvent::DeviceDiscovered { .. }));
    }

    #[test]
    fn multiple_targets_independent() {
        let targets = vec![
            make_target(0x40, "temperature"),
            make_target(0x29, "ranging"),
        ];
        let key_a = device_key_for(0x40, "temperature");
        let _key_b = device_key_for(0x29, "ranging");
        let mut states = vec![
            TargetState::Active {
                key: key_a.clone(),
                consecutive_read_failures: 0,
            },
            TargetState::new_pending(),
        ];

        let outcomes = vec![
            PollOutcome::Reading {
                key: key_a.clone(),
                reading: make_reading(),
            },
            PollOutcome::ProbeFailed {
                target_index: 1,
                message: "NACK".into(),
            },
        ];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        // Target A: SensorData
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AdapterEvent::SensorData { .. }));

        // Target A: still Active with counter 0.
        match &states[0] {
            TargetState::Active {
                consecutive_read_failures,
                ..
            } => assert_eq!(*consecutive_read_failures, 0),
            _ => panic!("expected Active state for target A"),
        }

        // Target B: still Pending with incremented probe failures.
        match &states[1] {
            TargetState::Pending {
                consecutive_probe_failures,
                ..
            } => assert_eq!(*consecutive_probe_failures, 1),
            _ => panic!("expected Pending state for target B"),
        }
    }
}
