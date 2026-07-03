# Wave 0 計画4: gatewayctl+R11読み出し面+R17 retention/水位+R22手動スナップショット Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wave 0「動く最小」を完結させる——運用CLI(R7最小)、読み出しAPI(R11)、retention+水位+検疫時限失効(R17/D5)、手動スナップショット(R22)、旧v2テーブル削除、監督ループの閉塞解消。

**Architecture:** 新クレート `iotkit-gatewayctl`(clapベースCLI、gatewayデーモンと同一SQLite DBをWALで共有)+ 既存core群への読み出し/操作APIの追加。**別プロセス変異とデーモンの整合はledger generation counterで取る**(D5決定3の明文要求)。HTTPサーバは作らない(Wave 1)。R12ヘルスはgatewayが周期書き出しするJSONファイル+CLI表示。

**Tech Stack:** Rust / rusqlite / clap(新規) / nix(statvfs、新規)

**改訂履歴:** 初版→Fable+codex並行レビュー(BLOCKER 5/MAJOR 10/MINOR 9、重複統合で約20論点)全採用で改訂2(2026-07-03)。裁定記録は末尾。

## Global Constraints(設計追補の全掃引結果。全タスクの要求に暗黙に含まれる)

- **Wave 0スコープはD3 Waveマーキングに厳密に従う**: CLIは**R7最小**(台帳CLI登録/承認+replace-hardwareガードレールCLI版)であり、R14操作カタログ(dry-run・権限段階)は**Wave 1——作らない**。値域変更(series set-range)もR14操作でありWave 0では作らない。R11=範囲クエリ+範囲集計+CSV+メタデータ面。R12=ヘルスJSON最小。R17=retention+水位。R22=手動export+復元(R22最小契約)。
- **台帳変異の整合(D5決定3)**: 「台帳の変異は必ずコレクタ経由(またはgeneration counter共有)」。gatewayctlは別プロセスでDBを直接変異するため、**generation counter共有**を採用する: `ledger_meta` の `generation`(整数)を全変異Txでインクリメントし、コレクタはエンベロープ処理の冒頭で1読取して前回値と不一致なら`ResolutionCache`を全捨てする。**この機構なしにCLI変異コマンドを実装してはならない**(activate/retire/replace/aliasの全て)。
- **R11の範囲クエリはevent_time基準を正とする**(D7決定9)。event_timeの導出規則(D7決定3、**3段**):
  1. デバイス申告時刻(time_source=device_ntp/device_rtc)→ `event_time_source='device'`
  2. **age_ms復元時刻**(received_at−age_ms、time_source=gateway_adjusted)→ `'gateway_adjusted'`
  3. どちらもなければ received_at → `'received_at'`
  未来方向のみ妥当窓検査(許容ズレ既定=**300_000ms**。超過は3へ降格)。過去方向の窓は存在しない。
  **現状collectorは `item.age_ms` を捨てている**——本計画で復元を実装する(候補2の実体化)。
- **時間軸の使い分け**: 表示・集計=event_time / retention・汚染区間マーキング=**received_at**(挿入時系)。「いつ書き込まれたか」を問う操作にデバイス申告由来のevent_timeを使ってはならない(バックログ遅着ですり抜ける)。
- **D6監査追記(計画4宛2件)**:
  1. **エイリアス確立時の検疫解除はチャネル適合seriesに限る**。不適合seriesは検疫維持+`quarantine_reason`を`undeclared_channel`へ更新。
  2. **実体化seriesのチャネル不変規則(本計画で確定)**: series_keyは**チャネル含め不変**。single型のNone/Some(0)観測の解決順: (key,-1)実体化済み→-1 / なければ(key,0)実体化済み→0 / どちらもなければ-1(新規は正準形)。**-1と0が併存する場合は正準(-1)を優先**し、初回検出時に監査イベント`"channel_form_conflict"`を記録する。
- **gatewayctlの変異Txは `Transaction::new_unchecked(conn, TransactionBehavior::Immediate)` で開始する**(DbHandleは`&Connection`しか渡さないため`conn.transaction()`は使えない。既定DEFERREDはWAL下の並行デーモン書き込みとread→write昇格で衝突し`busy_timeout`で救済されない)。
- **gatewayctlはDBファイルが存在しなければエラー**(無音の空DB生成禁止)。生成が正当なのは`snapshot restore --create`のみ。
- **D5 replace-hardwareガードレール(Wave 0=CLI版)**: 観測プロファイル突合は**完全一致を既定**とし、あらゆる不一致(不足・過剰とも)はブロック(`--force`のみ上書き)。プロファイルは**staged_readingsと、新hardware_idがaliveなdeviceに解決される場合はそのseries集合の両ソース**から。確認プロンプトにuser_label+直近測定値/undo=汚染区間のreadings検疫マーキング(**received_at基準**)/交換確定時: 旧候補はretire+`superseded_by`、全seriesに較正要再確認、dedup台帳は触らない。
- **検疫の時限自動失効(D5 Wave 0登録経路)**: 「検疫遷移は時限自動失効+CLI解除のみ」。デバイス検疫はTTL(設定キー、既定7日)で自動activate+監査イベント。retentionタスクに同居。
- **スキーマ回収**: `series.calibration_review` 列(D3「初日から存在」に反して欠落)を本計画で追加。
- **R22最小契約(D2 §3.5)**: manifest+`format_version`+`secrets`予約セクションを初版から。**Wave 0はsecrets空=平文JSONで可**(D2 R22最小契約3の明文。暗号化コンテナはフォーマット仕様に予約記述のみ——実装はsecrets実体化=Wave 1と同時)。readings本体は対象外。**復元時は必ず新エポック採番**+監査イベント。
- **マイグレーションは集合差ベース**。欠番(v2撤去後の2)許容をテストで固定。
- **既存ファイルへのrustfmt一括整形禁止**。機能diffのみ。
- テスト実行: `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test -p <crate>`。新依存(clap/nix)は**親がcargo fetch済み**の前提。
- コミット規約: `feat(crate):`等+`Implemented-By: codex (gpt-5.5)`+Co-Authored-By行。

