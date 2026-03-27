# Command / Query Boundary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define the core-adapter command/query boundary so that core can send DeviceCommands (RequestReading, QueryConfig, SetOutput) to adapters and receive DeviceConfig responses as async events.

**Architecture:** Extend `AdapterCommand` with a `DeviceCommand` envelope carrying `DeviceKey` + `DeviceCommandPayload`. Add `AdapterEvent::DeviceConfig` for ParameterGet responses. Wire downlink bytes through a write channel from event_loop to reader thread. ConfigFrame conversion moves from convert.rs to event_loop where the devices map is available.

**Tech Stack:** Rust, tokio mpsc channels, BravePI codec (existing `encode_downlink`)

**Spec:** `docs/superpowers/specs/2026-03-27-command-boundary-design.md`

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `core/types/src/lib.rs` | Domain entity types, adapter-core boundary types | Add DeviceCommand, DeviceCommandPayload, DeviceConfigData, ConfigValue, new AdapterCommand/AdapterEvent variants |
| `bravepi-adapter/src/task/event_loop.rs` | Async event loop: frame dispatch, device lifecycle, command handling | Add DeviceTarget, extend DeviceState, add handle_device_command(), ConfigFrame→DeviceConfig, accept write_tx |
| `bravepi-adapter/src/task/convert.rs` | BravePiFrame → AdapterEvent pure conversion | Remove Config branch (moved to event_loop) |
| `bravepi-adapter/src/task/serial_source.rs` | Serial port I/O thread with reconnect | Accept write_rx, drain writes in read loop |
| `bravepi-adapter/src/task/handle.rs` | Adapter startup and lifecycle | Create write channel, plumb to serial_source and event_loop |
| `bravepi-adapter/src/transport.rs` | Transport layer type aliases | Add BytesSender type alias |
| `bravepi-adapter/src/task/event_loop_test.rs` | Integration tests for event_loop | Add command handling and ConfigFrame tests |
| `bravepi-adapter/src/task/convert_test.rs` | Unit tests for convert.rs | Update config_frame_returns_none test |

---

### Task 1: Core types — DeviceCommand, DeviceConfigData, ConfigValue

**Files:**
- Modify: `core/types/src/lib.rs`

- [ ] **Step 1: Write failing tests for new types**

