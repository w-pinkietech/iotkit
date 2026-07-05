# Wave 1 出口契約 MVE（R10 歩く骨格）設計仕様

> **For agentic workers:** この spec は brainstorming の成果物。実装は writing-plans → subagent-driven-development で行う。本文書は「契約」ではなく「Wave 1 実装 spec」——契約正本は [D7](../../../../docs/redesign/decisions/D7-exit-contract.md)。

**Goal:** アーカイブ責任消費者1台へ measurement レコードストリームを外向き HTTP push で at-least-once 配送し、その ack で正本(readings)のパージを許可する「end-to-end custody ループ」の最小実装。

**Architecture:** 新クレート `core/publish`（publication log[outbox] + target registry）+ `iotkit-gateway` 常駐 push タスク + `iotkit-gatewayctl target` 管理 CLI。measurement/annotation が共有する単調 `pub_seq` を outbox に採番し、`(epoch, pub_seq)` を出口カーソル同一性とする。ack 済み水位を R17 retention のパージ判定に配線する。

**Tech Stack:** Rust / tokio / rusqlite(SQLite WAL) / `reqwest`（gateway push タスク=`default-features=false, features=["json","rustls-tls"]`、gatewayctl smoke=`features=["blocking","json","rustls-tls"]`）。

---

## 1. スコープ

### 1.1 位置づけ
- Wave 1「他人に配れる」の最初の sub-project（出口契約 R10）。Wave 0（動く最小、全4計画 master マージ済み）の上に立つ。
- 契約は [D7](../../../../docs/redesign/decisions/D7-exit-contract.md) で確定済み。本 spec は契約を再定義しない。[D3](../../../../docs/redesign/decisions/D3-process-and-wave-decisions.md) 読み替え規則「契約は本番形のまま実装だけ削る」に従い、契約の一部だけを実装する。
- MVE = **歩く骨格**（single-target・measurement 族中心）。

### 1.2 実装する（IN）
1. `core/publish` クレート（outbox + target registry のスキーマ/store）
2. publication log（outbox）: measurement/annotation 共有の単調 `pub_seq`、`(epoch, pub_seq)` カーソル同一性
3. 単一 target registry（HTTPS 限定・per-target token・archive_responsible・cursor・schema_version）
4. 外向き HTTP push 配送タスク（有界バッチ POST・同期 ack・cursor 前進・retry/backoff・**決定的** publication_id）
5. annotation 族の**最小**（`epoch_start` のみ配送）
6. custody→R17 retention 作り替え（クラス① = ack 済み ∧ フロア超過のみ削除、未ack**正本**は保護。検疫行は保護しない）
7. auth 骨子（per-target bearer + HTTPS）
8. target 管理 CLI（`add|list|remove`）と登録ガード（HTTPS 強制・ledger 監査・疎通スモーク成功まで archive_responsible 無効・v1 版チェック）
9. R22 連携（target/token・outbox とも snapshot 非含有、復元後 target 再登録。§12）
10. 適合テスト消費者（リポジトリ内フィクスチャ）

### 1.3 繰り延べる/封じる（DEFERRED / SEALED）
| 項目 | 扱い | 封じ方（無音の穴を作らない） |
|---|---|---|
| custody_lost annotation | SEALED | クラス④（**保存済み**未ack正本の削除）を実装しない＝保存済みデータを消さないので custody_lost が発生しない。圧力時の劣化は front-door（既存の bounded channel / ingest spool の drop = **未受理**データの損失で custody_lost ではない）+ R12 可視化。**新規の逆圧経路は作らない**（§8.2） |
| 検疫遷移 annotation / 検疫解除 renumber | SEALED | 解除経路を **hard reject でガード（§9）**: archive target 登録中は解除を拒否（override 必要）。無音の custody 欠落を作らない。過去検疫行は R11 で読める |
| 行レベル検疫 readings の再配送 | SEALED（明示制限） | 行レベル検疫（out_of_range/device_quarantined）は pub_seq を持たず custody 対象外。**保護せず**、既存の検疫期限失効＋フロアで purge（無限保持しない、§8.2/§8.3） |
| publication snapshot | DEFERRED | R22 restore 時は `epoch_start` annotation で新 epoch へ載り替え（新 epoch は pub_seq 1 から）。復元前データの backfill は D7決定8B どおり約束しない |
| bounded backfill | DEFERRED | — |
| multi-target fan-out / 購読フィルタ | DEFERRED | single-target 固定。archive_responsible target に購読フィルタは元々不適用（D7決定7） |
| cursor_expired / gap 復帰通知 | DEFERRED | MVE はフロア内保持のため、単一 target が追随できる前提。長期断は front-door 劣化で吸収 |
| R14 型付き操作フレームワーク | DEFERRED（sub-project E） | target 操作は §11 の最小ガードのみ |
| R19 完全認証（相互認証・証明書ピン） | DEFERRED（sub-project B） | per-target bearer + HTTPS 骨子のみ |

---

## 2. 実装する D7 決定とレビュー裁定

