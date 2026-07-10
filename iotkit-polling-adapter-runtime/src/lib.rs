//! iotkit-polling-adapter-runtime: shared scaffolding for I2C-bus polling sensor adapters.

mod ingest_map;
mod polling_loop;

use std::collections::HashSet;
use std::sync::Arc;

use tokio::runtime::Handle;
use tokio::sync::mpsc;

use iotkit_core_supervision::{AdapterCommand, AdapterEvent};
use iotkit_core_types::{AdapterId, SensorIdentity, SensorReading};

// ── SensorDriver trait ────────────────────────────────────

/// Trait implemented by each IC driver (e.g. SDP810, VL53L1X).
///
/// # Panic safety
///
/// The polling runtime catches panics from `detect()`, `init()`, and `read()`
/// per-target using `catch_unwind`. After a panic, the same driver instance will
/// be called again on subsequent poll cycles. Implementations must therefore be
/// safe to reuse after a panic — avoid interior mutable state that could be left
/// in an inconsistent state. Stateless drivers (the common case) satisfy this
/// trivially.
pub trait SensorDriver: Send + Sync {
    /// Read-only detection. Must NOT write to hardware.
    /// Reads device ID registers and returns identity on match.
    ///
    /// Error strings **must** include the bus path and address for field debugging.
    fn detect(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String>;

    /// Initialize hardware by writing config registers.
    /// Called only after detect() succeeds. Must be idempotent.
    ///
    /// Error strings **must** include the bus path and address for field debugging.
    fn init(&self, bus_path: &str, address: u8) -> Result<(), String>;

    /// Read the sensor at the given I2C address. Returns a reading on success.
    ///
    /// Error strings **must** include the bus path and address for field debugging.
    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String>;

    /// Return the IC part name (e.g. "opt3001", "mcp9600").
    fn ic_name(&self) -> &'static str;
    fn validate(&self, poll_interval_ms: u64) -> Result<(), String> {
        let _ = poll_interval_ms;
        Ok(())
    }
}

// ── Config types ──────────────────────────────────────────

/// Configuration for a polling adapter instance.
pub struct PollingAdapterConfig {
    pub bus_path: String,
    pub poll_interval_ms: u64,
    pub targets: Vec<SensorTargetConfig>,
}

/// One sensor target on the bus.
///
/// Each target should own a **distinct** driver instance. Do not share a single
/// `Arc<dyn SensorDriver>` across multiple targets — a panic in one target's
/// driver could leave shared interior state inconsistent, weakening per-target
/// isolation.
pub struct SensorTargetConfig {
    pub address: u8,
    pub driver: Arc<dyn SensorDriver>,
    pub key_suffix: Option<String>,
}

// ── AdapterHandle ─────────────────────────────────────────

/// Handle returned by [`start`]. Owns the background task and channels.
pub struct AdapterHandle {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for AdapterHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdapterHandle")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl AdapterHandle {
    /// Gracefully shut down the adapter.
    pub async fn shutdown(&mut self) {
        self.event_rx.close();
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }
    }
}

/// Parts returned by [`AdapterHandle::into_parts`].
pub struct AdapterParts {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub shutdown: ShutdownHandle,
}

