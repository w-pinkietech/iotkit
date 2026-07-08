# Wave 1 計画5: 制御プレーンの土台 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ゲートウェイ内に初の HTTPS API サーバー（自己署名 TLS + フィンガープリント）を立て、R14 型付き操作カタログ（権限3分類 + read-only スコープ、dry-run、全操作監査）と認証の座席（管理者パスフレーズ・setupモード・operatorトークン）を作る。

**Architecture:** 新クレート `core/ops`（カタログ框組 + `standard_catalog()` 単一組み立て + 認証ストア + migration 0012）+ `iotkit-gateway/src/api/`（axum + axum-server rustls の常駐タスク）+ `iotkit-gatewayctl` 追加コマンド。dispatch は「1 回の `with_conn` + 1 つの Immediate Tx + SQL SAVEPOINT + 監査 INSERT + commit」。

**Tech Stack:** Rust 1.95 / tokio / rusqlite(bundled, WAL) / axum 0.8 / axum-server 0.8（`tls-rustls-no-provider`）/ `rustls = { version = "0.23", default-features = false, features = ["ring","std","tls12"] }` / rcgen 0.13 / argon2 0.5 / sha2 0.10 / getrandom 0.4 / base64 0.22。

**設計正本:** [spec](../specs/2026-07-08-wave1-plan5-control-plane-foundation-design.md)（codex xhigh + Claude review-max の並行2ラウンドで収束）。各タスクは spec の §番号を参照する。設計リポジトリ正本: D3決定5 / D12決定3 / D13決定1・2 / D6決定9 / R11 / R12 / R14。

## Global Constraints

すべてのタスクに暗黙に含まれる。値は spec から逐語。

> **plan-eval 反映（2026-07-08, codex xhigh + Claude review-max 並行）**: 現実照合はほぼ全項目一致（migration concat 行番号・Immediate Tx 前例3箇所・既存 fn 署名・依存木・466テスト実測）。両者収束の構造課題3件（C1 gateway テスト基盤/I1 型順序逆転/I2 dispatch内トークン再検査）+ 実装即死系（I4 既存テスト破壊・I5 dev-dep 欠落・I6 クレート境界越え fingerprint）+ 署名精密化を本改訂で反映。詳細は各タスク内の「plan-eval」注記。

- **migration は version 12**（既存最大 11。`Migration { version, label, sql }`、gateway `main.rs:61-66` と gatewayctl `main.rs:132-137` の migration concat **両方**に `iotkit_core_ops::MIGRATIONS` を追加して再ソート。sort 行のコメント `// 1,3,…,11` を `…,12` へ）。
- **認証オフ・公開許可の設定キーを存在させない**（G3/G15。受け入れ基準4: grep で不在確認）。
- **Immediate Tx は `rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)`**（`unchecked_transaction()` は Deferred。前例: `iotkit-gatewayctl/src/cmd/devices.rs:86` / `iotkit-gateway/src/retention.rs:142` / `core/collector/src/actor.rs:180`）。
- **SAVEPOINT は SQL 直発行**（`tx.execute_batch("SAVEPOINT op")` → 失敗/dry_run: `ROLLBACK TO op; RELEASE op` / 成功: `RELEASE op`）。rusqlite の `Savepoint` 型は使わない（`&mut Transaction` 要求のため）。
- **dispatch 全体（トークン再検査→validate→preconditions→SAVEPOINT内 dry_run/execute→監査→commit）を 1 回の `with_conn` クロージャ + 1 つの Immediate Tx に入れる**（TOCTOU 排除、spec §6.1）。auth_layer は Bearer 抽出+`authenticate()` で **Actor を組み立てるが、dispatch は Tx 冒頭で `actor.actor_id`（token_id）の `revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now)` を1クエリ再検査**する（失効直後の窓を閉じる。SetupMode/LocalCli actor は再検査を飛ばす）。plan-eval I2/C2 反映。
- **argon2 照合は DB Mutex の外**（spec §4.5: throttle 判定→ハッシュ読み（with_conn）→ロック解放→`spawn_blocking` で照合→成功後に dispatch の with_conn へ）。
- **dry_run は SAVEPOINT を成功時も rollback**。書き込みは ledger_events（監査）のみ。
- **step_up_passphrase はリクエスト body 直下（`params` の外）**。監査 detail に記録するのは `params` のみ（G8）。
- **トークン**: 平文 = `iko_` プレフィックス + base64url(32byte, URL_SAFE_NO_PAD, 43文字) = **全長47文字**、応答で1回のみ。保存は SHA-256(平文全体) BLOB。token_id = `tok_` + base64url(16byte, 22文字) = **全長26文字**。パスフレーズ/トークン平文・ハッシュを Debug/ログ/エラー/監査 detail に出さない（秘密を包む型は Debug 手書き `[REDACTED]`）。
- **AI トークン構造遮断の二重化**: `operator_token.issue` の事前条件 + DB `CHECK (kind != 'ai' OR tier_ceiling IN ('read_only','routine'))`。
- **setupモード判定 = `admin_credential` 行の不在**。専用フラグを作らない。
- **setupモードのステータス**: 閉集合外=401、bulk=403（spec §4.3）。
- **実効 tier = descriptor.tier + (targets>1 && bulk_escalates ? 1段昇格 : 0)**。実効 Construction は step-up 必須。descriptor.tier に ReadOnly 不可（assert）。
- **非プライベート発ガードはハードコード**: loopback / RFC1918 / IPv6 ULA(fc00::/7) / link-local 以外の source は 403。**IPv4-mapped は unmap（`to_canonical()`）してから判定**。bind は IPv4 のみ許容（IPv6 bind は config validate で拒否）。
- **gateway を lib+bin 化**（plan-eval C1）: `iotkit-gateway/src/lib.rs` を新設し api/health/config 等を公開。統合テストは in-process で `spawn_api_task` + `bind 127.0.0.1:0` で ephemeral port を取り、`ApiHandle` から実ポートを得る。テスト内 ring install は `let _ = rustls::crypto::ring::default_provider().install_default();`（プロセス内2回目以降は Err を無視）。
- **fan-in ループ終了条件**（plan-eval C1）: 現状アダプタ0台で即 break（`should_stop_after_all_adapter_streams_closed`）。**api タスク稼働中はプロセスを生かす**よう、終了条件に「api 無効 or api タスク終了」を AND 追加（アダプタ無し+api 有効=常駐）。この変更を spec §10 の意図として明記。
- **rustls ring provider を起動時 install**: `rustls::crypto::ring::default_provider().install_default()`（aws-lc-rs を混入させない。axum-server は `tls-rustls-no-provider`）。
- **TLS ファイル規律**: `{db_path親}/tls/` 0700、key.pem 0600、temp+rename、片方欠損は両方再生成、有効期限100年（spec §5）。
- **監査粒度（G6）**: kind=`r14_op`、detail JSON に op/actor/actor_kind/tier/effective_tier/dry_run/params/result/targets/source。system_id 列は「宛先が system_id である op の単一宛先」のみ。
- **本計画マージ後、新規の変更系操作を R14 を通さず作ることを禁止**（plan-review 基準へ追記——Task 10）。
- **SetupMode actor の `tier_ceiling` = Daily**（閉集合の approve_sighting/resolve_unknown_key は Daily。bulk 昇格は §4.3 で別途拒否）。閉集合は `catalog.rs` の `pub const SETUP_ALLOWED_OPS: &[&str]` に1箇所定義し、dispatch（ceiling+閉集合判定）と HTTP 層（401 整形）の両方が参照する（plan-eval I8 反映）。
- **既存テストヘルパへの ops migration 追加も「concat 箇所」に数える**: ランタイム2箇所（gateway/gatewayctl main）に加え、テストヘルパ **`iotkit-gatewayctl/tests/cli.rs` の `all_migrations()` と `iotkit-gatewayctl/src/cmd/target.rs` の `test_conn()`** にも 0012 を追加（plan-eval I4 反映）。
- **エラー変換**: `From<StorageError|LedgerError|RegistryError|OpsError> for OpError` を core/ops に定義。`with_conn` クロージャは `Result<T, StorageError>` を返すので、dispatch は `with_conn(|c| { ... Ok(dispatch_inner(c, ...)) })` の入れ子で OpError を `Ok` 内に運ぶ（StorageError と混ぜない）。plan-eval M5 反映。
- 既存 466 テスト全緑 + `cargo clippy --workspace --all-targets -D warnings` クリーン維持。コミットは `feat(crate):` 等 + `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`。