## File Structure(新規/主変更)

```
iotkit-gatewayctl/                    # 新クレート(バイナリ)。workspace membersに追加
  Cargo.toml                          # clap, core群, iotkit-ingest-contract(T5), rusqlite, serde_json
  src/main.rs
  src/cmd/devices.rs                  # sightings list / device list|add|approve|activate|retire
  src/cmd/replace.rs                  # replace-hardware+undo
  src/cmd/registry.rs                 # registry list|enable|alias / series list
  src/cmd/query.rs                    # readings query|aggregate|export / events tail / health
  src/cmd/snapshot.rs                 # snapshot export|restore
core/timeseries/migrations/0007_drop_sensor_readings.sql   # T1
core/timeseries/migrations/0008_event_time.sql             # T2
core/ledger/migrations/0009_calibration_review.sql         # T2
core/timeseries/src/lib.rs / src/query.rs(新規)           # T1/T2/T3
core/collector/src/actor.rs           # T2(age_ms復元)+T4(generationチェック)
core/ledger/src/store.rs              # T3一覧+T4 retire/generation+T5 replace
core/registry/src/{store.rs,policy.rs}                     # T6
iotkit-gateway/src/{retention.rs,health.rs,main.rs,config.rs}  # T7/T9
```

**Interfaces全体図**: gatewayctlはcoreクレート群に直接依存(デーモン非依存)。整合はgeneration counter(上記)。ヘルスJSONはデーモン→ファイル→CLIの一方向。

---

### Task 1: 旧sensor_readings(v2)の撤去

**Files:**
- Create: `core/timeseries/migrations/0007_drop_sensor_readings.sql`
- Modify: `core/timeseries/src/lib.rs`(MIGRATIONS配列、v2 API 4関数=行124-332、v2側mod tests=行334-667の20ケース)
- Modify: `core/timeseries/src/model.rs`(`ReadingRow`と`TimeRange`を削除——T3のAPIは全てi64 msでTimeRangeを使わない。model.rsが空になるならファイルごと削除しlib.rsのmod宣言を除去)
- Modify: `iotkit-gateway/src/main.rs:50-53`(コメント更新: `// v2, v4`→`// v4, v7`、ソート結果コメント`// 1,2,3,4,5,6`→`// 1,3,4,5,6,7`)
- Test: `core/storage/src/migrate.rs`集合差テスト+timeseries DROPテスト

**Interfaces:**
- Consumes: なし(v2 APIの本番呼び出しはゼロ=調査済み事実)
- Produces: timeseries MIGRATIONS = v4, v7。v3系API不変

- [ ] **Step 1: 失敗するテストを2本書く**(v3側テストモジュール。DBヘルパの実名は `v3_db()`——v2側の`test_db()`は削除対象)

```rust
#[test]
fn migration_v7_drops_sensor_readings_from_legacy_db() {
    // 「v2適用済み旧DB」を合成: 生SQLでsensor_readingsを作り、_schema_version テーブル
    // (migrate.rsの実名。schema_migrationsではない)にversion=2を記録した一時DBを用意
    // → 全MIGRATIONS(v7含む)適用 → テーブルが消える
    // (DROP経路の実動作を検証する——新規DBはv2自体を適用しないためこの経路が唯一のDROP実行)
}
#[test]
fn migration_set_difference_tolerates_gap() {
    // 新規DBに{1,3,4,5,6,7}適用: 2欠番でも完全適用され、readingsは無傷
}
```

- [ ] **Step 2: 0007マイグレーションを書く**

```sql
-- 計画4 T1: 旧v2テーブル撤去。本番呼び出しゼロを確認済み(2026-07-03調査)。
-- 既存DB(開発機)にはテーブルが存在し、新規DBではv2マイグレーション自体を撤去するため
-- IF EXISTSで両対応する。
DROP TABLE IF EXISTS sensor_readings;
```

MIGRATIONS配列: v2エントリを削除し、v7(`label: "drop_sensor_readings"`)を追加。

- [ ] **Step 3: v2 API 4関数・ReadingRow・TimeRange・v2テスト20件を削除**(不要になったuse——`iotkit_core_types::{AdapterId, DeviceKey, SensorType}`等——も除去)

- [ ] **Step 4: テスト実行→コミット**

Run: `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test -p iotkit-core-timeseries -p iotkit-core-storage && cargo build -p iotkit-gateway`
Expected: 全緑+ビルド成功(参照残りの検出)。
`git commit -m "feat(timeseries): drop legacy sensor_readings (v2) table and API"`

---

### Task 2: スキーマ改訂——event_time実体化(3段導出)+calibration_review回収

**Files:**
- Create: `core/timeseries/migrations/0008_event_time.sql`
- Create: `core/ledger/migrations/0009_calibration_review.sql`
- Modify: `core/timeseries/src/lib.rs`(`insert_reading_v3`に導出追加。`NewReading`は署名不変)
- Modify: `core/collector/src/actor.rs`(**age_ms復元**: NewReading構築箇所=行272付近)
- Modify: `core/ledger/src/store.rs`(MIGRATIONS配列にv9追加)
- Test: timeseries/ledger/collectorテスト

**Interfaces:**
- Consumes: `ReadingItem.age_ms`(envelope.rs:31——現状collectorが捨てている)
- Produces: `readings.event_time INTEGER NOT NULL`/`readings.event_time_source TEXT NOT NULL`(**'device'|'gateway_adjusted'|'received_at'**)/`idx_readings_series_event_time`/`series.calibration_review INTEGER NOT NULL DEFAULT 0`/`pub const FUTURE_TOLERANCE_MS: i64 = 300_000;`

