# Phase 1A: Adapter Standalone — Design Spec

Date: 2026-03-29
Status: Draft
Issues: #40, #41, #42, #31

## Goal

rpi-local-adapter を独立バイナリとしてパッケージングし、I2C センサー読み取り → MQTT publish が end-to-end で動く状態にする。既存の gateway 組み込みモードは変更しない。

## Architecture

```
┌─────────────────────────────────────────────┐
│ iotkit-rpi-local (binary crate)             │
│                                             │
│  TOML config → rpi-local-adapter::start()   │
│                    │                        │
│                event_rx                     │
│                    │                        │
│           adapter-runner::run()             │
│              │            │                 │
│         envelope      MQTT client           │
│         conversion    (rumqttc)             │
│              │            │                 │
│         mqtt-contract  publish              │
└─────────────────────────────────────────────┘
        │
        ▼ MQTT
   Any broker (Mosquitto, AWS IoT, HiveMQ, etc.)
```

### Crate Structure

| Crate | Type | 責務 |
|-------|------|------|
| `core/mqtt-contract` | lib | MQTT topic schema + JSON envelope DTOs + AdapterEvent 変換 |
| `iotkit-adapter-runner` | lib | 共通 edge runtime: MQTT client, LWT, signal handling, event publish loop |
| `iotkit-rpi-local` | bin | rpi-local-adapter を standalone 実行する composition root |

依存関係: `iotkit-rpi-local` → `iotkit-adapter-runner` → `core/mqtt-contract` → `core/types`

既存 crate (`iotkit-polling-adapter-runtime`, `core/types`, `iotkit-gateway`) は変更しない。`rpi-local-adapter` は adapter_id を外部から受け取れるよう `start()` の signature を拡張する（後述）。

## 1. MQTT Event Envelope (core/mqtt-contract)

### Topic Schema

```
iotkit/v1/{adapter_id}/telemetry     # SensorData readings
iotkit/v1/{adapter_id}/discovery     # DeviceDiscovered
iotkit/v1/{adapter_id}/loss          # DeviceLost
iotkit/v1/{adapter_id}/error         # AdapterError
iotkit/v1/{adapter_id}/status        # online/offline (retained, QoS 1)
iotkit/v1/{adapter_id}/inventory/{device_key}  # per-device discovery (retained)
```

`{adapter_id}` は MQTT topic-safe に percent-encoding でエスケープ（`:` → `%3A`, `/` → `%2F` 等）。可逆変換のため topic から元の adapter_id を復元可能。元の adapter_id は envelope 内にも保持。

### Envelope Format

全 envelope に共通ヘッダ:

```json
{
  "v": 1,
  "adapter_id": "rpi-local:default",
  "ts": 1711700000000
}
```

- `v`: envelope version。将来の互換性のため。consumer は未知の `v` を無視する。
- `ts`: envelope 生成時刻 (unix ms)。telemetry の `ingested_at` とは別。

#### Telemetry

```json
{
  "v": 1,
  "adapter_id": "rpi-local:default",
  "ts": 1711700000000,
  "device_key": "i2c:0x60:mcp9600",
  "sensor_type": "temperature",
  "ingested_at": 1711700000000,
  "values": [25.3],
  "labels": ["temperature_c"],
  "rssi": null,
  "battery_pct": null
}
```

#### Discovery

```json
{
  "v": 1,
  "adapter_id": "rpi-local:default",
  "ts": 1711700000000,
  "device_key": "i2c:0x60:mcp9600",
  "identity": {
    "manufacturer": "Microchip",
    "ic_part_number": "MCP9600",
    "sensor_type": "temperature",
    "connection": {
      "kind": "i2c",
      "parameters": {"address": "0x60", "bus_path": "/dev/i2c-1"}
    }
  }
}
```

#### Loss

```json
{
  "v": 1,
  "adapter_id": "rpi-local:default",
  "ts": 1711700000000,
  "device_key": "i2c:0x60:mcp9600",
  "reason": "5 consecutive read failures"
}
```

#### Error

```json
{
  "v": 1,
  "adapter_id": "rpi-local:default",
  "ts": 1711700000000,
  "device_key": "i2c:0x60:mcp9600",
  "error": "I2C bus error: NACK"
}
```

`device_key` は null 可（adapter-level error の場合）。

#### Status (retained)

```json
{
  "v": 1,
  "adapter_id": "rpi-local:default",
  "ts": 1711700000000,
  "online": true
}
```

LWT payload: `{"v":1,"adapter_id":"rpi-local:default","ts":0,"online":false}` (`ts: 0` は LWT では設定時刻が不明なため)

#### Inventory Recovery

