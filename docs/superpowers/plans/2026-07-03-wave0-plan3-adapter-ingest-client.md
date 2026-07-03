# Wave 0 計画3: アダプタ内取り込みクライアント化+ブリッジ削除 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** D4の空席「取り込みクライアント」を `iotkit-ingest-client`(inproc)として新設し、measurement写像を各アダプタへ移設、ゲートウェイの暫定ブリッジ(bridge.rs)を削除する。

**Architecture:** アダプタ=ドライバ+ランタイム+契約クライアントの3部品(D4)。センサーデータはアダプタ内で `Envelope` 化され、inprocクライアント(有界spool+NoAck再送)経由でコレクタへ届く。`AdapterEvent` はengine/監督用のfrozen vocabularyとして並走を維持する(engine無改修=D5、D1移行フェーズ「engine/projection温存」)。ブリッジの翻訳責務は消滅し、bridge.rsを削除する。

**Tech Stack:** Rust (edition 2024), tokio, rusqlite 0.32, uuid。設計正本: D4(アダプタ解剖学+2026-07-03監査追記)、D1(Rust実装の地雷+移行フェーズ)、D5/D6(正準チャネル・有限必須の監査追記)。

## Global Constraints(設計追補の全掃引済み: D1/D4/D5/D6の2026-07-03監査追記を含む)

- **ドライバのクランプ・飽和・値域補正は禁止**(D4監査追記)。デコードはデータシート定義の変換のみ。値域判定はR8専権。
- **単位対応表の宣言義務**(D4監査追記): 各アダプタのmeasurement写像モジュールは「ドライバ出力単位×変換係数→D6正準単位」の対応表をdocコメント+コードで宣言する。**本計画の必須成果物**。
- **プロセス内クライアントはNoAck(再送対象)とClosed(監督対象)を型で区別**(D1監査追記)。計画2で実装済みの `SubmitError::NoAck/Closed` を消費する。
- **envelope_idは再送で不変**(D1)。Envelope構築時に一度だけ採番(プロセス内はUUIDv4可)し、クライアントの再送はエンベロープを不変のまま送る(dedupが吸収)。
- **チャネルの送出規約**(D5/D6監査追記との整合): 単ch測定は `channel_index: None`、固定役割(acceleration)は `Some(0..=2)`、汎用多ch(ADC)は値ごとに `Some(i)` へ分割。受信側正準化(None/Some(0)→-1)は評価器の仕事であり、送信側は上記の素直な形を送る。
- **AdapterEventはfrozen vocabulary**(D4): 依存を増やさない。本計画は取り込み経路をAdapterEventから外すが、engine/監督向けの既存流路は無改修で温存する。
- **Rejected/ItemRejectedはクライアントで再送しない**(D1: 終端)。`Duplicate` は成功扱い。`NoAck` と `Deferred` は**エンベロープ不変で再送**(Deferredはinprocでは返らないが、将来バインディング共用のため意味論どおり実装する=D1)。
- **再送バックオフにはジッタ必須**(D1: 同時再送ストーム対策)。乱数依存を避け、envelope_idのバイトから導く決定的ジッタでよい。
- **再送待機中も入力の吸い上げを止めない**: クライアントはバックオフsleepと入力受信をselectし、spoolへの排出(drop-oldest)を継続する。再送ループが入力を飢餓させると「最古からドロップ」が機能せず入力キュー側で最新が落ちる。
- 派生値(例: 加速度magnitude)は正準チャネル外——ワイヤに乗せない(D6決定11。導出はR9/消費者側)。
- テストコマンドは `CARGO_NET_OFFLINE=true cargo test -p <crate>`、最終タスクで `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test --workspace`。
- コミット規約: `feat(crate):` / `fix(crate):` / `refactor(crate):` + Co-Authored-By行(親が代行)。

## スコープ確認

**やる**: iotkit-ingest-client新設(inprocのみ。feature `inproc` をdefaultに)、bravepi/polling両ランタイムへの写像移設+クライアント注入、LIS2DUXS12ドライバのmG素通し化(往復変換の解消)、bridge.rs削除、ゲートウェイ配線(クライアント死活の監視)、CLAUDE.mdの移行注記更新。

**やらない(宛先)**: HTTP/UDS/MQTTバインディング(D1フェーズ3/4)、永続spool(Wave 0はメモリ有界spool——プロセス断で失われるのはD1軽量プロファイルの範囲)、engine/projectionの改修(キュー3確定後)、supervisionバックオフのタイマーメッセージ化(計画4)、能力宣言(キュー5)。

## 実物調査の要点(計画作成時+二重レビュー裁定で確認済み)

- `SensorReading.labels` は既に `Vec<String>`(D1フェーズ1消化済み)。残存クランプはgrepでゼロ(VL53L1Xは計画2で除去済み)。
- **LIS2DUXS12ドライバはワイヤ値を÷1000でg化し(`x_g`ラベル)、旧ブリッジが×1000で戻す往復変換**をしていた(`lis2duxs12.rs:45,58`。UART=Float32LE mG、I2C=Int16LE×0.244mG)。本計画でドライバをmG素通し+magnitude廃止に改修する(データシートの数学=ワイヤ単位のデコードまで)。
- mcp3427はドライバ内でV→mV変換済み(出力mV、ラベルは`ch1_volt`で誤解を招く→`ch1_mv`へ改修)。mcp9600=℃、opt3001=lux、sdp810=Pa、vl53l1x=mm——正準単位と一致(×1)。
- **contactは多値=1接点の時系列サンプル**(`contact.rs:8-14`: payloadの先頭`data_count`バイトが連続サンプル)。チャネル分割ではなく**サンプルごとの複数item(channel_index=None)**へ分割する。※現行bridge.rsは汎用len>1分割でサンプル番号をチャネル化する誤モデル化をしており(genericモードが受理するため無検疫)、本計画の写像で是正される。
- polling runtimeのSensorData構築は純関数 `apply_outcomes`(`polling_loop.rs:156-173`)内。**注入はイベント送出側**で行い(bridge同様のdevice_key基準写像)、`apply_outcomes`とPollOutcomeは無改修とする。
- `polling_loop()` は現状 `AdapterId` を受け取っていない(positional hardware_idと`Envelope.source`に必要→引数追加)。
- `start` シグネチャ変更の既存呼び出し元(同一タスク内で追従必須): gateway `main.rs:319/344`、`bravepi-mainboard-adapter/poc/src/main.rs:24`、`rpi-local-adapter/tests/integration.rs:21`。
- ingest_map系は `iotkit_ingest_contract` の型を直接useする——bravepi/polling-runtime両Cargo.tomlに**contract依存の追加が必要**(現状なし)。
- engineの状態消費者はシャットダウンログ1行のみ(main.rs:302)。並走維持のコストは実質ゼロ。

## File Structure

```
iotkit-ingest-client/              # 新クレート(D4の空席)
├── Cargo.toml                     # features: inproc(default)。依存: contract(常時)、collector/tokio(inproc)
├── src/lib.rs                     # 公開面: IngestClient / spawn_inproc / new_envelope / channel_for_test
└── tests/inproc_e2e.rs            # 実コレクタ相手のE2E(受理/NoAck再送/Closed退出/spool溢れ)
bravepi-mainboard-adapter/
├── sensors/src/lis2duxs12.rs      # 変更: mG素通し・magnitude廃止・ラベルx_mg/y_mg/z_mg
├── sensors/src/mcp3427.rs         # 変更: ラベルch1_mv/ch2_mv(値は従来どおりmV)
├── src/task/ingest_map.rs         # 新規: 単位対応表+SensorData→Envelope写像
├── src/task/mod.rs                # 変更: pub(crate) mod ingest_map
├── src/task/handle.rs             # 変更: start()にIngestClient引数追加
├── src/task/event_loop.rs         # 変更: SensorData送出点でEnvelope送出を並走
└── src/task/event_loop_test.rs    # 変更: envelope捕捉アサート追加
iotkit-polling-adapter-runtime/
├── src/ingest_map.rs              # 新規: 単位対応表+読み取り→Envelope写像(汎用)
├── src/lib.rs                     # 変更: start署名にingest引数追加(configは無変更)、adapter_idをループへ
└── src/polling_loop.rs            # 変更: 読み取り成功時にEnvelope送出を並走
rpi-local-adapter/src/lib.rs       # 変更: start()にIngestClient引数追加(素通し)
iotkit-gateway/
├── src/main.rs                    # 変更: クライアントspawn+注入+fan-inのブリッジ呼び出し削除+client死活監視
├── src/bridge.rs                  # 削除
└── Cargo.toml                     # 変更: iotkit-ingest-client依存追加
CLAUDE.md                          # 変更: 移行注記(ブリッジ1ファイル限定→削除完了)の更新
Cargo.toml                         # 変更: workspace membersにiotkit-ingest-client
```

