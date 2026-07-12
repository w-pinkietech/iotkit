# Wave 0 計画2: 測定レジストリ(D6) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** D6の現場レジストリ+標準語彙カタログをWave 0スコープで実装し、計画1の暫定 `PermissiveRegistry` を受理判別表(D6決定6)の本実装 `SqliteRegistry` に差し替える。

**Architecture:** 新クレート `core/registry`(iotkit-core-registry)がD6のドメインを所有する: バイナリ同梱の標準語彙カタログ(TOML、`include_str!`)、現場レジストリのSQLiteスキーマ(copy-on-enable+エントリrevision)、受理判別表の評価器。コレクタは自分が必要とする港(`RegistryPolicy` トレイト)を定義し続け、registryクレートがそれを実装する(ports-and-adapters。依存方向: registry → collector)。ゲートウェイのcomposition rootが `SqliteRegistry` を配線する。

**Tech Stack:** Rust (edition 2024), rusqlite 0.32 (bundled), toml 0.8, serde, sha2, tokio。設計正本: [D6](../../redesign/decisions/D6-measurement-registry.md)(Wave分割節がスコープの正)。

## Global Constraints

- **ストレージ起因の失敗はackを返さない(Rejected禁止)。** レジストリ評価はDBに触れる(auto-enable等)ため可謬であり、`RegistryPolicy::evaluate` は `Result<RegistryVerdict, String>` を返す。`Err` は `process_item` の `?` で上へ伝播し、ack_txドロップ=送信側再送に委ねる(D1。計画1のT6教訓)。**評価器の中でストレージ失敗を `RejectItem` に変換してはならない。**
- **Rejected(terminal)は決定的な契約違反専用**: `malformed_measurement_key`(文法違反)と `value_type_mismatch`(構造的に解釈不能)のみ(D6決定6)。
- **ackの検疫理由可視化(D1追補 2026-07-03)**: `ItemStatus::Stored` に任意フィールド `quarantine_reason`(`out_of_range` / `unknown_key` / `undeclared_channel` / `device_quarantined`、D6判別表と1:1)をadditiveに追加する。D1は「実装はレジストリ実装と同時」と規定しており、本計画がそのレジストリ実装である。
- **未知キーは拒否ではなく検疫**: 文法適合の未知キーは `accepted` + disposition `quarantined`。series自動実体化(quarantined=true, quarantine_reason付き)。
- **measurement_key文法**: セグメント `[a-z][a-z0-9_]*`、ドット区切り、コロン禁止、上限64(実装済み `validate_measurement_key` を使う。再実装禁止)。
- **キーとエイリアスは単一名前空間**(D6決定2): entry有効化時にalias衝突検査、alias定義時にentry衝突検査。両方向。
- **series_key不変規則**(D6決定3(a)): エイリアス解決は「当該subjectで申告キーのseriesが未実体化」の場合のみcanonicalへ写像する。実体化済みなら申告キーのまま。
- **値域フォールバック順**: series個別 → レジストリエントリ現場既定 → カタログ物理限界(D6決定7)。**min/max各辺独立にフォールバック**する(片辺のみのseries上書きで反対辺の限界が消えると「外殻の拡張」になり決定7違反)。単位は全層不変。
- **チャネル正規化は一箇所**: `channel_index: Option<u16>` → DB表現 `i32`(番兵-1)と既定variant `"primary"` の正規化は `iotkit-core-ledger` の定数に一本化する(collectorとregistryで重複実装しない。CLAUDE.md変換境界規律)。さらに受理経路の**正準チャネルは評価器が決めて `RegistryVerdict::Accept.channel_index` で返す**(single modeでは `Some(0)` も番兵-1へ正規化——`None`/`Some(0)` で同一測定が別seriesに分裂するのを防ぐ)。コレクタは自前でチャネルを再計算しない。
- **カタログはバイナリ同梱・ランタイムのネットワーク参照なし**(D6決定1/4)。catalog_version = `"1.0.0"`。
- **監査イベント**: カタログキーの自動有効化は `ledger_events`(R13最小下地)への行追記必須(D6決定4)。
- テストコマンドは `cargo test -p <crate>`、最終タスクで `cargo test --workspace`。
- コミット規約: `feat(crate):` / `fix(crate):` + `Co-Authored-By: Claude <noreply@anthropic.com>`。

## Wave 0スコープ確認(D6 Wave分割節より)

**やる**: 現場レジストリ(SQLite)、copy-on-enable+エントリrevision、自動有効化+監査イベント行、R8判別表、初期語彙10キー(TOML)、値域フォールバック、legacy_sensor_type移行シム表(スキーマ+播種関数のみ。ゲートウェイ起動には配線しない)。

**やらない(宛先)**: ドリフトレポート/corrective自動適用(Wave 1)、custom輸出入(Wave 1)、R14定義変更操作・alias定義CLI(計画4/Wave 1——ただし`define_alias`関数自体は本計画でテスト用に実装する)、deprecated+superseded_by運用(Wave 1)、vibration_spectrumの保存・配送(第二波——契約予約としてカタログには載せ、受信時はvalue_type_mismatchで拒否)、検疫解除操作(計画4)。

## File Structure

```
core/registry/                     # 新クレート iotkit-core-registry
├── Cargo.toml
├── catalog/standard-v1.toml       # 標準語彙カタログ(データ資産・版管理)
├── migrations/0006_registry.sql   # 現場レジストリ+エイリアス+legacyシム表
└── src/
    ├── lib.rs                     # 公開面+MIGRATIONS
    ├── catalog.rs                 # カタログ型・TOMLパース・整合検証・revisionハッシュ
    ├── store.rs                   # enable_entry / find_resolution / define_alias / legacyシム
    └── policy.rs                  # SqliteRegistry(D6判別表の評価器)
core/ledger/
├── migrations/0005_series_quarantine_reason.sql   # 新規
└── src/{lib.rs,store.rs}          # 変更: MIGRATIONS拡張、record_event公開、
                                   #   ensure_series署名変更、find_series_meta/series_exists_for_key、
                                   #   CHANNEL_NA/DEFAULT_VARIANT定数
core/collector/src/
├── registry_policy.rs             # 変更: トレイト署名(conn+system_id+Result)、
│                                  #   RegistryVerdict拡張(resolved_key/channel_index/quarantine)
└── actor.rs                       # 変更: 文法precheck→subject→評価→series(reason付き)の順序、
                                   #   ackへのquarantine_reason写像
core/timeseries/src/lib.rs         # 変更: v3_tests の ensure_series 呼び出し(引数追加のみ)
iotkit-ingest-contract/src/
├── ack.rs                         # 変更: QuarantineReason新設+Stored.quarantine_reason(D1追補、additive)
└── lib.rs                         # 変更: re-export追加
iotkit-gateway/src/
├── main.rs                        # 変更: migrations連結にv5,v6追加、SqliteRegistry配線
└── bridge.rs                      # 変更: 統合テストをSqliteRegistryに切り替え
```

---

### Task 1: core/registry クレート新設+標準語彙カタログ(TOML)+パーサ

**Files:**
- Create: `core/registry/Cargo.toml`
- Create: `core/registry/catalog/standard-v1.toml`
- Create: `core/registry/src/lib.rs`
- Create: `core/registry/src/catalog.rs`
- Modify: `Cargo.toml`(ワークスペースmembersに `"core/registry"` を追加)

**Interfaces:**
- Consumes: `iotkit_ingest_contract::validate_measurement_key`
- Produces: `iotkit_core_registry::catalog::{Catalog, CatalogEntry, ValueType, ChannelMode, Range, standard_catalog}`、`CatalogEntry::revision() -> String`。後続タスクは `standard_catalog()`(検証済み・`&'static Catalog`)と `find(key)` を使う。

- [ ] **Step 1: ワークスペースとクレートの骨組み**

ルート `Cargo.toml` のmembersに `"core/collector",` の直後に追加:

```toml
    "core/registry",
```

`core/registry/Cargo.toml`:

```toml
[package]
name = "iotkit-core-registry"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-ingest-contract = { path = "../../iotkit-ingest-contract" }
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
toml = "0.8"
tracing = "0.1"

[dev-dependencies]
iotkit-core-storage = { path = "../storage", features = ["test-util"] }
```

(注: `iotkit-core-ledger` / `iotkit-core-collector` への依存はTask 3/5で追加する。このタスクではカタログのみ。)

`core/registry/src/lib.rs`:

```rust
//! iotkit-core-registry: D6測定レジストリ(標準語彙カタログ+現場レジストリ)。
//! 正本文書: docs/redesign/decisions/D6-measurement-registry.md
pub mod catalog;

pub use catalog::{standard_catalog, Catalog, CatalogEntry, ChannelMode, Range, ValueType};
```

- [ ] **Step 2: カタログデータファイルを書く**

`core/registry/catalog/standard-v1.toml`(D6決定11の初期語彙10キー。**legacy_sensor_type番号はこのファイルに書かない**——移行シムはカタログの一部ではない=D6決定11の位置づけ):

```toml
# IoTKit 標準語彙カタログ v1(D6決定11)。契約仕様書の一部として公開される資産。
# 変更規律: additive(追加)/corrective(互換修正)/breaking(禁止: 新キー+superseded_byのみ)=D6決定4/5
catalog_version = "1.0.0"

[[measurement]]
key = "contact_state"
unit_ucum = "1"
value_type = "bool"
semantic_class = "sensor"
channel_mode = "generic"

[[measurement]]
key = "contact_output_state"
unit_ucum = "1"
value_type = "bool"
semantic_class = "actuator_state"
channel_mode = "generic"

[[measurement]]
key = "voltage_mv"
unit_ucum = "mV"
unit_display = "mV"
value_type = "float"
semantic_class = "sensor"
channel_mode = "generic"
# 物理限界はADC仕様がプロバイダごとに異なるため未定義(必要になればcorrective追加=D6決定4)

[[measurement]]
key = "distance_mm"
unit_ucum = "mm"
unit_display = "mm"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
physical_range = { min = 0.0, max = 4000.0 } # ToF測距センサー(VL53L1X系)の最大測距4m

[[measurement]]
key = "temperature_c"
unit_ucum = "Cel"
unit_display = "℃"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
physical_range = { min = -200.0, max = 1372.0 } # K型熱電対の規格測定範囲

[[measurement]]
key = "acceleration_mg"
unit_ucum = "m[G]"
unit_display = "mG"
value_type = "float"
semantic_class = "sensor"
channel_mode = "fixed"
channel_roles = ["x", "y", "z"]
physical_range = { min = -16000.0, max = 16000.0 } # ±16g(LIS2DUXS12の最大レンジ)

[[measurement]]
key = "differential_pressure_pa"
unit_ucum = "Pa"
unit_display = "Pa"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
physical_range = { min = -500.0, max = 500.0 } # SDP810-500Pa系の測定範囲

[[measurement]]
key = "illuminance_lux"
unit_ucum = "lx"
unit_display = "lx"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
physical_range = { min = 0.0, max = 88000.0 } # OPT3001の最大測定照度(83k lux+余裕)

[[measurement]]
key = "current_ma"
unit_ucum = "mA"
unit_display = "mA"
value_type = "float"
semantic_class = "sensor"
channel_mode = "generic"
physical_range = { min = 0.0, max = 25.0 } # 4-20mAカレントループ(断線/過電流検知の余裕含む)

[[measurement]]
key = "vibration_spectrum"
value_type = "record"
semantic_class = "sensor"
channel_mode = "single"
# 契約予約のみ(D6決定10)。record型のワイヤ表現は第二波の契約追補で定義する。
# Wave 0で受信した場合はf64配列として構造的に解釈不能 → value_type_mismatch で拒否。
```

- [ ] **Step 3: 失敗するテストを書く**

`core/registry/src/catalog.rs`(型定義とテストを先に。パース関数は最小スタブ):