- [ ] **Step 1: 失敗するテストを書く**(導出4ケース: device採用/gateway_adjusted採用/未来降格/フォールバック)

```rust
#[test] fn event_time_prefers_device_time_within_tolerance() { /* dt=ra-3h → 'device' */ }
#[test] fn event_time_uses_age_ms_restoration() {
    // device_time=None, age_ms=5000 → collector側でdevice_time_ms=ra-5000,
    // time_source="gateway_adjusted" に復元され、event_time=ra-5000, source='gateway_adjusted'
}
#[test] fn event_time_demotes_future_device_time() { /* dt=ra+10min → 'received_at' */ }
#[test] fn event_time_falls_back_to_received_at() { /* dt=None,age=None → 'received_at' */ }
```

- [ ] **Step 2: マイグレーションを書く**

`0008_event_time.sql`(**バックフィルにも同一の未来降格規則を適用する**——無条件COALESCEは時計狂い行の未来event_timeを固定化する):

```sql
-- 計画4 T2(D7決定3/9): R11範囲クエリの正準時間軸。バックフィルは導出規則と同一
-- (未来方向許容ズレ300_000ms超のdevice_timeはreceived_atへ降格。過去方向に窓はない)。
ALTER TABLE readings ADD COLUMN event_time INTEGER NOT NULL DEFAULT 0;
ALTER TABLE readings ADD COLUMN event_time_source TEXT NOT NULL DEFAULT 'received_at';
UPDATE readings SET
    event_time = CASE
        WHEN device_time IS NOT NULL AND device_time <= received_at + 300000 THEN device_time
        ELSE received_at END,
    event_time_source = CASE
        WHEN device_time IS NOT NULL AND device_time <= received_at + 300000
            THEN (CASE WHEN time_source = 'gateway_adjusted' THEN 'gateway_adjusted' ELSE 'device' END)
        ELSE 'received_at' END;
CREATE INDEX idx_readings_series_event_time ON readings(series_id, event_time);
```

`0009_calibration_review.sql`:

```sql
-- 計画4 T2(D3「較正要再確認状態の列は初日から」の回収。D5決定2/replace-hardware動線)
ALTER TABLE series ADD COLUMN calibration_review INTEGER NOT NULL DEFAULT 0;
```

- [ ] **Step 3: collectorのage_ms復元を実装**(actor.rsのNewReading構築点)

```rust
// D1: RTCなしデバイスのage_ms → received_at - age_ms で復元(time_source=gateway_adjusted)。
// item.device_time_msが既にあればそれが優先(申告時刻>復元時刻)。
let (device_time_ms, time_source) = match (item.device_time_ms, item.age_ms) {
    (Some(dt), _) => (Some(dt), item.time_source.clone()),        // 申告どおり透過
    (None, Some(age)) => (Some(received_at - age as i64), "gateway_adjusted".to_string()),
    (None, None) => (None, item.time_source.clone()),
};
```

(実物のNewReading構築コードの形に合わせて適用。`item.time_source`のフィールド名・型は実装時にenvelope.rsで確認。)

- [ ] **Step 4: insert_reading_v3の導出実装**

```rust
pub const FUTURE_TOLERANCE_MS: i64 = 300_000; // D7決定3: 未来方向許容ズレ既定5分

/// D7決定3: event_time導出。time_sourceが'gateway_adjusted'なら復元由来と表示する。
fn derive_event_time(received_at_ms: i64, device_time_ms: Option<i64>, time_source: &str)
    -> (i64, &'static str) {
    match device_time_ms {
        Some(dt) if dt <= received_at_ms + FUTURE_TOLERANCE_MS => {
            if time_source == "gateway_adjusted" { (dt, "gateway_adjusted") } else { (dt, "device") }
        }
        _ => (received_at_ms, "received_at"),
    }
}
```

- [ ] **Step 5: テスト→コミット**

Run: `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test -p iotkit-core-timeseries -p iotkit-core-ledger -p iotkit-core-collector`
`git commit -m "feat(timeseries,collector,ledger): materialize event_time with 3-stage derivation (D7) and recover calibration_review"`

---

### Task 3: R11読み出し層——クエリ/集計/CSV/一覧API

**Files:**
- Create: `core/timeseries/src/query.rs`
- Modify: `core/timeseries/src/lib.rs`(`pub mod query;`)
- Modify: `core/ledger/src/store.rs`・`core/registry/src/store.rs`(一覧API)
- Test: 各クレート

**Interfaces:**
- Consumes: readings v3(T2後)、series/devices、registry
- Produces(T4/T5/T7がこの署名を使う。**T8スナップショットはこれらを使わない**——表示用の部分列であり全列復元に不適。T8は独自の全列ダンプを持つ):