---

### Task 1: iotkit-ingest-client クレート(inprocバインディング)

**Files:**
- Create: `iotkit-ingest-client/Cargo.toml`
- Create: `iotkit-ingest-client/src/lib.rs`
- Create: `iotkit-ingest-client/tests/inproc_e2e.rs`
- Modify: `Cargo.toml`(members追加)

**Interfaces:**
- Consumes: `iotkit_core_collector::{Collector, SubmitError}`、`iotkit_ingest_contract::*`
- Produces(後続タスクが使う):
  - `IngestClient`(Clone可): `pub fn try_submit(&self, envelope: Envelope) -> Result<(), IngestClientFull>`(非同期文脈を要求しない非ブロッキング投入。満杯時Err=呼び出し側はドロップ+ログ)
  - `pub fn spawn_inproc(collector: Collector, queue_cap: usize, spool_cap: usize) -> (IngestClient, tokio::task::JoinHandle<()>)`——クライアントタスク: 入力mpsc→有界spool(VecDeque)→ `collector.submit`。ack処理: Accepted(quarantined含む)/Duplicate=完了、Rejected/ItemRejected=warnログ+完了(終端)、`NoAck`=バックオフ再送(エンベロープ不変)、`Closed`=タスク終了(JoinHandle経由でゲートウェイが検知)
  - `pub fn new_envelope(source: &str, items: Vec<ReadingItem>) -> Envelope`(UUIDv4採番を一箇所に。uuid依存は本クレートが所有)
  - `pub fn channel_for_test(cap: usize) -> (IngestClient, tokio::sync::mpsc::Receiver<Envelope>)`(アダプタ側テスト用: 実タスクなしでEnvelopeを捕捉)
- 定数: `pub const DEFAULT_QUEUE_CAP: usize = 256;` / `pub const DEFAULT_SPOOL_CAP: usize = 1024;` / `pub const RETRY_BACKOFF_MS: [u64; 4] = [100, 500, 2000, 5000];`(以後5000固定)

- [ ] **Step 1: クレート骨格**

ルート `Cargo.toml` membersの `"iotkit-ingest-contract",` の直後に追加:

```toml
    "iotkit-ingest-client",
```

`iotkit-ingest-client/Cargo.toml`:

```toml
[package]
name = "iotkit-ingest-client"
version = "0.1.0"
edition = "2024"

[features]
default = ["inproc"]
inproc = ["dep:iotkit-core-collector", "dep:tokio"]

[dependencies]
iotkit-ingest-contract = { path = "../iotkit-ingest-contract" }
uuid = { version = "1", features = ["v4"] }
tracing = "0.1"
iotkit-core-collector = { path = "../core/collector", optional = true }
tokio = { version = "1", features = ["sync", "rt", "macros", "time"], optional = true }

[dev-dependencies]
iotkit-core-storage = { path = "../core/storage", features = ["test-util"] }
iotkit-core-ledger = { path = "../core/ledger" }
iotkit-core-timeseries = { path = "../core/timeseries" }
iotkit-core-registry = { path = "../core/registry" }
tokio = { version = "1", features = ["sync", "rt", "macros", "time", "rt-multi-thread"] }
```

- [ ] **Step 2: 失敗するテストを書く(E2E)**

`iotkit-ingest-client/tests/inproc_e2e.rs`:

