# Wave 0 計画1: 取り込み契約とコアの土台 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 取り込み契約(Envelope/Ack)・デバイス台帳(core/ledger)・コレクタ(R8: 台帳解決+dedup+ack耐久点)を新設し、既存2アダプタのデータが新経路(プロセス内バインディング)で series_id 付きの readings v3 に流れる状態にする。

**Architecture:** D1(取り込み契約)/D5(series識別)/D6(測定レジストリ)の Wave 0 実装。アダプタ→(暫定ブリッジ)→Envelope→コレクタactor(mpsc+oneshot)→SQLite同一トランザクション(ingest_dedup+readings)→コミット後ack。台帳解決(hardware_id→system_id→series_id)はコレクタが実行。レジストリ検証は `RegistryPolicy` トレイトのフックとして席だけ用意し、計画2が実装を差し込む。

**Tech Stack:** Rust 2024 / tokio / rusqlite 0.32 (bundled) / serde / uuid (v4+v7)。設計正本: `/home/kenta/dev/iot/docs/redesign/` の terminology.md, responsibility-ledger.md, decisions/D1・D5・D6。

## Global Constraints

- 既存クレートのリネーム・分割は**しない**(D4決定。core/typesに契約型を足さない)
- DB層は **rusqlite 継続**(sqlxへ移行しない。ADR 0025処置)
- `SensorReading.labels` は `Vec<String>` へ変更(D1: serdeデシリアライズ不可のため)
- SQLite PRAGMA は **WAL + `synchronous=NORMAL`**(D1。現行FULLから変更)
- **ack = 耐久点**: SQLiteコミット完了後にのみ accepted を返す(D1)
- dedupキー = **(認証済み送信者アイデンティティ, envelope_id)**。ウィンドウはTTL 72h+サイズ上限(D1)
- measurement_key文法: セグメント=`[a-z][a-z0-9_]*`、区切りドット、**コロン禁止**、長さ上限64(D6決定2)
- channel_index の'na'はDB内では**番兵値-1**(D5決定3)。system_idはDB内BLOB16/API境界TEXT36
- readings のカーソルは **AUTOINCREMENT単調seq**(rowid直用禁止、D5決定3)
- `panic = "abort"` 禁止(D1)。テストは `cargo test --workspace` 全緑を維持
- コミット規約: `feat(crate):` / `fix(crate):` 等 + `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`(リポジトリCLAUDE.md)。
  **計画中の `git commit -m "..."` 例は簡略表記**——実際のコミットは必ず上記trailerを2つ目の `-m` で付けること
- 実行体制: タスクごとに新しいdevサブエージェント(Sonnet)+TDD。各タスク完了時に `codex exec` によるeval(リポジトリCLAUDE.mdの規律)

---

### Task 1: テストハーネス健全化(cargo test --workspace を並列で緑にする)

**Files:**
- Modify: `iotkit-gateway/Cargo.toml`(dev-dependenciesに serial_test 追加)
- Modify: `iotkit-gateway/src/config.rs`(環境変数/カレントディレクトリを触るテストに `#[serial]` 付与)

**Interfaces:**
- Consumes: なし
- Produces: `cargo test --workspace` がオプションなしで全緑(以後の全タスクの検証コマンドの前提)

- [ ] **Step 1: 現状のFAILを確認**

Run: `cd /home/kenta/dev/iot/iotkit-next && cargo test -p iotkit-gateway 2>&1 | tail -20`
Expected: config系テスト6件前後がFAIL(env var/cwd競合。`config.rs:316`付近のコメントに「--test-threads=1必須」とある既知問題)

- [ ] **Step 2: serial_test を追加**

`iotkit-gateway/Cargo.toml` の `[dev-dependencies]` に追記:

```toml
serial_test = "3"
```

- [ ] **Step 3: 該当テストに #[serial] を付与**

`iotkit-gateway/src/config.rs` のテストモジュールで、`with_env_vars` または `CwdGuard` を使う全テスト関数に付与:

```rust
use serial_test::serial;

#[test]
#[serial]
fn 既存のテスト名はそのまま() { /* 変更なし */ }
```

対象の特定: `grep -n 'with_env_vars\|CwdGuard' iotkit-gateway/src/config.rs` で出る行を含むテスト関数すべて。

- [ ] **Step 4: 並列実行で全緑を確認**

Run: `cargo test --workspace`
Expected: 全crate PASS(`--test-threads=1` なしで)

- [ ] **Step 5: Commit**

```bash
git add iotkit-gateway/Cargo.toml iotkit-gateway/src/config.rs
git commit -m "fix(gateway): serialize env-dependent config tests so workspace tests pass in parallel"
```

---

### Task 2: iotkit-ingest-contract クレート新設

**Files:**
- Create: `iotkit-ingest-contract/Cargo.toml`
- Create: `iotkit-ingest-contract/src/lib.rs`
- Create: `iotkit-ingest-contract/src/envelope.rs`
- Create: `iotkit-ingest-contract/src/ack.rs`
- Create: `iotkit-ingest-contract/src/measurement_key.rs`
- Modify: ルート `Cargo.toml`(membersに追加)

**Interfaces:**
- Consumes: なし(**依存はserdeのみ**=D4の掟。tokio/rusqlite/uuidを入れない)
- Produces(後続タスクが使う正確な型):
  - `Envelope { envelope_id: String, source: String, declaration_version: Option<u32>, items: Vec<ReadingItem> }`
  - `ReadingItem { subject_hint: Option<String>, measurement_key: String, channel_index: Option<u16>, series_variant: Option<String>, values: Vec<f64>, device_time_ms: Option<i64>, time_source: TimeSource, age_ms: Option<u64>, rssi: Option<i16>, battery_pct: Option<u8> }`
  - `TimeSource { DeviceNtp, DeviceRtc, Gateway, GatewayAdjusted }`
  - `EnvelopeAck { envelope_id: String, status: AckStatus }`
  - `AckStatus { Accepted { items: Vec<ItemStatus> }, Duplicate, Rejected { reason_code: ReasonCode, message: String }, Deferred }`
  - `ItemStatus { Stored { disposition: Disposition }, ItemRejected { reason_code: ReasonCode, message: String } }`
  - `Disposition { Durable, Staged, Quarantined }`(D1監査追記+D6決定6の3値)
  - `ReasonCode { MalformedMeasurementKey, ValueTypeMismatch, UnknownSubject, SubjectScopeViolation, BatchTooLarge, StaleTimestamp, Internal }`
  - `validate_measurement_key(&str) -> Result<(), MeasurementKeyError>` / `pub const MAX_MEASUREMENT_KEY_LEN: usize = 64`
  - `external_envelope_id(sender_id: &str, boot_epoch: u64, seq: u64) -> String`(外部デバイス向け採番レシピ=D1推奨プロファイルの文字列形式。プロセス内はUUIDv4を**呼び出し側**で生成)

- [ ] **Step 1: クレートの骨格を作る**

`iotkit-ingest-contract/Cargo.toml`:

```toml
[package]
name = "iotkit-ingest-contract"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "1", features = ["derive"] }

[dev-dependencies]
serde_json = "1"
```

ルート `Cargo.toml` の `members` に `"iotkit-ingest-contract"` を追加。

`src/lib.rs`:

```rust
//! 取り込み契約 v1(安定意図)。ワイヤ契約が規範、このクレートは正本のRust表現。
//! 正本文書: docs/redesign/decisions/D1-ingest-model.md, D6-measurement-registry.md
pub mod ack;
pub mod envelope;
pub mod measurement_key;

pub use ack::{AckStatus, Disposition, EnvelopeAck, ItemStatus, ReasonCode};
pub use envelope::{Envelope, ReadingItem, TimeSource};
pub use measurement_key::{
    external_envelope_id, validate_measurement_key, MeasurementKeyError, MAX_MEASUREMENT_KEY_LEN,
};
```

- [ ] **Step 2: measurement_key検証の失敗テストを書く**

`src/measurement_key.rs`(テストのみ先行):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_standard_and_custom_keys() {
        for k in ["temperature_c", "voltage_mv", "custom.tank_level", "a", "x9_z.b_1"] {
            assert!(validate_measurement_key(k).is_ok(), "{k} should be valid");
        }
    }

    #[test]
    fn rejects_colon_uppercase_and_bad_segments() {
        for k in ["custom:temp", "Temp", "9abc", "a..b", ".a", "a.", "", "温度"] {
            assert!(validate_measurement_key(k).is_err(), "{k} should be invalid");
        }
    }

    #[test]
    fn rejects_over_64_chars() {
        let k = "a".repeat(65);
        assert!(matches!(
            validate_measurement_key(&k),
            Err(MeasurementKeyError::TooLong { .. })
        ));
    }

    #[test]
    fn envelope_id_recipe_is_stable() {
        assert_eq!(external_envelope_id("dev1", 3, 42), "dev1-3-42");
    }
}
```

- [ ] **Step 3: 失敗を確認**

Run: `cargo test -p iotkit-ingest-contract`
Expected: FAIL(validate_measurement_key未定義のコンパイルエラー)

- [ ] **Step 4: 実装**

`src/measurement_key.rs` の本体:

```rust
pub const MAX_MEASUREMENT_KEY_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum MeasurementKeyError {
    Empty,
    TooLong { len: usize },
    /// コロン等の禁止文字、大文字、セグメント先頭が英小文字でない、空セグメント
    InvalidSegment { segment: String },
}

impl std::fmt::Display for MeasurementKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "measurement_key is empty"),
            Self::TooLong { len } => {
                write!(f, "measurement_key length {len} exceeds {MAX_MEASUREMENT_KEY_LEN}")
            }
            Self::InvalidSegment { segment } => {
                write!(f, "invalid measurement_key segment '{segment}': expected [a-z][a-z0-9_]*")
            }
        }
    }
}
impl std::error::Error for MeasurementKeyError {}

/// D6決定2: セグメント=[a-z][a-z0-9_]*、区切りドット、コロン禁止(charsetで排除)、上限64。
pub fn validate_measurement_key(key: &str) -> Result<(), MeasurementKeyError> {
    if key.is_empty() {
        return Err(MeasurementKeyError::Empty);
    }
    if key.len() > MAX_MEASUREMENT_KEY_LEN {
        return Err(MeasurementKeyError::TooLong { len: key.len() });
    }
    for seg in key.split('.') {
        let mut chars = seg.chars();
        let valid = matches!(chars.next(), Some('a'..='z'))
            && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'));
        if !valid {
            return Err(MeasurementKeyError::InvalidSegment { segment: seg.to_string() });
        }
    }
    Ok(())
}