---

## File Structure

- **`core/ops/`**（新規クレート `iotkit-core-ops`）
  - `Cargo.toml` — deps: `iotkit-core-storage`, `iotkit-core-ledger`, `iotkit-core-registry`, `rusqlite`(bundled), `serde`, `serde_json`, `thiserror`, `argon2`, `sha2`, `getrandom = "0.4"`, `base64 = "0.22"`, `tracing`; **dev: `tempfile`, `iotkit-core-storage = { path = "../storage", features = ["test-util"] }`**（`init_db_memory` は `test-util` feature 下。plan-eval I5）。
  - `migrations/0012_ops.sql` — `admin_credential` + `operator_tokens`（spec §4.1 逐語）。
  - `src/lib.rs` — `pub const MIGRATIONS`, re-exports, `OpsError`。
  - `src/tier.rs` — `Tier` enum + 序列 + parse/表示、`Actor`/`ActorKind`（**Task 2 で作成**——auth が消費するため前倒し）。
  - `src/auth.rs` — パスフレーズ set/verify/reset、トークン issue/authenticate/revoke/list、setupモード判定、認証系監査。
  - `src/fingerprint.rs` — `fingerprint_of_pem(pem: &str) -> Result<String, OpsError>`（SHA-256(DER)。gateway tls.rs と gatewayctl の両方が消費——**クレート境界を越えるため core/ops に置く**。plan-eval I6 反映）。
  - `src/catalog.rs` — `OpDescriptor` / `DispatchRequest` / `dispatch()` / `SETUP_ALLOWED_OPS`。
  - `src/ops/` — `registry_ops.rs`（resolve_unknown_key）, `device_ops.rs`（approve/retire）, `token_ops.rs`（issue/revoke）+ `standard_catalog()`。
- **`core/registry/src/store.rs`**（修正）— `define_custom_entry` 新設（spec §6.2）。
- **`core/ledger/src/`**（修正）— `series_key` 合成/分解の移設 + `find_series_by_key` + `list_series`（Task 4）。
- **`iotkit-gateway/src/lib.rs`**（**新規**）— `iotkit-gateway` を lib+bin 化する（現状 bin-only で `tests/` から `api::` を import 不能=plan-eval C1）。`pub mod api; pub mod health; pub mod config;` 等を公開し、`main.rs` は `use iotkit_gateway::…` に切り替え。crate 名 `iotkit_gateway`。`[lib]`+`[[bin]]` を Cargo.toml に明記。
- **`iotkit-gateway/src/api/`**（新規）— `mod.rs`（`spawn_api_task` → `ApiHandle` 返却）, `tls.rs`（証明書。fingerprint は core/ops 参照）, `guard.rs`（プライベート発ガード+throttle）, `auth_layer.rs`（Bearer→Actor、setup gate）, `routes.rs`（全エンドポイント）。
- **`iotkit-gateway/src/health.rs`**（修正）— `ApiHealth` + `render_health_json` pub 化 + api セクション。
- **`iotkit-gateway/src/config.rs`**（修正）— `[api]` セクション（enabled/bind、IPv4検証）+ `gateway_name`（`/box` 用。既定=hostname）。
- **`iotkit-gateway/src/main.rs`**（修正）— ring install、ops MIGRATIONS concat、api タスク spawn、fan-in ループ終了条件（**アダプタ0台でも api 稼働中は継続**=plan-eval C1）。
- **`iotkit-gateway/src/record.rs`**（修正）— series_key 合成を core/ledger 参照に置換。
- **`iotkit-gatewayctl/src/cmd/`** — `passphrase.rs` / `token.rs` / `fingerprint.rs`（新規）、`target.rs`（setup 拒否）、`main.rs`（サブコマンド+ops MIGRATIONS concat）。
- **`Cargo.toml`**（root）— members に `core/ops`。
- **`docs/eval/plan-review.md`**（修正）— R14 迂回禁止の基準追記。

