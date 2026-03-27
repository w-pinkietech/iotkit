# Sub-project D: Command / Query Boundary --- 設計 Spec

## 目的

core と adapter の間の command/query 公開契約を固定する。
BravePI の既存 downlink (ContactOutput, ImmediateUplink, ParameterGet) を対象に、
最小の boundary を定義する。

これにより:
- `AdapterCommand` が Shutdown 以外の device-targeted command を運べるようになる
- ParameterGet の応答が `AdapterEvent::DeviceConfig` として core に流れる
- adapter 固有の wire encoding が core に漏れない構造が確定する
- 将来の pair/scan/DFU を追加するときに boundary を壊さずに variant を足せる

## 設計判断

### AdapterCommand の拡張

```rust
// core/types/src/lib.rs

pub enum AdapterCommand {
    Shutdown,
    DeviceCommand(DeviceCommand),
}

pub struct DeviceCommand {
    pub device_key: DeviceKey,
    pub payload: DeviceCommandPayload,
}

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
```

設計ポイント:
- `DeviceCommand` は device-targeted command の共通 envelope。
  `device_key` は全 command で必須。将来 `request_id` や `issued_at` を
  足す場所として機能する。
- `DeviceCommandPayload` の variant 名は adapter 横断で意味が通る名前。
  BravePI 固有の opcode や wire 表現は含まない。
- `SetOutput` は `value: bool` + `duration_ms: Option<u32>`。
  BravePI の `signal_mode: u8` / `signal_out_time: u16` への変換は adapter の責務。
  Vec<f64> や multi-channel は現時点では不要。必要になったら後で広げる。
- `Shutdown` は adapter lifecycle command として `DeviceCommand` とは別に残す。

### AdapterEvent::DeviceConfig の追加

```rust
// core/types/src/lib.rs

pub enum AdapterEvent {
    SensorData { ... },
    DeviceDiscovered { ... },
    DeviceLost { ... },
    AdapterError { ... },

    /// デバイス設定の応答。QueryConfig の結果として非同期に返る。
    DeviceConfig {
        device_key: DeviceKey,
        config: DeviceConfigData,
    },
}

pub struct DeviceConfigData {
    pub firmware_version: Option<String>,
    pub uplink_interval_secs: Option<u32>,
    pub properties: BTreeMap<String, ConfigValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    String(String),
    Integer(i64),
    Float(f64),
    Bool(bool),
}
```

設計ポイント:
- `DeviceConfig` は `DeviceDiscovered` とは独立した event。
  discovery は「初回観測時に 1 回」、config は「QueryConfig の応答ごと」で lifecycle が異なる。
- `DeviceConfigData` は core の汎用 DTO。
  `firmware_version` と `uplink_interval_secs` は adapter 横断で意味が強いため named field。
  adapter 固有の値 (`timezone`, `ble_mode`, `tx_power`, `advertise_interval`) は
  `properties: BTreeMap<String, ConfigValue>` に typed で入る。
- `ConfigValue` は lossless な型付き値。
  全部 String に潰すと UI/API 側が毎回 parse し直すことになるため、
  数値・真偽値はそのまま持つ。
- `uplink_interval_secs` は `u32`。BravePI の ConfigFrame の `uplink_interval` が
  もともと `u32` なので f64 にする理由がない。

### DeviceState の拡張と DeviceTarget

```rust
// bravepi-adapter/src/task/event_loop.rs

struct DeviceTarget {
    device_number_hex: String,
    raw_sensor_type: u16,
}

struct DeviceState {
    last_seen: tokio::time::Instant,
    target: DeviceTarget,
}
```

設計ポイント:
- `DeviceTarget` は downlink command routing に必要な情報をまとめた構造体。
  `device_number_hex` は `BravePiCodec::encode_downlink()` がそのまま要求する文字列。
  `raw_sensor_type` は `ImmediateUplink` の sensor_type パラメータに使う。
- `DeviceState` の lifecycle 情報 (`last_seen`) と address 情報 (`target`) を分離。
  将来 DeviceLost や pending query 追跡を足しても構造が崩れない。
- DeviceKey の文字列を parse しない。
  device_key → transmitter_id の解決は DeviceState 経由で行う。

### Write 経路: event_loop → reader thread

```
event_loop                    reader thread
    |                              |
    |-- write_tx.send(bytes) -->   |
    |                         try_recv() drain
    |                         transport.write()
    |                              |
    |                         transport.read()
    |  <-- bytes_tx.send() -----   |
```

設計ポイント:
- serial_source に write channel (`mpsc::Sender<Vec<u8>>`) を追加する。
  event_loop は DeviceCommand を encode した bytes を write_tx に送る。
