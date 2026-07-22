use std::panic;
use std::sync::Arc;
use tokio::sync::mpsc;

use iotkit_core_types::{AdapterId, DeviceKey, SensorIdentity, SensorReading};

use crate::{
    PollingAdapterConfig, PollingCommand as AdapterCommand, PollingEvent as AdapterEvent,
    SensorDriver,
};

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
    _adapter_id: AdapterId,
    _legacy_ingest_removed: Option<()>,
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
                    if event_tx.send(event).await.is_err() {
                        tracing::warn!("event channel closed during poll cycle");
                        return;
                    }
                }
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
#[path = "../tests/unit/polling_loop_tests.rs"]
mod tests;