---

## Task 1: `core/ops` 骨格 + migration 0012 + 依存解決スパイク

**Files:**
- Create: `core/ops/Cargo.toml`, `core/ops/src/lib.rs`, `core/ops/migrations/0012_ops.sql`
- Modify: root `Cargo.toml`（members）, `iotkit-gateway/Cargo.toml`（axum/axum-server/rustls/rcgen/core-ops 依存の**先行宣言**）, `iotkit-gateway/src/main.rs:61-66` + `iotkit-gatewayctl/src/main.rs`（migration concat）, `iotkit-gatewayctl/Cargo.toml`（core-ops）
- Test: `core/ops/src/lib.rs` `#[cfg(test)]`

**Interfaces:**
- Produces: `iotkit_core_ops::MIGRATIONS: &[Migration]`（version 12, label "ops"）, `iotkit_core_ops::OpsError`。
- Consumes: `iotkit_core_storage::{Migration, init_db_memory}`。

- [ ] **Step 1: migration SQL**（spec §4.1 を逐語で。CHECK 制約2本を含むこと）
- [ ] **Step 2: クレート骨格**（lib.rs: MIGRATIONS + thiserror の OpsError { Storage/Sqlite/NotFound/Conflict/Forbidden/Validation }）。root members 追加
- [ ] **Step 3: gateway/gatewayctl の migration concat に ops を追加**（コメント `// v3, v5, v9, v11` の行を `// …, v12` へ更新）
- [ ] **Step 4: 依存解決スパイク** — `iotkit-gateway/Cargo.toml` に本計画の全新規依存を宣言（axum 0.8, axum-server 0.8 `features=["tls-rustls-no-provider"]`, rustls（Tech Stack の feature 指定）, rcgen 0.13, time, core-ops）し、`main.rs` 冒頭に ring `install_default()` を1行だけ入れて **`cargo build --workspace` が通ること**（feature 解決 + プロバイダ衝突の地雷を最初に踏む=spec §5.4。TLS リスナーはまだ立てない——依存が解決し ring が入るのを確認するだけ）
- [ ] **Step 5: テスト** — `init_db_memory(&all)` で 0012 が適用され、`operator_tokens` へ kind='ai', tier_ceiling='daily' の INSERT が **CHECK 違反で失敗**すること
- [ ] **Step 6: `cargo test -p iotkit-core-ops` 緑 → commit** `feat(core/ops): crate skeleton + migration 0012 (admin_credential, operator_tokens)`

## Task 2: 認証ストア（Tier/Actor + パスフレーズ / トークン / setupモード）

**Files:**
- Create: `core/ops/src/tier.rs`（`Tier`/`Actor`/`ActorKind`——auth が消費するため Task 3 から前倒し。plan-eval I1）, `core/ops/src/auth.rs`
- Test: 各ファイル `#[cfg(test)]`

**Interfaces（Produces。後続タスクはこの署名に依存）:**

```rust
// tier.rs（前倒し）
pub enum Tier { ReadOnly, Routine, Daily, Construction }   // Ord: ReadOnly<Routine<Daily<Construction>
pub enum TokenKind { Human, Ai }                            // DB kind 列（local_cli は含まない）
pub enum ActorKind { Human, Ai, LocalCli, SetupMode }
pub struct Actor { pub actor_id: String, pub actor_kind: ActorKind, pub tier_ceiling: Tier }
    // authenticate は operator_tokens.kind→ActorKind（Human/Ai）、tier_ceiling 列→Tier で組む
// auth.rs
pub fn is_setup_mode(conn: &Connection) -> Result<bool, OpsError>;            // admin_credential 行の不在
pub fn set_passphrase(conn: &Connection, plaintext: &str) -> Result<SetOutcome, OpsError>;
    // SetOutcome::{FirstSet, AlreadySet}  — id=1 INSERT の先勝ち。既存行あり=AlreadySet(API層で409)
pub fn reset_passphrase(conn: &Connection, plaintext: &str) -> Result<(), OpsError>; // UPSERT(gatewayctl 用)
pub fn load_passphrase_hash(conn: &Connection) -> Result<Option<String>, OpsError>;  // PHC文字列(照合はロック外)
pub fn verify_passphrase(phc: &str, plaintext: &str) -> bool;                 // argon2id。純関数(DBなし)
pub struct IssuedToken { pub token_id: String, pub plaintext: Secret }        // Secret: Debug=[REDACTED]
pub fn issue_token(conn: &Connection, name: &str, kind: TokenKind, ceiling: Tier,
                   is_session: bool, expires_at: Option<i64>, audit_source: Option<&str>)
                   -> Result<IssuedToken, OpsError>;
    // audit_source: session ルートが接続元を渡す。is_session の監査 kind は auth_session_issued、それ以外 operator_token_issued
pub fn authenticate(conn: &Connection, plaintext: &str, now_ms: i64) -> Result<Option<Actor>, OpsError>;
    // SHA-256→UNIQUE index 引き当て。revoked/expired は None。last_used_at は前回から60_000ms超のみ UPDATE
pub fn revoke_token(conn: &Connection, token_id: &str) -> Result<(), OpsError>;
pub fn list_tokens(conn: &Connection) -> Result<Vec<TokenRow>, OpsError>;     // ハッシュ・平文は含まない
```