```rust
//! inprocクライアントのE2E: 実コレクタ(SqliteRegistry)相手にD1クライアント義務を検証する。
//! - ack意味論の消費(Accepted/Duplicate=完了、Rejected=終端、NoAck=不変再送)
//! - 有界spool(溢れはdrop-oldest+警告)
//! - Closed(コレクタ死亡)でのタスク退出
use iotkit_core_collector::Collector;
use iotkit_core_ledger as ledger;
use iotkit_core_registry::SqliteRegistry;
use iotkit_ingest_client::{new_envelope, spawn_inproc};
use iotkit_ingest_contract::*;
use std::sync::Arc;

fn full_db() -> iotkit_core_storage::DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.sort_by_key(|m| m.version);
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

fn item(hw: &str, key: &str, value: f64) -> ReadingItem {
    ReadingItem {
        subject_hint: Some(hw.into()),
        measurement_key: key.into(),
        channel_index: None,
        series_variant: None,
        values: vec![value],
        device_time_ms: None,
        time_source: TimeSource::Gateway,
        age_ms: None, rssi: None, battery_pct: None,
    }
}

async fn readings_count(db: &iotkit_core_storage::DbHandle) -> i64 {
    db.with_conn_sync(|conn| {
        Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap())
    }).unwrap()
}

/// クライアントの完了を能動的に待つ(ポーリング。テスト専用)
async fn wait_for_readings(db: &iotkit_core_storage::DbHandle, n: i64) {
    for _ in 0..200 {
        if readings_count(db).await >= n { return; }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {n} readings");
}

#[tokio::test]
async fn accepted_envelope_reaches_readings() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _ch) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let (client, _h) = spawn_inproc(collector, 16, 64);
    let e = new_envelope("bravepi-mainboard:/dev/ttyAMA0",
        vec![item("ble:aa", "temperature_c", 21.5)]);
    client.try_submit(e).unwrap();
    wait_for_readings(&db, 1).await;
}

#[tokio::test]
async fn envelope_id_is_stable_and_duplicate_is_success() {
    // 同一エンベロープを2回投入 → コレクタのdedupがDuplicateを返し、クライアントは成功扱いで前進する
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _ch) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let (client, _h) = spawn_inproc(collector, 16, 64);
    let e = new_envelope("test-adapter", vec![item("ble:aa", "temperature_c", 21.5)]);
    client.try_submit(e.clone()).unwrap();
    client.try_submit(e).unwrap();
    // 後続が処理されることの証拠に3通目を流す
    let e3 = new_envelope("test-adapter", vec![item("ble:aa", "temperature_c", 22.0)]);
    client.try_submit(e3).unwrap();
    wait_for_readings(&db, 2).await; // 1通目+3通目のみ書かれる(2通目はDuplicate)
    assert_eq!(readings_count(&db).await, 2);
}

#[tokio::test]
async fn noack_is_retried_with_same_envelope_until_recovery() {
    // トリガーでregistry_entriesへのINSERT(auto-enable)を失敗させNoAckを誘発 →
    // クライアントはエンベロープ不変で再送し続け、トリガー除去後に自然回復する(D1)。
    let db = full_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER fail_enable BEFORE INSERT ON registry_entries
             BEGIN SELECT RAISE(ABORT, 'simulated'); END;",
        )?;
        Ok(())
    }).unwrap();
    let (collector, _ch) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let (client, _h) = spawn_inproc(collector, 16, 64);
    let e = new_envelope("test-adapter", vec![item("ble:aa", "temperature_c", 21.5)]);
    client.try_submit(e).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await; // 数回のNoAck再送を経過させる
    assert_eq!(readings_count(&db).await, 0, "障害中は未耐久のまま");
    db.with_conn_sync(|conn| { conn.execute_batch("DROP TRIGGER fail_enable;")?; Ok(()) }).unwrap();
    wait_for_readings(&db, 1).await; // 再送で回復。entryも監査イベントも1つずつ
    let (entries, events): (i64, i64) = db.with_conn_sync(|conn| {
        Ok((
            conn.query_row("SELECT COUNT(*) FROM registry_entries", [], |r| r.get(0)).unwrap(),
            conn.query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                [], |r| r.get(0)).unwrap(),
        ))
    }).unwrap();
    assert_eq!((entries, events), (1, 1));
}

#[tokio::test]
async fn terminal_rejection_is_not_retried() {
    // 文法違反キー=エンベロープ内item拒否(終端)。クライアントは再送せず前進する
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _ch) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let (client, _h) = spawn_inproc(collector, 16, 64);
    client.try_submit(new_envelope("test-adapter",
        vec![item("ble:aa", "Bad:Key", 1.0)])).unwrap();
    client.try_submit(new_envelope("test-adapter",
        vec![item("ble:aa", "temperature_c", 21.5)])).unwrap();
    wait_for_readings(&db, 1).await; // 2通目が届く=1通目で停滞していない
    assert_eq!(readings_count(&db).await, 1);
}

#[tokio::test]
async fn spool_overflow_drops_oldest_and_keeps_newest() {
    // コレクタを恒久障害にし、全量投入→解除の決定的手順で有界性とdrop-oldestを検証する。
    // バックオフ待機中も入力排出が継続する設計なので、12件全てがspool(cap=4)へ流れ込み
    // 古い側から溢れる——生き残るのは新しい側のみ。
    let db = full_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER fail_all BEFORE INSERT ON ingest_dedup
             BEGIN SELECT RAISE(ABORT, 'down'); END;",
        )?;
        Ok(())
    }).unwrap();
    let (collector, _ch) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let (client, _h) = spawn_inproc(collector, 64, 4); // queue_cap=64: 入力側では落ちない
    for i in 0..12 {
        let e = new_envelope("test-adapter",
            vec![item("ble:aa", "temperature_c", 20.0 + i as f64)]);
        client.try_submit(e).unwrap();
    }
    // 全量がspoolへ排出されdrop-oldestが起きるまで待つ(障害中は未耐久のまま)
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(readings_count(&db).await, 0, "障害中は未耐久");
    db.with_conn_sync(|conn| { conn.execute_batch("DROP TRIGGER fail_all;")?; Ok(()) }).unwrap();
    wait_for_readings(&db, 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    let n = readings_count(&db).await;
    assert!((1..=5).contains(&n), "有界spool(cap=4+送信中1): n={n}");
    // drop-oldestの証拠: 最新エンベロープ(値31.0)が生き残る
    let max: f64 = db.with_conn_sync(|conn| {
        Ok(conn.query_row(
            "SELECT MAX(CAST(json_extract(values_json, '$[0]') AS REAL)) FROM readings",
            [], |r| r.get(0)).unwrap())
    }).unwrap();
    assert_eq!(max, 31.0, "最新エンベロープはドロップされない(drop-oldest)");
}

#[tokio::test]
async fn collector_death_exits_client_task() {
    // コレクタのJoinHandleをabort → submitがClosed → クライアントタスクが退出する
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, collector_handle) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let (client, client_handle) = spawn_inproc(collector, 16, 64);
    collector_handle.abort();
    let _ = client.try_submit(new_envelope("test-adapter",
        vec![item("ble:aa", "temperature_c", 21.5)]));
    // クライアントタスクはClosed検知で終了する(ゲートウェイのfail-fast検知点)
    tokio::time::timeout(std::time::Duration::from_secs(5), client_handle)
        .await
        .expect("client task must exit after collector death")
        .expect("client task must not panic");
}

#[test]
fn new_envelope_assigns_unique_ids_and_source() {
    let e1 = iotkit_ingest_client::new_envelope("s", vec![]);
    let e2 = iotkit_ingest_client::new_envelope("s", vec![]);
    assert_ne!(e1.envelope_id, e2.envelope_id);
    assert_eq!(e1.source, "s");
    assert_eq!(e1.declaration_version, None);
}
```

- [ ] **Step 3: テストが失敗することを確認**

Run: `CARGO_NET_OFFLINE=true cargo test -p iotkit-ingest-client`
Expected: FAIL(コンパイルエラー: lib未実装)

- [ ] **Step 4: 実装**

`iotkit-ingest-client/src/lib.rs`:

```rust
//! iotkit-ingest-client: 取り込み契約クライアント(D4の第3部品、北向き専用)。
//! Wave 0はinprocバインディングのみ。ワイヤ契約が規範であり、本クレートは便宜品(D4)。
//!
//! クライアントの義務(D1):
//! - ack意味論の消費: Accepted/Duplicate=完了、Rejected/ItemRejected=終端(再送しない)、
//!   ackなし(NoAck)=エンベロープ不変のままバックオフ再送
//! - envelope_idは構築時に一度だけ採番し、再送で変えない(dedupが吸収)
//! - 有界spool: 溢れは最古からドロップ+警告(Wave 0はメモリのみ=D1軽量プロファイル)
use iotkit_ingest_contract::{Envelope, ReadingItem};

pub const DEFAULT_QUEUE_CAP: usize = 256;
pub const DEFAULT_SPOOL_CAP: usize = 1024;
pub const RETRY_BACKOFF_MS: [u64; 4] = [100, 500, 2000, 5000];

/// エンベロープ採番の一箇所(プロセス内はUUIDv4可=D1)。
pub fn new_envelope(source: &str, items: Vec<ReadingItem>) -> Envelope {
    Envelope {
        envelope_id: uuid::Uuid::new_v4().to_string(),
        source: source.to_string(),
        declaration_version: None,
        items,
    }
}

#[cfg(feature = "inproc")]
pub use inproc::{channel_for_test, spawn_inproc, IngestClient, IngestClientFull};

#[cfg(feature = "inproc")]
mod inproc {
    use super::*;
    use iotkit_core_collector::{Collector, SubmitError};
    use iotkit_ingest_contract::{AckStatus, ItemStatus};
    use std::collections::VecDeque;
    use tokio::sync::mpsc;

    /// 入力キュー満杯(呼び出し側はドロップしてよい——送信側の逆圧シグナル)。
    #[derive(Debug)]
    pub struct IngestClientFull;

    /// アダプタランタイムが持つ送信ハンドル。非ブロッキング(ポーリングループ/イベントループを
    /// コレクタの都合で止めない)。
    #[derive(Clone)]
    pub struct IngestClient {
        tx: mpsc::Sender<Envelope>,
    }

    impl IngestClient {
        pub fn try_submit(&self, envelope: Envelope) -> Result<(), IngestClientFull> {
            self.tx.try_send(envelope).map_err(|_| IngestClientFull)
        }
    }

    /// テスト用: 実タスクなしでEnvelopeを捕捉する受け口を返す。
    pub fn channel_for_test(cap: usize) -> (IngestClient, mpsc::Receiver<Envelope>) {
        let (tx, rx) = mpsc::channel(cap);
        (IngestClient { tx }, rx)
    }

    /// inprocクライアントタスクを起動する。タスクはコレクタ死亡(Closed)で退出し、
    /// ゲートウェイはJoinHandleでそれを監視する(fail-fast=計画2のSubmitError分離の消費)。
    ///
    /// 設計不変則(計画レビュー裁定反映):
    /// - バックオフ待機中も入力の吸い上げ(spoolへの排出+drop-oldest)を止めない
    ///   (再送ループが入力を飢餓させると入力キュー側で最新が落ち、「最古からドロップ」が嘘になる)
    /// - NoAck/Deferredはエンベロープ不変で再送、ジッタ付きバックオフ(D1)
    pub fn spawn_inproc(
        collector: Collector,
        queue_cap: usize,
        spool_cap: usize,
    ) -> (IngestClient, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<Envelope>(queue_cap);
        let handle = tokio::spawn(async move {
            let mut spool: VecDeque<Envelope> = VecDeque::new();
            let mut backoff_until: Option<tokio::time::Instant> = None;
            let mut attempt = 0usize;
            loop {
                // 1) 送信可能なら先頭を送る
                let ready = !spool.is_empty()
                    && backoff_until.map_or(true, |t| tokio::time::Instant::now() >= t);
                if ready {
                    let envelope = spool.front().cloned().expect("spool non-empty");
                    match collector.submit(envelope.clone()).await {
                        Ok(ack) if matches!(ack.status, AckStatus::Deferred) => {
                            // inprocでは返らないが、将来バインディング共用のため意味論どおり
                            // 不変再試行する(D1)
                            schedule_retry(&mut backoff_until, &mut attempt,
                                &envelope.envelope_id, "deferred");
                        }
                        Ok(ack) => {
                            log_ack(&ack.status, &envelope.envelope_id);
                            spool.pop_front();
                            attempt = 0;
                            backoff_until = None;
                        }
                        Err(SubmitError::NoAck) => {
                            schedule_retry(&mut backoff_until, &mut attempt,
                                &envelope.envelope_id, "no ack (storage failure)");
                        }
                        Err(SubmitError::Closed) => {
                            tracing::error!(spooled = spool.len(),
                                "collector closed; ingest client exiting (supervisor will fail-fast)");
                            return;
                        }
                    }
                    continue;
                }
                // 2) 待機: 入力受信(バックオフ中も排出継続)またはバックオフ満了
                if let Some(deadline) = backoff_until {
                    tokio::select! {
                        maybe = rx.recv() => match maybe {
                            Some(e) => push_bounded(&mut spool, e, spool_cap),
                            None => { shutdown_note(&spool); return; }
                        },
                        _ = tokio::time::sleep_until(deadline) => { backoff_until = None; }
                    }
                } else {
                    // ここに来るのはspoolが空の場合のみ
                    match rx.recv().await {
                        Some(e) => push_bounded(&mut spool, e, spool_cap),
                        None => { shutdown_note(&spool); return; }
                    }
                }
            }
        });
        (IngestClient { tx }, handle)
    }

    fn push_bounded(spool: &mut VecDeque<Envelope>, e: Envelope, cap: usize) {
        if spool.len() >= cap {
            let dropped = spool.pop_front();
            tracing::warn!(
                envelope_id = dropped.as_ref().map(|d| d.envelope_id.as_str()),
                "ingest spool overflow: dropping oldest (bounded spool, D1 lightweight profile)"
            );
        }
        spool.push_back(e);
    }

    /// バックオフ再送の予約。ジッタはenvelope_idバイト和から導く決定的値
    /// (乱数依存なしでD1のジッタ義務を満たす)。
    fn schedule_retry(
        backoff_until: &mut Option<tokio::time::Instant>,
        attempt: &mut usize,
        envelope_id: &str,
        why: &str,
    ) {
        let base = RETRY_BACKOFF_MS[(*attempt).min(RETRY_BACKOFF_MS.len() - 1)];
        let jitter = envelope_id
            .bytes()
            .fold(0u64, |a, b| a.wrapping_add(b as u64))
            % (base / 4 + 1);
        *attempt += 1;
        tracing::warn!(envelope_id, attempt = *attempt, backoff_ms = base + jitter, why,
            "retrying same envelope");
        *backoff_until = Some(
            tokio::time::Instant::now() + std::time::Duration::from_millis(base + jitter),
        );
    }

    fn shutdown_note(spool: &VecDeque<Envelope>) {
        if !spool.is_empty() {
            tracing::warn!(spooled = spool.len(),
                "ingest client shutting down with unsent envelopes (memory spool, D1 lightweight profile)");
        }
    }

    fn log_ack(status: &AckStatus, envelope_id: &str) {
        match status {
            AckStatus::Accepted { items } => {
                for (i, it) in items.iter().enumerate() {
                    if let ItemStatus::ItemRejected { reason_code, message } = it {
                        tracing::warn!(envelope_id, item = i, ?reason_code, message,
                            "item terminally rejected");
                    }
                }
            }
            AckStatus::Duplicate => {
                tracing::debug!(envelope_id, "duplicate (already durable)");
            }
            AckStatus::Rejected { reason_code, message } => {
                tracing::warn!(envelope_id, ?reason_code, message,
                    "envelope terminally rejected (not retried)");
            }
            AckStatus::Deferred => {
                // プロセス内では返らない(D1)。防御的にログのみ
                tracing::error!(envelope_id, "unexpected Deferred on inproc binding");
            }
        }
    }
}
```

- [ ] **Step 5: テストが通ることを確認**

Run: `CARGO_NET_OFFLINE=true cargo test -p iotkit-ingest-client`
Expected: PASS(E2E 6本+unit 1本)

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml iotkit-ingest-client
git commit -m "feat(ingest-client): inproc binding with bounded spool and NoAck retry (D4/D1)"
```

---

### Task 2: LIS2DUXS12のmG素通し化+mcp3427ラベル修正(ドライバ規律の適用)

**Files:**
- Modify: `bravepi-mainboard-adapter/sensors/src/lis2duxs12.rs`
- Modify: `bravepi-mainboard-adapter/sensors/src/mcp3427.rs`

**Interfaces:**
- Produces: `lis2duxs12` の両デコード経路が `values=[x_mg, y_mg, z_mg]`(3値・mG)、`labels=["x_mg","y_mg","z_mg"]` を返す。派生値magnitudeは廃止(D6決定11: 派生はR9/消費者側)。

**背景**: ドライバはワイヤのmG値を÷1000でg化し、旧ブリッジが×1000で戻していた(無意味な往復+`x_g`ラベルの単位偽装)。D4監査追記「デコード=データシート(ワイヤ)定義の変換のみ」に従い素通しへ。

- [ ] **Step 1: 失敗するテストを書く**

`lis2duxs12.rs` のテストを書き換える(実装者は既存テストの期待値を実物で確認して置換する。パターン):

```rust
    #[test]
    fn uart_values_are_passed_through_in_mg() {
        // ワイヤはFloat32LE×3(mG)。ドライバは単位変換せず素通しする(D4: データシートの数学のみ)。
        // 旧実装は÷1000でg化し旧ブリッジが×1000で戻す往復変換をしていた(計画3で解消)。
        let mut payload = Vec::new();
        for v in [12.0f32, -34.0, 998.0] {
            payload.extend_from_slice(&v.to_le_bytes());
        }
        let reading = from_uart_payload(&payload);
        assert_eq!(reading.values.len(), 3, "派生値magnitudeはワイヤに乗せない(D6決定11)");
        assert!((reading.values[0] - 12.0).abs() < 1e-3);
        assert!((reading.values[2] - 998.0).abs() < 1e-3);
        assert_eq!(reading.labels, vec!["x_mg", "y_mg", "z_mg"]);
    }