/// Opaque handle for shutting down a polling adapter.
///
/// Does NOT close the event receiver — that is the caller's responsibility
/// (e.g. by dropping the `event_rx` or the stream wrapping it).
/// ShutdownHandle only sends `Shutdown` and awaits the background task.
pub struct ShutdownHandle {
    command_tx: mpsc::Sender<AdapterCommand>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ShutdownHandle {
    pub async fn shutdown(mut self) -> Result<(), String> {
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.task_handle.take() {
            handle
                .await
                .map_err(|e| format!("polling task panicked: {e}"))?;
        }
        Ok(())
    }
}

impl AdapterHandle {
    /// Decompose this handle into parts for use with an adapter host.
    ///
    /// The existing [`AdapterHandle::shutdown`] method remains available
    /// for direct use — `into_parts` is an additive API.
    pub fn into_parts(self) -> AdapterParts {
        AdapterParts {
            id: self.id,
            event_rx: self.event_rx,
            shutdown: ShutdownHandle {
                command_tx: self.command_tx,
                task_handle: self.task_handle,
            },
        }
    }
}

// ── validate_config ───────────────────────────────────────

/// Validate a [`PollingAdapterConfig`], returning `Err` on the first problem.
pub fn validate_config(config: &PollingAdapterConfig) -> Result<(), String> {
    if config.bus_path.is_empty() {
        return Err("bus_path must not be empty".into());
    }
    if config.poll_interval_ms == 0 {
        return Err("poll_interval_ms must be > 0".into());
    }
    if config.targets.is_empty() {
        return Err("targets must not be empty".into());
    }

    let mut seen = HashSet::new();
    for target in &config.targets {
        if !(0x08..=0x77).contains(&target.address) {
            return Err(format!(
                "address 0x{:02X} is outside valid I2C range 0x08..=0x77",
                target.address,
            ));
        }
        if !seen.insert(target.address) {
            return Err(format!("duplicate address 0x{:02X}", target.address));
        }
        target.driver.validate(config.poll_interval_ms)?;
    }

    Ok(())
}

// ── start ─────────────────────────────────────────────────

/// Start the adapter. Must be called from within a Tokio runtime.
pub fn start(
    id: AdapterId,
    config: PollingAdapterConfig,
    ingest: Option<iotkit_ingest_client::IngestClient>,
) -> Result<AdapterHandle, std::io::Error> {
    validate_config(&config).map_err(std::io::Error::other)?;

    // Ensure we are inside a Tokio runtime.
    let _handle = Handle::try_current().map_err(std::io::Error::other)?;

    // Fail fast: verify the bus path is accessible.
    std::fs::File::open(&config.bus_path).map_err(|e| {
        std::io::Error::other(format!("cannot open bus_path '{}': {}", config.bus_path, e))
    })?;

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(256);
    let (command_tx, command_rx) = mpsc::channel::<AdapterCommand>(32);

    let task_handle = tokio::spawn(polling_loop::polling_loop(
        id.clone(),
        ingest,
        config,
        event_tx,
        command_rx,
    ));

    Ok(AdapterHandle {
        id,
        event_rx,
        command_tx,
        task_handle: Some(task_handle),
    })
}

// ── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::{
        ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading, SensorType,
    };
    use std::collections::BTreeMap;

    /// Minimal no-op driver for tests.
    struct StubDriver;

    impl SensorDriver for StubDriver {
        fn detect(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
            Ok(SensorIdentity {
                manufacturer: "Test".into(),
                ic_part_number: "STUB".into(),
                sensor_type: SensorType::Temperature,
                connection: ConnectionInfo {
                    kind: ConnectionKind::I2c,
                    parameters: BTreeMap::new(),
                },
            })
        }
        fn init(&self, _bus_path: &str, _address: u8) -> Result<(), String> {
            Ok(())
        }
        fn read(&self, _bus_path: &str, _address: u8) -> Result<SensorReading, String> {
            Ok(SensorReading::empty(SensorType::Temperature))
        }
        fn ic_name(&self) -> &'static str {
            "STUB"
        }
    }

    /// Driver that rejects poll intervals below a threshold.
    struct StrictDriver {
        min_interval_ms: u64,
    }

    impl SensorDriver for StrictDriver {
        fn detect(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
            Ok(SensorIdentity {
                manufacturer: "Test".into(),
                ic_part_number: "STRICT".into(),
                sensor_type: SensorType::Temperature,
                connection: ConnectionInfo {
                    kind: ConnectionKind::I2c,
                    parameters: BTreeMap::new(),
                },
            })
        }
        fn init(&self, _bus_path: &str, _address: u8) -> Result<(), String> {
            Ok(())
        }
        fn read(&self, _bus_path: &str, _address: u8) -> Result<SensorReading, String> {
            Ok(SensorReading::empty(SensorType::Temperature))
        }
        fn ic_name(&self) -> &'static str {
            "STRICT"
        }
        fn validate(&self, poll_interval_ms: u64) -> Result<(), String> {
            if poll_interval_ms < self.min_interval_ms {
                return Err(format!(
                    "poll_interval_ms {} too short, minimum is {}",
                    poll_interval_ms, self.min_interval_ms,
                ));
            }
            Ok(())
        }
    }

    fn stub_config() -> PollingAdapterConfig {
        PollingAdapterConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 1000,
            targets: vec![SensorTargetConfig {
                address: 0x40,
                driver: Arc::new(StubDriver),
                key_suffix: None,
            }],
        }
    }

    #[test]
    fn valid_config_passes() {
        assert!(validate_config(&stub_config()).is_ok());
    }

    #[test]
    fn empty_bus_path_rejected() {
        let mut cfg = stub_config();
        cfg.bus_path = String::new();
        let err = validate_config(&cfg).unwrap_err();
        assert!(err.contains("bus_path"), "unexpected error: {err}");
    }

    #[test]
    fn empty_targets_rejected() {
        let mut cfg = stub_config();
        cfg.targets.clear();
        let err = validate_config(&cfg).unwrap_err();
        assert!(err.contains("targets"), "unexpected error: {err}");
    }

    #[test]
    fn zero_poll_interval_rejected() {
        let mut cfg = stub_config();
        cfg.poll_interval_ms = 0;
        let err = validate_config(&cfg).unwrap_err();
        assert!(err.contains("poll_interval_ms"), "unexpected error: {err}");
    }

    #[test]
    fn duplicate_address_rejected() {
        let mut cfg = stub_config();
        cfg.targets.push(SensorTargetConfig {
            address: 0x40,
            driver: Arc::new(StubDriver),
            key_suffix: Some("second".into()),
        });
        let err = validate_config(&cfg).unwrap_err();
        assert!(err.contains("duplicate"), "unexpected error: {err}");
    }

    #[test]
    fn address_out_of_range_rejected() {
        for bad_addr in [0x00, 0x07, 0x78, 0xFF] {
            let cfg = PollingAdapterConfig {
                bus_path: "/dev/i2c-1".into(),
                poll_interval_ms: 1000,
                targets: vec![SensorTargetConfig {
                    address: bad_addr,
                    driver: Arc::new(StubDriver),
                    key_suffix: None,
                }],
            };
            let err = validate_config(&cfg).unwrap_err();
            assert!(
                err.contains("outside valid I2C range"),
                "addr 0x{bad_addr:02X}: unexpected error: {err}",
            );
        }
    }

    #[test]
    fn driver_validate_called() {
        let cfg = PollingAdapterConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 50,
            targets: vec![SensorTargetConfig {
                address: 0x40,
                driver: Arc::new(StrictDriver {
                    min_interval_ms: 100,
                }),
                key_suffix: None,
            }],
        };
        let err = validate_config(&cfg).unwrap_err();
        assert!(err.contains("too short"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn into_parts_preserves_id_and_channels() {
        use iotkit_core_types::SensorType;

        let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(1);
        let (command_tx, mut command_rx) = mpsc::channel::<AdapterCommand>(1);
        let handle = AdapterHandle {
            id: AdapterId::new("test:into-parts"),
            event_rx,
            command_tx,
            task_handle: None,
        };
        let parts = handle.into_parts();

        assert_eq!(parts.id.as_str(), "test:into-parts");

        let mut event_rx = parts.event_rx;
        event_tx
            .send(AdapterEvent::SensorData {
                device_key: iotkit_core_types::DeviceKey::new("test:0"),
                reading: SensorReading::empty(SensorType::Temperature),
                rssi: None,
                battery_pct: None,
                ingested_at: std::time::SystemTime::now(),
            })
            .await
            .unwrap();
        let received = event_rx.recv().await;
        assert!(received.is_some(), "event_rx should receive the sent event");

        parts
            .shutdown
            .shutdown()
            .await
            .expect("shutdown should succeed");
        let cmd = command_rx.recv().await;
        assert!(
            matches!(cmd, Some(AdapterCommand::Shutdown)),
            "shutdown should send Shutdown command"
        );
    }

    #[test]
    fn start_without_runtime_returns_error() {
        // No Tokio runtime active on this thread.
        let cfg = stub_config();
        let err = start(AdapterId::new("test"), cfg, None).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no reactor") || msg.contains("runtime"),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    async fn start_with_bad_bus_path() {
        let cfg = PollingAdapterConfig {
            bus_path: "/tmp/iotkit-nonexistent-bus-path-test".into(),
            poll_interval_ms: 1000,
            targets: vec![SensorTargetConfig {
                address: 0x40,
                driver: Arc::new(StubDriver),
                key_suffix: None,
            }],
        };
        let err = start(AdapterId::new("test"), cfg, None).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("cannot open bus_path"),
            "unexpected error: {msg}"
        );
    }
}
