# Wave 1 出口契約 MVE（R10 歩く骨格）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** アーカイブ責任消費者1台へ measurement レコードストリームを外向き HTTP push で at-least-once 配送し、その ack で正本(readings)のパージを許可する end-to-end custody ループの最小実装。

**Architecture:** 新クレート `core/publish`（publication_log[outbox] + target_registry）+ `iotkit-gateway` 常駐 push タスク + `iotkit-gatewayctl target` CLI。collector が非検疫 measurement を reading 挿入と同一 Immediate Tx で outbox に enqueue（別採番 `pub_seq`）。push タスクが `(epoch, pub_seq)` カーソル以降を有界バッチで HTTPS POST→同期 ack→cursor 前進。ack 済み水位を R17 retention のパージ判定へ配線。

**Tech Stack:** Rust / tokio / rusqlite(SQLite WAL, bundled) / `reqwest`（gateway=`default-features=false, features=["json","rustls-tls"]`（async）、gatewayctl=`default-features=false, features=["blocking","json","rustls-tls"]`。spec §9 に一致）。

**設計正本:** [spec](../specs/2026-07-05-wave1-exit-contract-mve-design.md)（codex×3+Sonnet×3 の5周並行レビューで収束、HEAD 0fca885）。各タスクは spec の §番号を参照する。

## Global Constraints

すべてのタスクに暗黙に含まれる。値は spec/recon から逐語。

- **新規 migration は version 10 から**。version 2 は retire 済みで再利用不可（`_schema_version` 差集合が壊れる）。`run_migrations` は strictly-ascending のみ要求、gap は上方向のみ可。`Migration { version: u32, label: &'static str, sql: include_str!("...") }`（`core/storage/src/migrate.rs:1`）。
- **`core/publish` の MIGRATIONS を production concat 2箇所へ追加**: `iotkit-gateway/src/main.rs:57-61` と `iotkit-gatewayctl/src/main.rs:127-131`（どちらも storage+ledger+timeseries+registry を concat し `.sort_by_key(|m| m.version)`）。publish を足して再ソート。
- **カーソル同一性は必ず `(epoch, pub_seq)` の複合**（spec §4.1）。pub_seq 単独で ack/配送/パージ判定をしない。「ack 済み」= `publication_log.epoch == target.cursor_epoch == current_epoch` かつ `pub_seq <= target.cursor_pub_seq`。epoch 不一致・`cursor_epoch` NULL は effective cursor=0。
- **retention 保護は pub_seq 付き未ack非検疫行のみ**（spec §8.2）。検疫行（quarantined=1、pub_seq 無し）・enqueue されなかった行は floor で消す。これは**新規 readings purge branch**（旧 `purge_readings_before` 置換）。デバイス検疫期限失効（`expire_quarantined_devices`=`devices.state` のみ）は readings を消さない。
- **HTTP POST は必ず `with_conn`/`with_conn_sync` の外**（spec §6.2、DB は単一 `Arc<Mutex<Connection>>` `core/storage/src/handle.rs:16`）。push は3スコープ [A]lock内 read+build → [B]lock外 POST/ack → [C]lock内 cursor 前進。
- **`endpoint_url` は `https://` のみ**。per-target bearer token は `Authorization: Bearer`。token は秘密で R22 snapshot に含めない（spec §12）。
- **reqwest**: `iotkit-gateway` に async（`default-features=false, features=["json","rustls-tls"]`）、`iotkit-gatewayctl` に blocking（`default-features=false, features=["blocking","json","rustls-tls"]`）。inline 宣言（workspace.dependencies 不使用）。両者とも default-features=false（spec §9）。
- **check-then-mutate は単一 `Transaction::new_unchecked(conn, TransactionBehavior::Immediate)`**（TOCTOU 防止、spec §3.3/§8.3。gatewayctl は gateway と別プロセスで in-proc Mutex は跨がない）。gatewayctl の変異は `mutate()` ヘルパ（`cmd/devices.rs:82`、Immediate Tx+`bump_generation`+commit）。
- **最低保持フロア既定 72h**（spec §8.2、[D1:155]）。有界バッチ = 件数上限 N + byte cap（先に達した方、**ただし単一超過でも最低1件**、spec §6.2）。
- **決定的 publication_id = `hash(target_id, current_epoch, cursor_start, cursor_end)`**（spec §10）。バッチ = `WHERE epoch=current_epoch AND pub_seq>cursor ORDER BY pub_seq ASC LIMIT N`、cursor 排他。
- **検疫遷移ガードは両方向 hard-reject**（spec §9）: archive_responsible target 登録中は §9.1 検疫解除（`define_alias` 経由）と §9.2 遡及検疫（`replace-undo`）を override 無しで拒否。
- **annotation は `epoch_start` のみ**（spec §5.2）。partial `UNIQUE(epoch, subtype) WHERE kind='annotation'` で二重 enqueue を排除。collector spawn 前に enqueue（最小 pub_seq 保証）。
- **series_key**（spec §7、[D5:27]）= `format!("{}:{}:{}:{}", system_id.to_text(), measurement_key, ch, variant)`、`ch = if channel_index==ledger::CHANNEL_NA {"na"} else {channel_index}`（precedent `cmd/replace.rs:158`）。

---

## File Structure

- **`core/publish/`**（新規クレート）
  - `Cargo.toml` — deps: `iotkit-core-storage`, `iotkit-core-ledger`, `iotkit-core-timeseries`, `rusqlite`(bundled), `serde`, `serde_json`, `thiserror`, `uuid`(v7), `tracing`; dev: `tempfile`.
  - `migrations/0010_publish.sql` — `publication_log` + `target_registry` + partial UNIQUE index。
  - `src/lib.rs` — `pub const MIGRATIONS`, re-export store, `PublishError`。
  - `src/store.rs` — outbox writes/batch/prune、target CRUD、cursor、custody 判定クエリ。
- **`core/collector/src/actor.rs`**（修正）— `process_item` の insert 後に enqueue（seq 捕捉）。
- **`iotkit-gateway/src/publish_task.rs`**（新規）— push 常駐タスク。
- **`iotkit-gateway/src/record.rs`**（新規）— measurement/annotation JSON 型 + 実体化。
- **`iotkit-gateway/src/main.rs`**（修正）— migration concat、epoch_start trigger、push task spawn。
- **`iotkit-gateway/src/retention.rs`**（修正）— custody 対応パージへ作り替え。
- **`iotkit-gateway/src/health.rs`**（修正）— per-target 配送状態。
- **`iotkit-gateway/Cargo.toml`**（修正）— reqwest async、core-publish。
- **`iotkit-gatewayctl/src/cmd/target.rs`**（新規）+ `main.rs`（修正）— target CLI。
- **`iotkit-gatewayctl/src/cmd/replace.rs`**（修正）— §9.2 ガード + outbox prune。
- **`iotkit-gatewayctl/Cargo.toml`**（修正）— reqwest blocking、core-publish。
- **`core/registry/src/store.rs`**（修正）— §9.1 ガード。
- **`core/timeseries/migrations/0004_readings_v3.sql`**（修正）— stale コメント。
- **`Cargo.toml`**（root、修正）— members に `core/publish`。

---

## Task 1: `core/publish` クレート + migration 0010