```

(I2C経路にも同型のテスト。既存のmagnitude期待テストは削除。)

- [ ] **Step 2: 失敗確認 → 実装 → 成功確認**

Run: `CARGO_NET_OFFLINE=true cargo test -p bravepi-sensors`(FAIL→実装→PASS)

実装: 両経路の `x / 1000.0` 等を素通しに、magnitude計算とラベル4本目を削除、docコメントを「mG素通し」に更新。mcp3427はラベル文字列 `"ch1_volt"/"ch2_volt"` → `"ch1_mv"/"ch2_mv"` のみ変更(値は従来どおりmV)し、該当テストの期待ラベルを追従。

**注意(bridge.rsの同時追従——Task 5削除までの単位整合維持)**: ドライバ出力がmGになるため、旧bridge.rsの加速度特別分岐(g→mG ×1000+`take(3)`)は本タスクで**分岐ごと削除**する(`canonical_values` は常に `reading.values.clone()` に単純化)。bridgeテストの追従を明示する:
- `multi_value_reading_becomes_per_channel_items`: 入力を `vec![12.0, -34.0, 998.0]`(mG)+ラベル `x_mg/y_mg/z_mg` に変更、期待値も同値(×1000期待を除去)
- `acceleration_derived_magnitude_channel_is_dropped`: **削除**(ドライバがmagnitudeを出さなくなり検証対象が消滅。3値分割の検証はmulti_valueテストが担う)
- `bridge_output_flows_through_collector_to_readings` は温度なので無影響

`convert.rs` / `convert_test.rs` のg値・ラベル依存も掃引(`grep -rn "magnitude\|x_g\|ch1_volt" bravepi-mainboard-adapter/`)して追従する。Filesに `iotkit-gateway/src/bridge.rs` を追加。

- [ ] **Step 3: ワークスペース確認+Commit**

Run: `CARGO_NET_OFFLINE=true cargo test -p bravepi-sensors -p bravepi-mainboard-adapter -p iotkit-gateway`
Expected: PASS

```bash
git add bravepi-mainboard-adapter iotkit-gateway
git commit -m "refactor(sensors): LIS2DUXS12 passes wire mG through; drop derived magnitude; honest ADC labels"
```

---

### Task 3: bravepi measurement写像モジュール+event_loopへのクライアント注入

**Files:**
- Create: `bravepi-mainboard-adapter/src/task/ingest_map.rs`
- Modify: `bravepi-mainboard-adapter/src/task/mod.rs`(`pub(crate) mod ingest_map;`)
- Modify: `bravepi-mainboard-adapter/src/task/handle.rs`(`start(port, ingest: Option<IngestClient>)`)
- Modify: `bravepi-mainboard-adapter/src/task/event_loop.rs`(SensorData送出点でEnvelope並走)
- Modify: `bravepi-mainboard-adapter/src/task/event_loop_test.rs`(envelope捕捉)
- Modify: `bravepi-mainboard-adapter/Cargo.toml`(`iotkit-ingest-client` と **`iotkit-ingest-contract`** の依存追加——ingest_mapがReadingItem/TimeSourceを直接useする)
- Modify: `iotkit-gateway/src/main.rs`(`start_bravepi` 内の `task::start` 呼び出しに `None` を追加——Task 5でSomeに変わる。**同一タスク内で追従しworkspaceを壊さない**)
- Modify: `bravepi-mainboard-adapter/poc/src/main.rs`(`task::start` 呼び出しに `None` 追加)

**Interfaces:**
- Consumes: Task 1の `IngestClient` / `new_envelope` / `channel_for_test`
- Produces: `ingest_map::to_items(device_key, reading, rssi, battery_pct) -> Option<Vec<ReadingItem>>`——**単位対応表(下表)を実装+docコメントで宣言**

**単位対応表(D4監査追記の宣言義務——docコメントとしてモジュール冒頭に逐語掲載する)**:

```text
| SensorType(ドライバ)          | ドライバ出力                       | 変換 | measurement_key(D6)     | 分割規約                                        |
|-------------------------------|-----------------------------------|------|--------------------------|-------------------------------------------------|
| ContactInput (contact)        | 0/1 ×data_count(時系列サンプル) | ×1   | contact_state            | サンプルごとに複数item・**channel_index=None**   |
| ContactOutput (contact)       | 0/1 ×data_count(同上)           | ×1   | contact_output_state     | 同上                                             |
| Adc (mcp3427)                 | mV ch1,ch2(物理2ch)             | ×1   | voltage_mv               | 値ごとに Some(0),Some(1)                         |
| Ranging (vl53l1x)             | mm                                | ×1   | distance_mm              | 単一item・None                                   |
| Temperature (mcp9600)         | ℃                                | ×1   | temperature_c            | 単一item・None                                   |
| Acceleration (lis2duxs12)     | mG x,y,z(Task 2改修後)          | ×1   | acceleration_mg          | 値ごとに Some(0..=2)(固定役割=D6決定12)       |
| DifferentialPressure (sdp810) | Pa                                | ×1   | differential_pressure_pa | 単一item・None                                   |
| Illuminance (opt3001)         | lux                               | ×1   | illuminance_lux          | 単一item・None                                   |
| Unknown(_)                    | -                                 | 送出しない(warnログ)                                        |

分割規約の根拠: 多値の意味はSensorTypeごとに異なる——ADC/加速度は「物理チャネル」(channel_index化)、
接点は「1接点の時系列サンプル」(channel化するとサンプル番号をチャネルとして捏造する。計画レビューBLOCKER)。
汎用のlen>1分割は禁止し、型ごとに宣言する。
```

- [ ] **Step 1: 失敗するテストを書く(ingest_map単体)**

`ingest_map.rs` に上記対応表のdocコメント+スタブ+テスト。テストは旧bridge.rsのテスト資産を移植する(実装者はbridge.rsの `bravepi_key_maps_to_ble_hardware_id_and_d6_key` / `multi_value_reading_becomes_per_channel_items` / `unknown_sensor_type_returns_none` を本モジュール向けに書き換え移植。加速度は**mG素通し**が前提になったので×1000期待は書かない):

```rust
//! measurement写像(D4: ランタイムの責務)。SensorData(旧語彙)→ ReadingItem(取り込み契約)。
//! (モジュール冒頭に上記単位対応表を逐語掲載)
use iotkit_core_types::{DeviceKey, SensorReading, SensorType};
use iotkit_ingest_contract::{ReadingItem, TimeSource};

/// DeviceKey → hardware_id 正規形(D5決定2)。
/// BravePI: "bravepi-mainboard:{device_number}:{suffix}" → 個体識別型 "ble:{device_number}"
pub(crate) fn hardware_id_for(device_key: &DeviceKey) -> Option<String> {
    let parts: Vec<&str> = device_key.as_str().split(':').collect();
    match parts.as_slice() {
        ["bravepi-mainboard", device_number, _suffix] => Some(format!("ble:{device_number}")),
        _ => None,
    }
}

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

fn make_item(
    hw: &str, key: &str, channel_index: Option<u16>, values: Vec<f64>,
    rssi: Option<i16>, battery_pct: Option<u8>,
) -> ReadingItem {
    ReadingItem {
        subject_hint: Some(hw.to_string()),
        measurement_key: key.to_string(),
        channel_index,
        series_variant: None,
        values,
        device_time_ms: None,
        time_source: TimeSource::Gateway,
        age_ms: None, rssi, battery_pct,
    }
}

