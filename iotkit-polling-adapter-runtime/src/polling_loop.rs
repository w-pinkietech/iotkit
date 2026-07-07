use std::panic;
use std::sync::Arc;
use tokio::sync::mpsc;

use iotkit_core_types::{
    AdapterCommand, AdapterEvent, AdapterId, DeviceKey, SensorIdentity, SensorReading,
};
use iotkit_ingest_client::IngestClient;

use crate::{PollingAdapterConfig, SensorDriver};

// ── Internal failure threshold constants ─────────────────

const MAX_READ_FAILURES: u32 = 5;
const MAX_DETECT_FAILURES: u32 = 10;
const MAX_INIT_FAILURES: u32 = 5;

// ── TargetState ──────────────────────────────────────────

#[derive(Clone, Debug)]
pub(crate) enum TargetState {
    Pending {
        consecutive_detect_failures: u32,
        escalation_emitted: bool,
    },
    Detected {
        identity: SensorIdentity,
        consecutive_init_failures: u32,
    },
    Active {
        key: DeviceKey,
        consecutive_read_failures: u32,
    },
}

impl TargetState {
    pub(crate) fn new_pending() -> Self {
        TargetState::Pending {
            consecutive_detect_failures: 0,
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

#[derive(Debug)]
pub(crate) enum PollOutcome {
    /// detect() + init() both succeeded. Emit DeviceDiscovered.
    Discovered {
        target_index: usize,
        key: DeviceKey,
        identity: SensorIdentity,
    },
    /// detect() succeeded but init() failed. Enter/maintain Detected state.
    InitFailed {
        target_index: usize,
        identity: SensorIdentity,
        message: String,
        #[allow(dead_code)]
        is_panic: bool,
    },
    /// Successful sensor reading.
    Reading {
        key: DeviceKey,
        reading: SensorReading,
        observed_at: std::time::SystemTime,
    },
    /// read() failed.
    ReadError {
        target_index: usize,
        key: DeviceKey,
        message: String,
        #[allow(dead_code)]
        is_panic: bool,
    },
    /// detect() failed.
    DetectFailed {
        target_index: usize,
        message: String,
        is_panic: bool,
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

            PollOutcome::InitFailed {
                target_index,
                identity,
                message,
                ..
            } => {
                let new_failures = match &states[target_index] {
                    TargetState::Detected {
                        consecutive_init_failures,
                        ..
                    } => consecutive_init_failures + 1,
                    _ => 1, // First init failure (from Pending same-cycle)
                };

                if new_failures >= MAX_INIT_FAILURES {
                    // Too many init failures — return to Pending for fresh detect
                    states[target_index] = TargetState::new_pending();
                } else {
                    states[target_index] = TargetState::Detected {
                        identity,
                        consecutive_init_failures: new_failures,
                    };
                }

                events.push(AdapterEvent::AdapterError {
                    device_key: None,
                    error: format!(
                        "init failed ({}/{}): {}",
                        new_failures, MAX_INIT_FAILURES, message,
                    ),
                });
            }

            PollOutcome::Reading {
                key,
                reading,
                observed_at,
            } => {
                // Reset read failure counter for matching Active state.
                for state in states.iter_mut() {
                    if let TargetState::Active {
                        key: k,
                        consecutive_read_failures,
                    } = state
                        && k.as_str() == key.as_str()
                    {
                        *consecutive_read_failures = 0;
                        break;
                    }
                }
                events.push(AdapterEvent::SensorData {
                    device_key: key,
                    reading,
                    rssi: None,
                    battery_pct: None,
                    ingested_at: observed_at,
                });
            }

            PollOutcome::ReadError {
                target_index,
                key,
                message,
                ..
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
                        let reason = format!("consecutive read failures ({n}): {message}");
                        tracing::info!(device_key = key.as_str(), "device lost: {reason}");
                        events.push(AdapterEvent::DeviceLost {
                            device_key: key,
                            reason,
                        });
                        states[target_index] = TargetState::new_pending();
                    }
                }
            }

            PollOutcome::DetectFailed {
                target_index,
                message,
                is_panic,
            } => {
                if let TargetState::Pending {
                    ref mut consecutive_detect_failures,
                    ref mut escalation_emitted,
                } = states[target_index]
                {
                    *consecutive_detect_failures += 1;
                    let n = *consecutive_detect_failures;

                    if is_panic {
                        // Panic: emit immediately but do NOT consume escalation_emitted,
                        // so the normal threshold path still fires later if needed.
                        // (Preserves current probe panic behavior.)
                        let addr = targets[target_index].address;
                        events.push(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!(
                                "target 0x{addr:02x} detect failed (driver panic): {message}"
                            ),
                        });
                    } else if n >= MAX_DETECT_FAILURES && !*escalation_emitted {
                        let addr = targets[target_index].address;
                        events.push(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!(
                                "target 0x{addr:02x} detect failed {n} consecutive times: {message}"
                            ),
                        });
                        *escalation_emitted = true;
                    } else {
                        tracing::warn!(target_index, "detect failed: {message}");
                    }
                }
            }
        }
    }

    events
}

// ── panic_message ────────────────────────────────────────