**認証系監査は auth fn 内で書く**（plan-eval I7。呼び出し側でなく fn 内に固定——API 経由 issue は r14_op と二重でよい=§6.4 教義）: `set_passphrase`→`admin_passphrase_set`、`reset_passphrase`→`admin_passphrase_reset`、`issue_token`→`operator_token_issued`、`revoke_token`→`operator_token_revoked`。`auth_session_issued` は issue_token(is_session=true) 時に別 kind で記録（呼び出し側の session ルートが actor/source を渡せるよう、`issue_token` に `audit_source: Option<&str>` を足す）。detail にパスフレーズ・平文・ハッシュを入れない。

- [ ] **Step 1: 失敗するテストを書く**（往復: set→verify 真/偽、setup判定が set 前 true/後 false、同時 set の後勝ち AlreadySet、issue→authenticate→Actor{tier_ceiling}、revoke 後 None、expires_at 超過 None、kind=ai×ceiling=daily が **issue の事前検査で** Err、last_used_at 間引き（同一秒2回で1回だけ更新））
- [ ] **Step 2: 実装**（argon2: `Argon2::default()` + `SaltString::generate(&mut password_hash::rand_core::OsRng)`。トークン: `getrandom::fill` 32byte → `base64::engine::general_purpose::URL_SAFE_NO_PAD`。Secret 型の Debug 手書き）
- [ ] **Step 3: テスト緑 → commit** `feat(core/ops): auth store (passphrase, operator tokens, setup-mode)`

## Task 3: R14 框組（OpDescriptor / dispatch）

**Files:**
- Create: `core/ops/src/catalog.rs`（`Tier`/`Actor`/`ActorKind` は Task 2 の tier.rs から使う）
- Test: `core/ops/tests/dispatch.rs`（fake op で框組だけを検証）

**Interfaces（Produces）:**

```rust
// Tier / Actor / ActorKind は Task 2（tier.rs）で定義済み。ここでは再掲しない
pub struct OpDescriptor {
    pub name: &'static str,
    pub tier: Tier,                                   // ReadOnly 禁止(standard_catalog がデバッグassert)
    pub bulk_escalates: bool,
    pub params_schema: fn() -> serde_json::Value,
    pub targets: fn(&serde_json::Value) -> Vec<String>,
    pub preconditions: fn(&rusqlite::Transaction<'_>, &serde_json::Value) -> Result<(), OpError>,
    pub dry_run: fn(&rusqlite::Transaction<'_>, &serde_json::Value) -> Result<serde_json::Value, OpError>,
    pub execute: fn(&rusqlite::Transaction<'_>, &serde_json::Value) -> Result<serde_json::Value, OpError>,
}
pub struct DispatchRequest { pub op: String, pub params: serde_json::Value, pub dry_run: bool,
                             pub actor: Actor, pub source: Option<String>, pub step_up_verified: bool }
pub enum OpError { NotFound, Forbidden(String), StepUpRequired, PreconditionFailed(String),
                   Validation(String), Internal(String) }
pub fn dispatch(conn: &Connection, catalog: &[OpDescriptor], req: DispatchRequest)
    -> Result<serde_json::Value, OpError>;
```

**dispatch の内部順序（spec §6.1 逐語。1 関数に直書き）:**
`Transaction::new_unchecked(conn, Immediate)` → **①トークン再検査**（actor_kind が Human/Ai のとき
`SELECT 1 FROM operator_tokens WHERE token_id=? AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>?)`。
0行なら Forbidden("token_revoked")。SetupMode/LocalCli は飛ばす。plan-eval I2）→ 実効 tier 計算（targets 数 + bulk_escalates）→
`actor.tier_ceiling >= 実効tier` 検査（SetupMode actor は `SETUP_ALLOWED_OPS` 照合 + bulk 一律 Forbidden をここで）→
実効 Construction なら `step_up_verified` 必須 → params_schema による validate → preconditions →
`SAVEPOINT op` → dry_run なら fn 実行後 **必ず** `ROLLBACK TO op; RELEASE op` / execute は成功
`RELEASE`（**成功時は同 SAVEPOINT 内で `ledger::bump_generation(&tx)` を呼ぶ**——ledger/registry 変異の
generation 共有=plan-eval I5/既存 CLI 流儀 devices.rs:88）/ 失敗 `ROLLBACK TO; RELEASE` →
`record_event(&tx, "r14_op", system_id_opt, &detail_json)` → `tx.commit()`。
失敗（Forbidden/Precondition/実行エラー）でも監査は書いて commit する（結果コード付き）。
- **SETUP_ALLOWED_OPS**（`pub const &[&str]`、この catalog.rs に定義）= `["registry.resolve_unknown_key","device.approve_sighting"]`。dispatch と HTTP 層（Task 6/8）が参照。

- [ ] **Step 1: 失敗するテスト**（fake catalog 2op: `fake.write`（execute で `CREATE TABLE`… ではなく registry の無害な行 INSERT）/ `fake.fail`（必ず Err）。検証: dry_run で対象テーブル無変化+監査1行 / execute 成功で変化+監査 / execute 失敗で**変化なし+失敗監査あり** / ceiling 不足 403 相当 Err+監査 / bulk 昇格で step_up_verified=false→StepUpRequired / SetupMode actor の閉集合外 op→Forbidden / SetupMode + bulk→Forbidden(“bulk”理由) / 存在しない op→NotFound（監査なし））
- [ ] **Step 2: 実装**（tier Ord・detail JSON 構築は `serde_json::json!`——文字列連結禁止=G7）
- [ ] **Step 3: テスト緑 → commit** `feat(core/ops): R14 dispatch framework (tier enforcement, savepoint, audit)`