/// SensorData 1件 → ReadingItem列。分割規約は単位対応表(冒頭)のとおり**SensorTypeごとに宣言**——
/// 汎用のlen>1分割は禁止(接点の時系列サンプルをチャネル化する事故の再発防止)。
/// Unknown型・非BravePI形式キーはNone(送出しない。warnは呼び出し側)。
pub(crate) fn to_items(
    device_key: &DeviceKey,
    reading: &SensorReading,
    rssi: Option<i16>,
    battery_pct: Option<u8>,
) -> Option<Vec<ReadingItem>> {
    let key = measurement_key_for(&reading.sensor_type)?;
    let hw = hardware_id_for(device_key)?;
    let items = match reading.sensor_type {
        // 物理チャネル/固定役割: 値ごとにchannel_index付きで分割(D6決定12)
        SensorType::Acceleration | SensorType::Adc if reading.values.len() > 1 => reading
            .values.iter().enumerate()
            .map(|(i, v)| make_item(&hw, key, Some(i as u16), vec![*v], rssi, battery_pct))
            .collect(),
        // 接点: 多値は1接点の時系列サンプル——サンプルごとのitem・channelなし
        SensorType::ContactInput | SensorType::ContactOutput if reading.values.len() > 1 => reading
            .values.iter()
            .map(|v| make_item(&hw, key, None, vec![*v], rssi, battery_pct))
            .collect(),
        // 単ch型(および全型の単値): 単一item・channelなし
        _ => vec![make_item(&hw, key, None, reading.values.clone(), rssi, battery_pct)],
    };
    Some(items)
}
```

テスト(同モジュール内、bridge資産の移植+mG前提):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::{DeviceKey, SensorReading, SensorType};

    #[test]
    fn bravepi_key_maps_to_ble_hardware_id_and_d6_key() {
        let items = to_items(
            &DeviceKey::new("bravepi-mainboard:00000000000000ab:temperature"),
            &SensorReading::new(SensorType::Temperature, vec![21.5], vec!["celsius".into()]),
            Some(-60), Some(90),
        ).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].subject_hint.as_deref(), Some("ble:00000000000000ab"));
        assert_eq!(items[0].measurement_key, "temperature_c");
        assert_eq!(items[0].channel_index, None);
        assert_eq!(items[0].values, vec![21.5]);
        assert_eq!(items[0].rssi, Some(-60));
    }

    #[test]
    fn acceleration_mg_splits_into_three_fixed_channels_without_conversion() {
        // ドライバ出力は既にmG(Task 2)——写像は×1(単位対応表どおり)
        let items = to_items(
            &DeviceKey::new("bravepi-mainboard:00000000000000cc:acceleration"),
            &SensorReading::new(SensorType::Acceleration, vec![12.0, -34.0, 998.0],
                vec!["x_mg".into(), "y_mg".into(), "z_mg".into()]),
            None, None,
        ).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].channel_index, Some(0));
        assert_eq!(items[0].values, vec![12.0]);
        assert_eq!(items[2].channel_index, Some(2));
        assert_eq!(items[2].values, vec![998.0]);
    }

    #[test]
    fn adc_two_channels_split() {
        let items = to_items(
            &DeviceKey::new("bravepi-mainboard:00000000000000dd:adc"),
            &SensorReading::new(SensorType::Adc, vec![1650.0, 3300.0],
                vec!["ch1_mv".into(), "ch2_mv".into()]),
            None, None,
        ).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].channel_index, Some(1));
        assert_eq!(items[1].measurement_key, "voltage_mv");
    }

    #[test]
    fn contact_samples_split_into_items_without_channel() {
        // 接点の多値は時系列サンプル(contact.rs)——サンプル番号をチャネル化しない
        let items = to_items(
            &DeviceKey::new("bravepi-mainboard:00000000000000ee:contact_input"),
            &SensorReading::new(SensorType::ContactInput, vec![1.0, 0.0, 1.0], vec![]),
            None, None,
        ).unwrap();
        assert_eq!(items.len(), 3, "サンプルごとにitem分割");
        assert!(items.iter().all(|i| i.channel_index.is_none()),
            "channel_index=None(サンプル番号のチャネル捏造禁止)");
        assert_eq!(items[0].values, vec![1.0]);
        assert_eq!(items[1].values, vec![0.0]);
        assert!(items.iter().all(|i| i.measurement_key == "contact_state"));
    }

    #[test]
    fn unknown_sensor_type_and_foreign_key_form_return_none() {
        assert!(to_items(
            &DeviceKey::new("bravepi-mainboard:aa:x"),
            &SensorReading::new(SensorType::Unknown("mystery".into()), vec![1.0], vec![]),
            None, None,
        ).is_none());
        assert!(to_items(
            &DeviceKey::new("i2c:0x44:illuminance"),
            &SensorReading::new(SensorType::Illuminance, vec![512.0], vec!["lux".into()]),
            None, None,
        ).is_none(), "非BravePI形式キーはこの写像の担当外");
    }
}
```

- [ ] **Step 2: 失敗確認 → 実装 → 成功確認**

Run: `CARGO_NET_OFFLINE=true cargo test -p bravepi-mainboard-adapter`(FAIL→PASS)

- [ ] **Step 3: event_loop注入(失敗するテスト→実装)**

`handle.rs` の `start` に `ingest: Option<iotkit_ingest_client::IngestClient>` を追加し、`event_loop` まで引き渡す(既存呼び出し元: gateway `start_bravepi`——Task 5で更新するため、本タスクでは`None`を渡す一時行でコンパイルを保つ。実装者はhandle.rs実物のシグネチャに合わせる)。

`event_loop.rs` のSensorData送出点(`if let AdapterEvent::SensorData { .. }` 分岐)で、AdapterEvent送出の**前**に:

```rust
                                // 取り込み経路(D4: アダプタ内クライアント)。AdapterEventは
                                // engine/監督用に従来どおり並走(frozen vocabulary)
                                if let Some(client) = &ingest {
                                    if let AdapterEvent::SensorData {
                                        ref device_key, ref reading, rssi, battery_pct, ..
                                    } = event {
                                        match super::ingest_map::to_items(device_key, reading, rssi, battery_pct) {
                                            Some(items) => {
                                                let e = iotkit_ingest_client::new_envelope(
                                                    adapter_id.as_str(), items);
                                                if client.try_submit(e).is_err() {
                                                    tracing::warn!("ingest queue full; dropping reading");
                                                }
                                            }
                                            None => tracing::warn!(
                                                device_key = %device_key,
                                                "no measurement mapping; reading not ingested"),
                                        }
                                    }
                                }
```

(`adapter_id` がevent_loopスコープに無い場合はhandle.rsから引き渡す——実装者は実物を確認。`Envelope.source` は実在形式 `bravepi-mainboard:{port_path}`。)

**既存呼び出し元の同一タスク内追従**(workspaceを壊さない):
- `iotkit-gateway/src/main.rs` の `start_bravepi` 内 `task::start(port.to_string())` → `task::start(port.to_string(), None)`(Task 5でSomeに変わる)
- `bravepi-mainboard-adapter/poc/src/main.rs` の `task::start(...)` → 末尾に `None` 追加

`event_loop_test.rs` に追加: 既存のテストハーネス(注入チャネル)を使い、`channel_for_test` のclientを渡してセンサーフレーム1件からEnvelopeが捕捉されること・`source`と`subject_hint`("ble:...")の形式・Unknown型でEnvelopeが出ないことを検証する(3アサート程度。既存テストの流儀に従う)。

- [ ] **Step 4: 成功確認+Commit**

Run: `CARGO_NET_OFFLINE=true cargo test -p bravepi-mainboard-adapter && CARGO_NET_OFFLINE=true cargo build -p iotkit-gateway -p bravepi-poc`(pocのパッケージ実名はCargo.tomlで確認)
Expected: PASS / ビルド成功

```bash
git add bravepi-mainboard-adapter iotkit-gateway
git commit -m "feat(bravepi): in-adapter ingest client with declared unit correspondence table (D4)"
```

---

### Task 4: polling runtime写像+注入+rpi-local貫通

**Files:**
- Create: `iotkit-polling-adapter-runtime/src/ingest_map.rs`
- Modify: `iotkit-polling-adapter-runtime/src/lib.rs`(`mod ingest_map;`+`start`署名変更)
- Modify: `iotkit-polling-adapter-runtime/src/polling_loop.rs`(署名変更+イベント送出側での並走送出)
- Modify: `iotkit-polling-adapter-runtime/Cargo.toml`(`iotkit-ingest-client` と **`iotkit-ingest-contract`** の依存追加)
- Modify: `rpi-local-adapter/src/lib.rs`(`start(config, ingest)`素通し)
- Modify: `rpi-local-adapter/tests/integration.rs`(`start(config)` 呼び出しへの `None` 追加——コンパイル対象)
- Modify: `iotkit-gateway/src/main.rs`(`start_rpi_local` 内の呼び出しに `None` 追加——Task 5でSomeに)