### 2.1 実装する D7 決定
- **決定1**（生レコードストリーム一本、意味付けは消費者側）、**決定2**（record family + schema 版、measurement/annotation 族）、**決定3**（正準 event_time + 出自併載）、**決定4**（挿入順 `(epoch, seq)` カーソル、pub_seq は publication log 実体で readings.seq と別採番）、**決定5**（第一波=外向き HTTP push、at-least-once + 冪等 publication_id、再送権威=gateway outbox）、**決定6**（target registry・per-target cursor・archive フラグ・auth 骨子）、**決定7**（custody 範囲・R17 4クラスパージ順序）の各 MVE 分。
- 加えて **決定8**（コールドスタート回復のうち epoch_start による epoch 載り替え = case B の最小）、**決定9**（per-target 配送状態の R12 公開の最小）の各スライス。

### 2.2 codex アーキテクチャレビュー裁定（2026-07-04, xhigh, read-only）
アーキテクチャ案を D7 と実コードに照合。私のコードベース主張7件は全て「確認」（幻覚なし）。指摘8件の裁定:

| # | 指摘 | 裁定 |
|---|---|---|
| 1 | annotation 族の丸ごと繰り延べは契約違反 | 採用 → §5（epoch_start 実装、他トリガ封じ） |
| 2 | retention を現構造(received_at cutoff)のまま足すと未ack正本を消す | 採用 → §8 作り替え |
| 3 | cursor だけでは ack後フロア不可、`archive_acked_at` が要る | **部分採用/一部棄却**: フロア遵守は採用。ただし**フロアはデータ年齢(received_at)基準**（[台帳:115](../../../../docs/redesign/responsibility-ledger.md)「正常時のデータ残高≒フロア分のみ」）なので `archive_acked_at` は不要。遠隔ack時刻案は過剰として棄却 |
| 4 | 検疫解除 renumber 繰り延べは既存解除操作を封じないと穴 | 採用 → §9 ガード |
| 5 | publication_id は再送・クラッシュ後も安定必要 | 採用 → §10 決定的ID |
| 6 | target 登録骨子不足（版交渉/疎通スモーク/監査） | 部分採用 → §11 最小ガード、R14 本体は E へ |
| 7 | measurement JSON フィールド不足、readings.seq を出口IDにするな | 採用 → §7 全フィールド固定 |
| 8 | readings.seq migration コメントが stale | 採用 → 触る時に修正 |

### 2.3 codex spec-eval round-2 裁定（2026-07-05, xhigh, read-only）
書き上げた spec を codex に再照合。**裁定#3(フロア=データ年齢)は独立に支持**。コードベース主張は全て確認。新規8指摘を全採用: epoch guard 明記（§4/§6/§8）、R22 snapshot への token 非含有（§12）、決定的バッチ組成（§6.2/§10）、retention 既存機能維持（§8.1）、outbox prune 規則（§8.3）、水位対応（§8.2）、epoch_start 冪等 UNIQUE（§4.1/§5.2）、検疫解除 hard reject（§9、ユーザー裁定）。

### 2.4 最終並行レビュー round-3 裁定（2026-07-05, codex xhigh + Sonnet/最大深度 並行）
確定版(b121220)を codex（契約忠実性 + round-2 修正の landing 検証）と Sonnet（実装機構・並行/クラッシュ整合・境界値・plannability）で並行独立レビュー。両者の指摘を突き合わせ**全採用**（棄却なし）。Sonnet は健全性も確認（epoch-guard のレース fail-closed／floor 境界／zero-ack 保護／単一 mutex 直列化）。

| 出所 | 重大度 | 指摘 | 反映 |
|---|---|---|---|
| both | 高 | 逆圧が現実の inproc drop 挙動・D1(mpsc await, deferred非返却) と不整合・signal path 無し | §8.2 再スコープ: custody_lost 封じ=保存済み未ack を消さない。圧力劣化は front-door の既存 drop（未受理損失、custody_lost でない）+R12。新規逆圧経路は作らない |
| Sonnet | 高 | 行レベル検疫 readings が archive target 存在時に永久 unpurgeable=無限成長 | §1.3/§8.2/§8.3: 検疫行は custody 対象外、既存の検疫期限失効+フロアで purge（保護しない） |
| Sonnet | 高 | target lifecycle が add\|list のみ=token rotation の道無し・restore precondition が publish 表非カバー | §3.3/§11 に `target remove`（rotation=remove+add）、§12 に restore 相互作用 |
| both | 高 | epoch_start の trigger/順序/prior_epoch 未指定（collector 先行で pub_seq 1 崩れる） | §5.2/§6.4/§15: 起動時 collector 開始前に enqueue、prior_epoch=最新 epoch_renewed の旧値、初回 boot 除外 |
| codex | 中 | epoch guard 残矛盾（§6.3 単独要約／§8.3 floor-only 過広） | §6.3 に epoch 一致、§8.3 floor-only 条件を厳格化 |
| both | 中 | retention delete+prune+audit のクラッシュ原子性・FK 削除順 未定義 | §8.3/§4.1: 単一 Immediate Tx、outbox prune→readings delete 順 |
| Sonnet | 中 | push サイクルの単一 conn ロックを HTTP 往復越しに保持する危険 | §6.2: POST は with_conn の外、3スコープ分離 |
| Sonnet | 中 | byte-cap 単一レコード超過で永久空バッチ=custody 配送 stall | §6.2: 常に最低1件含める |
| Sonnet | 中 | gatewayctl に tokio/reqwest 無し（smoke に必要） | §11/Tech Stack: gatewayctl は reqwest blocking |
| Sonnet | 中 | current_epoch を per-cycle fresh に読む必要（health.rs は誤った precedent） | §6.2/§8: 各 cycle の Tx 内で fresh 読み |
| codex | 中 | ack 検証条件曖昧 | §6.2: publication_id/epoch/cursor_end 一致確認で cursor 前進 |
| both | 中 | §14 テスト穴 | §14 に epoch guard neg/圧力劣化/hard reject/prune 電源断/検疫無限保持/byte-cap 追加 |
| Sonnet | 低 | select! パターンは既存タスクに無い（net-new） | §6.2/§13 修正 |
| Sonnet | 低 | §2.1 が 決定8/9 スライス未列挙 | §2.1 追記済み |