Add at the bottom of `core/types/src/lib.rs`, inside a new `#[cfg(test)]` module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_command_construction() {
        let cmd = DeviceCommand {
            device_key: DeviceKey::new("bravepi:abc:temperature"),
            payload: DeviceCommandPayload::RequestReading,
        };
        assert_eq!(cmd.device_key.as_str(), "bravepi:abc:temperature");
    }

    #[test]
    fn device_command_set_output() {
        let cmd = DeviceCommand {
            device_key: DeviceKey::new("bravepi:abc:contact_output"),
            payload: DeviceCommandPayload::SetOutput {
                value: true,
                duration_ms: Some(5000),
            },
        };
        match cmd.payload {
            DeviceCommandPayload::SetOutput { value, duration_ms } => {
                assert!(value);
                assert_eq!(duration_ms, Some(5000));
            }
            _ => panic!("expected SetOutput"),
        }
    }

    #[test]
    fn adapter_command_device_command_variant() {
        let cmd = AdapterCommand::DeviceCommand(DeviceCommand {
            device_key: DeviceKey::new("test"),
            payload: DeviceCommandPayload::QueryConfig,
        });
        match cmd {
            AdapterCommand::DeviceCommand(dc) => {
                assert_eq!(dc.device_key.as_str(), "test");
            }
            _ => panic!("expected DeviceCommand"),
        }
    }

    #[test]
    fn device_config_data_construction() {
        let config = DeviceConfigData {
            firmware_version: Some("1.2.3".to_string()),
            uplink_interval_secs: Some(60),
            properties: BTreeMap::from([
                ("timezone".into(), ConfigValue::Integer(9)),
                ("ble_mode".into(), ConfigValue::Integer(1)),
            ]),
        };
        assert_eq!(config.firmware_version.as_deref(), Some("1.2.3"));
        assert_eq!(config.uplink_interval_secs, Some(60));
        assert_eq!(config.properties.len(), 2);
    }

    #[test]
    fn config_value_variants() {
        assert_eq!(ConfigValue::String("hello".into()), ConfigValue::String("hello".into()));
        assert_eq!(ConfigValue::Integer(42), ConfigValue::Integer(42));
        assert_eq!(ConfigValue::Float(3.14), ConfigValue::Float(3.14));
        assert_eq!(ConfigValue::Bool(true), ConfigValue::Bool(true));
    }

    #[test]
    fn adapter_event_device_config_variant() {
        let event = AdapterEvent::DeviceConfig {
            device_key: DeviceKey::new("bravepi:abc:temperature"),
            config: DeviceConfigData {
                firmware_version: Some("1.0.0".to_string()),
                uplink_interval_secs: None,
                properties: BTreeMap::new(),
            },
        };
        match event {
            AdapterEvent::DeviceConfig { device_key, config } => {
                assert_eq!(device_key.as_str(), "bravepi:abc:temperature");
                assert_eq!(config.firmware_version.as_deref(), Some("1.0.0"));
            }
            _ => panic!("expected DeviceConfig"),
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p iotkit-core-types 2>&1`
Expected: FAIL — `DeviceCommand`, `DeviceCommandPayload`, `DeviceConfigData`, `ConfigValue` not found.

- [ ] **Step 3: Add the new types and variants**

In `core/types/src/lib.rs`, add after the `AdapterCommand` enum (after line 173):

```rust
/// device-targeted command の共通 envelope。
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCommand {
    pub device_key: DeviceKey,
    pub payload: DeviceCommandPayload,
}

/// device command の payload。adapter 横断で意味が通る名前。
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceCommandPayload {
    /// センサーに即時読み取りを要求する。
    RequestReading,
    /// デバイスの設定情報を問い合わせる。
    QueryConfig,
    /// 接点出力を設定する。
    SetOutput {
        value: bool,
        duration_ms: Option<u32>,
    },
}

/// デバイス設定の応答 DTO。adapter 横断で共通の named field + adapter 固有の typed properties。
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceConfigData {
    pub firmware_version: Option<String>,
    pub uplink_interval_secs: Option<u32>,
    pub properties: BTreeMap<String, ConfigValue>,
}

/// 型付き設定値。downstream が parse 不要で使える lossless 表現。
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    String(String),
    Integer(i64),
    Float(f64),
    Bool(bool),
}
```

Modify the `AdapterCommand` enum to add the new variant:

```rust
/// core → adapter へ送信するコマンド。
#[derive(Debug, Clone, PartialEq)]
pub enum AdapterCommand {
    /// シャットダウン要求。
    Shutdown,
    /// デバイス宛コマンド。
    DeviceCommand(DeviceCommand),
}
```

Modify the `AdapterEvent` enum to add the new variant (after `AdapterError`):

```rust
    /// デバイス設定の応答。QueryConfig の結果として非同期に返る。
    DeviceConfig {
        device_key: DeviceKey,
        config: DeviceConfigData,
    },
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p iotkit-core-types 2>&1`
Expected: all 6 new tests PASS.

- [ ] **Step 5: Fix downstream compilation**

The `AdapterCommand` enum change breaks existing `match` in `event_loop.rs` (line 38-43). Update the match to handle the new variant temporarily:

In `bravepi-adapter/src/task/event_loop.rs`, change lines 38-43 from:

```rust
            cmd = command_rx.recv() => {
                match cmd {
                    Some(AdapterCommand::Shutdown) | None => {
                        tracing::info!("BravePI adapter shutting down");
                        return;
                    }
                }
            }
```

to:

```rust
            cmd = command_rx.recv() => {
                match cmd {
                    Some(AdapterCommand::Shutdown) | None => {
                        tracing::info!("BravePI adapter shutting down");
                        return;
                    }
                    Some(AdapterCommand::DeviceCommand(_)) => {
                        tracing::warn!("DeviceCommand not yet implemented");
                    }
                }
            }
```

Run: `cargo test -p iotkit-core-types -p bravepi-adapter -p bravepi-codec 2>&1`
Expected: all tests PASS (existing + new).

- [ ] **Step 6: Commit**

```bash
git add core/types/src/lib.rs bravepi-adapter/src/task/event_loop.rs
git commit -m "feat(core): add DeviceCommand, DeviceConfigData, ConfigValue types"
```

---

### Task 2: serial_source write channel

**Files:**
- Modify: `bravepi-adapter/src/task/serial_source.rs`
- Modify: `bravepi-adapter/src/transport.rs`
- Modify: `bravepi-adapter/src/task/handle.rs`

- [ ] **Step 1: Add BytesSender type alias**

In `bravepi-adapter/src/transport.rs`, add after the existing `BytesReceiver` type alias (line 19):

```rust
/// event_loop から serial_source に送る downlink byte stream の型。
pub(crate) type BytesSender = mpsc::Sender<Vec<u8>>;
```

- [ ] **Step 2: Update serial_source to accept write_rx**

In `bravepi-adapter/src/task/serial_source.rs`, make the following changes:

Add import at top (after line 4):

```rust
use crate::transport::BytesSender;
```

Change `SerialSource` struct to include the write sender:

```rust
pub(crate) struct SerialSource {
    pub bytes_rx: BytesReceiver,
    pub write_tx: BytesSender,
    pub handle: SerialSourceHandle,
}
```

Change `start()` function to create the write channel and pass it:

```rust
pub(crate) fn start(port_path: &str) -> Result<SerialSource, std::io::Error> {
    let config = serial_config();
    let transport = SerialTransport::open(port_path, &config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let (bytes_tx, bytes_rx) = mpsc::channel(64);
    let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(16);
    let owned_path = port_path.to_string();
    let thread_handle = std::thread::Builder::new()
        .name(format!("bravepi-serial-{}", port_path))
        .spawn(move || serial_reader_thread(owned_path, transport, bytes_tx, write_rx))?;
    Ok(SerialSource {
        bytes_rx,
        write_tx,
        handle: SerialSourceHandle { thread_handle },
    })
}
```

Update `serial_reader_thread` signature and add write drain logic:

```rust
fn serial_reader_thread(
    port_path: String,
    mut transport: SerialTransport,
    bytes_tx: mpsc::Sender<Result<Vec<u8>, TransportError>>,
    mut write_rx: mpsc::Receiver<Vec<u8>>,
) {
    tracing::info!(port = %port_path, "Serial reader thread started");
    let mut buf = [0u8; 4096];
    let timeout = Duration::from_millis(500);
    let mut retry_count: u32 = 0;

    loop {
        if bytes_tx.is_closed() {
            tracing::info!("Bytes channel closed, reader thread exiting");
            return;
        }

        // Drain pending writes before reading
        while let Ok(data) = write_rx.try_recv() {
            if let Err(e) = transport.write(&data) {
                tracing::error!(error = %e, port = %port_path, "Serial write error");
            }
        }

        match transport.read(&mut buf, timeout) {
            Ok(0) => continue,
            Ok(n) => {
                retry_count = 0;
                if bytes_tx.blocking_send(Ok(buf[..n].to_vec())).is_err() {
                    tracing::info!("Bytes channel closed, reader thread exiting");
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => {
                tracing::error!(error = %e, port = %port_path, "Serial read error");
                drop(transport);

                loop {
                    retry_count += 1;
                    if retry_count > MAX_RETRIES {
                        let msg = format!(
                            "Serial read error on {}: {} (max retries {} exceeded)",
                            port_path, e, MAX_RETRIES
                        );
                        tracing::error!("{}", msg);
                        let _ = bytes_tx.blocking_send(Err(TransportError { message: msg }));
                        return;
                    }

                    if bytes_tx.is_closed() {
                        tracing::info!("Bytes channel closed during retry, exiting");
                        return;
                    }

                    let backoff_secs = (1u64 << retry_count.min(5)).min(MAX_BACKOFF_SECS);
                    tracing::warn!(
                        port = %port_path,
                        retry = retry_count,
                        backoff_secs = backoff_secs,
                        "Attempting serial reconnect"
                    );
                    for _ in 0..backoff_secs {
                        if bytes_tx.is_closed() {
                            tracing::info!("Bytes channel closed during retry, exiting");
                            return;
                        }
                        std::thread::sleep(Duration::from_secs(1));
                    }

                    let config = serial_config();
                    match SerialTransport::open(&port_path, &config) {
                        Ok(new_transport) => {
                            tracing::info!(port = %port_path, "Serial reconnected");
                            transport = new_transport;
                            retry_count = 0;
                            break;
                        }
                        Err(open_err) => {
                            tracing::warn!(
                                error = %open_err,
                                port = %port_path,
                                "Reconnect failed"
                            );
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 3: Update handle.rs to plumb write_tx to event_loop**

In `bravepi-adapter/src/task/handle.rs`, update `start()` to pass write_tx:

```rust
use crate::transport::BytesSender;
```

Add to the existing imports. Then change the event_loop spawn call:

```rust
pub fn start(port_path: String) -> Result<AdapterHandle, std::io::Error> {
    let runtime_handle = tokio::runtime::Handle::try_current()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let source = serial_source::start(&port_path)?;
    let write_tx = source.write_tx.clone();

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(256);
    let (command_tx, command_rx) = mpsc::channel::<AdapterCommand>(32);
    let id = AdapterId::new(format!("bravepi:{}", port_path));

    let event_loop_handle = runtime_handle.spawn(
        event_loop(port_path, source.bytes_rx, event_tx, command_rx, write_tx)
    );

    Ok(AdapterHandle {
        id,
        event_rx,
        command_tx,
        source_handle: Some(source.handle),
        event_loop_handle: Some(event_loop_handle),
    })
}
```

- [ ] **Step 4: Update event_loop signature to accept write_tx**

In `bravepi-adapter/src/task/event_loop.rs`, update the function signature:

```rust
use crate::transport::BytesSender;
```

```rust
pub(crate) async fn event_loop(
    port_path: String,
    mut bytes_rx: BytesReceiver,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
    write_tx: BytesSender,
) {
```

Add `let _ = write_tx;` at the start of the function body (temporary, until Task 3 uses it).

- [ ] **Step 5: Update all event_loop_test.rs calls to pass write_tx**

In `bravepi-adapter/src/task/event_loop_test.rs`, each test creates an event_loop. Update every `tokio::spawn(event_loop(...))` call to include a write_tx.

Add at the top of the file:

```rust
use tokio::sync::mpsc as tokio_mpsc;
```

In each test, add after the command channel creation:

```rust
    let (write_tx, _write_rx) = tokio_mpsc::channel::<Vec<u8>>(16);
```

And update each `event_loop(...)` call to include `write_tx`. For example, in `shutdown_command_exits_event_loop`:

```rust
    let handle = tokio::spawn(event_loop("test".into(), bytes_rx, event_tx, command_rx, write_tx));
```

Apply the same pattern to all 6 tests: `shutdown_command_exits_event_loop`, `bytes_channel_error_produces_adapter_error`, `bytes_channel_close_produces_adapter_error`, `normal_data_flow_produces_device_discovered_then_sensor_data`, `contact_input_produces_device_discovered`, `same_transmitter_different_sensor_type_produces_two_discoveries`.

- [ ] **Step 6: Run tests to verify everything compiles and passes**

Run: `cargo test -p bravepi-adapter -p iotkit-core-types 2>&1`
Expected: all tests PASS.

- [ ] **Step 7: Commit**

```bash
git add bravepi-adapter/src/transport.rs bravepi-adapter/src/task/serial_source.rs bravepi-adapter/src/task/handle.rs bravepi-adapter/src/task/event_loop.rs bravepi-adapter/src/task/event_loop_test.rs
git commit -m "feat(bravepi): add write channel to serial_source for downlink"
```

---

### Task 3: DeviceState extension — DeviceTarget

**Files:**
- Modify: `bravepi-adapter/src/task/event_loop.rs`
- Modify: `bravepi-adapter/src/task/event_loop_test.rs`

- [ ] **Step 1: Write failing test for DeviceTarget in DeviceState**

In `bravepi-adapter/src/task/event_loop_test.rs`, add a new test that verifies a command sent to a discovered device produces encoded bytes on the write channel. This test will fail because `handle_device_command` doesn't exist yet, and `DeviceState` doesn't have `target`.

But first, we need to update the internal types. Since DeviceState is private to event_loop, we test it indirectly through integration tests. We'll add the integration test in Task 4. For now, update the struct.

- [ ] **Step 2: Update DeviceState and add DeviceTarget**

In `bravepi-adapter/src/task/event_loop.rs`, replace the `DeviceState` struct:

```rust
struct DeviceTarget {
    device_number_hex: String,
    raw_sensor_type: u16,
}

struct DeviceState {
    #[allow(dead_code)]
    last_seen: tokio::time::Instant,
    target: DeviceTarget,
}
```

- [ ] **Step 3: Update device insertion to populate DeviceTarget**

In `bravepi-adapter/src/task/event_loop.rs`, the `frame_to_event` call returns `(event, identity)` but doesn't expose `device_number` and `sensor_type_raw`. We need to extract this information from the frame before calling `frame_to_event`, or change `frame_to_event` to return it.

The cleanest approach: extract `device_number` and `sensor_type_raw` from the `BravePiFrame::Sensor` before passing to `frame_to_event`. Since `frame_to_event` takes ownership of the frame, we need to clone these values first.

Update the frame processing in the `bytes_rx` branch. Replace lines 49-86:

```rust
                        while let Some(frame) = codec.decode() {
                            // Extract target info from Sensor frames before frame_to_event consumes the frame
                            let target_info = match &frame {
                                bravepi_codec::BravePiFrame::Sensor(s) => {
                                    Some((s.device_number.clone(), s.sensor_type_raw))
                                }
                                _ => None,
                            };

                            if let Some((event, identity)) = frame_to_event(frame, &port_path) {
                                if let AdapterEvent::SensorData { ref device_key, .. } = event {
                                    if !devices.contains_key(device_key) {
                                        match identity {
                                            Some(identity) => {
                                                let discovered = AdapterEvent::DeviceDiscovered {
                                                    device_key: device_key.clone(),
                                                    identity,
                                                };
                                                if event_tx.send(discovered).await.is_err() {
                                                    tracing::warn!("Event channel closed, shutting down");
                                                    return;
                                                }
                                                let target = target_info.map(|(dn, rst)| DeviceTarget {
                                                    device_number_hex: dn,
                                                    raw_sensor_type: rst,
                                                }).expect("SensorData always comes from Sensor frame");
                                                devices.insert(
                                                    device_key.clone(),
                                                    DeviceState {
                                                        last_seen: tokio::time::Instant::now(),
                                                        target,
                                                    },
                                                );
                                            }
                                            None => {
                                                tracing::warn!(
                                                    device_key = %device_key,
                                                    "New device without identity, skipping"
                                                );
                                                continue;
                                            }
                                        }
                                    } else {
                                        devices.get_mut(device_key).unwrap().last_seen =
                                            tokio::time::Instant::now();
                                    }
                                }

                                if event_tx.send(event).await.is_err() {
                                    tracing::warn!("Event channel closed, shutting down");
                                    return;
                                }
                            }
                        }
```

- [ ] **Step 4: Run tests to verify everything still passes**

Run: `cargo test -p bravepi-adapter 2>&1`
Expected: all 27 existing tests PASS.

- [ ] **Step 5: Commit**

```bash
git add bravepi-adapter/src/task/event_loop.rs
git commit -m "refactor(bravepi): extend DeviceState with DeviceTarget for command routing"
```

---

### Task 4: handle_device_command — command dispatch and encoding

**Files:**
- Modify: `bravepi-adapter/src/task/event_loop.rs`
- Modify: `bravepi-adapter/src/task/event_loop_test.rs`

- [ ] **Step 1: Write failing test — RequestReading produces encoded bytes on write channel**

In `bravepi-adapter/src/task/event_loop_test.rs`, add:

```rust
#[tokio::test]
async fn device_command_request_reading_produces_downlink_bytes() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx, write_tx));

    // First, discover a device so it exists in the devices map
    let device: u64 = 0x246880020140018b;
    let frame_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    // Drain DeviceDiscovered + SensorData
    let _ = event_rx.recv().await.unwrap(); // DeviceDiscovered
    let _ = event_rx.recv().await.unwrap(); // SensorData

    // Send RequestReading command
    command_tx.send(AdapterCommand::DeviceCommand(
        iotkit_core_types::DeviceCommand {
            device_key: iotkit_core_types::DeviceKey::new("bravepi:246880020140018b:temperature"),
            payload: iotkit_core_types::DeviceCommandPayload::RequestReading,
        }
    )).await.unwrap();

    // The encoded bytes should appear on the write channel
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        write_rx.recv(),
    ).await.expect("should receive within timeout").expect("write channel should have data");

    // Verify it's a valid ImmediateUplink frame (opcode 0x00, sensor_type 261)
    assert!(!bytes.is_empty());
    // Byte 0 = 0x00 (downlink direction)
    assert_eq!(bytes[0], 0x00);

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 2: Write failing test — unknown device_key produces AdapterError**

```rust
#[tokio::test]
async fn device_command_unknown_device_produces_adapter_error() {
    let (_bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx, write_tx));

    // Send command to unknown device (no discovery happened)
    command_tx.send(AdapterCommand::DeviceCommand(
        iotkit_core_types::DeviceCommand {
            device_key: iotkit_core_types::DeviceKey::new("bravepi:unknown:temperature"),
            payload: iotkit_core_types::DeviceCommandPayload::RequestReading,
        }
    )).await.unwrap();

    let event = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        event_rx.recv(),
    ).await.expect("should receive within timeout").expect("should receive event");

    match event {
        AdapterEvent::AdapterError { device_key, error } => {
            assert_eq!(device_key.unwrap().as_str(), "bravepi:unknown:temperature");
            assert!(error.contains("unknown device"), "error was: {}", error);
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 3: Write failing test — SetOutput to non-ContactOutput device produces AdapterError**

```rust
#[tokio::test]
async fn set_output_to_non_contact_device_produces_adapter_error() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx, write_tx));

    // Discover a temperature device
    let device: u64 = 0x246880020140018b;
    let frame_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();
    let _ = event_rx.recv().await.unwrap(); // DeviceDiscovered
    let _ = event_rx.recv().await.unwrap(); // SensorData

    // Send SetOutput to temperature device (wrong type)
    command_tx.send(AdapterCommand::DeviceCommand(
        iotkit_core_types::DeviceCommand {
            device_key: iotkit_core_types::DeviceKey::new("bravepi:246880020140018b:temperature"),
            payload: iotkit_core_types::DeviceCommandPayload::SetOutput {
                value: true,
                duration_ms: Some(1000),
            },
        }
    )).await.unwrap();

    let event = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        event_rx.recv(),
    ).await.expect("should receive within timeout").expect("should receive event");

    match event {
        AdapterEvent::AdapterError { device_key, error } => {
            assert!(device_key.is_some());
            assert!(error.contains("ContactOutput"), "error was: {}", error);
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 4: Write failing test — SetOutput duration_ms exceeding u16::MAX produces AdapterError**

```rust
#[tokio::test]
async fn set_output_duration_exceeds_u16_max_produces_adapter_error() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx, write_tx));

    // Discover a contact_output device
    let device: u64 = 0x1234567890abcdef;
    let frame_bytes = build_sensor_frame_bytes(device, 258, -70, 100, 2, &[0x00, 0x01]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();
    let _ = event_rx.recv().await.unwrap(); // DeviceDiscovered
    let _ = event_rx.recv().await.unwrap(); // SensorData

    // Send SetOutput with duration exceeding u16::MAX
    command_tx.send(AdapterCommand::DeviceCommand(
        iotkit_core_types::DeviceCommand {
            device_key: iotkit_core_types::DeviceKey::new("bravepi:1234567890abcdef:contact_output"),
            payload: iotkit_core_types::DeviceCommandPayload::SetOutput {
                value: true,
                duration_ms: Some(70000), // > 65535
            },
        }
    )).await.unwrap();

    let event = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        event_rx.recv(),
    ).await.expect("should receive within timeout").expect("should receive event");

    match event {
        AdapterEvent::AdapterError { device_key, error } => {
            assert!(device_key.is_some());
            assert!(error.contains("duration_ms"), "error was: {}", error);
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 5: Run tests to verify they fail**

Run: `cargo test -p bravepi-adapter 2>&1`
Expected: 4 new tests FAIL (handle_device_command not implemented).

- [ ] **Step 6: Implement handle_device_command**

In `bravepi-adapter/src/task/event_loop.rs`, add these imports at the top:

```rust
use bravepi_codec::{BravePiCodec, DownlinkCommand};
use iotkit_core_types::{AdapterCommand, AdapterEvent, DeviceKey, DeviceCommandPayload, SensorType};
use crate::registry::lookup_handler;
```

(Replace the existing `use iotkit_core_types::{AdapterCommand, AdapterEvent, DeviceKey};` line.)

Add the `handle_device_command` function after the `event_loop` function:

```rust
async fn handle_device_command(
    cmd: iotkit_core_types::DeviceCommand,
    devices: &HashMap<DeviceKey, DeviceState>,
    write_tx: &BytesSender,
    event_tx: &mpsc::Sender<AdapterEvent>,
) {
    let state = match devices.get(&cmd.device_key) {
        Some(s) => s,
        None => {
            let _ = event_tx.send(AdapterEvent::AdapterError {
                device_key: Some(cmd.device_key),
                error: "unknown device".to_string(),
            }).await;
            return;
        }
    };

    let target = &state.target;

    // Validate SetOutput constraints
    if let DeviceCommandPayload::SetOutput { duration_ms, .. } = &cmd.payload {
        // Must be ContactOutput endpoint
        if let Some(handler) = lookup_handler(target.raw_sensor_type) {
            if handler.sensor_type != SensorType::ContactOutput {
                let _ = event_tx.send(AdapterEvent::AdapterError {
                    device_key: Some(cmd.device_key),
                    error: "SetOutput sent to non-ContactOutput device".to_string(),
                }).await;
                return;
            }
        } else {
            let _ = event_tx.send(AdapterEvent::AdapterError {
                device_key: Some(cmd.device_key),
                error: "SetOutput: unknown sensor type in registry".to_string(),
            }).await;
            return;
        }

        // duration_ms must fit in u16
        if let Some(ms) = duration_ms {
            if *ms > u16::MAX as u32 {
                let _ = event_tx.send(AdapterEvent::AdapterError {
                    device_key: Some(cmd.device_key),
                    error: format!("duration_ms {} exceeds u16 range (max {})", ms, u16::MAX),
                }).await;
                return;
            }
        }
    }

    // Convert payload to DownlinkCommand
    let downlink_cmd = match cmd.payload {
        DeviceCommandPayload::RequestReading => {
            DownlinkCommand::ImmediateUplink { sensor_type: target.raw_sensor_type }
        }
        DeviceCommandPayload::QueryConfig => {
            DownlinkCommand::ParameterGet
        }
        DeviceCommandPayload::SetOutput { value, duration_ms } => {
            DownlinkCommand::ContactOutput {
                signal_mode: if value { 1 } else { 0 },
                signal_out_time: duration_ms.map(|ms| ms as u16).unwrap_or(0),
            }
        }
    };

    // Encode to bytes
    let bytes = match BravePiCodec::encode_downlink(&target.device_number_hex, &downlink_cmd) {
        Ok(b) => b,
        Err(e) => {
            let _ = event_tx.send(AdapterEvent::AdapterError {
                device_key: Some(cmd.device_key),
                error: format!("encode_downlink failed: {}", e),
            }).await;
            return;
        }
    };

    // Send to serial_source via write channel (non-blocking)
    match write_tx.try_send(bytes) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            let _ = event_tx.send(AdapterEvent::AdapterError {
                device_key: Some(cmd.device_key),
                error: "downlink queue full".to_string(),
            }).await;
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            let _ = event_tx.send(AdapterEvent::AdapterError {
                device_key: None,
                error: "write channel closed (transport failure)".to_string(),
            }).await;
        }
    }
}
```

Replace the temporary `DeviceCommand` match arm in the `command_rx` branch:

```rust
                    Some(AdapterCommand::DeviceCommand(cmd)) => {
                        handle_device_command(cmd, &devices, &write_tx, &event_tx).await;
                    }
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p bravepi-adapter 2>&1`
Expected: all tests PASS (existing 27 + 4 new = 31).

- [ ] **Step 8: Commit**

```bash
git add bravepi-adapter/src/task/event_loop.rs bravepi-adapter/src/task/event_loop_test.rs
git commit -m "feat(bravepi): implement handle_device_command with validation and encoding"
```

---

### Task 5: ConfigFrame → DeviceConfig event

**Files:**
- Modify: `bravepi-adapter/src/task/event_loop.rs`
- Modify: `bravepi-adapter/src/task/convert.rs`
- Modify: `bravepi-adapter/src/task/event_loop_test.rs`
- Modify: `bravepi-adapter/src/task/convert_test.rs`

- [ ] **Step 1: Write failing test — ConfigFrame produces DeviceConfig event**

In `bravepi-adapter/src/task/event_loop_test.rs`, add:

```rust
fn build_config_frame_bytes(device_number: u64, true_sensor_type: u16) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&true_sensor_type.to_le_bytes()); // true_sensor_type
    payload.extend_from_slice(&[1, 2, 3]); // firmware_version "1.2.3"
    payload.push(9);   // timezone
    payload.push(1);   // ble_mode
    payload.push(4);   // tx_power
    payload.extend_from_slice(&1000u16.to_le_bytes()); // advertise_interval
    payload.extend_from_slice(&60u32.to_le_bytes());   // uplink_interval

    // Build uplink frame with sensor_type=0 (config)
    let payload_len = payload.len() as u16;
    let mut frame = Vec::new();
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&device_number.to_le_bytes());
    frame.extend_from_slice(&0u16.to_le_bytes()); // sensor_type = 0 means config
    frame.push((-50i8) as u8); // rssi
    frame.push(0x00); // flag
    frame.extend_from_slice(&payload);
    frame
}

#[tokio::test]
async fn config_frame_produces_device_config_event() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx, write_tx));

    let device: u64 = 0x246880020140018b;

    // First, discover the device via a sensor frame
    let sensor_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(sensor_bytes)).await.unwrap();
    let _ = event_rx.recv().await.unwrap(); // DeviceDiscovered
    let _ = event_rx.recv().await.unwrap(); // SensorData

    // Send a config frame for the same device
    let config_bytes = build_config_frame_bytes(device, 261);
    bytes_tx.send(Ok(config_bytes)).await.unwrap();

    let event = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        event_rx.recv(),
    ).await.expect("should receive within timeout").expect("should receive event");

    match event {
        AdapterEvent::DeviceConfig { device_key, config } => {
            assert_eq!(device_key.as_str(), "bravepi:246880020140018b:temperature");
            assert_eq!(config.firmware_version.as_deref(), Some("1.2.3"));
            assert_eq!(config.uplink_interval_secs, Some(60));
            assert_eq!(
                config.properties.get("timezone"),
                Some(&iotkit_core_types::ConfigValue::Integer(9))
            );
        }
        other => panic!("expected DeviceConfig, got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 2: Write failing test — ConfigFrame for undiscovered device is dropped**

```rust
#[tokio::test]
async fn config_frame_for_undiscovered_device_is_dropped() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx, write_tx));

    // Send config frame without any prior discovery
    let device: u64 = 0x246880020140018b;
    let config_bytes = build_config_frame_bytes(device, 261);
    bytes_tx.send(Ok(config_bytes)).await.unwrap();

    // Send shutdown — if config was dropped, shutdown will be the next event_loop action
    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();

    // No DeviceConfig event should have been sent
    assert!(event_rx.recv().await.is_none(), "should receive no events before channel close");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p bravepi-adapter -- config_frame 2>&1`
Expected: FAIL — ConfigFrame handling not yet implemented.

- [ ] **Step 4: Remove Config branch from convert.rs**

In `bravepi-adapter/src/task/convert.rs`, replace the `BravePiFrame::Config` branch (lines 52-60):

```rust
        BravePiFrame::Config(_) => None,
```

This keeps the same return type (None) but removes the logging. Config handling moves to event_loop.

- [ ] **Step 5: Update convert_test.rs**

In `bravepi-adapter/src/task/convert_test.rs`, update the `config_frame_returns_none` test (lines 118-132). The test still expects None, so no behavior change:

```rust
#[test]
fn config_frame_returns_none() {
    let frame = BravePiFrame::Config(ConfigFrame {
        device_number: "246880020140018b".to_string(),
        rssi: -55,
        true_sensor_type: 261,
        firmware_version: "1.2.3".to_string(),
        timezone: 9,
        ble_mode: 1,
        tx_power: 4,
        advertise_interval: 1000,
        uplink_interval: 60,
    });
    assert!(frame_to_event(frame, "/dev/test").is_none());
}
```

(This test is unchanged; it still passes. Just confirming it stays valid.)

- [ ] **Step 6: Implement ConfigFrame handling in event_loop**

In `bravepi-adapter/src/task/event_loop.rs`, add these imports:

```rust
use std::collections::BTreeMap;
use iotkit_core_types::{DeviceConfigData, ConfigValue};
```

In the `while let Some(frame) = codec.decode()` block, add ConfigFrame handling *before* the `frame_to_event` call. Replace the entire `while let` block:

```rust
                        while let Some(frame) = codec.decode() {
                            // Handle ConfigFrame directly (needs devices map)
                            if let bravepi_codec::BravePiFrame::Config(ref cfg) = frame {
                                handle_config_frame(cfg, &devices, &port_path, &event_tx).await;
                                continue;
                            }

                            // Extract target info from Sensor frames before frame_to_event consumes the frame
                            let target_info = match &frame {
                                bravepi_codec::BravePiFrame::Sensor(s) => {
                                    Some((s.device_number.clone(), s.sensor_type_raw))
                                }
                                _ => None,
                            };

                            if let Some((event, identity)) = frame_to_event(frame, &port_path) {
                                if let AdapterEvent::SensorData { ref device_key, .. } = event {
                                    if !devices.contains_key(device_key) {
                                        match identity {
                                            Some(identity) => {
                                                let discovered = AdapterEvent::DeviceDiscovered {
                                                    device_key: device_key.clone(),
                                                    identity,
                                                };
                                                if event_tx.send(discovered).await.is_err() {
                                                    tracing::warn!("Event channel closed, shutting down");
                                                    return;
                                                }
                                                let target = target_info.map(|(dn, rst)| DeviceTarget {
                                                    device_number_hex: dn,
                                                    raw_sensor_type: rst,
                                                }).expect("SensorData always comes from Sensor frame");
                                                devices.insert(
                                                    device_key.clone(),
                                                    DeviceState {
                                                        last_seen: tokio::time::Instant::now(),
                                                        target,
                                                    },
                                                );
                                            }
                                            None => {
                                                tracing::warn!(
                                                    device_key = %device_key,
                                                    "New device without identity, skipping"
                                                );
                                                continue;
                                            }
                                        }
                                    } else {
                                        devices.get_mut(device_key).unwrap().last_seen =
                                            tokio::time::Instant::now();
                                    }
                                }

                                if event_tx.send(event).await.is_err() {
                                    tracing::warn!("Event channel closed, shutting down");
                                    return;
                                }
                            }
                        }
```

Add the `handle_config_frame` function:

```rust
async fn handle_config_frame(
    cfg: &bravepi_codec::ConfigFrame,
    devices: &HashMap<DeviceKey, DeviceState>,
    port_path: &str,
    event_tx: &mpsc::Sender<AdapterEvent>,
) {
    let handler = match lookup_handler(cfg.true_sensor_type) {
        Some(h) => h,
        None => {
            tracing::warn!(
                raw = cfg.true_sensor_type,
                device = %cfg.device_number,
                "ConfigFrame with unknown sensor type, dropping"
            );
            return;
        }
    };

    let device_key = DeviceKey::new(
        format!("bravepi:{}:{}", cfg.device_number, handler.key_suffix),
    );

    if !devices.contains_key(&device_key) {
        tracing::warn!(
            device_key = %device_key,
            "ConfigFrame for undiscovered device, dropping"
        );
        return;
    }

    let config = DeviceConfigData {
        firmware_version: Some(cfg.firmware_version.clone()),
        uplink_interval_secs: Some(cfg.uplink_interval),
        properties: BTreeMap::from([
            ("timezone".into(), ConfigValue::Integer(cfg.timezone as i64)),
            ("ble_mode".into(), ConfigValue::Integer(cfg.ble_mode as i64)),
            ("tx_power".into(), ConfigValue::Integer(cfg.tx_power as i64)),
            ("advertise_interval".into(), ConfigValue::Integer(cfg.advertise_interval as i64)),
        ]),
    };

    tracing::info!(
        device = %cfg.device_number,
        firmware = %cfg.firmware_version,
        "Config frame received, sending DeviceConfig event"
    );

    let _ = event_tx.send(AdapterEvent::DeviceConfig {
        device_key,
        config,
    }).await;
}
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p bravepi-adapter -p iotkit-core-types 2>&1`
Expected: all tests PASS (existing + new config tests).

- [ ] **Step 8: Commit**

```bash
git add bravepi-adapter/src/task/event_loop.rs bravepi-adapter/src/task/convert.rs bravepi-adapter/src/task/convert_test.rs bravepi-adapter/src/task/event_loop_test.rs
git commit -m "feat(bravepi): convert ConfigFrame to DeviceConfig event in event_loop"
```

---

### Task 6: SetOutput happy path test + final verification

**Files:**
- Modify: `bravepi-adapter/src/task/event_loop_test.rs`

- [ ] **Step 1: Write test — SetOutput to ContactOutput device produces encoded bytes**

```rust
#[tokio::test]
async fn set_output_to_contact_output_device_produces_downlink_bytes() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx, write_tx));

    // Discover a contact_output device
    let device: u64 = 0x1234567890abcdef;
    let frame_bytes = build_sensor_frame_bytes(device, 258, -70, 100, 2, &[0x00, 0x01]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();
    let _ = event_rx.recv().await.unwrap(); // DeviceDiscovered
    let _ = event_rx.recv().await.unwrap(); // SensorData

    // Send SetOutput command
    command_tx.send(AdapterCommand::DeviceCommand(
        iotkit_core_types::DeviceCommand {
            device_key: iotkit_core_types::DeviceKey::new("bravepi:1234567890abcdef:contact_output"),
            payload: iotkit_core_types::DeviceCommandPayload::SetOutput {
                value: true,
                duration_ms: Some(5000),
            },
        }
    )).await.unwrap();

    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        write_rx.recv(),
    ).await.expect("should receive within timeout").expect("write channel should have data");

    // Verify it's a valid ContactOutput frame
    assert!(!bytes.is_empty());
    assert_eq!(bytes[0], 0x00); // downlink direction

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 2: Write test — QueryConfig produces encoded bytes**

```rust
#[tokio::test]
async fn query_config_produces_downlink_bytes() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx, write_tx));

    // Discover a device
    let device: u64 = 0x246880020140018b;
    let frame_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();
    let _ = event_rx.recv().await.unwrap(); // DeviceDiscovered
    let _ = event_rx.recv().await.unwrap(); // SensorData

    // Send QueryConfig command
    command_tx.send(AdapterCommand::DeviceCommand(
        iotkit_core_types::DeviceCommand {
            device_key: iotkit_core_types::DeviceKey::new("bravepi:246880020140018b:temperature"),
            payload: iotkit_core_types::DeviceCommandPayload::QueryConfig,
        }
    )).await.unwrap();

    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        write_rx.recv(),
    ).await.expect("should receive within timeout").expect("write channel should have data");

    assert!(!bytes.is_empty());
    assert_eq!(bytes[0], 0x00); // downlink direction

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 3: Run all tests**

Run: `cargo test -p iotkit-core-types -p bravepi-adapter -p bravepi-codec -p bravepi-sensors 2>&1`
Expected: all tests PASS.

- [ ] **Step 4: Run clippy**

Run: `cargo clippy -p iotkit-core-types -p bravepi-adapter -p bravepi-sensors --lib --tests -- -D warnings 2>&1`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add bravepi-adapter/src/task/event_loop_test.rs
git commit -m "test(bravepi): add SetOutput and QueryConfig happy path tests"
```

- [ ] **Step 6: Final check — git diff --check**

Run: `git diff --check master...HEAD 2>&1`
Expected: no trailing whitespace issues.