Discovery message は **retained** として per-device topic に publish する:
- Topic: `iotkit/v1/{adapter_id}/inventory/{device_key}` (percent-encoded)
- Payload: Discovery envelope と同一 JSON (retained, QoS 1)
- DeviceLost 時: 該当 device の inventory topic に empty retained message を publish（broker から削除）
- MQTT reconnect 時: adapter は全 active device の discovery を再 publish する

これにより、subscriber が後から接続しても retained message から全 active device の inventory を復元できる。通常の `discovery` topic は non-retained のイベント通知として残す（リアルタイム通知用）。

**Design Decision:** per-device retained topic を使う理由は、adapter が device 単位で inventory を更新・削除でき、subscriber が wildcard subscribe (`iotkit/v1/+/inventory/+`) で全 adapter の全 device を取得できるため。adapter-level の一括 inventory snapshot だと、1 device の追加/削除で全体を再 publish する必要がある。10 devices なら問題ないが、将来 100+ devices で不必要に大きい retained message になる。

### Design Decisions

**なぜ JSON か:** デバッグしやすい。MessagePack 等のバイナリフォーマットは throughput が問題になってから検討する。10 sensors × 1Hz = 10 msg/sec、JSON 1msg ≈ 200 bytes → 2KB/sec。Mosquitto のローカル throughput は 10,000+ msg/sec なので headroom 99.9%以上。

**なぜ QoS 1 か:** at-least-once delivery。QoS 0 はネットワーク不安定時にデータロスし、QoS 2 はオーバーヘッドが大きい。gateway 側で idempotent write（ON CONFLICT DO NOTHING）が既にあるため、重複配信は安全。

**なぜ adapter_id を topic に含めるか:** gateway が adapter 単位で subscribe/filter できる。wildcard subscribe (`iotkit/v1/+/telemetry`) で全 adapter を受信可能。

**Rejected: per-device topic for telemetry (`iotkit/v1/{adapter_id}/{device_key}/telemetry`):** topic 数が sensor 数に比例して増える。10 sensors なら問題ないが、将来 100+ sensors で broker の topic tree が膨張する。adapter 単位でまとめる方がスケーラブル。ただし inventory は per-device retained topic を使う（inventory recovery のため）。

**adapter_id の source of truth:** TOML config の `adapter_id` が唯一の source of truth。rpi-local-adapter の `start()` に `adapter_id` を渡せるよう signature を拡張する。既存の gateway 組み込みモードでは従来通り内部で生成する（backward compatible に optional parameter として追加）。

**Rejected: adapter handle から adapter_id を取得:** 現在の `start()` が内部で hardcode しているが、standalone 運用では config から制御可能にする必要がある。handle 内の ID と MQTT publish の ID が一致しないリスクを排除するため、config を single source of truth とする。