```rust
// core/timeseries/src/query.rs
pub struct ReadingRowV3 {
    pub seq: i64, pub series_id: i64,
    pub event_time: i64, pub event_time_source: String,
    pub received_at: i64, pub device_time: Option<i64>,
    pub time_source: String, pub time_quality: String,
    pub values: Vec<f64>, pub rssi: Option<i16>, pub battery_pct: Option<u8>,
    pub quarantined: bool,
}
pub fn query_readings_v3(conn, series_id: i64, from_event_ms: i64, to_event_ms: i64,
    limit: u32, include_quarantined: bool) -> Result<Vec<ReadingRowV3>, TimeseriesError>;
pub struct Bucket { pub bucket_start: i64, pub count: i64, pub min: f64, pub max: f64, pub avg: f64 }
pub fn aggregate_readings_v3(conn, series_id: i64, from_event_ms: i64, to_event_ms: i64,
    bucket_ms: i64, include_quarantined: bool) -> Result<Vec<Bucket>, TimeseriesError>;
/// CSVヘッダ: seq,event_time,event_time_source,received_at,device_time,time_source,
/// time_quality,quarantined,rssi,battery_pct,v0..vN(D7決定3: 出自フィールド併載——device_time含む)
pub fn export_csv<W: std::io::Write>(w: &mut W, rows: &[ReadingRowV3]) -> std::io::Result<()>;
pub fn latest_by_series(conn, series_id: i64) -> Result<Option<ReadingRowV3>, TimeseriesError>;
pub fn list_staged_for_hardware(conn, hardware_id: &str, limit: u32)
    -> Result<Vec<(i64, String)>, TimeseriesError>;  // (received_at, payload_json)。T5が使う

// core/ledger/src/store.rs 追加分
pub fn list_devices(conn, include_retired: bool) -> Result<Vec<DeviceRow>, LedgerError>;
pub fn get_device(conn, system_id: &SystemId) -> Result<Option<DeviceRow>, LedgerError>;
pub struct SeriesRow { pub series_id: i64, pub system_id: SystemId,
    pub measurement_key: String, pub channel_index: i32, pub variant: String,
    pub quarantined: bool, pub quarantine_reason: Option<String>,
    pub value_semantics: String, pub unit: Option<String>,
    pub range_min: Option<f64>, pub range_max: Option<f64>, pub calibration_review: bool }
pub fn list_series_for_device(conn, system_id: &SystemId) -> Result<Vec<SeriesRow>, LedgerError>;
pub struct SightingRow { pub hardware_id: String, pub source: String,
    pub first_seen: i64, pub last_seen: i64, pub observations: i64 }
pub fn list_sightings(conn) -> Result<Vec<SightingRow>, LedgerError>;
pub struct EventRow { pub event_id: i64, pub at: i64, pub kind: String,
    pub system_id: Option<SystemId>, pub detail: String }
pub fn list_recent_events(conn, limit: u32) -> Result<Vec<EventRow>, LedgerError>;

// core/registry/src/store.rs 追加分
pub fn list_entries(conn) -> Result<Vec<EntryRow>, RegistryError>;
pub struct AliasRow { pub alias: String, pub measurement_key: String, pub alias_kind: String }
pub fn list_aliases(conn) -> Result<Vec<AliasRow>, RegistryError>;
```

- [ ] **Step 1: 失敗するテストを書く**(範囲・順序・limit・quarantined切替/バケット境界・空バケットなし・多値Err/CSVヘッダ列にdevice_time/latestタイブレーク=event_time最大→seq最大/各一覧1件往復)

- [ ] **Step 2: 実装**(`WHERE series_id=? AND event_time>=? AND event_time<? ORDER BY event_time ASC, seq ASC LIMIT ?`。集計はSQL `GROUP BY (event_time - ?from) / ?bucket`。CSVは手書きwriter——値は数値のみでエスケープ不要)

- [ ] **Step 3: テスト→コミット**

Run: `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test -p iotkit-core-timeseries -p iotkit-core-ledger -p iotkit-core-registry`
`git commit -m "feat(timeseries,ledger,registry): R11 read layer (event_time queries, aggregation, CSV, listings)"`

---

### Task 4: gatewayctlクレート+generation counter+device系コマンド

**Files:**
- Create: `iotkit-gatewayctl/Cargo.toml`(clap derive, core群, rusqlite, serde_json)、`src/main.rs`、`src/cmd/devices.rs`、`src/cmd/query.rs`
- Modify: ルート`Cargo.toml`(members追加)
- Modify: `core/ledger/src/store.rs`(`retire_device`/`bump_generation`/`current_generation`)
- Modify: `core/collector/src/actor.rs`(**generationチェック**)
- Test: ledger/collector/gatewayctl統合

**Interfaces:**
- Consumes: T3一覧API、既存approve_sighting/activate_device/insert_device
- Produces:

```rust
// core/ledger
/// retire(墓標): 行は消さない。system_id再利用は永久禁止(D5決定4)。
pub fn retire_device(conn, system_id: &SystemId) -> Result<(), LedgerError>;
// UPDATE devices SET state='retired', retired_at=now WHERE system_id=? AND state!='retired'
// + record_event("device_retired")。0件更新はNotFound。
/// D5決定3 generation counter共有: 台帳変異の世代番号。CLI変異Txの最終手順で必ず呼ぶ。
pub fn bump_generation(conn) -> Result<i64, LedgerError>;   // ledger_meta 'generation' +1
pub fn current_generation(conn) -> Result<i64, LedgerError>; // 行なし=0
```

collector側(actor.rs): `ResolutionCache` に `generation: i64` を持たせ、**エンベロープ処理Txの冒頭**で `current_generation` を読み、不一致ならキャッシュ全クリア+保持世代更新(1エンベロープ1回のPK読取=無害)。
**併せてcollectorのTx開始(actor.rs:171の`conn.unchecked_transaction()`)を
`Transaction::new_unchecked(conn, TransactionBehavior::Immediate)`に変更する**——冒頭にgeneration
readを足すとDEFERREDはread→write昇格になり、CLI側で避けたWAL BUSY/SNAPSHOT衝突をcollector側に
作ってしまう。collector Txは元々最初の実操作がwrite(try_claim_envelope)なのでImmediateが意味的にも正しい。

CLI(clap derive):

```
gatewayctl --db <path>   # 既定: $IOTKIT_DB_PATH。ファイルが存在しなければエラー
                          # (無音の空DB生成禁止。生成はsnapshot restore --createのみ)
  sightings list
  device list [--all]
  device add --hardware-id <hw> --label <l> [--kind positional|individual] [--active]
  device approve <hardware_id> [--label <l>] [--kind ...]
  device activate <system_id_text>
  device retire <system_id_text> [--yes]
  events tail [--limit N]
  readings query <series_id> --from <unix_ms> --to <unix_ms> [--limit N] [--quarantined]
  readings aggregate <series_id> --from --to --bucket <ms>
  readings export <series_id> --from --to --out <path.csv>
```

