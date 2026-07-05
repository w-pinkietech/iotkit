# Wave 1 出口契約 MVE（R10 歩く骨格）設計仕様

> **For agentic workers:** この spec は brainstorming の成果物。実装は writing-plans → subagent-driven-development で行う。本文書は「契約」ではなく「Wave 1 実装 spec」——契約正本は [D7](../../../../docs/redesign/decisions/D7-exit-contract.md)。

**Goal:** アーカイブ責任消費者1台へ measurement レコードストリームを外向き HTTP push で at-least-once 配送し、その ack で正本(readings)のパージを許可する「end-to-end custody ループ」の最小実装。

**Architecture:** 新クレート `core/publish`（publication log[outbox] + target registry）+ `iotkit-gateway` 常駐 push タスク + `iotkit-gatewayctl target` 管理 CLI。measurement/annotation が共有する単調 `pub_seq` を outbox に採番し、`(epoch, pub_seq)` を出口カーソル同一性とする。ack 済み水位を R17 retention のパージ判定に配線する。

**Tech Stack:** Rust / tokio / rusqlite(SQLite WAL) / `reqwest`(`default-features=false, features=["json","rustls-tls"]`)。

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
6. custody→R17 retention 作り替え（クラス① = ack 済み ∧ フロア超過のみ削除、未ack正本は保護）
7. auth 骨子（per-target bearer + HTTPS）
8. target 登録ガード（HTTPS 強制・ledger 監査・疎通スモーク成功まで archive_responsible 無効・v1 版チェック）
9. R22 連携（target/token・outbox とも snapshot 非含有、復元後 target 再登録。§12/codex#2）
10. 適合テスト消費者（リポジトリ内フィクスチャ）

### 1.3 繰り延べる/封じる（DEFERRED / SEALED）
| 項目 | 扱い | 封じ方（無音の穴を作らない） |
|---|---|---|
| custody_lost annotation | SEALED | 未ack正本の圧力パージ（クラス④）を実装せず、圧力時は逆圧+R12警報。→ custody_lost が発生しない |
| 検疫遷移 annotation / 検疫解除 renumber | SEALED | 解除経路（alias 定義）をガード（§9）: 登録 target がある時、過去検疫 readings は再配送されない旨を ledger 監査記録。R11 では引き続き読める |
| publication snapshot | DEFERRED | R22 restore 時は `epoch_start` annotation で新 epoch へ載り替え（新 epoch は pub_seq 1 から）。復元前データの backfill は D7决定8B どおり約束しない |
| bounded backfill | DEFERRED | — |
| multi-target fan-out / 購読フィルタ | DEFERRED | single-target 固定。archive_responsible target に購読フィルタは元々不適用（D7决定7） |
| cursor_expired / gap 復帰通知 | DEFERRED | MVE はフロア内保持のため、単一 target が追随できる前提。長期断は逆圧で吸収 |
| R14 型付き操作フレームワーク | DEFERRED（sub-project E） | target 登録は §8 の最小ガードのみ |
| R19 完全認証（相互認証・証明書ピン） | DEFERRED（sub-project B） | per-target bearer + HTTPS 骨子のみ |

---

## 2. 実装する D7 決定と codex レビュー裁定

### 2.1 実装する D7 決定
- **決定1**（生レコードストリーム一本、意味付けは消費者側）、**決定2**（record family + schema 版、measurement/annotation 族）、**決定3**（正準 event_time + 出自併載）、**決定4**（挿入順 `(epoch, seq)` カーソル、pub_seq は publication log 実体で readings.seq と別採番）、**決定5**（第一波=外向き HTTP push、at-least-once + 冪等 publication_id、再送権威=gateway outbox）、**決定6**（target registry・per-target cursor・archive フラグ・auth 骨子）、**決定7**（custody 範囲・R17 4クラスパージ順序）の各 MVE 分。

### 2.2 codex 独立レビュー裁定（2026-07-04, xhigh, read-only）
アーキテクチャ案を D7 と実コードに照合。私のコードベース主張7件は全て「確認」（幻覚なし）。指摘8件の裁定:

| # | 指摘 | 裁定 |
|---|---|---|
| 1 | annotation 族の丸ごと繰り延べは契約違反 | 採用 → §5（epoch_start 実装、他トリガ封じ） |
| 2 | retention を現構造(received_at cutoff)のまま足すと未ack正本を消す | 採用 → §8 作り替え |
| 3 | cursor だけでは ack後フロア不可、`archive_acked_at` が要る | **部分採用/一部棄却**: フロア遵守は採用。ただし**フロアはデータ年齢(received_at)基準**（[台帳:115](../../../../docs/redesign/responsibility-ledger.md)「正常時のデータ残高≒フロア分のみ」）なので `archive_acked_at` は不要 = ack水位 + readings.received_at + フロア定数で足りる。遠隔ack時刻案は過剰として棄却 |
| 4 | 検疫解除 renumber 繰り延べは既存解除操作を封じないと穴 | 採用 → §9 ガード |
| 5 | publication_id は再送・クラッシュ後も安定必要 | 採用 → §10 決定的ID |
| 6 | target 登録骨子不足（版交渉/疎通スモーク/監査） | 部分採用 → §8 最小ガード、R14 本体は E へ |
| 7 | measurement JSON フィールド不足、readings.seq を出口IDにするな | 採用 → §7 全フィールド固定 |
| 8 | readings.seq migration コメントが stale | 採用 → 触る時に修正 |

---

### 2.3 codex spec-eval round-2 裁定（2026-07-05, xhigh, read-only）
書き上げた spec を codex に再照合。**裁定#3(フロア=データ年齢)は独立に支持**（archive_acked_at 不要を再確認、ただし epoch guard 必須）。コードベース主張は全て確認。新規8指摘を裁定:

| # | 重大度 | 指摘 | 裁定 | 反映 |
|---|---|---|---|---|
| 1 | 高 | R22 restore 後の旧 cursor_pub_seq が新 epoch に誤適用（未配送を ack 済み扱い→誤パージ） | 採用 | §4.2/§6.4/§8.2 に **epoch guard** 明記 |
| 2 | 高 | target_registry を平文 R22 snapshot に入れると bearer token 漏洩（D2:75/98 暗号化必須） | 採用 | §12 反転: token/target を snapshot に**含めない**、復元後再登録 |
| 3 | 高 | publication_id 決定性が spec 未担保（バッチ組成規則 D7:276 は Wave 1 spec 宿題） | 採用 | §6.2/§10 に決定的バッチ組成規則を固定 |
| 4 | 高 | 検疫解除ガード(§9 soft)は archive 消費者に無音 custody 欠落、hard reject が契約忠実 | **保留→ユーザー裁定** | 承認済み §9 と衝突。§9 に ⚠、報告で再提示 |
| 5 | 中 | publication_log/readings ライフサイクル(outbox prune)未定義 | 採用 | §4.1/§8.3 に prune 規則 |
| 6 | 中 | retention 作り替えで既存機能(dedup TTL/検疫失効/statvfs ラッチ/health)を落とす危険 | 採用 | §8.1 に**維持**明記 |
| 7 | 中 | custody_lost 封じの逆圧が未確定(どの水位でどの ingress に deferred) | 採用 | §8.2 に watermark→collector 逆圧経路 |
| 8 | 中 | epoch_start enqueue 冪等性未定義(二重 enqueue) | 採用 | §4.1/§5.2 に UNIQUE(epoch,subtype) |

reality-check 補足: `core/collector/src/actor.rs:314` は現状 `insert_reading_v3` の `seq` を破棄している → enqueue で捕捉が必要（§6.1/§15）。

## 3. コンポーネントとクレート構成

### 3.1 新クレート `core/publish`
既存の `core/<name>` パターンに従う（自前 `pub const MIGRATIONS`、`store.rs` は `&Connection` を取る、gateway/main.rs と gatewayctl/main.rs の migration 連結2箇所へ1行追加、`Cargo.toml` members へ1行）。

- **責務**: publication log（outbox）と target registry のスキーマ・型・store 関数。純データ層（HTTP は持たない）。
- **依存**: `core/storage`（Migration/DbHandle）、`core/ledger`（epoch/generation 読み取り、監査イベント append）。readings の実体は `core/timeseries`。

### 3.2 push 配送タスク（`iotkit-gateway`）
- **責務**: 常駐 tokio タスク。target cursor 以降の outbox 行を有界バッチ化 → JSON レコード列を構築 → per-target token + HTTPS で POST → 同期 ack → cursor 前進。失敗で retry/backoff。collector/retention と同じ「gateway が spawn する常駐タスク」形。
- **依存**: `core/publish`（outbox/target 読み・cursor 更新）、`core/timeseries`（measurement 実体の JOIN）、`core/ledger`（series_key/epoch）、`reqwest`。