```rust
//! 標準語彙カタログ(D6決定1のリポジトリ資産層)。バイナリ同梱、起動時+ビルド時テストで整合検証。
use serde::Deserialize;
use std::sync::OnceLock;

pub const STANDARD_CATALOG_TOML: &str = include_str!("../catalog/standard-v1.toml");

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Catalog {
    pub catalog_version: String,
    #[serde(rename = "measurement")]
    pub measurements: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CatalogEntry {
    pub key: String,
    #[serde(default)]
    pub unit_ucum: Option<String>,
    #[serde(default)]
    pub unit_display: Option<String>,
    pub value_type: ValueType,
    pub semantic_class: String,
    pub channel_mode: ChannelMode,
    #[serde(default)]
    pub channel_roles: Vec<String>,
    #[serde(default)]
    pub physical_range: Option<Range>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Float,
    Int,
    Bool,
    Record,
}

impl ValueType {
    pub fn as_db(&self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Int => "int",
            Self::Bool => "bool",
            Self::Record => "record",
        }
    }
    pub fn from_db(s: &str) -> Self {
        match s {
            "int" => Self::Int,
            "bool" => Self::Bool,
            "record" => Self::Record,
            _ => Self::Float,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMode {
    /// 単ch: channel_indexなし(またはSome(0))のみ正準
    Single,
    /// 汎用: デバイス側ラベルに委譲(D6決定12)。Wave 0は宣言照合なし=全channel_index受理
    Generic,
    /// 固定: カタログが役割を固定(index < roles.len() のみ正準)
    Fixed,
}

impl ChannelMode {
    pub fn as_db(&self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Generic => "generic",
            Self::Fixed => "fixed",
        }
    }
    pub fn from_db(s: &str) -> Self {
        match s {
            "generic" => Self::Generic,
            "fixed" => Self::Fixed,
            _ => Self::Single,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Range {
    pub min: f64,
    pub max: f64,
}

#[derive(Debug)]
pub enum CatalogError {
    Parse(String),
    Invalid(String),
}
impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "catalog parse error: {e}"),
            Self::Invalid(e) => write!(f, "catalog invalid: {e}"),
        }
    }
}
impl std::error::Error for CatalogError {}

pub fn parse_catalog(toml_text: &str) -> Result<Catalog, CatalogError> {
    todo!()
}

/// 同梱カタログ(検証済み)。パース/検証失敗はプログラミングエラーなのでpanic
/// (ビルド時テストが同じ経路を通すため、壊れたカタログはCIで落ちる)。
pub fn standard_catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        parse_catalog(STANDARD_CATALOG_TOML).expect("bundled standard catalog must be valid")
    })
}

impl Catalog {
    pub fn find(&self, key: &str) -> Option<&CatalogEntry> {
        self.measurements.iter().find(|m| m.key == key)
    }
}

impl CatalogEntry {
    /// エントリrevision(内容ハッシュ、D6決定4)。カタログ版全体スタンプとは独立に
    /// 「このエントリの定義内容」を識別する。
    pub fn revision(&self) -> String {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_catalog_parses_with_10_keys_and_version() {
        let c = standard_catalog();
        assert_eq!(c.catalog_version, "1.0.0");
        assert_eq!(c.measurements.len(), 10);
        for k in [
            "contact_state", "contact_output_state", "voltage_mv", "distance_mm",
            "temperature_c", "acceleration_mg", "differential_pressure_pa",
            "illuminance_lux", "current_ma", "vibration_spectrum",
        ] {
            assert!(c.find(k).is_some(), "{k} must be in the standard catalog");
        }
    }

    #[test]
    fn acceleration_is_fixed_xyz_and_temperature_is_single() {
        let c = standard_catalog();
        let acc = c.find("acceleration_mg").unwrap();
        assert_eq!(acc.channel_mode, ChannelMode::Fixed);
        assert_eq!(acc.channel_roles, vec!["x", "y", "z"]);
        assert_eq!(acc.physical_range, Some(Range { min: -16000.0, max: 16000.0 }));
        let t = c.find("temperature_c").unwrap();
        assert_eq!(t.channel_mode, ChannelMode::Single);
        assert_eq!(t.unit_ucum.as_deref(), Some("Cel"));
        assert_eq!(t.unit_display.as_deref(), Some("℃"));
    }

    #[test]
    fn vibration_spectrum_is_reserved_record() {
        let v = standard_catalog().find("vibration_spectrum").unwrap();
        assert_eq!(v.value_type, ValueType::Record);
        assert!(v.physical_range.is_none());
    }

    #[test]
    fn all_catalog_keys_pass_contract_grammar() {
        for m in &standard_catalog().measurements {
            assert!(
                iotkit_ingest_contract::validate_measurement_key(&m.key).is_ok(),
                "{} must satisfy D6決定2 grammar", m.key
            );
        }
    }

    #[test]
    fn parse_rejects_duplicate_keys() {
        let dup = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "temperature_c"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
[[measurement]]
key = "temperature_c"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
"#;
        assert!(matches!(parse_catalog(dup), Err(CatalogError::Invalid(_))));
    }

    #[test]
    fn parse_rejects_fixed_without_roles_and_roles_on_non_fixed() {
        let fixed_no_roles = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "a"
value_type = "float"
semantic_class = "sensor"
channel_mode = "fixed"
"#;
        assert!(matches!(parse_catalog(fixed_no_roles), Err(CatalogError::Invalid(_))));
        let single_with_roles = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "a"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
channel_roles = ["x"]
"#;
        assert!(matches!(parse_catalog(single_with_roles), Err(CatalogError::Invalid(_))));
    }

    #[test]
    fn parse_rejects_bad_key_grammar_and_inverted_range() {
        let bad_key = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "Bad:Key"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
"#;
        assert!(matches!(parse_catalog(bad_key), Err(CatalogError::Invalid(_))));
        let inverted = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "a"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
physical_range = { min = 10.0, max = 1.0 }
"#;
        assert!(matches!(parse_catalog(inverted), Err(CatalogError::Invalid(_))));
    }

    #[test]
    fn revision_is_stable_and_content_sensitive() {
        let c = standard_catalog();
        let t = c.find("temperature_c").unwrap();
        let r1 = t.revision();
        let r2 = t.revision();
        assert_eq!(r1, r2, "same content → same revision");
        assert_eq!(r1.len(), 64, "sha256 hex");
        let mut altered = t.clone();
        altered.physical_range = Some(Range { min: -200.0, max: 9999.0 });
        assert_ne!(r1, altered.revision(), "content change → revision change");
    }
}
```

- [ ] **Step 4: テストが失敗することを確認**

Run: `cargo test -p iotkit-core-registry`
Expected: FAIL(`todo!()` パニック / コンパイルエラー)

- [ ] **Step 5: 実装(parse_catalog / revision)**

`catalog.rs` の `todo!()` を置き換え:

```rust
pub fn parse_catalog(toml_text: &str) -> Result<Catalog, CatalogError> {
    let catalog: Catalog =
        toml::from_str(toml_text).map_err(|e| CatalogError::Parse(e.to_string()))?;
    let mut seen = std::collections::HashSet::new();
    for m in &catalog.measurements {
        iotkit_ingest_contract::validate_measurement_key(&m.key)
            .map_err(|e| CatalogError::Invalid(format!("key '{}': {e}", m.key)))?;
        if !seen.insert(m.key.clone()) {
            return Err(CatalogError::Invalid(format!("duplicate key '{}'", m.key)));
        }
        match m.channel_mode {
            ChannelMode::Fixed if m.channel_roles.is_empty() => {
                return Err(CatalogError::Invalid(format!(
                    "key '{}': fixed channel_mode requires channel_roles", m.key
                )));
            }
            ChannelMode::Single | ChannelMode::Generic if !m.channel_roles.is_empty() => {
                return Err(CatalogError::Invalid(format!(
                    "key '{}': channel_roles only allowed for fixed mode", m.key
                )));
            }
            _ => {}
        }
        if let Some(r) = &m.physical_range {
            if !(r.min < r.max) {
                return Err(CatalogError::Invalid(format!(
                    "key '{}': physical_range min must be < max", m.key
                )));
            }
        }
        if m.value_type == ValueType::Record && m.physical_range.is_some() {
            return Err(CatalogError::Invalid(format!(
                "key '{}': record type cannot carry a physical_range", m.key
            )));
        }
    }
    Ok(catalog)
}
```

```rust
    pub fn revision(&self) -> String {
        use sha2::{Digest, Sha256};
        let roles = self.channel_roles.join(",");
        let range = self
            .physical_range
            .map(|r| format!("{}..{}", r.min, r.max))
            .unwrap_or_default();
        let canonical = format!(
            "{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}",
            self.key,
            self.unit_ucum.as_deref().unwrap_or(""),
            self.unit_display.as_deref().unwrap_or(""),
            self.value_type.as_db(),
            self.semantic_class,
            self.channel_mode.as_db(),
            roles,
            range,
        );
        let hash = Sha256::digest(canonical.as_bytes());
        hash.iter().map(|b| format!("{b:02x}")).collect()
    }
```

- [ ] **Step 6: テストが通ることを確認**

Run: `cargo test -p iotkit-core-registry`
Expected: PASS(8テスト)

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml core/registry
git commit -m "feat(registry): add core/registry crate with bundled standard catalog v1 (D6)"
```

---

### Task 2: マイグレーション(ledger v5 + registry v6)と台帳の拡張

**Files:**
- Create: `core/ledger/migrations/0005_series_quarantine_reason.sql`
- Create: `core/registry/migrations/0006_registry.sql`
- Modify: `core/ledger/src/lib.rs`(MIGRATIONS 2本化、re-export追加)
- Modify: `core/ledger/src/store.rs`(record_event公開、ensure_series署名、find_series_meta、series_exists_for_key、定数)
- Modify: `core/registry/src/lib.rs`(MIGRATIONS追加)
- Modify: `core/registry/Cargo.toml`(dependenciesに `iotkit-core-ledger`、dev-dependenciesは既存)
- Modify: `core/collector/src/actor.rs`(ensure_series呼び出しに引数1つ追加——機械的)
- Modify: `core/timeseries/src/lib.rs`(v3_testsのensure_series呼び出し——機械的)

**Interfaces:**
- Consumes: 既存 `ledger::{ensure_series, LedgerError, SystemId}`
- Produces:
  - `ledger::CHANNEL_NA: i32 = -1` / `ledger::DEFAULT_VARIANT: &str = "primary"`(チャネル正規化の一箇所)
  - `ledger::record_event(conn, kind, system_id: Option<&SystemId>, detail) -> Result<(), LedgerError>`(旧private `audit` の公開名)
  - `ledger::ensure_series(conn, system_id, measurement_key, channel_index, variant, quarantined, quarantine_reason: Option<&str>) -> Result<i64, LedgerError>`(**引数追加**)
  - `ledger::find_series_meta(conn, system_id, measurement_key, channel_index, variant) -> Result<Option<SeriesMeta>, LedgerError>` / `pub struct SeriesMeta { pub series_id: i64, pub quarantined: bool, pub quarantine_reason: Option<String>, pub range_min: Option<f64>, pub range_max: Option<f64> }`
  - `ledger::series_exists_for_key(conn, system_id, measurement_key) -> Result<bool, LedgerError>`(channel/variant不問。D6決定3(a)のseries_key不変規則用)
  - `ledger::release_series_quarantine_for_key(conn, measurement_key, reason) -> Result<Vec<i64>, LedgerError>`(エイリアス確立時の検疫解除=D6決定3(a)。解除したseries_id一覧を返す)
  - `iotkit_core_registry::MIGRATIONS`(v6・1本)

- [ ] **Step 1: マイグレーションSQLを書く**

`core/ledger/migrations/0005_series_quarantine_reason.sql`:

```sql
-- D6決定6: 未知キー検疫series実体化にquarantine_reasonを付す(unknown_key | undeclared_channel)
ALTER TABLE series ADD COLUMN quarantine_reason TEXT;
```

`core/registry/migrations/0006_registry.sql`:

```sql
-- D6: 現場レジストリ(受理判定R8の唯一の参照先)。copy-on-enable+エントリrevision(決定4)
CREATE TABLE registry_entries (
    measurement_key   TEXT PRIMARY KEY,
    origin            TEXT NOT NULL CHECK (origin IN ('catalog','custom')),
    catalog_version   TEXT,             -- origin='catalog'のみ(エントリ単位スタンプ=決定4)
    entry_revision    TEXT NOT NULL,    -- 内容ハッシュ(決定4)
    unit_ucum         TEXT,
    unit_display      TEXT,
    value_type        TEXT NOT NULL CHECK (value_type IN ('float','int','bool','record')),
    semantic_class    TEXT NOT NULL,
    channel_mode      TEXT NOT NULL CHECK (channel_mode IN ('single','generic','fixed')),
    channel_roles_json TEXT,            -- fixedのみ(JSON配列)
    physical_min      REAL,             -- カタログ物理限界(外殻=決定7)
    physical_max      REAL,
    site_min          REAL,             -- 現場既定(外殻内)。Wave 0では設定APIなし(R14=Wave 1)
    site_max          REAL,
    enabled_at        INTEGER NOT NULL
);

-- D6決定3: エイリアス表(alias → measurement_key、多:1可)。キーと単一名前空間(決定2、
-- 相互衝突はアプリ層がenable/define時に同一トランザクション内で検査する)
CREATE TABLE registry_aliases (
    alias            TEXT PRIMARY KEY,
    measurement_key  TEXT NOT NULL REFERENCES registry_entries(measurement_key),
    alias_kind       TEXT NOT NULL CHECK (alias_kind IN ('rename','site_mapping')),
    created_at       INTEGER NOT NULL
);

-- D6決定3/11: legacy_sensor_type移行シム(ワイヤエイリアスではない型付き対応表)。
-- 播種はレガシー移行(D2 Phase 3.5)時のみ。Wave 0のゲートウェイ起動では触らない。
CREATE TABLE legacy_sensor_type_map (
    sensor_type      INTEGER PRIMARY KEY,
    measurement_key  TEXT NOT NULL,
    created_at       INTEGER NOT NULL
);
```

- [ ] **Step 2: 失敗するテストを書く(ledger側)**

`core/ledger/src/store.rs` のtestsモジュールに追加:

```rust
    #[test]
    fn ensure_series_stores_quarantine_reason() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(conn, &NewDevice {
                hardware_id: "ble:qr".into(), user_label: None, parent: None,
                kind: DeviceKind::Individual, initial_state: DeviceState::Active,
            }).unwrap();
            let id = ensure_series(conn, &sid, "custom.mystery", CHANNEL_NA, DEFAULT_VARIANT,
                true, Some("unknown_key")).unwrap();
            let meta = find_series_meta(conn, &sid, "custom.mystery", CHANNEL_NA, DEFAULT_VARIANT)
                .unwrap().unwrap();
            assert_eq!(meta.series_id, id);
            assert!(meta.quarantined);
            assert_eq!(meta.quarantine_reason.as_deref(), Some("unknown_key"));
            assert_eq!(meta.range_min, None);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn series_exists_for_key_ignores_channel_and_variant() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(conn, &NewDevice {
                hardware_id: "ble:ex".into(), user_label: None, parent: None,
                kind: DeviceKind::Individual, initial_state: DeviceState::Active,
            }).unwrap();
            assert!(!series_exists_for_key(conn, &sid, "temp_old").unwrap());
            ensure_series(conn, &sid, "temp_old", 2, "count", false, None).unwrap();
            assert!(series_exists_for_key(conn, &sid, "temp_old").unwrap());
            Ok(())
        }).unwrap();
    }

    #[test]
    fn record_event_appends_to_ledger_events() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            record_event(conn, "registry_entry_enabled", None, r#"{"key":"temperature_c"}"#).unwrap();
            let (kind, detail): (String, String) = conn.query_row(
                "SELECT kind, detail FROM ledger_events ORDER BY event_id DESC LIMIT 1",
                [], |r| Ok((r.get(0)?, r.get(1)?)),
            ).unwrap();
            assert_eq!(kind, "registry_entry_enabled");
            assert!(detail.contains("temperature_c"));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn release_series_quarantine_clears_matching_reason_only() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(conn, &NewDevice {
                hardware_id: "ble:rel".into(), user_label: None, parent: None,
                kind: DeviceKind::Individual, initial_state: DeviceState::Active,
            }).unwrap();
            let a = ensure_series(conn, &sid, "temp_old", CHANNEL_NA, DEFAULT_VARIANT,
                true, Some("unknown_key")).unwrap();
            ensure_series(conn, &sid, "other_key", CHANNEL_NA, DEFAULT_VARIANT,
                true, Some("undeclared_channel")).unwrap();
            let released = release_series_quarantine_for_key(conn, "temp_old", "unknown_key").unwrap();
            assert_eq!(released, vec![a]);
            let meta = find_series_meta(conn, &sid, "temp_old", CHANNEL_NA, DEFAULT_VARIANT)
                .unwrap().unwrap();
            assert!(!meta.quarantined);
            assert_eq!(meta.quarantine_reason, None);
            // キーも理由も異なるseriesは対象外
            let other = find_series_meta(conn, &sid, "other_key", CHANNEL_NA, DEFAULT_VARIANT)
                .unwrap().unwrap();
            assert!(other.quarantined);
            // 対象なしの冪等呼び出し
            assert!(release_series_quarantine_for_key(conn, "temp_old", "unknown_key").unwrap().is_empty());
            Ok(())
        }).unwrap();
    }

    #[test]
    fn find_series_meta_returns_range_override() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = insert_device(conn, &NewDevice {
                hardware_id: "ble:rng".into(), user_label: None, parent: None,
                kind: DeviceKind::Individual, initial_state: DeviceState::Active,
            }).unwrap();
            ensure_series(conn, &sid, "temperature_c", CHANNEL_NA, DEFAULT_VARIANT, false, None).unwrap();
            // Wave 0にはseries値域の設定APIがない(R14=計画4)ため、直接SQLで個別上書きを模擬
            conn.execute(
                "UPDATE series SET range_min = -10.0, range_max = 50.0
                 WHERE system_id = ?1 AND measurement_key = 'temperature_c'",
                params![sid.as_bytes().to_vec()],
            ).unwrap();
            let meta = find_series_meta(conn, &sid, "temperature_c", CHANNEL_NA, DEFAULT_VARIANT)
                .unwrap().unwrap();
            assert_eq!(meta.range_min, Some(-10.0));
            assert_eq!(meta.range_max, Some(50.0));
            Ok(())
        }).unwrap();
    }
