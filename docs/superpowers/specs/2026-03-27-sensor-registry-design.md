# Sub-project C: Sensor Registry / Dispatch — 設計 Spec

## 目的

センサー追加の散在した変更点を registry に集約する。
新センサー追加を「sensors/ に 1 ファイル + registry に 1 entry + core に SensorType variant」に限定する。

これにより:
- `convert.rs` の大きな match 文が消え、lookup + event assembly だけになる
- `sensor_type_from_bravepi_raw()` と `device_key_suffix()` が registry に吸収される
- ContactInput/ContactOutput が他の IC センサーと同じ形式で扱われる
- sensors crate が adapter 非依存になり、将来の別 adapter や direct I2C に流用しやすくなる

## 設計判断

### 2 層構成: sensor handler と protocol mapping

sensor の decode 知識と BravePI プロトコルの番号体系は別の関心事として分離する。

- **sensors crate** (`bravepi-adapter/sensors/`): `SensorHandler` を定義・export する。
  sensor/endpoint の decode と identity 生成の知識を持つ。BravePI の raw code は知らない。
- **adapter crate** (`bravepi-adapter/src/`): raw_sensor_type → SensorHandler の対応表を持つ。
  BravePI プロトコル固有の番号体系はここだけに閉じる。

```
                       sensors crate                adapter crate
                  ┌──────────────────────┐    ┌─────────────────────────┐
                  │ SensorHandler        │    │ BRAVEPI_REGISTRY        │
                  │   sensor_type        │    │   raw: u16 → &Handler   │
                  │   key_suffix         │◄───│                         │
                  │   identity(conn)     │    │ convert.rs              │
                  │   decode_uart(sample) │    │   lookup + event asm   │
                  └──────────────────────┘    └─────────────────────────┘
```

### SensorHandler (sensors crate)

```rust
// bravepi-adapter/sensors/src/lib.rs

/// UART デコードの入力。payload + data_count を含む。
pub struct UartSample<'a> {
    pub payload: &'a [u8],
    pub data_count: u16,
}

/// センサー/endpoint の decode と identity 生成を 1 つにまとめた descriptor。
pub struct SensorHandler {
    pub sensor_type: SensorType,
    pub key_suffix: &'static str,
    pub identity: fn(ConnectionInfo) -> SensorIdentity,
    pub decode_uart: fn(UartSample<'_>) -> SensorReading,
}
```

設計ポイント:
- `identity` と `decode_uart` は分離した関数ポインタ。identity は payload 非依存。
- `decode_uart` の失敗は `SensorReading::empty(...)` で表現 (現状維持)。
  decode error semantics の変更は Sub-project C のスコープ外。
- `UartSample` に `data_count` を含めることで ContactInput/ContactOutput も
  同じ `decode_uart` シグネチャに乗る。
- trait ではなく struct + fn pointer。今のモジュールは全て stateless な純粋関数なので、
  trait object や ZST の儀式は不要。

### 各 sensor module の変更

各モジュール (`mcp9600.rs`, `opt3001.rs`, etc.) は:
- 既存の `pub fn identity()` と `pub fn from_uart_payload()` はそのまま残す
- `decode_uart` 用の名前付き wrapper 関数を追加する
- `pub const HANDLER: SensorHandler` を追加する

`from_uart_payload` の既存シグネチャ `fn(&[u8]) -> SensorReading` は
`decode_uart` の `fn(UartSample) -> SensorReading` と異なるため、
名前付き wrapper 関数を各モジュールに追加する。closure ではなく名前付き関数にすることで
可読性が高く、data_count を使う handler (contact) との見た目も揃う。

```rust
// mcp9600.rs (追加)

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: SensorHandler = SensorHandler {
    sensor_type: SensorType::Temperature,
    key_suffix: "temperature",
    identity: identity,
    decode_uart: decode_uart,
};
```

IC 固有の I2C 関数 (`from_i2c_raw`, レジスタ定数, etc.) は影響なし。

### ContactInput/ContactOutput の統合

`contact.rs` を sensors crate に新規追加する。