fn panic_message(val: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = val.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = val.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
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
                // Phase 1: detect (read-only)
                let detect_result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    target.driver.detect(bus_path, target.address)
                }));

                match detect_result {
                    Ok(Ok(identity)) => {
                        // detect succeeded — immediately try init (same cycle)
                        let init_result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                            target.driver.init(bus_path, target.address)
                        }));
                        match init_result {
                            Ok(Ok(())) => {
                                let key = device_key_for(target.address, &target.key_suffix);
                                outcomes.push(PollOutcome::Discovered {
                                    target_index: i,
                                    key,
                                    identity,
                                });
                            }
                            Ok(Err(msg)) => {
                                outcomes.push(PollOutcome::InitFailed {
                                    target_index: i,
                                    identity,
                                    message: msg,
                                    is_panic: false,
                                });
                            }
                            Err(panic_val) => {
                                let msg = panic_message(&panic_val);
                                tracing::error!(
                                    address = format_args!("0x{:02x}", target.address),
                                    bus_path,
                                    "driver panicked during init: {msg}",
                                );
                                outcomes.push(PollOutcome::InitFailed {
                                    target_index: i,
                                    identity,
                                    message: format!(
                                        "driver panic during init 0x{:02x}@{}: {}",
                                        target.address, bus_path, msg,
                                    ),
                                    is_panic: true,
                                });
                            }
                        }
                    }
                    Ok(Err(msg)) => {
                        outcomes.push(PollOutcome::DetectFailed {
                            target_index: i,
                            message: msg,
                            is_panic: false,
                        });
                    }
                    Err(panic_val) => {
                        let msg = panic_message(&panic_val);
                        tracing::error!(
                            address = format_args!("0x{:02x}", target.address),
                            bus_path,
                            "driver panicked during detect: {msg}",
                        );
                        outcomes.push(PollOutcome::DetectFailed {
                            target_index: i,
                            message: format!(
                                "driver panic during detect 0x{:02x}@{}: {}",
                                target.address, bus_path, msg,
                            ),
                            is_panic: true,
                        });
                    }
                }
            }
            TargetState::Detected { identity, .. } => {
                // Phase 2: init retry (from previous failed init)
                let init_result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    target.driver.init(bus_path, target.address)
                }));
                match init_result {
                    Ok(Ok(())) => {
                        let key = device_key_for(target.address, &target.key_suffix);
                        outcomes.push(PollOutcome::Discovered {
                            target_index: i,
                            key,
                            identity: identity.clone(),
                        });
                    }
                    Ok(Err(msg)) => {
                        outcomes.push(PollOutcome::InitFailed {
                            target_index: i,
                            identity: identity.clone(),
                            message: msg,
                            is_panic: false,
                        });
                    }
                    Err(panic_val) => {
                        let msg = panic_message(&panic_val);
                        tracing::error!(
                            address = format_args!("0x{:02x}", target.address),
                            bus_path,
                            "driver panicked during init: {msg}",
                        );
                        outcomes.push(PollOutcome::InitFailed {
                            target_index: i,
                            identity: identity.clone(),
                            message: format!(
                                "driver panic during init 0x{:02x}@{}: {}",
                                target.address, bus_path, msg,
                            ),
                            is_panic: true,
                        });
                    }
                }
            }
            TargetState::Active { key, .. } => {
                // Phase 3: read (unchanged logic)
                match panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    target.driver.read(bus_path, target.address)
                })) {
                    Ok(Ok(reading)) => {
                        outcomes.push(PollOutcome::Reading {
                            key: key.clone(),
                            reading,
                            observed_at: std::time::SystemTime::now(),
                        });
                    }
                    Ok(Err(msg)) => {
                        outcomes.push(PollOutcome::ReadError {
                            target_index: i,
                            key: key.clone(),
                            message: msg,
                            is_panic: false,
                        });
                    }
                    Err(panic_val) => {
                        let msg = panic_message(&panic_val);
                        tracing::error!(
                            address = format_args!("0x{:02x}", target.address),
                            bus_path,
                            "driver panicked during read: {msg}",
                        );
                        outcomes.push(PollOutcome::ReadError {
                            target_index: i,
                            key: key.clone(),
                            message: format!(
                                "driver panic during read 0x{:02x}@{}: {}",
                                target.address, bus_path, msg,
                            ),
                            is_panic: true,
                        });
                    }
                }
            }
        }
    }
    outcomes
}

// ── polling_loop ─────────────────────────────────────────