```

既存テストの `ensure_series(conn, &sid, "temperature_c", -1, "primary", false)` 呼び出し(3箇所: `ensure_series_is_idempotent_and_monotonic` 内)は末尾引数 `, None` を追加し、番兵は定数に置換する(例: `ensure_series(conn, &sid, "temperature_c", CHANNEL_NA, DEFAULT_VARIANT, false, None)`)。

- [ ] **Step 3: テストが失敗することを確認**

Run: `cargo test -p iotkit-core-ledger`
Expected: FAIL(コンパイルエラー: 未定義シンボル `CHANNEL_NA` / `record_event` / `find_series_meta` / 引数不一致)

- [ ] **Step 4: 実装(ledger)**

`core/ledger/src/lib.rs`:

```rust
pub mod ids;
pub mod store;

pub use ids::SystemId;
pub use store::{
    activate_device, approve_sighting, ensure_series, find_alive_by_hardware_id,
    find_series_meta, insert_device, ledger_epoch, record_event, record_sighting,
    release_series_quarantine_for_key, series_exists_for_key, DeviceKind, DeviceRow,
    DeviceState, LedgerError, NewDevice, SeriesMeta, CHANNEL_NA, DEFAULT_VARIANT,
};

use iotkit_core_storage::Migration;

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 3,
        label: "ledger",
        sql: include_str!("../migrations/0003_ledger.sql"),
    },
    Migration {
        version: 5,
        label: "series_quarantine_reason",
        sql: include_str!("../migrations/0005_series_quarantine_reason.sql"),
    },
];
```

`core/ledger/src/store.rs` の変更:

```rust
/// チャネル正規化の一箇所(CLAUDE.md変換境界規律): channel_indexなしの番兵値と既定variant。
/// collectorとregistryの両方がこの定数を使う(重複定義禁止)。
pub const CHANNEL_NA: i32 = -1;
pub const DEFAULT_VARIANT: &str = "primary";

/// append-only監査イベント(R13最小下地)への行追記。registryクレート等の外部呼び出し用公開面。
pub fn record_event(
    conn: &Connection,
    kind: &str,
    system_id: Option<&SystemId>,
    detail: &str,
) -> Result<(), LedgerError> {
    conn.execute(
        "INSERT INTO ledger_events (at, kind, system_id, detail) VALUES (?1, ?2, ?3, ?4)",
        params![now_ms(), kind, system_id.map(|s| s.as_bytes().to_vec()), detail],
    )?;
    Ok(())
}
```

既存のprivate `fn audit(...)` は削除し、内部呼び出し(insert_device/approve_sighting/activate_device)を `record_event(conn, kind, system_id, detail)` に置換する。

`ensure_series` の署名変更(INSERT文にquarantine_reason列を追加):

```rust
pub fn ensure_series(
    conn: &Connection,
    system_id: &SystemId,
    measurement_key: &str,
    channel_index: i32,
    variant: &str,
    quarantined: bool,
    quarantine_reason: Option<&str>,
) -> Result<i64, LedgerError> {
    if let Some(id) = conn
        .query_row(
            "SELECT series_id FROM series
         WHERE system_id = ?1 AND measurement_key = ?2 AND channel_index = ?3 AND variant = ?4",
            params![system_id.as_bytes().to_vec(), measurement_key, channel_index, variant],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO series (system_id, measurement_key, channel_index, variant, quarantined, quarantine_reason, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![system_id.as_bytes().to_vec(), measurement_key, channel_index, variant,
            quarantined as i32, quarantine_reason, now_ms()],
    )?;
    Ok(conn.last_insert_rowid())
}
```

新規関数:

```rust
#[derive(Debug, Clone)]
pub struct SeriesMeta {
    pub series_id: i64,
    pub quarantined: bool,
    pub quarantine_reason: Option<String>,
    pub range_min: Option<f64>,
    pub range_max: Option<f64>,
}

pub fn find_series_meta(
    conn: &Connection,
    system_id: &SystemId,
    measurement_key: &str,
    channel_index: i32,
    variant: &str,
) -> Result<Option<SeriesMeta>, LedgerError> {
    conn.query_row(
        "SELECT series_id, quarantined, quarantine_reason, range_min, range_max FROM series
         WHERE system_id = ?1 AND measurement_key = ?2 AND channel_index = ?3 AND variant = ?4",
        params![system_id.as_bytes().to_vec(), measurement_key, channel_index, variant],
        |row| {
            Ok(SeriesMeta {
                series_id: row.get(0)?,
                quarantined: row.get::<_, i32>(1)? != 0,
                quarantine_reason: row.get(2)?,
                range_min: row.get(3)?,
                range_max: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(LedgerError::from)
}

/// D6決定3(a): 当該subjectで申告キーのseriesが(channel/variant不問で)実体化済みか。
pub fn series_exists_for_key(
    conn: &Connection,
    system_id: &SystemId,
    measurement_key: &str,
) -> Result<bool, LedgerError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM series WHERE system_id = ?1 AND measurement_key = ?2",
        params![system_id.as_bytes().to_vec(), measurement_key],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

/// エイリアス確立時の検疫解除(D6決定3(a)): 申告キーのまま実体化済みの検疫seriesに
/// canonical定義がバインドされたため、series級検疫を解く。過去の検疫行(readings)は
/// 履歴としてそのまま(保存済みデータの解釈を遡って変えない)。解除対象は
/// `quarantine_reason` が一致するseriesのみ(undeclared_channel等はエイリアスでは解決しない)。
pub fn release_series_quarantine_for_key(
    conn: &Connection,
    measurement_key: &str,
    reason: &str,
) -> Result<Vec<i64>, LedgerError> {
    let mut stmt = conn.prepare(
        "SELECT series_id FROM series
         WHERE measurement_key = ?1 AND quarantined = 1 AND quarantine_reason = ?2",
    )?;
    let ids: Vec<i64> = stmt
        .query_map(params![measurement_key, reason], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    if !ids.is_empty() {
        conn.execute(
            "UPDATE series SET quarantined = 0, quarantine_reason = NULL
             WHERE measurement_key = ?1 AND quarantined = 1 AND quarantine_reason = ?2",
            params![measurement_key, reason],
        )?;
    }
    Ok(ids)
}
```

- [ ] **Step 5: registry側のMIGRATIONS公開と機械的な呼び出し更新**

`core/registry/Cargo.toml` のdependenciesに追加:

```toml
iotkit-core-ledger = { path = "../ledger" }
iotkit-core-storage = { path = "../storage" }
```

`core/registry/src/lib.rs`:

```rust
//! iotkit-core-registry: D6測定レジストリ(標準語彙カタログ+現場レジストリ)。
//! 正本文書: docs/redesign/decisions/D6-measurement-registry.md
pub mod catalog;

pub use catalog::{standard_catalog, Catalog, CatalogEntry, ChannelMode, Range, ValueType};

use iotkit_core_storage::Migration;

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 6,
    label: "registry",
    sql: include_str!("../migrations/0006_registry.sql"),
}];
```

機械的更新(コンパイルを通す。挙動不変):
- `core/collector/src/actor.rs` の `ensure_series(conn, &system_id, &item.measurement_key, channel, &variant, false)` → 末尾に `, None` を追加。
- `core/timeseries/src/lib.rs` の `v3_tests::seed_series` 内 `ensure_series(conn, &sid, "temperature_c", -1, "primary", false)` → `ensure_series(conn, &sid, "temperature_c", ledger::CHANNEL_NA, ledger::DEFAULT_VARIANT, false, None)`。

registryのマイグレーション適用テストを `core/registry/src/lib.rs` の末尾に追加:

```rust
#[cfg(test)]
mod migration_tests {
    #[test]
    fn ledger_and_registry_migrations_apply() {
        // ledger+registry連結(1,3,5,6——集合差ベースのrunnerは番号の飛びを許容する)。
        // timeseriesを含むゲートウェイ完全連結(1..6)の検証はTask 6のE2Eが担う。
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS); // 3, 5
        all.extend_from_slice(crate::MIGRATIONS); // 6
        all.sort_by_key(|m| m.version);
        let db = iotkit_core_storage::init_db_memory(&all).unwrap();
        db.with_conn_sync(|conn| {
            for t in ["registry_entries", "registry_aliases", "legacy_sensor_type_map"] {
                let exists: bool = conn.query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
                    [t], |r| r.get(0),
                ).unwrap();
                assert!(exists, "{t} must exist");
            }
            // series.quarantine_reason列(v5)
            let has_col: bool = conn.query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('series') WHERE name='quarantine_reason'",
                [], |r| r.get(0),
            ).unwrap();
            assert!(has_col, "series.quarantine_reason must exist");
            Ok(())
        }).unwrap();
    }
}
```

- [ ] **Step 6: テストが通ることを確認**

Run: `cargo test -p iotkit-core-ledger && cargo test -p iotkit-core-registry && cargo test -p iotkit-core-collector && cargo test -p iotkit-core-timeseries`
Expected: PASS(全クレート。collector/timeseriesは既存テストが引数追加後も通る)

- [ ] **Step 7: Commit**

```bash
git add core/ledger core/registry core/collector/src/actor.rs core/timeseries/src/lib.rs
git commit -m "feat(ledger,registry): add registry schema v6, series quarantine_reason v5, ledger series/eventaccessors"
```

---

### Task 3: レジストリ書き込み層(enable_entry / find_resolution / define_alias / legacyシム)

**Files:**
- Create: `core/registry/src/store.rs`
- Modify: `core/registry/src/lib.rs`(`pub mod store;` と re-export追加)

**Interfaces:**
- Consumes: Task 1の `CatalogEntry`、Task 2の `ledger::record_event`
- Produces:
  - `registry::store::EntryRow { measurement_key, origin, catalog_version, entry_revision, unit_ucum, unit_display, value_type: ValueType, semantic_class, channel_mode: ChannelMode, channel_roles: Vec<String>, physical_min/max: Option<f64>, site_min/max: Option<f64> }`
  - `registry::store::enable_entry(conn, entry: &CatalogEntry, catalog_version: &str, trigger: &str) -> Result<EntryRow, RegistryError>`(冪等。初回のみINSERT+監査イベント)
  - `registry::store::find_resolution(conn, declared_key) -> Result<Option<Resolution>, RegistryError>` / `enum Resolution { Entry(EntryRow), Alias { canonical: EntryRow, alias_kind: String } }`
  - `registry::store::define_alias(conn, alias, target_key, kind: AliasKind) -> Result<(), RegistryError>` / `enum AliasKind { Rename, SiteMapping }`——**確立時に申告キー(=alias名)で実体化済みの `unknown_key` 検疫seriesを解除し、`series_quarantine_released` 監査イベントを発行する(D6決定3(a)「カタログ定義をバインドして検疫解除」)**
  - `registry::store::seed_legacy_sensor_map(conn) -> Result<usize, RegistryError>` / `registry::store::lookup_legacy(conn, sensor_type: u16) -> Result<Option<String>, RegistryError>` / `pub const LEGACY_SENSOR_MAP: &[(u16, &str)]`
  - `registry::RegistryError`

- [ ] **Step 1: 失敗するテストを書く**

`core/registry/src/store.rs`(型と関数スタブ+テスト):

```rust
//! 現場レジストリの書き込み層(D6決定3/4)。受理判定(R8)の唯一の参照先。
use crate::catalog::{CatalogEntry, ChannelMode, ValueType};
use iotkit_core_ledger as ledger;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug)]
pub enum RegistryError {
    /// 単一名前空間の衝突(D6決定2): キー⇔エイリアス間
    NamespaceCollision(String),
    /// alias定義の対象エントリが存在しない
    TargetNotFound(String),
    /// aliasが既に定義済み
    AliasExists(String),
    InvalidKey(String),
    Sqlite(rusqlite::Error),
    Ledger(ledger::LedgerError),
}
impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NamespaceCollision(k) => write!(f, "name '{k}' collides across key/alias namespace"),
            Self::TargetNotFound(k) => write!(f, "alias target entry not found: {k}"),
            Self::AliasExists(a) => write!(f, "alias already defined: {a}"),
            Self::InvalidKey(k) => write!(f, "invalid measurement_key: {k}"),
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::Ledger(e) => write!(f, "ledger error: {e}"),
        }
    }
}
impl std::error::Error for RegistryError {}
impl From<rusqlite::Error> for RegistryError {
    fn from(e: rusqlite::Error) -> Self { Self::Sqlite(e) }
}
impl From<ledger::LedgerError> for RegistryError {
    fn from(e: ledger::LedgerError) -> Self { Self::Ledger(e) }
}