```rust
// bravepi-adapter/sensors/src/contact.rs

// 共通の値変換ロジック (bytes → 0.0/1.0)
fn decode_values(sample: &UartSample<'_>) -> Vec<f64> {
    sample.payload.iter()
        .take(sample.data_count as usize)
        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
        .collect()
}

fn decode_contact_input(sample: UartSample<'_>) -> SensorReading {
    SensorReading::new(SensorType::ContactInput, decode_values(&sample), vec![])
}

fn decode_contact_output(sample: UartSample<'_>) -> SensorReading {
    SensorReading::new(SensorType::ContactOutput, decode_values(&sample), vec![])
}

pub const CONTACT_INPUT: SensorHandler = SensorHandler {
    sensor_type: SensorType::ContactInput,
    key_suffix: "contact_input",
    identity: contact_input_identity,
    decode_uart: decode_contact_input,
};

pub const CONTACT_OUTPUT: SensorHandler = SensorHandler {
    sensor_type: SensorType::ContactOutput,
    key_suffix: "contact_output",
    identity: contact_output_identity,
    decode_uart: decode_contact_output,
};
```

現在 `convert.rs` にある inline の decode ロジック (bytes → 0.0/1.0 変換) と
`contact_identity()` を `contact.rs` に移動する。

ContactInput と ContactOutput は SensorType が異なる 2 つの handler として扱う。
IC driver ではなく endpoint decoder として位置づける。

### BravePI Registry (adapter crate)

```rust
// bravepi-adapter/src/registry.rs

use bravepi_sensors::SensorHandler;

struct BravepiRegistryEntry {
    raw_sensor_type: u16,
    handler: &'static SensorHandler,
}

static BRAVEPI_REGISTRY: &[BravepiRegistryEntry] = &[
    BravepiRegistryEntry { raw_sensor_type: 257, handler: &bravepi_sensors::contact::CONTACT_INPUT },
    BravepiRegistryEntry { raw_sensor_type: 258, handler: &bravepi_sensors::contact::CONTACT_OUTPUT },
    BravepiRegistryEntry { raw_sensor_type: 259, handler: &bravepi_sensors::mcp3427::HANDLER },
    BravepiRegistryEntry { raw_sensor_type: 260, handler: &bravepi_sensors::vl53l1x::HANDLER },
    BravepiRegistryEntry { raw_sensor_type: 261, handler: &bravepi_sensors::mcp9600::HANDLER },
    BravepiRegistryEntry { raw_sensor_type: 262, handler: &bravepi_sensors::lis2duxs12::HANDLER },
    BravepiRegistryEntry { raw_sensor_type: 263, handler: &bravepi_sensors::sdp810::HANDLER },
    BravepiRegistryEntry { raw_sensor_type: 264, handler: &bravepi_sensors::opt3001::HANDLER },
];

pub fn lookup_handler(raw: u16) -> Option<&'static SensorHandler> {
    BRAVEPI_REGISTRY.iter()
        .find(|e| e.raw_sensor_type == raw)
        .map(|e| e.handler)
}
```

実装は static 配列 + linear scan。8 エントリなので HashMap は不要。
依存も初期化も不要で、const に寄せやすい。

### convert.rs の変形

`frame_to_event()` は registry lookup + event assembly だけになる。