reality-check（両レビューで確認）: `core/collector/src/actor.rs:314` は `insert_reading_v3` の `seq` を破棄（→ 捕捉が必要）。`DbHandle` は単一 `Arc<Mutex<Connection>>`（`core/storage/src/handle.rs:14`、プロセス全体で直列化）。`PRAGMA foreign_keys=ON`（`core/storage/src/lib.rs:20`）。`renew_epoch` は `epoch_renewed{old_epoch}` を記録（新 epoch は記録しない、`core/ledger/src/store.rs:699`）。既存 retention/health タスクは bare loop で `select!` shutdown を持たない。

---

## 3. コンポーネントとクレート構成

### 3.1 新クレート `core/publish`
既存の `core/<name>` パターンに従う（自前 `pub const MIGRATIONS`、`store.rs` は `&Connection` を取る、gateway/main.rs と gatewayctl/main.rs の migration 連結2箇所へ1行追加、`Cargo.toml` members へ1行）。

- **責務**: publication log（outbox）と target registry のスキーマ・型・store 関数。純データ層（HTTP は持たない）。
- **依存**: `core/storage`（Migration/DbHandle）、`core/ledger`（epoch/generation 読み取り、監査イベント append）。readings の実体は `core/timeseries`。

### 3.2 push 配送タスク（`iotkit-gateway`）
- **責務**: 常駐 tokio タスク。target cursor 以降の outbox 行を有界バッチ化 → JSON レコード列を構築 → per-target token + HTTPS で POST → 同期 ack → cursor 前進。失敗で retry/backoff。collector/retention と同じ「gateway が spawn する常駐タスク」形。**shutdown 応答は `select!` で net-new に実装**（既存タスクに流用可能な select! shutdown は無い＝§13）。
- **依存**: `core/publish`（outbox/target 読み・cursor 更新）、`core/timeseries`（measurement 実体の JOIN）、`core/ledger`（series_key/epoch）、`reqwest`（async）。

### 3.3 target 管理 CLI（`iotkit-gatewayctl`）
- **責務**: `gatewayctl target add|list|remove`。登録時ガード（§11）。既存 cmd/ モジュール（devices/registry/query/snapshot）と同型。
- **`remove`**: target 行を削除（+対応 cursor 破棄）。**token rotation = remove + add**（漏洩トークンの失効経路。D1 line 67「トークンは漏れる前提」に対応）。
- **注**: smoke-test POST（§11）のため、現状 sync な gatewayctl に **`reqwest` blocking client** を追加（gateway daemon 側の async reqwest とは feature 違い）。

### 3.4 enqueue フック（`core/collector`）
- collector の envelope 処理 Immediate Tx 内（`core/collector/src/actor.rs`、reading insert と同一 Tx）で、非検疫 measurement 行1件につき outbox 行1件を enqueue。`insert_reading_v3` の返り `seq` を捕捉（現状 :314 で破棄）。

### 3.5 retention 作り替え（`iotkit-gateway/src/retention.rs`）
- 現 `purge_readings_before(received_at cutoff)` を custody 対応パージ（§8）へ置換。既存の dedup TTL/検疫期限失効/statvfs ラッチ/health 更新は維持。

---

## 4. データモデル（新テーブル、`core/publish` migration）