### 3.3 target 管理 CLI（`iotkit-gatewayctl`）
- **責務**: `gatewayctl target add|list`。登録時ガード（§8）。既存 cmd/ モジュール（devices/registry/query/snapshot）と同型。

### 3.4 enqueue フック（`core/collector`）
- collector の envelope 処理 Immediate Tx 内（`core/collector/src/actor.rs`、reading insert と同一 Tx）で、非検疫 measurement 行1件につき outbox 行1件を enqueue。

### 3.5 retention 作り替え（`iotkit-gateway/src/retention.rs`）
- 現 `purge_readings_before(received_at cutoff)` を custody 対応パージ（§8）へ置換。

---

## 4. データモデル（新テーブル、`core/publish` migration）

### 4.1 `publication_log`（outbox）
```
pub_seq        INTEGER PRIMARY KEY AUTOINCREMENT,  -- 出口seqの実体（D7决定4）。readings.seq とは別採番
epoch          TEXT    NOT NULL,                   -- 採番時点の台帳epoch（ledger_meta）
kind           TEXT    NOT NULL,                   -- 'measurement' | 'annotation'
subtype        TEXT,                               -- kind=annotation: 'epoch_start' 等。measurement は NULL
reading_seq    INTEGER,                            -- kind=measurement: readings.seq への参照（JOINで実体化）。annotation は NULL
annotation_json TEXT,                              -- kind=annotation: 自己完結ペイロード。measurement は NULL
created_at     INTEGER NOT NULL
-- 冪等性（codex#8）: 部分 UNIQUE index  UNIQUE(epoch, subtype) WHERE kind='annotation'
--   → epoch_start の二重 enqueue（起動時再検知後 ack 前クラッシュ）を DB 制約で排除
```
- `pub_seq` は DB ライフタイムで単調（SQLite AUTOINCREMENT、再利用なし）。epoch を併載するので `(epoch, pub_seq)` が大域一意。R22 restore 後は outbox が空（snapshot 非含有）+ epoch 新規なので、新 epoch 下で pub_seq が 1 から再スタートしても `(epoch,pub_seq)` は旧世代と衝突しない。
- **カーソル同一性は必ず `(epoch, pub_seq)` の複合**（codex#1）。pub_seq 単独で ack/配送/パージ判定をしてはならない（epoch 跨ぎの誤適用を防ぐ。§6.4/§8.2 の epoch guard）。
- measurement は実体を持たず readings を参照（重複保存回避）。annotation は backing row が無いのでペイロードを inline 保存。
- **不変条件**: outbox は非検疫行のみ持つ。検疫行は解除まで採番しない（D7决定4）。MVE では検疫解除 renumber を封じる（§9）。

### 4.2 `target_registry`
```
target_id           TEXT PRIMARY KEY,       -- 運用者指定の識別子
endpoint_url        TEXT NOT NULL,          -- https:// のみ（§8で強制）
credential_token    TEXT NOT NULL,          -- per-target bearer（Authorization: Bearer）
archive_responsible INTEGER NOT NULL DEFAULT 0,  -- スモーク成功まで 0（§8）
schema_version      INTEGER NOT NULL,       -- 合意した measurement 族 major 版（MVE=1）
cursor_epoch        TEXT,                   -- 最後に ack された epoch
cursor_pub_seq      INTEGER NOT NULL DEFAULT 0,  -- 最後に ack された pub_seq（この値まで配送済み・パージ許可）
created_at          INTEGER NOT NULL
```
- MVE は1行のみ運用（複数 target は DEFERRED）。
- `(cursor_epoch, cursor_pub_seq)` = target 別カーソル（D7决定6、target 単位保持）。**両方セットで判定**（epoch guard、§6.4/§8.2）。
- **`credential_token` は秘密** → R22 snapshot に含めない（§12、平文退避を避ける）。
- HTTPS 強制・疎通スモーク・登録監査は §11。

---

## 5. record family とストリーム

### 5.1 measurement 族（JSON、§7 に全フィールド）
一時点=1レコード（D7决定2）。`values` は単一 series の1観測の値ベクトル（多チャネル束ねでも時間ブロックでもない）。