## Task 4: 初期カタログ5op + registry/ledger の新設 fn

**Files:**
- Create: `core/ops/src/ops/{mod.rs, registry_ops.rs, device_ops.rs, token_ops.rs}`
- Modify: `core/registry/src/store.rs`（`define_custom_entry`）, `core/ledger/src/lib.rs`+`store.rs`（`series_key_of` / `parse_series_key` / `find_series_by_key` / `list_series` 移設・新設）, `iotkit-gateway/src/record.rs`（合成を ledger 参照へ）
- Test: `core/ops/tests/catalog.rs` + 各クレートの既存テスト増分

**Interfaces（Produces）:**

```rust
// core/ops
pub fn standard_catalog() -> &'static [OpDescriptor];   // 5op。単一組み立て点(G14)
// core/registry
pub struct CustomEntrySpec { pub measurement_key: String, pub unit_ucum: Option<String>,
    pub unit_display: Option<String>, pub value_type: ValueType, pub semantic_class: String,
    pub channel_mode: ChannelMode, pub channel_roles: Option<Vec<String>>,
    pub physical_min: Option<f64>, pub physical_max: Option<f64> }
pub fn define_custom_entry(conn: &Connection, spec: &CustomEntrySpec) -> Result<EntryRow, RegistryError>;
    // origin='custom', catalog_version=NULL, entry_revision=内容ハッシュ(CatalogEntry::revision()と同レシピ)
    // 検査: measurement_key は "custom." 接頭辞必須 + validate_measurement_key、entry/alias 衝突は enable_entry/define_alias と同一
// core/ledger（series_key_of は record.rs から移設。channel_index は i32=実物一致・plan-eval M2）
pub fn series_key_of(system_id: &SystemId, measurement_key: &str, channel_index: i32, variant: &str) -> String;
pub fn parse_series_key(key: &str) -> Result<ParsedSeriesKey, LedgerError>;   // コロン4分割(na→CHANNEL_NA=i32)
pub fn find_series_by_key(conn: &Connection, key: &str) -> Result<Option<i64>, LedgerError>;
pub fn list_series(conn: &Connection) -> Result<Vec<SeriesListRow>, LedgerError>; // {series_id, series_key, system_id, user_label}——series_id は内部用(API §7.1 は落とす)
```

**5op の tier/params は spec §6.2 の表を逐語で。既存 fn の実署名（plan-eval で確認）:**
- resolve_unknown_key: alias 枝=既存 `define_alias(conn, 申告キー, target, AliasKind::SiteMapping)` / custom 枝=`define_custom_entry`（Step 2 で新設）+alias。**注記（plan-eval M11）**: `define_alias` は series 級 unknown_key の遡及検疫解除（`series_quarantine_released` 監査）という既存副作用を持つ。これは spec §6.2 の「検疫**行**の遡及解除は行わない」（=検疫 readings の話）と矛盾しない——**この副作用は残す**（実装者が「余計」と誤除去しない）。
- approve_sighting: params `{"hardware_ids":[..]}` → `approve_sighting(conn, hw, user_label, kind)`。**`user_label=None`・`kind=DeviceKind::Individual` 固定**（CLI 既定と一致=devices.rs:45。plan-eval I3）。sighting 行が無ければ `LedgerError::NotFound`。
- retire: params `{"system_ids":[..]}` → `retire_device(conn, &SystemId)`（store.rs:677）。既 retired は NotFound（bulk は SAVEPOINT で all-or-nothing）。
- token issue/revoke: Task 2 の fn。

- [ ] **Step 1: ledger の series_key 移設**（record.rs:32 の合成を core/ledger へ移し record.rs は再 export/参照。ゴールデンテスト: `uuid:temperature_c:na:primary` 往復、ch=2、variant=count。※replace.rs:156-158 は na **表示**の前例であって series_key 合成の前例ではない=plan-eval M1）→ 緑 → commit `refactor(core/ledger): move series_key compose/parse into ledger`
- [ ] **Step 2: `define_custom_entry` の失敗するテスト**（custom. 接頭辞なし→Err / 既存 entry・alias と衝突→Err / 成功で origin='custom'+revision 非空 / fixed で channel_roles 必須）→ 実装 → 緑 → commit `feat(core/registry): define_custom_entry (D6決定9 custom branch)`
- [ ] **Step 3: 5op + standard_catalog の失敗するテスト**——**tier 執行マトリクスをそのままテスト化**（ceiling {read_only, routine, daily, construction} × 5op × {単数, bulk} × {step_up 有無}。期待値表を tests 内の定数配列で書き、ループで dispatch）。加えて: resolve alias 枝で registry_aliases に行 / custom 枝で registry_entries+alias / approve→devices 遷移+既存 `sighting_approved` 監査**も**残る（r14_op と二重=仕様）/ ai トークンで token.issue→Forbidden
- [ ] **Step 4: 実装 → 緑 → commit** `feat(core/ops): standard catalog (5 ops) + tier matrix tests`

## Task 5: TLS モジュール（rcgen + フィンガープリント + ring install）

**Files:**
- Create: `core/ops/src/fingerprint.rs`（`fingerprint_of_pem`——**クレート境界を越えるため core/ops に置く**。gateway tls.rs と gatewayctl の両方が消費=plan-eval I6）, `iotkit-gateway/src/api/tls.rs`（+ `src/api/mod.rs` 骨格）
- Modify: `iotkit-gateway/Cargo.toml`（Task 1 で宣言済みの依存に加え `time`——rcgen の `not_after` 用）, `iotkit-gateway/src/main.rs`（ring install）
- Test: 各ファイル `#[cfg(test)]`（tempdir）

