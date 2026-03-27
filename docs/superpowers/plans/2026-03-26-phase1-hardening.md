# Phase 1 Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 4 remaining issues from code review before proceeding to Phase 2.

**Architecture:** All changes are in `bravepi-adapter/src/task.rs` and its tests. `frame_to_event` signature changes to return identity alongside events. `event_loop` becomes pub for testability. Serial reader thread gains auto-retry with exponential backoff. `AdapterHandle` gains JoinHandle and shutdown method.

**Tech Stack:** Rust, tokio (mpsc, select), std::thread, std::collections::HashSet

---

### Task 1: JoinHandle retention in AdapterHandle

**Files:**
- Modify: `bravepi-adapter/src/task.rs:18-54`

- [ ] **Step 1: Add reader_thread field to AdapterHandle and shutdown method**

Change `AdapterHandle` and `start()` in `bravepi-adapter/src/task.rs`:

```rust
/// adapter 起動結果。core はこの handle を使って adapter と通信する。
pub struct AdapterHandle {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
    reader_thread: Option<std::thread::JoinHandle<()>>,
}

impl AdapterHandle {
    /// Send Shutdown command and wait for the reader thread to exit.
    pub async fn shutdown(mut self) -> Result<(), String> {
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.reader_thread.take() {
            handle.join().map_err(|_| "Reader thread panicked".to_string())?;
        }
        Ok(())
    }
}
```

In `start()`, capture the JoinHandle:

```rust
    let join_handle = std::thread::Builder::new()
        .name(format!("bravepi-serial-{}", port_path))
        .spawn(move || serial_reader_thread(reader_port, transport, bytes_tx))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    // ...

    Ok(AdapterHandle {
        id,
        event_rx,
        command_tx,
        reader_thread: Some(join_handle),
    })
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check -p bravepi-adapter 2>&1`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add bravepi-adapter/src/task.rs
git commit -m "feat(adapter): retain reader thread JoinHandle in AdapterHandle

Add shutdown() method for graceful thread cleanup."
```

---

### Task 2: Serial error auto-retry in reader thread

**Files:**
- Modify: `bravepi-adapter/src/task.rs:56-89`

- [ ] **Step 1: Rewrite serial_reader_thread with retry logic**

Replace the entire `serial_reader_thread` function:

```rust
const MAX_RETRIES: u32 = 10;
const MAX_BACKOFF_SECS: u64 = 30;