### 5.2 annotation 族（最小: `epoch_start` のみ）
- 全 target 共有 seq（購読フィルタ不可、D7决定2）。MVE は single-target なので実質同義。
- **`epoch_start`**: R22 restore で台帳 epoch が更新された時、新 epoch 下の**最初の outbox 行**として enqueue。ペイロード = `{prior_epoch}`（[snapshot 復元が記録する `epoch_renewed` 監査イベント](../../../../docs/redesign/decisions/D7-exit-contract.md) の旧値、D7决定8B: 新 epoch annotation には旧 epoch ID のみ記載）。消費者は自分のカーソルと突合し、新 epoch の pub_seq 1 から載り替える。**冪等（codex#8）**: §4.1 の部分 UNIQUE(epoch,subtype) により、起動時再検知後 ack 前クラッシュでも二重 enqueue しない。
- custody_lost / 検疫遷移 annotation は MVE では**発生させない**（§8/§9 でトリガ封じ）。

---

## 6. データフロー

### 6.1 取り込み → outbox enqueue（collector Tx 内、exact-once）
1. collector が envelope を Immediate Tx で処理（既存）。
2. 各 reading item を registry policy 評価 → `row_quarantined` 決定（既存）。
3. `insert_reading_v3` で readings 挿入。**現状 `core/collector/src/actor.rs:314` は返り値 `seq` を破棄しているので、enqueue のため `seq` を捕捉する（codex reality-check）**。
4. **[新] `row_quarantined == false` の時のみ**、同一 Tx で `publication_log`(kind=measurement, reading_seq=<捕捉した seq>, epoch=<Tx冒頭で読んだ ledger_epoch>) を挿入。pub_seq は AUTOINCREMENT で採番。
5. Tx commit（既存の generation bump と同一 commit）。

電源断は正常系: enqueue は reading 挿入と同一 Tx なので、reading があって outbox が無い/その逆は起きない（クラッシュ整合性）。

### 6.2 push サイクル（常駐タスク）
1. target を1行読む。**effective cursor 決定（codex#1 epoch guard）**: `target.cursor_epoch == current_epoch` なら `cursor = target.cursor_pub_seq`、そうでなければ（R22 restore 後の旧 epoch 等）`cursor = 0`。
2. **決定的バッチ組成（codex#3, D7:276 宿題を確定）**: `SELECT ... FROM publication_log WHERE epoch = current_epoch AND pub_seq > cursor ORDER BY pub_seq ASC LIMIT N`（件数上限 N または byte cap の先に達した方で切る。切り口は pub_seq 昇順で決定的）。**カーソルは排他（`pub_seq > cursor`）**。cursor_start = cursor+1、cursor_end = バッチ末尾 pub_seq。
3. measurement 行は readings を JOIN + series/epoch メタで JSON 実体化。annotation 行は inline JSON をそのまま。
4. バッチの `publication_id = hash(target_id, current_epoch, cursor_start, cursor_end)`（**決定的**、§10）。
5. `POST endpoint_url`、`Authorization: Bearer <token>`、body=JSON バッチ、HTTPS。
6. 同期レスポンスで ack（消費者が「pub_seq end まで耐久化」を返す）。
7. ack 成功 → `target_registry.cursor_epoch = current_epoch`、`cursor_pub_seq = cursor_end` へ前進（Tx 永続化。epoch も必ず更新）。
8. 失敗（接続断/非2xx/タイムアウト）→ retry with bounded exponential backoff。shutdown シグナルに応答。

### 6.3 ack → custody → パージ
- retention タスク（§8）が target の `cursor_pub_seq`（archive_responsible=1 の行）を読み、`pub_seq ≤ cursor_pub_seq` かつ `received_at < now - フロア` の readings をクラス①として削除。

### 6.4 R22 restore → epoch 載り替え（codex#1/#2 反映）
- restore で epoch 更新（既存）。outbox/readings は空（snapshot 非含有）。**target_registry も snapshot 非含有（§12）なので target は消え、運用者が再登録**。
- 新 epoch の最初の outbox 行として `epoch_start` annotation を enqueue（`prior_epoch` = restore が記録する `epoch_renewed` 監査の旧値。冪等は §4.1 の UNIQUE(epoch,subtype)）。→ push で消費者へ。
- **epoch guard**: 将来 target 永続化を実装して旧 `cursor_epoch` を持つ target が復元されても、push/retention は `cursor_epoch != current_epoch` を検知し effective cursor=0、新 epoch を pub_seq 1 から扱う。消費者は epoch 不一致で再 baseline（D7决定8B）。