### 4.1 `publication_log`（outbox）
```
pub_seq        INTEGER PRIMARY KEY AUTOINCREMENT,  -- 出口seqの実体（D7決定4）。readings.seq とは別採番
epoch          TEXT    NOT NULL,                   -- 採番時点の台帳epoch（ledger_meta）
kind           TEXT    NOT NULL,                   -- 'measurement' | 'annotation'
subtype        TEXT,                               -- kind=annotation: 'epoch_start' 等。measurement は NULL
reading_seq    INTEGER,                            -- kind=measurement: readings.seq への参照（JOINで実体化）。annotation は NULL
annotation_json TEXT,                              -- kind=annotation: 自己完結ペイロード。measurement は NULL
created_at     INTEGER NOT NULL
-- 冪等性: 部分 UNIQUE index  UNIQUE(epoch, subtype) WHERE kind='annotation'
--   → epoch_start の二重 enqueue（起動時再検知後 ack 前クラッシュ）を DB 制約で排除
```
- `pub_seq` は DB ライフタイムで単調（SQLite AUTOINCREMENT、再利用なし）。epoch を併載するので `(epoch, pub_seq)` が大域一意。R22 restore 後は outbox が空（snapshot 非含有）+ epoch 新規なので、新 epoch 下で pub_seq が 1 から再スタートしても `(epoch,pub_seq)` は旧世代と衝突しない。
- **カーソル同一性は必ず `(epoch, pub_seq)` の複合**。pub_seq 単独で ack/配送/パージ判定をしてはならない（epoch 跨ぎの誤適用を防ぐ。§6.4/§8.2 の epoch guard）。
- measurement は実体を持たず readings を参照（重複保存回避）。annotation は backing row が無いのでペイロードを inline 保存。
- **`reading_seq` と readings の FK / 削除順**: `PRAGMA foreign_keys=ON`（`core/storage/src/lib.rs:20`）。retention のクラス①削除は **outbox 行 prune → readings 行 delete の順**を単一 Immediate Tx で行う（§8.3）。FK を張る場合は `ON DELETE` 挙動、張らない場合は削除順を、writing-plans で確定（既定＝この削除順を守れば FK 有無どちらでも安全）。
- **不変条件**: outbox は非検疫行のみ持つ。検疫行は解除まで採番しない（D7決定4）。MVE では検疫解除 renumber を封じる（§9）。

### 4.2 `target_registry`
```
target_id           TEXT PRIMARY KEY,       -- 運用者指定の識別子
endpoint_url        TEXT NOT NULL,          -- https:// のみ（§11で強制）
credential_token    TEXT NOT NULL,          -- per-target bearer（Authorization: Bearer）
archive_responsible INTEGER NOT NULL DEFAULT 0,  -- スモーク成功まで 0（§11）
schema_version      INTEGER NOT NULL,       -- 合意した measurement 族 major 版（MVE=1）
cursor_epoch        TEXT,                   -- 最後に ack された epoch（初期 NULL）
cursor_pub_seq      INTEGER NOT NULL DEFAULT 0,  -- 最後に ack された pub_seq
created_at          INTEGER NOT NULL
```
- MVE は1行のみ運用（複数 target は DEFERRED）。
- `(cursor_epoch, cursor_pub_seq)` = target 別カーソル（D7決定6）。**両方セットで判定**（epoch guard、§6.4/§8.2）。初期 `cursor_epoch=NULL`（未 ack）は「どの epoch にも一致しない」＝effective cursor 0 として fail-closed に保護される（Sonnet 確認: `NULL = 'epoch'` は false）。
- **`credential_token` は秘密** → R22 snapshot に含めない（§12）。rotation は §3.3 の remove+add。

---

## 5. record family とストリーム

### 5.1 measurement 族（JSON、§7 に全フィールド）
一時点=1レコード（D7決定2）。`values` は単一 series の1観測の値ベクトル（多チャネル束ねでも時間ブロックでもない）。

### 5.2 annotation 族（最小: `epoch_start` のみ）
- 全 target 共有 seq（購読フィルタ不可、D7決定2）。MVE は single-target なので実質同義。
- **`epoch_start`**: R22 restore で台帳 epoch が更新された時、新 epoch 下の**最初の outbox 行**として enqueue。ペイロード = `{prior_epoch}`（D7決定8B: 新 epoch annotation には旧 epoch ID のみ記載）。消費者は自分のカーソルと突合し、新 epoch の pub_seq 1 から載り替える。
- **trigger アルゴリズム（§6.4 でも参照）**: gateway 起動時、collector を spawn する**前**に判定する。(1) 最新の `epoch_renewed` ledger イベントが存在し、その結果 epoch が現 `ledger_epoch` に一致するか（＝この起動が restore 由来か）。一致すれば restore を経ている。(2) `prior_epoch` = その `epoch_renewed` イベントの `old_epoch`。(3) 現 epoch について `epoch_start` が未 enqueue（部分 UNIQUE が保証）なら enqueue。**初回 boot（`epoch_renewed` イベント無し）は enqueue しない**。
- **冪等**: §4.1 の部分 UNIQUE(epoch,subtype) により、起動時再検知後 ack 前クラッシュでも二重 enqueue しない。
- **順序保証**: collector spawn 前に enqueue するので、新 epoch の最初の measurement より前に pub_seq が付く（pub_seq 1）。
- custody_lost / 検疫遷移 annotation は MVE では**発生させない**（§8/§9 でトリガ封じ）。

---

## 6. データフロー

### 6.1 取り込み → outbox enqueue（collector Tx 内、exact-once）
1. collector が envelope を Immediate Tx で処理（既存）。
2. 各 reading item を registry policy 評価 → `row_quarantined` 決定（既存、`actor.rs:293`）。
3. `insert_reading_v3` で readings 挿入。**現状 `actor.rs:314` は返り値 `seq` を破棄しているので、enqueue のため `seq` を捕捉する**。
4. **[新] `row_quarantined == false` の時のみ**、同一 Tx で `publication_log`(kind=measurement, reading_seq=<捕捉した seq>, epoch=<Tx冒頭で読んだ ledger_epoch>) を挿入。pub_seq は AUTOINCREMENT で採番。
5. Tx commit（既存の generation bump と同一 commit）。