/// 専用スレッド: serial port から読んで bytes channel に送る。
/// エラー時は exponential backoff で再接続を試みる。
fn serial_reader_thread(
    port_path: String,
    mut transport: SerialTransport,
    bytes_tx: mpsc::Sender<Result<Vec<u8>, String>>,
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

        match transport.read(&mut buf, timeout) {
            Ok(0) => continue,
            Ok(n) => {
                retry_count = 0; // 読み取り成功でリセット
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
                        let _ = bytes_tx.blocking_send(Err(msg));
                        return;
                    }

                    let backoff = Duration::from_secs(
                        (1u64 << retry_count.min(5)).min(MAX_BACKOFF_SECS)
                    );
                    tracing::warn!(
                        port = %port_path,
                        retry = retry_count,
                        backoff_secs = backoff.as_secs(),
                        "Attempting serial reconnect"
                    );
                    std::thread::sleep(backoff);

                    if bytes_tx.is_closed() {
                        tracing::info!("Bytes channel closed during retry, exiting");
                        return;
                    }

                    let config = serial_config();
                    match SerialTransport::open(&port_path, &config) {
                        Ok(new_transport) => {
                            tracing::info!(port = %port_path, "Serial reconnected");
                            transport = new_transport;
                            retry_count = 0;
                            break; // 外側の read loop に戻る
                        }
                        Err(open_err) => {
                            tracing::warn!(
                                error = %open_err,
                                port = %port_path,
                                "Reconnect failed"
                            );
                            // 次の retry へ
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check -p bravepi-adapter 2>&1`
Expected: compiles with no errors

- [ ] **Step 3: Run existing tests**

Run: `cargo test --workspace 2>&1 | grep "test result"`
Expected: all tests pass (44 tests)

- [ ] **Step 4: Commit**

```bash
git add bravepi-adapter/src/task.rs
git commit -m "feat(adapter): auto-retry serial connection with exponential backoff

MAX_RETRIES=10, backoff up to 30s. Reader thread reconnects
automatically on serial errors instead of dying permanently."
```

---

### Task 3: Change frame_to_event signature to return SensorIdentity

**Files:**
- Modify: `bravepi-adapter/src/task.rs:150-205`
- Modify: `bravepi-adapter/tests/frame_to_event_test.rs` (all 12 tests)

- [ ] **Step 1: Update frame_to_event signature and implementation**

Add imports at the top of `task.rs`:

```rust
use iotkit_core_types::{
    AdapterCommand, AdapterEvent, AdapterId, ConnectionInfo, ConnectionKind,
    DeviceKey, SensorIdentity, SensorReading, SensorType,
};
use std::collections::BTreeMap;
```

Replace `frame_to_event`:

```rust
/// BravePiFrame を AdapterEvent に変換する。
/// SensorData フレームの場合は SensorIdentity も返す (DeviceDiscovered 用)。
/// None を返す場合、そのフレームは core に通知する必要がない。
pub fn frame_to_event(frame: BravePiFrame, port_path: &str) -> Option<(AdapterEvent, Option<SensorIdentity>)> {
    match frame {
        BravePiFrame::Sensor(s) => {
            let sensor_type = sensor_type_from_bravepi_raw(s.sensor_type_raw);
            let device_key = DeviceKey(s.device_number.clone());

            let reading = match sensor_type {
                SensorType::Temperature => mcp9600::from_uart_payload(&s.value_data),
                SensorType::Illuminance => opt3001::from_uart_payload(&s.value_data),
                SensorType::Adc => mcp3427::from_uart_payload(&s.value_data),
                SensorType::Ranging => vl53l1x::from_uart_payload(&s.value_data),
                SensorType::DifferentialPressure => sdp810::from_uart_payload(&s.value_data),
                SensorType::Acceleration => lis2duxs12::from_uart_payload(&s.value_data),
                SensorType::ContactInput | SensorType::ContactOutput => {
                    let values: Vec<f64> = s
                        .value_data
                        .iter()
                        .take(s.data_count as usize)
                        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
                        .collect();
                    SensorReading::new(sensor_type.clone(), values)
                }
                SensorType::Unknown(_) => {
                    tracing::warn!(raw = s.sensor_type_raw, "Unknown sensor type, skipping");
                    return None;
                }
            };

            let conn_info = ConnectionInfo {
                kind: ConnectionKind::Uart,
                parameters: BTreeMap::from([
                    ("port".into(), port_path.to_string()),
                    ("transmitter_id".into(), s.device_number.clone()),
                ]),
            };

            let identity = match sensor_type {
                SensorType::Temperature => Some(mcp9600::identity(conn_info)),
                SensorType::Illuminance => Some(opt3001::identity(conn_info)),
                SensorType::Adc => Some(mcp3427::identity(conn_info)),
                SensorType::Ranging => Some(vl53l1x::identity(conn_info)),
                SensorType::DifferentialPressure => Some(sdp810::identity(conn_info)),
                SensorType::Acceleration => Some(lis2duxs12::identity(conn_info)),
                _ => None,
            };

            let event = AdapterEvent::SensorData {
                device_key,
                reading,
                rssi: Some(s.rssi as i16),
                battery_pct: Some(s.battery),
            };

            Some((event, identity))
        }
        BravePiFrame::Config(cfg) => {
            tracing::info!(
                device = %cfg.device_number,
                sensor_type = cfg.true_sensor_type,
                firmware = %cfg.firmware_version,
                "Config frame received"
            );
            None
        }
        BravePiFrame::DecodeError {
            device_number,
            sensor_type_raw,
            reason,
        } => Some((
            AdapterEvent::AdapterError {
                device_key: Some(DeviceKey(device_number)),
                error: format!("Decode error (type={}): {}", sensor_type_raw, reason),
            },
            None,
        )),
    }
}
```

- [ ] **Step 2: Update all 12 existing tests for new signature**

In `bravepi-adapter/tests/frame_to_event_test.rs`, update the import and helper usage.

Change line 1:
```rust
use bravepi_adapter::task::frame_to_event;
use bravepi_codec::codec::{BravePiFrame, ConfigFrame, SensorFrame};
use iotkit_core_types::{AdapterEvent, SensorType};
```

Every test that calls `frame_to_event(frame)` becomes `frame_to_event(frame, "/dev/test")`.

Every test that does `.expect("should produce event")` now gets a tuple: change pattern to `let (event, _identity) = frame_to_event(frame, "/dev/test").expect(...)`.

For `unknown_sensor_type_returns_none` and `config_frame_returns_none`: update to `frame_to_event(frame, "/dev/test").is_none()`.

Add one new test for identity:
```rust
#[test]
fn temperature_frame_returns_identity() {
    let frame = BravePiFrame::Sensor(make_sensor_frame(261, vec![0x00, 0x80, 0xb3, 0x41]));
    let (_event, identity) = frame_to_event(frame, "/dev/ttyAMA0").expect("should produce event");

    let identity = identity.expect("temperature should have identity");
    assert_eq!(identity.manufacturer, "Braveridge");
    assert_eq!(identity.ic_part_number, "MCP9600");
    assert_eq!(identity.sensor_type, SensorType::Temperature);
}

#[test]
fn contact_input_has_no_identity() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "test".to_string(),
        sensor_type_raw: 257,
        rssi: -50,
        battery: 80,
        data_count: 1,
        value_data: vec![0x01],
    });
    let (_event, identity) = frame_to_event(frame, "/dev/test").expect("should produce event");
    assert!(identity.is_none());
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p bravepi-adapter --test frame_to_event_test 2>&1`
Expected: 14 tests pass (12 updated + 2 new)

- [ ] **Step 4: Commit**

```bash
git add bravepi-adapter/src/task.rs bravepi-adapter/tests/frame_to_event_test.rs
git commit -m "feat(adapter): frame_to_event returns SensorIdentity alongside event

Prepares for DeviceDiscovered emission. Pure function, no state."
```

---

### Task 4: DeviceDiscovered emission in event_loop

**Files:**
- Modify: `bravepi-adapter/src/task.rs:91-148` (event_loop)

- [ ] **Step 1: Add HashSet import and update event_loop**

Add to imports at top of `task.rs`:
```rust
use std::collections::HashSet;
```

Make `event_loop` pub and add seen_devices tracking:

```rust
/// async task: raw bytes → codec → AdapterEvent。
pub async fn event_loop(
    port_path: String,
    mut bytes_rx: mpsc::Receiver<Result<Vec<u8>, String>>,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
) {
    tracing::info!(port = %port_path, "BravePI adapter event loop started");

    let mut codec = BravePiCodec::new();
    let mut seen_devices: HashSet<DeviceKey> = HashSet::new();

    loop {
        tokio::select! {
            biased;

            cmd = command_rx.recv() => {
                match cmd {
                    Some(AdapterCommand::Shutdown) | None => {
                        tracing::info!("BravePI adapter shutting down");
                        return;
                    }
                }
            }
            result = bytes_rx.recv() => {
                match result {
                    Some(Ok(data)) => {
                        codec.feed(&data);
                        while let Some(frame) = codec.decode() {
                            if let Some((event, identity)) = frame_to_event(frame, &port_path) {
                                // 初回デバイスは DeviceDiscovered を先に送信
                                if let AdapterEvent::SensorData { ref device_key, .. } = event {
                                    if let Some(identity) = identity {
                                        if seen_devices.insert(device_key.clone()) {
                                            let discovered = AdapterEvent::DeviceDiscovered {
                                                device_key: device_key.clone(),
                                                identity,
                                            };
                                            if event_tx.send(discovered).await.is_err() {
                                                tracing::warn!("Event channel closed, shutting down");
                                                return;
                                            }
                                        }
                                    }
                                }

                                if event_tx.send(event).await.is_err() {
                                    tracing::warn!("Event channel closed, shutting down");
                                    return;
                                }
                            }
                        }
                    }
                    Some(Err(error)) => {
                        tracing::error!(%error, "Serial reader reported error");
                        let _ = event_tx.send(AdapterEvent::AdapterError {
                            device_key: None,
                            error,
                        }).await;
                        return;
                    }
                    None => {
                        tracing::warn!("Serial reader thread exited without error report");
                        let _ = event_tx.send(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!("Serial reader thread for {} exited unexpectedly", port_path),
                        }).await;
                        return;
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check -p bravepi-adapter 2>&1`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add bravepi-adapter/src/task.rs
git commit -m "feat(adapter): emit DeviceDiscovered on first sensor data per device

event_loop tracks seen devices with HashSet. First SensorData from
a new device triggers DeviceDiscovered before the SensorData event."
```

---

### Task 5: event_loop async integration tests

**Files:**
- Create: `bravepi-adapter/tests/event_loop_test.rs`

- [ ] **Step 1: Create event_loop_test.rs with all 4 test scenarios**

Create `bravepi-adapter/tests/event_loop_test.rs`:

```rust
use bravepi_adapter::task::event_loop;
use iotkit_core_types::{AdapterCommand, AdapterEvent, SensorType};
use tokio::sync::mpsc;

/// ヘルパー: codec テストから流用。フレームバイト列を構築する。
fn build_sensor_frame_bytes(device_number: u64, sensor_type: u16, rssi: i8, battery: u8, count: u16, values: &[u8]) -> Vec<u8> {
    let mut payload = vec![battery];
    payload.extend_from_slice(&count.to_le_bytes());
    payload.extend_from_slice(values);

    let payload_len = payload.len() as u16;
    let mut frame = Vec::new();
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&device_number.to_le_bytes());
    frame.extend_from_slice(&sensor_type.to_le_bytes());
    frame.push(rssi as u8);
    frame.push(0x00); // flag = no continuation
    frame.extend_from_slice(&payload);
    frame
}

#[tokio::test]
async fn shutdown_command_exits_event_loop() {
    let (_bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("test".into(), bytes_rx, event_tx, command_rx));

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();

    // event_loop が終了すると event_tx が drop され、recv は None を返す
    assert!(event_rx.recv().await.is_none());
}

#[tokio::test]
async fn bytes_channel_error_produces_adapter_error() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("test".into(), bytes_rx, event_tx, command_rx));

    bytes_tx.send(Err("serial port disconnected".to_string())).await.unwrap();
    handle.await.unwrap();

    match event_rx.recv().await.expect("should receive event") {
        AdapterEvent::AdapterError { device_key, error } => {
            assert!(device_key.is_none());
            assert!(error.contains("serial port disconnected"));
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }
}

#[tokio::test]
async fn bytes_channel_close_produces_adapter_error() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("test".into(), bytes_rx, event_tx, command_rx));

    drop(bytes_tx); // reader thread が死んだシミュレーション
    handle.await.unwrap();

    match event_rx.recv().await.expect("should receive event") {
        AdapterEvent::AdapterError { error, .. } => {
            assert!(error.contains("exited unexpectedly"));
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }
}

#[tokio::test]
async fn normal_data_flow_produces_sensor_data_and_device_discovered() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx));

    // Temperature frame (sensor_type=261), mcp9600 uart payload: Float32LE 22.4375
    let device: u64 = 0x246880020140018b;
    let frame_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &0x00_80_b3_41u32.to_be_bytes());
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    // 初回デバイスなので DeviceDiscovered が先に届く
    match event_rx.recv().await.expect("should receive DeviceDiscovered") {
        AdapterEvent::DeviceDiscovered { device_key, identity } => {
            assert_eq!(device_key.0, "246880020140018b");
            assert_eq!(identity.manufacturer, "Braveridge");
            assert_eq!(identity.ic_part_number, "MCP9600");
        }
        other => panic!("expected DeviceDiscovered, got {:?}", other),
    }

    // 次に SensorData が届く
    match event_rx.recv().await.expect("should receive SensorData") {
        AdapterEvent::SensorData { device_key, reading, .. } => {
            assert_eq!(device_key.0, "246880020140018b");
            assert_eq!(reading.sensor_type, SensorType::Temperature);
            assert!((reading.values[0] - 22.4375).abs() < 0.01);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }

    // 同じデバイスの2回目 → DeviceDiscovered は来ない、SensorData のみ
    let frame_bytes2 = build_sensor_frame_bytes(device, 261, -55, 90, 1, &0x00_80_b3_41u32.to_be_bytes());
    bytes_tx.send(Ok(frame_bytes2)).await.unwrap();

    match event_rx.recv().await.expect("should receive SensorData") {
        AdapterEvent::SensorData { .. } => {} // OK
        other => panic!("expected SensorData, got {:?}", other),
    }

    // クリーンアップ
    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 2: Run the event_loop tests**

Run: `cargo test -p bravepi-adapter --test event_loop_test 2>&1`
Expected: 4 tests pass

- [ ] **Step 3: Run full test suite**

Run: `cargo test --workspace 2>&1 | grep "test result"`
Expected: all tests pass (44 + 4 + 2 = 50 tests total)

- [ ] **Step 4: Commit**

```bash
git add bravepi-adapter/tests/event_loop_test.rs
git commit -m "test(adapter): add event_loop async integration tests

4 scenarios: shutdown, bytes error, bytes close, normal data flow.
Verifies DeviceDiscovered + SensorData ordering for new devices."
```

---

### Task 6: Update PoC binary for new API

**Files:**
- Modify: `bravepi-adapter/poc/src/main.rs`

- [ ] **Step 1: Update PoC to use shutdown on Ctrl+C**

Replace `bravepi-adapter/poc/src/main.rs`:

```rust
//! Phase 1 PoC — channel ベースの adapter-core 境界検証。
//!
//! BravePI adapter を async task として起動し、
//! core 側は AdapterEvent を受信して表示するだけの最小ループ。

use bravepi_adapter::task;
use iotkit_core_types::AdapterEvent;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let port_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/dev/ttyAMA0".to_string());

    tracing::info!(port = %port_path, "Phase 1 PoC: channel-based adapter-core boundary");

    let mut handle = match task::start(port_path) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "Failed to start BravePI adapter");
            std::process::exit(1);
        }
    };

    tracing::info!(adapter_id = %handle.id, "Adapter started, listening for events...");

    // core 側の最小受信ループ (Ctrl+C で graceful shutdown)
    loop {
        tokio::select! {
            biased;

            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Ctrl+C received, shutting down...");
                if let Err(e) = handle.shutdown().await {
                    tracing::error!(error = %e, "Shutdown error");
                }
                break;
            }
            event = handle.event_rx.recv() => {
                match event {
                    Some(AdapterEvent::SensorData { device_key, reading, rssi, battery_pct }) => {
                        tracing::info!(
                            device = %device_key,
                            sensor_type = %reading.sensor_type,
                            values = ?reading.values,
                            rssi = ?rssi,
                            battery = ?battery_pct,
                            "SensorData"
                        );
                    }
                    Some(AdapterEvent::DeviceDiscovered { device_key, identity }) => {
                        tracing::info!(
                            device = %device_key,
                            manufacturer = %identity.manufacturer,
                            ic = %identity.ic_part_number,
                            sensor_type = %identity.sensor_type,
                            "DeviceDiscovered"
                        );
                    }
                    Some(AdapterEvent::DeviceLost { device_key, reason }) => {
                        tracing::warn!(device = %device_key, reason = %reason, "DeviceLost");
                    }
                    Some(AdapterEvent::AdapterError { device_key, error }) => {
                        tracing::error!(device = ?device_key, error = %error, "AdapterError");
                    }
                    None => {
                        tracing::info!("Event channel closed, exiting");
                        break;
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Run cargo check on poc**

Run: `cargo check -p bravepi-poc 2>&1`
Expected: compiles with no errors

- [ ] **Step 3: Run full test suite**

Run: `cargo test --workspace 2>&1 | grep "test result"`
Expected: all 50 tests pass

- [ ] **Step 4: Commit**

```bash
git add bravepi-adapter/poc/src/main.rs
git commit -m "feat(poc): graceful shutdown with Ctrl+C, log DeviceDiscovered"
```