**変異コマンド(add/approve/activate/retire)の共通形**: `Transaction::new_unchecked(conn, TransactionBehavior::Immediate)` でTx開始→操作→`bump_generation`→commit。

- [ ] **Step 1: 親がclapをcargo fetch。クレート雛形+members登録**
- [ ] **Step 2: retire/bump_generation/current_generationの失敗テスト→実装**(既存テストのraw SQL retire模擬=store.rs:697付近を新APIに置換)
- [ ] **Step 3: collectorのgenerationチェック失敗テスト→実装**

```rust
#[test]
fn resolution_cache_invalidated_on_generation_bump() {
    // device activate相当をCLI経路(別接続)で実行+bump → 次エンベロープで
    // キャッシュが捨てられ、新stateで受理される(検疫刻印が残らない)
}
```

- [ ] **Step 4: main.rs骨格+各コマンド実装**(コマンドは`cmd::xxx::run_yyy(&conn, args)`公開関数に委譲。確認系は`--yes`で非対話化)。マイグレーション連結はgatewayと同一4行(循環依存で共通化不能)——**一致を統合テストで固定**(適用後version集合={1,3,4,5,6,7,8,9})
- [ ] **Step 5: 統合テスト**(tempdir DB: seed→approve→activate→list→retire往復+存在しないDBパスでエラー)
- [ ] **Step 6: テスト→コミット**

Run: `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test -p iotkit-gatewayctl -p iotkit-core-ledger -p iotkit-core-collector`
`git commit -m "feat(gatewayctl,ledger,collector): CLI crate with device lifecycle and generation-counter cache invalidation"`

---

### Task 5: replace-hardware CLI+ガードレール+undo

**Files:**
- Create: `iotkit-gatewayctl/src/cmd/replace.rs`
- Modify: `iotkit-gatewayctl/src/main.rs`(subcommand配線——T6/T8も同様に自タスクのコマンドを配線する)
- Modify: `iotkit-gatewayctl/Cargo.toml`(**iotkit-ingest-contract追加**——ReadingItemのデシリアライズに必要)
- Modify: `core/ledger/src/store.rs`(`replace_hardware`/`set_calibration_review`)
- Modify: `core/timeseries/src/query.rs`(`mark_readings_quarantined`)
- Test: ledger+gatewayctl統合

**Interfaces:**
- Consumes: `list_series_for_device`/`latest_by_series`/`list_staged_for_hardware`(T3)、`find_alive_by_hardware_id`(既存)
- Produces:

```rust
// core/ledger
pub struct ReplaceOutcome { pub replaced: SystemId, pub old_hardware_id: String,
    pub retired_candidates: Vec<SystemId> }
pub fn replace_hardware(conn, system_id: &SystemId, new_hardware_id: &str)
    -> Result<ReplaceOutcome, LedgerError>;
// Tx内: hardware_id張替え+新hw名義のalive候補エントリをretire+superseded_by=対象
// +当該deviceの全seriesにcalibration_review=1+record_event("hardware_replaced",
//  detail: {old_hw, new_hw, at})。※Txとbump_generationは呼び出し側(CLI層)が張る
pub fn set_calibration_review(conn, system_id: &SystemId, flag: bool) -> Result<usize, LedgerError>;

// core/timeseries::query
/// 汚染区間マーキング(D5ガードレール3)。軸は**received_at**(挿入時系)——
/// 「replace〜undo間に書き込まれた行」が対象。event_timeを使うとバックログ遅着行が
/// すり抜ける(信頼判定の軸を信頼できないデバイスの申告時刻に置かない)。
pub fn mark_readings_quarantined(conn, series_ids: &[i64],
    from_received_ms: i64, to_received_ms: i64) -> Result<u64, TimeseriesError>;
```

CLI:

```
gatewayctl device replace <system_id_text> --new-hardware-id <hw> [--force] [--yes]
gatewayctl device replace-undo <system_id_text> --old-hardware-id <hw> [--since <unix_ms>]
    # --since省略時は直近の"hardware_replaced"監査イベントのat(detailのnew_hwが一致するもの)
    # から自動導出。マーキングはreceived_at基準で[since, now]
```

- [ ] **Step 1: 観測プロファイル突合の失敗テスト**

プロファイル導出(新hardware_id側)は**2ソース**: ①staged_readings——payload_jsonは`iotkit_ingest_contract::ReadingItem`のserde直列化(`core/collector/src/actor.rs:221`で確定)。デシリアライズして`measurement_key×channel_index`集合を作る(失敗行はスキップ+警告)。②新hardware_idが**aliveなdeviceに解決される場合**(承認済み検疫デバイス等)、そのdeviceのseries集合(`list_series_for_device`)。①②の和集合をプロファイルとする。
突合: 対象deviceの既存series集合(key×channel)と**完全一致のみ通す**(不足も過剰もブロック=D5「不一致は既定ブロック」。部分集合許容は「測定が欠けた別個体」を通す)。両ソースとも空なら突合不能ブロック。いずれのブロックも`--force`のみ上書き。

- [ ] **Step 2: replace_hardware実装**(CLI層でImmediate Tx: replace→bump_generation→commit)
- [ ] **Step 3: 確認プロンプト**(user_label・hw旧→新・series数・直近測定値最大5件+`type 'replace' to confirm`。`--yes`でスキップ)
- [ ] **Step 4: undo実装**(Immediate Tx内: hardware_id復元(alive一意性を自前確認)+`mark_readings_quarantined`(received_at基準、区間=sinceから現在)+calibration_reviewは立てたまま+`record_event("hardware_replace_undone", detail: {range, rows})`+bump_generation)
- [ ] **Step 5: テスト→コミット**