電源断は正常系: enqueue は reading 挿入と同一 Tx なので、reading があって outbox が無い/その逆は起きない（クラッシュ整合性）。`row_quarantined` は同一 Tx 内で一度計算した値を両用途（readings.quarantined と enqueue 判定）に使うので TOCTOU 窓は無い（Sonnet 確認）。

### 6.2 push サイクル（常駐タスク）
DB は単一 `Arc<Mutex<Connection>>` 共有（`handle.rs:14`）。**HTTP POST は必ず `with_conn` スコープの外で行う**（3スコープ: [A] target 読み+current_epoch 読み+バッチ組成 → [B] ロック外で POST/ack → [C] cursor 永続化）。ロックを HTTP 往復（retry/backoff で長時間化しうる）越しに保持すると collector の ack 耐久化と retention を stall させる。

- **[A] ロック内**:
  1. `current_epoch = ledger_epoch(conn)` を**この cycle で fresh に読む**（起動時キャッシュしない。health.rs の起動時1回読みは踏襲しない）。
  2. **effective cursor**（epoch guard）: `target.cursor_epoch == current_epoch` なら `cursor = target.cursor_pub_seq`、そうでなければ `cursor = 0`。
  3. **決定的バッチ組成**（D7:276 宿題を確定）: `SELECT ... FROM publication_log WHERE epoch = current_epoch AND pub_seq > cursor ORDER BY pub_seq ASC LIMIT N`。件数上限 N または byte cap の先に達した方で切る。**ただし単一レコードが byte cap を超えても、バッチには最低1件含める**（空バッチで無限 stall しない、Sonnet 指摘）。cursor_start = cursor+1、cursor_end = バッチ末尾 pub_seq。バッチが空（該当行なし）なら POST を**スキップ**。
  4. measurement 行は readings を JOIN + series/epoch メタで JSON 実体化。annotation 行は inline JSON をそのまま。
  5. `publication_id = hash(target_id, current_epoch, cursor_start, cursor_end)`（決定的、§10）。
- **[B] ロック外**: `POST endpoint_url`、`Authorization: Bearer <token>`、body=JSON バッチ、HTTPS。同期レスポンスで ack。
- **[C] ロック内**: ack が **publication_id 一致・epoch=current_epoch・cursor_end まで耐久化**を確認できた時のみ、`target_registry.cursor_epoch = current_epoch`、`cursor_pub_seq = cursor_end` を単一 UPDATE で前進。確認できなければ cursor を進めず retry。
- 失敗（接続断/非2xx/タイムアウト/ack 不一致）→ bounded exponential backoff で retry。`select!` で shutdown シグナルに応答。

### 6.3 ack → custody → パージ
- retention タスク（§8）が archive_responsible=1 の target の `(cursor_epoch, cursor_pub_seq)` と `current_epoch`（cycle 内 fresh）を読み、**`publication_log.epoch == cursor_epoch == current_epoch` かつ `pub_seq ≤ cursor_pub_seq`（=ack済み）** かつ `received_at < now - フロア` の readings をクラス①として削除（§8.2）。epoch 不一致なら ack 済み扱いにしない。

### 6.4 R22 restore → epoch 載り替え
- restore で epoch 更新（既存、`renew_epoch`）。outbox/readings は空（snapshot 非含有）。**target_registry も snapshot 非含有（§12）なので、pristine な交換箱では target は無く、運用者が再登録**。
- gateway 起動時、**collector spawn 前**に §5.2 の trigger アルゴリズムで `epoch_start` を enqueue（新 epoch の pub_seq 1 を保証）。
- **epoch guard**: 万一 target が旧 `cursor_epoch` を持って残っていても（非 pristine 復元）、push/retention は `cursor_epoch != current_epoch` を検知し effective cursor=0、新 epoch を pub_seq 1 から扱う。書込側レース（POST 中に別 restore で epoch が変わり stale cursor を書き戻す）も、次 cycle の再比較で無視され fail-closed（Sonnet 確認）。消費者は epoch 不一致で再 baseline（D7決定8B）。

---

## 7. measurement レコードスキーマ（全必須フィールド）

出口 ID は `pub_seq`（**readings.seq を出さない**）。

```json
{
  "family": "measurement",
  "schema_version": 1,
  "epoch": "<台帳epoch UUIDv7>",
  "pub_seq": 12345,
  "series_key": "<D5 series_key>",
  "values": [<f64...>],
  "event_time": <ms>,
  "event_time_source": "device|gateway_adjusted|received_at",
  "time_source": "device_ntp|device_rtc|gateway|gateway_adjusted",
  "time_quality": "<D1 time_quality>",
  "received_at": <ms>,
  "device_time": <ms|null>
}
```
- `event_time`/`event_time_source` は Wave 0 で materialized 済み（migration 0008、計画4 T2）。
- レコード同一性 = `(epoch, pub_seq)`。消費者はこれで冪等 upsert（D7決定4）。

annotation `epoch_start`:
```json
{ "family": "annotation", "schema_version": 1, "epoch": "<新epoch>", "pub_seq": <n>,
  "subtype": "epoch_start", "prior_epoch": "<旧epoch>" }
```

---

## 8. custody と retention（R17 作り替え）

**「追加」でなく置換**。