**Files:**
- Create: `core/publish/Cargo.toml`, `core/publish/src/lib.rs`, `core/publish/migrations/0010_publish.sql`
- Modify: `Cargo.toml`（root members +`"core/publish"`）, `iotkit-gateway/src/main.rs:57-61`, `iotkit-gatewayctl/src/main.rs:127-131`, `iotkit-gateway/Cargo.toml`, `iotkit-gatewayctl/Cargo.toml`（core-publish dep）
- Test: `core/publish/src/lib.rs`（`#[cfg(test)]`）

**Interfaces:**
- Produces: `iotkit_core_publish::MIGRATIONS: &[Migration]`（version 10）, `iotkit_core_publish::PublishError`。
- Consumes: `iotkit_core_storage::{Migration, run_migrations}`（migrate.rs）。

**migration SQL**（spec §4.1/§4.2、`row_quarantined==false` のみ measurement enqueue、annotation は inline）:

- [ ] **Step 1: migration SQL を書く** — `core/publish/migrations/0010_publish.sql`:

```sql
-- 出口 publication log(outbox)。pub_seq は readings.seq と別採番(D7決定4)
CREATE TABLE publication_log (
    pub_seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    epoch           TEXT    NOT NULL,
    kind            TEXT    NOT NULL,              -- 'measurement' | 'annotation'
    subtype         TEXT,                          -- annotation: 'epoch_start'。measurement は NULL
    reading_seq     INTEGER,                       -- measurement: readings.seq 参照。annotation は NULL
    annotation_json TEXT,                          -- annotation: inline payload。measurement は NULL
    created_at      INTEGER NOT NULL
);
-- epoch_start の二重 enqueue を DB 制約で排除(spec §5.2/§8 冪等)
CREATE UNIQUE INDEX ux_publog_annotation_epoch
    ON publication_log(epoch, subtype) WHERE kind = 'annotation';
-- retention/push の batch/prune 用
CREATE INDEX ix_publog_epoch_seq ON publication_log(epoch, pub_seq);
CREATE INDEX ix_publog_reading   ON publication_log(reading_seq) WHERE reading_seq IS NOT NULL;

-- 出口配送先。MVE は1行のみ運用(spec §4.2)
CREATE TABLE target_registry (
    target_id           TEXT PRIMARY KEY,
    endpoint_url        TEXT NOT NULL,             -- https:// のみ(§11で強制)
    credential_token    TEXT NOT NULL,             -- per-target bearer。秘密(snapshot 非含有)
    archive_responsible INTEGER NOT NULL DEFAULT 0,
    schema_version      INTEGER NOT NULL,
    cursor_epoch        TEXT,                      -- 最後に ack された epoch(初期 NULL)
    cursor_pub_seq      INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL
);
```

- [ ] **Step 2: `core/publish/Cargo.toml`** — 既存 `core/ledger/Cargo.toml` の dep スタイルを踏襲:

```toml
[package]
name = "iotkit-core-publish"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-core-storage = { path = "../storage" }
iotkit-core-ledger = { path = "../ledger" }
iotkit-core-timeseries = { path = "../timeseries" }
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
uuid = { version = "1", features = ["v7"] }
tracing = "0.1"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: `core/publish/src/lib.rs` の骨格 + 失敗テスト** — MIGRATIONS と、DB 適用でテーブルが出来ることを検証:

```rust
use iotkit_core_storage::Migration;

pub mod store;

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("ledger: {0}")]
    Ledger(String),
    #[error("invalid: {0}")]
    Invalid(String),
}

pub const MIGRATIONS: &[Migration] = &[
    Migration { version: 10, label: "publish", sql: include_str!("../migrations/0010_publish.sql") },
];