---

## 7. measurement レコードスキーマ（全必須フィールド）

codex#7 採用。出口 ID は `pub_seq`（**readings.seq を出さない**）。

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
- レコード同一性 = `(epoch, pub_seq)`。消費者はこれで冪等 upsert（D7决定4）。

annotation `epoch_start`:
```json
{ "family": "annotation", "schema_version": 1, "epoch": "<新epoch>", "pub_seq": <n>,
  "subtype": "epoch_start", "prior_epoch": "<旧epoch>" }
```

---

## 8. custody と retention（R17 作り替え）

codex#2/#3 採用。**「追加」でなく置換**。

### 8.1 現状（Wave 0）と維持する機能（codex#6）
`retention.rs` は同一周期で: readings の `purge_readings_before(received_at cutoff)` + **dedup TTL パージ** + **検疫期限失効** + **statvfs 水位ラッチ** + **health 更新** を行う（`iotkit-gateway/src/retention.rs`）。
- **本 spec が変えるのは readings パージの判定規則だけ**。dedup TTL パージ・検疫期限失効・statvfs 水位ラッチ・health 更新は**維持**する。custody/ack を知らない `received_at` cutoff 削除を §8.2 の custody 対応パージへ置換する。

### 8.2 MVE の custody 対応パージ
D7决定7 の4クラス順序のうち **MVE はクラス① のみ実装**:
- **クラス① eligibility（codex#1 epoch guard）**: `target.archive_responsible=1` かつ **`target.cursor_epoch == current_epoch`** の時のみ有効。ある reading が「ack 済み」= **`publication_log.epoch == target.cursor_epoch == current_epoch` かつ `pub_seq <= target.cursor_pub_seq`**。epoch 不一致時は新 epoch 行を一切 ack 済み扱いにしない（effective cursor=0）。
- **削除対象**: 上記で ack 済み **かつ** `readings.received_at < now - 最低保持フロア` の行。
- **最低保持フロア**: [D1:154](../../../../docs/redesign/decisions/D1-ingest-model.md) / [台帳:115](../../../../docs/redesign/responsibility-ledger.md) = 既定 72h・設定可。**データ年齢(received_at)基準**（ack 相対でなくデータの新しさ。「正常時のデータ残高≒フロア分のみ、断線時のみ水位上昇」）。→ `archive_acked_at` 列は不要（codex#3、round-2 で codex 支持）。
- **未ack正本は保護**: ack 済みでない readings は received_at が古くても**削除しない**（従来の無条件時刻カットオフ削除を廃止）。
- **圧力時の逆圧（codex#7、custody_lost トリガ封じ）**: クラス①を出し切っても statvfs 高水位が続く場合、クラス④（未ack正本削除+custody_lost）は MVE では**実装しない**。代わりに **statvfs 高水位ラッチ時、collector が新規取り込みに D1 `deferred` ack を返す/受理を止める逆圧**を掛ける（水位→collector への圧力伝達経路を新設）。R12 に per-target 配送状態（遅延・target 死亡）を公開。→ 未ack正本の無音破棄を起こさない。水位閾値の具体値は writing-plans で確定。

### 8.3 archive target 不在時 と outbox prune（codex#5）
- **archive target 不在時**: target 未登録 or archive_responsible=0 は custody 約束なし。パージ上限は**フロアのみ**（received_at < now - フロア）に縮退（バッファ最小化）。実装は「有効な archive cursor があればそれを上限、無ければフロアのみ」。
- **outbox prune 規則**: `publication_log` 行は「配送 retry のため ack まで保持」。以下で prune:
  - ack 済み（`epoch==cursor_epoch==current` かつ `pub_seq <= cursor_pub_seq`）の行は、対応 readings のクラス①削除と**同一 retention 周期**で prune（dangling ref を作らない）。
  - archive target 不在の floor-only 削除で readings を消す時は、対応する outbox 行も同時に prune（outbox が readings より長生きしない・無制限成長しない）。
  - 旧 epoch の残 outbox 行はフロア基準で prune 可。

---

## 9. 検疫解除経路のガード