#[derive(Debug, Clone)]
pub struct EntryRow {
    pub measurement_key: String,
    pub origin: String,
    pub catalog_version: Option<String>,
    pub entry_revision: String,
    pub unit_ucum: Option<String>,
    pub unit_display: Option<String>,
    pub value_type: ValueType,
    pub semantic_class: String,
    pub channel_mode: ChannelMode,
    pub channel_roles: Vec<String>,
    pub physical_min: Option<f64>,
    pub physical_max: Option<f64>,
    pub site_min: Option<f64>,
    pub site_max: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum Resolution {
    Entry(EntryRow),
    Alias { canonical: EntryRow, alias_kind: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasKind {
    Rename,
    SiteMapping,
}
impl AliasKind {
    fn as_db(&self) -> &'static str {
        match self {
            Self::Rename => "rename",
            Self::SiteMapping => "site_mapping",
        }
    }
}

/// D6決定11のlegacy_sensor_type対応表(移行シム。カタログの一部ではない)。
/// 262はacceleration_mg側のみ(スペクトログラム側の分解はアダプタ写像=決定3)。
pub const LEGACY_SENSOR_MAP: &[(u16, &str)] = &[
    (257, "contact_state"),
    (258, "contact_output_state"),
    (259, "voltage_mv"),
    (260, "distance_mm"),
    (261, "temperature_c"),
    (262, "acceleration_mg"),
    (263, "differential_pressure_pa"),
    (264, "illuminance_lux"),
    (294, "contact_state"),
    (295, "contact_state"),
    (296, "contact_output_state"),
    (297, "temperature_c"),
    (298, "current_ma"),
    (299, "voltage_mv"),
];

pub fn enable_entry(
    conn: &Connection,
    entry: &CatalogEntry,
    catalog_version: &str,
    trigger: &str,
) -> Result<EntryRow, RegistryError> {
    todo!()
}

pub fn get_entry(conn: &Connection, key: &str) -> Result<Option<EntryRow>, RegistryError> {
    todo!()
}

pub fn find_resolution(
    conn: &Connection,
    declared_key: &str,
) -> Result<Option<Resolution>, RegistryError> {
    todo!()
}

pub fn define_alias(
    conn: &Connection,
    alias: &str,
    target_key: &str,
    kind: AliasKind,
) -> Result<(), RegistryError> {
    todo!()
}

pub fn seed_legacy_sensor_map(conn: &Connection) -> Result<usize, RegistryError> {
    todo!()
}

pub fn lookup_legacy(conn: &Connection, sensor_type: u16) -> Result<Option<String>, RegistryError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::standard_catalog;

    fn test_db() -> iotkit_core_storage::DbHandle {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(crate::MIGRATIONS);
        all.sort_by_key(|m| m.version);
        iotkit_core_storage::init_db_memory(&all).unwrap()
    }

    #[test]
    fn enable_entry_copies_catalog_and_stamps_revision_and_audits() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let cat = standard_catalog();
            let t = cat.find("temperature_c").unwrap();
            let row = enable_entry(conn, t, &cat.catalog_version, "auto").unwrap();
            assert_eq!(row.measurement_key, "temperature_c");
            assert_eq!(row.origin, "catalog");
            assert_eq!(row.catalog_version.as_deref(), Some("1.0.0"));
            assert_eq!(row.entry_revision, t.revision());
            assert_eq!(row.physical_min, Some(-200.0));
            assert_eq!(row.physical_max, Some(1372.0));
            assert_eq!(row.site_min, None, "現場既定はWave 0では未設定");
            // 監査イベント必須(D6決定4)
            let detail: String = conn.query_row(
                "SELECT detail FROM ledger_events WHERE kind='registry_entry_enabled'
                 ORDER BY event_id DESC LIMIT 1",
                [], |r| r.get(0),
            ).unwrap();
            assert!(detail.contains("temperature_c") && detail.contains(&t.revision()));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn enable_entry_is_idempotent() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let cat = standard_catalog();
            let t = cat.find("temperature_c").unwrap();
            enable_entry(conn, t, &cat.catalog_version, "auto").unwrap();
            enable_entry(conn, t, &cat.catalog_version, "auto").unwrap();
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM registry_entries WHERE measurement_key='temperature_c'",
                [], |r| r.get(0),
            ).unwrap();
            assert_eq!(n, 1);
            let events: i64 = conn.query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                [], |r| r.get(0),
            ).unwrap();
            assert_eq!(events, 1, "冪等re-enableは監査イベントを重複させない");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn fixed_channel_roles_round_trip_through_db() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let cat = standard_catalog();
            let acc = cat.find("acceleration_mg").unwrap();
            enable_entry(conn, acc, &cat.catalog_version, "auto").unwrap();
            let row = get_entry(conn, "acceleration_mg").unwrap().unwrap();
            assert_eq!(row.channel_mode, ChannelMode::Fixed);
            assert_eq!(row.channel_roles, vec!["x", "y", "z"]);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn define_alias_and_resolve() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let cat = standard_catalog();
            enable_entry(conn, cat.find("temperature_c").unwrap(), &cat.catalog_version, "auto").unwrap();
            define_alias(conn, "temp_old", "temperature_c", AliasKind::SiteMapping).unwrap();
            match find_resolution(conn, "temp_old").unwrap().unwrap() {
                Resolution::Alias { canonical, alias_kind } => {
                    assert_eq!(canonical.measurement_key, "temperature_c");
                    assert_eq!(alias_kind, "site_mapping");
                }
                other => panic!("expected Alias resolution, got {other:?}"),
            }
            match find_resolution(conn, "temperature_c").unwrap().unwrap() {
                Resolution::Entry(e) => assert_eq!(e.measurement_key, "temperature_c"),
                other => panic!("expected Entry resolution, got {other:?}"),
            }
            assert!(find_resolution(conn, "custom.nothing").unwrap().is_none());
            Ok(())
        }).unwrap();
    }

    #[test]
    fn single_namespace_collisions_are_blocked_both_ways() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let cat = standard_catalog();
            enable_entry(conn, cat.find("temperature_c").unwrap(), &cat.catalog_version, "auto").unwrap();
            // alias名がエントリキーと衝突 → 拒否
            assert!(matches!(
                define_alias(conn, "temperature_c", "temperature_c", AliasKind::SiteMapping),
                Err(RegistryError::NamespaceCollision(_))
            ));
            // 逆方向: 既存aliasと同名のエントリ有効化 → 拒否(D6決定3の衝突検査)
            define_alias(conn, "voltage_mv", "temperature_c", AliasKind::SiteMapping).unwrap();
            assert!(matches!(
                enable_entry(conn, cat.find("voltage_mv").unwrap(), &cat.catalog_version, "auto"),
                Err(RegistryError::NamespaceCollision(_))
            ));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn define_alias_releases_unknown_key_quarantined_series_and_audits() {
        // D6決定3(a): 実体化済み申告キーへのエイリアス確立=canonical定義バインド → 検疫解除
        let db = test_db();
        db.with_conn_sync(|conn| {
            let cat = standard_catalog();
            enable_entry(conn, cat.find("temperature_c").unwrap(), &cat.catalog_version, "auto").unwrap();
            let sid = iotkit_core_ledger::insert_device(conn, &iotkit_core_ledger::NewDevice {
                hardware_id: "ble:aa".into(), user_label: None, parent: None,
                kind: iotkit_core_ledger::DeviceKind::Individual,
                initial_state: iotkit_core_ledger::DeviceState::Active,
            }).unwrap();
            // 検疫期にunknown_keyとして実体化済みのseries
            iotkit_core_ledger::ensure_series(conn, &sid, "temp_old",
                iotkit_core_ledger::CHANNEL_NA, iotkit_core_ledger::DEFAULT_VARIANT,
                true, Some("unknown_key")).unwrap();
            define_alias(conn, "temp_old", "temperature_c", AliasKind::SiteMapping).unwrap();
            let meta = iotkit_core_ledger::find_series_meta(conn, &sid, "temp_old",
                iotkit_core_ledger::CHANNEL_NA, iotkit_core_ledger::DEFAULT_VARIANT)
                .unwrap().unwrap();
            assert!(!meta.quarantined, "series_keyは不変のまま検疫解除される");
            assert_eq!(meta.quarantine_reason, None);
            let detail: String = conn.query_row(
                "SELECT detail FROM ledger_events WHERE kind='series_quarantine_released'
                 ORDER BY event_id DESC LIMIT 1",
                [], |r| r.get(0),
            ).unwrap();
            assert!(detail.contains("temp_old") && detail.contains("temperature_c"));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn define_alias_rejects_missing_target_dup_and_bad_grammar() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let cat = standard_catalog();
            assert!(matches!(
                define_alias(conn, "temp_old", "temperature_c", AliasKind::Rename),
                Err(RegistryError::TargetNotFound(_))
            ));
            enable_entry(conn, cat.find("temperature_c").unwrap(), &cat.catalog_version, "auto").unwrap();
            define_alias(conn, "temp_old", "temperature_c", AliasKind::Rename).unwrap();
            assert!(matches!(
                define_alias(conn, "temp_old", "temperature_c", AliasKind::Rename),
                Err(RegistryError::AliasExists(_))
            ));
            assert!(matches!(
                define_alias(conn, "Bad:Alias", "temperature_c", AliasKind::Rename),
                Err(RegistryError::InvalidKey(_))
            ));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn legacy_map_seeds_and_resolves() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let n = seed_legacy_sensor_map(conn).unwrap();
            assert_eq!(n, LEGACY_SENSOR_MAP.len());
            assert_eq!(lookup_legacy(conn, 261).unwrap().as_deref(), Some("temperature_c"));
            assert_eq!(lookup_legacy(conn, 294).unwrap().as_deref(), Some("contact_state"));
            assert_eq!(lookup_legacy(conn, 9999).unwrap(), None);
            // 冪等
            assert_eq!(seed_legacy_sensor_map(conn).unwrap(), 0);
            Ok(())
        }).unwrap();
    }
}
```

`core/registry/src/lib.rs` に追加:

```rust
pub mod store;

pub use store::{
    define_alias, enable_entry, find_resolution, get_entry, lookup_legacy,
    seed_legacy_sensor_map, AliasKind, EntryRow, RegistryError, Resolution,
    LEGACY_SENSOR_MAP,
};
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p iotkit-core-registry`
Expected: FAIL(`todo!()` パニック)

- [ ] **Step 3: 実装**

```rust
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn alias_exists(conn: &Connection, name: &str) -> Result<bool, RegistryError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM registry_aliases WHERE alias = ?1",
        params![name], |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> Result<EntryRow, rusqlite::Error> {
    let roles_json: Option<String> = row.get(9)?;
    let channel_roles: Vec<String> = roles_json
        .as_deref()
        .map(|j| serde_json::from_str(j).unwrap_or_default())
        .unwrap_or_default();
    Ok(EntryRow {
        measurement_key: row.get(0)?,
        origin: row.get(1)?,
        catalog_version: row.get(2)?,
        entry_revision: row.get(3)?,
        unit_ucum: row.get(4)?,
        unit_display: row.get(5)?,
        value_type: ValueType::from_db(&row.get::<_, String>(6)?),
        semantic_class: row.get(7)?,
        channel_mode: ChannelMode::from_db(&row.get::<_, String>(8)?),
        channel_roles,
        physical_min: row.get(10)?,
        physical_max: row.get(11)?,
        site_min: row.get(12)?,
        site_max: row.get(13)?,
    })
}

const ENTRY_COLS: &str = "measurement_key, origin, catalog_version, entry_revision, unit_ucum, \
     unit_display, value_type, semantic_class, channel_mode, channel_roles_json, \
     physical_min, physical_max, site_min, site_max";

pub fn get_entry(conn: &Connection, key: &str) -> Result<Option<EntryRow>, RegistryError> {
    conn.query_row(
        &format!("SELECT {ENTRY_COLS} FROM registry_entries WHERE measurement_key = ?1"),
        params![key],
        row_to_entry,
    )
    .optional()
    .map_err(RegistryError::from)
}

pub fn enable_entry(
    conn: &Connection,
    entry: &CatalogEntry,
    catalog_version: &str,
    trigger: &str,
) -> Result<EntryRow, RegistryError> {
    if let Some(existing) = get_entry(conn, &entry.key)? {
        return Ok(existing); // 冪等(copy-on-enableは初回のみ)
    }
    if alias_exists(conn, &entry.key)? {
        return Err(RegistryError::NamespaceCollision(entry.key.clone()));
    }
    let revision = entry.revision();
    let roles_json = if entry.channel_roles.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&entry.channel_roles).expect("string vec serializes"))
    };
    conn.execute(
        "INSERT INTO registry_entries (measurement_key, origin, catalog_version, entry_revision,
            unit_ucum, unit_display, value_type, semantic_class, channel_mode, channel_roles_json,
            physical_min, physical_max, site_min, site_max, enabled_at)
         VALUES (?1, 'catalog', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, ?12)",
        params![
            entry.key, catalog_version, revision,
            entry.unit_ucum, entry.unit_display,
            entry.value_type.as_db(), entry.semantic_class, entry.channel_mode.as_db(),
            roles_json,
            entry.physical_range.map(|r| r.min), entry.physical_range.map(|r| r.max),
            now_ms(),
        ],
    )?;
    let detail = serde_json::json!({
        "key": entry.key, "revision": revision,
        "catalog_version": catalog_version, "trigger": trigger,
    })
    .to_string();
    ledger::record_event(conn, "registry_entry_enabled", None, &detail)?;
    get_entry(conn, &entry.key)?.ok_or_else(|| {
        RegistryError::Sqlite(rusqlite::Error::QueryReturnedNoRows)
    })
}