```rust
// bravepi-adapter/src/task/convert.rs (変更後)

pub(crate) fn frame_to_event(
    frame: BravePiFrame,
    port_path: &str,
) -> Option<(AdapterEvent, Option<SensorIdentity>)> {
    match frame {
        BravePiFrame::Sensor(s) => {
            let handler = lookup_handler(s.sensor_type_raw).or_else(|| {
                tracing::warn!(raw = s.sensor_type_raw, "Unknown sensor type, skipping");
                None
            })?;

            let transmitter_id = s.device_number.clone();
            let device_key = DeviceKey::new(
                format!("bravepi:{}:{}", transmitter_id, handler.key_suffix),
            );

            let conn_info = BravepiConnection::Uart {
                port: port_path.to_string(),
                transmitter_id,
            }
            .to_connection_info();

            let sample = UartSample {
                payload: &s.value_data,
                data_count: s.data_count,
            };
            let reading = (handler.decode_uart)(sample);
            let identity = (handler.identity)(conn_info);

            let event = AdapterEvent::SensorData {
                device_key,
                reading,
                rssi: Some(s.rssi as i16),
                battery_pct: Some(s.battery),
            };

            Some((event, Some(identity)))
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
        } => {
            let device_key = if device_number == "unknown" {
                None
            } else {
                lookup_handler(sensor_type_raw).map(|h| {
                    DeviceKey::new(format!("bravepi:{}:{}", device_number, h.key_suffix))
                })
            };
            Some((
                AdapterEvent::AdapterError {
                    device_key,
                    error: format!("Decode error (type={}): {}", sensor_type_raw, reason),
                },
                None,
            ))
        }
    }
}
```

### 削除される関数

- `bravepi-adapter/src/lib.rs` の `sensor_type_from_bravepi_raw()` → registry に吸収
- `bravepi-adapter/src/task/convert.rs` の `device_key_suffix()` → handler.key_suffix に吸収
- `bravepi-adapter/src/task/convert.rs` の `contact_identity()` → contact.rs に移動

## ファイル構成と変更範囲

### 新規

- `bravepi-adapter/sensors/src/contact.rs`
  - ContactInput / ContactOutput の handler 定義
  - decode_uart (bytes → 0.0/1.0 変換) と identity 生成

### 変更

- `bravepi-adapter/sensors/src/lib.rs`
  - `UartSample` struct 追加
  - `SensorHandler` struct 追加
  - `pub mod contact;` 追加
- `bravepi-adapter/sensors/src/mcp9600.rs`
  - `pub const HANDLER: SensorHandler` 追加
- `bravepi-adapter/sensors/src/opt3001.rs`
  - `pub const HANDLER: SensorHandler` 追加
- `bravepi-adapter/sensors/src/mcp3427.rs`
  - `pub const HANDLER: SensorHandler` 追加
- `bravepi-adapter/sensors/src/vl53l1x.rs`
  - `pub const HANDLER: SensorHandler` 追加
- `bravepi-adapter/sensors/src/sdp810.rs`
  - `pub const HANDLER: SensorHandler` 追加
- `bravepi-adapter/sensors/src/lis2duxs12.rs`
  - `pub const HANDLER: SensorHandler` 追加
- `bravepi-adapter/src/registry.rs` (新規)
  - BravePI registry (raw code → handler 対応表)
  - `pub(crate) fn lookup_handler()`
- `bravepi-adapter/src/lib.rs`
  - `pub(crate) mod registry;` 追加
  - `sensor_type_from_bravepi_raw()` 削除
- `bravepi-adapter/src/task/convert.rs`
  - match 文を registry lookup に置換
  - `device_key_suffix()` 削除
  - `contact_identity()` 削除

### テスト変更

- `bravepi-adapter/src/task/convert_test.rs`
  - 既存テストの振る舞いは全て維持 (regression guard)
  - dispatch 経路が変わるだけで、入出力は同一
- `bravepi-adapter/sensors/src/contact.rs`
  - contact decode の unit test 追加 (convert_test.rs から移動)
- 各 sensor module の既存テストは変更なし

### 変更なし

- `core/types/src/lib.rs`
- `bravepi-adapter/codec/` 全体
- `bravepi-adapter/src/transport.rs`
- `bravepi-adapter/src/task/serial_source.rs`
- `bravepi-adapter/src/task/handle.rs`
- `bravepi-adapter/src/task/event_loop.rs`
- `bravepi-adapter/src/task/event_loop_test.rs`

### スコープ外

- decode error semantics の変更 (Result/Option 化) — 別 sub-project
- I2C handler の追加 (decode_i2c) — sensors crate は対応可能だが今は不要
- SensorType enum への variant 追加 — 新センサー追加時に必要だが C のスコープではない
- DeviceLost / timeout-based lifecycle — Sub-project B の後続