pub(crate) async fn polling_loop(
    adapter_id: AdapterId,
    ingest: Option<IngestClient>,
    config: PollingAdapterConfig,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
) {
    use std::time::Duration;
    use tokio::time::{Instant, MissedTickBehavior, interval_at};

    let bus_path = config.bus_path;
    let poll_interval_ms = config.poll_interval_ms;
    let period = Duration::from_millis(poll_interval_ms);

    // Build Arc<Vec<TargetRuntime>> from config.
    let targets: Arc<Vec<TargetRuntime>> = Arc::new(
        config
            .targets
            .into_iter()
            .map(|t| {
                let key_suffix = t
                    .key_suffix
                    .unwrap_or_else(|| t.driver.ic_name().to_string());
                TargetRuntime {
                    address: t.address,
                    driver: t.driver,
                    key_suffix,
                }
            })
            .collect(),
    );

    let mut states: Vec<TargetState> = targets.iter().map(|_| TargetState::new_pending()).collect();

    // ── Startup probe ────────────────────────────────────
    if !targets.is_empty() {
        let t = Arc::clone(&targets);
        let bp = bus_path.clone();
        let s_snap: Vec<TargetState> = states.to_vec();

        let outcomes = match tokio::task::spawn_blocking(move || poll_cycle(&t, &s_snap, &bp)).await
        {
            Ok(outcomes) => outcomes,
            Err(e) => {
                tracing::error!("startup probe spawn_blocking failed: {e}");
                let event = AdapterEvent::AdapterError {
                    device_key: None,
                    error: format!("fatal: startup probe task failed: {e}"),
                };
                if event_tx.send(event).await.is_err() {
                    tracing::warn!("event channel closed while sending fatal startup error");
                }
                return;
            }
        };

        let all_failed = !outcomes.is_empty()
            && outcomes.iter().all(|o| {
                matches!(
                    o,
                    PollOutcome::DetectFailed { .. } | PollOutcome::InitFailed { .. }
                )
            });

        let events = apply_outcomes(outcomes, &mut states, &targets);

        for event in events {
            submit_ingest(&adapter_id, &ingest, &event);
            if event_tx.send(event).await.is_err() {
                tracing::warn!("event channel closed during startup probe");
                return;
            }
        }

        if all_failed {
            let addrs: Vec<String> = targets
                .iter()
                .map(|t| format!("0x{:02x}", t.address))
                .collect();
            let event = AdapterEvent::AdapterError {
                device_key: None,
                error: format!(
                    "all targets failed startup probe on bus {}: [{}]",
                    bus_path,
                    addrs.join(", "),
                ),
            };
            if event_tx.send(event).await.is_err() {
                tracing::warn!("event channel closed during startup probe");
                return;
            }
        }
    }

    // ── Timer ────────────────────────────────────────────
    let now = Instant::now();
    let mut ticker = interval_at(now + period, period);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    // ── Main loop ────────────────────────────────────────
    loop {
        if event_tx.is_closed() {
            tracing::warn!("event channel closed, exiting polling loop");
            return;
        }

        tokio::select! {
            cmd_opt = command_rx.recv() => {
                match cmd_opt {
                    Some(AdapterCommand::Shutdown) | None => return,
                    Some(AdapterCommand::DeviceCommand(cmd)) => {
                        let event = AdapterEvent::AdapterError {
                            device_key: Some(cmd.device_key),
                            error: "unsupported: I2C polling adapter v1 does not handle DeviceCommand".into(),
                        };
                        if event_tx.send(event).await.is_err() {
                            tracing::warn!("event channel closed while sending DeviceCommand rejection");
                            return;
                        }
                    }
                }
            }
            _ = ticker.tick() => {
                let t = Arc::clone(&targets);
                let bp = bus_path.clone();
                let s_snap: Vec<TargetState> = states.to_vec();

                let cycle_start = Instant::now();
                let outcomes = match tokio::task::spawn_blocking(move || poll_cycle(&t, &s_snap, &bp)).await {
                    Ok(outcomes) => outcomes,
                    Err(e) => {
                        tracing::error!("poll cycle spawn_blocking failed: {e}");
                        let event = AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!("fatal: poll cycle task failed: {e}"),
                        };
                        if event_tx.send(event).await.is_err() {
                            tracing::warn!("event channel closed while sending fatal poll cycle error");
                        }
                        return;
                    }
                };
                let cycle_duration = cycle_start.elapsed();

                if cycle_duration > period {
                    tracing::warn!(
                        cycle_ms = cycle_duration.as_millis() as u64,
                        poll_interval_ms,
                        "poll cycle exceeded interval"
                    );
                }

                let events = apply_outcomes(outcomes, &mut states, &targets);
                for event in events {
                    submit_ingest(&adapter_id, &ingest, &event);
                    if event_tx.send(event).await.is_err() {
                        tracing::warn!("event channel closed during poll cycle");
                        return;
                    }
                }
            }
        }
    }
}