pub fn find_resolution(
    conn: &Connection,
    declared_key: &str,
) -> Result<Option<Resolution>, RegistryError> {
    if let Some(entry) = get_entry(conn, declared_key)? {
        return Ok(Some(Resolution::Entry(entry)));
    }
    let alias: Option<(String, String)> = conn
        .query_row(
            "SELECT measurement_key, alias_kind FROM registry_aliases WHERE alias = ?1",
            params![declared_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match alias {
        Some((target, kind)) => {
            let canonical = get_entry(conn, &target)?
                .ok_or_else(|| RegistryError::TargetNotFound(target))?;
            Ok(Some(Resolution::Alias { canonical, alias_kind: kind }))
        }
        None => Ok(None),
    }
}

pub fn define_alias(
    conn: &Connection,
    alias: &str,
    target_key: &str,
    kind: AliasKind,
) -> Result<(), RegistryError> {
    iotkit_ingest_contract::validate_measurement_key(alias)
        .map_err(|e| RegistryError::InvalidKey(format!("{alias}: {e}")))?;
    if get_entry(conn, alias)?.is_some() {
        return Err(RegistryError::NamespaceCollision(alias.to_string()));
    }
    if alias_exists(conn, alias)? {
        return Err(RegistryError::AliasExists(alias.to_string()));
    }
    if get_entry(conn, target_key)?.is_none() {
        return Err(RegistryError::TargetNotFound(target_key.to_string()));
    }
    conn.execute(
        "INSERT INTO registry_aliases (alias, measurement_key, alias_kind, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![alias, target_key, kind.as_db(), now_ms()],
    )?;
    // D6決定3(a): 申告キー(=alias名)のまま実体化済みのunknown_key検疫seriesへcanonical定義が
    // バインドされた → series級検疫を解除(series_keyは不変=履歴を切らない)。
    // undeclared_channel等の検疫はエイリアスでは解決しないため対象外。
    let released = ledger::release_series_quarantine_for_key(conn, alias, "unknown_key")?;
    if !released.is_empty() {
        let detail = serde_json::json!({
            "alias": alias, "canonical": target_key, "series_ids": released,
        })
        .to_string();
        ledger::record_event(conn, "series_quarantine_released", None, &detail)?;
    }
    Ok(())
}

pub fn seed_legacy_sensor_map(conn: &Connection) -> Result<usize, RegistryError> {
    let mut inserted = 0;
    for (st, key) in LEGACY_SENSOR_MAP {
        inserted += conn.execute(
            "INSERT INTO legacy_sensor_type_map (sensor_type, measurement_key, created_at)
             VALUES (?1, ?2, ?3) ON CONFLICT(sensor_type) DO NOTHING",
            params![st, key, now_ms()],
        )?;
    }
    Ok(inserted)
}

pub fn lookup_legacy(conn: &Connection, sensor_type: u16) -> Result<Option<String>, RegistryError> {
    conn.query_row(
        "SELECT measurement_key FROM legacy_sensor_type_map WHERE sensor_type = ?1",
        params![sensor_type],
        |row| row.get(0),
    )
    .optional()
    .map_err(RegistryError::from)
}
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p iotkit-core-registry`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add core/registry
git commit -m "feat(registry): copy-on-enable entries, aliases, legacy sensor-type shim (D6)"
```

---

### Task 4: 契約のquarantine_reason追加+コレクタの港(RegistryPolicy)拡張と受理順序の再配線

**Files:**
- Modify: `iotkit-ingest-contract/src/ack.rs`(QuarantineReason新設、Stored拡張=D1追補)
- Modify: `iotkit-ingest-contract/src/lib.rs`(re-export追加)
- Modify: `iotkit-ingest-contract/Cargo.toml`(dev-dependenciesに `serde_json = "1"` が無ければ追加)
- Modify: `core/collector/src/registry_policy.rs`
- Modify: `core/collector/src/actor.rs`
- Modify: `core/collector/src/lib.rs`(re-export追加)

**Interfaces:**
- Consumes: Task 2の `ledger::{CHANNEL_NA, DEFAULT_VARIANT, ensure_series(7引数), SystemId}`
- Produces(**この署名がTask 5のSqliteRegistry実装対象**):

```rust
// iotkit-ingest-contract(ワイヤ契約。D1追補と1:1)
pub enum QuarantineReason { OutOfRange, UnknownKey, UndeclaredChannel, DeviceQuarantined }
// as_str(): "out_of_range" | "unknown_key" | "undeclared_channel" | "device_quarantined"
//   (ワイヤserde表現とDBのseries.quarantine_reason列で同じ正準文字列)
ItemStatus::Stored { disposition, quarantine_reason: Option<QuarantineReason> } // additive

// core/collector(港)
pub fn is_series_level(reason: QuarantineReason) -> bool
// UnknownKey | UndeclaredChannel => true(series実体に検疫マーク)、
// OutOfRange | DeviceQuarantined => false(行レベルのみ)

pub enum RegistryVerdict {
    Accept {
        resolved_key: String,
        /// 評価器が決めた正準チャネル(DB表現。single modeのSome(0)→CHANNEL_NA正規化込み)
        channel_index: i32,
        quarantine: Option<QuarantineReason>,
    },
    RejectItem { reason_code: ReasonCode, message: String },
}

pub trait RegistryPolicy: Send + Sync {
    /// D6判別表(決定6)の評価。conn=受理トランザクション内の接続(auto-enable等の書き込みは
    /// エンベロープと同一トランザクションでcommit/rollbackされる)。
    /// Errはストレージ失敗であり、呼び出し元はackを返さない(D1。RejectItemに変換禁止)。
    /// 評価器はDeviceQuarantinedを返さない(デバイス状態はコレクタの管轄)。
    fn evaluate(
        &self,
        conn: &rusqlite::Connection,
        system_id: &iotkit_core_ledger::SystemId,
        item: &ReadingItem,
    ) -> Result<RegistryVerdict, String>;
}
```

- [ ] **Step 0: 契約クレートにquarantine_reasonを追加(D1追補 2026-07-03)**

`iotkit-ingest-contract/src/ack.rs` に追加・変更:

```rust
/// D1追補(2026-07-03): 検疫理由の可視化。D6判別表と1:1(実装はレジストリ実装=本計画と同時)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuarantineReason {
    OutOfRange,
    UnknownKey,
    UndeclaredChannel,
    DeviceQuarantined,
}

impl QuarantineReason {
    /// ワイヤserde表現とDB(series.quarantine_reason列)で同じ正準文字列を使う
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OutOfRange => "out_of_range",
            Self::UnknownKey => "unknown_key",
            Self::UndeclaredChannel => "undeclared_channel",
            Self::DeviceQuarantined => "device_quarantined",
        }
    }
}
```

`ItemStatus` を変更(additive: 旧送信者・旧JSONと互換):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ItemStatus {
    Stored {
        disposition: Disposition,
        /// disposition=quarantined のとき理由を可視化(D1追補。省略時はワイヤに現れない)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quarantine_reason: Option<QuarantineReason>,
    },
    ItemRejected { reason_code: ReasonCode, message: String },
}
```

`iotkit-ingest-contract/src/lib.rs` のre-exportに `QuarantineReason` を追加:

```rust
pub use ack::{AckStatus, Disposition, EnvelopeAck, ItemStatus, QuarantineReason, ReasonCode};
```

`ack.rs` 末尾にserde互換テストを追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_reason_is_additive_on_the_wire() {
        let s = ItemStatus::Stored { disposition: Disposition::Durable, quarantine_reason: None };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("quarantine_reason"), "additive: 省略時はワイヤに現れない");
        // 旧形式(フィールドなし)のJSONも読める
        let old: ItemStatus =
            serde_json::from_str(r#"{"kind":"stored","disposition":"quarantined"}"#).unwrap();
        assert!(matches!(old, ItemStatus::Stored { quarantine_reason: None, .. }));
        let with: ItemStatus = serde_json::from_str(
            r#"{"kind":"stored","disposition":"quarantined","quarantine_reason":"out_of_range"}"#,
        ).unwrap();
        assert!(matches!(with,
            ItemStatus::Stored { quarantine_reason: Some(QuarantineReason::OutOfRange), .. }));
    }
}
```

**機械的波及**(同Step内でコンパイルを回復させる):
- `core/collector/src/actor.rs` の `ItemStatus::Stored { disposition: ... }` **構築**2箇所(ステージング経路と最終書き込み)に `quarantine_reason: None` を仮追加(次Stepで本実装に置き換える)。
- `core/collector/src/actor.rs` テスト内の `ItemStatus::Stored { disposition: ... }` **パターン**4箇所(`known_subject_...` / `unknown_subject_...` / `malformed_...` / `cache_is_reset_...`)に `, ..` を追加。

Run: `cargo test -p iotkit-ingest-contract && cargo build -p iotkit-core-collector`
Expected: PASS / ビルド成功

- [ ] **Step 1: 失敗するテストを書く**

`core/collector/src/registry_policy.rs` を全面書き換え:

```rust
use iotkit_core_ledger::{SystemId, CHANNEL_NA};
use iotkit_ingest_contract::{QuarantineReason, ReadingItem, ReasonCode};

/// series行にも検疫マークを付ける理由か(D6決定6)。
/// UnknownKey/UndeclaredChannelはseries実体そのものが疑わしい。
/// OutOfRangeはseriesは健全で観測だけが外れ値、DeviceQuarantinedはデバイス状態由来なのでfalse。
pub fn is_series_level(reason: QuarantineReason) -> bool {
    matches!(reason, QuarantineReason::UnknownKey | QuarantineReason::UndeclaredChannel)
}

#[derive(Debug, Clone)]
pub enum RegistryVerdict {
    Accept {
        /// エイリアス解決後のmeasurement_key(D6決定3。series実体化にはこちらを使う)
        resolved_key: String,
        /// 評価器が決めた正準チャネル(DB表現)。single modeの Some(0)→CHANNEL_NA 正規化込み
        /// (None/Some(0)で同一測定が別seriesに分裂するのを防ぐ)。コレクタは再計算しない。
        channel_index: i32,
        quarantine: Option<QuarantineReason>,
    },
    RejectItem { reason_code: ReasonCode, message: String },
}

/// 受理時のレジストリ検証フック(D6判別表)。本実装はiotkit-core-registryのSqliteRegistry。
/// Errはストレージ失敗として呼び出し元がackなしで処理する(D1)——RejectItemへの変換は禁止。
/// 評価器はDeviceQuarantinedを返さない(デバイス状態はコレクタの管轄)。
pub trait RegistryPolicy: Send + Sync {
    fn evaluate(
        &self,
        conn: &rusqlite::Connection,
        system_id: &SystemId,
        item: &ReadingItem,
    ) -> Result<RegistryVerdict, String>;
}

/// テスト用の素通し実装: 常にAccept(検疫なし・キー無変換・チャネル生写像)。
/// 文法検査は計画2以降コレクタ本体のprecheckに移った(このポリシーの仕事ではない)。
pub struct PermissiveRegistry;

impl RegistryPolicy for PermissiveRegistry {
    fn evaluate(
        &self,
        _conn: &rusqlite::Connection,
        _system_id: &SystemId,
        item: &ReadingItem,
    ) -> Result<RegistryVerdict, String> {
        Ok(RegistryVerdict::Accept {
            resolved_key: item.measurement_key.clone(),
            channel_index: item.channel_index.map(i32::from).unwrap_or(CHANNEL_NA),
            quarantine: None,
        })
    }
}
```

`core/collector/src/actor.rs` のtestsモジュールに追加(既存テストは全て維持——`malformed_measurement_key_rejects_item_but_stores_valid_sibling` は文法precheckがコレクタ本体に移っても同じ挙動を検証し続ける。`QuarantineReason` は `use iotkit_ingest_contract::*` 経由で既にスコープ内)。スタブとテストを追加:

```rust
    fn raw_channel(item: &ReadingItem) -> i32 {
        item.channel_index.map(i32::from).unwrap_or(ledger::CHANNEL_NA)
    }

    /// 検疫理由付きのスタブポリシー(コレクタがverdictをseries/行/ackへ正しく写像するかの検証用)
    struct QuarantiningStub(QuarantineReason);
    impl crate::registry_policy::RegistryPolicy for QuarantiningStub {
        fn evaluate(
            &self,
            _conn: &rusqlite::Connection,
            _system_id: &ledger::SystemId,
            item: &ReadingItem,
        ) -> Result<crate::registry_policy::RegistryVerdict, String> {
            Ok(crate::registry_policy::RegistryVerdict::Accept {
                resolved_key: item.measurement_key.clone(),
                channel_index: raw_channel(item),
                quarantine: Some(self.0),
            })
        }
    }

    /// キーとチャネルを書き換えるスタブ(verdictの写像がseries実体化に反映されるかの検証用)
    struct RenamingStub;
    impl crate::registry_policy::RegistryPolicy for RenamingStub {
        fn evaluate(
            &self,
            _conn: &rusqlite::Connection,
            _system_id: &ledger::SystemId,
            _item: &ReadingItem,
        ) -> Result<crate::registry_policy::RegistryVerdict, String> {
            Ok(crate::registry_policy::RegistryVerdict::Accept {
                resolved_key: "temperature_c".into(),
                channel_index: 7, // コレクタが自前計算せずverdictの値を使うことの検証
                quarantine: None,
            })
        }
    }

    /// Errを返すスタブ(ストレージ失敗の伝播=ackなしの検証用)
    struct FailingPolicy;
    impl crate::registry_policy::RegistryPolicy for FailingPolicy {
        fn evaluate(
            &self,
            _conn: &rusqlite::Connection,
            _system_id: &ledger::SystemId,
            _item: &ReadingItem,
        ) -> Result<crate::registry_policy::RegistryVerdict, String> {
            Err("simulated registry storage failure".into())
        }
    }

    #[tokio::test]
    async fn unknown_key_quarantine_marks_series_row_and_ack_reason() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(
            db.clone(), Arc::new(QuarantiningStub(QuarantineReason::UnknownKey)), 16);
        let ack = collector.submit(env("e-q1", "ble:aa", "custom.mystery")).await.unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored {
                disposition: Disposition::Quarantined,
                quarantine_reason: Some(QuarantineReason::UnknownKey),
            })), "ackに検疫理由が可視化される(D1追補)");
        let (s_q, s_reason, r_q): (i64, Option<String>, i64) = db.with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT quarantined FROM series", [], |r| r.get(0)).unwrap(),
                conn.query_row("SELECT quarantine_reason FROM series", [], |r| r.get(0)).unwrap(),
                conn.query_row("SELECT quarantined FROM readings", [], |r| r.get(0)).unwrap(),
            ))
        }).unwrap();
        assert_eq!(s_q, 1, "unknown keyはseries級検疫");
        assert_eq!(s_reason.as_deref(), Some("unknown_key"));
        assert_eq!(r_q, 1);
    }

    #[tokio::test]
    async fn out_of_range_quarantines_row_but_not_series() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(
            db.clone(), Arc::new(QuarantiningStub(QuarantineReason::OutOfRange)), 16);
        let ack = collector.submit(env("e-q2", "ble:aa", "temperature_c")).await.unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored {
                disposition: Disposition::Quarantined,
                quarantine_reason: Some(QuarantineReason::OutOfRange),
            })));
        let (s_q, s_reason, r_q): (i64, Option<String>, i64) = db.with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT quarantined FROM series", [], |r| r.get(0)).unwrap(),
                conn.query_row("SELECT quarantine_reason FROM series", [], |r| r.get(0)).unwrap(),
                conn.query_row("SELECT quarantined FROM readings", [], |r| r.get(0)).unwrap(),
            ))
        }).unwrap();
        assert_eq!(s_q, 0, "値域外はseriesを汚さない(行級のみ)");
        assert_eq!(s_reason, None);
        assert_eq!(r_q, 1);
    }

    #[tokio::test]
    async fn device_quarantine_is_visible_as_ack_reason() {
        // 検疫状態デバイス(D5経路A: 承認→検疫→active の途中)のデータは行検疫+理由device_quarantined
        let db = test_db();
        db.with_conn_sync(|conn| {
            ledger::record_sighting(conn, "ble:q", "test-adapter").unwrap();
            ledger::approve_sighting(conn, "ble:q", None, ledger::DeviceKind::Individual).unwrap();
            Ok(())
        }).unwrap();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let ack = collector.submit(env("e-dq", "ble:q", "temperature_c")).await.unwrap();
        let AckStatus::Accepted { items } = ack.status else { panic!("expected Accepted") };
        assert!(matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Quarantined,
            quarantine_reason: Some(QuarantineReason::DeviceQuarantined),
        }));
    }

    #[tokio::test]
    async fn verdict_resolved_key_and_channel_are_used_for_series() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(RenamingStub), 16);
        collector.submit(env("e-alias", "ble:aa", "temp_old")).await.unwrap();
        let (key, ch): (String, i32) = db.with_conn_sync(|conn| {
            Ok(conn.query_row(
                "SELECT measurement_key, channel_index FROM series", [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ).unwrap())
        }).unwrap();
        assert_eq!(key, "temperature_c", "series実体化はresolved_keyを使う");
        assert_eq!(ch, 7, "コレクタはチャネルを再計算せずverdictのchannel_indexを使う");
    }

    #[tokio::test]
    async fn policy_storage_failure_produces_no_ack() {
        // レジストリ評価のErrはRejectedではなくackなし(D1。計画1 T6教訓の踏襲)
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(FailingPolicy), 16);
        let result = collector.submit(env("e-fail", "ble:aa", "temperature_c")).await;
        assert!(matches!(result, Err(CollectorClosed)));
        let n: i64 = db.with_conn_sync(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap())
        }).unwrap();
        assert_eq!(n, 0, "エンベロープ全体がロールバックされる");
    }
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p iotkit-core-collector`
Expected: FAIL(コンパイルエラー: actor.rsが旧トレイト署名のまま)

- [ ] **Step 3: actor.rs の process_item を再配線**

処理順序を変更する: **文法precheck → subject解決(未知ならステージング) → レジストリ評価 → series実体化(resolved_key+検疫reason) → 書き込み**。既知subjectのみレジストリ評価に到達する(ステージング経路はレジストリを参照しない——検疫デバイス承認前のデータはpayload_json保全のみ=計画1と同じ)。

```rust
fn process_item(
    conn: &rusqlite::Connection,
    cache: &mut ResolutionCache,
    policy: &dyn RegistryPolicy,
    envelope: &Envelope,
    item: &ReadingItem,
    received_at: i64,
) -> Result<ItemStatus, String> {
    // 1) 文法検査(決定的契約違反。レジストリにもDBにも触れず判定できるためprecheck)
    if let Err(e) = validate_measurement_key(&item.measurement_key) {
        return Ok(ItemStatus::ItemRejected {
            reason_code: ReasonCode::MalformedMeasurementKey,
            message: e.to_string(),
        });
    }
    // 2) subject解決(D5決定1: 送信者+subject_hint→台帳)。hint欠如も決定的な契約違反
    let Some(hw) = item.subject_hint.as_deref() else {
        return Ok(ItemStatus::ItemRejected {
            reason_code: ReasonCode::UnknownSubject,
            message: "subject_hint required for multi-subject sender".into(),
        });
    };
    let resolved = match cache.devices.get(hw) {
        Some(hit) => Some(*hit),
        None => match ledger::find_alive_by_hardware_id(conn, hw).map_err(|e| e.to_string())? {
            Some(row) => {
                cache.devices.insert(hw.to_string(), (row.system_id, row.state));
                Some((row.system_id, row.state))
            }
            None => None,
        },
    };
    let Some((system_id, state)) = resolved else {
        // 3) 未知subject → 目撃ステージング(D5決定4経路A、ack=staged)。レジストリ評価はしない
        let payload = serde_json::to_string(item).unwrap_or_else(|_| "{}".into());
        ledger::record_sighting(conn, hw, &envelope.source).map_err(|e| e.to_string())?;
        ts::insert_staged_reading(conn, hw, received_at, &payload).map_err(|e| e.to_string())?;
        return Ok(ItemStatus::Stored {
            disposition: Disposition::Staged,
            quarantine_reason: None, // stagedとquarantinedは直列に成立しない(D1: subject解決が常に先)
        });
    };
    // 4) レジストリ評価(D6判別表)。Errはストレージ失敗=ackなしへ伝播(D1)
    let (resolved_key, channel, registry_quarantine) =
        match policy.evaluate(conn, &system_id, item)? {
            RegistryVerdict::Accept { resolved_key, channel_index, quarantine } => {
                (resolved_key, channel_index, quarantine)
            }
            RegistryVerdict::RejectItem { reason_code, message } => {
                return Ok(ItemStatus::ItemRejected { reason_code, message });
            }
        };
    // 5) series解決(検疫デバイスのデータは検疫行として保存=D1オンボーディング)。
    //    チャネルは評価器が返した正準値をそのまま使う(再計算しない=Global Constraints)
    let device_quarantined = state == ledger::DeviceState::Quarantined;
    let variant = item
        .series_variant
        .as_deref()
        .unwrap_or(ledger::DEFAULT_VARIANT)
        .to_string();
    let series_quarantined = registry_quarantine.map_or(false, is_series_level);
    let skey = (system_id, resolved_key.clone(), channel, variant.clone());
    let series_id = match cache.series.get(&skey) {
        Some(id) => *id,
        None => {
            let reason = registry_quarantine
                .filter(|q| is_series_level(*q))
                .map(|q| q.as_str());
            let id = ledger::ensure_series(
                conn, &system_id, &resolved_key, channel, &variant, series_quarantined, reason,
            )
            .map_err(|e| e.to_string())?;
            cache.series.insert(skey, id);
            id
        }
    };
    // 6) 書き込み+ackへの検疫理由可視化(D1追補)。レジストリ起因の理由が具体的なので優先し、
    //    無ければデバイス検疫を報告する
    let row_quarantined = registry_quarantine.is_some() || device_quarantined;
    let wire_reason = registry_quarantine
        .or_else(|| device_quarantined.then_some(QuarantineReason::DeviceQuarantined));
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
    ts::insert_reading_v3(conn, &new).map_err(|e| e.to_string())?;
    Ok(ItemStatus::Stored {
        disposition: if row_quarantined { Disposition::Quarantined } else { Disposition::Durable },
        quarantine_reason: if row_quarantined { wire_reason } else { None },
    })
}
```

`actor.rs` の `use` 節を `use crate::registry_policy::{is_series_level, RegistryPolicy, RegistryVerdict};` に更新する(`QuarantineReason` は `use iotkit_ingest_contract::*` 経由)。`core/collector/src/lib.rs`:

```rust
pub use registry_policy::{is_series_level, PermissiveRegistry, RegistryPolicy, RegistryVerdict};
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p iotkit-ingest-contract && cargo test -p iotkit-core-collector`
Expected: PASS(collectorは既存9テスト+新6テスト。既存テストは挙動不変の回帰確認になる)

- [ ] **Step 5: Commit**

```bash
git add iotkit-ingest-contract core/collector
git commit -m "feat(contract,collector): ack quarantine_reason (D1 addendum) + widen RegistryPolicy port for D6 verdict"
```

---

### Task 5: SqliteRegistry評価器(D6判別表の本実装)

**Files:**
- Create: `core/registry/src/policy.rs`
- Modify: `core/registry/src/lib.rs`(`pub mod policy;` と `pub use policy::SqliteRegistry;`)
- Modify: `core/registry/Cargo.toml`(dependenciesに `iotkit-core-collector = { path = "../collector" }`)

**Interfaces:**
- Consumes: Task 3のstore関数、Task 4の港(`RegistryPolicy` / `RegistryVerdict` / `QuarantineReason`)、`ledger::{find_series_meta, series_exists_for_key, CHANNEL_NA, DEFAULT_VARIANT}`
- Produces: `iotkit_core_registry::SqliteRegistry`(unit struct、`RegistryPolicy` 実装)

**評価順序(D6決定6の判別表+決定3(a)+決定7)**:
1. 解決: entries直ヒット → aliasヒット(series不変規則: 当該subjectで申告キーが実体化済みなら申告キー維持、未実体化ならcanonicalへ。**申告キーがカタログキーでもある場合はエイリアスが自動有効化を遮蔽している状態なのでwarnログで可視化**=D6決定3の「明示解決を要求」のWave 0形) → カタログヒット(auto-enable+監査) → 未知(→ `Accept{申告キー, 生チャネル, Some(UnknownKey)}`、以降の検査はスキップ)
2. 値型検査(→ `RejectItem(value_type_mismatch)`): record型 / values空 / スカラーでvalues.len()≠1 / boolで値∉{0,1} / intで非整数
3. チャネル検査+**正準化**: single: `None|Some(0)` → `CHANNEL_NA` に正規化して通す(series分裂防止)、それ以外→ `Some(UndeclaredChannel)` / fixed: `Some(i), i<roles.len()` → `i` で通す、それ以外(Noneも帰属不能)→検疫 / generic: 生写像で常に通す(宣言照合はWave 1)。**正準チャネルはAcceptの `channel_index` で返す**
4. 値域検査(→ `Some(OutOfRange)`): **min/max各辺独立に** series個別 → エントリ現場既定(site) → カタログ物理限界(physical) をフォールバック(片辺のみのseries上書きでも反対辺は外殻が生きる=決定7外殻不変則)
5. 検疫理由の優先順位: UnknownKey > UndeclaredChannel > OutOfRange(最初にヒットしたもの1つを返す)。DeviceQuarantinedは返さない(コレクタの管轄)

- [ ] **Step 1: 失敗するテストを書く**

`core/registry/src/policy.rs`:

```rust
//! SqliteRegistry: D6決定6(受理判別表)の評価器。受理トランザクション内で呼ばれる。
use crate::catalog::{standard_catalog, ChannelMode, ValueType};
use crate::store::{self, EntryRow, Resolution};
use iotkit_core_collector::{RegistryPolicy, RegistryVerdict};
use iotkit_core_ledger as ledger;
use iotkit_ingest_contract::{QuarantineReason, ReadingItem, ReasonCode};

/// 現場レジストリ(SQLite)を参照するRegistryPolicy本実装。状態はすべてDBにあり、
/// この構造体自体はステートレス(Arcで共有可)。
pub struct SqliteRegistry;

impl RegistryPolicy for SqliteRegistry {
    fn evaluate(
        &self,
        conn: &rusqlite::Connection,
        system_id: &ledger::SystemId,
        item: &ReadingItem,
    ) -> Result<RegistryVerdict, String> {
        evaluate_item(conn, system_id, item)
    }
}

fn evaluate_item(
    conn: &rusqlite::Connection,
    system_id: &ledger::SystemId,
    item: &ReadingItem,
) -> Result<RegistryVerdict, String> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{define_alias, enable_entry, AliasKind};
    use iotkit_core_ledger::{
        ensure_series, insert_device, DeviceKind, DeviceState, NewDevice, SystemId,
        CHANNEL_NA, DEFAULT_VARIANT,
    };
    use iotkit_ingest_contract::TimeSource;

    fn test_db() -> iotkit_core_storage::DbHandle {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(crate::MIGRATIONS);
        all.sort_by_key(|m| m.version);
        iotkit_core_storage::init_db_memory(&all).unwrap()
    }

    fn device(conn: &rusqlite::Connection) -> SystemId {
        insert_device(conn, &NewDevice {
            hardware_id: "ble:aa".into(), user_label: None, parent: None,
            kind: DeviceKind::Individual, initial_state: DeviceState::Active,
        }).unwrap()
    }

    fn item(key: &str, channel: Option<u16>, values: Vec<f64>) -> ReadingItem {
        ReadingItem {
            subject_hint: Some("ble:aa".into()),
            measurement_key: key.into(),
            channel_index: channel,
            series_variant: None,
            values,
            device_time_ms: None,
            time_source: TimeSource::Gateway,
            age_ms: None, rssi: None, battery_pct: None,
        }
    }

    fn eval(
        conn: &rusqlite::Connection, sid: &SystemId, it: &ReadingItem,
    ) -> RegistryVerdict {
        evaluate_item(conn, sid, it).unwrap()
    }

    #[test]
    fn catalog_key_first_arrival_auto_enables_with_audit() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            let v = eval(conn, &sid, &item("temperature_c", None, vec![21.5]));
            assert!(matches!(v,
                RegistryVerdict::Accept { ref resolved_key, quarantine: None, .. }
                if resolved_key == "temperature_c"));
            // copy-on-enableされている
            let entry = store::get_entry(conn, "temperature_c").unwrap().unwrap();
            assert_eq!(entry.origin, "catalog");
            // 監査イベント(D6決定4で必須)
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                [], |r| r.get(0),
            ).unwrap();
            assert_eq!(n, 1);
            // 2回目はauto-enableしない(冪等)
            eval(conn, &sid, &item("temperature_c", None, vec![22.0]));
            let n2: i64 = conn.query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                [], |r| r.get(0),
            ).unwrap();
            assert_eq!(n2, 1);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn unknown_key_is_quarantined_not_enabled() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            let v = eval(conn, &sid, &item("custom.tank_level", None, vec![42.0]));
            assert!(matches!(v,
                RegistryVerdict::Accept { ref resolved_key, quarantine: Some(QuarantineReason::UnknownKey), .. }
                if resolved_key == "custom.tank_level"));
            assert!(store::get_entry(conn, "custom.tank_level").unwrap().is_none(),
                "カタログ外キーは有効化されない(D6決定4)");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn in_range_and_out_of_range_against_catalog_physical_limit() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            let ok = eval(conn, &sid, &item("temperature_c", None, vec![21.5]));
            assert!(matches!(ok, RegistryVerdict::Accept { quarantine: None, .. }));
            let hot = eval(conn, &sid, &item("temperature_c", None, vec![5000.0]));
            assert!(matches!(hot,
                RegistryVerdict::Accept { quarantine: Some(QuarantineReason::OutOfRange), .. }));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn series_range_override_narrows_catalog_range() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            // series実体化+個別値域を直接設定(設定APIはR14=Wave 1のためSQL直書きで模擬)
            ensure_series(conn, &sid, "temperature_c", CHANNEL_NA, DEFAULT_VARIANT, false, None).unwrap();
            conn.execute(
                "UPDATE series SET range_min = -10.0, range_max = 50.0 WHERE measurement_key='temperature_c'",
                [],
            ).unwrap();
            // 物理限界内(-200..1372)だがseries個別(-10..50)の外 → OutOfRange
            let v = eval(conn, &sid, &item("temperature_c", None, vec![100.0]));
            assert!(matches!(v,
                RegistryVerdict::Accept { quarantine: Some(QuarantineReason::OutOfRange), .. }));
            let ok = eval(conn, &sid, &item("temperature_c", None, vec![25.0]));
            assert!(matches!(ok, RegistryVerdict::Accept { quarantine: None, .. }));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn site_default_range_applies_when_no_series_override() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            eval(conn, &sid, &item("temperature_c", None, vec![21.5])); // auto-enable
            conn.execute(
                "UPDATE registry_entries SET site_min = 0.0, site_max = 100.0
                 WHERE measurement_key='temperature_c'",
                [],
            ).unwrap();
            let v = eval(conn, &sid, &item("temperature_c", None, vec![150.0]));
            assert!(matches!(v,
                RegistryVerdict::Accept { quarantine: Some(QuarantineReason::OutOfRange), .. }));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn bool_value_type_mismatch_is_terminal_reject() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            let ok = eval(conn, &sid, &item("contact_state", None, vec![1.0]));
            assert!(matches!(ok, RegistryVerdict::Accept { quarantine: None, .. }));
            let bad = eval(conn, &sid, &item("contact_state", None, vec![3.0]));
            assert!(matches!(bad,
                RegistryVerdict::RejectItem { reason_code: ReasonCode::ValueTypeMismatch, .. }));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn scalar_with_multiple_values_and_empty_values_are_rejected() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            let multi = eval(conn, &sid, &item("temperature_c", None, vec![1.0, 2.0]));
            assert!(matches!(multi,
                RegistryVerdict::RejectItem { reason_code: ReasonCode::ValueTypeMismatch, .. }));
            let empty = eval(conn, &sid, &item("temperature_c", None, vec![]));
            assert!(matches!(empty,
                RegistryVerdict::RejectItem { reason_code: ReasonCode::ValueTypeMismatch, .. }));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn vibration_spectrum_record_is_rejected_in_wave0() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            let v = eval(conn, &sid, &item("vibration_spectrum", None, vec![1.0]));
            assert!(matches!(v,
                RegistryVerdict::RejectItem { reason_code: ReasonCode::ValueTypeMismatch, .. }),
                "record型のワイヤ表現は第二波(D6決定10)——f64配列としては解釈不能");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn fixed_channel_bounds_are_enforced() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            for ch in 0..3u16 {
                let v = eval(conn, &sid, &item("acceleration_mg", Some(ch), vec![100.0]));
                assert!(matches!(v, RegistryVerdict::Accept { quarantine: None, .. }),
                    "channel {ch} is declared");
            }
            let v = eval(conn, &sid, &item("acceleration_mg", Some(3), vec![100.0]));
            assert!(matches!(v,
                RegistryVerdict::Accept { quarantine: Some(QuarantineReason::UndeclaredChannel), .. }));
            let none = eval(conn, &sid, &item("acceleration_mg", None, vec![100.0]));
            assert!(matches!(none,
                RegistryVerdict::Accept { quarantine: Some(QuarantineReason::UndeclaredChannel), .. }),
                "fixed型でchannel_indexなしは帰属不能=宣言外扱い");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn single_channel_accepts_none_or_zero_only() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            assert!(matches!(eval(conn, &sid, &item("distance_mm", None, vec![100.0])),
                RegistryVerdict::Accept { quarantine: None, .. }));
            assert!(matches!(eval(conn, &sid, &item("distance_mm", Some(0), vec![100.0])),
                RegistryVerdict::Accept { quarantine: None, .. }));
            assert!(matches!(eval(conn, &sid, &item("distance_mm", Some(1), vec![100.0])),
                RegistryVerdict::Accept { quarantine: Some(QuarantineReason::UndeclaredChannel), .. }));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn generic_channel_accepts_any_index_in_wave0() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            for ch in [None, Some(0), Some(1), Some(7)] {
                let v = eval(conn, &sid, &item("voltage_mv", ch, vec![1650.0]));
                assert!(matches!(v, RegistryVerdict::Accept { quarantine: None, .. }),
                    "generic modeは宣言照合なし(Wave 1)なので{ch:?}を通す");
            }
            Ok(())
        }).unwrap();
    }

    #[test]
    fn alias_resolves_to_canonical_for_unmaterialized_declared_key() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            let cat = standard_catalog();
            enable_entry(conn, cat.find("temperature_c").unwrap(), &cat.catalog_version, "test").unwrap();
            define_alias(conn, "temp_old", "temperature_c", AliasKind::SiteMapping).unwrap();
            let v = eval(conn, &sid, &item("temp_old", None, vec![21.5]));
            assert!(matches!(v,
                RegistryVerdict::Accept { ref resolved_key, quarantine: None, .. }
                if resolved_key == "temperature_c"),
                "未実体化の申告はcanonicalへ写像(D6決定3(b))");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn alias_keeps_declared_key_when_series_already_materialized() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            let cat = standard_catalog();
            enable_entry(conn, cat.find("temperature_c").unwrap(), &cat.catalog_version, "test").unwrap();
            // 先に申告キーのままのseriesが存在する状況を作る(検疫期にunknown keyとして実体化済み)
            ensure_series(conn, &sid, "temp_old", CHANNEL_NA, DEFAULT_VARIANT, true, Some("unknown_key")).unwrap();
            // エイリアス確立=canonical定義バインドで検疫解除される(Task 3)
            define_alias(conn, "temp_old", "temperature_c", AliasKind::SiteMapping).unwrap();
            let meta = iotkit_core_ledger::find_series_meta(
                conn, &sid, "temp_old", CHANNEL_NA, DEFAULT_VARIANT).unwrap().unwrap();
            assert!(!meta.quarantined, "確立時点でseries検疫は解除済み(D6決定3(a))");
            let v = eval(conn, &sid, &item("temp_old", None, vec![21.5]));
            assert!(matches!(v,
                RegistryVerdict::Accept { ref resolved_key, quarantine: None, .. }
                if resolved_key == "temp_old"),
                "実体化済み申告キーはseries_key不変(D6決定3(a))。検証はcanonical定義で行う");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn single_mode_normalizes_some_zero_to_channel_na() {
        // single測定の None / Some(0) が同一seriesに落ちる(正準チャネル=番兵-1)
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            let v0 = eval(conn, &sid, &item("distance_mm", Some(0), vec![100.0]));
            assert!(matches!(v0,
                RegistryVerdict::Accept { channel_index: ledger::CHANNEL_NA, quarantine: None, .. }));
            let vn = eval(conn, &sid, &item("distance_mm", None, vec![100.0]));
            assert!(matches!(vn,
                RegistryVerdict::Accept { channel_index: ledger::CHANNEL_NA, quarantine: None, .. }));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn range_fallback_is_per_side_preserving_outer_shell() {
        // D6決定7外殻不変則: series個別がminのみ設定でも、max側はカタログ物理限界が生きる
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            ensure_series(conn, &sid, "temperature_c", CHANNEL_NA, DEFAULT_VARIANT, false, None).unwrap();
            conn.execute(
                "UPDATE series SET range_min = -10.0 WHERE measurement_key='temperature_c'",
                [],
            ).unwrap(); // range_maxはNULLのまま
            let hot = eval(conn, &sid, &item("temperature_c", None, vec![5000.0]));
            assert!(matches!(hot,
                RegistryVerdict::Accept { quarantine: Some(QuarantineReason::OutOfRange), .. }),
                "max側はカタログ物理限界(1372)が生きる——外殻は消えない");
            let cold = eval(conn, &sid, &item("temperature_c", None, vec![-50.0]));
            assert!(matches!(cold,
                RegistryVerdict::Accept { quarantine: Some(QuarantineReason::OutOfRange), .. }),
                "min側はseries個別(-10)が適用される");
            let ok = eval(conn, &sid, &item("temperature_c", None, vec![25.0]));
            assert!(matches!(ok, RegistryVerdict::Accept { quarantine: None, .. }));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn unknown_key_priority_beats_channel_and_range_checks() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            // 未知キー+変なchannel: UnknownKeyが優先(定義がないので他の検査は無意味)
            let v = eval(conn, &sid, &item("custom.x", Some(9), vec![1e18]));
            assert!(matches!(v,
                RegistryVerdict::Accept { quarantine: Some(QuarantineReason::UnknownKey), .. }));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn evaluate_propagates_storage_failure_as_err() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            let sid = device(conn);
            conn.execute_batch("PRAGMA query_only = ON;").unwrap();
            // auto-enableのINSERTが失敗する → Err(RejectItemに変換されないこと=D1)
            let r = evaluate_item(conn, &sid, &item("temperature_c", None, vec![21.5]));
            assert!(r.is_err(), "storage failure must surface as Err, got {r:?}");
            conn.execute_batch("PRAGMA query_only = OFF;").unwrap();
            Ok(())
        }).unwrap();
    }
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p iotkit-core-registry`
Expected: FAIL(`todo!()`)

- [ ] **Step 3: 実装(evaluate_item)**

```rust
fn evaluate_item(
    conn: &rusqlite::Connection,
    system_id: &ledger::SystemId,
    item: &ReadingItem,
) -> Result<RegistryVerdict, String> {
    let raw_channel: i32 = item.channel_index.map(i32::from).unwrap_or(ledger::CHANNEL_NA);
    // 1) 解決: entries → aliases(series_key不変規則) → カタログauto-enable → 未知
    let declared = item.measurement_key.as_str();
    let (entry, resolved_key): (EntryRow, String) =
        match store::find_resolution(conn, declared).map_err(|e| e.to_string())? {
            Some(Resolution::Entry(e)) => (e, declared.to_string()),
            Some(Resolution::Alias { canonical, .. }) => {
                if standard_catalog().find(declared).is_some() {
                    // D6決定3: カタログキーと同名の現場エイリアスは自動有効化を遮蔽する。
                    // 明示解決(R14)が要る状態——Wave 0はwarnログで可視化する。
                    tracing::warn!(
                        key = declared,
                        "catalog key shadowed by site alias; explicit resolution required (D6)"
                    );
                }
                let materialized = ledger::series_exists_for_key(conn, system_id, declared)
                    .map_err(|e| e.to_string())?;
                if materialized {
                    // D6決定3(a): 実体化済み申告キーは不変。検証はcanonical定義で行う
                    // (series級検疫の解除はdefine_alias確立時に済んでいる=Task 3)
                    (canonical, declared.to_string())
                } else {
                    // D6決定3(b): 未実体化はcanonicalとして実体化
                    let key = canonical.measurement_key.clone();
                    (canonical, key)
                }
            }
            None => match standard_catalog().find(declared) {
                Some(cat_entry) => {
                    // D6決定4: カタログ内キーの初観測は自動有効化+監査イベント必須。
                    // ストレージ失敗はErrのまま上へ(ackなし=D1)——RejectItemに変換しない。
                    let e = store::enable_entry(
                        conn, cat_entry, &standard_catalog().catalog_version, "auto",
                    )
                    .map_err(|e| e.to_string())?;
                    (e, declared.to_string())
                }
                None => {
                    // 文法適合の未知キー → 検疫(D6決定6)。定義がないため以降の検査は行わない
                    return Ok(RegistryVerdict::Accept {
                        resolved_key: declared.to_string(),
                        channel_index: raw_channel,
                        quarantine: Some(QuarantineReason::UnknownKey),
                    });
                }
            },
        };

    // 2) 値型検査(構造的に解釈不能=終端Rejected、D6決定6)
    if entry.value_type == ValueType::Record {
        return Ok(RegistryVerdict::RejectItem {
            reason_code: ReasonCode::ValueTypeMismatch,
            message: format!(
                "'{}' is a record type: wire representation is reserved for a future contract addendum (D6)",
                entry.measurement_key
            ),
        });
    }
    if item.values.len() != 1 {
        return Ok(RegistryVerdict::RejectItem {
            reason_code: ReasonCode::ValueTypeMismatch,
            message: format!(
                "scalar measurement expects exactly 1 value, got {} (multi-channel data must be split into per-channel items)",
                item.values.len()
            ),
        });
    }
    let value = item.values[0];
    match entry.value_type {
        ValueType::Bool if value != 0.0 && value != 1.0 => {
            return Ok(RegistryVerdict::RejectItem {
                reason_code: ReasonCode::ValueTypeMismatch,
                message: format!("bool measurement expects 0 or 1, got {value}"),
            });
        }
        ValueType::Int if value.fract() != 0.0 => {
            return Ok(RegistryVerdict::RejectItem {
                reason_code: ReasonCode::ValueTypeMismatch,
                message: format!("int measurement expects an integral value, got {value}"),
            });
        }
        _ => {}
    }

    // 3) チャネル検査+正準化(D6決定6/12)。single modeは Some(0) も番兵-1へ寄せ、
    //    None/Some(0)で同一測定が別seriesに分裂するのを防ぐ
    let (channel, undeclared_channel) = match entry.channel_mode {
        ChannelMode::Single => match item.channel_index {
            None | Some(0) => (ledger::CHANNEL_NA, false),
            Some(_) => (raw_channel, true),
        },
        ChannelMode::Fixed => match item.channel_index {
            Some(i) if (i as usize) < entry.channel_roles.len() => (raw_channel, false),
            _ => (raw_channel, true), // 範囲外もNone(帰属不能)も宣言外
        },
        ChannelMode::Generic => (raw_channel, false), // 宣言照合はWave 1(能力宣言=キュー5)
    };
    if undeclared_channel {
        return Ok(RegistryVerdict::Accept {
            resolved_key,
            channel_index: channel,
            quarantine: Some(QuarantineReason::UndeclaredChannel),
        });
    }

    // 4) 値域検査: min/max各辺独立に series個別 → エントリ現場既定 → カタログ物理限界を
    //    フォールバック(D6決定7外殻不変則: 片辺のみのseries上書きでも反対辺は外殻が生きる)
    let variant = item.series_variant.as_deref().unwrap_or(ledger::DEFAULT_VARIANT);
    let series_meta = ledger::find_series_meta(conn, system_id, &resolved_key, channel, variant)
        .map_err(|e| e.to_string())?;
    let series_min = series_meta.as_ref().and_then(|m| m.range_min);
    let series_max = series_meta.as_ref().and_then(|m| m.range_max);
    let min = series_min.or(entry.site_min).or(entry.physical_min);
    let max = series_max.or(entry.site_max).or(entry.physical_max);
    let out_of_range =
        min.map_or(false, |lo| value < lo) || max.map_or(false, |hi| value > hi);
    if out_of_range {
        return Ok(RegistryVerdict::Accept {
            resolved_key,
            channel_index: channel,
            quarantine: Some(QuarantineReason::OutOfRange),
        });
    }

    Ok(RegistryVerdict::Accept { resolved_key, channel_index: channel, quarantine: None })
}
```

`core/registry/src/lib.rs`:

```rust
pub mod policy;

pub use policy::SqliteRegistry;
```

`core/registry/Cargo.toml` dependenciesに追加:

```toml
iotkit-core-collector = { path = "../collector" }
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p iotkit-core-registry`
Expected: PASS(policy.rsの判別表・正規化・外殻テスト17本+既存)

- [ ] **Step 5: Commit**

```bash
git add core/registry
git commit -m "feat(registry): SqliteRegistry evaluator implementing the D6 admission decision table"
```

---

### Task 6: E2E(コレクタ×SqliteRegistry)+ゲートウェイ配線+全体テスト

**Files:**
- Create: `core/registry/tests/e2e_collector.rs`(統合テスト)
- Modify: `core/registry/Cargo.toml`(dev-dependenciesに tokio / iotkit-core-timeseries)
- Modify: `iotkit-gateway/Cargo.toml`(dependenciesに `iotkit-core-registry`)
- Modify: `iotkit-gateway/src/main.rs`(migrations連結+SqliteRegistry配線)
- Modify: `iotkit-gateway/src/bridge.rs`(統合テストのSqliteRegistry切り替え)

**Interfaces:**
- Consumes: 全前タスクの成果物
- Produces: 実行系(iotkit-gateway)がD6判別表で受理判定する構成

- [ ] **Step 1: 失敗するテストを書く(E2E)**

`core/registry/Cargo.toml` のdev-dependenciesに追加:

```toml
iotkit-core-timeseries = { path = "../timeseries" }
tokio = { version = "1", features = ["sync", "rt", "macros", "rt-multi-thread"] }
```

`core/registry/tests/e2e_collector.rs`:

```rust
//! D6判別表のE2E: Envelope → Collector(SqliteRegistry) → readings/series/registry_entries。
//! ackの各語彙とDB状態の対応を、コレクタ実物のトランザクション境界越しに検証する。
use iotkit_core_collector::Collector;
use iotkit_core_ledger as ledger;
use iotkit_core_registry::SqliteRegistry;
use iotkit_ingest_contract::*;
use std::sync::Arc;

fn full_db() -> iotkit_core_storage::DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(ledger::MIGRATIONS); // 3, 5
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS); // 2, 4
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS); // 6
    all.sort_by_key(|m| m.version); // 1..=6
    iotkit_core_storage::init_db_memory(&all).unwrap()
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

fn env_with(id: &str, hw: &str, key: &str, channel: Option<u16>, values: Vec<f64>) -> Envelope {
    Envelope {
        envelope_id: id.into(),
        source: "bravepi-mainboard:/dev/ttyAMA0".into(), // 実在ID形式(handle.rs:109)
        declaration_version: None,
        items: vec![ReadingItem {
            subject_hint: Some(hw.into()),
            measurement_key: key.into(),
            channel_index: channel,
            series_variant: None,
            values,
            device_time_ms: None,
            time_source: TimeSource::Gateway,
            age_ms: None, rssi: None, battery_pct: None,
        }],
    }
}

#[tokio::test]
async fn known_key_in_range_is_durable_and_auto_enables() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let ack = collector
        .submit(env_with("e-1", "ble:aa", "temperature_c", None, vec![21.5]))
        .await.unwrap();
    assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Durable, quarantine_reason: None })));
    let (entries, events, readings): (i64, i64, i64) = db.with_conn_sync(|conn| {
        Ok((
            conn.query_row("SELECT COUNT(*) FROM registry_entries WHERE measurement_key='temperature_c'", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT COUNT(*) FROM readings WHERE quarantined=0", [], |r| r.get(0)).unwrap(),
        ))
    }).unwrap();
    assert_eq!((entries, events, readings), (1, 1, 1),
        "auto-enable+監査イベント+durable行が同一トランザクションで揃う");
}

#[tokio::test]
async fn out_of_range_is_quarantined_row_with_clean_series() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let ack = collector
        .submit(env_with("e-2", "ble:aa", "temperature_c", None, vec![5000.0]))
        .await.unwrap();
    assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Quarantined,
            quarantine_reason: Some(QuarantineReason::OutOfRange),
        })), "検疫理由がワイヤで可視化される(D1追補)");
    let (s_q, r_q): (i64, i64) = db.with_conn_sync(|conn| {
        Ok((
            conn.query_row("SELECT quarantined FROM series", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT quarantined FROM readings", [], |r| r.get(0)).unwrap(),
        ))
    }).unwrap();
    assert_eq!((s_q, r_q), (0, 1));
}

#[tokio::test]
async fn unknown_key_materializes_quarantined_series_with_reason() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let ack = collector
        .submit(env_with("e-3", "ble:aa", "custom.tank_level", None, vec![42.0]))
        .await.unwrap();
    assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Quarantined,
            quarantine_reason: Some(QuarantineReason::UnknownKey),
        })));
    let (key, q, reason): (String, i64, Option<String>) = db.with_conn_sync(|conn| {
        Ok(conn.query_row(
            "SELECT measurement_key, quarantined, quarantine_reason FROM series",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap())
    }).unwrap();
    assert_eq!(key, "custom.tank_level");
    assert_eq!(q, 1);
    assert_eq!(reason.as_deref(), Some("unknown_key"));
}

#[tokio::test]
async fn value_type_mismatch_rejects_item_but_stores_valid_sibling() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let mut e = env_with("e-4", "ble:aa", "temperature_c", None, vec![21.5]);
    e.items.push(ReadingItem {
        subject_hint: Some("ble:aa".into()),
        measurement_key: "contact_state".into(),
        channel_index: None,
        series_variant: None,
        values: vec![3.0], // boolに3.0 → 構造的に解釈不能
        device_time_ms: None,
        time_source: TimeSource::Gateway,
        age_ms: None, rssi: None, battery_pct: None,
    });
    let ack = collector.submit(e).await.unwrap();
    let AckStatus::Accepted { items } = ack.status else { panic!("expected Accepted") };
    assert!(matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable, .. }));
    assert!(matches!(items[1],
        ItemStatus::ItemRejected { reason_code: ReasonCode::ValueTypeMismatch, .. }));
}

#[tokio::test]
async fn undeclared_acceleration_channel_is_quarantined() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let ok = collector
        .submit(env_with("e-5", "ble:aa", "acceleration_mg", Some(2), vec![100.0]))
        .await.unwrap();
    assert!(matches!(ok.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable, .. })));
    let bad = collector
        .submit(env_with("e-6", "ble:aa", "acceleration_mg", Some(3), vec![100.0]))
        .await.unwrap();
    assert!(matches!(bad.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Quarantined,
            quarantine_reason: Some(QuarantineReason::UndeclaredChannel),
        })));
}

#[tokio::test]
async fn single_mode_none_and_zero_channel_share_one_series() {
    // 正準化(評価器のchannel_index)により None / Some(0) が同一seriesへ落ちる
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    collector.submit(env_with("e-c1", "ble:aa", "distance_mm", None, vec![100.0])).await.unwrap();
    collector.submit(env_with("e-c2", "ble:aa", "distance_mm", Some(0), vec![200.0])).await.unwrap();
    let (series, channel, readings): (i64, i32, i64) = db.with_conn_sync(|conn| {
        Ok((
            conn.query_row("SELECT COUNT(*) FROM series", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT channel_index FROM series", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap(),
        ))
    }).unwrap();
    assert_eq!((series, channel, readings), (1, -1, 2), "series分裂しない(正準チャネル=-1)");
}

#[tokio::test]
async fn alias_routes_new_series_to_canonical_key() {
    let db = full_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        let cat = iotkit_core_registry::standard_catalog();
        iotkit_core_registry::enable_entry(
            conn, cat.find("temperature_c").unwrap(), &cat.catalog_version, "test",
        ).unwrap();
        iotkit_core_registry::define_alias(
            conn, "temp_old", "temperature_c", iotkit_core_registry::AliasKind::SiteMapping,
        ).unwrap();
        Ok(())
    }).unwrap();
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    collector
        .submit(env_with("e-7", "ble:aa", "temp_old", None, vec![21.5]))
        .await.unwrap();
    let key: String = db.with_conn_sync(|conn| {
        Ok(conn.query_row("SELECT measurement_key FROM series", [], |r| r.get(0)).unwrap())
    }).unwrap();
    assert_eq!(key, "temperature_c");
}

#[tokio::test]
async fn auto_enable_failure_produces_no_ack_and_retry_recovers_consistently() {
    // auto-enable(registry_entriesへのINSERT)**だけ**をトリガーで失敗させる(query_only方式だと
    // 手前のdedup INSERTで落ちてauto-enable経路を通らない)。ストレージ失敗 → ackなし(D1)。
    // エンベロープ全体がロールバックされるため、dedup予約・entry・監査イベントは何も残らず、
    // トリガー除去後の**同一envelope_id再送**は重複扱いにならず受理され、entryと監査イベントが
    // ちょうど1つずつになる(計画1のキャッシュ全捨て教訓のレジストリ版整合検証)。
    let db = full_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER fail_enable BEFORE INSERT ON registry_entries
             BEGIN SELECT RAISE(ABORT, 'simulated registry failure'); END;",
        )?;
        Ok(())
    }).unwrap();
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let e = env_with("e-8", "ble:aa", "temperature_c", None, vec![21.5]);
    let result = collector.submit(e.clone()).await;
    assert!(matches!(result, Err(iotkit_core_collector::CollectorClosed)),
        "auto-enable失敗はRejectedではなくackなし(D1)");
    let (dedup, entries, events, readings): (i64, i64, i64, i64) = db.with_conn_sync(|conn| {
        conn.execute_batch("DROP TRIGGER fail_enable;")?;
        Ok((
            conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT COUNT(*) FROM registry_entries", [], |r| r.get(0)).unwrap(),
            conn.query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap(),
        ))
    }).unwrap();
    assert_eq!((dedup, entries, events, readings), (0, 0, 0, 0), "エンベロープ全体ロールバック");
    // 同一コレクタ(キャッシュ全捨て済み)への再送 → 受理・整合
    let ack = collector.submit(e).await.expect("retry must be accepted");
    assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable, .. })));
    let (entries2, events2): (i64, i64) = db.with_conn_sync(|conn| {
        Ok((
            conn.query_row("SELECT COUNT(*) FROM registry_entries", [], |r| r.get(0)).unwrap(),
            conn.query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                [], |r| r.get(0)).unwrap(),
        ))
    }).unwrap();
    assert_eq!((entries2, events2), (1, 1), "再送後にentryと監査イベントがちょうど1つずつ");
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p iotkit-core-registry --test e2e_collector`
Expected: FAIL(コンパイルエラー: dev-deps不足)→ dev-deps追加後に全テスト実行で通ることを確認する流れでもよい(統合テストはStep 1の追加自体が実装。落ちたテストがあればSqliteRegistry/コレクタの統合バグ)

- [ ] **Step 3: ゲートウェイ配線**

`iotkit-gateway/Cargo.toml` のdependenciesに追加:

```toml
iotkit-core-registry = { path = "../core/registry" }
```

`iotkit-gateway/src/main.rs` の変更(2箇所):

```rust
    let mut all_migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    all_migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS); // v3, v5
    all_migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS); // v2, v4
    all_migrations.extend_from_slice(iotkit_core_registry::MIGRATIONS); // v6
    all_migrations.sort_by_key(|m| m.version); // 1,2,3,4,5,6
```

```rust
    // Ingest collector: fan-inループのSensorData分岐が経由する耐久点(D1)。
    // 受理判定はD6判別表(SqliteRegistry=現場レジストリ参照、計画2)。
    let (collector, _collector_handle) = iotkit_core_collector::Collector::spawn(
        db.clone(),
        std::sync::Arc::new(iotkit_core_registry::SqliteRegistry),
        256,
    );
```

`iotkit-gateway/src/bridge.rs` の統合テスト `bridge_output_flows_through_collector_to_readings` を更新:

```rust
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
        all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
        all.sort_by_key(|m| m.version);
        let db = iotkit_core_storage::init_db_memory(&all).unwrap();
```

および spawn を:

```rust
        let (collector, _h) = iotkit_core_collector::Collector::spawn(
            db.clone(), std::sync::Arc::new(iotkit_core_registry::SqliteRegistry), 16);
```

(このテストはtemperature_c/21.5なので、SqliteRegistry化により「実配線でauto-enable込みでdurable」を検証するE2Eに格上げされる。)

- [ ] **Step 4: ワークスペース全体テスト**

Run: `cargo test --workspace`
Expected: PASS(全クレート)

- [ ] **Step 5: Commit**

```bash
git add core/registry iotkit-gateway
git commit -m "feat(gateway): wire SqliteRegistry (D6 admission) into the composition root"
```

---

## Self-Review記録(計画作成時)

- 判別表(D6決定6)の6行すべてに対応テストがある: 既知キー値域内(T6 e2e)/既知キー値域外(T5+T6)/未知キー(T5+T6)/宣言外channel(T5+T6)/値型不一致(T5+T6)/文法違反(T4既存テスト維持)。
- 「ストレージ失敗→ackなし」不変則は、トレイトのResult化(T4)+FailingPolicyテスト(T4)+auto-enable失敗E2E(T6・トリガー注入で当該経路を直接通す)の3層で守る。
- エイリアスのseries_key不変規則(D6決定3(a)/(b))はT5の2テストで両分岐を検証。確立時の検疫解除(3(a))はT2の解除関数+T3のdefine_alias+T5のDB状態assertで貫通。
- チャネル正規化定数はledgerに一本化し、正準チャネルは評価器がverdictで返す(T4のverdict配線テスト+T5の正規化テスト+T6のseries非分裂E2E)。
- Wave 1以降に属するもの(ドリフトレポート、custom輸出入、R14操作、手動検疫解除、vibration_spectrum実装)は実装しない。`define_alias`/`seed_legacy_sensor_map` は関数+テストのみで実行系に配線しない。

## レビュー裁定記録(2026-07-03、Fable+codex並行レビュー)

計画初版に対し、Fableレビューエージェントとcodex(gpt-5.5/xhigh、plan-review.md観点)を同時並行で実施。
両者が独立に同一の重大指摘2件に到達した。裁定は全件採用(棄却ゼロ)、本版に反映済み。

| 指摘 | 出典 | 裁定・反映先 |
|---|---|---|
| D6決定3(a)エイリアス確立時の既存series検疫解除が欠落(検疫seriesにdurable行が混在する不整合を作る) | codex BLOCKER + Fable MAJOR | 採用。T2 `release_series_quarantine_for_key` / T3 define_alias解除+`series_quarantine_released`監査 / T5 DB状態assert |
| ack `quarantine_reason`(D1追補「レジストリ実装と同時」)の欠落 | codex MAJOR + Fable MAJOR | 採用。T4 Step 0で契約に4値enum追加(additive)、コレクタが写像(レジストリ理由優先→device_quarantined) |
| singleチャネル `None`/`Some(0)` のseries分裂 | codex MAJOR + Fable MINOR | 採用。verdictに正準 `channel_index` を追加、評価器が正規化(T4配線テスト/T5正規化テスト/T6非分裂E2E) |
| 値域フォールバックの外殻破れ(層丸ごと選択だと片辺のみ上書きで反対辺の限界が消える) | Fable MINOR | 採用。min/max各辺独立フォールバックに変更(T5テスト追加) |
| エイリアスがカタログキーを無音遮蔽(D6決定3「明示解決を要求」のシグナル欠落) | Fable MINOR | 採用。Wave 0はwarnログで可視化(T5) |
| auto-enable失敗テストが実際は手前のdedup INSERTで落ちる+ロールバック後の再送整合が未検証 | codex MINOR + Fable MINOR | 採用。トリガー注入方式に変更し再送整合検証を統合(T6) |
| マイグレーションテスト名の過大表示/テスト数の記載誤り | codex MINOR + Fable MINOR | 採用。記述修正 |

## 最終ブランチレビューの記録(2026-07-03、実装完了後)

実装は全6タスクcodex(gpt-5.5)、タスクレビュー6回Fable全Approved。最終はFableブランチレビュー
(Ready to merge)+codex xhigh最終実装レビューの二重検査。裁定と修正波(3コミット):

| 指摘 | 出典 | 裁定 |
|---|---|---|
| VL53L1Xドライバ2000mmクランプがカタログ物理限界0..4000と矛盾(実測3m→2m改変) | codex[高] | **修正済み**(fix(sensors)): クランプ除去。値域判定はR8の仕事(D6決定8) |
| 非有限値(NaN/Inf)が評価器素通り→insert失敗→ackなし恒久再送ループ | Fable(持ち越しd) | **修正済み**(fix(registry)): 終端拒否value_type_mismatch(D1決定的違反) |
| ゲートウェイがNoAckをコレクタ死亡と混同→プロセス再起動 | codex[中] | **修正済み**(fix(collector,gateway)): SubmitError::NoAck/Closed分離。再送スプールは計画3 |
| エイリアス解除がtargetのchannel_mode未検証(無効チャネルseriesまで解除) | codex[中] | **計画4へ持ち越し**: define_aliasはWave 0実行系から未呼出(R14 CLI=計画4)で露出ゼロ。計画4で解除時channel検証を実装すること |
| 未知キー期にSome(0)実体化されたseriesがエイリアス後に正準化(-1)で分裂 | Fable Minor | 後続。D6 3(a)「履歴を切らない」との軽微な緊張として記録 |
| record型(vibration_spectrum)の拒否時auto-enable | Fable Minor | 意図通りと裁定(キーの有効化=定義コピーは観測の受理と別問題)。対応不要 |

教訓: カタログ/設計値とドライバ実出力の突合が2計画連続で実データ破壊を検出
(plan-review.md Active Watchpointに昇格済み)。
