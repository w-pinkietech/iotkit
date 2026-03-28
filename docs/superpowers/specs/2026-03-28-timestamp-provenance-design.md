# AdapterEvent タイムスタンプ/provenance 追加

## Goal

`AdapterEvent::SensorData` に壁時計タイムスタンプ (`ingested_at`) を追加し、
timeseries-service で正確な時刻付きデータ永続化を可能にする。

## Background

現在 `SensorReading` も `AdapterEvent` もタイムスタンプを持たない。
Engine 内部では `tokio::time::Instant`（相対時刻）で `last_seen` を管理しているが、
これは永続化には使えない。timeseries-service (#22) が前提とする壁時計時刻を、
永続化層を作る前に入れておく。

## Design Decisions

### 1. タイムスタンプの配置: AdapterEvent に置く

**選択: `AdapterEvent::SensorData` に `ingested_at: SystemTime` フィールドを追加**

SensorReading はセンサー値の純粋なデータ構造として保つ。タイムスタンプは
「いつこの event が生成されたか」というメタデータであり、event レベルが適切。

- SensorReading に入れると、SensorReading の PartialEq に時刻が含まれて
  テストが煩雑になる
- 全 AdapterEvent variant に共通 envelope を作る案は YAGNI。
  SensorData と DeviceConfig だけがタイムスタンプを必要とする

DeviceConfig にも `ingested_at` を追加する。DeviceDiscovered / DeviceLost /
AdapterError は時刻を必要としない（Engine 内部の Instant で十分）。

### 2. 型の選択: `std::time::SystemTime`

- **chrono / time クレート不要** — 壁時計時刻は `SystemTime` で十分
- Engine 内部の `Instant`（相対時刻）は維持する。`last_seen` の用途
  （timeout 判定）には `Instant` が適切
- timeseries-service が SQLite に書く際に `SystemTime` → Unix epoch ミリ秒に
  変換する（その変換は timeseries-service の責務）

### 3. タイムスタンプ生成の場所: adapter 内

各 adapter が event を構築する時点で `SystemTime::now()` を呼ぶ。

- BravePI: `event_loop.rs` の `event_tx.send(AdapterEvent::SensorData { .. })` 時
- Polling runtime: `apply_outcomes()` で `AdapterEvent::SensorData` 生成時

Engine は受け取ったタイムスタンプをそのまま `DeviceView` に格納する。
Engine 側で `SystemTime::now()` を呼ばない — adapter と engine の間にキュー遅延が
あるため、adapter 側の時刻がより正確。

### 4. Engine state の変更

- `DeviceView` に `last_reading_at: Option<SystemTime>` を追加
- `State::apply` で SensorData 受信時に `ingested_at` を `last_reading_at` に保存
- 既存の `last_seen: Instant` は維持（timeout 判定用、この issue のスコープ外）

### 5. core-types の変更まとめ

```rust
// AdapterEvent::SensorData に追加
SensorData {
    device_key: DeviceKey,
    reading: SensorReading,
    rssi: Option<i16>,
    battery_pct: Option<u8>,
    ingested_at: std::time::SystemTime,  // NEW
},

// AdapterEvent::DeviceConfig に追加
DeviceConfig {
    device_key: DeviceKey,
    config: DeviceConfigData,
    ingested_at: std::time::SystemTime,  // NEW
},
```

### 6. 変更対象のファイル

| Crate | File | Change |
|-------|------|--------|
| core/types | `src/lib.rs` | SensorData, DeviceConfig に `ingested_at` 追加 |
| core/engine | `src/lib.rs` | DeviceView に `last_reading_at` 追加 |
| core/engine | `src/state.rs` | apply で `ingested_at` → `last_reading_at` 保存 |
| core/engine | `src/state_test.rs` | テスト更新 |
| bravepi-mainboard-adapter | `src/task/event_loop.rs` | event 生成時に `SystemTime::now()` |
| iotkit-polling-adapter-runtime | `src/polling_loop.rs` | apply_outcomes で `SystemTime::now()` |
| iotkit-gateway | `src/adapter_host.rs` | テスト内の stub event 更新 |
| bravepi-mainboard-adapter | `src/task/handle.rs` | テスト内の event 更新 |
| iotkit-polling-adapter-runtime | `src/lib.rs` | テスト内の event 更新 |

### 7. テスト方針

- core/types: 既存テストに `ingested_at: SystemTime::now()` を追加（コンパイル通過）
- core/engine: `sensor_data_updates_reading` テストで `last_reading_at` が
  `Some(_)` であることを検証
- adapter テスト: stub event に `ingested_at` を追加してコンパイル通過

SensorReading の PartialEq は影響なし（タイムスタンプは SensorReading に入れない）。

### 8. スコープ外

- `discovered_at` / `last_seen` の `Instant` → `SystemTime` 移行（不要）
- timeseries-service での永続化ロジック（#22 の範囲）
- adapter 間のクロック同期（単一マシン前提）