/// D1推奨プロファイル `sender_id + boot_epoch + 単調seq` の正準文字列形式。
pub fn external_envelope_id(sender_id: &str, boot_epoch: u64, seq: u64) -> String {
    format!("{sender_id}-{boot_epoch}-{seq}")
}
```

- [ ] **Step 5: テスト通過を確認**

Run: `cargo test -p iotkit-ingest-contract`
Expected: PASS

- [ ] **Step 6: Envelope/Ack型のserde往復テストを書く**

`src/envelope.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub envelope_id: String,
    /// 送信者の自己記述(D1: adapter_idの出所はチャネルキーでなくエンベロープ自身)
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration_version: Option<u32>,
    pub items: Vec<ReadingItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingItem {
    /// = hardware_id。多subject送信者(親子束ね)は必須(D5決定1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_hint: Option<String>,
    pub measurement_key: String,
    /// None = 'na'(DB内では番兵値-1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_index: Option<u16>,
    /// None = "primary"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_variant: Option<String>,
    pub values: Vec<f64>,
    /// デバイス申告時刻(unix ms)。オプショナル(D1: 時刻がないからrejectedは禁止)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_time_ms: Option<i64>,
    pub time_source: TimeSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rssi: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_pct: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeSource {
    DeviceNtp,
    DeviceRtc,
    Gateway,
    GatewayAdjusted,
}
```

`src/ack.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeAck {
    pub envelope_id: String,
    pub status: AckStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AckStatus {
    /// エンベロープ全体が耐久化された。items は入力itemsと同数・同順(部分受理の内訳)
    Accepted { items: Vec<ItemStatus> },
    Duplicate,
    /// エンベロープ単位の終端拒否(送信側はspoolから除去=D1)
    Rejected { reason_code: ReasonCode, message: String },
    /// 一時的過負荷専用。同一エンベロープを不変のまま再試行(D1)
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ItemStatus {
    Stored { disposition: Disposition },
    ItemRejected { reason_code: ReasonCode, message: String },
}

/// D1監査追記(durable|staged)+D6決定6(quarantined)の3値
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Durable,
    Staged,
    Quarantined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    MalformedMeasurementKey,
    ValueTypeMismatch,
    UnknownSubject,
    SubjectScopeViolation,
    BatchTooLarge,
    StaleTimestamp,
    Internal,
}
```

`src/envelope.rs` 末尾にテスト:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ack::{AckStatus, Disposition, EnvelopeAck, ItemStatus};

    fn sample_envelope() -> Envelope {
        Envelope {
            envelope_id: "gw-1-1".into(),
            source: "bravepi-mainboard".into(),
            declaration_version: None,
            items: vec![ReadingItem {
                subject_hint: Some("ble:00000000000000ab".into()),
                measurement_key: "temperature_c".into(),
                channel_index: None,
                series_variant: None,
                values: vec![21.5],
                device_time_ms: None,
                time_source: TimeSource::Gateway,
                age_ms: None,
                rssi: Some(-60),
                battery_pct: Some(88),
            }],
        }
    }

    #[test]
    fn envelope_json_round_trip() {
        let e = sample_envelope();
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<Envelope>(&json).unwrap(), e);
        // オプショナル欄は省略される(ワイヤの軽さ)
        assert!(!json.contains("device_time_ms"));
    }

    #[test]
    fn ack_json_round_trip() {
        let ack = EnvelopeAck {
            envelope_id: "gw-1-1".into(),
            status: AckStatus::Accepted {
                items: vec![ItemStatus::Stored { disposition: Disposition::Quarantined }],
            },
        };
        let json = serde_json::to_string(&ack).unwrap();
        assert!(json.contains("\"disposition\":\"quarantined\""));
        assert_eq!(serde_json::from_str::<EnvelopeAck>(&json).unwrap(), ack);
    }
}
```

- [ ] **Step 7: 全テスト通過を確認**

Run: `cargo test -p iotkit-ingest-contract && cargo test --workspace`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml iotkit-ingest-contract/
git commit -m "feat(ingest-contract): add envelope/ack contract types, measurement_key grammar, envelope_id recipe"
```

---

### Task 3: SensorReading.labels を Vec<String> へ変更(D1フェーズ1・機械的)

**Files:**
- Modify: `core/types/src/lib.rs:121-136`(SensorReading定義)
- Modify: `bravepi-mainboard-adapter/sensors/src/` 配下の全ドライバ(decode_uart/from_i2c_raw実装のlabels構築。
  rpi-local-adapterのドライバは `bravepi_sensors::{mcp9600,opt3001}::from_i2c_raw` へ委譲しているため
  **bravepi-sensors側の修正で足りる**——rpi-local側にlabels構築はない)
- Modify: `core/engine/src/state_test.rs`(`SensorReading::new(..., vec!["temperature_c"])` 等の呼び出し実在)
- Modify: 上記を参照する全テスト(Step 2のコンパイルエラー列挙が正)

**Interfaces:**
- Consumes: なし
- Produces: `SensorReading { sensor_type: SensorType, values: Vec<f64>, labels: Vec<String> }`、`SensorReading::new(sensor_type, values: Vec<f64>, labels: Vec<String>)`(後続タスクとブリッジがchannelラベルとして読む)

- [ ] **Step 1: core/typesの型を変更**

`core/types/src/lib.rs` の SensorReading:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct SensorReading {
    pub sensor_type: SensorType,
    pub values: Vec<f64>,
    pub labels: Vec<String>,
}

impl SensorReading {
    pub fn new(sensor_type: SensorType, values: Vec<f64>, labels: Vec<String>) -> Self {
        Self { sensor_type, values, labels }
    }
    pub fn empty(sensor_type: SensorType) -> Self {
        Self { sensor_type, values: Vec::new(), labels: Vec::new() }
    }
}
```

- [ ] **Step 2: コンパイルエラーを網羅リストとして採取**

Run: `cargo build --workspace 2>&1 | grep -E '^error|-->' | head -60`
Expected: `Vec<&'static str>` を渡している全呼び出し箇所がエラーで列挙される(bravepi-sensors の各HANDLER、rpi-local-adapterドライバ、テスト)

- [ ] **Step 3: 全呼び出し箇所を機械的に修正**

パターン: `vec!["ch0", "ch1"]` → `vec!["ch0".to_string(), "ch1".to_string()]`。
静的スライスから作る箇所は `["x","y","z"].iter().map(|s| s.to_string()).collect()`。
`grep -rn 'labels' --include='*.rs' bravepi-mainboard-adapter/ rpi-local-adapter/ core/` で漏れゼロを確認。

- [ ] **Step 4: 全テスト通過を確認**

Run: `cargo test --workspace`
Expected: PASS(挙動変更なし・型のみ)

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(types): change SensorReading.labels to Vec<String> for serde compatibility (D1 phase 1)"
```

---

### Task 4: core/ledger クレート新設(デバイス台帳+series台帳+目撃ステージング)

**Files:**
- Create: `core/ledger/Cargo.toml`
- Create: `core/ledger/migrations/0003_ledger.sql`
- Create: `core/ledger/src/lib.rs`
- Create: `core/ledger/src/ids.rs`
- Create: `core/ledger/src/store.rs`
- Modify: ルート `Cargo.toml`(members追加)

**Interfaces:**
- Consumes: `iotkit-core-storage::{Migration, StorageError}`、rusqlite `Connection`
- Produces(コレクタ・CLI・計画4が使う):
  - `pub const MIGRATIONS: &[Migration]`(version 3, label "ledger")
  - `SystemId`(内部 `[u8;16]` UUIDv7。`SystemId::generate()`, `as_bytes() -> &[u8;16]`, `to_text() -> String`(36字), `from_text(&str) -> Result<SystemId, LedgerError>`)
  - `DeviceKind { Individual, Positional }`(D5決定2の2分類)
  - `DeviceState { Quarantined, Active, Retired }`
  - `DeviceRow { system_id: SystemId, hardware_id: String, user_label: Option<String>, parent: Option<SystemId>, kind: DeviceKind, state: DeviceState, declaration_version: Option<u32> }`
  - `NewDevice { hardware_id: String, user_label: Option<String>, parent: Option<SystemId>, kind: DeviceKind, initial_state: DeviceState }`
  - `insert_device(conn: &Connection, new: &NewDevice) -> Result<SystemId, LedgerError>`(生きたhardware_id重複は `LedgerError::HardwareIdInUse`。ledger_eventsへ監査行)
  - `find_alive_by_hardware_id(conn: &Connection, hardware_id: &str) -> Result<Option<DeviceRow>, LedgerError>`
  - `ensure_series(conn: &Connection, system_id: &SystemId, measurement_key: &str, channel_index: i32, variant: &str, quarantined: bool) -> Result<i64, LedgerError>`(既存なら既存series_id、なければINSERTして新series_id)
  - `record_sighting(conn: &Connection, hardware_id: &str, source: &str) -> Result<(), LedgerError>`(upsert: first_seen/last_seen/count)
  - `approve_sighting(conn: &Connection, hardware_id: &str, user_label: Option<&str>, kind: DeviceKind) -> Result<SystemId, LedgerError>`(D5経路A: 採番+エントリ作成(state=Quarantined)+sighting行削除+監査行。staged_readingsの本流化は計画4のCLIで実施)
  - `activate_device(conn: &Connection, system_id: &SystemId) -> Result<(), LedgerError>`(検疫→active)
  - `ledger_epoch(conn: &Connection) -> Result<String, LedgerError>`(台帳エポック。初回アクセス時に生成しledger_metaへ永続化)
  - `LedgerError { HardwareIdInUse(String), NotFound(String), InvalidId(String), Storage(StorageError), Sqlite(rusqlite::Error) }`

- [ ] **Step 1: クレート骨格とマイグレーションSQL**

`core/ledger/Cargo.toml`:

```toml
[package]
name = "iotkit-core-ledger"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-core-storage = { path = "../storage" }
rusqlite = { version = "0.32", features = ["bundled"] }
uuid = { version = "1", features = ["v7"] }
tracing = "0.1"

[dev-dependencies]
iotkit-core-storage = { path = "../storage", features = ["test-util"] }
```

ルート `Cargo.toml` members に `"core/ledger"` を追加。

`core/ledger/migrations/0003_ledger.sql`:

```sql
-- D5: デバイス台帳(R7)+series台帳実体化+目撃ステージング+監査+台帳メタ
CREATE TABLE devices (
    system_id            BLOB PRIMARY KEY,          -- UUIDv7 16bytes(D5決定3)
    hardware_id          TEXT NOT NULL,
    user_label           TEXT,
    parent_system_id     BLOB REFERENCES devices(system_id),
    kind                 TEXT NOT NULL CHECK (kind IN ('individual','positional')),
    state                TEXT NOT NULL CHECK (state IN ('quarantined','active','retired')),
    declaration_version  INTEGER,
    superseded_by        BLOB REFERENCES devices(system_id),
    created_at           INTEGER NOT NULL,
    retired_at           INTEGER
);
-- 生きたエントリ間でのみhardware_id一意(D5決定1: retiredは除外)
CREATE UNIQUE INDEX idx_devices_hardware_alive
    ON devices(hardware_id) WHERE state != 'retired';

CREATE TABLE series (
    series_id        INTEGER PRIMARY KEY AUTOINCREMENT,  -- 単調・再利用なし(D5決定3)
    system_id        BLOB NOT NULL REFERENCES devices(system_id),
    measurement_key  TEXT NOT NULL,
    channel_index    INTEGER NOT NULL DEFAULT -1,        -- 'na'は番兵値-1(D5決定3)
    variant          TEXT NOT NULL DEFAULT 'primary',
    quarantined      INTEGER NOT NULL DEFAULT 0,
    value_semantics  TEXT NOT NULL DEFAULT 'calibrated', -- raw_legacy|calibrated(D5)
    unit             TEXT,
    range_min        REAL,
    range_max        REAL,
    legacy_sensor_type INTEGER,
    created_at       INTEGER NOT NULL,
    UNIQUE (system_id, measurement_key, channel_index, variant)
);

-- 目撃ステージング: 有界・パージ可能(D5決定4経路A)
CREATE TABLE sightings (
    hardware_id  TEXT PRIMARY KEY,
    source       TEXT NOT NULL,
    first_seen   INTEGER NOT NULL,
    last_seen    INTEGER NOT NULL,
    observations INTEGER NOT NULL DEFAULT 1
);

-- append-only監査(R13の最小下地)
CREATE TABLE ledger_events (
    event_id   INTEGER PRIMARY KEY AUTOINCREMENT,
    at         INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    system_id  BLOB,
    detail     TEXT NOT NULL
);

CREATE TABLE ledger_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

`src/lib.rs`:

```rust
pub mod ids;
pub mod store;

pub use ids::SystemId;
pub use store::{
    activate_device, approve_sighting, ensure_series, find_alive_by_hardware_id, insert_device,
    ledger_epoch, record_sighting, DeviceKind, DeviceRow, DeviceState, LedgerError, NewDevice,
};

use iotkit_core_storage::Migration;

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 3,
    label: "ledger",
    sql: include_str!("../migrations/0003_ledger.sql"),
}];
```

- [ ] **Step 2: SystemIdの失敗テストを書く**

`src/ids.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_unique_ids_and_text_round_trip() {
        let a = SystemId::generate();
        let b = SystemId::generate();
        assert_ne!(a, b);
        let text = a.to_text();
        assert_eq!(text.len(), 36);
        assert_eq!(SystemId::from_text(&text).unwrap(), a);
    }

    #[test]
    fn from_text_rejects_garbage() {
        assert!(SystemId::from_text("not-a-uuid").is_err());
    }
}
```

- [ ] **Step 3: 失敗を確認して実装**

Run: `cargo test -p iotkit-core-ledger`(FAIL確認後)

`src/ids.rs` 本体:

```rust
use crate::store::LedgerError;

/// 論理デバイスの主キー。UUIDv7・不変・台帳のみ発行・再利用永久禁止(D5決定1)。
/// DB内はBLOB16、API境界はTEXT36(D5決定3)。順序性には依存しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SystemId([u8; 16]);

impl SystemId {
    pub fn generate() -> Self {
        Self(*uuid::Uuid::now_v7().as_bytes())
    }
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
    pub fn from_bytes(b: [u8; 16]) -> Self {
        Self(b)
    }
    pub fn to_text(&self) -> String {
        uuid::Uuid::from_bytes(self.0).to_string()
    }
    pub fn from_text(s: &str) -> Result<Self, LedgerError> {
        uuid::Uuid::parse_str(s)
            .map(|u| Self(*u.as_bytes()))
            .map_err(|_| LedgerError::InvalidId(s.to_string()))
    }
}
```

Run: `cargo test -p iotkit-core-ledger`
Expected: PASS

- [ ] **Step 4: storeの失敗テストを書く**

`src/store.rs` のテストモジュール(`init_db_memory` + 自クレートMIGRATIONSを使用。storage v1と結合するため `iotkit_core_storage::MIGRATIONS` と連結):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_storage::init_db_memory;

    fn test_db() -> iotkit_core_storage::DbHandle {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(crate::MIGRATIONS);
        init_db_memory(&all).expect("in-memory db")
    }

    #[test]
    fn insert_and_resolve_device_by_hardware_id() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(conn, &NewDevice {
                hardware_id: "ble:00000000000000ab".into(),
                user_label: Some("炉1温度".into()),
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            }).unwrap();
            let row = find_alive_by_hardware_id(conn, "ble:00000000000000ab").unwrap().unwrap();
            assert_eq!(row.system_id, sid);
            assert_eq!(row.kind, DeviceKind::Individual);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn duplicate_alive_hardware_id_is_rejected() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let nd = NewDevice {
                hardware_id: "i2c:0x60".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Positional,
                initial_state: DeviceState::Active,
            };
            insert_device(conn, &nd).unwrap();
            assert!(matches!(insert_device(conn, &nd), Err(LedgerError::HardwareIdInUse(_))));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn ensure_series_is_idempotent_and_monotonic() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(conn, &NewDevice {
                hardware_id: "ble:cc".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            }).unwrap();
            let s1 = ensure_series(conn, &sid, "temperature_c", -1, "primary", false).unwrap();
            let s2 = ensure_series(conn, &sid, "temperature_c", -1, "primary", false).unwrap();
            let s3 = ensure_series(conn, &sid, "voltage_mv", 0, "primary", false).unwrap();
            assert_eq!(s1, s2);
            assert!(s3 > s1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn sighting_then_approve_creates_quarantined_device() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            record_sighting(conn, "ble:ff", "bravepi-mainboard").unwrap();
            record_sighting(conn, "ble:ff", "bravepi-mainboard").unwrap();
            let sid = approve_sighting(conn, "ble:ff", Some("新センサー"), DeviceKind::Individual).unwrap();
            let row = find_alive_by_hardware_id(conn, "ble:ff").unwrap().unwrap();
            assert_eq!(row.system_id, sid);
            assert_eq!(row.state, DeviceState::Quarantined);
            activate_device(conn, &sid).unwrap();
            assert_eq!(
                find_alive_by_hardware_id(conn, "ble:ff").unwrap().unwrap().state,
                DeviceState::Active
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn ledger_epoch_is_generated_once_and_stable() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let e1 = ledger_epoch(conn).unwrap();
            let e2 = ledger_epoch(conn).unwrap();
            assert_eq!(e1, e2);
            assert!(!e1.is_empty());
            Ok(())
        })
        .unwrap();
    }
}
```

注意: `with_conn_sync` のクロージャは `Result<T, StorageError>` を返す規約なので、`LedgerError` は各テスト内で `.map_err(...)` するか、テスト用に `Result<(), LedgerError>` を扱う小ヘルパを書いてよい(実装者判断。ただしプロダクションコードでは `LedgerError` を保つこと)。

- [ ] **Step 5: 失敗を確認して実装**

Run: `cargo test -p iotkit-core-ledger`(FAIL確認)

`src/store.rs` 本体(要点。now_msはSystemTimeから):

```rust
use crate::ids::SystemId;
use iotkit_core_storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug)]
pub enum LedgerError {
    HardwareIdInUse(String),
    NotFound(String),
    InvalidId(String),
    Storage(StorageError),
    Sqlite(rusqlite::Error),
}
impl std::fmt::Display for LedgerError { /* 各variantを一行で */
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HardwareIdInUse(h) => write!(f, "hardware_id already in use by a live entry: {h}"),
            Self::NotFound(w) => write!(f, "not found: {w}"),
            Self::InvalidId(s) => write!(f, "invalid system_id text: {s}"),
            Self::Storage(e) => write!(f, "storage error: {e}"),
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
        }
    }
}
impl std::error::Error for LedgerError {}
impl From<rusqlite::Error> for LedgerError {
    fn from(e: rusqlite::Error) -> Self { Self::Sqlite(e) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind { Individual, Positional }
impl DeviceKind {
    fn as_db(&self) -> &'static str {
        match self { Self::Individual => "individual", Self::Positional => "positional" }
    }
    fn from_db(s: &str) -> Self {
        if s == "positional" { Self::Positional } else { Self::Individual }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState { Quarantined, Active, Retired }
impl DeviceState {
    fn as_db(&self) -> &'static str {
        match self { Self::Quarantined => "quarantined", Self::Active => "active", Self::Retired => "retired" }
    }
    fn from_db(s: &str) -> Self {
        match s { "active" => Self::Active, "retired" => Self::Retired, _ => Self::Quarantined }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceRow {
    pub system_id: SystemId,
    pub hardware_id: String,
    pub user_label: Option<String>,
    pub parent: Option<SystemId>,
    pub kind: DeviceKind,
    pub state: DeviceState,
    pub declaration_version: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct NewDevice {
    pub hardware_id: String,
    pub user_label: Option<String>,
    pub parent: Option<SystemId>,
    pub kind: DeviceKind,
    pub initial_state: DeviceState,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn audit(conn: &Connection, kind: &str, system_id: Option<&SystemId>, detail: &str) -> Result<(), LedgerError> {
    conn.execute(
        "INSERT INTO ledger_events (at, kind, system_id, detail) VALUES (?1, ?2, ?3, ?4)",
        params![now_ms(), kind, system_id.map(|s| s.as_bytes().to_vec()), detail],
    )?;
    Ok(())
}

pub fn insert_device(conn: &Connection, new: &NewDevice) -> Result<SystemId, LedgerError> {
    if find_alive_by_hardware_id(conn, &new.hardware_id)?.is_some() {
        return Err(LedgerError::HardwareIdInUse(new.hardware_id.clone()));
    }
    let sid = SystemId::generate();
    conn.execute(
        "INSERT INTO devices (system_id, hardware_id, user_label, parent_system_id, kind, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            sid.as_bytes().to_vec(), new.hardware_id, new.user_label,
            new.parent.map(|p| p.as_bytes().to_vec()),
            new.kind.as_db(), new.initial_state.as_db(), now_ms()
        ],
    )?;
    audit(conn, "device_registered", Some(&sid), &new.hardware_id)?;
    Ok(sid)
}

pub fn find_alive_by_hardware_id(conn: &Connection, hardware_id: &str) -> Result<Option<DeviceRow>, LedgerError> {
    conn.query_row(
        "SELECT system_id, hardware_id, user_label, parent_system_id, kind, state, declaration_version
         FROM devices WHERE hardware_id = ?1 AND state != 'retired'",
        params![hardware_id],
        |row| {
            let sid: Vec<u8> = row.get(0)?;
            let parent: Option<Vec<u8>> = row.get(3)?;
            Ok(DeviceRow {
                system_id: SystemId::from_bytes(sid.try_into().expect("16-byte system_id")),
                hardware_id: row.get(1)?,
                user_label: row.get(2)?,
                parent: parent.map(|p| SystemId::from_bytes(p.try_into().expect("16-byte parent id"))),
                kind: DeviceKind::from_db(&row.get::<_, String>(4)?),
                state: DeviceState::from_db(&row.get::<_, String>(5)?),
                declaration_version: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(LedgerError::from)
}

pub fn ensure_series(
    conn: &Connection, system_id: &SystemId, measurement_key: &str,
    channel_index: i32, variant: &str, quarantined: bool,
) -> Result<i64, LedgerError> {
    if let Some(id) = conn.query_row(
        "SELECT series_id FROM series
         WHERE system_id = ?1 AND measurement_key = ?2 AND channel_index = ?3 AND variant = ?4",
        params![system_id.as_bytes().to_vec(), measurement_key, channel_index, variant],
        |row| row.get::<_, i64>(0),
    ).optional()? {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO series (system_id, measurement_key, channel_index, variant, quarantined, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![system_id.as_bytes().to_vec(), measurement_key, channel_index, variant, quarantined as i32, now_ms()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn record_sighting(conn: &Connection, hardware_id: &str, source: &str) -> Result<(), LedgerError> {
    conn.execute(
        "INSERT INTO sightings (hardware_id, source, first_seen, last_seen, observations)
         VALUES (?1, ?2, ?3, ?3, 1)
         ON CONFLICT(hardware_id) DO UPDATE SET last_seen = ?3, observations = observations + 1",
        params![hardware_id, source, now_ms()],
    )?;
    Ok(())
}

pub fn approve_sighting(
    conn: &Connection, hardware_id: &str, user_label: Option<&str>, kind: DeviceKind,
) -> Result<SystemId, LedgerError> {
    let seen: bool = conn.query_row(
        "SELECT 1 FROM sightings WHERE hardware_id = ?1", params![hardware_id], |_| Ok(true),
    ).optional()?.unwrap_or(false);
    if !seen {
        return Err(LedgerError::NotFound(format!("sighting {hardware_id}")));
    }
    let sid = insert_device(conn, &NewDevice {
        hardware_id: hardware_id.to_string(),
        user_label: user_label.map(String::from),
        parent: None,
        kind,
        initial_state: DeviceState::Quarantined, // D5経路A: 承認→検疫→active
    })?;
    conn.execute("DELETE FROM sightings WHERE hardware_id = ?1", params![hardware_id])?;
    audit(conn, "sighting_approved", Some(&sid), hardware_id)?;
    Ok(sid)
}

pub fn activate_device(conn: &Connection, system_id: &SystemId) -> Result<(), LedgerError> {
    let n = conn.execute(
        "UPDATE devices SET state = 'active' WHERE system_id = ?1 AND state = 'quarantined'",
        params![system_id.as_bytes().to_vec()],
    )?;
    if n == 0 {
        return Err(LedgerError::NotFound(format!("quarantined device {}", system_id.to_text())));
    }
    audit(conn, "device_activated", Some(system_id), "")?;
    Ok(())
}

/// 台帳エポック(D5決定3の複合カーソル (epoch, seq) の前半)。初回に生成し永続化。
pub fn ledger_epoch(conn: &Connection) -> Result<String, LedgerError> {
    if let Some(v) = conn.query_row(
        "SELECT value FROM ledger_meta WHERE key = 'epoch'", [], |row| row.get::<_, String>(0),
    ).optional()? {
        return Ok(v);
    }
    let epoch = uuid::Uuid::now_v7().to_string();
    conn.execute("INSERT INTO ledger_meta (key, value) VALUES ('epoch', ?1)", params![epoch])?;
    Ok(epoch)
}
```

- [ ] **Step 6: テスト通過を確認**

Run: `cargo test -p iotkit-core-ledger && cargo test --workspace`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml core/ledger/
git commit -m "feat(ledger): add device/series ledger with sighting staging and ledger epoch (D5 wave 0)"
```

---

### Task 5: ストレージ拡張(synchronous=NORMAL、readings v3、ingest_dedup+staged_readings)

**Files:**
- Modify: `core/storage/src/lib.rs:16-25`(configure_pragmas: synchronous FULL→NORMAL)
- Create: `core/timeseries/migrations/0004_readings_v3.sql`
- Modify: `core/timeseries/src/lib.rs`(MIGRATIONSにv4追加+`insert_reading_v3`+dedup/staging関数)
- Modify: `core/timeseries/Cargo.toml`(ledgerへの依存は**追加しない**。series_idはi64で受ける)

**Interfaces:**
- Consumes: Task 4の series_id(i64)
- Produces(コレクタが同一トランザクションで呼ぶ、**同期・&Connection受け**の関数群):
  - `pub const MIGRATIONS: &[Migration]`(既存v2に加えv4 "readings_v3" を含む)
  - `try_claim_envelope(conn: &Connection, sender_id: &str, envelope_id: &str) -> Result<bool, TimeseriesError>`(ingest_dedupへINSERT。既存ならfalse=Duplicate)
  - `insert_reading_v3(conn: &Connection, r: &NewReading) -> Result<i64, TimeseriesError>`(戻り値=seq)
  - `NewReading { series_id: i64, received_at_ms: i64, device_time_ms: Option<i64>, time_source: String, values: Vec<f64>, rssi: Option<i16>, battery_pct: Option<u8>, quarantined: bool }`
  - `insert_staged_reading(conn: &Connection, hardware_id: &str, received_at_ms: i64, payload_json: &str) -> Result<(), TimeseriesError>`(hardware_idごと上限1000行、超過は最古削除=有界・パージ可能)
  - `purge_dedup_before(conn: &Connection, cutoff_ms: i64) -> Result<u64, TimeseriesError>`(TTL 72hの実行部)

- [ ] **Step 1: PRAGMAテストを修正して先に失敗させる**

`core/storage/src/lib.rs` のテスト(既存のpragma検証テストがあればそれを、なければ追加)で `synchronous` の期待値を NORMAL(=1) に:

```rust
#[test]
fn pragmas_use_wal_and_normal_sync() {
    let db = init_db_memory(&[]).unwrap();
    db.with_conn_sync(|conn| {
        let sync: i64 = conn.query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap();
        assert_eq!(sync, 1, "synchronous must be NORMAL (D1)");
        Ok(())
    })
    .unwrap();
}
```

Run: `cargo test -p iotkit-core-storage pragmas_use_wal_and_normal_sync`
Expected: FAIL(現行はFULL=2)

注: in-memory DBはjournal_mode=walにならない(memory固定)ため、このテストではsynchronousのみ検証する。

- [ ] **Step 2: configure_pragmasを変更して通す**

`core/storage/src/lib.rs` の `synchronous = FULL` を `synchronous = NORMAL` に変更(D1: WALでは安全十分・SD fsync負荷低減)。

Run: `cargo test -p iotkit-core-storage`
Expected: PASS

- [ ] **Step 3: readings v3マイグレーションを書く**

`core/timeseries/migrations/0004_readings_v3.sql`:

```sql
-- D1フェーズ1.5: series_id FK・挿入順単調seq・時刻を一意性に使わない(旧v2の同一ms暗黙dedupはD1と矛盾)
CREATE TABLE readings (
    seq            INTEGER PRIMARY KEY AUTOINCREMENT,  -- 出口カーソル(epoch, seq)の後半(D5決定3)
    series_id      INTEGER NOT NULL REFERENCES series(series_id),
    received_at    INTEGER NOT NULL,                   -- コレクタが必ず付与(D1)
    device_time    INTEGER,                            -- デバイス申告時刻(任意)
    time_source    TEXT NOT NULL,
    time_quality   TEXT NOT NULL DEFAULT 'unsynced',   -- R18受信側刻印。Wave 0は既定値固定(D3境界の明文化・
                                                       -- 外部レビュー第2回反映。NTP状態評価はWave 1、列だけ初日から)
    values_json    TEXT NOT NULL,
    rssi           INTEGER,
    battery_pct    INTEGER,
    quarantined    INTEGER NOT NULL DEFAULT 0          -- 値域外・未知キー等の行レベル検疫(D1/D6)
);
CREATE INDEX idx_readings_series_time ON readings(series_id, received_at);

-- D1: dedupキー=(認証済み送信者, envelope_id)。TTL+サイズ上限で有界
CREATE TABLE ingest_dedup (
    sender_id    TEXT NOT NULL,
    envelope_id  TEXT NOT NULL,
    received_at  INTEGER NOT NULL,
    PRIMARY KEY (sender_id, envelope_id)
);
CREATE INDEX idx_ingest_dedup_time ON ingest_dedup(received_at);

-- D5経路A: 目撃ステージング中のデータ保持(有界・パージ可能。承認時に本流化)
CREATE TABLE staged_readings (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    hardware_id  TEXT NOT NULL,
    received_at  INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX idx_staged_hw ON staged_readings(hardware_id, id);
```

`core/timeseries/src/lib.rs` のMIGRATIONSを2要素に:

```rust
pub const MIGRATIONS: &[Migration] = &[
    Migration { version: 2, label: "timeseries", sql: include_str!("../migrations/0002_timeseries.sql") },
    Migration { version: 4, label: "readings_v3", sql: include_str!("../migrations/0004_readings_v3.sql") },
];
```

(gateway側でv3=ledgerを間に挟んで連結する。versionは昇順検証があるため 1,2,3,4 の順に並べて渡す=Task 7)

- [ ] **Step 4: 新関数の失敗テストを書く**

`core/timeseries/src/lib.rs` テストモジュールに追加(テストDBはstorage v1+**ダミーseries表**が要るため、ledger MIGRATIONSと同等の最小 `CREATE TABLE series/devices` をテスト内SQLで作るのではなく、**dev-dependencyに `iotkit-core-ledger` を追加**して本物のMIGRATIONSで組む):

`core/timeseries/Cargo.toml` の `[dev-dependencies]` に `iotkit-core-ledger = { path = "../ledger" }` を追加(本依存には足さない=クレート依存の方向を保つ)。

```rust
#[cfg(test)]
mod v3_tests {
    use super::*;
    use iotkit_core_ledger as ledger;
    use iotkit_core_storage::init_db_memory;

    fn v3_db() -> iotkit_core_storage::DbHandle {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(ledger::MIGRATIONS);
        all.extend_from_slice(crate::MIGRATIONS); // v2, v4
        // 昇順必須: 1(ledgerなし), 2, 3, 4 の順に並べ替え
        all.sort_by_key(|m| m.version);
        init_db_memory(&all).unwrap()
    }

    fn seed_series(conn: &rusqlite::Connection) -> i64 {
        let sid = ledger::insert_device(conn, &ledger::NewDevice {
            hardware_id: "ble:aa".into(), user_label: None, parent: None,
            kind: ledger::DeviceKind::Individual,
            initial_state: ledger::DeviceState::Active,
        }).unwrap();
        ledger::ensure_series(conn, &sid, "temperature_c", -1, "primary", false).unwrap()
    }

    #[test]
    fn claim_envelope_detects_duplicates() {
        let db = v3_db();
        db.with_conn_sync(|conn| {
            assert!(try_claim_envelope(conn, "adapterA", "e-1").unwrap());
            assert!(!try_claim_envelope(conn, "adapterA", "e-1").unwrap());
            assert!(try_claim_envelope(conn, "adapterB", "e-1").unwrap()); // 送信者スコープ(D1)
            Ok(())
        }).unwrap();
    }

    #[test]
    fn insert_reading_v3_returns_monotonic_seq() {
        let db = v3_db();
        db.with_conn_sync(|conn| {
            let series_id = seed_series(conn);
            let r = NewReading {
                series_id, received_at_ms: 1000, device_time_ms: None,
                time_source: "gateway".into(), values: vec![21.5],
                rssi: None, battery_pct: None, quarantined: false,
            };
            let s1 = insert_reading_v3(conn, &r).unwrap();
            let s2 = insert_reading_v3(conn, &r).unwrap(); // 同時刻・同値でも別行(v2の暗黙dedup廃止)
            assert!(s2 > s1);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn staged_readings_are_bounded_per_hardware_id() {
        let db = v3_db();
        db.with_conn_sync(|conn| {
            for i in 0..1005 {
                insert_staged_reading(conn, "ble:new", i, "{}").unwrap();
            }
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM staged_readings WHERE hardware_id='ble:new'", [], |r| r.get(0),
            ).unwrap();
            assert_eq!(n, 1000);
            let oldest: i64 = conn.query_row(
                "SELECT MIN(received_at) FROM staged_readings WHERE hardware_id='ble:new'", [], |r| r.get(0),
            ).unwrap();
            assert_eq!(oldest, 5); // 最古削除
            Ok(())
        }).unwrap();
    }

    #[test]
    fn purge_dedup_before_removes_old_entries() {
        let db = v3_db();
        db.with_conn_sync(|conn| {
            try_claim_envelope(conn, "a", "old").unwrap();
            conn.execute("UPDATE ingest_dedup SET received_at = 0", []).unwrap();
            try_claim_envelope(conn, "a", "new").unwrap();
            assert_eq!(purge_dedup_before(conn, 1).unwrap(), 1);
            Ok(())
        }).unwrap();
    }
}
```

- [ ] **Step 5: 失敗を確認して実装**

Run: `cargo test -p iotkit-core-timeseries`(FAIL確認)

`core/timeseries/src/lib.rs` に追加:

```rust
pub struct NewReading {
    pub series_id: i64,
    pub received_at_ms: i64,
    pub device_time_ms: Option<i64>,
    pub time_source: String,
    pub values: Vec<f64>,
    pub rssi: Option<i16>,
    pub battery_pct: Option<u8>,
    pub quarantined: bool,
}

pub const STAGED_READINGS_CAP_PER_HW: i64 = 1000;

pub fn try_claim_envelope(
    conn: &rusqlite::Connection, sender_id: &str, envelope_id: &str,
) -> Result<bool, TimeseriesError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let n = conn.execute(
        "INSERT INTO ingest_dedup (sender_id, envelope_id, received_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(sender_id, envelope_id) DO NOTHING",
        rusqlite::params![sender_id, envelope_id, now],
    ).map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    Ok(n == 1)
}

pub fn insert_reading_v3(
    conn: &rusqlite::Connection, r: &NewReading,
) -> Result<i64, TimeseriesError> {
    for v in &r.values {
        if !v.is_finite() {
            return Err(TimeseriesError::InvalidReading(format!("non-finite value {v}")));
        }
    }
    let values_json = serde_json::to_string(&r.values)
        .map_err(|e| TimeseriesError::InvalidReading(e.to_string()))?;
    conn.execute(
        "INSERT INTO readings (series_id, received_at, device_time, time_source, values_json, rssi, battery_pct, quarantined)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            r.series_id, r.received_at_ms, r.device_time_ms, r.time_source,
            values_json, r.rssi, r.battery_pct, r.quarantined as i32
        ],
    ).map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    Ok(conn.last_insert_rowid())
}

pub fn insert_staged_reading(
    conn: &rusqlite::Connection, hardware_id: &str, received_at_ms: i64, payload_json: &str,
) -> Result<(), TimeseriesError> {
    conn.execute(
        "INSERT INTO staged_readings (hardware_id, received_at, payload_json) VALUES (?1, ?2, ?3)",
        rusqlite::params![hardware_id, received_at_ms, payload_json],
    ).map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    conn.execute(
        "DELETE FROM staged_readings WHERE hardware_id = ?1 AND id NOT IN (
            SELECT id FROM staged_readings WHERE hardware_id = ?1 ORDER BY id DESC LIMIT ?2)",
        rusqlite::params![hardware_id, STAGED_READINGS_CAP_PER_HW],
    ).map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    Ok(())
}

pub fn purge_dedup_before(
    conn: &rusqlite::Connection, cutoff_ms: i64,
) -> Result<u64, TimeseriesError> {
    let n = conn.execute(
        "DELETE FROM ingest_dedup WHERE received_at < ?1", rusqlite::params![cutoff_ms],
    ).map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    Ok(n as u64)
}
```

(StorageErrorのvariant名は実物に合わせること: `core/storage/src/error.rs:3-17` では `Sqlite(rusqlite::Error)`)

- [ ] **Step 6: テスト通過を確認**

Run: `cargo test -p iotkit-core-timeseries && cargo test --workspace`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add core/storage/ core/timeseries/
git commit -m "feat(timeseries): readings v3 with series_id FK + ingest_dedup + staged_readings; storage synchronous=NORMAL"
```

---

### Task 6: core/collector クレート新設(R8: 受理の権威)

**Files:**
- Create: `core/collector/Cargo.toml`
- Create: `core/collector/src/lib.rs`
- Create: `core/collector/src/actor.rs`
- Create: `core/collector/src/registry_policy.rs`
- Modify: ルート `Cargo.toml`(members追加)

**Interfaces:**
- Consumes: Task 2の契約型、Task 4の台帳関数、Task 5のv3関数
- Produces(gatewayとCLI、計画2が使う):
  - `Collector`(Clone可能なハンドル)
    - `Collector::spawn(db: DbHandle, policy: Arc<dyn RegistryPolicy>, queue_cap: usize) -> (Collector, tokio::task::JoinHandle<()>)`
    - `async fn submit(&self, envelope: Envelope) -> Result<EnvelopeAck, CollectorClosed>`(**ackはコミット後にのみ返る**)
  - `pub struct CollectorClosed;`
  - `trait RegistryPolicy: Send + Sync { fn evaluate(&self, item: &ReadingItem) -> RegistryVerdict; }`
  - `enum RegistryVerdict { Accept { quarantine: bool }, RejectItem { reason_code: ReasonCode, message: String } }`
  - `PermissiveRegistry`(計画1の暫定実装: measurement_key文法検証のみ。文法違反→RejectItem(MalformedMeasurementKey)、それ以外→Accept{quarantine:false}。**計画2が現場レジストリ実装に差し替える**)
  - `pub const MAX_ITEMS_PER_ENVELOPE: usize = 256;`

- [ ] **Step 1: クレート骨格**

`core/collector/Cargo.toml`:

```toml
[package]
name = "iotkit-core-collector"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-ingest-contract = { path = "../../iotkit-ingest-contract" }
iotkit-core-ledger = { path = "../ledger" }
iotkit-core-storage = { path = "../storage" }
iotkit-core-timeseries = { path = "../timeseries" }
rusqlite = { version = "0.32", features = ["bundled"] }
serde_json = "1"
tokio = { version = "1", features = ["sync", "rt", "macros", "time"] }
tracing = "0.1"

[dev-dependencies]
iotkit-core-storage = { path = "../storage", features = ["test-util"] }
tokio = { version = "1", features = ["sync", "rt", "macros", "time", "rt-multi-thread"] }
```

ルート `Cargo.toml` members に `"core/collector"` を追加。

- [ ] **Step 2: 受理シナリオの失敗テストを書く(適合テストの原型)**

`core/collector/src/actor.rs` のテストモジュール:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry_policy::PermissiveRegistry;
    use iotkit_core_ledger as ledger;
    use iotkit_ingest_contract::*;
    use std::sync::Arc;

    fn test_db() -> iotkit_core_storage::DbHandle {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
        all.sort_by_key(|m| m.version);
        iotkit_core_storage::init_db_memory(&all).unwrap()
    }

    fn env(id: &str, hw: &str, key: &str) -> Envelope {
        Envelope {
            envelope_id: id.into(),
            source: "test-adapter".into(),
            declaration_version: None,
            items: vec![ReadingItem {
                subject_hint: Some(hw.into()),
                measurement_key: key.into(),
                channel_index: None,
                series_variant: None,
                values: vec![1.0],
                device_time_ms: None,
                time_source: TimeSource::Gateway,
                age_ms: None, rssi: None, battery_pct: None,
            }],
        }
    }

    fn register_active(db: &iotkit_core_storage::DbHandle, hw: &str) {
        db.with_conn_sync(|conn| {
            ledger::insert_device(conn, &ledger::NewDevice {
                hardware_id: hw.into(), user_label: None, parent: None,
                kind: ledger::DeviceKind::Individual,
                initial_state: ledger::DeviceState::Active,
            }).unwrap();
            Ok(())
        }).unwrap();
    }

    #[tokio::test]
    async fn known_subject_is_accepted_durable_and_row_exists_before_ack_returns() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let ack = collector.submit(env("e-1", "ble:aa", "temperature_c")).await.unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable })));
        // ack = 耐久点: ackが返った時点で行が存在する(D1)
        let n: i64 = db.with_conn_sync(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap())
        }).unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn duplicate_envelope_is_reported_and_not_written_twice() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let e = env("e-dup", "ble:aa", "temperature_c");
        let a1 = collector.submit(e.clone()).await.unwrap();
        let a2 = collector.submit(e).await.unwrap();
        assert!(matches!(a1.status, AckStatus::Accepted { .. }));
        assert!(matches!(a2.status, AckStatus::Duplicate));
        let n: i64 = db.with_conn_sync(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap())
        }).unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn unknown_subject_goes_to_sighting_staging_with_staged_disposition() {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let ack = collector.submit(env("e-2", "ble:unknown", "temperature_c")).await.unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Staged })));
        let (sightings, staged): (i64, i64) = db.with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM sightings", [], |r| r.get(0)).unwrap(),
                conn.query_row("SELECT COUNT(*) FROM staged_readings", [], |r| r.get(0)).unwrap(),
            ))
        }).unwrap();
        assert_eq!((sightings, staged), (1, 1));
    }

    #[tokio::test]
    async fn malformed_measurement_key_rejects_item_but_stores_valid_sibling() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut e = env("e-3", "ble:aa", "temperature_c");
        let mut bad = e.items[0].clone();
        bad.measurement_key = "Bad:Key".into();
        e.items.push(bad);
        let ack = collector.submit(e).await.unwrap();
        let AckStatus::Accepted { items } = ack.status else { panic!("expected Accepted") };
        assert!(matches!(items[0], ItemStatus::Stored { .. }));
        assert!(matches!(items[1],
            ItemStatus::ItemRejected { reason_code: ReasonCode::MalformedMeasurementKey, .. }));
    }

    #[tokio::test]
    async fn missing_subject_hint_is_rejected() {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut e = env("e-4", "ble:aa", "temperature_c");
        e.items[0].subject_hint = None; // ブリッジは多subject送信者なのでhint必須(D5決定1)
        let ack = collector.submit(e).await.unwrap();
        let AckStatus::Accepted { items } = ack.status else { panic!("expected Accepted") };
        assert!(matches!(items[0],
            ItemStatus::ItemRejected { reason_code: ReasonCode::UnknownSubject, .. }));
    }

    #[tokio::test]
    async fn oversized_envelope_is_rejected_whole() {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut e = env("e-5", "ble:aa", "temperature_c");
        let item = e.items[0].clone();
        e.items = std::iter::repeat_with(|| item.clone()).take(MAX_ITEMS_PER_ENVELOPE + 1).collect();
        let ack = collector.submit(e).await.unwrap();
        assert!(matches!(ack.status,
            AckStatus::Rejected { reason_code: ReasonCode::BatchTooLarge, .. }));
    }
}
```

- [ ] **Step 3: 失敗を確認**

Run: `cargo test -p iotkit-core-collector`
Expected: FAIL(Collector未定義)

- [ ] **Step 4: registry_policy を実装**

`core/collector/src/registry_policy.rs`:

```rust
use iotkit_ingest_contract::{validate_measurement_key, ReadingItem, ReasonCode};

#[derive(Debug, Clone)]
pub enum RegistryVerdict {
    Accept { quarantine: bool },
    RejectItem { reason_code: ReasonCode, message: String },
}

/// 受理時のレジストリ検証フック。計画2(D6現場レジストリ)が本実装を差し込む。
pub trait RegistryPolicy: Send + Sync {
    fn evaluate(&self, item: &ReadingItem) -> RegistryVerdict;
}

/// 計画1の暫定実装: 文法検証のみ(D6決定2)。値域・未知キー検疫は計画2で。
pub struct PermissiveRegistry;

impl RegistryPolicy for PermissiveRegistry {
    fn evaluate(&self, item: &ReadingItem) -> RegistryVerdict {
        match validate_measurement_key(&item.measurement_key) {
            Ok(()) => RegistryVerdict::Accept { quarantine: false },
            Err(e) => RegistryVerdict::RejectItem {
                reason_code: ReasonCode::MalformedMeasurementKey,
                message: e.to_string(),
            },
        }
    }
}
```

- [ ] **Step 5: actor を実装**

`core/collector/src/actor.rs` 本体(D1のactor request-reply定石。キャッシュはクロージャに移動して戻すパターン):

```rust
use crate::registry_policy::{RegistryPolicy, RegistryVerdict};
use iotkit_core_ledger as ledger;
use iotkit_core_storage::DbHandle;
use iotkit_core_timeseries as ts;
use iotkit_ingest_contract::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

pub const MAX_ITEMS_PER_ENVELOPE: usize = 256;

pub struct IngestRequest {
    pub envelope: Envelope,
    pub ack_tx: oneshot::Sender<EnvelopeAck>,
}

#[derive(Clone)]
pub struct Collector {
    tx: mpsc::Sender<IngestRequest>,
}

#[derive(Debug)]
pub struct CollectorClosed;

/// タスク所有キャッシュ(D5: 起動時全ロードはWave 0では行数が小さいため遅延ロードで開始し、
/// ミス時にDBを引く。台帳変異は必ずコレクタ経由なので無効化漏れは構造上起きない)
#[derive(Default)]
struct ResolutionCache {
    devices: HashMap<String, (ledger::SystemId, ledger::DeviceState)>, // hardware_id →
    series: HashMap<(ledger::SystemId, String, i32, String), i64>,
}

impl Collector {
    pub fn spawn(
        db: DbHandle,
        policy: Arc<dyn RegistryPolicy>,
        queue_cap: usize,
    ) -> (Collector, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<IngestRequest>(queue_cap);
        let handle = tokio::spawn(async move {
            let mut cache = ResolutionCache::default();
            while let Some(req) = rx.recv().await {
                let taken = std::mem::take(&mut cache);
                let policy = Arc::clone(&policy);
                let envelope = req.envelope;
                let result = db
                    .with_conn(move |conn| {
                        let mut c = taken;
                        let ack = process_envelope(conn, &mut c, policy.as_ref(), &envelope);
                        Ok((ack, c))
                    })
                    .await;
                match result {
                    Ok((ack, c)) => {
                        cache = c;
                        let _ = req.ack_tx.send(ack);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "collector: storage failure");
                        // ack_tx をドロップ = 送信側はタイムアウトで再送(ackなし=未耐久、D1と整合)
                    }
                }
            }
        });
        (Collector { tx }, handle)
    }

    pub async fn submit(&self, envelope: Envelope) -> Result<EnvelopeAck, CollectorClosed> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(IngestRequest { envelope, ack_tx })
            .await
            .map_err(|_| CollectorClosed)?;
        ack_rx.await.map_err(|_| CollectorClosed)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 1エンベロープの受理。**全体が単一トランザクション**(dedup+全item書き込み=ack耐久点、D1)。
fn process_envelope(
    conn: &rusqlite::Connection,
    cache: &mut ResolutionCache,
    policy: &dyn RegistryPolicy,
    envelope: &Envelope,
) -> EnvelopeAck {
    let eid = envelope.envelope_id.clone();
    if envelope.items.len() > MAX_ITEMS_PER_ENVELOPE {
        return EnvelopeAck {
            envelope_id: eid,
            status: AckStatus::Rejected {
                reason_code: ReasonCode::BatchTooLarge,
                message: format!("items {} > {}", envelope.items.len(), MAX_ITEMS_PER_ENVELOPE),
            },
        };
    }
    let tx = match conn.unchecked_transaction() {
        Ok(t) => t,
        Err(e) => return internal_reject(eid, &e.to_string()),
    };
    match ts::try_claim_envelope(&tx, &envelope.source, &envelope.envelope_id) {
        Ok(true) => {}
        Ok(false) => {
            drop(tx); // dedup判定のみ・書き込みなし
            return EnvelopeAck { envelope_id: eid, status: AckStatus::Duplicate };
        }
        Err(e) => return internal_reject(eid, &e.to_string()),
    }
    let received_at = now_ms();
    let mut item_statuses = Vec::with_capacity(envelope.items.len());
    for item in &envelope.items {
        item_statuses.push(process_item(&tx, cache, policy, envelope, item, received_at));
    }
    if let Err(e) = tx.commit() {
        return internal_reject(eid, &e.to_string());
    }
    EnvelopeAck { envelope_id: eid, status: AckStatus::Accepted { items: item_statuses } }
}

fn process_item(
    conn: &rusqlite::Connection,
    cache: &mut ResolutionCache,
    policy: &dyn RegistryPolicy,
    envelope: &Envelope,
    item: &ReadingItem,
    received_at: i64,
) -> ItemStatus {
    // 1) レジストリ検証(文法。計画2で値域・未知キー判定に拡張)
    let quarantine = match policy.evaluate(item) {
        RegistryVerdict::Accept { quarantine } => quarantine,
        RegistryVerdict::RejectItem { reason_code, message } => {
            return ItemStatus::ItemRejected { reason_code, message };
        }
    };
    // 2) subject解決(D5決定1: 送信者+subject_hint→台帳)
    let Some(hw) = item.subject_hint.as_deref() else {
        return ItemStatus::ItemRejected {
            reason_code: ReasonCode::UnknownSubject,
            message: "subject_hint required for multi-subject sender".into(),
        };
    };
    let resolved = match cache.devices.get(hw) {
        Some(hit) => Some(*hit),
        None => match ledger::find_alive_by_hardware_id(conn, hw) {
            Ok(Some(row)) => {
                cache.devices.insert(hw.to_string(), (row.system_id, row.state));
                Some((row.system_id, row.state))
            }
            Ok(None) => None,
            Err(e) => {
                return ItemStatus::ItemRejected {
                    reason_code: ReasonCode::Internal, message: e.to_string(),
                };
            }
        },
    };
    let Some((system_id, state)) = resolved else {
        // 3) 未知subject → 目撃ステージング(D5決定4経路A、ack=staged)
        let payload = serde_json::to_string(item).unwrap_or_else(|_| "{}".into());
        if let Err(e) = ledger::record_sighting(conn, hw, &envelope.source)
            .map_err(|e| e.to_string())
            .and_then(|_| ts::insert_staged_reading(conn, hw, received_at, &payload).map_err(|e| e.to_string()))
        {
            return ItemStatus::ItemRejected { reason_code: ReasonCode::Internal, message: e };
        }
        return ItemStatus::Stored { disposition: Disposition::Staged };
    };
    // 4) series解決(検疫デバイスのデータは検疫行として保存=D1オンボーディング)
    let device_quarantined = state == ledger::DeviceState::Quarantined;
    let channel: i32 = item.channel_index.map(i32::from).unwrap_or(-1);
    let variant = item.series_variant.as_deref().unwrap_or("primary").to_string();
    let skey = (system_id, item.measurement_key.clone(), channel, variant.clone());
    let series_id = match cache.series.get(&skey) {
        Some(id) => *id,
        None => match ledger::ensure_series(conn, &system_id, &item.measurement_key, channel, &variant, false) {
            Ok(id) => { cache.series.insert(skey, id); id }
            Err(e) => {
                return ItemStatus::ItemRejected {
                    reason_code: ReasonCode::Internal, message: e.to_string(),
                };
            }
        },
    };
    // 5) 書き込み
    let row_quarantined = quarantine || device_quarantined;
    let time_source = match item.time_source {
        TimeSource::DeviceNtp => "device_ntp", TimeSource::DeviceRtc => "device_rtc",
        TimeSource::Gateway => "gateway", TimeSource::GatewayAdjusted => "gateway_adjusted",
    };
    let new = ts::NewReading {
        series_id,
        received_at_ms: received_at,
        device_time_ms: item.device_time_ms,
        time_source: time_source.to_string(),
        values: item.values.clone(),
        rssi: item.rssi,
        battery_pct: item.battery_pct,
        quarantined: row_quarantined,
    };
    match ts::insert_reading_v3(conn, &new) {
        Ok(_seq) => ItemStatus::Stored {
            disposition: if row_quarantined { Disposition::Quarantined } else { Disposition::Durable },
        },
        Err(e) => ItemStatus::ItemRejected { reason_code: ReasonCode::Internal, message: e.to_string() },
    }
}

fn internal_reject(envelope_id: String, msg: &str) -> EnvelopeAck {
    EnvelopeAck {
        envelope_id,
        status: AckStatus::Rejected {
            reason_code: ReasonCode::Internal,
            message: msg.to_string(),
        },
    }
}
```

`src/lib.rs`:

```rust
pub mod actor;
pub mod registry_policy;

pub use actor::{Collector, CollectorClosed, IngestRequest, MAX_ITEMS_PER_ENVELOPE};
pub use registry_policy::{PermissiveRegistry, RegistryPolicy, RegistryVerdict};
```

実装ノート:
- **Deferredはこのコレクタからは決して返さない**(D1: プロセス内バインディングではmpscの
  `send().await` 自体が逆圧であり、`Deferred` はHTTP/UDSバインディング(Wave 1)専用の意味論。
  契約型に存在するのはワイヤ契約の完全性のため)。queue fullで待つのは正しい挙動。
- `unchecked_transaction` は `&Connection` からトランザクションを作るrusqlite API(DbHandleがConnectionを`&`でしか貸さないため)。
- Duplicateのとき `drop(tx)` はロールバックだが書き込みゼロなので正しい。dedup行自体は**最初のaccepted時のトランザクション内**で入っている。
- item単位rejectはトランザクションをロールバックしない(部分受理=D1のall-or-nothing禁止をitemレベルに適用)。

- [ ] **Step 6: テスト通過を確認**

Run: `cargo test -p iotkit-core-collector && cargo test --workspace`
Expected: PASS(6シナリオすべて)

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml core/collector/
git commit -m "feat(collector): ingest actor with ledger resolution, dedup, durable ack, registry hook (R8)"
```

---

### Task 7: ゲートウェイ配線(暫定ブリッジ+位置型デバイス起動時登録)

**Files:**
- Create: `iotkit-gateway/src/bridge.rs`
- Modify: `iotkit-gateway/src/main.rs`(migrations連結、コレクタspawn、fan-inループの書き込み経路差し替え、位置型デバイスの起動時登録)
- Modify: `iotkit-gateway/Cargo.toml`(依存追加: iotkit-ingest-contract, iotkit-core-collector, iotkit-core-ledger, uuid v4)

**Interfaces:**
- Consumes: Task 2/4/6 の全公開型
- Produces: `bridge::adapter_event_to_envelope(adapter_id: &AdapterId, device_key: &DeviceKey, reading: &SensorReading, rssi: Option<i16>, battery_pct: Option<u8>) -> Option<Envelope>`(計画3でアダプタ内取り込みクライアントに置き換わる**明示的に暫定**の写像)

このブリッジは、AdapterEvent(旧語彙)→Envelope(新契約)の翻訳をゲートウェイ内で行う移行足場。D4の最終形(アダプタランタイム+取り込みクライアント)は計画3で実現し、このファイルは計画3で削除される。

- [ ] **Step 1: ブリッジ写像の失敗テストを書く**

`iotkit-gateway/src/bridge.rs` テスト:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::{AdapterId, DeviceKey, SensorReading, SensorType};

    #[test]
    fn bravepi_key_maps_to_ble_hardware_id_and_d6_key() {
        let e = adapter_event_to_envelope(
            &AdapterId::new("bravepi-mainboard:/dev/ttyAMA0"),
            &DeviceKey::new("bravepi-mainboard:00000000000000ab:temperature"),
            &SensorReading::new(SensorType::Temperature, vec![21.5], vec!["temp".into()]),
            Some(-60), Some(90),
        ).unwrap();
        // 実物のBravePI AdapterIdは "bravepi-mainboard:{port_path}"(handle.rs:109)
        assert_eq!(e.source, "bravepi-mainboard:/dev/ttyAMA0");
        assert_eq!(e.items.len(), 1);
        let item = &e.items[0];
        assert_eq!(item.subject_hint.as_deref(), Some("ble:00000000000000ab"));
        assert_eq!(item.measurement_key, "temperature_c");
        assert_eq!(item.channel_index, None);
        assert_eq!(item.values, vec![21.5]);
    }

    #[test]
    fn i2c_key_maps_to_sender_scoped_positional_hardware_id() {
        let e = adapter_event_to_envelope(
            &AdapterId::new("rpi-local:default"),
            &DeviceKey::new("i2c:0x44:illuminance"),
            &SensorReading::new(SensorType::Illuminance, vec![512.0], vec!["lux".into()]),
            None, None,
        ).unwrap();
        // 位置識別型は送信者スコープを含む(D5決定2)
        assert_eq!(e.items[0].subject_hint.as_deref(), Some("rpi-local:default:i2c:0x44"));
        assert_eq!(e.items[0].measurement_key, "illuminance_lux");
    }

    #[test]
    fn multi_value_reading_becomes_per_channel_items() {
        let e = adapter_event_to_envelope(
            &AdapterId::new("bravepi-mainboard:/dev/ttyAMA0"),
            &DeviceKey::new("bravepi-mainboard:00000000000000cc:acceleration"),
            &SensorReading::new(SensorType::Acceleration, vec![1.0, 2.0, 3.0],
                vec!["x".into(), "y".into(), "z".into()]),
            Some(-55), Some(80),
        ).unwrap();
        assert_eq!(e.items.len(), 3);
        assert_eq!(e.items[0].channel_index, Some(0));
        assert_eq!(e.items[2].channel_index, Some(2));
        assert_eq!(e.items[2].values, vec![3.0]);
        assert_eq!(e.items[2].measurement_key, "acceleration_mg");
    }

    #[test]
    fn unknown_sensor_type_returns_none() {
        let r = SensorReading::new(SensorType::Unknown("mystery".into()), vec![1.0], vec![]);
        assert!(adapter_event_to_envelope(
            &AdapterId::new("a"), &DeviceKey::new("a:b:c"), &r, None, None
        ).is_none());
    }
}
```

- [ ] **Step 2: 失敗を確認して実装**

Run: `cargo test -p iotkit-gateway bridge`(FAIL確認)

`iotkit-gateway/src/bridge.rs` 本体:

```rust
//! 暫定ブリッジ(計画3でアダプタ内取り込みクライアントに置き換え、削除予定)。
//! AdapterEvent(旧語彙)→ 取り込み契約Envelope の翻訳と、
//! SensorType → D6初期語彙measurement_key の写像。
use iotkit_core_types::{AdapterId, DeviceKey, SensorReading, SensorType};
use iotkit_ingest_contract::{Envelope, ReadingItem, TimeSource};

/// D6決定11の初期語彙への写像
fn measurement_key_for(sensor_type: &SensorType) -> Option<&'static str> {
    Some(match sensor_type {
        SensorType::ContactInput => "contact_state",
        SensorType::ContactOutput => "contact_output_state",
        SensorType::Adc => "voltage_mv",
        SensorType::Ranging => "distance_mm",
        SensorType::Temperature => "temperature_c",
        SensorType::Acceleration => "acceleration_mg",
        SensorType::DifferentialPressure => "differential_pressure_pa",
        SensorType::Illuminance => "illuminance_lux",
        SensorType::Unknown(_) => return None,
    })
}

/// DeviceKey → hardware_id 正規形(D5決定2)
/// - BravePI: "bravepi-mainboard:{device_number}:{suffix}" → 個体識別型 "ble:{device_number}"
/// - I2Cポーリング: "i2c:0x44:{suffix}" → 位置識別型(送信者スコープ付き) "{adapter_id}:i2c:0x44"
fn hardware_id_for(adapter_id: &AdapterId, device_key: &DeviceKey) -> Option<String> {
    let parts: Vec<&str> = device_key.as_str().split(':').collect();
    match parts.as_slice() {
        ["bravepi-mainboard", device_number, _suffix] => Some(format!("ble:{device_number}")),
        ["i2c", addr, _suffix] => Some(format!("{}:i2c:{addr}", adapter_id.as_str())),
        _ => None,
    }
}

pub fn adapter_event_to_envelope(
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    reading: &SensorReading,
    rssi: Option<i16>,
    battery_pct: Option<u8>,
) -> Option<Envelope> {
    let key = measurement_key_for(&reading.sensor_type)?;
    let hw = hardware_id_for(adapter_id, device_key)?;
    let items: Vec<ReadingItem> = if reading.values.len() > 1 {
        reading.values.iter().enumerate().map(|(i, v)| ReadingItem {
            subject_hint: Some(hw.clone()),
            measurement_key: key.to_string(),
            channel_index: Some(i as u16),
            series_variant: None,
            values: vec![*v],
            device_time_ms: None,
            time_source: TimeSource::Gateway,
            age_ms: None, rssi, battery_pct,
        }).collect()
    } else {
        vec![ReadingItem {
            subject_hint: Some(hw),
            measurement_key: key.to_string(),
            channel_index: None,
            series_variant: None,
            values: reading.values.clone(),
            device_time_ms: None,
            time_source: TimeSource::Gateway,
            age_ms: None, rssi, battery_pct,
        }]
    };
    Some(Envelope {
        envelope_id: uuid::Uuid::new_v4().to_string(), // プロセス内はUUIDv4可(D1)
        source: adapter_id.as_str().to_string(),
        declaration_version: None,
        items,
    })
}
```

`iotkit-gateway/Cargo.toml` に依存追加:

```toml
iotkit-ingest-contract = { path = "../iotkit-ingest-contract" }
iotkit-core-collector = { path = "../core/collector" }
iotkit-core-ledger = { path = "../core/ledger" }
uuid = { version = "1", features = ["v4"] }
```

`main.rs` に `mod bridge;` を追加。

Run: `cargo test -p iotkit-gateway bridge`
Expected: PASS

- [ ] **Step 3: main.rs の配線を差し替える**

`iotkit-gateway/src/main.rs` の変更点(4箇所):

(1) migrations連結(main.rs:44-45付近)を昇順に:

```rust
let mut all_migrations = iotkit_core_storage::MIGRATIONS.to_vec();
all_migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS);      // v3
all_migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);  // v2, v4
all_migrations.sort_by_key(|m| m.version);                             // 1,2,3,4
```

(2) `run()` 冒頭でコレクタをspawnし、位置型デバイスを登録(D5経路B: 定義=登録):

```rust
let (collector, _collector_handle) = iotkit_core_collector::Collector::spawn(
    db.clone(),
    std::sync::Arc::new(iotkit_core_collector::PermissiveRegistry),
    256,
);
if config.rpi_local.is_some() {
    // hardcoded_rpi_local_targets()と同じ2アドレス(0x60, 0x44)を位置型として登録
    db.with_conn(|conn| {
        for (addr, label) in [(0x60u8, "MCP9600 thermocouple"), (0x44u8, "OPT3001 illuminance")] {
            let hw = format!("rpi-local:default:i2c:0x{addr:02x}");
            if iotkit_core_ledger::find_alive_by_hardware_id(conn, &hw)
                .map_err(|e| iotkit_core_storage::StorageError::Sqlite(
                    rusqlite::Error::ModuleError(e.to_string())))?.is_none()
            {
                iotkit_core_ledger::insert_device(conn, &iotkit_core_ledger::NewDevice {
                    hardware_id: hw,
                    user_label: Some(label.to_string()),
                    parent: None,
                    kind: iotkit_core_ledger::DeviceKind::Positional,
                    initial_state: iotkit_core_ledger::DeviceState::Active,
                }).map_err(|e| iotkit_core_storage::StorageError::Sqlite(
                    rusqlite::Error::ModuleError(e.to_string())))?;
            }
        }
        Ok(())
    }).await.expect("positional device registration");
}
```

(エラー変換が不格好な場合、gateway内に `fn ledger_to_storage_err` ヘルパを切ってよい。rusqliteのvariant名はビルドエラーに従い調整——意図は「起動時失敗はexpectで落とす」)

(3) fan-inループ(main.rs:139付近)の SensorData 分岐: 既存の `timeseries::insert_reading(...)` 呼び出しを**削除**し、ブリッジ+コレクタ送信に置き換え:

```rust
// engine.apply(ev) は従来どおり(projectionは旧語彙のまま=D5「engineはWave 0無改修」)
if let Some(envelope) = bridge::adapter_event_to_envelope(&adapter_id, &device_key, &reading, rssi, battery_pct) {
    match tokio::time::timeout(std::time::Duration::from_secs(5), collector.submit(envelope)).await {
        Ok(Ok(ack)) => {
            if !matches!(ack.status, iotkit_ingest_contract::AckStatus::Accepted { .. }
                | iotkit_ingest_contract::AckStatus::Duplicate)
            {
                tracing::warn!(?ack.status, "ingest not accepted");
            }
        }
        Ok(Err(_)) => tracing::error!("collector closed"),
        Err(_) => tracing::error!("collector ack timeout (5s)"), // D1: ackタイムアウト必須
    }
}
```

(4) 旧 `insert_reading` のrate-limitログ変数等、不要になったコードを削除。

- [ ] **Step 4: 手動疎通テスト(統合テスト)を書く**

`iotkit-gateway/src/main.rs` は統合テストしづらいので、fan-in差し替えの検証は既存のgatewayテスト構造に従い、**ブリッジ+コレクタ結合テスト**を `iotkit-gateway/src/bridge.rs` のテストに追加:

```rust
#[tokio::test]
async fn bridge_output_flows_through_collector_to_readings() {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    let db = iotkit_core_storage::init_db_memory(&all).unwrap();
    db.with_conn_sync(|conn| {
        iotkit_core_ledger::insert_device(conn, &iotkit_core_ledger::NewDevice {
            hardware_id: "ble:00000000000000ab".into(),
            user_label: None, parent: None,
            kind: iotkit_core_ledger::DeviceKind::Individual,
            initial_state: iotkit_core_ledger::DeviceState::Active,
        }).unwrap();
        Ok(())
    }).unwrap();
    let (collector, _h) = iotkit_core_collector::Collector::spawn(
        db.clone(), std::sync::Arc::new(iotkit_core_collector::PermissiveRegistry), 16);
    let e = adapter_event_to_envelope(
        &iotkit_core_types::AdapterId::new("bravepi-mainboard:/dev/ttyAMA0"),
        &iotkit_core_types::DeviceKey::new("bravepi-mainboard:00000000000000ab:temperature"),
        &iotkit_core_types::SensorReading::new(
            iotkit_core_types::SensorType::Temperature, vec![21.5], vec!["temp".into()]),
        Some(-60), Some(90),
    ).unwrap();
    let ack = collector.submit(e).await.unwrap();
    assert!(matches!(ack.status, iotkit_ingest_contract::AckStatus::Accepted { .. }));
    let n: i64 = db.with_conn_sync(|conn| {
        Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap())
    }).unwrap();
    assert_eq!(n, 1);
}
```

(このテストのdev-dependencies追加が必要: `iotkit-core-storage` の `test-util` feature)

- [ ] **Step 5: 全テスト通過とビルド確認**

Run: `cargo test --workspace && cargo build -p iotkit-gateway`
Expected: PASS / ビルド成功

- [ ] **Step 6: Commit**

```bash
git add iotkit-gateway/
git commit -m "feat(gateway): route sensor data through ingest collector via transitional bridge; register positional devices at startup"
```

---

### Task 8: タスク監督の強化(JoinSet+panicフック+再起動バックオフ)

**Files:**
- Modify: `iotkit-gateway/src/main.rs`(panicフック設置、AdapterClosed時の再起動ポリシー)
- Modify: `iotkit-gateway/src/adapter_host.rs`(**deregister APIの追加**——現行の `register`(adapter_host.rs:44)は
  `streams` だけでなく `adapters` Vec内の既存IDも重複拒否するため、Closed後に除去しないと再登録が失敗する)
- Create: `iotkit-gateway/src/supervision.rs`

**Interfaces:**
- Consumes: `AdapterHost`(既存)、`AdapterHostEvent::AdapterClosed(AdapterId)`
- Produces:
  - `supervision::RestartPolicy { max_restarts: u32, base_backoff: Duration, max_backoff: Duration }`(既定: 5回 / 1s / 60s)
  - `supervision::RestartTracker`: `fn next_delay(&mut self, id: &AdapterId) -> Option<Duration>`(Noneで永続degraded=D1「N回超で永続degraded+イベント発行」)、`fn note_healthy(&mut self, id: &AdapterId)`(健全稼働で計数リセット)
  - グローバルpanicフック(backtraceをtracing::errorで出す=D1)

- [ ] **Step 1: RestartTrackerの失敗テストを書く**

`iotkit-gateway/src/supervision.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::AdapterId;
    use std::time::Duration;

    #[test]
    fn backoff_grows_exponentially_with_cap_and_exhausts() {
        let policy = RestartPolicy {
            max_restarts: 3,
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(4),
        };
        let mut t = RestartTracker::new(policy);
        let id = AdapterId::new("bravepi-mainboard:/dev/ttyAMA0");
        assert_eq!(t.next_delay(&id), Some(Duration::from_secs(1)));
        assert_eq!(t.next_delay(&id), Some(Duration::from_secs(2)));
        assert_eq!(t.next_delay(&id), Some(Duration::from_secs(4))); // cap
        assert_eq!(t.next_delay(&id), None); // exhausted → 永続degraded
    }

    #[test]
    fn healthy_note_resets_counter() {
        let mut t = RestartTracker::new(RestartPolicy::default());
        let id = AdapterId::new("a");
        t.next_delay(&id);
        t.note_healthy(&id);
        assert_eq!(t.next_delay(&id), Some(RestartPolicy::default().base_backoff));
    }
}
```

- [ ] **Step 2: 失敗を確認して実装**

Run: `cargo test -p iotkit-gateway supervision`(FAIL確認)

`supervision.rs` 本体:

```rust
//! R20: アプリレベル監督。プロセスレベルはsystemdに委譲(責務台帳)。
use iotkit_core_types::AdapterId;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct RestartPolicy {
    pub max_restarts: u32,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 5,
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
        }
    }
}

pub struct RestartTracker {
    policy: RestartPolicy,
    counts: HashMap<AdapterId, u32>,
}

impl RestartTracker {
    pub fn new(policy: RestartPolicy) -> Self {
        Self { policy, counts: HashMap::new() }
    }

    /// 次の再起動までの待ち時間。予算超過ならNone(永続degraded)。
    pub fn next_delay(&mut self, id: &AdapterId) -> Option<Duration> {
        let count = self.counts.entry(id.clone()).or_insert(0);
        if *count >= self.policy.max_restarts {
            return None;
        }
        let delay = self.policy.base_backoff
            .saturating_mul(2u32.saturating_pow(*count))
            .min(self.policy.max_backoff);
        *count += 1;
        Some(delay)
    }

    pub fn note_healthy(&mut self, id: &AdapterId) {
        self.counts.remove(id);
    }
}

/// D1: グローバルpanicフックでbacktraceをログ(panic="abort"禁止はCargo.toml側で保証)
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(%info, %backtrace, "panic captured");
        default(info);
    }));
}
```

Run: `cargo test -p iotkit-gateway supervision`
Expected: PASS

- [ ] **Step 3: AdapterHostにderegisterを追加(失敗テスト→実装)**

`iotkit-gateway/src/adapter_host.rs` に追加(テスト先行。既存テストモジュールの流儀に合わせる):

```rust
/// Closed済みアダプタを登録簿から除去し、同一IDでの再registerを可能にする。
/// 戻り値: 除去したらtrue。streamsに残っていれば併せて除去する。
pub fn deregister(&mut self, id: &AdapterId) -> bool {
    self.streams.remove(id);
    let before = self.adapters.len();
    self.adapters.retain(|a| a.id() != *id);
    before != self.adapters.len()
}
```

(ManagedAdapterのid取得方法は実物のフィールド構造に従う——`a.id` フィールド直参照ならそれで良い)

テスト(adapter_host.rsの既存 `#[cfg(test)]` 内):

```rust
#[tokio::test]
async fn deregister_allows_reregistration_of_same_id() {
    let mut host = AdapterHost::new();
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    host.register(AdapterId::new("a"), rx, Box::new(|| Box::pin(async { Ok(()) }))).unwrap();
    drop(tx); // チャネルを閉じる
    // AdapterClosedを消費
    while let Some(ev) = host.next_event().await {
        if matches!(ev, AdapterHostEvent::AdapterClosed(_)) { break; }
    }
    assert!(host.deregister(&AdapterId::new("a")));
    let (_tx2, rx2) = tokio::sync::mpsc::channel(4);
    assert!(host.register(AdapterId::new("a"), rx2, Box::new(|| Box::pin(async { Ok(()) }))).is_ok());
}
```

(registerの正確なシグネチャ・戻り値は実物(adapter_host.rs:44)に合わせて調整)

Run: `cargo test -p iotkit-gateway adapter_host`
Expected: 追加テストがderegister未実装でFAIL→実装後PASS

- [ ] **Step 3b: main.rsへ配線**

`main.rs` の変更:
- `main()` 冒頭(tracing初期化直後)に `supervision::install_panic_hook();`
- `run()` に `RestartTracker` を持たせ、`AdapterHostEvent::AdapterClosed(id)` の分岐で
  **まず `host.deregister(&id)` を呼んでから**:
  - BravePI/rpi-localの**公式アダプタのみ**(D4: 再起動権限は形態①のみ)、`tracker.next_delay(&id)` が `Some(d)` なら `tokio::time::sleep(d).await` 後に該当アダプタの `start()`+`host.register()` を再実行(起動時と同じコードパスを`fn start_bravepi(...)`/`fn start_rpi_local(...)`に関数抽出して共用)
  - `None` なら `tracing::error!(adapter = %id, "adapter permanently degraded")` を出して再起動しない(プロセスは他アダプタのために生き続ける)
  - 正常受信イベントを一定回数観測したら `note_healthy`(実装簡略化: SensorData受信のたびに呼んでよい——HashMap::removeは冪等で安価)
- ジッタ: `sleep` 前に `d + Duration::from_millis(u64::from(std::process::id() % 1000))` 程度の決定的オフセットで可(D1のジッタ要件は同時再送ストーム対策。単一プロセス内では簡易で足りる旨をコメント)

- [ ] **Step 4: workspace panic設定の検査テスト**

`iotkit-gateway/src/supervision.rs` のテストに追加:

```rust
#[test]
fn workspace_does_not_use_panic_abort() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(!toml.contains("panic = \"abort\""), "panic=abort breaks task supervision (D1)");
}
```

- [ ] **Step 5: 全テスト通過を確認**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add iotkit-gateway/
git commit -m "feat(gateway): adapter restart supervision with exponential backoff and global panic hook (R20)"
```

---

## 計画1完了の受け入れ確認(最終チェック)

- [ ] `cargo test --workspace` 全緑(オプションなし・並列)
- [ ] `cargo build --release -p iotkit-gateway` 成功
- [ ] 適合シナリオ6件(Task 6)が仕様どおり: durable ack / duplicate / staged / 部分受理 / subject_hint必須 / バッチ上限
- [ ] git log がタスク単位のコミットになっている

## 明示的スコープ外(後続計画へ)

- 現場レジストリ実装・受理判別表の完全化(D6決定4/6/7)→ **計画2**(RegistryPolicyの差し替え)
- アダプタ内取り込みクライアント化・ブリッジ削除・BravePI/I2Cのmeasurement写像のアダプタランタイム移設 → **計画3**
- gatewayctl(device approve / replace-hardware / staged本流化)・R11クエリ+CSV・R12ヘルスJSON・R17 retention+dedup TTLパージの定期実行・R22エクスポート → **計画4**
- HTTP/UDS/MQTTバインディング・トークン認証 → Wave 1(D1フェーズ3/4)
- 旧 `sensor_readings`(v2)テーブルの削除 → 計画4(R11がv3クエリに切り替わった後)