#[cfg(test)]
mod tests {
    use super::*;
    fn open() -> rusqlite::Connection {
        let mut all: Vec<Migration> = Vec::new();
        all.extend_from_slice(iotkit_core_storage::MIGRATIONS);
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
        all.extend_from_slice(MIGRATIONS);
        all.sort_by_key(|m| m.version);
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        iotkit_core_storage::run_migrations(&conn, &all).unwrap();
        conn
    }
    #[test]
    fn migration_creates_tables() {
        let conn = open();
        let n: i64 = conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('publication_log','target_registry')",
            [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
    }
}
```

> 注: `run_migrations` の正確なシグネチャは `core/storage/src/migrate.rs` を確認して合わせる（`&Connection, &[Migration]`）。`store` モジュールは Task 2 で実装するので、Step 3 時点では `pub mod store;` が空（`core/publish/src/store.rs` に空ファイル）でコンパイルが通る形にする。

- [ ] **Step 4: workspace members + concat 追加** — root `Cargo.toml` の members に `"core/publish"`。`iotkit-gateway/Cargo.toml` と `iotkit-gatewayctl/Cargo.toml` の `[dependencies]` に `iotkit-core-publish = { path = "../core/publish" }`（gateway 側は `../core/publish`、gatewayctl 側も同様に相対）。`iotkit-gateway/src/main.rs:57-61` と `iotkit-gatewayctl/src/main.rs:127-131` の concat に `all.extend_from_slice(iotkit_core_publish::MIGRATIONS);` を追加（`.sort_by_key` の前）。

- [ ] **Step 5: テスト実行**

Run: `cargo test -p iotkit-core-publish migration_creates_tables`
Expected: PASS。

Run: `cargo build -p iotkit-gateway -p iotkit-gatewayctl`
Expected: コンパイル成功（concat と dep 追加が通る）。

- [ ] **Step 6: Commit**

```bash
git add core/publish Cargo.toml iotkit-gateway/Cargo.toml iotkit-gatewayctl/Cargo.toml iotkit-gateway/src/main.rs iotkit-gatewayctl/src/main.rs
git commit -m "feat(publish): core/publish crate with outbox+target migration (v10)"
```

---

## Task 2: `core/publish` store（outbox・target・cursor・batch・prune）

**Files:**
- Modify: `core/publish/src/store.rs`, `core/publish/src/lib.rs`（tests）
- Test: `core/publish/src/store.rs`（`#[cfg(test)]`）

**Interfaces（Produces — 後続タスクが依存する正確なシグネチャ）:**
```rust
// 全て `conn: &rusqlite::Connection`（Tx でも &*tx で渡せる）。now_ms は呼び出し側が渡す(テスト容易化)。
pub struct OutboxRow { pub pub_seq: i64, pub epoch: String, pub kind: String,
    pub subtype: Option<String>, pub reading_seq: Option<i64>, pub annotation_json: Option<String> }
pub struct TargetRow { pub target_id: String, pub endpoint_url: String, pub credential_token: String,
    pub archive_responsible: bool, pub schema_version: i64,
    pub cursor_epoch: Option<String>, pub cursor_pub_seq: i64 }

pub fn enqueue_measurement(conn: &Connection, epoch: &str, reading_seq: i64, now_ms: i64) -> Result<i64, PublishError>; // returns pub_seq
pub fn enqueue_annotation(conn: &Connection, epoch: &str, subtype: &str, payload_json: &str, now_ms: i64) -> Result<Option<i64>, PublishError>; // None if UNIQUE 衝突(既存)
pub fn select_batch(conn: &Connection, epoch: &str, after_pub_seq: i64, limit: u32) -> Result<Vec<OutboxRow>, PublishError>; // WHERE epoch=?1 AND pub_seq>?2 ORDER BY pub_seq ASC LIMIT ?3
pub fn prune_outbox_by_reading_seqs(conn: &Connection, reading_seqs: &[i64]) -> Result<u64, PublishError>; // 遡及検疫用(§9.2)
pub fn prune_acked_outbox(conn: &Connection, epoch: &str, upto_pub_seq: i64) -> Result<u64, PublishError>;

pub fn target_insert(conn: &Connection, t: &TargetRow, now_ms: i64) -> Result<(), PublishError>;
pub fn target_get(conn: &Connection) -> Result<Option<TargetRow>, PublishError>; // MVE 単一 target
pub fn target_count(conn: &Connection) -> Result<i64, PublishError>;
pub fn target_delete(conn: &Connection, target_id: &str) -> Result<(), PublishError>;
pub fn target_set_token(conn: &Connection, target_id: &str, token: &str) -> Result<(), PublishError>;
pub fn target_set_archive_responsible(conn: &Connection, target_id: &str, on: bool) -> Result<(), PublishError>;
pub fn target_advance_cursor(conn: &Connection, target_id: &str, epoch: &str, pub_seq: i64) -> Result<(), PublishError>;

// custody 判定(§9/§retention)
pub fn has_unacked_pubseq_rows(conn: &Connection, current_epoch: &str, target: &TargetRow, reading_seqs: &[i64]) -> Result<bool, PublishError>;
pub fn archive_target_registered(conn: &Connection) -> Result<bool, PublishError>; // target_count>=1 && archive_responsible
pub fn any_unacked_for_target(conn: &Connection, current_epoch: &str, target: &TargetRow) -> Result<bool, PublishError>; // outbox に epoch=current かつ pub_seq>effective_cursor が1つでもあるか(remove ガード用、§11)
```

- [ ] **Step 1: enqueue+select+prune の失敗テスト**（`store.rs` `#[cfg(test)]`）:

```rust
#[test]
fn enqueue_and_select_batch_is_ordered_and_exclusive() {
    let conn = super::tests_support::open(); // Task 1 の open() を store test からも使えるよう pub(crate) 化
    let e = "epoch-A";
    // 3件 enqueue → pub_seq 昇順・排他カーソル
    let s1 = enqueue_measurement(&conn, e, 100, 1).unwrap();
    let s2 = enqueue_measurement(&conn, e, 101, 2).unwrap();
    let _s3 = enqueue_measurement(&conn, e, 102, 3).unwrap();
    assert!(s2 > s1);
    let batch = select_batch(&conn, e, s1, 10).unwrap(); // s1 排他 → s2,s3
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].pub_seq, s2);
    assert_eq!(batch[0].reading_seq, Some(101));
}
#[test]
fn enqueue_annotation_idempotent_on_epoch_subtype() {
    let conn = super::tests_support::open();
    let a = enqueue_annotation(&conn, "epoch-A", "epoch_start", "{}", 1).unwrap();
    assert!(a.is_some());
    let b = enqueue_annotation(&conn, "epoch-A", "epoch_start", "{}", 2).unwrap();
    assert!(b.is_none(), "二重 enqueue は UNIQUE で None");
}
```

- [ ] **Step 2: テストが fail することを確認** — Run: `cargo test -p iotkit-core-publish -- store::tests`  Expected: FAIL（関数未定義）。

- [ ] **Step 3: store.rs 実装** — 上記シグネチャを rusqlite で実装。要点:
  - `enqueue_measurement`: `INSERT INTO publication_log(epoch,kind,reading_seq,created_at) VALUES(?1,'measurement',?2,?3)` → `conn.last_insert_rowid()`。
  - `enqueue_annotation`: `INSERT ... kind='annotation', subtype=?2, annotation_json=?3 ...`。UNIQUE 衝突（`rusqlite::Error::SqliteFailure` code `ConstraintViolation`）は `Ok(None)`、それ以外は `?`。成功時 `Ok(Some(last_insert_rowid))`。
  - `select_batch`: 上記 WHERE/ORDER/LIMIT。行を `OutboxRow` に map。
  - `prune_outbox_by_reading_seqs` / `prune_acked_outbox`: `DELETE`（前者は `reading_seq IN (...)`、後者は `epoch=?1 AND pub_seq<=?2`）。
  - target 系: 単純 CRUD。`target_get` は `SELECT ... LIMIT 1`。`archive_target_registered` = `SELECT count(*) FROM target_registry WHERE archive_responsible=1` > 0。
  - `has_unacked_pubseq_rows`: reading_seqs のうち、対応 outbox 行が「epoch=current かつ (target.cursor_epoch!=current OR pub_seq>cursor_pub_seq)」＝未ack、が1つでもあれば true。effective cursor は `if target.cursor_epoch.as_deref()==Some(current_epoch) {cursor_pub_seq} else {0}`。