> ⚠ **未決（codex spec-eval#2 の#4、ユーザー裁定待ち）**: 下記の承認済み方針（文書化+監査、hard reject なし）に対し、codex は「archive target 登録中は過去検疫 readings が無音で custody 欠落する（audit は R10 消費者に届かない）。MVE で renumber しないなら archive target 存在時は**解除を hard reject** するのが D7:33/D5:89 に契約忠実」と指摘。承認済み方針と衝突するため保留。→ 推奨は hard reject（小さく契約忠実、renumber は出口契約拡張へ）。

Wave 0 の alias 定義（`registry::define_alias` → `release_series_quarantine_for_key_checked`）は series 検疫フラグを clear するが、過去 readings 行は触らず outbox 化もしない。MVE は renumber を封じるので、**無音の配送欠落**を防ぐガードを入れる:

- **方針（承認済み: 文書化+監査）**: 解除は従来どおり series フラグ clear（以後の新規 readings は非検疫として outbox に流れる）。ただし登録 target がある状態での解除時、「解除された series の過去検疫 readings は本 MVE では出口へ再配送されない（R11 では読める）」旨を **ledger 監査イベントに記録**。ハード拒否はしない。
- 完全な検疫解除 renumber（過去行の新規採番 + measurement 再配送 + 検疫遷移 annotation）は次段。

---

## 10. クラッシュ整合性と冪等性（codex#5 採用）

- **決定的 publication_id**: `publication_id = hash(target_id, current_epoch, cursor_start, cursor_end)`。§6.2 の決定的バッチ組成（epoch 一致 + `pub_seq > cursor` + ORDER BY pub_seq ASC + LIMIT）で、同一 cursor から同一 `(cursor_start, cursor_end)` が再現 → 同一 ID（codex#3）。
- **push 後 ack 前クラッシュ**: 再起動で cursor 不変 → 同一範囲を再バッチ → 同一 publication_id → 消費者が dedup。
- **ack 後 cursor 永続化前クラッシュ**: 同上（cursor 不変なので再送、消費者 dedup）。レコード同一性 `(epoch,pub_seq)` でも二重吸収。
- cursor 前進は ack 成功後の単一 UPDATE（Tx）。at-least-once + 冪等（D7决定5）。

---

## 11. auth 骨子と target 登録ガード（codex#6 部分採用）

- **配送接続**: gateway=client, consumer=server。gateway が per-target token を `Authorization: Bearer` で提示、消費者がそれで gateway を認証。HTTPS(rustls) でチャネル保護。相互認証・証明書ピンは R19（sub-project B）。
- **`gatewayctl target add` ガード（MVE 最小）**:
  1. `endpoint_url` は `https://` のみ受理（平文拒否）。
  2. 登録を ledger 監査イベントに記録。
  3. 登録時**疎通スモーク**（空/ping バッチを POST し 2xx+ack を確認）。**スモーク成功まで archive_responsible=0**（誤設定 archive target へ配送→未受信のままパージ＝custody 損失を防ぐ）。
  4. `schema_version` 一致チェック（MVE=1、不一致は拒否）。
- 完全な R14 型付き操作（権限段階・dry-run・全操作カタログ）は sub-project E。

---

## 12. R22 連携（codex spec-eval#2 反映）

- **target_registry を平文 R22 snapshot に含めない**。理由: `credential_token` は秘密で、現 R22 snapshot は平文 JSON 書き出し（`iotkit-gatewayctl/src/cmd/snapshot.rs:126`）。[D2:75/98](../../../../docs/redesign/decisions/D2-data-authority-topology-operations.md) は secrets 非空 snapshot の暗号化を必須とする。R22 暗号化は MVE スコープ外なので、**target/token を snapshot に入れない**（当初案を反転）。
- **復元後の target 再登録**: R22 restore（箱交換）後は epoch fence で消費者がどのみち再 baseline する（§6.4）。運用者が `target add` で target を再登録（credential 再発行 + スモークで archive_responsible 再有効化）。target config の暗号化退避は R22 暗号化と同時に後続 sub-project へ。
- **publication_log（outbox）は data-plane** → readings と同様 snapshot に**含めない**。復元後は空 + epoch 新規。

---

## 13. エラーハンドリング