**Interfaces（Produces）:**

```rust
// core/ops::fingerprint（sha2/base64 は既存 dep。PEM→DER は自前: BEGIN/END 行除去+base64 decode）
pub fn fingerprint_of_pem(cert_pem: &str) -> Result<String, OpsError>;  // SHA-256(DER) 16進コロン区切り
// iotkit-gateway::api::tls
pub struct TlsMaterial { pub cert_pem_path: PathBuf, pub key_pem_path: PathBuf, pub fingerprint: String }
pub fn ensure_tls_material(data_dir: &Path) -> Result<TlsMaterial, TlsError>;  // fingerprint は ops の fn 経由
```

- [ ] **Step 1: 失敗するテスト**（初回生成で cert/key が作られ fingerprint 一致 / 再呼び出しで**同一** fingerprint（再生成しない）/ key.pem のみ削除→両方再生成され fingerprint が変わる / Unix で tls/=0700・key.pem=0600 / 生成は temp+rename）
- [ ] **Step 2: fingerprint_of_pem を core/ops に実装**（PEM ヘッダ除去→base64 decode→sha2→コロン16進。単体テスト: 既知 PEM→既知 fingerprint）
- [ ] **Step 3: tls.rs 実装**（rcgen `CertificateParams`（SAN: `iotkit-gateway.local`+hostname）に `not_after = now+100年`（`time::OffsetDateTime`）→ `self_signed(&KeyPair)`。fingerprint は `iotkit_core_ops::fingerprint_of_pem` 経由）
- [ ] **Step 4: main.rs 冒頭（tokio runtime 生成前）に `rustls::crypto::ring::default_provider().install_default().expect("ring provider")` を追加**（テストは `let _ =` で握り潰す=プロセス内2回目 Err）
- [ ] **Step 5: テスト緑 → commit** `feat(gateway/api): self-signed TLS material + fingerprint (ring provider)`

## Task 6: API サーバー骨格（config / ガード / 認証層 / box / session / setup）

**Files:**
- Create: `iotkit-gateway/src/lib.rs`（**lib+bin 化**=plan-eval C1。`pub mod api; pub mod health; pub mod config; pub mod record;` 等を公開）、`iotkit-gateway/src/api/{guard.rs, auth_layer.rs, routes.rs}`、`iotkit-gateway/src/api/mod.rs`（`spawn_api_task`）
- Modify: `iotkit-gateway/Cargo.toml`（`[lib] name="iotkit_gateway"` + `[[bin]] name="iotkit-gateway" path="src/main.rs"`）, `iotkit-gateway/src/main.rs`（`use iotkit_gateway::…` へ切替）, `iotkit-gateway/src/config.rs`（`[api]`: `enabled: bool=true`, `bind: String="0.0.0.0:8443"`, `gateway_name: Option<String>`（既定=hostname）。**validate: IPv4 SocketAddr のみ受理**、IPv6 は Validation エラー）
- Test: `iotkit-gateway/tests/api_basic.rs`（in-process `spawn_api_task`+`bind 127.0.0.1:0`、reqwest `danger_accept_invalid_certs(true)`）

**Interfaces:**
- Produces: `api::spawn_api_task(db, health, cfg, epoch) -> Result<ApiHandle, ApiError>` where `ApiHandle { pub local_addr: SocketAddr, pub fingerprint: String, shutdown: oneshot::Sender<()>, join: JoinHandle<()> }`（bind fail-fast を Result で、ephemeral port を local_addr で、graceful shutdown を shutdown sender で表現=plan-eval C1/I8）。ルート: `GET /api/v1/box`、`POST /api/v1/session`、`POST /api/v1/setup/passphrase`。
- Consumes: Task 2 auth fn 群、Task 5 TLS。
- `/box` の `gateway_name` = config 値 or hostname（`hostname` crate は足さず `std` の gethostname 代替 or 既存手段。無ければ config 必須化）。`health_summary.status`="ok"（collector_alive && 全 adapter alive）|"degraded"。

**要点（spec 逐語）:**
- `into_make_service_with_connect_info::<SocketAddr>()` で source IP を取得。`guard::is_private_source(ip)`: `to_canonical()` で unmap → v4: loopback|RFC1918|link-local / v6: loopback|ULA(`seg[0]&0xfe00==0xfc00`)|link-local(fe80::/10)。非該当は 403（**ミドルウェア最外周**）。
- throttle（guard.rs 内 in-memory）: per-source 失敗 n 回→`min(2^n,60)`s 429、グローバル 10 req/s 超 429。**照合前**に判定。`auth_failed` 監査。
- `POST /session`: throttle → `load_passphrase_hash`（with_conn）→ **ロック外** `spawn_blocking(verify_passphrase)` → 成功で `issue_token(kind=Human, ceiling=Construction, is_session=1, expires=now+30d)`。未設定なら 409。
- `POST /setup/passphrase`: setupモードのみ（設定済み 409=SetOutcome::AlreadySet）。成功でセッション返却+`admin_passphrase_set` 監査。
- `auth_layer`: Bearer→`authenticate()`→`Actor` を extension へ。setupモード中は §4.3 の無認証許可（actor=SetupMode を合成、source を detail 用に保持）。それ以外の未認証は 401。
- Body 上限 64KB（`DefaultBodyLimit`）+ タイムアウト。エラー形 `{error:{code,message}}` を統一 IntoResponse で。
- 全リクエスト trace ログ（Authorization/body 出さない）。