  > テスト補助: `tests_support::open()` を `core/publish/src/lib.rs` に `pub(crate) mod tests_support { pub fn open() -> rusqlite::Connection {...} }`（Task 1 の open() を移設）として両モジュールから使えるようにする。

- [ ] **Step 4: custody 判定テスト追加**:

```rust
#[test]
fn has_unacked_true_when_cursor_behind_same_epoch() {
    let conn = super::tests_support::open();
    let s = enqueue_measurement(&conn, "E", 500, 1).unwrap();
    let t = TargetRow { target_id:"t".into(), endpoint_url:"https://x".into(), credential_token:"k".into(),
        archive_responsible:true, schema_version:1, cursor_epoch:Some("E".into()), cursor_pub_seq:s-1 };
    assert!(has_unacked_pubseq_rows(&conn, "E", &t, &[500]).unwrap());
}
#[test]
fn has_unacked_false_when_cursor_epoch_mismatch_means_effective_zero_but_no_current_epoch_rows() {
    // 旧 epoch のみの reading は current epoch に存在しない → false
    let conn = super::tests_support::open();
    enqueue_measurement(&conn, "OLD", 500, 1).unwrap();
    let t = TargetRow { target_id:"t".into(), endpoint_url:"https://x".into(), credential_token:"k".into(),
        archive_responsible:true, schema_version:1, cursor_epoch:Some("OLD".into()), cursor_pub_seq:9999 };
    assert!(!has_unacked_pubseq_rows(&conn, "NEW", &t, &[500]).unwrap());
}
```

- [ ] **Step 5: 全テスト PASS** — Run: `cargo test -p iotkit-core-publish`  Expected: PASS（全 store テスト + Task 1 migration テスト）。

- [ ] **Step 6: Commit**

```bash
git add core/publish/src
git commit -m "feat(publish): outbox/target store — enqueue, batch, prune, cursor, custody predicates"
```

---

## Task 3: collector enqueue フック（同一 Tx、exact-once）

**Files:**
- Modify: `core/collector/src/actor.rs`（`process_item` の insert 直後 :314、`process_envelope` :155-196）, `core/collector/Cargo.toml`（`iotkit-core-publish` dep）
- Test: `core/collector/src/actor.rs`（`#[cfg(test)]`）

**Interfaces:**
- Consumes: `iotkit_core_publish::store::enqueue_measurement`, `iotkit_core_ledger::ledger_epoch`。
- 既存 `iotkit_core_timeseries::insert_reading_v3(conn,&NewReading)->Result<i64,_>`（seq を返す）。

- [ ] **Step 1: 失敗テスト** — envelope 処理後、非検疫 measurement 1件につき outbox 行が `(epoch, reading_seq)` で1件でき、検疫行は outbox に入らないこと:

```rust
#[test]
fn non_quarantined_reading_is_enqueued_to_outbox_same_tx() {
    // 既存の collector テスト補助(open+seed device/series)を使う。envelope を1件 process。
    // 期待: readings 1行 & publication_log 1行(kind='measurement', reading_seq=その seq, epoch=ledger_epoch)。
    // 検疫させる envelope(unknown_key 等)では publication_log 0行。
}
```
（既存 `actor.rs` テストの device/series seed パターンと `process_envelope` 呼び出しを踏襲。検疫は既存の `row_quarantined` 経路 :293 を利用。）

- [ ] **Step 2: fail 確認** — Run: `cargo test -p iotkit-core-collector non_quarantined_reading_is_enqueued`  Expected: FAIL。

- [ ] **Step 3: 実装** — `process_item`（:215-319）の `insert_reading_v3` 呼び出し(:314)を seq 捕捉に変更し、非検疫時のみ enqueue。epoch は Tx 冒頭で一度読む（`process_envelope` :172 の Tx 開始直後に `let epoch = ledger::ledger_epoch(&tx)?;` を読み、`process_item` へ `&str` で渡す）。now_ms は既存の時刻取得を流用:

```rust
// process_item 内、:314 を置換。received_at は既存の in-scope 変数名(actor.rs:221)
let seq = ts::insert_reading_v3(conn, &new).map_err(|e| e.to_string())?;
if !row_quarantined {
    iotkit_core_publish::store::enqueue_measurement(conn, epoch, seq, received_at)
        .map_err(|e| e.to_string())?;
}
```
epoch は Tx 冒頭で一度読む: `process_envelope`(:172)の Tx 開始直後に `let epoch = ledger::ledger_epoch(&tx).map_err(|e| e.to_string())?;`（`process_envelope` の error 型は `String` なので `LedgerError` は必ず `.map_err(|e| e.to_string())`）。`process_item` シグネチャに `epoch: &str` を追加、呼び出し(:192)で渡す。`conn` は `&tx`（Deref）なので同一 Immediate Tx に入る＝クラッシュ整合（spec §6.1）。

- [ ] **Step 4: PASS 確認** — Run: `cargo test -p iotkit-core-collector`  Expected: PASS（既存テスト含め緑）。既存の全緑を壊さないこと。

- [ ] **Step 5: Commit**

```bash
git add core/collector
git commit -m "feat(collector): enqueue non-quarantined measurements to outbox in the reading tx"
```

---

## Task 4: measurement/annotation レコード型 + 実体化

**Files:**
- Create: `iotkit-gateway/src/record.rs`
- Modify: `iotkit-gateway/src/main.rs`（`mod record;`）
- Test: `iotkit-gateway/src/record.rs`（`#[cfg(test)]`）

**Interfaces（Produces）:**
```rust
#[derive(serde::Serialize)] pub struct MeasurementRecord { pub family: &'static str /* "measurement" */,
    pub schema_version: u32, pub epoch: String, pub pub_seq: i64, pub series_key: String, pub values: Vec<f64>,
    pub event_time: i64, pub event_time_source: String, pub time_source: String, pub time_quality: String,
    pub received_at: i64, pub device_time: Option<i64> }
#[derive(serde::Serialize)] pub struct AnnotationRecord { pub family: &'static str /* "annotation" */,
    pub schema_version: u32, pub epoch: String, pub pub_seq: i64, pub subtype: String, pub prior_epoch: String }
// outbox 行 + readings/series を JOIN して1バッチ分の JSON 値を作る
pub fn materialize_batch(conn: &Connection, rows: &[iotkit_core_publish::store::OutboxRow]) -> Result<Vec<serde_json::Value>, String>;
pub fn series_key_of(system_id: &SystemId, measurement_key: &str, channel_index: i32, variant: &str) -> String;
```

- [ ] **Step 1: series_key 合成の失敗テスト**（spec §7、na 番兵）:

```rust
#[test]
fn series_key_renders_na_channel() {
    let sid = /* 既知 UUIDv7 の SystemId */;
    let k = series_key_of(&sid, "temperature", iotkit_core_ledger::CHANNEL_NA, "primary");
    assert!(k.ends_with(":temperature:na:primary"));
    assert!(k.starts_with(&sid.to_text()));
}
#[test]
fn measurement_record_has_all_spec7_fields() {
    let r = MeasurementRecord { family:"measurement", schema_version:1, epoch:"E".into(), pub_seq:5,
        series_key:"s".into(), values:vec![1.0], event_time:10, event_time_source:"device".into(),
        time_source:"device_ntp".into(), time_quality:"unsynced".into(), received_at:9, device_time:Some(8) };
    let v = serde_json::to_value(&r).unwrap();
    for f in ["family","schema_version","epoch","pub_seq","series_key","values","event_time",
              "event_time_source","time_source","time_quality","received_at","device_time"] {
        assert!(v.get(f).is_some(), "missing {f}");
    }
    assert!(v.get("seq").is_none(), "readings.seq を出口に出さない");
}
```

- [ ] **Step 2: fail 確認** — Run: `cargo test -p iotkit-gateway record::`  Expected: FAIL。

- [ ] **Step 3: 実装** — `series_key_of` は `format!("{}:{}:{}:{}", system_id.to_text(), measurement_key, ch, variant)`（ch は CHANNEL_NA→"na"）。`materialize_batch`: 各 OutboxRow について、
  - `kind=="measurement"`: `reading_seq` で `readings` を引き（`SELECT ... FROM readings WHERE seq=?`、`ReadingRowV3` 相当）、`series_id` で series（`ledger` の `SeriesRow`、system_id/measurement_key/channel_index/variant）を引き、`MeasurementRecord`（epoch/pub_seq は outbox 行、event_time 等は readings、series_key は合成）を作り `serde_json::to_value`。
  - `kind=="annotation"`: `annotation_json`（inline）を `serde_json::from_str` して `pub_seq`/`epoch` を上書き添付、`Value` で返す。

