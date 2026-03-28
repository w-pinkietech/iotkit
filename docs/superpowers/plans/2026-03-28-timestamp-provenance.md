# Timestamp/Provenance 追加 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** AdapterEvent::SensorData と DeviceConfig に壁時計タイムスタンプ (ingested_at) を追加し、Engine の DeviceView に last_reading_at として公開する。

**Architecture:** core/types の AdapterEvent variant にフィールド追加 → Engine state で保存 → 各 adapter で生成時刻を付与。変更は内側 (types) から外側 (adapters) へ。

**Tech Stack:** Rust, std::time::SystemTime, tokio

---

### Task 1: core/types に ingested_at フィールド追加

**Files:**
- Modify: `core/types/src/lib.rs`

- [ ] **Step 1: AdapterEvent::SensorData に ingested_at 追加**

`core/types/src/lib.rs` の `AdapterEvent::SensorData` variant に `ingested_at: std::time::SystemTime` を追加:

```rust
/// センサーデータ受信。
SensorData {
    device_key: DeviceKey,
    reading: SensorReading,
    rssi: Option<i16>,
    battery_pct: Option<u8>,
    ingested_at: std::time::SystemTime,
},
```

- [ ] **Step 2: AdapterEvent::DeviceConfig に ingested_at 追加**

同ファイルの `AdapterEvent::DeviceConfig` variant に追加:

```rust
/// デバイス設定の応答。QueryConfig の結果として非同期に返る。
DeviceConfig {
    device_key: DeviceKey,
    config: DeviceConfigData,
    ingested_at: std::time::SystemTime,
},
```

- [ ] **Step 3: 同ファイル内のテストを更新**

`adapter_event_device_config_variant` テストの AdapterEvent::DeviceConfig 構築に `ingested_at: std::time::SystemTime::now()` を追加。match arm のパターンにも `ingested_at: _` を追加。

- [ ] **Step 4: cargo check で core/types のコンパイルエラー確認**

Run: `cargo check -p iotkit-core-types 2>&1`
Expected: PASS (types 単体は通る)

Run: `cargo check 2>&1 | head -50`
Expected: 他の crate でコンパイルエラー (ingested_at が足りない)。これは後続 Task で修正する。

- [ ] **Step 5: Commit**

```bash
git add core/types/src/lib.rs
git commit -m "feat(core-types): add ingested_at to SensorData and DeviceConfig"
```

---

### Task 2: Engine に last_reading_at を追加

**Files:**
- Modify: `core/engine/src/lib.rs`
- Modify: `core/engine/src/state.rs`
- Modify: `core/engine/src/state_test.rs`

- [ ] **Step 1: DeviceView に last_reading_at 追加**

`core/engine/src/lib.rs` の DeviceView に追加:

```rust
pub struct DeviceView {
    pub key: EngineDeviceKey,
    pub identity: SensorIdentity,
    pub last_reading: Option<SensorReading>,
    pub last_reading_at: Option<std::time::SystemTime>,  // NEW
    pub rssi: Option<i16>,
    pub battery_pct: Option<u8>,
    pub config: Option<DeviceConfigData>,
    pub last_error: Option<String>,
}
```

- [ ] **Step 2: State::apply で ingested_at を保存**

`core/engine/src/state.rs` の `SensorData` arm を更新:

```rust
AdapterEvent::SensorData { device_key, reading, rssi, battery_pct, ingested_at } => {
    let key = EngineDeviceKey {
        adapter_id,
        device_key: device_key.clone(),
    };
    match self.devices.get_mut(&key) {
        Some(ds) => {
            ds.view.last_reading = Some(reading);
            ds.view.last_reading_at = Some(ingested_at);
            ds.view.rssi = rssi;
            ds.view.battery_pct = battery_pct;
            ds.last_seen = Instant::now();
        }
        None => {
            tracing::warn!(
                device_key = %device_key,
                "SensorData for unknown device, ignoring"
            );
        }
    }
}
```

DeviceConfig arm も更新 (ingested_at を destructure するだけ、DeviceView には保存しない):

```rust
AdapterEvent::DeviceConfig { device_key, config, ingested_at: _ } => {
```

- [ ] **Step 3: apply_discovered で last_reading_at: None を初期化**

`state.rs` の `apply_discovered` で DeviceView 構築に `last_reading_at: None` を追加。

- [ ] **Step 4: state_test.rs のテストを更新**