### 8.1 現状（Wave 0）と維持する機能
`retention.rs` は同一周期で: readings の `purge_readings_before(received_at cutoff)` + **dedup TTL パージ** + **検疫期限失効** + **statvfs 水位ラッチ** + **health 更新** を行う（`iotkit-gateway/src/retention.rs`。現状は purge 後に別 Tx で監査/検疫処理 :61-74）。
- **本 spec が変えるのは readings パージの判定規則だけ**。dedup TTL パージ・検疫期限失効・statvfs 水位ラッチ・health 更新は**維持**する。custody/ack を知らない `received_at` cutoff 削除を §8.2 の custody 対応パージへ置換する。

### 8.2 MVE の custody 対応パージ
D7決定7 の4クラス順序のうち **MVE はクラス① のみ実装**:
- **クラス① eligibility（epoch guard）**: `target.archive_responsible=1` かつ **`target.cursor_epoch == current_epoch`** の時のみ有効。ある reading が「ack 済み」= **`publication_log.epoch == target.cursor_epoch == current_epoch` かつ `pub_seq <= target.cursor_pub_seq`**。epoch 不一致・cursor_epoch NULL 時は新 epoch 行を一切 ack 済み扱いにしない（effective cursor=0）。
- **削除対象**: 上記で ack 済み **かつ** `readings.received_at < now - 最低保持フロア` の行。
- **最低保持フロア**: [D1:154](../../../../docs/redesign/decisions/D1-ingest-model.md) / [台帳:115](../../../../docs/redesign/responsibility-ledger.md) = 既定 72h・設定可。**データ年齢(received_at)基準**（ack 相対でなくデータの新しさ。「正常時のデータ残高≒フロア分のみ、断線時のみ水位上昇」）。→ `archive_acked_at` 列は不要。
- **未ack「正本」は保護**: pub_seq を持つ（=配送対象の）非検疫 readings で未 ack のものは received_at が古くても**削除しない**（従来の無条件時刻カットオフ削除を廃止）。
- **検疫行は保護しない**（Sonnet 指摘の無限保持を回避）: 行レベル検疫（out_of_range/device_quarantined）や series 検疫の readings は pub_seq を持たず custody 対象外。これらは**既存の検疫期限失効＋フロア**で従来どおり purge する（保護対象は「custody を約束した=pub_seq 付き未 ack 正本」だけ）。
- **圧力時の劣化（custody_lost トリガ封じ）**: クラス①を出し切っても statvfs 高水位が続く場合、クラス④（**保存済み**未ack正本の削除+custody_lost）は MVE では**実装しない**＝保存済みデータを消さないので custody_lost が定義上発生しない。**新規の「水位→collector deferred 逆圧」経路は作らない**（D1 はプロセス内逆圧を mpsc await と規定し `Deferred` は返さない [D1:111]。現実装も inproc は Full 時 drop）。この場合の劣化は **front-door**: 既存の bounded channel / ingest spool が overflow で**未受理**データを drop する（`bravepi event_loop:136`, `polling_loop:622`, `ingest-client spool:194`）。これは custody（受理・保存済み正本の約束）ではないので custody_lost ではない。R12 に per-target 配送状態（遅延・target 死亡・水位）を公開して可視化する（D7決定9 最小）。水位閾値の具体値は writing-plans。

### 8.3 archive target 不在時 と outbox prune・Tx 原子性
- **floor-only へ縮退する条件を厳格化**: **target が1行も登録されていない、または archive_responsible=0 の時のみ** custody 約束が無いので floor-only（received_at < now - フロア）に縮退する。**archive_responsible=1 の target が登録済みだが cursor_epoch が未一致/NULL（未 ack）の場合は floor-only にしない**——effective cursor=0 として全 pub_seq 付き行を保護する（未 ack 正本の無音破棄は契約違反 [D2:30]）。
- **クラス①パージの原子性**: eligibility select → **outbox 行 prune → readings 行 delete** → 監査、を**単一 Immediate Tx** で行う（現 retention の purge 後別 Tx を改める）。電源断で dangling ref や半端状態を残さない。削除順は outbox→readings（FK 方向、§4.1）。
- **outbox prune 規則**: `publication_log` 行は配送 retry のため ack まで保持し、以下で prune:
  - ack 済み（`epoch==cursor_epoch==current` かつ `pub_seq <= cursor_pub_seq`）行は、対応 readings のクラス①削除と同一 Tx で prune。
  - archive target 不在の floor-only 削除で readings を消す時は、対応 outbox 行も同時に prune（outbox が readings より長生きしない・無制限成長しない）。
  - 旧 epoch の残 outbox 行はフロア基準で prune 可。

---

## 9. 検疫解除経路のガード（hard reject、ユーザー裁定 2026-07-05）

Wave 0 の alias 定義（`registry::define_alias` → `release_series_quarantine_for_key_checked`）は series 検疫フラグを clear するが、過去 readings 行は触らず outbox 化もしない。MVE は renumber を封じるので、**無音の配送欠落を作らないよう解除を hard reject でガードする**（D7:33/D5:89 に契約忠実）:

- **ルール**: **archive_responsible target が登録されている間、検疫を解除する操作（＝未配送の検疫 readings を持つ series の解除）は、明示オーバーライドフラグ無しでは拒否**する。
- **実装**: 解除経路に「登録済み archive target があり、対象 series に未配送の検疫 readings があるか」のチェックを追加。該当すれば `gatewayctl` はエラーで中断し選択肢を提示（① archive target を remove ② renumber 実装（後続）を待つ ③ `--release-abandon-past` で過去分を放棄して解除、監査記録）。
- **保持**: 拒否されている間、過去検疫行は検疫のまま `readings` に残り R11 で読める（検疫期限失効までは保持）。**黙って消えない・黙って解除されない**。
- **解除後の未来データ**: 通常どおり非検疫として outbox に流れる。欠けるのは解除前 backlog のみ。
- **完全な検疫解除 renumber**（過去行の新規採番 + measurement 再配送 + 検疫遷移 annotation）は出口契約拡張（後続 sub-project）。導入後は archive target 登録中でも過去分ごと配送でき、本ガードを緩められる。

---

## 10. クラッシュ整合性と冪等性

- **決定的 publication_id**: `publication_id = hash(target_id, current_epoch, cursor_start, cursor_end)`。§6.2 の決定的バッチ組成（epoch 一致 + `pub_seq > cursor` + ORDER BY pub_seq ASC + LIMIT + 最低1件）で、同一 cursor から同一 `(cursor_start, cursor_end)` が再現 → 同一 ID。
- **push 後 ack 前クラッシュ**: 再起動で cursor 不変 → 同一範囲を再バッチ → 同一 publication_id → 消費者が dedup。
- **ack 後 cursor 永続化前クラッシュ**: 同上（cursor 不変なので再送、消費者 dedup）。レコード同一性 `(epoch,pub_seq)` でも二重吸収。
- cursor 前進は ack 検証成功後の単一 UPDATE（§6.2 [C]）。at-least-once + 冪等（D7決定5）。

---

## 11. auth 骨子と target 管理ガード（codex#6 部分採用）

- **配送接続**: gateway=client, consumer=server。gateway が per-target token を `Authorization: Bearer` で提示、消費者がそれで gateway を認証。HTTPS(rustls) でチャネル保護。相互認証・証明書ピンは R19（sub-project B）。
- **`gatewayctl target add` ガード（MVE 最小）**:
  1. `endpoint_url` は `https://` のみ受理（平文拒否）。
  2. 登録を ledger 監査イベントに記録。
  3. 登録時**疎通スモーク**（空/ping バッチを POST し 2xx+ack を確認）。**スモーク成功まで archive_responsible=0**（誤設定 archive target へ配送→未受信のままパージ＝custody 損失を防ぐ）。
  4. `schema_version` 一致チェック（MVE=1、不一致は拒否）。
- **`target remove`**: target 行と cursor を削除。漏洩トークンの失効 = remove + 新トークンで add（§3.3）。
- **HTTP クライアント**: smoke-test POST のため gatewayctl に `reqwest`(blocking) を追加（gateway daemon の async reqwest とは別 feature）。
- 完全な R14 型付き操作（権限段階・dry-run・全操作カタログ）は sub-project E。

---

## 12. R22 連携

- **target_registry を平文 R22 snapshot に含めない**。理由: `credential_token` は秘密で、現 R22 snapshot は平文 JSON 書き出し（`iotkit-gatewayctl/src/cmd/snapshot.rs:126`）。[D2:75/98](../../../../docs/redesign/decisions/D2-data-authority-topology-operations.md) は secrets 非空 snapshot の暗号化を必須とする。R22 暗号化は MVE スコープ外なので、**target/token を snapshot に入れない**。
- **publication_log（outbox）も data-plane** → readings 同様 snapshot に含めない。
- **restore 相互作用**: `run_restore` の空判定（`snapshot.rs:259`）は 5 SECTIONS のみを見る＝ publish 2表は判定にもリストアにも関与しない。pristine な交換箱では publish 2表は空で、運用者が `target add` で再登録（credential 再発行 + スモークで archive_responsible 再有効化）。非 pristine な箱へ restore して古い target が残っても、epoch guard が stale cursor を fail-closed に無効化する（§6.4）ので誤パージ・誤配送は起きない（推奨は restore 前後に `target remove`/再登録）。target config の暗号化退避は R22 暗号化と同時に後続 sub-project へ。

---

## 13. エラーハンドリング

- push 失敗（接続断/非2xx/タイムアウト/ack 不一致）: bounded exponential backoff（上限あり）。**shutdown 応答は net-new**: 既存の retention/health タスクは bare loop（`select!` shutdown 無し）で流用できないため、push タスクは main fan-in ループ（`main.rs:253` 系）の select! パターンを参考に**新規実装**する。
- スモーク失敗: `target add` はエラーで中断、archive_responsible を立てない。
- 消費者が長期死亡: cursor 進まず outbox 滞留 → フロア超過分もパージできず水位上昇 → R12 警報。極端な圧力では front-door の未受理 drop（§8.2）に劣化する（保存済み正本は消さない）。MVE は「単一の制御された archive 消費者」前提でこれを許容（文書化された制限）。
- 全体障害と個別 target 障害を混同しない（MVE は single-target だが、エラー型は target_id 文脈を持つ）。

---

## 14. テスト戦略