fn submit_ingest(adapter_id: &AdapterId, ingest: &Option<IngestClient>, event: &AdapterEvent) {
    if let (
        Some(client),
        AdapterEvent::SensorData {
            device_key,
            reading,
            ..
        },
    ) = (ingest, event)
    {
        match crate::ingest_map::to_items(adapter_id, device_key, reading) {
            Some(items) => {
                let envelope = iotkit_ingest_client::new_envelope(adapter_id.as_str(), items);
                if let Err(e) = client.try_submit(envelope) {
                    match e {
                        iotkit_ingest_client::IngestClientError::Full => {
                            tracing::warn!("ingest queue full; dropping reading");
                        }
                        iotkit_ingest_client::IngestClientError::Closed => {
                            tracing::warn!("ingest client closed; dropping reading");
                        }
                    }
                }
            }
            None => tracing::warn!(
                device_key = device_key.as_str(),
                "no measurement mapping; reading not ingested"
            ),
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
        detect_results: std::sync::Mutex<VecDeque<Result<SensorIdentity, String>>>,
        init_results: std::sync::Mutex<VecDeque<Result<(), String>>>,
        read_results: std::sync::Mutex<VecDeque<Result<SensorReading, String>>>,
    }

    impl MockDriver {
        fn new(
            name: &'static str,
            detect_results: Vec<Result<SensorIdentity, String>>,
            init_results: Vec<Result<(), String>>,
            read_results: Vec<Result<SensorReading, String>>,
        ) -> Self {
            MockDriver {
                name,
                detect_results: std::sync::Mutex::new(VecDeque::from(detect_results)),
                init_results: std::sync::Mutex::new(VecDeque::from(init_results)),
                read_results: std::sync::Mutex::new(VecDeque::from(read_results)),
            }
        }
    }

    impl SensorDriver for MockDriver {
        fn detect(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
            self.detect_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("no more detect results for {}", self.name))
        }
        fn init(&self, _bus_path: &str, _address: u8) -> Result<(), String> {
            self.init_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("no more init results for {}", self.name))
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
        detect_results: Vec<Result<SensorIdentity, String>>,
        init_results: Vec<Result<(), String>>,
        read_results: Vec<Result<SensorReading, String>>,
    ) -> TargetRuntime {
        TargetRuntime {
            address,
            driver: Arc::new(MockDriver::new(
                "MOCK",
                detect_results,
                init_results,
                read_results,
            )),
            key_suffix: suffix.to_string(),
        }
    }

    use crate::{PollingAdapterConfig, SensorTargetConfig};
    use std::time::Duration;

    fn make_config(targets: Vec<SensorTargetConfig>) -> PollingAdapterConfig {
        PollingAdapterConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 50,
            targets,
        }
    }

    fn make_sensor_target(
        address: u8,
        key_suffix: Option<String>,
        detect_results: Vec<Result<SensorIdentity, String>>,
        init_results: Vec<Result<(), String>>,
        read_results: Vec<Result<SensorReading, String>>,
    ) -> SensorTargetConfig {
        SensorTargetConfig {
            address,
            driver: Arc::new(MockDriver::new(
                "MOCK",
                detect_results,
                init_results,
                read_results,
            )),
            key_suffix,
        }
    }

    // ── async polling_loop tests ────────────────────────

    #[tokio::test]
    async fn shutdown_command_stops_loop() {
        let config = make_config(vec![]);
        let (event_tx, _event_rx) = mpsc::channel(16);
        let (command_tx, command_rx) = mpsc::channel(16);

        let handle = tokio::spawn(super::polling_loop(
            AdapterId::new("test"),
            None,
            config,
            event_tx,
            command_rx,
        ));

        command_tx.send(AdapterCommand::Shutdown).await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("timeout")
            .expect("task panicked");
    }

    #[tokio::test]
    async fn command_channel_drop_stops_loop() {
        let config = make_config(vec![]);
        let (event_tx, _event_rx) = mpsc::channel(16);
        let (command_tx, command_rx) = mpsc::channel(16);

        let handle = tokio::spawn(super::polling_loop(
            AdapterId::new("test"),
            None,
            config,
            event_tx,
            command_rx,
        ));

        drop(command_tx);

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("timeout")
            .expect("task panicked");
    }

    #[tokio::test]
    async fn event_channel_close_detected_with_events() {
        let config = make_config(vec![]);
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, command_rx) = mpsc::channel(16);

        let handle = tokio::spawn(super::polling_loop(
            AdapterId::new("test"),
            None,
            config,
            event_tx,
            command_rx,
        ));

        drop(event_rx);

        // Send a DeviceCommand to trigger a send on the closed event channel.
        let _ = command_tx
            .send(AdapterCommand::DeviceCommand(
                iotkit_core_types::DeviceCommand {
                    device_key: DeviceKey::new("test"),
                    payload: iotkit_core_types::DeviceCommandPayload::QueryConfig,
                },
            ))
            .await;

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("timeout")
            .expect("task panicked");
    }

    #[tokio::test]
    async fn event_channel_close_detected_without_events() {
        let config = make_config(vec![]);
        let (event_tx, event_rx) = mpsc::channel(16);
        let (_command_tx, command_rx) = mpsc::channel(16);

        let handle = tokio::spawn(super::polling_loop(
            AdapterId::new("test"),
            None,
            config,
            event_tx,
            command_rx,
        ));

        drop(event_rx);

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("timeout")
            .expect("task panicked");
    }

    #[tokio::test]
    async fn device_command_rejection_preserves_device_key() {
        let config = make_config(vec![]);
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let (command_tx, command_rx) = mpsc::channel(16);

        let handle = tokio::spawn(super::polling_loop(
            AdapterId::new("test"),
            None,
            config,
            event_tx,
            command_rx,
        ));

        command_tx
            .send(AdapterCommand::DeviceCommand(
                iotkit_core_types::DeviceCommand {
                    device_key: DeviceKey::new("i2c:0x40:temperature"),
                    payload: iotkit_core_types::DeviceCommandPayload::QueryConfig,
                },
            ))
            .await
            .unwrap();

        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        match event {
            AdapterEvent::AdapterError { device_key, error } => {
                assert_eq!(
                    device_key.as_ref().unwrap().as_str(),
                    "i2c:0x40:temperature"
                );
                assert!(error.contains("unsupported"));
            }
            other => panic!("expected AdapterError, got {other:?}"),
        }

        command_tx.send(AdapterCommand::Shutdown).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("timeout")
            .expect("task panicked");
    }

    #[tokio::test]
    async fn mock_probe_discovery_then_read() {
        let identity = make_identity();
        let reading = make_reading();
        let target = make_sensor_target(
            0x40,
            Some("temperature".into()),
            vec![Ok(identity.clone())],
            vec![Ok(())],
            vec![Ok(reading.clone())],
        );
        let config = make_config(vec![target]);

        let (event_tx, mut event_rx) = mpsc::channel(16);
        let (command_tx, command_rx) = mpsc::channel(16);

        let handle = tokio::spawn(super::polling_loop(
            AdapterId::new("test"),
            None,
            config,
            event_tx,
            command_rx,
        ));

        // First event: DeviceDiscovered from startup probe.
        let ev1 = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        assert!(
            matches!(ev1, AdapterEvent::DeviceDiscovered { .. }),
            "expected DeviceDiscovered, got {ev1:?}"
        );

        // Second event: SensorData from first tick read.
        let ev2 = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        assert!(
            matches!(ev2, AdapterEvent::SensorData { .. }),
            "expected SensorData, got {ev2:?}"
        );

        command_tx.send(AdapterCommand::Shutdown).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("timeout")
            .expect("task panicked");
    }

    #[tokio::test]
    async fn sensor_data_is_submitted_to_ingest_client() {
        let identity = make_identity();
        let reading = make_reading();
        let target = make_sensor_target(
            0x40,
            Some("temperature".into()),
            vec![Ok(identity.clone())],
            vec![Ok(())],
            vec![Ok(reading.clone())],
        );
        let config = make_config(vec![target]);

        let (event_tx, mut event_rx) = mpsc::channel(16);
        let (command_tx, command_rx) = mpsc::channel(16);
        let (ingest, mut ingest_rx) = iotkit_ingest_client::channel_for_test(4);

        let handle = tokio::spawn(super::polling_loop(
            iotkit_core_types::AdapterId::new("rpi-local:default"),
            Some(ingest),
            config,
            event_tx,
            command_rx,
        ));

        let _ = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        assert!(
            matches!(event, AdapterEvent::SensorData { .. }),
            "expected SensorData, got {event:?}"
        );

        let envelope = tokio::time::timeout(Duration::from_secs(2), ingest_rx.recv())
            .await
            .expect("timeout waiting for ingest envelope")
            .expect("ingest channel closed");
        assert_eq!(envelope.source, "rpi-local:default");
        assert_eq!(envelope.items.len(), 1);
        assert_eq!(
            envelope.items[0].subject_hint.as_deref(),
            Some("rpi-local:default:i2c:0x40")
        );
        assert_eq!(envelope.items[0].measurement_key, "temperature_c");
        assert_eq!(envelope.items[0].values, reading.values);

        command_tx.send(AdapterCommand::Shutdown).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("timeout")
            .expect("task panicked");
    }

    #[tokio::test]
    async fn empty_targets_no_startup_error() {
        let config = make_config(vec![]);
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let (command_tx, command_rx) = mpsc::channel(16);

        let handle = tokio::spawn(super::polling_loop(
            AdapterId::new("test"),
            None,
            config,
            event_tx,
            command_rx,
        ));

        command_tx.send(AdapterCommand::Shutdown).await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("timeout")
            .expect("task panicked");

        // No events should have been emitted.
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn all_targets_fail_startup_emits_immediate_error() {
        let target = make_sensor_target(
            0x40,
            Some("temperature".into()),
            vec![Err("NACK".into())],
            vec![],
            vec![],
        );
        let config = make_config(vec![target]);

        let (event_tx, mut event_rx) = mpsc::channel(16);
        let (command_tx, command_rx) = mpsc::channel(16);

        let handle = tokio::spawn(super::polling_loop(
            AdapterId::new("test"),
            None,
            config,
            event_tx,
            command_rx,
        ));

        // Should receive an immediate AdapterError for all-targets-failed.
        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        match event {
            AdapterEvent::AdapterError { device_key, error } => {
                assert!(device_key.is_none());
                assert!(
                    error.contains("all targets failed startup probe"),
                    "unexpected error: {error}"
                );
            }
            other => panic!("expected AdapterError, got {other:?}"),
        }

        command_tx.send(AdapterCommand::Shutdown).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("timeout")
            .expect("task panicked");
    }

    // ── poll_cycle tests ─────────────────────────────────

    #[test]
    fn pending_target_detect_init_success() {
        let identity = make_identity();
        let targets = vec![make_mock_target(
            0x40,
            "temperature",
            vec![Ok(identity.clone())],
            vec![Ok(())],
            vec![],
        )];
        let states = vec![TargetState::new_pending()];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::Discovered {
                target_index,
                key,
                identity: id,
            } => {
                assert_eq!(*target_index, 0);
                assert_eq!(key.as_str(), "i2c:0x40:temperature");
                assert_eq!(id.ic_part_number, identity.ic_part_number);
            }
            _ => panic!("expected Discovered"),
        }
    }

    #[test]
    fn pending_target_detect_failure() {
        let targets = vec![make_mock_target(
            0x40,
            "temperature",
            vec![Err("NACK".into())],
            vec![],
            vec![],
        )];
        let states = vec![TargetState::new_pending()];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::DetectFailed {
                target_index,
                message,
                ..
            } => {
                assert_eq!(*target_index, 0);
                assert_eq!(message, "NACK");
            }
            _ => panic!("expected DetectFailed"),
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
            PollOutcome::Reading {
                key: k, reading: r, ..
            } => {
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
            PollOutcome::ReadError {
                target_index,
                key: k,
                message,
                ..
            } => {
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
        fn detect(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
            unimplemented!("not called by apply_outcomes")
        }
        fn init(&self, _bus_path: &str, _address: u8) -> Result<(), String> {
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
        SensorReading::new(
            SensorType::Temperature,
            vec![25.0],
            vec!["temp_c".to_string()],
        )
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
            observed_at: std::time::SystemTime::now(),
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AdapterEvent::SensorData {
                device_key,
                reading,
                rssi,
                battery_pct,
                ..
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
            is_panic: false,
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AdapterEvent::AdapterError { device_key, error } => {
                assert_eq!(
                    device_key.as_ref().unwrap().as_str(),
                    "i2c:0x40:temperature"
                );
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
            is_panic: false,
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
                consecutive_detect_failures,
                escalation_emitted,
            } => {
                assert_eq!(*consecutive_detect_failures, 0);
                assert!(!*escalation_emitted);
            }
            _ => panic!("expected Pending state"),
        }
    }

    #[test]
    fn detect_failure_below_threshold_emits_no_event() {
        let targets = vec![make_target(0x40, "temperature")];
        let mut states = vec![TargetState::new_pending()];

        let outcomes = vec![PollOutcome::DetectFailed {
            target_index: 0,
            message: "NACK".into(),
            is_panic: false,
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        assert!(events.is_empty());

        match &states[0] {
            TargetState::Pending {
                consecutive_detect_failures,
                escalation_emitted,
            } => {
                assert_eq!(*consecutive_detect_failures, 1);
                assert!(!*escalation_emitted);
            }
            _ => panic!("expected Pending state"),
        }
    }

    #[test]
    fn detect_failure_at_threshold_emits_one_error() {
        let targets = vec![make_target(0x40, "temperature")];
        let mut states = vec![TargetState::Pending {
            consecutive_detect_failures: MAX_DETECT_FAILURES - 1,
            escalation_emitted: false,
        }];

        let outcomes = vec![PollOutcome::DetectFailed {
            target_index: 0,
            message: "device not found".into(),
            is_panic: false,
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        assert_eq!(events.len(), 1);
        match &events[0] {
            AdapterEvent::AdapterError { device_key, error } => {
                assert!(device_key.is_none());
                assert!(error.contains("target 0x40 detect failed 10 consecutive times"));
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

        // A second detect failure should NOT emit another event.
        let outcomes2 = vec![PollOutcome::DetectFailed {
            target_index: 0,
            message: "device not found".into(),
            is_panic: false,
        }];
        let events2 = apply_outcomes(outcomes2, &mut states, &targets);
        assert!(events2.is_empty());
    }

    #[test]
    fn detect_init_success_after_escalation_resets() {
        let targets = vec![make_target(0x40, "temperature")];
        let mut states = vec![TargetState::Pending {
            consecutive_detect_failures: 15,
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
                observed_at: std::time::SystemTime::now(),
            },
            PollOutcome::DetectFailed {
                target_index: 1,
                message: "NACK".into(),
                is_panic: false,
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

        // Target B: still Pending with incremented detect failures.
        match &states[1] {
            TargetState::Pending {
                consecutive_detect_failures,
                ..
            } => assert_eq!(*consecutive_detect_failures, 1),
            _ => panic!("expected Pending state for target B"),
        }
    }

    // ── Panic isolation tests ────────────────────────────

    /// A driver that panics on detect, init, and/or read.
    struct PanickingDriver {
        panic_on_detect: bool,
        panic_on_init: bool,
        panic_on_read: bool,
    }

    impl SensorDriver for PanickingDriver {
        fn detect(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
            if self.panic_on_detect {
                panic!("intentional detect panic");
            }
            Ok(make_identity())
        }
        fn init(&self, _bus_path: &str, _address: u8) -> Result<(), String> {
            if self.panic_on_init {
                panic!("intentional init panic");
            }
            Ok(())
        }
        fn read(&self, _bus_path: &str, _address: u8) -> Result<SensorReading, String> {
            if self.panic_on_read {
                panic!("intentional read panic");
            }
            Ok(make_reading())
        }
        fn ic_name(&self) -> &'static str {
            "PANICKER"
        }
    }

    #[test]
    fn detect_panic_becomes_detect_failed() {
        let targets = vec![TargetRuntime {
            address: 0x40,
            driver: Arc::new(PanickingDriver {
                panic_on_detect: true,
                panic_on_init: false,
                panic_on_read: false,
            }),
            key_suffix: "panic".to_string(),
        }];
        let states = vec![TargetState::new_pending()];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::DetectFailed {
                target_index,
                message,
                is_panic,
            } => {
                assert_eq!(*target_index, 0);
                assert!(
                    message.contains("panic"),
                    "expected panic in message: {message}"
                );
                assert!(
                    message.contains("0x40"),
                    "expected address in message: {message}"
                );
                assert!(*is_panic, "expected is_panic=true for panicking driver");
            }
            other => panic!("expected DetectFailed, got {other:?}"),
        }
    }

    #[test]
    fn read_panic_becomes_read_error() {
        let targets = vec![TargetRuntime {
            address: 0x44,
            driver: Arc::new(PanickingDriver {
                panic_on_detect: false,
                panic_on_init: false,
                panic_on_read: true,
            }),
            key_suffix: "panic".to_string(),
        }];
        let states = vec![TargetState::Active {
            key: DeviceKey::new("i2c-0x44-panic"),
            consecutive_read_failures: 0,
        }];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::ReadError {
                target_index,
                message,
                ..
            } => {
                assert_eq!(*target_index, 0);
                assert!(
                    message.contains("panic"),
                    "expected panic in message: {message}"
                );
                assert!(
                    message.contains("0x44"),
                    "expected address in message: {message}"
                );
            }
            other => panic!("expected ReadError, got {other:?}"),
        }
    }

    #[test]
    fn panic_in_one_target_does_not_affect_sibling() {
        let targets = vec![
            TargetRuntime {
                address: 0x40,
                driver: Arc::new(PanickingDriver {
                    panic_on_detect: true,
                    panic_on_init: false,
                    panic_on_read: false,
                }),
                key_suffix: "panicker".to_string(),
            },
            TargetRuntime {
                address: 0x44,
                driver: Arc::new(PanickingDriver {
                    panic_on_detect: false,
                    panic_on_init: false,
                    panic_on_read: false,
                }),
                key_suffix: "healthy".to_string(),
            },
        ];
        let states = vec![TargetState::new_pending(), TargetState::new_pending()];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 2);
        // First target panicked → DetectFailed
        assert!(matches!(
            &outcomes[0],
            PollOutcome::DetectFailed {
                target_index: 0,
                ..
            }
        ));
        // Second target succeeded → Discovered (detect+init same cycle)
        assert!(matches!(
            &outcomes[1],
            PollOutcome::Discovered {
                target_index: 1,
                ..
            }
        ));
    }

    #[test]
    fn detect_panic_emits_immediate_adapter_error() {
        // A panic during detect should emit AdapterError on the FIRST failure,
        // not wait for MAX_DETECT_FAILURES threshold.
        let targets = vec![make_target(0x40, "temperature")];
        let mut states = vec![TargetState::new_pending()];

        let outcomes = vec![PollOutcome::DetectFailed {
            target_index: 0,
            message: "driver panic during detect 0x40@/dev/i2c-1: boom".into(),
            is_panic: true,
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        // Should emit AdapterError immediately despite consecutive_detect_failures == 1
        assert_eq!(events.len(), 1);
        match &events[0] {
            AdapterEvent::AdapterError { error, .. } => {
                assert!(
                    error.contains("detect failed"),
                    "expected detect failed in error: {error}"
                );
            }
            other => panic!("expected AdapterError, got {other:?}"),
        }
    }

    #[test]
    fn detect_panic_does_not_consume_escalation() {
        // After a panic detect, subsequent non-panic detect failures should
        // still escalate at the threshold (escalation_emitted must NOT be set by panic path).
        let targets = vec![make_target(0x40, "temperature")];
        let mut states = vec![TargetState::new_pending()];

        // 1. Panic detect — should emit immediately
        let outcomes = vec![PollOutcome::DetectFailed {
            target_index: 0,
            message: "driver panic".into(),
            is_panic: true,
        }];
        let events = apply_outcomes(outcomes, &mut states, &targets);
        assert_eq!(events.len(), 1, "panic should emit immediate AdapterError");

        // 2. Simulate MAX_DETECT_FAILURES - 1 more normal failures (already at 1 from panic)
        for _ in 0..(MAX_DETECT_FAILURES - 2) {
            let outcomes = vec![PollOutcome::DetectFailed {
                target_index: 0,
                message: "NACK".into(),
                is_panic: false,
            }];
            let events = apply_outcomes(outcomes, &mut states, &targets);
            assert!(events.is_empty(), "below threshold, no event expected");
        }

        // 3. One more normal failure should hit threshold and emit
        let outcomes = vec![PollOutcome::DetectFailed {
            target_index: 0,
            message: "NACK".into(),
            is_panic: false,
        }];
        let events = apply_outcomes(outcomes, &mut states, &targets);
        assert_eq!(
            events.len(),
            1,
            "threshold reached, should emit AdapterError"
        );
        match &events[0] {
            AdapterEvent::AdapterError { error, .. } => {
                assert!(
                    error.contains("consecutive times"),
                    "expected threshold error, got: {error}"
                );
            }
            other => panic!("expected AdapterError, got {other:?}"),
        }
    }

    // ── Init failure tests ──────────────────────────────

    /// A driver that succeeds detect but fails init.
    struct DetectOnlyDriver;

    impl SensorDriver for DetectOnlyDriver {
        fn detect(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
            Ok(make_identity())
        }
        fn init(&self, _bus_path: &str, address: u8) -> Result<(), String> {
            Err(format!(
                "init failed for 0x{:02x}: config write error",
                address
            ))
        }
        fn read(&self, _bus_path: &str, _address: u8) -> Result<SensorReading, String> {
            unimplemented!("should not be called")
        }
        fn ic_name(&self) -> &'static str {
            "DETECT_ONLY"
        }
    }

    #[test]
    fn detect_success_init_failure_enters_detected_state() {
        let targets = vec![TargetRuntime {
            address: 0x40,
            driver: Arc::new(DetectOnlyDriver),
            key_suffix: "temperature".to_string(),
        }];
        let states = vec![TargetState::new_pending()];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::InitFailed {
                target_index,
                identity,
                message,
                is_panic,
            } => {
                assert_eq!(*target_index, 0);
                assert_eq!(identity.ic_part_number, "STUB");
                assert!(
                    message.contains("init failed"),
                    "expected init error: {message}"
                );
                assert!(!*is_panic);
            }
            other => panic!("expected InitFailed, got {other:?}"),
        }

        // Apply outcomes
        let mut states = vec![TargetState::new_pending()];
        let events = apply_outcomes(outcomes, &mut states, &targets);

        // State should be Detected with 1 init failure
        match &states[0] {
            TargetState::Detected {
                identity,
                consecutive_init_failures,
            } => {
                assert_eq!(identity.ic_part_number, "STUB");
                assert_eq!(*consecutive_init_failures, 1);
            }
            other => panic!("expected Detected state, got: {other:?}"),
        }

        // Should have emitted AdapterError
        assert_eq!(events.len(), 1);
        match &events[0] {
            AdapterEvent::AdapterError { device_key, error } => {
                assert!(device_key.is_none());
                assert!(
                    error.contains("init failed (1/5)"),
                    "unexpected error: {error}"
                );
            }
            other => panic!("expected AdapterError, got {other:?}"),
        }
    }

    #[test]
    fn detected_state_retries_init_and_succeeds() {
        let identity = make_identity();
        let targets = vec![make_mock_target(
            0x40,
            "temperature",
            vec![], // detect not called for Detected state
            vec![Ok(())],
            vec![],
        )];
        let states = vec![TargetState::Detected {
            identity: identity.clone(),
            consecutive_init_failures: 1,
        }];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::Discovered {
                target_index,
                key,
                identity: id,
            } => {
                assert_eq!(*target_index, 0);
                assert_eq!(key.as_str(), "i2c:0x40:temperature");
                assert_eq!(id.ic_part_number, identity.ic_part_number);
            }
            other => panic!("expected Discovered, got {other:?}"),
        }

        // Apply outcomes
        let mut states = vec![TargetState::Detected {
            identity: identity.clone(),
            consecutive_init_failures: 1,
        }];
        let events = apply_outcomes(outcomes, &mut states, &targets);

        // State should be Active
        match &states[0] {
            TargetState::Active {
                key,
                consecutive_read_failures,
            } => {
                assert_eq!(key.as_str(), "i2c:0x40:temperature");
                assert_eq!(*consecutive_read_failures, 0);
            }
            other => panic!("expected Active state, got: {other:?}"),
        }

        // Should emit DeviceDiscovered
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AdapterEvent::DeviceDiscovered { .. }));
    }

    #[test]
    fn max_init_failures_returns_to_pending() {
        let targets = vec![make_target(0x40, "temperature")];
        let identity = make_identity();
        let mut states = vec![TargetState::Detected {
            identity: identity.clone(),
            consecutive_init_failures: MAX_INIT_FAILURES - 1,
        }];

        let outcomes = vec![PollOutcome::InitFailed {
            target_index: 0,
            identity,
            message: "config write error".into(),
            is_panic: false,
        }];

        let events = apply_outcomes(outcomes, &mut states, &targets);

        // State should return to Pending
        match &states[0] {
            TargetState::Pending {
                consecutive_detect_failures,
                escalation_emitted,
            } => {
                assert_eq!(*consecutive_detect_failures, 0);
                assert!(!*escalation_emitted);
            }
            other => panic!("expected Pending state, got: {other:?}"),
        }

        // Should emit AdapterError
        assert_eq!(events.len(), 1);
        match &events[0] {
            AdapterEvent::AdapterError { error, .. } => {
                assert!(
                    error.contains("init failed (5/5)"),
                    "unexpected error: {error}"
                );
            }
            other => panic!("expected AdapterError, got {other:?}"),
        }
    }

    #[test]
    fn same_cycle_detect_init_success_emits_discovered() {
        let identity = make_identity();
        let targets = vec![make_mock_target(
            0x40,
            "temperature",
            vec![Ok(identity.clone())],
            vec![Ok(())],
            vec![],
        )];
        let states = vec![TargetState::new_pending()];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        // Exactly 1 outcome: Discovered (not two separate outcomes)
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(&outcomes[0], PollOutcome::Discovered { .. }));

        // Apply and verify single DeviceDiscovered event
        let mut states = vec![TargetState::new_pending()];
        let events = apply_outcomes(outcomes, &mut states, &targets);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AdapterEvent::DeviceDiscovered { .. }));
    }

    #[test]
    fn init_panic_becomes_init_failed() {
        let identity = make_identity();
        let targets = vec![TargetRuntime {
            address: 0x40,
            driver: Arc::new(PanickingDriver {
                panic_on_detect: false,
                panic_on_init: true,
                panic_on_read: false,
            }),
            key_suffix: "temperature".to_string(),
        }];
        let states = vec![TargetState::Detected {
            identity: identity.clone(),
            consecutive_init_failures: 0,
        }];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::InitFailed {
                target_index,
                is_panic,
                message,
                ..
            } => {
                assert_eq!(*target_index, 0);
                assert!(*is_panic, "expected is_panic=true");
                assert!(
                    message.contains("panic"),
                    "expected panic in message: {message}"
                );
            }
            other => panic!("expected InitFailed, got {other:?}"),
        }

        // Apply outcomes
        let mut states = vec![TargetState::Detected {
            identity,
            consecutive_init_failures: 0,
        }];
        let events = apply_outcomes(outcomes, &mut states, &targets);

        // State should be Detected with incremented failure count
        match &states[0] {
            TargetState::Detected {
                consecutive_init_failures,
                ..
            } => {
                assert_eq!(*consecutive_init_failures, 1);
            }
            other => panic!("expected Detected state, got: {other:?}"),
        }

        // Should emit AdapterError
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AdapterEvent::AdapterError { .. }));
    }

    #[test]
    fn init_panic_from_pending_same_cycle() {
        // detect succeeds, init panics in same cycle from Pending
        let targets = vec![TargetRuntime {
            address: 0x40,
            driver: Arc::new(PanickingDriver {
                panic_on_detect: false,
                panic_on_init: true,
                panic_on_read: false,
            }),
            key_suffix: "temperature".to_string(),
        }];
        let states = vec![TargetState::new_pending()];

        let outcomes = poll_cycle(&targets, &states, "/dev/i2c-1");

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PollOutcome::InitFailed {
                target_index,
                is_panic,
                ..
            } => {
                assert_eq!(*target_index, 0);
                assert!(*is_panic, "expected is_panic=true");
            }
            other => panic!("expected InitFailed, got {other:?}"),
        }

        // Apply outcomes
        let mut states = vec![TargetState::new_pending()];
        let events = apply_outcomes(outcomes, &mut states, &targets);

        // State should be Detected (not Pending, not Active)
        match &states[0] {
            TargetState::Detected {
                consecutive_init_failures,
                ..
            } => {
                assert_eq!(*consecutive_init_failures, 1);
            }
            other => panic!("expected Detected state, got: {other:?}"),
        }

        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AdapterEvent::AdapterError { .. }));
    }
}