  > readings 単行取得は既存 `query_readings_v3` が範囲前提なので、専用 `SELECT seq,series_id,event_time,event_time_source,received_at,device_time,time_source,time_quality,values_json FROM readings WHERE seq=?1` を record.rs 内に置く（値は `values_json` を `Vec<f64>` に parse）。series 取得は `system_id` 経由の `list_series_for_device` では非効率なので、`SELECT system_id,measurement_key,channel_index,variant FROM series WHERE series_id=?1` を直接引く。

- [ ] **Step 4: PASS 確認** — Run: `cargo test -p iotkit-gateway record::`  Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add iotkit-gateway/src/record.rs iotkit-gateway/src/main.rs
git commit -m "feat(gateway): exit record types (measurement/annotation) and batch materialization"
```

---

## Task 5: push 配送タスク + 適合テスト消費者 + e2e

**Files:**
- Create: `iotkit-gateway/src/publish_task.rs`
- Modify: `iotkit-gateway/src/main.rs`（`mod publish_task;`、spawn :126 直後）, `iotkit-gateway/Cargo.toml`（`reqwest = { version="0.12", default-features=false, features=["json","rustls-tls"] }`）
- Test: `iotkit-gateway/src/publish_task.rs`（`#[cfg(test)]`、tokio + 極小 listener）

**Interfaces:**
- Consumes: `iotkit_core_publish::store::{target_get, select_batch, target_advance_cursor}`, `record::materialize_batch`, `iotkit_core_ledger::ledger_epoch`, `DbHandle::with_conn`。
- Produces: `pub fn spawn_publish_task(db: DbHandle, health: Arc<Mutex<HealthState>>, interval: Duration) -> tokio::task::JoinHandle<()>`（health は T10 で per-target 状態を書くため最初から受ける。T5 では最低限 last_push/last_error を更新）。

**push サイクル**（spec §6.2、3スコープ、epoch guard、決定的 publication_id、ack 検証）:

- [ ] **Step 1: 適合テスト消費者フィクスチャ + e2e 失敗テスト** — tokio でループバック HTTP サーバを立て、バッチ POST を受けて `{"publication_id": <受信した publication_id をそのまま echo>, "acked_pub_seq": <max pub_seq in batch>}` を返す（ack 検証 §6.2 [C] を踏むため publication_id を必ず含める）。テストは target を1件 seed（endpoint=そのサーバ）+ outbox に2行 enqueue → `run_publish_cycle` 1回 → cursor が batch 末尾 pub_seq へ進むこと:

```rust
#[tokio::test]
async fn push_cycle_delivers_batch_and_advances_cursor() {
    // 1) in-memory or temp DB を DbHandle で開き、target_registry に1行(archive_responsible=1, schema_version=1,
    //    endpoint=http://127.0.0.1:PORT, cursor_epoch=NULL) を入れる。
    // 2) publication_log に epoch=ledger_epoch で measurement 2行(reading も seed)。
    // 3) 極小 consumer を spawn(受信バッチの max pub_seq を記録し {"acked_pub_seq":max} を返す)。
    // 4) run_publish_cycle(&db).await.unwrap();
    // 5) target_get の cursor_epoch==current, cursor_pub_seq==末尾 pub_seq。consumer は2件受信。
}
#[tokio::test]
async fn byte_cap_single_oversized_record_still_delivers_one() {
    // 1レコードの values が byte cap を超える大きさでも、バッチは空にならず最低1件配送し cursor が進む(spec §6.2)。
}
#[tokio::test]
async fn ack_validation_failure_does_not_advance_cursor() {
    // consumer が publication_id 不一致(または acked_pub_seq < cursor_end)を返す → cursor 前進せず(spec §6.2 [C])。次 cycle で再送。
}
#[tokio::test]
async fn push_epoch_mismatch_redelivers_from_effective_cursor_zero() {
    // target.cursor_epoch != current_epoch → effective cursor=0 で current epoch を最小 pub_seq から再配送(push側 epoch guard)。
}
```

- [ ] **Step 2: fail 確認** — Run: `cargo test -p iotkit-gateway push_cycle_delivers`  Expected: FAIL。

- [ ] **Step 3: 実装** — `run_publish_cycle(db: &DbHandle) -> Result<(), String>`:
  - **[A] lock 内**（`db.with_conn` 1回）: `current_epoch=ledger_epoch(conn)`（毎 cycle fresh、spec §6.2）。`target_get` で target 取得（無ければ return Ok）。effective cursor = `if target.cursor_epoch==Some(current) {cursor_pub_seq} else {0}`。`select_batch(conn, &current, cursor, N=256)`。空なら return Ok（POST skip）。`materialize_batch(conn, &rows)` で JSON。**byte cap**: materialize 後、累積シリアライズ長が 1 MiB を超えたら手前で切る（**ただし最低1件は必ず含める**、spec §6.2）。ロック外へ渡すため `Vec<Value>` と `cursor_start=cursor+1`, `cursor_end=（切った後の）バッチ末尾 pub_seq`, `endpoint`, `token`, `target_id` を move で取り出す。
  - **[B] lock 外**: `publication_id = format!("{}:{}:{}:{}", target_id, current_epoch, cursor_start, cursor_end)`（**決定的文字列**。spec §10 は決定性のみ要求＝この合成で十分、ハッシュ化不要。epoch を含むので publication_id 一致 ⟹ epoch 一致）。`reqwest::Client::post(endpoint).bearer_auth(token).json(&body).send().await`（body = `{"publication_id": ..., "records": [...]}`）。非2xx/timeout はエラー→retry。ack JSON を parse。
  - **[C] lock 内**（別 `with_conn`）: ack が `acked_pub_seq >= cursor_end`（かつ publication_id 一致、spec §6.2 [C]）を満たすときのみ `target_advance_cursor(conn, target_id, &current, cursor_end)`。
  - retry: bounded exponential backoff（`spawn_publish_task` のループで cycle 失敗時に間隔を空ける）。`spawn_publish_task` は `tokio::select!` で shutdown 対応（spec §13、net-new。`main.rs` の fan-in と同様の select! パターンを新規実装）。