Run: `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test -p iotkit-gatewayctl -p iotkit-core-ledger -p iotkit-core-timeseries`
`git commit -m "feat(gatewayctl,ledger): replace-hardware with guardrails and undo (D5 decision 4, Wave 0 CLI)"`

---

### Task 6: registry系CLI+チャネル適合解除+チャネル不変ルーティング

**Files:**
- Create: `iotkit-gatewayctl/src/cmd/registry.rs`
- Modify: `core/ledger/src/store.rs`(`release_series_quarantine_for_key`→checked版へ置換。呼び出し元はregistry::define_aliasのみ=調査済み)
- Modify: `core/registry/src/store.rs`(define_alias改修)・`core/registry/src/policy.rs`(ルーティング)
- Test: registry policy/store+e2e

**Interfaces:**

```rust
// core/ledger(置換)
/// D6監査追記: チャネル適合seriesのみ解除。不適合はquarantine_reasonを
/// 'undeclared_channel'へ更新して検疫維持。戻り値=(解除id群, 不適合更新id群)。
pub fn release_series_quarantine_for_key_checked(
    conn, measurement_key: &str, reason: &str, channel_ok: &dyn Fn(i32) -> bool,
) -> Result<(Vec<i64>, Vec<i64>), LedgerError>;
```

define_alias側の判定クロージャ: Single: `ch == CHANNEL_NA || ch == 0` / Fixed: `0 <= ch && (ch as usize) < roles.len()` / Generic: true。不適合があれば監査detailに`channel_mismatch_ids`。

policy.rs single modeルーティング(Global Constraintsの規則): `None|Some(0)`のとき `(key,-1)`実体化済み→-1 / なければ`(key,0)`実体化済み→0 / どちらもなし→-1。**併存時は-1優先+`"channel_form_conflict"`監査(同一(system,key)につき初回のみ——既出判定はledger_events検索でなくWave 0は毎回記録でも可、ノイズが実測で問題なら絞る。判断を実装時に固定しreportに記す)**。

CLI:

```
gatewayctl registry list [--aliases]
gatewayctl registry enable <measurement_key>        # Immediate Tx+bump_generation
gatewayctl registry alias <alias> <canonical_key>   # 同上
gatewayctl series list <system_id_text>
```

(**series set-rangeは作らない**——値域変更はdry-run付きR14操作=Wave 1。D6のWave節どおり。)