- push 失敗（接続断/非2xx/タイムアウト）: bounded exponential backoff（上限あり）、shutdown への応答性確保（既存タスクの select! パターンに倣う）。
- スモーク失敗: `target add` はエラーで中断、archive_responsible を立てない。
- 消費者が長期死亡: cursor 進まず outbox 滞留 → フロア超過分もパージできず水位上昇 → R12 警報 + ingest 逆圧（§8.2）。MVE は「単一の制御された archive 消費者」前提でこれを許容（文書化された制限）。
- 全体障害と個別 target 障害を混同しない（MVE は single-target なので単純だが、エラー型は target_id 文脈を持つ）。

---

## 14. テスト戦略

- **適合テスト消費者**: リポジトリ内フィクスチャ（バッチ POST を受け、pub_seq end を ack する極小 HTTP サーバ。`axum` 不使用でも `reqwest` のテスト用に tokio + 単純 listener で可）。
- **end-to-end custody ループ**: reading 挿入 → outbox enqueue → push → ack → cursor 前進 → retention クラス①で当該 readings 削除、を1本で検証。
- **クラッシュ冪等性**: push 後 ack 前 / ack 後 cursor 前 の擬似クラッシュで、再送が同一 publication_id・`(epoch,pub_seq)` で消費者 dedup されること。
- **検疫除外**: 検疫行は outbox に入らない（配送されない）。
- **retention フロア**: ack 済みでもフロア(received_at)内は削除されない。未ack正本は received_at が古くても保護される。
- **epoch 載り替え**: R22 restore 後、epoch_start annotation が新 epoch 最初の pub_seq で配送される。
- **target 登録ガード**: http:// 拒否、スモーク失敗で archive_responsible が立たない、schema_version 不一致拒否。
- 契約(D7)を見るテストであって実装詳細に張り付かない。malformed ack・oversized batch・shutdown 競合を含める。

---

## 15. 統合ポイント（触るファイル）

| 対象 | 変更 |
|---|---|
| `core/publish/`（新規） | クレート・migration・store。`Cargo.toml` members +1 |
| `iotkit-gateway/src/main.rs` | migration 連結に publish 追加、push タスク spawn |
| `iotkit-gateway/src/retention.rs` | custody 対応パージへ作り替え（§8） |
| `iotkit-gateway/src/health.rs` | per-target 配送状態を health.json へ（R12、D7决定9 最小） |
| `core/collector/src/actor.rs` | Tx 内 outbox enqueue フック（§6.1）。**現状 :314 で破棄している `insert_reading_v3` の `seq` を捕捉**。統合水位時の逆圧（§8.2）も collector 側 |
| `iotkit-gatewayctl/src/cmd/target.rs`（新規）+ `main.rs` | `target add|list`、migration 連結 +1 |
| `iotkit-gatewayctl/src/cmd/snapshot.rs` | **変更なし**（§12 反転: target/token・outbox とも snapshot 非含有。R22 暗号化まで target 永続化はしない） |
| `core/registry/src/store.rs`（or 呼出側） | 検疫解除ガードの監査記録（§9） |
| `core/timeseries/migrations/0004_readings_v3.sql` | stale コメント修正（codex#8） |

---

## 16. 宿題ピン（設計スペック段階で確定すべき値）

- 最低保持フロア既定値: 72h（[D1:155](../../../../docs/redesign/decisions/D1-ingest-model.md)）。設定手段（config or ledger_meta）は writing-plans で確定。
- 有界バッチ上限 N（件数/バイト）: writing-plans で確定（RPi メモリ現実、D7决定5 有界バッチ）。
- ack レスポンス形式（pub_seq end 返却の JSON 形）: measurement 族スキーマと合わせ writing-plans で確定。MVE は all-or-nothing バッチ ack（部分 ack は DEFERRED）。
- retry backoff パラメータ・push 間隔。

---

## 17. 後続 sub-project への送り

| 送るもの | 送り先 |
|---|---|
| annotation 族フルセット（custody_lost・検疫遷移）+ クラス④パージ + 検疫解除 renumber | 出口契約 拡張（Wave 1 後続） |
| publication snapshot + bounded backfill | 出口契約 拡張 |
| multi-target fan-out・購読フィルタ・cursor_expired/gap 復帰 | 出口契約 拡張 |
| R14 型付き操作フレームワーク（target 操作の権限段階・dry-run） | sub-project E（制御面） |
| R19 完全認証（相互認証・証明書ピン・秘密管理） | sub-project B（セキュリティ基盤） |
| 統合メタデータ読み面（D7决定9 の unified endpoint、必要なら） | R11 拡張 |