**Interfaces:**
- Consumes: Task 1の `IngestClient` / `new_envelope` / `channel_for_test`
- Produces:
  - `iotkit_polling_adapter_runtime::start(id: AdapterId, config: PollingAdapterConfig, ingest: Option<IngestClient>)`(**PollingAdapterConfigは無変更**——構築箇所の波及を避け、ingestは引数で通す)
  - `polling_loop(adapter_id: AdapterId, ingest: Option<IngestClient>, config, event_tx, command_rx)`(内部)
  - `rpi_local_adapter::start(config: RpiLocalConfig, ingest: Option<IngestClient>)`
  - `ingest_map::to_items(adapter_id, device_key, reading) -> Option<Vec<ReadingItem>>`(**device_key基準**——SensorData構築は純関数 `apply_outcomes` 内のため、注入はイベント送出側で行いapply_outcomes/PollOutcomeは無改修とする)

**単位対応表(polling系。モジュール冒頭に逐語掲載)**:

```text
| SensorType(ドライバ)   | ドライバ出力 | 変換 | measurement_key(D6) | channel_index |
|------------------------|-------------|------|----------------------|---------------|
| Temperature (mcp9600)  | ℃          | ×1   | temperature_c        | None          |
| Illuminance (opt3001)  | lux         | ×1   | illuminance_lux      | None          |
| その他(将来ドライバ)  | 対応表未宣言の型は送出しない(warnログ)——表の更新を強制する |
```

- [ ] **Step 1: 失敗するテストを書く(ingest_map単体)**

`ingest_map.rs`(位置識別型hardware_id=送信者スコープ付き=D5決定2。DeviceKey `"i2c:0x{addr:02x}:{suffix}"`(`polling_loop.rs:94-95` の `device_key_for` が生成)から導出する——旧bridgeの `i2c_key_maps_to_sender_scoped_positional_hardware_id` テストを移植):

```rust
//! measurement写像(polling系)。(冒頭に上記単位対応表を逐語掲載)
use iotkit_core_types::{AdapterId, DeviceKey, SensorReading, SensorType};
use iotkit_ingest_contract::{ReadingItem, TimeSource};

fn measurement_key_for(sensor_type: &SensorType) -> Option<&'static str> {
    Some(match sensor_type {
        SensorType::Temperature => "temperature_c",
        SensorType::Illuminance => "illuminance_lux",
        _ => return None, // 対応表未宣言の型は送出しない(表の更新を強制)
    })
}

/// DeviceKey "i2c:0x{addr:02x}:{suffix}" → 位置識別型hardware_id(送信者スコープ付き=D5決定2)。
pub(crate) fn to_items(
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    reading: &SensorReading,
) -> Option<Vec<ReadingItem>> {
    let key = measurement_key_for(&reading.sensor_type)?;
    let parts: Vec<&str> = device_key.as_str().split(':').collect();
    let hw = match parts.as_slice() {
        ["i2c", addr, _suffix] => format!("{}:i2c:{addr}", adapter_id.as_str()),
        _ => return None,
    };
    Some(vec![ReadingItem {
        subject_hint: Some(hw),
        measurement_key: key.to_string(),
        channel_index: None,
        series_variant: None,
        values: reading.values.clone(),
        device_time_ms: None,
        time_source: TimeSource::Gateway,
        age_ms: None, rssi: None, battery_pct: None,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2c_key_maps_to_sender_scoped_positional_hardware_id() {
        let items = to_items(
            &AdapterId::new("rpi-local:default"),
            &DeviceKey::new("i2c:0x44:illuminance"),
            &SensorReading::new(SensorType::Illuminance, vec![512.0], vec!["lux".into()]),
        ).unwrap();
        assert_eq!(items[0].subject_hint.as_deref(), Some("rpi-local:default:i2c:0x44"));
        assert_eq!(items[0].measurement_key, "illuminance_lux");
        assert_eq!(items[0].channel_index, None);
    }

    #[test]
    fn undeclared_sensor_type_and_foreign_key_form_are_not_emitted() {
        assert!(to_items(
            &AdapterId::new("rpi-local:default"),
            &DeviceKey::new("i2c:0x29:ranging"),
            &SensorReading::new(SensorType::Ranging, vec![100.0], vec![]),
        ).is_none(), "対応表未宣言の型は送出しない(単位対応表の更新を強制)");
        assert!(to_items(
            &AdapterId::new("rpi-local:default"),
            &DeviceKey::new("bravepi-mainboard:aa:temperature"),
            &SensorReading::new(SensorType::Temperature, vec![21.5], vec![]),
        ).is_none(), "非polling形式キーはこの写像の担当外");
    }
}
```

- [ ] **Step 2: 注入(失敗するテスト→実装)**

`lib.rs`: `start` の署名を `pub fn start(id: AdapterId, config: PollingAdapterConfig, ingest: Option<iotkit_ingest_client::IngestClient>)` に変更(**PollingAdapterConfigは無変更**——構築箇所への波及を避ける)。`polling_loop` 呼び出しを `polling_loop(id.clone(), ingest, config, event_tx, command_rx)` に(`AdapterHandle.id` 用のidは従来どおり保持)。

`polling_loop.rs`: 署名に `adapter_id: AdapterId, ingest: Option<IngestClient>` を追加。**SensorData構築点(`apply_outcomes`=純関数)は無改修**とし、`apply_outcomes` の戻り値イベント列を `event_tx` へ送る箇所(実装者が実物で特定する。複数箇所あれば送出ヘルパに集約してよい)で送出直前に割り込む:

```rust
                for event in events {
                    // 取り込み経路(D4: アダプタ内クライアント)。AdapterEventはengine/監督用に並走
                    if let (Some(client), AdapterEvent::SensorData { device_key, reading, .. }) =
                        (&ingest, &event)
                    {
                        match crate::ingest_map::to_items(&adapter_id, device_key, reading) {
                            Some(items) => {
                                let e = iotkit_ingest_client::new_envelope(
                                    adapter_id.as_str(), items);
                                if client.try_submit(e).is_err() {
                                    tracing::warn!("ingest queue full; dropping reading");
                                }
                            }
                            None => tracing::warn!(device_key = device_key.as_str(),
                                "no measurement mapping; reading not ingested"),
                        }
                    }
                    // (既存のevent_tx.send(event)処理をここに続ける——実物の送出コードに合わせる)
                }
```

`rpi-local-adapter/src/lib.rs`: `pub fn start(config: RpiLocalConfig, ingest: Option<IngestClient>)` にし、runtimeの `start(id, polling_config, ingest)` へ素通し。既存呼び出し元を同一タスク内で追従:
- `rpi-local-adapter/src/lib.rs` 内テストの `start(config)` → `start(config, None)`
- `rpi-local-adapter/tests/integration.rs` の `start(config)` **全2箇所**(:21 と :59)→ `start(config, None)`
- `iotkit-gateway/src/main.rs` の `start_rpi_local` 内 `rpi_local_adapter::start(adapter_config)` → `rpi_local_adapter::start(adapter_config, None)`(Task 5でSomeに)

テスト: ingest_map単体テスト(上記)+既存テストのコンパイル追従。ループ実体のE2Eは実バス依存のため対象外(既存の実I2Cテストと同じ扱い。写像単体+Task 1のクライアントE2E+コレクタE2Eで鎖は覆われている)。

- [ ] **Step 3: 成功確認+Commit**

Run: `CARGO_NET_OFFLINE=true cargo test -p iotkit-polling-adapter-runtime -p rpi-local-adapter && CARGO_NET_OFFLINE=true cargo build -p iotkit-gateway`
Expected: PASS / ビルド成功

```bash
git add iotkit-polling-adapter-runtime rpi-local-adapter iotkit-gateway
git commit -m "feat(polling-runtime): in-adapter ingest with positional hardware_id mapping (D4/D5)"
```

---

### Task 5: ゲートウェイ配線+bridge.rs削除+全体テスト

**Files:**
- Modify: `iotkit-gateway/src/main.rs`
- Delete: `iotkit-gateway/src/bridge.rs`
- Modify: `iotkit-gateway/Cargo.toml`(iotkit-ingest-client依存追加)
- Modify: `CLAUDE.md`(移行注記の更新)