  > `reqwest` バージョンは `Cargo.lock` の他 crate と矛盾しない安定版（例 0.12）を選ぶ。`rustls-tls` で openssl 不要。テストの consumer は同一プロセス内 tokio listener で `http://127.0.0.1` を使う（TLS は本番のみ、テストは平文ループバックで可）。

- [ ] **Step 4: main.rs へ spawn 配線** — `iotkit-gateway/src/main.rs` の health task spawn(:126) 直後・`Collector::spawn`(:130) 前に `let _publish = publish_task::spawn_publish_task(db.clone(), health.clone(), Duration::from_secs(30));`（`health: Arc<Mutex<HealthState>>` は :121-126 で既に構築済み）。

- [ ] **Step 5: PASS 確認** — Run: `cargo test -p iotkit-gateway push_cycle_delivers`  Expected: PASS。 Run: `cargo build -p iotkit-gateway` PASS。

- [ ] **Step 6: Commit**

```bash
git add iotkit-gateway/src/publish_task.rs iotkit-gateway/src/main.rs iotkit-gateway/Cargo.toml
git commit -m "feat(gateway): outbound HTTP push task (3-scope, epoch guard, deterministic batch, ack-verified cursor)"
```

---

## Task 6: target CLI（add/list/rotate-token/remove）+ 登録ガード

**Files:**
- Create: `iotkit-gatewayctl/src/cmd/target.rs`
- Modify: `iotkit-gatewayctl/src/main.rs`（Subcommand + dispatch）, `iotkit-gatewayctl/Cargo.toml`（reqwest blocking）
- Test: `iotkit-gatewayctl/src/cmd/target.rs`（`#[cfg(test)]`）

**Interfaces:**
- Consumes: `iotkit_core_publish::store::{target_insert,target_get,target_count,target_delete,target_set_token,target_set_archive_responsible,has_unacked_pubseq_rows}`, `cmd::devices::mutate`, `ledger::{ledger_epoch,record_event}`。
- Produces: clap `TargetCommand` + `run_target_*`。

- [ ] **Step 1: ガードの失敗テスト**（in-DB、smoke は関数分離してモック）:

```rust
#[test]
fn add_rejects_non_https() { /* endpoint http:// → Err */ }
#[test]
fn add_rejects_second_target() { /* 既に1行あれば add → Err(§11) */ }
#[test]
fn add_keeps_archive_responsible_zero_until_smoke_ok() { /* smoke 失敗 → archive_responsible=0 */ }
#[test]
fn remove_refuses_when_unacked_rows_exist_without_override() {
    // target 登録 + 未ack pub_seq 行あり → remove(no --abandon-custody) は Err。--abandon-custody で Ok。
}
#[test]
fn rotate_token_keeps_archive_responsible_1_and_cursor() {
    // rotate-token 成功で token 更新・archive_responsible=1 維持・cursor 不変。
}
#[test]
fn add_rejects_schema_version_mismatch() { /* schema_version != 1 → Err(§11) */ }
#[test]
fn rotate_token_smoke_failure_rolls_back_token_and_keeps_archive_responsible_1() {
    // 再スモーク失敗 → token 旧値へロールバック、archive_responsible 終始 1(spec §3.3、iter3 [中])。
}
#[test]
fn rotation_window_protects_backlog_no_floor_only_misfire() {
    // 4日超 backlog(>floor) 保持中に rotate-token → target 消えず floor-only 発火せず backlog 保護(iter2 [高])。
}
```

- [ ] **Step 2: fail 確認** — Run: `cargo test -p iotkit-gatewayctl target`  Expected: FAIL。

- [ ] **Step 3: 実装** — `cmd/target.rs`:
  - `run_target_add`: `endpoint` を `https://` 前提で検証（違反は `AppError`）。`target_count>0` なら拒否。`mutate(conn, |tx| { target_insert(tx, &row(archive_responsible=false), now); record_event(tx,"target_added",None,&detail)?; Ok(())})`。その後 smoke（`run_smoke(&endpoint,&token)`、reqwest blocking で空/ping バッチ POST→2xx+ack 確認）。smoke OK なら別 `mutate` で `target_set_archive_responsible(tx, id, true)`。schema_version は引数（既定1）で不一致拒否。
  - `run_target_rotate_token`: `mutate(conn, |tx| { target_set_token(tx,id,new)?; Ok(()) })`（archive_responsible/cursor は触らない）。次に smoke。smoke 失敗なら旧 token へ戻す（`mutate` で `target_set_token(tx,id,old)`）。archive_responsible は終始不変（spec §3.3）。
  - `run_target_remove`: **単一 Immediate Tx** で「未ack検査→削除/拒否+監査」。`mutate(conn, |tx| { let cur=ledger_epoch(tx)?; let t=target_get(tx)?...; if !abandon && has_unacked_pubseq_rows(tx,&cur,&t, &all_reading_seqs_of_target?)? { return Err(...) } target_delete(tx,id)?; record_event(tx,"target_removed",None,&detail)?; Ok(()) })`。`--abandon-custody` で検査スキップ（監査に abandon 記録）。
  - `run_target_list`: `target_get` を println。token はマスク表示。
  - clap: `TargetCommand::{Add{...},List,RotateToken{...},Remove{--abandon-custody}}` を `main.rs` の Subcommand と dispatch に配線。

  > `has_unacked_pubseq_rows` は reading_seqs を要る設計だが、target 全体の未ack は「outbox に epoch=current かつ pub_seq>effective_cursor の行が1つでもあるか」で十分。Task 2 に `any_unacked_for_target(conn,current_epoch,&target)->bool`（`SELECT EXISTS(SELECT 1 FROM publication_log WHERE epoch=?1 AND pub_seq>?2)`）を足し、remove はこれを使う（reading_seqs 版は §9.2 用）。

- [ ] **Step 4: reqwest blocking 追加** — `iotkit-gatewayctl/Cargo.toml` に `reqwest = { version="0.12", default-features=false, features=["blocking","json","rustls-tls"] }`。smoke は blocking client（gatewayctl は sync CLI、runtime 無し）。

- [ ] **Step 5: PASS 確認** — Run: `cargo test -p iotkit-gatewayctl target`  Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add iotkit-gatewayctl/src/cmd/target.rs iotkit-gatewayctl/src/main.rs iotkit-gatewayctl/Cargo.toml
git commit -m "feat(gatewayctl): target add/list/rotate-token/remove with guards (https, smoke, TOCTOU tx, custody refuse)"
```

---

## Task 7: retention 作り替え（custody 対応パージ、単一 Tx）

**Files:**
- Modify: `iotkit-gateway/src/retention.rs`（`run_retention_once_with_latch` :47-119）
- Test: `iotkit-gateway/src/retention.rs`（`#[cfg(test)]`）

**Interfaces:**
- Consumes: `iotkit_core_publish::store::{target_get, prune_acked_outbox}`, `ledger::ledger_epoch`。既存 `expire_quarantined_devices`, dedup purge, `observe_watermark_latched` は維持。

**新パージ判定**（spec §8.2/§8.3、単一 Immediate Tx で select→outbox prune→readings delete→audit）:

- [ ] **Step 1: 失敗テスト**（4本、spec §8.2/§14）:

```rust
#[tokio::test]
async fn floor_protects_recent_and_purges_old_acked() { /* ack済み∧floor超過 のみ削除、floor内は残す */ }
#[tokio::test]
async fn unacked_pubseq_rows_are_protected_even_if_old() { /* pub_seq付き未ack は received_at 古くても残す */ }
#[tokio::test]
async fn quarantined_rows_floor_purge_not_protected() { /* quarantined=1(pub_seq無し) は floor 超過で消える */ }
#[tokio::test]
async fn epoch_mismatch_treats_all_current_as_unacked() { /* cursor_epoch!=current(分岐b) → 新epoch行を purge しない */ }
#[tokio::test]
async fn no_target_registered_purges_by_floor_only() { /* 分岐a: target 0行 → pub_seq付き行も floor 超過で消える(保護なし) */ }
#[tokio::test]
async fn old_epoch_outbox_and_readings_pruned_as_pair() { /* 旧epoch の outbox 行と対応 readings が同一 Tx でペア削除、orphan 残らない(spec §8.3) */ }
```

- [ ] **Step 2: fail 確認** — Run: `cargo test -p iotkit-gateway retention`  Expected: FAIL。

- [ ] **Step 3: 実装** — `run_retention_once_with_latch` の readings purge を作り替え。旧 `purge_readings_before`(:61) を削り、**Immediate Tx 内**（:74 の Tx を前倒しし purge を含める）で:
  - `let cur = ledger_epoch(&tx)?; let target = publish::target_get(&tx)?;`
  - effective cursor（§8.2/§8.3 epoch guard、**3分岐**、codex/Sonnet plan-review [高]）:
    - (a) **target が1行も無い or `archive_responsible=0`** → **floor-only**（pub_seq 保護なし。`eff_cursor` は使わず、保護集合を空にする）。
    - (b) **archive target 登録済み ∧ `cursor_epoch != cur`（NULL 含む）** → floor-only にしない。**effective cursor=0** で current epoch の pub_seq 付き行を**全保護**（未ack正本の無音破棄は契約違反 [D2:30]）。
    - (c) **archive target 登録済み ∧ `cursor_epoch==Some(cur)`** → `eff_cursor = cursor_pub_seq`。
    実装は `let eff_cursor: Option<i64> = match target { None => None /*floor-only*/, Some(t) if !t.archive_responsible => None, Some(t) if t.cursor_epoch.as_deref()==Some(cur) => Some(t.cursor_pub_seq), Some(_) => Some(0) };`（None=floor-only、Some(n)=保護あり）。
  - **保護集合 = pub_seq 付き未ack非検疫**。削除 SQL（`eff_cursor` が Some のとき。None=floor-only 時は保護 subquery を外し `received_at < :cutoff` 単独）:
    ```sql
    DELETE FROM readings
     WHERE received_at < :cutoff
       AND NOT (
         quarantined = 0
         AND seq IN ( SELECT p.reading_seq FROM publication_log p
                       WHERE p.kind='measurement' AND p.reading_seq IS NOT NULL
                         AND p.epoch = :cur
                         AND p.pub_seq > :eff_cursor )   -- 未ack(effective cursor 排他)
       );
    ```
    保護されるのは **(非検疫 ∧ pub_seq付き ∧ 未ack)** だけ。検疫行(quarantined=1)・enqueue 無し行(outbox に無い)・ack 済み行(pub_seq ≤ eff_cursor)は floor 超過で削除される（spec §8.2）。**`AND quarantined=0` を DELETE のトップレベルに置かない**（それだと検疫行が無条件保護＝無限保持バグ）。
  - 削除された reading_seq に対応する outbox 行を**同一 Tx で prune**（`prune_acked_outbox(&tx, &cur, eff_cursor)` で ack 済み分、加えて floor で消した readings に紐づく行）。実装は「readings 削除前に対象 seq を SELECT し、`prune_outbox_by_reading_seqs` してから readings DELETE」の順（FK 方向 outbox→readings、spec §8.3）。
  - dedup purge / `expire_quarantined_devices` / `record_event("retention_purge")` は維持し、同一 Tx に含める。`observe_watermark_latched` と health 更新は従来どおり Tx 後（spec §8.1 維持）。
  - floor 既定 72h（`RetentionConfig` に floor を追加 or 既存 days を流用しつつ 72h 相当）。