- [ ] **Step 1: lib.rs 新設 + main.rs 切替**（`cargo build --workspace` が通る。既存 bin 挙動不変）→ commit `refactor(gateway): split into lib+bin to enable integration tests`
- [ ] **Step 2: 失敗するテスト**（in-process e2e: `spawn_api_task(bind 127.0.0.1:0)`→`ApiHandle.local_addr`へ reqwest。`/box` 200 で `setup_mode:true`+fingerprint / `POST /setup/passphrase`→セッション / 以後 `/box` は `setup_mode:false`・再 POST 409 / `POST /session` 誤パスフレーズ連発→429 と `auth_failed` 監査 / 正→トークン / `auth_session_issued` 監査 / 非プライベート source 403 は `guard::is_private_source` の単体表（IPv4-mapped unmap 含む）で / IPv6 bind が config validate で Err）
- [ ] **Step 3: 実装 → 緑 → commit** `feat(gateway/api): HTTPS control-plane server (box/session/setup, private-source guard, throttle)`

## Task 7: 読み出しルート（health / series / live / readings）

**Files:**
- Modify: `iotkit-gateway/src/api/routes.rs`、`iotkit-gateway/src/health.rs`（`ApiHealth { bind, tls_fingerprint }`・`render_health_json` **pub 化**+api セクション・タスク終了時 None 復帰）
- Test: `iotkit-gateway/tests/api_read.rs`

**要点:**
- `GET /health`: 認証マトリクスどおり **setup中も 401**。応答=`render_health_json(epoch, &snapshot)` そのまま。
- `GET /series`: `ledger::list_series` → `[{series_key, system_id, user_label}]`（series_id を**出さない**）。setup中無認証可。
- `GET /live`: series ごとに `timeseries::query::latest_by_series`（query.rs:209 既存）→ `[{series_key, event_time, event_time_source, quarantined, values}]`。setup中無認証可。
- `GET /readings?series_key&from_ms&to_ms&limit&include_quarantined`: from/to 必須（欠落 422）、`find_series_by_key`（未知 404）→ `query_readings_v3`。**setup中 401**。応答スキーマは spec §7.3 逐語。

- [ ] **Step 1: 失敗するテスト**（シード: ledger にデバイス+series+readings を直接 INSERT（既存テストヘルパ流儀）。検証: series 応答に series_id キーが**無い** / live が series ごと最新1件のみ / readings from 欠落 422・未知 series_key 404・CLI `query_readings_v3` と同一行 / setup中: series・live 200、readings・health 401）
- [ ] **Step 2: 実装 → 緑 → commit** `feat(gateway/api): read surface (health/series/live/readings)`

## Task 8: ops ルート（カタログ列挙 + dispatch + step-up）

**Files:**
- Modify: `iotkit-gateway/src/api/routes.rs`
- Test: `iotkit-gateway/tests/api_ops.rs`

**要点:**
- `GET /ops`: `standard_catalog()` を `[{name, tier, bulk_escalates, params_schema}]` で返す。
- `POST /ops/{name}`: body `{params, dry_run?, step_up_passphrase?}`。step_up があれば throttle→ロック外 argon2（session と同経路）→ `step_up_verified=true`。dispatch は Task 3 の `dispatch()` を 1 回の with_conn で。OpError→HTTP: NotFound=404 / Forbidden=403 / StepUpRequired=403(code=step_up_required) / PreconditionFailed=409 / Validation=422。
- setupモード中: 閉集合（resolve_unknown_key / approve_sighting / setup 経由の許可 GET）以外 401、bulk 403（dispatch 内判定と二重でよい——外側は HTTP コード整形）。

- [ ] **Step 1: 失敗するテスト**（e2e: setup で approve_sighting 単数 200・bulk 403・retire 401 → passphrase 設定 → session トークンで retire 200 / token.issue は step_up 無し 403(step_up_required)・有り 200 で**平文が応答にのみ**現れ監査 detail に無い / ai トークン発行→そのトークンで token.issue 403（Construction 遮断）・**同 ai トークンで `GET /series` 200**（受け入れ基準3「ai×Routine相当は成功」を読み出しGETで実証=plan-eval M8）/ dry_run=true で devices 無変化+r14_op(dry_run:true) 監査行 / **監査 INSERT を故意に失敗させると操作ごと rollback**（spec §11）/ ledger_events に G6 粒度キーが揃う。sightings シードは `ledger::record_sighting` を実形式 hardware_id（`rpi-local:default:i2c:0x60` 等=plan-eval M10）で）
- [ ] **Step 2: 実装 → 緑 → commit** `feat(gateway/api): R14 ops dispatch endpoint (+step-up)`

## Task 9: gatewayctl（passphrase / fingerprint / token / target ガード）

**Files:**
- Create: `iotkit-gatewayctl/src/cmd/{passphrase.rs, token.rs, fingerprint.rs}`
- Modify: `iotkit-gatewayctl/src/main.rs`（サブコマンド登録 + migration concat に ops 追加）, `iotkit-gatewayctl/src/cmd/target.rs`（`target add` 冒頭に setup 拒否 + **`test_conn()` に 0012 追加 + admin 行 seed**=plan-eval I4）, `iotkit-gatewayctl/tests/cli.rs`（`all_migrations()` に ops 追加=plan-eval I4）
- Test: `iotkit-gatewayctl/tests/`（既存流儀の CLI 統合テスト）

**要点:**
- `passphrase reset`: 対話入力（`rpassword` は追加しない——標準入力 read_line + 確認2回で足りる。echo 抑制なしを許容と明記）→ `auth::reset_passphrase` → `admin_passphrase_reset` 監査（actor=local_cli）。
- `token issue|revoke|list`: **Task 2 の同一 fn を直接呼ぶ**（G14 の「同一関数」= core/ops の fn。CLI は物理アクセス=root相当なので auth 層を通さない——監査 actor=local_cli）。issue は平文を1回だけ stdout。
- `fingerprint`: **`iotkit_core_ops::fingerprint_of_pem`** を使う（gateway クレートに依存しない=plan-eval I6。cert.pem を読んで渡す。無ければ「未生成（gateway 未起動）」）。
- `target add`: `ops::auth::is_setup_mode(conn)?` が true なら明示エラー「setupモード中は出口target登録不可（D13）。管理者パスフレーズを設定してから」（spec §1.2-12）。
- **既存 target テストの修復**（plan-eval I4）: `target.rs` の `test_conn()` は現状 storage/ledger/timeseries/registry/publish のみ。0012 を足し、setup 拒否テスト以外の既存5テストは冒頭で `auth::reset_passphrase(&conn, "test")` して admin 行を seed（= setupモードを抜けてから target add を検証）。