**Interfaces:**
- Consumes: Task 1〜4の全成果物

- [ ] **Step 1: main.rs配線**

1. `mod bridge;` を削除。
2. コレクタspawn直後にクライアントspawn:

```rust
    // 取り込みクライアント(D4の第3部品、inproc)。アダプタが直接Envelopeを送る。
    // AdapterEventはengine/監督用のfrozen vocabularyとして並走(D4)。
    let (ingest_client, ingest_client_handle) = iotkit_ingest_client::spawn_inproc(
        collector.clone(),
        iotkit_ingest_client::DEFAULT_QUEUE_CAP,
        iotkit_ingest_client::DEFAULT_SPOOL_CAP,
    );
```

3. `start_bravepi` / `start_rpi_local` のヘルパ署名に `ingest: Option<IngestClient>` を追加し、**初回起動と再起動の両呼び出し箇所**を更新する:
   - 初回起動(config分岐内): `start_bravepi(&mut host, &bp.port, Some(ingest_client.clone()))` 等
   - **再起動アーム**(fan-inループのAdapterClosed分岐、`RestartSpec::BravePi{port}` / `RestartSpec::RpiLocal{config}` のmatch): 同様に `Some(ingest_client.clone())` を渡す(`ingest_client` は `run()` スコープに生存しておりクローン可能。RestartSpec自体は無変更)
   - 再起動後アダプタのingest保持は実行ハーネスが無いため、**両起動経路が同一引数を渡すことをタスクレビューでdiff上検証する**(watchpoint「再起動・再登録経路の整合」の検証条件として報告書に明記)
4. fan-inループのSensorData分岐から `bridge::adapter_event_to_envelope`〜`collector.submit` のブロックを削除(`note_healthy` と `engine.apply` は維持)。`collector` 変数はクライアントに渡した後fan-inでは未使用になる——所有権を整理。
5. selectループに腕を追加:

```rust
            _ = &mut ingest_client_pinned => {
                // クライアントタスク退出=コレクタ死亡(Closed)。取り込み全損なのでfail-fast
                tracing::error!("ingest client exited (collector closed); aborting fan-in loop");
                collector_alive = false;
                break;
            }
```

(`let mut ingest_client_pinned = ingest_client_handle;` をループ前に。JoinHandleは`&mut`でpoll可能。実装者はselect構文の実物に合わせる。)
6. 旧 `SubmitError` 分岐(NoAck/Closed)はfan-inから消える(クライアント内へ移動済み)。

- [ ] **Step 2: bridge.rs削除と参照掃引**

```bash
git rm iotkit-gateway/src/bridge.rs
grep -rn "bridge" iotkit-gateway/src/ && echo "参照残存" || echo "clean"
```

bridge.rsのE2Eテスト(`bridge_output_flows_through_collector_to_readings`)の役割はTask 1のクライアントE2E+ingest_map単体テストが引き継ぐ(削除で失われる検証がないことを実装者が確認し、報告書に明記)。

bridge削除でgatewayの `uuid` 依存が未使用になる場合は `iotkit-gateway/Cargo.toml` から除去する(`cargo build -p iotkit-gateway` で未使用を確認してから)。

- [ ] **Step 3: CLAUDE.md更新**

「移行期間中、旧語彙(AdapterEvent)と新契約(Envelope)の変換はゲートウェイ内ブリッジ1ファイルに限定。新規コードはAdapterEventへの依存を増やさない(D4)。」を:

```markdown
- 取り込み経路はアダプタ内クライアント(iotkit-ingest-client)が正(D4)。旧語彙(AdapterEvent)は
  engine/監督専用のfrozen vocabulary——新規コードは依存を増やさない。ブリッジは削除済み(計画3)。
```

- [ ] **Step 4: 全体テスト+Commit**

Run: `RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test --workspace`
Expected: 全スイートPASS

```bash
git add -A
git commit -m "feat(gateway): wire in-adapter ingest client; delete the transitional bridge (D4)"
```

---

## レビュー裁定記録(2026-07-03、Fable+codex並行計画レビュー)

計画初版に対しFable+codex(xhigh)を同時実施。**全12指摘採用(棄却ゼロ)**、本版に反映済み。
両者が独立に3つの急所(contact多値・polling_loop構造不一致・spool飢餓)に到達した。

| 指摘 | 出典 | 反映 |
|---|---|---|
| 単位対応表のcontact誤記——実出力はdata_count分の**時系列サンプル**。汎用len>1分割はサンプル番号をチャネル化する意味の捏造(現行bridgeが現にこの誤モデル化をしている) | Fable BLOCKER + codex MAJOR | 分割規約をSensorTypeごとに宣言(表改訂+to_items型別分岐+contactテスト追加) |
| spool/再送設計の飢餓——NoAck再送ループ中に入力を吸い上げずdrop-oldestが機能しない。ジッタ欠落(D1 MUST) | codex BLOCKER + Fable MAJOR | select式(バックオフ待機中も排出継続)+envelope_id由来の決定的ジッタに再設計 |
| polling_loop注入スニペットが実物構造と不一致(SensorData構築はapply_outcomes内・configは分解済み) | codex BLOCKER + Fable MAJOR | イベント送出側でのdevice_key基準写像に変更。apply_outcomes/PollOutcode無改修 |
| start署名変更のタスク内コンパイル破壊(gateway/poc/rpi-local integration test) | codex MAJOR + Fable MAJOR | 各タスクのFilesに呼び出し元を追加、同一タスク内でNone追従 |
| ingest_map系のcontract依存欠落 | Fable MAJOR | 両Cargo.tomlに`iotkit-ingest-contract`追加 |
| Task 2のbridge同時修正指示の自己矛盾 | Fable MAJOR | 指示書き直し(分岐削除+テスト2本の具体的処置) |
| 再起動経路の具体化不足(watchpoint) | codex MAJOR + Fable MINOR | 両起動経路への引数指定+レビューでのdiff検証条件を明記 |
| Deferred=完了扱いはD1と逆 | Fable MINOR | 不変再試行に変更(inproc到達不能だが将来共用) |
| overflowテストのタイミング脆弱性 | codex + Fable MINOR | 全量投入→解除の決定的手順+「最新が生き残る」assertへ |
| to_items署名のInterfaces/コード不一致 | codex + Fable MINOR | 統一 |
| gatewayのuuid依存の掃除漏れ | Fable MINOR | Task 5に追加 |

**プロセス教訓**: 今回の計画初版はpolling_loop/contactの実物精読を省いてスニペットを書き、両レビューに刺された。計画1・2で守れていた「スニペットを書く対象は精読が前提」を計画テンプレの規律として維持する。

## Self-Review記録(計画作成時)

- 設計追補の全掃引(CLAUDE.md新規律の初適用): D1監査追記(NoAck/Closed・非有限)、D4監査追記(クランプ禁止・単位対応表義務)、D5/D6監査追記(正準チャネル)をGlobal Constraintsに反映済み。D1「Rust実装の地雷」のlabels型(消化済み)・adapter_id出所(エンベロープ自己記述=new_envelopeのsource)・ackタイムアウト(HTTP用、inproc対象外)を確認済み。
- 単位対応表は計画作成者が全8ドライバの実物出力を確認して記載(LIS2DUXS12の往復変換とmcp3427のラベル偽装を発見・是正をタスク化)。
- 持ち越しの消化: 「ブリッジno-ack経路→計画3」(計画1由来)は本計画のspool+再送で解消。「T8監督タイマーメッセージ化」は計画4のまま。
- 各タスクは独立してコンパイル・テスト可能(Task 3/4の`None`一時配線でTask 5まで両立)。
- E2E鎖: ドライバ→写像(単体テスト)→クライアント(E2E)→コレクタ(計画2 E2E)——実ハードなしで検証可能な最長の鎖。