  > 実装が複雑なので、削除ロジックは `core/publish::store` か retention.rs 内のヘルパ `purge_readings_custody_aware(tx, cutoff_ms, current_epoch, effective_cursor: Option<i64>) -> Result<u64,_>` に切り出しユニットテスト可能にする（`Option<i64>` の None = floor-only）。

- [ ] **Step 4: PASS 確認** — Run: `cargo test -p iotkit-gateway retention`  Expected: PASS。既存 retention テストも緑維持（dedup/expiry/watermark/health）。

- [ ] **Step 5: Commit**

```bash
git add iotkit-gateway/src/retention.rs
git commit -m "feat(gateway): custody-aware retention — protect unacked pub_seq'd rows, floor-purge rest, single tx"
```

---

## Task 8: epoch_start annotation trigger（collector spawn 前）

**Files:**
- Create: `iotkit-gateway/src/epoch_start.rs`（`maybe_enqueue_epoch_start` + tests）
- Modify: `iotkit-gateway/src/main.rs`（`mod epoch_start;`、epoch 読み :109 と Collector::spawn :130 の間で呼ぶ）

**Interfaces:**
- Consumes: `iotkit_core_publish::store::enqueue_annotation`, `ledger::ledger_epoch`, 生 SQL で latest `epoch_renewed`。
- Produces: `pub fn maybe_enqueue_epoch_start(conn: &Connection) -> Result<(), String>`（純関数、テスト可能）。

- [ ] **Step 1: 失敗テスト**:

```rust
#[test]
fn first_boot_without_renew_does_not_enqueue() { /* epoch_renewed 無し → annotation 0行 */ }
#[test]
fn after_renew_enqueues_epoch_start_once_with_prior_epoch() {
    // renew_epoch を1回 → maybe_enqueue_epoch_start → annotation 1行(subtype=epoch_start, prior_epoch=旧)。
    // 2回目呼び出しで増えない(UNIQUE 冪等)。
}
```

- [ ] **Step 2: fail 確認** — Run: `cargo test -p iotkit-gateway epoch_start`  Expected: FAIL。

- [ ] **Step 3: 実装** — `pub fn maybe_enqueue_epoch_start(conn: &Connection) -> Result<(), String>`:
  - `let cur = ledger_epoch(conn).map_err(|e| e.to_string())?;`
  - 最新 `epoch_renewed` を生 SQL で読む: `SELECT detail FROM ledger_events WHERE kind='epoch_renewed' ORDER BY event_id DESC LIMIT 1`。**無ければ（初回 boot、pristine）return Ok**（enqueue しない、spec §5.2）。
  - `detail`(JSON) を parse し `old_epoch` を取得。**`old_epoch` が JSON `null`（DB 初回 renew=fresh box で `renew_epoch` が `old_epoch:None` を記録、`core/ledger/src/store.rs:713`）の場合も return Ok**（参照すべき prior epoch が無い＝消費者も fresh、Sonnet plan-review [低]）。有効な文字列の時のみ続行。
  - `payload = {"family":"annotation","subtype":"epoch_start","prior_epoch":<old>}`（epoch/pub_seq は record.rs の materialize 時に付与）。
  - `enqueue_annotation(conn, &cur, "epoch_start", &payload_json, now).map_err(|e| e.to_string())?`（UNIQUE 衝突は None=既出、冪等）。
  - `main.rs` で collector spawn(:130) の**前**に呼ぶ。`with_conn` は `Result<T, StorageError>` を要求するので、既存の変換ヘルパ `ledger_to_storage_err`（`main.rs:85`）に倣い `db.with_conn(|c| maybe_enqueue_epoch_start(c).map_err(iotkit_core_storage::StorageError::other)).await`（`StorageError::other(String)` が無ければ `main.rs:85` の変換関数を再利用、または nest-as-value パターン `actor.rs:71`）。epoch は既に :109 で読まれている。

- [ ] **Step 4: PASS 確認** — Run: `cargo test -p iotkit-gateway epoch_start`  Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add iotkit-gateway/src/main.rs iotkit-gateway/src/epoch_start.rs
git commit -m "feat(gateway): enqueue epoch_start annotation before collector spawn (idempotent, restore-only)"
```

---

## Task 9: §9.1 検疫解除ガード + §9.2 遡及検疫ガード

**Files:**
- Modify: `iotkit-gatewayctl/src/cmd/registry.rs`（§9.1 CLI preflight ガード、`--release-abandon-past`）, `iotkit-gatewayctl/src/cmd/replace.rs`（`run_replace_undo` :253-318、§9.2、`--abandon-custody`）
- Test: `iotkit-gatewayctl` の `#[cfg(test)]`（コア `core/registry/src/store.rs` の署名は変えない＝§9.1 は CLI 側 preflight）

**Interfaces:**
- Consumes: `iotkit_core_publish::store::{archive_target_registered, prune_outbox_by_reading_seqs, has_unacked_pubseq_rows}`。

- [ ] **Step 1: 失敗テスト**（両方向）:

```rust
// §9.1(gatewayctl cmd/registry.rs の preflight)
#[test]
fn release_rejected_while_archive_target_registered_without_override() { /* archive 登録中の alias 解除(検疫 series 対象)は Err、--release-abandon-past で Ok */ }
// §9.2(replace.rs)
#[test]
fn replace_undo_rejected_while_archive_target_registered_without_abandon() { /* Err、--abandon-custody で Ok */ }
#[test]
fn replace_undo_prunes_outbox_for_retroactively_quarantined_rows_same_tx() {
    // 既 pub_seq 行を replace-undo で quarantined=1 → 対応 outbox 行が同一 Tx で消える(orphan/FK 無し)。
}
```

- [ ] **Step 2: fail 確認** — Run: `cargo test -p iotkit-core-registry release_rejected; cargo test -p iotkit-gatewayctl replace_undo`  Expected: FAIL。

- [ ] **Step 3: 実装** —
  - **§9.1（CLI preflight に確定、コア registry 署名は不変）**: ガードは gatewayctl `cmd/registry.rs::run_registry_alias` に置く。`define_alias` 呼び出し**前**に単一 Immediate Tx 内で: (1) 対象 measurement_key に検疫 series があるか判定（`SELECT EXISTS(SELECT 1 FROM series WHERE measurement_key=?1 AND quarantined=1)` 等）、(2) `publish::archive_target_registered(tx)` が true、の**両方**なら、clap フラグ `--release-abandon-past` が無い限り `AppError::Refused(...)` で中断。フラグ有りなら通常どおり `define_alias(tx, ...)` を呼び、監査 detail に `abandon_past=true` を記録。フラグ名は spec §9.1/§14 の `--release-abandon-past` に一致させる。
  - **§9.2**: `run_replace_undo` の `mutate` クロージャ冒頭で `let cur=ledger_epoch(tx)?;` を読み、`archive_target_registered(tx)?` が true かつ `!args.abandon_custody` なら `Err(AppError::Refused(...))`。override（or archive 不在）時は既存の `mark_readings_quarantined(tx,&series_ids,since,to)` の**直後・同一 Tx**で `prune_outbox_by_reading_seqs(tx, &affected_reading_seqs)?`（affected = その series/範囲で quarantined にした reading の seq。`SELECT seq FROM readings WHERE series_id IN(...) AND received_at BETWEEN since AND to` で取得）。`--abandon-custody` は clap フラグ追加、監査 detail に記録。

- [ ] **Step 4: PASS 確認** — Run: `cargo test -p iotkit-core-registry -p iotkit-gatewayctl`  Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add iotkit-gatewayctl/src/cmd/registry.rs iotkit-gatewayctl/src/cmd/replace.rs
git commit -m "feat(gatewayctl): quarantine-transition guards — hard-reject release/replace-undo while archive target registered (§9.1/§9.2)"
```

---

## Task 10: per-target 配送状態(R12) + stale コメント + e2e 統合テスト

**Files:**
- Modify: `iotkit-gateway/src/health.rs`（`HealthState`/`render_health_json`）, `iotkit-gateway/src/publish_task.rs`（配送状態を health へ）, `core/timeseries/migrations/0004_readings_v3.sql`（コメント）
- Test: `iotkit-gateway/tests/`（新規 integration test）または `publish_task.rs`

**Interfaces:**
- Consumes: `HealthState`（`Arc<Mutex<..>>`）。

- [ ] **Step 1: stale コメント修正** — `core/timeseries/migrations/0004_readings_v3.sql:3` の `seq` コメント「出口カーソル(epoch, seq)の後半」を「内部挿入順。出口 seq は publication_log.pub_seq(D7決定4)」へ。**注**: 既適用 DB のスキーマは変わらない（コメントのみ）ので migration 差分は生じない＝この修正はソースコメントのみ（実 DB 影響なし）。migration 内容変更が `_schema_version` に影響しないことを確認（SQL 実行内容が同一なら OK。コメント変更は実行結果不変）。

- [ ] **Step 2: R12 per-target 状態の失敗テスト**:

```rust
#[tokio::test]
async fn health_json_reports_per_target_delivery_state() {
    // publish サイクル後、health.json(または HealthState)に target の cursor_pub_seq/last_push_at/last_error 等が載る。
}
```

- [ ] **Step 3: 実装** — `HealthState` に `pub publish: Vec<TargetDeliveryHealth>`（`{target_id, cursor_pub_seq, backlog, last_push_at, last_error}`）を追加。`render_health_json`（:123-151）に `"publish"` フィールド追加。`spawn_publish_task` は T5 で既に `health` を受けているので、T10 は cycle 毎の per-target 更新内容（cursor・backlog=outbox 未配送件数・成否）を充実させ render に載せるだけ（spawn 署名変更は不要）。

- [ ] **Step 4: e2e 統合テスト**（spec §14、full custody ループ + クラッシュ冪等）:

```rust
// iotkit-gateway/tests/exit_contract_e2e.rs
#[tokio::test]
async fn end_to_end_custody_loop() {
    // temp DB + 適合 consumer + target 登録 → reading 挿入(collector 経由) → push → ack → cursor 前進
    //   → retention クラス① で当該 readings 削除 + outbox prune(orphan 無し)。
}
#[tokio::test]
async fn crash_between_post_and_cursor_is_idempotent() {
    // ack 受信後 cursor 前進前に中断 → 再 cycle で同一 publication_id で再送 → consumer が (epoch,pub_seq) で dedup。
}
```

- [ ] **Step 5: PASS 確認** — Run: `cargo test -p iotkit-gateway --test exit_contract_e2e`  および `cargo test --workspace`  Expected: 全緑。

- [ ] **Step 6: Commit**

```bash
git add iotkit-gateway/src/health.rs iotkit-gateway/src/publish_task.rs core/timeseries/migrations/0004_readings_v3.sql iotkit-gateway/tests/exit_contract_e2e.rs
git commit -m "feat(gateway): per-target delivery status to R12 health, e2e custody-loop tests, fix stale seq comment"
```

---

## Global Constraints 網羅チェック（実装者向け）

各タスクが Global Constraints をどう満たすか:
- migration v10 = T1。concat 2箇所 = T1。
- `(epoch,pub_seq)` カーソル = T2(判定)/T5(push)/T7(retention)。
- retention 保護集合 = T7。POST lock 外 = T5。
- https/token/秘密 = T6/T1。reqwest 分離 = T5(async)/T6(blocking)。
- TOCTOU Immediate Tx = T6(remove)/T9(replace-undo)。
- floor 72h/バッチ最低1件/決定的 publication_id = T5/T7。
- §9.1/§9.2 hard-reject = T9。epoch_start UNIQUE = T1(index)/T8。series_key = T4。

## Homework Pins（spec §16、writing-plans で確定 → 実装時の既定）

- floor 既定 72h（T7 の `RetentionConfig`）。バッチ上限 N=256 件 かつ byte cap=1 MiB（累積シリアライズ長、**超過時も最低1件**、spec §6.2、T5 に byte-cap テストを含める）。push 間隔=既定 30s。retry backoff=指数（初期1s、上限60s）。
- ack レスポンス形式 = `{"publication_id": <string、送信値を echo>, "acked_pub_seq": <int>}`。cursor 前進は `response.publication_id == 送信 publication_id`（epoch 込み）**かつ** `acked_pub_seq >= cursor_end` の時のみ（spec §6.2 [C]、T5 で確定）。
- `publication_log.reading_seq` は FK 宣言しない（削除順 outbox→readings で担保、Global Constraints）。
- reqwest version = 0.12（Cargo.lock 整合を確認）。
- **R22 restore 前提（spec §12、Sonnet plan-review [中]）**: pristine 交換箱では series 空 ⟹ readings 空 ⟹ publication_log 空（FK 連鎖）なので publish 表の明示 cleanup は不要（benign）。非 pristine な箱への restore は運用外（推奨: restore 前後に `target remove`/再登録、epoch guard が stale cursor を fail-closed 無効化）。`run_restore` の空判定（`snapshot.rs:259`、5 SECTIONS のみ）を publish/readings へ拡張するのは後続 sub-project。本 MVE では `run_restore` を触らない。