**encode/decode の対象 event:** v1 では `SensorData`, `DeviceDiscovered`, `DeviceLost`, `AdapterError` の 4 event type を encode/decode 対象とする。`DeviceConfig` は Phase 2 の command bridge (#46) で扱い、Phase 1A では encode 対象外と明記する。telemetry envelope には `labels` フィールドを含め、SensorReading の完全な round-trip を保証する。

### Public API

```rust
// core/mqtt-contract/src/lib.rs

/// Build MQTT topic string for a given event type
pub fn topic(adapter_id: &AdapterId, event_type: EventType) -> String;

/// Event types for topic routing
pub enum EventType {
    Telemetry,
    Discovery,
    Loss,
    Error,
    Status,
}

/// Convert AdapterEvent to MQTT envelope bytes (JSON)
pub fn encode_event(adapter_id: &AdapterId, event: &AdapterEvent) -> Result<(EventType, Vec<u8>), EncodeError>;

/// Decode MQTT envelope bytes back to AdapterEvent (for gateway subscriber)
pub fn decode_event(event_type: EventType, payload: &[u8]) -> Result<(AdapterId, AdapterEvent), DecodeError>;

/// Encode status message
pub fn encode_status(adapter_id: &AdapterId, online: bool) -> Vec<u8>;

/// Decode status message
pub fn decode_status(payload: &[u8]) -> Result<(AdapterId, bool), DecodeError>;
```

Dependencies: `core/types`, `serde`, `serde_json`.

## 2. Standalone Adapter Runner (iotkit-adapter-runner)

### Responsibilities

1. MQTT client lifecycle: connect, reconnect (exponential backoff 1s→30s, jitter), keepalive
2. LWT 設定: broker 接続時に offline status を LWT として登録
3. 起動時に online status publish (retained, QoS 1)
4. event_rx → mqtt-contract::encode_event → MQTT publish (QoS 1)
5. DeviceDiscovered → inventory topic に retained publish。DeviceLost → inventory topic の retained message を削除（empty retained publish）
6. MQTT reconnect 時: 全 active device の discovery を inventory topic に再 publish
7. Signal handling: SIGTERM/SIGINT → offline status publish → MQTT disconnect → process exit
8. Connection loss handling: reconnect loop、再接続中は event を drop して warn! ログ

### Concurrency Model

rumqttc は `AsyncClient` (publish 用) と `EventLoop` (MQTT protocol pump) の 2つを返す。EventLoop は定期的に poll しないと keepalive や PUBACK が処理されない。

```
tokio::spawn(mqtt_eventloop_task)  ← EventLoop.poll() を無限ループ
tokio::spawn(publish_task)         ← event_rx → encode → client.publish()
main task: tokio::signal::ctrl_c() を待機
```

**publish_task と mqtt_eventloop_task は独立した tokio task として実行する。** `tokio::select!` で 1つの task に混ぜない。これにより:
- telemetry の burst が eventloop の poll を starve しない
- eventloop の処理遅延が event_rx の drain を block しない
- signal handling は main task で独立して動く

reconnect は eventloop task 内で処理。publish_task は `client.publish()` が Err を返したら warn! + drop。

### Config

```rust
pub struct MqttConfig {
    pub broker_url: String,         // mqtt:// or mqtts://
    pub client_id: Option<String>,  // auto-generated if None: "iotkit-{adapter_id}-{random}"
    pub keepalive_secs: u32,        // default: 30
    pub ca_path: Option<PathBuf>,   // TLS CA cert
    pub client_cert_path: Option<PathBuf>,  // mTLS client cert
    pub client_key_path: Option<PathBuf>,   // mTLS client key
}
```

### Public API

```rust
// iotkit-adapter-runner/src/lib.rs

/// Run the adapter event loop: receive events, publish to MQTT
/// Blocks until signal received or fatal error.
pub async fn run(
    adapter_id: AdapterId,
    mqtt_config: MqttConfig,
    event_rx: mpsc::Receiver<AdapterEvent>,
) -> Result<(), RunnerError>;
```

Dependencies: `core/mqtt-contract`, `core/types`, `rumqttc`, `tokio`, `tracing`.

### Design Decisions

**なぜ event drop on disconnect か:** adapter は sensor polling を止めない（データは engine の live state にも使われうる）。MQTT が切れている間の event をバッファすると OOM リスクがある。RPi Zero 2W (512MB) で 10 sensors × 1Hz × 30s disconnect = 300 events ≈ 60KB なので小さいが、長時間 disconnect では危険。Drop + warn! が最も安全。将来のバージョンで bounded buffer (1000 events) を追加可能。

**Rejected: local disk buffer:** ファイルへの書き出しは I/O 負荷が増え、RPi Zero 2W の SD カード寿命に影響する。MQTT QoS 1 + gateway の idempotent write で十分。

**なぜ exponential backoff with jitter か:** 複数 adapter が同時に reconnect すると broker に thundering herd が発生する。jitter で分散。初回 1s、最大 30s、jitter ±30%。

### Error Handling

- MQTT connect failure → exponential backoff retry、tracing::warn!
- MQTT publish failure → warn! + drop event (QoS 1 なので rumqttc が内部 retry するが、persistent failure は drop)
- TLS cert load failure → 起動時に即 error exit
- event_rx closed (adapter crashed) → offline publish → exit

## 3. rpi-local Standalone Binary (iotkit-rpi-local)

### Config (TOML)

```toml
adapter_id = "rpi-local:default"

[mqtt]
broker_url = "mqtt://localhost:1883"
# client_id = "iotkit-rpi-local-01"  # optional
# keepalive_secs = 30
# ca_path = "/etc/iotkit/certs/ca.pem"
# client_cert_path = "/etc/iotkit/certs/client.pem"
# client_key_path = "/etc/iotkit/certs/client.key"

[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapter.targets]]
driver = "mcp9600"
address = 0x60
thermocouple_type = "K"

[[adapter.targets]]
driver = "opt3001"
address = 0x44
```

### Binary Entrypoint

```rust
// iotkit-rpi-local/src/main.rs (pseudo)
fn main() {
    // 1. Init tracing
    // 2. Parse CLI args (--config path)
    // 3. Load + validate TOML config
    // 4. Build RpiLocalConfig from adapter section
    // 5. Validate adapter config (rpi_local_adapter::validate)
    // 6. Create tokio runtime
    // 7. rt.block_on(async {
    //      Start adapter (rpi_local_adapter::start) → Handle with event_rx
    //      Run adapter_runner::run(adapter_id, mqtt_config, event_rx)
    //    })
    // 8. On return: shutdown adapter handle
}
// Note: rpi_local_adapter::start() requires a live tokio runtime,
// so runtime creation MUST precede adapter start.
```

### Config Validation

- `adapter_id`: non-empty
- `mqtt.broker_url`: non-empty, starts with `mqtt://` or `mqtts://`
- `mqtt.keepalive_secs`: > 0 (if specified)
- `adapter.*`: delegated to `rpi_local_adapter::validate()`
- TLS: if `mqtts://`, at least `ca_path` must be specified

### CLI

```
iotkit-rpi-local --config /path/to/config.toml
iotkit-rpi-local --help
iotkit-rpi-local --version
```

`--config` のデフォルト: `./iotkit-rpi-local.toml` → `/etc/iotkit/iotkit-rpi-local.toml`

Dependencies: `iotkit-adapter-runner`, `rpi-local-adapter`, `toml`, `clap`, `tracing`, `tracing-subscriber`, `tokio`.

## 4. Deploy (#31)

### Directory Layout

```
/opt/iotkit/
├── bin/iotkit-rpi-local
├── etc/
│   ├── iotkit-rpi-local.toml
│   └── certs/                    # optional TLS certs
└── data/                         # future local storage
```

### systemd Unit

```ini
[Unit]
Description=iotkit rpi-local I2C sensor adapter
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/opt/iotkit/bin/iotkit-rpi-local --config /opt/iotkit/etc/iotkit-rpi-local.toml
Restart=on-failure
RestartSec=5
User=iotkit
Group=iotkit
WorkingDirectory=/opt/iotkit/data
StandardOutput=journal
StandardError=journal

# Security hardening
ProtectSystem=strict
ReadWritePaths=/opt/iotkit/data
NoNewPrivileges=true
ProtectHome=true

# I2C access
SupplementaryGroups=i2c

[Install]
WantedBy=multi-user.target
```

### Design Decisions

**なぜ `User=iotkit` か:** root で動かすとセキュリティリスク。`iotkit` ユーザーを作成し、`i2c` グループに追加して I2C bus アクセスを許可。

**なぜ `network-online.target` か:** MQTT broker 接続が必要。WiFi 接続完了を待つ。adapter runner の reconnect で最終的にはつながるが、不要な reconnect ログを減らせる。

## 5. Test Strategy

### mqtt-contract
- serde round-trip: encode → decode → assert equality (全 event type)
- topic builder: adapter_id のエスケープ、各 event type の topic 生成
- unknown version handling: `v: 99` の envelope → DecodeError

### adapter-runner
- unit test: event_rx → envelope 変換の正しさ
- integration test: rumqttc の test broker は外部依存が必要なため、MQTT client は trait behind で mock 可能にする。ただし v1 では concrete rumqttc で実装し、mock は後回し。
- signal handling: テスト困難。手動テスト手順を README に記載。

### rpi-local binary
- config parse: valid TOML → RpiLocalStandaloneConfig
- config validation: invalid cases (empty broker_url, missing bus_path, etc.)
- adapter validation: 既存の `rpi_local_adapter::validate()` に委譲

### end-to-end (手動)
1. `mosquitto -c /dev/null -v` でローカル broker 起動
2. `mosquitto_sub -t 'iotkit/v1/#' -v` で subscribe
3. `iotkit-rpi-local --config example.toml` で adapter 起動
4. telemetry/discovery/status メッセージが受信されることを確認
5. Ctrl+C → offline status が publish されることを確認

## 6. Quantitative Targets

| Metric | Target | Rationale |
|--------|--------|-----------|
| MQTT publish latency (local broker) | < 5ms per message | rumqttc async publish、ローカル loopback |
| Memory footprint (rpi-local binary) | < 20MB RSS | RPi Zero 2W の 512MB の 4% 以下 |
| Startup to first publish | < 3s | adapter detect + init + first read + MQTT connect |
| Reconnect backoff range | 1s → 30s | exponential with ±30% jitter |
| Event throughput | 100 msg/sec sustainable | 10× headroom over 10 sensors × 1Hz |

## 7. Out of Scope

- Gateway MQTT subscriber (#45) — Phase 2
- DeviceKey bus identity change (#33) — Phase 2。現行の `i2c:0x{addr}:{suffix}` フォーマットのまま
- Transform layer (#43, #44) — Phase 1B
- BravePI standalone adapter (#46) — Phase 2
- Local disk buffer for MQTT disconnect — 将来検討
- MQTT v5 features — v1 は MQTT v3.1.1
- Auto-detection (#35) — Phase 2