- [ ] **Step 1: ルーティングの失敗テスト3本**(0のみ実体化→0へ/未実体化→-1/**-1と0併存→-1優先+監査**)。既存テスト`single_mode_normalizes_some_zero_to_channel_na`は「実体化なし」前提でそのまま通ること(通らなければ前提崩れ=要精読)
- [ ] **Step 2: policy.rs実装**(policy.rs:122-126の正準化を上記規則に置換。`find_series_meta`で存在照合)
- [ ] **Step 3: checked版実装+define_alias改修+CLI実装**
- [ ] **Step 4: テスト→コミット**

Run: `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test -p iotkit-core-registry -p iotkit-core-ledger -p iotkit-gatewayctl`
`git commit -m "feat(registry,ledger,gatewayctl): channel-aware quarantine release and invariant channel routing (D6 audit notes)"`

---

### Task 7: R17 retention+水位+検疫時限失効+R12ヘルスJSON

**Files:**
- Create: `iotkit-gateway/src/retention.rs`、`iotkit-gateway/src/health.rs`
- Modify: `iotkit-gateway/src/main.rs`(タスク起動配線)、`iotkit-gateway/src/config.rs`(設定キー)
- Modify: `core/timeseries/src/query.rs`(`purge_readings_before`)・`core/ledger/src/store.rs`(`expire_quarantined_devices`)
- Modify: `iotkit-gatewayctl/src/cmd/query.rs`(`health`コマンド)
- Test: gateway/timeseries/ledger

**Interfaces:**
- config追加キー: `retention_days`(既定90、最小7クランプ)/`quarantine_ttl_days`(既定7)/`health_json_path`(既定`<db_pathの親dir>/health.json`)/`disk_high_watermark_pct`(既定90)
- Produces:

```rust
// core/timeseries::query
/// retention purge。軸はreceived_at(保存寿命=挿入時系。event_timeは表示軸)。
/// LIMIT付きバッチ削除ループ(1回=10_000行)——90日分の初回パージが単一巨大Txだと
/// 同一DbHandle上のコレクタackを長時間塞ぐ。戻り値=総削除行数。
pub fn purge_readings_before(conn, cutoff_received_ms: i64) -> Result<u64, TimeseriesError>;
// core/ledger
/// D5 Wave 0登録経路「検疫遷移は時限自動失効+CLI解除のみ」: quarantined状態で
/// created_atがTTL超のdeviceをactiveへ+record_event("quarantine_expired")。
pub fn expire_quarantined_devices(conn, ttl_ms: i64) -> Result<Vec<SystemId>, LedgerError>;
```

health.jsonスキーマ(R12最小): T7初版計画と同一(schema/written_at/epoch/uptime_s/collector_alive/adapters[]/db{size_bytes,disk_available_bytes,watermark_exceeded}/retention{days,last_purge_at,last_purged_rows})。

- [ ] **Step 1: purge/expireの失敗テスト→実装**(境界厳密・バッチループ・expire後のstate+イベント)
- [ ] **Step 2: retentionタスク**(起動時+24h間隔: readings purge+dedup TTL+**expire_quarantined_devicesとbump_generationを同一のImmediate Txで実行**(失効はdeviceのstate変異——別Txに分けるとexpire成功後bump失敗、または間にコレクタが挟まって自プロセスのキャッシュがTTL失効後も旧stateを使う窓ができる)+監査イベント`"retention_purge"`)。水位起因の緊急パージ(D2 4クラス②以降)は**Wave 1**——本タスクは観測のみ
- [ ] **Step 3: 水位観測**(nix statvfs+DBファイル(+`-wal`)サイズ。超過は監査イベント`"disk_watermark_exceeded"`を回復までラッチ+health反映)
- [ ] **Step 4: health.rs**(60s間隔、temp書き→rename。`Arc<Mutex<HealthState>>`をfan-inループと共有)
- [ ] **Step 5: `gatewayctl health [--path <p>]`**(既定パスは`--db`の親dir/health.json——デーモンconfig既定と同じ導出。written_atが5分超過去なら`STALE (daemon down?)`表示)
- [ ] **Step 6: テスト→コミット**

Run: `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test -p iotkit-gateway -p iotkit-core-timeseries -p iotkit-core-ledger -p iotkit-gatewayctl`
`git commit -m "feat(gateway): R17 retention+watermark, quarantine TTL expiry, and R12 minimal health JSON"`

---

### Task 8: R22手動スナップショット(export/restore、平文JSON)

**Files:**
- Create: `iotkit-gatewayctl/src/cmd/snapshot.rs`
- Modify: `core/ledger/src/store.rs`(`renew_epoch`)
- Test: gatewayctl統合(往復一致)

**Interfaces:**

```rust
// core/ledger
/// R22最小契約4: 復元=新世代。epochを新UUIDv7で置換(行が無ければ挿入——ledger_epochは
/// 遅延生成のためfresh DBには行が無い。この場合old_epoch=Noneでイベントに記録)。
pub fn renew_epoch(conn) -> Result<String, LedgerError>;
// record_event("epoch_renewed", detail: {"old_epoch": <string|null>})
```

スナップショット形式(**平文JSON**——D2 R22最小契約3: Wave 0はsecrets空=平文で可。暗号化コンテナ形式はsecrets実体化=Wave 1と同時に導入し、そのときmagic+`format_version`で判別する旨をmanifest仕様コメントに予約記述):

```json
{ "manifest": { "format_version": 1, "created_at": 0, "epoch": "…",
    "sections": ["devices","series","registry_entries","registry_aliases",
                 "legacy_sensor_type_map"] },
  "devices": [ … ], "series": [ … ], "registry_entries": [ … ],
  "registry_aliases": [ … ], "legacy_sensor_type_map": [ … ],
  "secrets": null, "calibration": null, "desired_config": null }
```

**ダンプはT3の一覧APIを使わない**(表示用の部分列——created_at/retired_at/superseded_by等が落ちる)。各テーブルを `SELECT * FROM <table>` し、`Statement::column_names`でカラム名→JSONオブジェクト配列に**全列**変換する共通ヘルパを書く。**BLOB↔UUID文字列の変換表を固定する**(この表以外の列は型そのまま):

| テーブル | BLOB列(export時UUID文字列化 / restore時16byte BLOBへ逆変換) |
|---|---|
| devices | system_id, parent_system_id(NULL可), superseded_by(NULL可) |
| series | system_id |

逆変換は`SystemId`のtext→bytes経路を使い、**restore後にFK整合(parent/superseded_byが全てdevicesに解決)とBLOB型(`typeof(system_id)='blob'`)を検証するテスト**を往復テストに含める(TEXTのまま入るとSystemId readerが落ちる/親子FKが壊れる)。restoreは逆変換で明示INSERT(series_idも元の値を保持——AUTOINCREMENTは明示INSERTでsqlite_sequenceが自動追従するため追加操作不要。単調性検証のSELECTをテストに入れる)。

CLI:

```
gatewayctl snapshot export <out_path>
gatewayctl snapshot restore <in_path> --db <path> [--create] [--yes]
    # 非空DB(devices>0)への復元は拒否(Wave 0: 復元は新しい箱の空DB専用。マージ復元はWave 1)
```

- [ ] **Step 1: 往復テスト**(seed(retire済み・superseded_by付き・alias含む)→export→manifest検証→fresh DBへrestore→**全列一致**(created_at/retired_at/superseded_by含む)+epochが変わっている+"epoch_renewed"イベント+series単調性)
- [ ] **Step 2: 全列ダンプ/復元ヘルパ実装**(**exportは全テーブル読み出しを単一読み取りTxで包む**——稼働デーモン並行時の断面一貫性)
- [ ] **Step 3: export/restore実装**(restoreは1 Immediate Tx: 全セクションINSERT→renew_epoch→bump_generation)
- [ ] **Step 4: テスト→コミット**

Run: `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test -p iotkit-gatewayctl -p iotkit-core-ledger`
`git commit -m "feat(gatewayctl,ledger): R22 manual snapshot export/restore (plaintext, full-column, epoch renewal)"`

---

### Task 9: 監督タイマー化+持ち越しクリーンアップ

**Files:**
- Modify: `iotkit-gateway/src/main.rs`(再起動バックオフの非閉塞化)
- Modify: `iotkit-ingest-client/src/lib.rs`(二重clone/try_submitのClosed区別/空valuesスキップ)
- Modify: `bravepi-mainboard-adapter/src/task/ingest_map.rs`(contact 256サンプル超チャンク化)
- Modify: 各テスト(50ms否定アサート除去/sleep整定E2EのNotify化)
- Modify: `core/ledger/src/store.rs`・`core/registry/src/store.rs`(from_db未知値/row_to_entry黙殺のwarnログ)
- Test: 既存置換+新規

**Interfaces:**
- Produces: 再起動待機中もイベント処理が継続するfan-inループ

- [ ] **Step 1: タイマー化の失敗テスト**(再起動待機中に別アダプタのイベントが処理される統合テスト。困難ならsupervision単体の遅延キューテストで代替し判断をreportに明記)
- [ ] **Step 2: 実装**(main.rs:236の`sleep(...).await`を除去。`mpsc::unbounded_channel::<AdapterId>()`を作り、AdapterClosed時は`tokio::spawn(async move { sleep(delay).await; let _ = tx.send(id); })`。selectに`Some(id) = rx_restart.recv()`腕を追加し再起動実行。restart_specs/trackerはループ所有のまま——spawnへはIDのみ)
- [ ] **Step 3: クリーンアップ9点**(計画3裁定記録どおり: ①ingest-client二重clone ②50ms否定アサート→決定的完了通知 ③collector.clone余剰 ④sleep整定E2E→Notify/チャネル完了待ち ⑤contact256超を複数itemに分割 ⑥try_submitのClosed/Full区別 ⑦空valuesは写像段でスキップ ⑧DeviceKind/ValueType/ChannelMode from_db未知値にwarn! ⑨row_to_entryのchannel_roles_json破損黙殺にwarn!)
- [ ] **Step 4: 全ワークスペーステスト→コミット**

Run: `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test --workspace`
`git commit -m "fix(gateway,ingest-client,adapters): non-blocking supervisor backoff and plan-3 carryover cleanups"`

---

## 実行順序と依存

T1→T2→T3→T4→T5→T6→T7→T8→T9(直列。T5/T6/T8はT4のクレート+generation規約に、T4〜T8はT3のAPIに依存)。

## 明示的な非目標(Wave 1へ)

R10 push配送・publication log・R14操作カタログ(dry-run/権限段階/**series値域変更**)・HTTPサーバ・水位起因の緊急パージ(D2 4クラス②以降の作動)・snapshotマージ復元・snapshot暗号化コンテナ(secrets実体化と同時)・トークン発行・保管対象ポリシー/購読フィルタの操作面・承認UI。

## レビュー裁定記録(2026-07-03 Fable+codex並行、統合25指摘→重複統合後20論点、全採用)

| 論点 | 出所 | 反映 |
|---|---|---|
| gatewayctl直接変異×ResolutionCache無効化なし(D5明文違反) | codex B1≡Fable B2 | generation counter共有(T4)。CLI全変異Tx+デーモン内変異(検疫失効)もbump |
| undo汚染マーキングの軸がevent_time(バックログすり抜け) | Fable B1 | received_at基準に変更+--since既定は監査イベントから導出(T5) |
| event_timeがage_ms復元(候補2)を脱落/collectorがage_ms破棄/バックフィル無条件採用 | codex B2≡Fable M1+M2 | 3段導出+collector復元実装+バックフィルにCASE降格(T2) |
| series set-range=R14侵食(D6はdry-run付きR14/Wave 1) | codex B3 | 削除。非目標に明記(T6) |
| snapshotが表示用一覧APIを再利用(全列復元にならない) | codex B4 | T8は独自のSELECT *全列ダンプ。T3とのIF依存を切断 |
| 突合の部分集合許容はD5「不一致ブロック」より緩い | codex M1 | 完全一致のみ通す(T5) |
| プロファイルがstaged単独(承認済み候補で突合不能) | Fable M3 | staged+aliveデバイスseries集合の2ソース(T5) |
| R22暗号化必須は誤引用(D2最小契約3: Wave 0平文可) | Fable M4 | 平文JSON+コンテナ形式は予約記述。chacha/argon2依存を除去(T8) |
| gatewayctl deps漏れ(ingest-contract)/Cargo.toml未列挙 | codex M2 | T5 Filesに追加 |
| conn.transaction()は実APIで不可/DEFERREDはBUSY衝突 | codex M3+Fable M6 | Transaction::new_unchecked(conn, Immediate)を規約化 |
| init_dbの無音空DB生成トラップ | Fable M5 | 既存ファイル必須。生成はrestore --createのみ |
| 検疫の時限自動失効(D5 Wave 0明文)の欠落 | Fable M7 | expire_quarantined_devicesをT7に追加 |
| T1テスト看板と実体の乖離(DROP経路未検証/v3_db実名/20件/ソートコメント) | Fable m1 | 合成旧DB→DROP検証テスト+記述修正 |
| TimeRange「T3で再利用」は虚偽 | Fable m2 | T1で削除 |
| -1/0併存時の規則未規定 | Fable m3 | -1優先+channel_form_conflict監査+テスト(T6) |
| purgeの単一巨大Tx | Fable m4 | LIMIT 10_000バッチループ(T7) |
| export読み取り一貫性/renew_epochのepoch行なし | Fable m5 | 単一読み取りTx+old_epoch=None扱い(T8) |
| sqlite_sequence手動更新は無用 | Fable m6 | 削除、単調性検証SELECTに置換(T8) |
| CSVにdevice_time欠落(D7出自併載) | Fable m7 | ヘッダ追加(T3) |
| 各タスクのcargoコマンド欠落/healthパス導出未規定 | codex MINOR+Fable m8 | 全タスクにRun行+health既定パス規定 |

### 差分再検証(codex、改訂2に対して): 20論点○/20、新規4件全採用で改訂3

| 新規指摘 | 反映 |
|---|---|
| [BLOCKER] collector冒頭のgeneration readがDEFERRED Txのread→write昇格を作る | collector Txを`Transaction::new_unchecked(conn, Immediate)`へ変更(T4。最初の実操作がwriteなので意味的にも正) |
| [MAJOR] T7のexpire+bumpが別Txに読める | 同一Immediate Txと明記 |
| [MAJOR] T8のBLOB↔UUID往復が曖昧(TEXT混入/FK破壊/reader落ち) | 列名固定の変換表+restore後のFK・BLOB型検証テストを明記 |
| [MAJOR] T1テストのテーブル実名(`_schema_version`) | 修正 |
| (補足) cmd/*.rs追加時のmain.rs配線漏れ | T5 Filesに明記(T6/T8も同様と注記) |