全テストの `AdapterEvent::SensorData { .. }` 構築に `ingested_at: std::time::SystemTime::now()` を追加。
`AdapterEvent::DeviceConfig { .. }` 構築にも同様。

`sensor_data_updates_reading` テストに検証を追加:

```rust
assert!(view.last_reading_at.is_some(), "last_reading_at should be set");
```

- [ ] **Step 5: cargo test で engine テスト実行**

Run: `cargo test -p iotkit-core-engine 2>&1`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add core/engine/src/lib.rs core/engine/src/state.rs core/engine/src/state_test.rs
git commit -m "feat(engine): store ingested_at as last_reading_at in DeviceView"
```

---

### Task 3: BravePI adapter を更新

**Files:**
- Modify: `bravepi-mainboard-adapter/src/task/event_loop.rs`
- Modify: `bravepi-mainboard-adapter/src/task/handle.rs`

- [ ] **Step 1: event_loop.rs の SensorData 生成時に ingested_at 追加**

`frame_to_event` 呼び出し後、`event_tx.send(event)` の前に、event 内の SensorData に `ingested_at` を付与する必要がある。

`bravepi-mainboard-adapter/src/task/convert.rs` の `frame_to_event` が `AdapterEvent` を返すので、`convert.rs` 側で `ingested_at: std::time::SystemTime::now()` を含めるか、`event_loop.rs` 側で付与する。

**方針:** `convert.rs` の `frame_to_event` で `SensorData` を構築する箇所に `ingested_at: std::time::SystemTime::now()` を追加。ConfigFrame は `event_loop.rs` の `handle_config_frame` で生成するので、そこに追加。

まず `convert.rs` を確認して SensorData 構築箇所を特定し、`ingested_at` を追加。

- [ ] **Step 2: handle_config_frame の DeviceConfig に ingested_at 追加**

`event_loop.rs` の `handle_config_frame` 内:

```rust
event_tx.send(AdapterEvent::DeviceConfig {
    device_key,
    config,
    ingested_at: std::time::SystemTime::now(),
}).await.is_err()
```

- [ ] **Step 3: handle.rs のテスト更新**

`into_parts_preserves_id_and_channels` テストの `AdapterEvent::SensorData` 構築に `ingested_at: std::time::SystemTime::now()` を追加。

- [ ] **Step 4: cargo test で bravepi テスト実行**

Run: `cargo test -p bravepi-mainboard-adapter 2>&1`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add bravepi-mainboard-adapter/
git commit -m "feat(bravepi): add ingested_at timestamp to SensorData and DeviceConfig events"
```

---

### Task 4: Polling adapter runtime を更新

**Files:**
- Modify: `iotkit-polling-adapter-runtime/src/polling_loop.rs`
- Modify: `iotkit-polling-adapter-runtime/src/lib.rs`

- [ ] **Step 1: apply_outcomes の SensorData 生成に ingested_at 追加**

`polling_loop.rs` の `apply_outcomes` 内、`PollOutcome::Reading` arm:

```rust
PollOutcome::Reading { key, reading } => {
    // Reset read failure counter...
    events.push(AdapterEvent::SensorData {
        device_key: key,
        reading,
        rssi: None,
        battery_pct: None,
        ingested_at: std::time::SystemTime::now(),
    });
}
```

- [ ] **Step 2: lib.rs のテスト更新**

`into_parts_preserves_id_and_channels` テストの `AdapterEvent::SensorData` に `ingested_at: std::time::SystemTime::now()` を追加。

- [ ] **Step 3: cargo test で polling runtime テスト実行**

Run: `cargo test -p iotkit-polling-adapter-runtime 2>&1`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add iotkit-polling-adapter-runtime/
git commit -m "feat(polling-runtime): add ingested_at timestamp to SensorData events"
```

---

### Task 5: Gateway テスト更新 + ワークスペース全体検証

**Files:**
- Modify: `iotkit-gateway/src/adapter_host.rs`

- [ ] **Step 1: adapter_host.rs のテスト内 stub_event 更新**

`stub_event()` 関数の `AdapterEvent::SensorData` に `ingested_at: std::time::SystemTime::now()` を追加。

- [ ] **Step 2: ワークスペース全体の cargo test**

Run: `cargo test --workspace 2>&1`
Expected: ALL PASS

- [ ] **Step 3: Commit**

```bash
git add iotkit-gateway/src/adapter_host.rs
git commit -m "chore(gateway): update test stubs with ingested_at field"
```