- **適合テスト消費者**: リポジトリ内フィクスチャ（バッチ POST を受け、pub_seq end を ack する極小 HTTP サーバ。tokio + 単純 listener で可）。
- **end-to-end custody ループ**: reading 挿入 → outbox enqueue → push → ack → cursor 前進 → retention クラス①で当該 readings 削除、を1本で検証。
- **クラッシュ冪等性**: push 後 ack 前 / ack 後 cursor 前 の擬似クラッシュで、再送が同一 publication_id・`(epoch,pub_seq)` で消費者 dedup されること。
- **検疫除外（配送）**: 検疫行は outbox に入らない（配送されない）。
- **検疫行の無限保持回避**: 行レベル検疫 readings が archive target 登録中でも検疫期限失効＋フロアで purge され、保護されないこと。
- **retention フロア**: ack 済みでもフロア(received_at)内は削除されない。未 ack 正本は received_at が古くても保護される。
- **epoch guard negative**: cursor_epoch != current_epoch（および NULL）の時、retention が新 epoch の未配送行を purge しない・push が effective cursor=0 で配送すること。
- **epoch 載り替え**: R22 restore 後、`epoch_start` が collector 開始前に enqueue され新 epoch 最初の pub_seq で配送される（初回 boot では出さない）。
- **outbox prune / no-dangling**: ack 済み行の同一 Tx prune、floor-only 時の prune、電源断（Tx 途中）で dangling ref が残らないこと。
- **byte-cap 単一超過**: 1レコードが byte cap を超えても空バッチにならず配送が進むこと。
- **hard reject（§9）**: archive target 登録中の検疫解除が override 無しで拒否され、`--release-abandon-past` で監査付き解除できること。
- **target 登録ガード**: http:// 拒否、スモーク失敗で archive_responsible が立たない、schema_version 不一致拒否。`remove` で失効できること。
- 契約(D7)を見るテストであって実装詳細に張り付かない。malformed ack・oversized batch・shutdown 競合を含める。

---

## 15. 統合ポイント（触るファイル）

| 対象 | 変更 |
|---|---|
| `core/publish/`（新規） | クレート・migration（outbox+target+部分UNIQUE）・store。`Cargo.toml` members +1 |
| `iotkit-gateway/src/main.rs` | migration 連結に publish 追加、**collector spawn 前に epoch_start trigger 判定**（§5.2）、push タスク spawn |
| `iotkit-gateway/src/retention.rs` | custody 対応パージへ作り替え（§8）。単一 Immediate Tx で select+prune+delete+audit。current_epoch を cycle 内 fresh 読み |
| `iotkit-gateway/src/health.rs` | per-target 配送状態を health.json へ（R12、D7決定9 最小） |
| `core/collector/src/actor.rs` | Tx 内 outbox enqueue フック（§6.1）。**:314 で破棄している `seq` を捕捉** |
| `iotkit-gatewayctl/src/cmd/target.rs`（新規）+ `main.rs` | `target add|list|remove`、smoke に reqwest(blocking)、migration 連結 +1 |
| `iotkit-gatewayctl/Cargo.toml` / `iotkit-gateway/Cargo.toml` | reqwest 追加（gatewayctl=blocking / gateway=async、rustls-tls） |
| `iotkit-gatewayctl/src/cmd/snapshot.rs` | **変更なし**（§12: target/token・outbox とも snapshot 非含有） |
| `core/registry/src/store.rs`（or 呼出側） | 検疫解除 hard reject ガード（§9） |
| `core/timeseries/migrations/0004_readings_v3.sql` | stale コメント修正 |

---

## 16. 宿題ピン（writing-plans で確定する値・判断）

- 最低保持フロア既定値: 72h（[D1:155](../../../../docs/redesign/decisions/D1-ingest-model.md)）。設定手段（config or ledger_meta）。
- 有界バッチ上限 N（件数）と byte cap。**単一レコード超過時は最低1件**（§6.2）。
- ack レスポンス形式（publication_id / epoch / cursor_end 到達の JSON 形、§6.2 [C]）。MVE は all-or-nothing バッチ ack（部分 ack は DEFERRED）。
- retry backoff パラメータ・push 間隔。
- `publication_log.reading_seq` の FK 宣言有無と `ON DELETE`（張らないなら削除順で担保、§4.1）。
- epoch_start trigger の実装場所（gateway 起動シーケンス内、collector spawn 前、§5.2/§6.4）。
- 水位→R12 公開の具体形と閾値（§8.2）。
- push タスクの `select!` shutdown 実装（net-new、§13）。

---

## 17. 後続 sub-project への送り

| 送るもの | 送り先 |
|---|---|
| annotation 族フルセット（custody_lost・検疫遷移）+ クラス④パージ + 検疫解除 renumber | 出口契約 拡張（Wave 1 後続） |
| publication snapshot + bounded backfill | 出口契約 拡張 |
| multi-target fan-out・購読フィルタ・cursor_expired/gap 復帰 | 出口契約 拡張 |
| R14 型付き操作フレームワーク（target 操作の権限段階・dry-run） | sub-project E（制御面） |
| R19 完全認証（相互認証・証明書ピン・秘密管理・target config 暗号化退避） | sub-project B（セキュリティ基盤） |
| 統合メタデータ読み面（D7決定9 の unified endpoint、必要なら） | R11 拡張 |