- event_loop は `write_tx.try_send(bytes)` で non-blocking に送信する。
  `TrySendError::Full` の場合は AdapterError (device_key: Some, "downlink queue full") を送信。
  `TrySendError::Closed` の場合は transport failure として AdapterError (device_key: None) を送信。
  `.send().await` を使うと、reconnect/backoff 中に event_loop 全体が blocking され、
  uplink 消費や Shutdown 観測が止まるため、try_send で fail-fast にする。
- reader thread が read ループの前後で `write_rx.try_recv()` を drain し、
  `transport.write()` を呼ぶ。read と write の排他は thread 内で自然に解決する。
- serial_source には protocol 知識を入れない。channel は `Vec<u8>` のまま。
  `DownlinkCommand` → bytes 変換は event_loop 側で行う。
- serial write failure は device_key: None の AdapterError として扱う。
  reader thread は protocol metadata を持たないため、どの device_key の write が
  失敗したかを特定できない。device-targeted な AdapterError は event_loop 内で
  完結するもの (unknown device, encode failure, queue full) だけ。
- 500ms read timeout 前提で、downlink の最大遅延は 500ms。
  BravePI の downlink は即時性を要求しないため許容範囲。
- reconnect 後も write channel は生きたまま。
  transport が再接続されれば、pending write は新しい transport に書き込まれる。

### Command 処理フロー (event_loop)

event_loop の `command_rx.recv()` ブランチを拡張する。

```rust
cmd = command_rx.recv() => {
    match cmd {
        Some(AdapterCommand::Shutdown) | None => {
            tracing::info!("BravePI adapter shutting down");
            return;
        }
        Some(AdapterCommand::DeviceCommand(cmd)) => {
            handle_device_command(cmd, &devices, &write_tx, &event_tx).await;
        }
    }
}
```

`handle_device_command` の処理:

1. `devices.get(&cmd.device_key)` で `DeviceTarget` を取得。なければ AdapterError (device_key: Some) を送信。
2. payload と target の整合性を検証:
   - `SetOutput` は `SensorType::ContactOutput` の endpoint に対してのみ有効。
     `registry::lookup_handler(target.raw_sensor_type)` で handler を引き、
     `handler.sensor_type != SensorType::ContactOutput` なら AdapterError (device_key: Some,
     "SetOutput sent to non-ContactOutput device") を送信。
   - `SetOutput { duration_ms: Some(ms) }` で `ms > u16::MAX as u32` (65535) の場合は
     AdapterError (device_key: Some, "duration_ms exceeds u16 range") を送信。
     silent truncation (`as u16`) はしない。
3. `DeviceCommandPayload` → `DownlinkCommand` に変換:
   - `RequestReading` → `DownlinkCommand::ImmediateUplink { sensor_type: target.raw_sensor_type }`
   - `QueryConfig` → `DownlinkCommand::ParameterGet`
   - `SetOutput { value, duration_ms }` → `DownlinkCommand::ContactOutput { signal_mode, signal_out_time }`
     - `value: true` → `signal_mode: 1`, `value: false` → `signal_mode: 0`
     - `duration_ms: Some(ms)` → `signal_out_time: ms as u16`, `None` → `signal_out_time: 0`
4. `BravePiCodec::encode_downlink(target.device_number_hex, &downlink_cmd)` でバイト列に変換。
   失敗時は AdapterError (device_key: Some) を送信。
5. `write_tx.try_send(bytes)` で serial_source に non-blocking 送信。
   `TrySendError::Full` → AdapterError (device_key: Some, "downlink queue full")。
   `TrySendError::Closed` → AdapterError (device_key: None, transport failure)。

### ConfigFrame → DeviceConfig の変換

ConfigFrame を受信したとき、device_key を再構築して DeviceConfig event を送信する。

1. `ConfigFrame.true_sensor_type` で `registry::lookup_handler()` を呼び、`key_suffix` を取得。
   失敗時は warn して drop。
2. `format!("bravepi:{}:{}", config_frame.device_number, handler.key_suffix)` で device_key を再構築。
3. `devices` HashMap に device_key が存在することを確認。
   存在しなければ warn して drop (DeviceDiscovered 未発行のデバイスに config を流すのは契約違反)。
4. ConfigFrame のフィールドを DeviceConfigData に変換:

```rust
DeviceConfigData {
    firmware_version: Some(config_frame.firmware_version.clone()),
    uplink_interval_secs: Some(config_frame.uplink_interval),
    properties: BTreeMap::from([
        ("timezone".into(), ConfigValue::Integer(config_frame.timezone as i64)),
        ("ble_mode".into(), ConfigValue::Integer(config_frame.ble_mode as i64)),
        ("tx_power".into(), ConfigValue::Integer(config_frame.tx_power as i64)),
        ("advertise_interval".into(), ConfigValue::Integer(config_frame.advertise_interval as i64)),
    ]),
}
```

### AdapterError のルール

device_key の有無で障害の種別を区別する:

- `device_key: Some(...)`:
  - unknown device (DeviceDiscovered 未発行の device_key に command が来た)
  - payload/target 不整合 (SetOutput を非 ContactOutput endpoint に送信)
  - validation failure (duration_ms > u16::MAX)
  - encode failure (DeviceCommandPayload → DownlinkCommand → bytes の変換失敗)
  - downlink queue full (write_tx.try_send が Full)
- `device_key: None`:
  - adapter 全体の transport failure (write channel closed 含む)
  - reader thread / serial_source 死亡
  - serial write failure (reader thread 内。device_key を特定できないため None)

### convert.rs の変更: ConfigFrame の変換

現在 `convert.rs` の `frame_to_event()` は `BravePiFrame::Config` に対してログだけ出している。
D では ConfigFrame → DeviceConfig event への変換を追加する。

ただし device_key の再構築には `devices` HashMap が必要なため、
ConfigFrame の変換は `frame_to_event()` ではなく `event_loop` 側で行う。

`frame_to_event()` は引き続き Sensor frame と DecodeError frame を処理し、
Config frame は event_loop が直接処理する形に分離する。

## ファイル構成と変更範囲

### 変更

- `core/types/src/lib.rs`
  - `DeviceCommand` struct 追加
  - `DeviceCommandPayload` enum 追加
  - `DeviceConfigData` struct 追加
  - `ConfigValue` enum 追加
  - `AdapterCommand::DeviceCommand` variant 追加
  - `AdapterEvent::DeviceConfig` variant 追加

- `bravepi-adapter/src/task/event_loop.rs`
  - `DeviceTarget` struct 追加
  - `DeviceState` に `target: DeviceTarget` 追加
  - `handle_device_command()` 関数追加
  - command_rx ブランチで `DeviceCommand` を処理
  - ConfigFrame → DeviceConfig の変換ロジック追加
  - write_tx を event_loop 引数に追加

- `bravepi-adapter/src/task/convert.rs`
  - `BravePiFrame::Config` の処理を削除 (event_loop に移動)

- `bravepi-adapter/src/task/serial_source.rs`
  - `SerialSource` に `write_tx: mpsc::Sender<Vec<u8>>` を追加
  - reader thread に `write_rx: mpsc::Receiver<Vec<u8>>` を渡す
  - read ループの前後で `write_rx.try_recv()` を drain して `transport.write()` を呼ぶ

- `bravepi-adapter/src/task/handle.rs`
  - `start()` で write channel を作成し、serial_source と event_loop に配る

### テスト変更

- `core/types/src/lib.rs` (またはテストファイル)
  - DeviceCommand, DeviceCommandPayload の構築テスト
  - ConfigValue の各 variant テスト

- `bravepi-adapter/src/task/event_loop_test.rs`
  - DeviceCommand → downlink → write channel にバイト列が届くテスト
  - unknown device_key の command → AdapterError テスト
  - ConfigFrame 受信 → DeviceConfig event テスト
  - ConfigFrame の unknown sensor_type → warn + drop テスト

- `bravepi-adapter/src/task/convert_test.rs`
  - ConfigFrame 関連のテストが event_loop 側に移動する場合は調整
  - 既存の Sensor/DecodeError テストは変更なし

- `bravepi-adapter/src/task/serial_source.rs` (テスト)
  - write channel 経由の write テスト (transport mock が必要な場合は後続に defer)

### 変更なし

- `bravepi-adapter/codec/` 全体 (DownlinkCommand, encode_downlink は既存のまま)
- `bravepi-adapter/sensors/` 全体
- `bravepi-adapter/src/registry.rs` (lookup_handler は既存のまま使用)
- `bravepi-adapter/src/transport.rs`

### スコープ外

- busy/timeout/retry/ACK の本実装 --- orchestrator 層の後続 sub-project
- DFU の多段 state machine --- 別 sub-project
- pair/scan mode --- 別 sub-project
- request_id / correlation ID --- orchestrator 層で必要になったら追加
- UI の command state 表示 --- 別 sub-project
- ConfigFrame の unsolicited 受信 (ParameterGet なしで来る config) の扱い --- 後続で検討
