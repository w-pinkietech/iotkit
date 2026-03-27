# Code Review Fixes: Critical + Important Items

Date: 2026-03-26
Status: Approved

## Background

Phase 1 PoC + hardening 完了後にアーキテクチャレビューを実施。
Critical 3件 + Important 7件が指摘された (うち2件は Phase 2 defer)。
本 spec は残り8件の修正を定義する。

## Scope

8 fixes:

### Critical (3件)
1. SensorReading に labels フィールド追加
2. Codec にフレームサイズ上限 (MAX_FRAME_SIZE) 追加
3. hex_to_device_bytes のエラーハンドリング修正

### Important (5件)
4. BravePiFrame / SensorFrame / ConfigFrame に Clone + PartialEq 追加
5. BravePiCodec に Default 実装追加
6. codec モジュールパスの冗長性解消 (re-export)
7. AdapterId / DeviceKey の newtype 強化 (フィールド private 化)
8. doc comment の言語統一 (日本語)

### Out of scope (Phase 2 defer)
- serial_reader_thread のテスタビリティ (trait 抽象化)
- reader thread shutdown 遅延の改善 (AtomicBool 等)

---

## Fix 1: SensorReading labels フィールド追加

### Location
`core/types/src/lib.rs` — `SensorReading`
`bravepi-adapter/sensors/src/*.rs` — 各 sensor module

### Design

**SensorReading 変更:**
```rust
#[derive(Debug, Clone, PartialEq)]
pub struct SensorReading {
    pub sensor_type: SensorType,
    pub values: Vec<f64>,
    pub labels: Vec<&'static str>,
}

impl SensorReading {
    pub fn new(sensor_type: SensorType, values: Vec<f64>, labels: Vec<&'static str>) -> Self {
        Self { sensor_type, values, labels }
    }

    pub fn empty(sensor_type: SensorType) -> Self {
        Self { sensor_type, values: vec![], labels: vec![] }
    }
}
```

**各 sensor module の labels:**

| Module | Labels |
|--------|--------|
| mcp9600 | `["celsius"]` |
| opt3001 | `["lux"]` |
| mcp3427 | `["ch1_volt", "ch2_volt"]` |
| vl53l1x | `["distance_mm"]` |
| sdp810 | `["pascal"]` |
| lis2duxs12 | `["x_g", "y_g", "z_g", "magnitude_g"]` |

**ContactInput / ContactOutput:**
labels は空 (`vec![]`)。data_count が動的で、値は 0.0/1.0 の on/off なので自明。

### Impact
- `SensorReading::new()` の全呼び出し元に labels 引数を追加
- 各 sensor module の `from_uart_payload` / `from_i2c_raw` を更新
- テストの期待値を更新

---

## Fix 2: Codec フレームサイズ上限

### Location
`bravepi-adapter/codec/src/codec.rs` — `BravePiCodec::decode()`

### Design

```rust
const MAX_FRAME_SIZE: usize = 4096;
```

`decode()` 内で `payload_len` を読み取った後:
1. `frame_len` (= HEADER + payload_len) が `MAX_FRAME_SIZE` を超える場合
2. バッファをクリア (`self.buf.clear()`, continuation state をリセット)
3. `Some(BravePiFrame::DecodeError)` を返す (device_number は "unknown", reason は "frame size exceeds maximum")

### Rationale
- BravePI の実際のフレームは最大でも数百バイト
- 4096 は十分な余裕を持った上限
- Phase 2 で設定可能にすることも可能だが、PoC では定数で十分

---

## Fix 3: hex_to_device_bytes エラーハンドリング

### Location
`bravepi-adapter/codec/src/codec.rs` — `encode_downlink()`, `hex_to_device_bytes()`

### Design

**hex_to_device_bytes 変更:**
```rust
fn hex_to_device_bytes(hex: &str) -> Result<[u8; 8], String> {
    let val = u64::from_str_radix(hex, 16)
        .map_err(|e| format!("Invalid device number hex '{}': {}", hex, e))?;
    Ok(val.to_be_bytes())
}
```

**encode_downlink 変更:**
```rust
pub fn encode_downlink(device_number_hex: &str, cmd: &DownlinkCommand) -> Result<Vec<u8>, String>
```

戻り値が `Vec<u8>` → `Result<Vec<u8>, String>` に変更。

### Impact
- `encode_downlink` の呼び出し元は現在 PoC 内にはない (将来の downlink 機能用)
- 破壊的変更だが、使用箇所がないため影響は軽微
- codec テストの encode 関連テストを更新

---

## Fix 4: BravePiFrame 等の derive 追加

### Location
`bravepi-adapter/codec/src/codec.rs`

### Design

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct SensorFrame { ... }

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigFrame { ... }

#[derive(Debug, Clone, PartialEq)]
pub enum BravePiFrame { ... }

#[derive(Debug, Clone, PartialEq)]
pub enum DownlinkCommand { ... }
```

全ての public 型に `Clone` + `PartialEq` を追加。

---

## Fix 5: BravePiCodec に Default 追加

### Location
`bravepi-adapter/codec/src/codec.rs`

### Design

```rust
impl Default for BravePiCodec {
    fn default() -> Self {
        Self::new()
    }
}
```

---

## Fix 6: codec パス冗長性解消

### Location
`bravepi-adapter/codec/src/lib.rs`

### Design

```rust
pub mod codec;

pub use codec::{BravePiCodec, BravePiFrame, SensorFrame, ConfigFrame, DownlinkCommand};
```

consumer は `bravepi_codec::BravePiCodec` で使えるようになる。
既存の `bravepi_codec::codec::*` パスも引き続き有効 (後方互換)。

### Impact
- bravepi-adapter 内の import を簡潔なパスに更新可能 (任意)

---

## Fix 7: AdapterId / DeviceKey の newtype 強化

### Location
`core/types/src/lib.rs` — `AdapterId`, `DeviceKey`
全 crate の使用箇所

### Design

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AdapterId(String);

impl AdapterId {
    pub fn new(id: impl Into<String>) -> Self { Self(id.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceKey(String);

impl DeviceKey {
    pub fn new(key: impl Into<String>) -> Self { Self(key.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}
```

`Display` 実装は既存のまま (内部で `self.0` を使用)。

### Impact
- `AdapterId("...".to_string())` → `AdapterId::new("...")`
- `device_key.0` → `device_key.as_str()`
- adapter, poc, テストの全使用箇所を更新

---

## Fix 8: doc comment 言語統一

### Location
`bravepi-adapter/src/task/*.rs` — adapter 内のモジュール

### Design

adapter 内の英語 doc comment を日本語に統一。対象:
- `handle.rs` の `shutdown()` doc: "Send Shutdown command..." → 日本語
- `reader.rs` のモジュール doc
- その他散在する英語コメント

transport crate (`rpi4b-driver/`) はスコープ外。

---

## Dependency Changes

なし。全て既存の依存で完結。

## Test Summary

| Test file | Changes |
|-----------|---------|
| `bravepi-adapter/codec/tests/codec_test.rs` | encode_downlink の Result 対応、MAX_FRAME_SIZE テスト追加 |
| `bravepi-adapter/sensors/src/*.rs` (各 module tests) | labels 引数追加 |
| `bravepi-adapter/tests/frame_to_event_test.rs` | labels 検証、DeviceKey アクセサ対応 |
| `bravepi-adapter/tests/event_loop_test.rs` | DeviceKey アクセサ対応 |