- [ ] **Step 1: 失敗するテスト**（token issue→list に token_id・ハッシュ非表示 / revoke 後 authenticate None / target add が setup 中 Err・passphrase seed 後成功 / passphrase reset 後に古いパスフレーズで verify 偽 / fingerprint が gateway 生成 cert と一致）
- [ ] **Step 2: 実装 + 既存 target テスト修復 → 緑 → commit** `feat(gatewayctl): passphrase/token/fingerprint commands + target-add setup guard`

## Task 10: 統合仕上げ（main 配線 / 受け入れ基準 / 還流）

**分担の固定（plan-eval M4）**: **spawn は Task 6**。Task 10 は「run() への配線・bind fail-fast・fan-in 継続条件・ctrl_c shutdown・起動ログ・api タスク終了時の health `api=None` 復帰」を担う。

**Files:**
- Modify: `iotkit-gateway/src/main.rs`（api spawn を run() へ、bind 失敗 fail-fast、**fan-in ループ終了条件に api 稼働 AND を追加**、ctrl_c で `ApiHandle.shutdown` 送出、起動ログに bind+fingerprint+有効インターフェース）, `iotkit-gateway/src/health.rs`（api タスク join 後 `api=None`）, `docs/eval/plan-review.md`（「新規の変更系操作は R14 dispatch 経由必須（計画5以降）」を Baseline に追記）, `.superpowers/sdd/progress.md`（計画5 クローズ+計画6 ブロッカー解除）, `README.md`（復元後は再 setup が必要+Tailscale/SSH forward の一言=spec §9/§7.6）
- Test: `iotkit-gateway/tests/api_e2e.rs`（受け入れ基準2の全シーケンス）

- [ ] **Step 1: 受け入れ基準2の e2e テスト**（初回起動→box(setup:true, fingerprint)→setup/passphrase→box(setup:false)→session→approve_sighting dry_run→execute→ledger_events 検証、を1テストで通し。sightings は実形式 hardware_id で seed）
- [ ] **Step 2: main 配線 + graceful shutdown + fan-in 継続条件**（bind 失敗で非ゼロ exit=DB init と同格。アダプタ0台+api 有効で常駐）
- [ ] **Step 3: 受け入れ基準4** — `grep -rin "disable\|insecure\|allow_public\|skip_auth\|no_auth" iotkit-gateway/src core/ops/src` で認証オフ/公開スイッチが無いことを確認（タスク手順として実施・出力を記録）
- [ ] **Step 4: `cargo test --workspace`（466+新規 全緑）+ `cargo clippy --workspace --all-targets -- -D warnings` → commit** `feat(gateway): wire control-plane API into runtime (plan 5 close)`
- [ ] **Step 5: 設計正本への還流**（spec §13）— 設計リポジトリ（`../docs/redesign/`）側の D13 保留節へ「工場リセット=setupモード再突入の意味論を計画5 spec で部分確定（fresh DB 復元=再突入）」、D13決定2 の回復経路実装確定、台帳 R14 行へ「計画5で骨格実装」を注記（別リポジトリのため**別コミット**、設計リポジトリの日本語 `docs:` 規約で）。progress.md / plan-review.md / README 更新 → iotkit-next 側 commit `docs: close wave1 plan5, unblock plan6 (R2 ingress)`

---

## Self-Review（plan-eval 反映後・記入済み）

- **Spec coverage**: §4(Task 2,6) §5(Task 5) §6(Task 3,4,8) §7(Task 6,7,8) §8(Task 9) §9(Task 10 README) §10(Task 6,10) §11(各タスクのテスト、監査INSERT失敗rollback=Task 8・throttleリセット=Task 6) §12(Task 8 基準3=ai×GET・Task 10) §13(Task 10 Step 5 で設計リポジトリへ別コミット還流)。
- **Type consistency**: `Tier`/`Actor`/`ActorKind` は **Task 2（tier.rs）** 定義を Task 3/4/6/8 が消費（順序逆転を解消=plan-eval I1）。`OpError`/`OpDescriptor`/`SETUP_ALLOWED_OPS` は Task 3 定義を Task 4/6/8 が消費。`find_series_by_key`/`list_series`（series_key_of は i32）は Task 4 定義を Task 7 が消費。`fingerprint_of_pem` は **core/ops（Task 5）** 定義を gateway tls.rs（Task 5）と gatewayctl（Task 9）が消費（クレート境界解消=plan-eval I6）。`ApiHandle` は Task 6 定義を Task 10 が消費。
- **依存順序**: Task 1(骨格+スパイク)→2(型+auth)→3(dispatch)→4(catalog)→5(TLS+fingerprint)→6(lib化+server)→7/8(routes)→9(CLI)→10(配線)。各タスクが独立コンパイル可能（Task 2 が型を持つため Task 2 単独で緑）。
- **Placeholder scan**: 「echo 抑制なしを許容」（Task 9）は判断確定済み（rpassword を足さない）。TBD なし。
- **plan-eval 未反映の残**: なし（C1/I1-I8/M1-M11 は各タスク or Global Constraints に反映）。M6 の gateway_name=hostname は Task 6 で「無ければ config 必須化」の fallback つき。
