# Wave 1 出口契約 MVE（R10 歩く骨格）設計仕様

> **For agentic workers:** この spec は brainstorming の成果物。実装は writing-plans → subagent-driven-development で行う。本文書は「契約」ではなく「Wave 1 実装 spec」——契約正本は [D7](../../redesign/decisions/D7-exit-contract.md)。

**Goal:** アーカイブ責任消費者1台へ measurement レコードストリームを外向き HTTP push で at-least-once 配送し、その ack で正本(readings)のパージを許可する「end-to-end custody ループ」の最小実装。

**Architecture:** 新クレート `core/publish`（publication log[outbox] + target registry）+ `iotkit-gateway` 常駐 push タスク + `iotkit-gatewayctl target` 管理 CLI。measurement/annotation が共有する単調 `pub_seq` を outbox に採番し、`(epoch, pub_seq)` を出口カーソル同一性とする。ack 済み水位を R17 retention のパージ判定に配線する。

**Tech Stack:** Rust / tokio / rusqlite(SQLite WAL) / `reqwest`（gateway push タスク=`default-features=false, features=["json","rustls-tls"]`、gatewayctl smoke=`features=["blocking","json","rustls-tls"]`）。

---

## 1. スコープ

### 1.1 位置づけ
- Wave 1「他人に配れる」の最初の sub-project（出口契約 R10）。Wave 0（動く最小、全4計画 master マージ済み）の上に立つ。
- 契約は [D7](../../redesign/decisions/D7-exit-contract.md) で確定済み。本 spec は契約を再定義しない。[D3](../../redesign/decisions/D3-process-and-wave-decisions.md) 読み替え規則「契約は本番形のまま実装だけ削る」に従い、契約の一部だけを実装する。
- MVE = **歩く骨格**（single-target・measurement 族中心）。

### 1.2 実装する（IN）
1. `core/publish` クレート（outbox + target registry のスキーマ/store）
2. publication log（outbox）: measurement/annotation 共有の単調 `pub_seq`、`(epoch, pub_seq)` カーソル同一性
3. 単一 target registry（HTTPS 限定・per-target token・archive_responsible・cursor・schema_version）
4. 外向き HTTP push 配送タスク（有界バッチ POST・同期 ack・cursor 前進・retry/backoff・**決定的** publication_id）
5. annotation 族の**最小**（`epoch_start` のみ配送）
6. custody→R17 retention 作り替え（クラス① = ack 済み ∧ フロア超過のみ削除、未ack**正本**は保護。検疫行は保護しない）
7. auth 骨子（per-target bearer + HTTPS）
8. target 管理 CLI（`add|list|rotate-token|remove`）と登録ガード（HTTPS 強制・ledger 監査・疎通スモーク成功まで archive_responsible 無効・v1 版チェック）
9. R22 連携（target/token・outbox とも snapshot 非含有、復元後 target 再登録。§12）
10. 適合テスト消費者（リポジトリ内フィクスチャ）

### 1.3 繰り延べる/封じる（DEFERRED / SEALED）
| 項目 | 扱い | 封じ方（無音の穴を作らない） |
|---|---|---|
| custody_lost annotation | SEALED | クラス④（**保存済み**未ack正本の削除）を実装しない＝保存済みデータを消さないので custody_lost が発生しない。能動逆圧も新設しない。圧力時は R12 が事前警報し、放置時は front-door drop（スループット由来・ディスク水位に非連動）でなく最終的に `ENOSPC` の明示的書込失敗に至る（無音損失なし、iter2 [中] で story 修正）。詳細 §8.2 |
| 検疫遷移 annotation / 検疫解除・付与 renumber | SEALED | 両方向を **hard reject でガード（§9.1 解除 / §9.2 付与=replace-undo）**: archive target 登録中は解除・遡及検疫とも override 無しで拒否。無音の custody 欠落を作らない。過去検疫行は R11 で読める |
| 行レベル検疫 readings の再配送 | SEALED（明示制限） | 行レベル検疫（out_of_range/device_quarantined）は pub_seq を持たず custody 対象外。**保護せず**、フロア（§8.2 の新規 purge branch）で消える（無限保持しない、§8.2/§8.3） |
| publication snapshot | DEFERRED | R22 restore 時は `epoch_start` annotation で新 epoch へ載り替え（新 epoch は最小 pub_seq から）。復元前データの backfill は D7決定8B どおり約束しない |
| bounded backfill | DEFERRED | — |
| multi-target fan-out / 購読フィルタ | DEFERRED | single-target 固定。archive_responsible target に購読フィルタは元々不適用（D7決定7） |
| cursor_expired / gap 復帰通知 | DEFERRED | MVE はフロア内保持のため、単一 target が追随できる前提。長期断は R12 事前警報＋保存済み正本保持（放置は ENOSPC 明示失敗、§8.2） |
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
| 3 | cursor だけでは ack後フロア不可、`archive_acked_at` が要る | **部分採用/一部棄却**: フロア遵守は採用。ただし**フロアはデータ年齢(received_at)基準**（[台帳:115](../../redesign/responsibility-ledger.md)「正常時のデータ残高≒フロア分のみ」）なので `archive_acked_at` は不要。遠隔ack時刻案は過剰として棄却 |
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

reality-check（両レビューで確認）: `core/collector/src/actor.rs:314` は `insert_reading_v3` の `seq` を破棄（→ 捕捉が必要）。`DbHandle` は単一 `Arc<Mutex<Connection>>`（`core/storage/src/handle.rs:16`、プロセス全体で直列化）。`PRAGMA foreign_keys=ON`（`core/storage/src/lib.rs:20`）。`renew_epoch` は `epoch_renewed{old_epoch}` を記録（新 epoch は記録しない、`core/ledger/src/store.rs:699`）。既存 retention/health タスクは bare loop で `select!` shutdown を持たない。

### 2.5 iteration 2（再レビュー）裁定（2026-07-05, codex + Sonnet 並行）
iter1 修正版(b9f7157)を再レビュー。codex は 7/8 landing OK、Sonnet は健全性の大半を trace 確認（epoch guard NULL・restore 書込レース・zero-ack・単一 Tx FK 順・3スコープ push・byte-cap・ack 検証・gatewayctl blocking を「実際に健全」と確認）。両者が**独立に同一の [高] を2件**指摘（＝強シグナル）+ [中]/[低]。全採用:

| 出所 | 重大度 | 指摘 | 反映 |
|---|---|---|---|
| both | 高 | `target remove`→add rotation の「target 不在」窓で §8.3 floor-only が保存済み未ack正本を誤パージ（D2:30 違反） | §3.3/§11: in-place `rotate-token`、`remove` は未ack時拒否/`--abandon-custody`。§8.3 floor-only は監査付き remove 後のみ到達 |
| Sonnet | 高 | 検疫 readings の purge を「既存の検疫期限失効」に誤帰属（実際は devices.state のみ、readings を消す `purge_readings_before` は置換される）→ 検疫行 purge 経路が消滅 | §8.1/§8.2: 新規 floor branch を明示 commission、保護は pub_seq 付き未ack のみ |
| Sonnet | 中 | pub_seq=1 は非 pristine で不成立（AUTOINCREMENT 継続） | §4.1/§5.2/§6.4: 「新 epoch 内最小 pub_seq」に緩和、test も ==1 を assert しない |
| Sonnet | 中 | front-door drop はスループット由来でディスク非連動→圧力 story が不正確 | §8.2/§13/§1.3: R12 事前警報→放置時 ENOSPC 明示失敗（無音損失なし）に修正 |
| both | 中 | 旧 epoch outbox prune が readings を orphan 化 / 非 pristine restore 残留 | §8.3/§12: 旧 epoch outbox+readings をペアで同一 Tx 削除、restore 前提を writing-plans で確定 |
| both | 中 | §14 に圧力劣化テスト等が実際には無い | §14 に rotation 窓/検疫 floor/旧 epoch orphan/圧力無音損失テスト追加 |
| Sonnet | 低 | 2nd target 行を拒否するガード無し | §11 add ガードに追加 |
| Sonnet | 低 | 引用行ズレ（handle.rs:14→16, snapshot.rs:126→129） | 修正 |

### 2.6 iteration 3（再レビュー）裁定（2026-07-05, codex + Sonnet 並行）
iter2 修正版(72463d7)を再レビュー。**codex は新規[高]なし**（stale 整合の[中]3/[低]1）。**Sonnet は新規[高]2件**を独立発見（codex 未検出＝別系統の価値）。全採用:

| 出所 | 重大度 | 指摘 | 反映 |
|---|---|---|---|
| Sonnet | 高 | `replace-undo` の遡及検疫が既 pub_seq 未ack 行を quarantined=1 に反転→§8.2 保護外→floor purge→outbox dangling（無音損失/FK）。付与方向ガード欠落 | §4.1/§9.2: `mark_readings_quarantined` が同一 Tx で outbox prune、既 ack 含む時は archive 登録中 override |
| Sonnet | 高 | NEW 退行: `remove` の未ack検査に TOCTOU（gatewayctl は別プロセス、in-proc Mutex 跨がず）→検査後に daemon が新 pub_seq commit→floor-only 無音パージ | §3.3/§11: 検査→削除/拒否＋監査を単一 Immediate Tx |
| Sonnet | 中 | rotate-token の再スモークが archive_responsible を一時 0 にすると §8.3 第2 disjunct が開く。スモーク失敗挙動未定 | §3.3/§11: archive_responsible 終始 1、スモーク失敗は token ロールバック |
| codex | 中 | target rotation の stale 矛盾（§4.2/§1.2/§15 に remove+add 残骸/CLI 一覧漏れ） | rotate-token へ統一 |
| codex | 中 | §1.3 cursor_expired が front-door 劣化文言（stale） | ENOSPC story へ整合 |
| codex | 中 | §9 「検疫期限失効まで保持」が §8.2 floor purge と矛盾 | 「floor purge まで R11 可読」へ |
| both | 低 | abandon-custody の Tx 原子性 / §6.4 restore 過度一般化 / pre-upgrade readings | §9.2 Tx、§6.4 pristine 限定、§8.2 一文 |

### 2.7 iteration 4（再レビュー）裁定（2026-07-05, codex + Sonnet 並行）
iter3 修正版(8e587b1)を再レビュー。**Sonnet: 新規[高]なし**、iter3 の3修正を実 SQLite/コード照合で「architecturally sound」と確認、総評「close — mechanical punch list, not a design problem」。両者一致の実質新規は **in-flight race 1件**、他は整合クリーンアップ。全採用:

| 出所 | 重大度 | 指摘 | 反映 |
|---|---|---|---|
| codex[高]/Sonnet[中] | — | §9.2 の機械 prune が push in-flight（POST は lock 外、§6.2）と競合: 送信中の未 ack 行が prune されても消費者へ届き、override(既 ack のみ)を素通り | §9.2 を **§9 対称の hard-reject に一本化**（archive 稼働中は replace-undo を override 無しで拒否）→ in-flight race 消滅、二段構え撤廃 |
| codex | 中 | §8.3「outbox prune は readings 削除とペア」が §9.2（readings 残し outbox のみ prune）と矛盾 | §8.3 に §9.2 を明示例外として追加 |
| both | 中 | §15 target.rs 行が rotate-token 漏れ | 修正 |
| Sonnet | 中 | §14 検疫 purge テスト文言が「検疫期限失効＋フロア」誤帰属を再導入 | §14 を新規 floor branch（§8.2）へ |
| Sonnet/codex | 低 | §6.2 handle.rs:14→16 / §5.2 trigger 因果 / §16 参照 / §9.2 epoch-guard 句 | §6.2/§5.2/§16 修正、§9.2 は hard-reject 化で predicate 消滅 |

### 2.8 iteration 5（収束確認）裁定（2026-07-05, codex + Sonnet 並行）→ **収束**
iter4 修正版(0a018b1)を再レビュー。**両者とも新規[高]/[中]の設計問題なし**。Sonnet は §9.2 hard-reject が in-flight race を design/code 両面で閉じたことを確認、全 file:line 引用も検証済み（no hallucinated/drifted citations）。残りは one-line doc 整合のみで**両者「追加レビュー周回不要」明言**。修正を当て収束:

| 出所 | 重大度 | 指摘 | 反映 |
|---|---|---|---|
| both | 中 | §14 遡及検疫テストが旧「既 ack だけ override」条件のまま（§9.2 hard-reject に未波及） | §14 を無条件 hard-reject へ |
| both | 低 | §9.1 見出し欠如 / §1.3 が §9 のみ参照（§9.2 漏れ） | §9.1 見出し追加、§1.3 に §9.2 |

**収束（iter5）**: 設計面は codex×3 + Sonnet×3 の敵対的並行レビュー5周で全 substantive 指摘（14→8→6→2→0 design）を解消。writing-plans へ。

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
- **責務**: `gatewayctl target add|list|rotate-token|remove`。登録時ガード（§11）。既存 cmd/ モジュール（devices/registry/query/snapshot）と同型。
- **`rotate-token`（token rotation の正路）**: target 行と cursor を**保持したまま** `credential_token` を in-place で UPDATE し再スモーク。**remove+add を rotation に使わない**——remove で target が一瞬でも消えると §8.3 floor-only 窓が開き保存済み未ack正本を誤パージし得るため（iter2 [高]）。**`archive_responsible` は操作全体で 1 のまま変更しない**（再スモークで一時 0 にすると §8.3 floor-only の第2 disjunct が開く。iter3 [中]）。スモーク失敗時は token を旧値へロールバック（動く token を維持、archive_responsible=1 のまま）。
- **`remove`（decommission=明示的 custody 放棄）**: target 行と cursor を削除。**未 ack の pub_seq 付き正本が cursor 超で残っている場合は拒否**し、`--abandon-custody`（監査記録）でのみ強行。**gatewayctl は gateway と別プロセスなので、この『未 ack 検査→削除/拒否＋監査』は単一 `BEGIN IMMEDIATE` Tx で行う**（daemon が検査と削除の間に新 pub_seq を commit する TOCTOU を防ぐ。iter3 [高]）。無音の floor-only 誤パージを起こさない。
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
- `pub_seq` は DB ライフタイムで単調（SQLite AUTOINCREMENT、再利用なし）。epoch を併載するので `(epoch, pub_seq)` が大域一意。R22 restore 後は epoch 新規なので `(epoch,pub_seq)` は旧世代と衝突しない。pristine 交換箱では outbox 空で pub_seq は 1 から。**非 pristine（旧 outbox 残存）では AUTOINCREMENT が継続し pub_seq は 1 に戻らないが、正しさは絶対値でなく「新 epoch 内で単調」+ `(epoch,pub_seq)` 一意 + effective cursor=0（epoch guard）で保たれる**（iter2 [中]）。
- **カーソル同一性は必ず `(epoch, pub_seq)` の複合**。pub_seq 単独で ack/配送/パージ判定をしてはならない（epoch 跨ぎの誤適用を防ぐ。§6.4/§8.2 の epoch guard）。
- measurement は実体を持たず readings を参照（重複保存回避）。annotation は backing row が無いのでペイロードを inline 保存。
- **`reading_seq` と readings の FK / 削除順**: `PRAGMA foreign_keys=ON`（`core/storage/src/lib.rs:20`）。retention のクラス①削除は **outbox 行 prune → readings 行 delete の順**を単一 Immediate Tx で行う（§8.3）。FK を張る場合は `ON DELETE` 挙動、張らない場合は削除順を、writing-plans で確定（既定＝この削除順を守れば FK 有無どちらでも安全）。
- **不変条件**: outbox は非検疫行のみ持つ。検疫行は採番しない（D7決定4）。**遡及検疫**（`mark_readings_quarantined`、`device replace-undo` 経由で既存行を quarantined=1 に反転）も同一 Tx で対応 outbox 行を prune し、『検疫⇒pub_seq 無し』不変を保つ（iter3 [高]、§9.2）。MVE では検疫解除 renumber を封じる（§9）。

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
- **`credential_token` は秘密** → R22 snapshot に含めない（§12）。rotation は §3.3 の in-place `rotate-token`（remove+add ではない）。

---

## 5. record family とストリーム

### 5.1 measurement 族（JSON、§7 に全フィールド）
一時点=1レコード（D7決定2）。`values` は単一 series の1観測の値ベクトル（多チャネル束ねでも時間ブロックでもない）。

### 5.2 annotation 族（最小: `epoch_start` のみ）
- 全 target 共有 seq（購読フィルタ不可、D7決定2）。MVE は single-target なので実質同義。
- **`epoch_start`**: R22 restore で台帳 epoch が更新された時、新 epoch 下の**最初の outbox 行**として enqueue。ペイロード = `{prior_epoch}`（D7決定8B: 新 epoch annotation には旧 epoch ID のみ記載）。消費者は自分のカーソルと突合し、新 epoch の**最小 pub_seq**から載り替える（pristine では 1）。
- **trigger アルゴリズム（§6.4 でも参照）**: gateway 起動時、collector を spawn する**前**に判定する。(1) 最新の `epoch_renewed` ledger イベントが存在するか（過去に restore を経たか。`renew_epoch` は epoch 更新と `epoch_renewed` を原子的に記録、pristine 初回 boot はイベント無し）。**厳密な「この boot が restore 由来か」検知は不要**——存在すれば毎 boot で enqueue を試み、重複は §4.1 の部分 UNIQUE(epoch,subtype) が吸収する。(2) `prior_epoch` = その `epoch_renewed` イベントの `old_epoch`。(3) **初回 boot（`epoch_renewed` 無し）は enqueue しない**。
- **冪等**: §4.1 の部分 UNIQUE(epoch,subtype) により、起動時再検知後 ack 前クラッシュでも二重 enqueue しない。
- **順序保証**: collector spawn 前に enqueue するので、新 epoch の最初の measurement より前に pub_seq が付く（＝新 epoch 内で**最小 pub_seq**。pristine では 1）。テストは `pub_seq==1` でなく「新 epoch 内の最小 pub_seq」を assert する（iter2 [中]）。
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
DB は単一 `Arc<Mutex<Connection>>` 共有（`handle.rs:16`）。**HTTP POST は必ず `with_conn` スコープの外で行う**（3スコープ: [A] target 読み+current_epoch 読み+バッチ組成 → [B] ロック外で POST/ack → [C] cursor 永続化）。ロックを HTTP 往復（retry/backoff で長時間化しうる）越しに保持すると collector の ack 耐久化と retention を stall させる。

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
- restore で epoch 更新（既存、`renew_epoch`）。**pristine な交換箱では** outbox/readings は空（非 pristine の残留は §12/§8.3 で回収）。**target_registry も snapshot 非含有（§12）なので、pristine では target は無く、運用者が再登録**。
- gateway 起動時、**collector spawn 前**に §5.2 の trigger アルゴリズムで `epoch_start` を enqueue（新 epoch 内で**最小 pub_seq**を保証。pristine では 1）。
- **epoch guard**: 万一 target が旧 `cursor_epoch` を持って残っていても（非 pristine 復元）、push/retention は `cursor_epoch != current_epoch` を検知し effective cursor=0、新 epoch を最小 pub_seq から扱う。書込側レース（POST 中に別 restore で epoch が変わり stale cursor を書き戻す）も、次 cycle の再比較で無視され fail-closed（Sonnet 確認）。消費者は epoch 不一致で再 baseline（D7決定8B）。

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
`retention.rs` は同一周期で: readings の `purge_readings_before(received_at cutoff)` + **dedup TTL パージ** + **デバイス検疫期限失効**（`expire_quarantined_devices`=`devices.state` を active に戻す処理で、**readings 行は消さない**。`core/ledger/src/store.rs:609`）+ **statvfs 水位ラッチ** + **health 更新** を行う。
- **本 spec が変えるのは readings パージの判定規則だけ**。dedup TTL パージ・デバイス検疫期限失効・statvfs 水位ラッチ・health 更新は**維持**する。custody/ack を知らない `received_at` cutoff 削除（`purge_readings_before`、`core/timeseries/src/query.rs:268`）を §8.2 の custody 対応パージへ置換する。
- **注意（iter2 [高]、Sonnet 指摘）**: `purge_readings_before` は quarantine を問わず全 readings を floor で消していた**唯一の readings purge**。これを置換するので、検疫 readings を floor で消す branch は §8.2 で**新規に実装する**（「既存の検疫期限失効が消す」は誤り——それは `devices.state` のみ）。

### 8.2 MVE の custody 対応パージ
D7決定7 の4クラス順序のうち **MVE はクラス① のみ実装**:
- **クラス① eligibility（epoch guard）**: `target.archive_responsible=1` かつ **`target.cursor_epoch == current_epoch`** の時のみ有効。ある reading が「ack 済み」= **`publication_log.epoch == target.cursor_epoch == current_epoch` かつ `pub_seq <= target.cursor_pub_seq`**。epoch 不一致・cursor_epoch NULL 時は新 epoch 行を一切 ack 済み扱いにしない（effective cursor=0）。
- **削除対象**: 上記で ack 済み **かつ** `readings.received_at < now - 最低保持フロア` の行。
- **最低保持フロア**: [D1:154](../../redesign/decisions/D1-ingest-model.md) / [台帳:115](../../redesign/responsibility-ledger.md) = 既定 72h・設定可。**データ年齢(received_at)基準**（ack 相対でなくデータの新しさ。「正常時のデータ残高≒フロア分のみ、断線時のみ水位上昇」）。→ `archive_acked_at` 列は不要。
- **未ack「正本」は保護**: pub_seq を持つ（=配送対象の）非検疫 readings で未 ack のものは received_at が古くても**削除しない**（従来の無条件時刻カットオフ削除を廃止）。
- **保護は pub_seq 付き未ack正本だけ / それ以外は floor で purge**（Sonnet iter2 [高]、無限保持回避）: 置換後の readings 判定を「**`received_at < now - floor` の readings を削除。ただし pub_seq を持つ（=配送対象）かつ未ack（epoch 一致で `pub_seq > cursor_pub_seq`）の非検疫行だけは保護（削除しない）**」と実装する。検疫行（quarantined=1、pub_seq 無し）・enqueue されなかった行は保護対象外で floor で消える。これは**新規の readings purge branch**（旧 `purge_readings_before` の置換）であり、「既存機構が検疫 readings を消す」ではない（デバイス検疫期限失効は `devices.state` のみ＝§8.1）。**Wave 0→1 アップグレード（restore でない）の既存 readings も pub_seq を持たない**ので保護対象外＝従来どおり floor で消える（新たな保護を与えないだけで退行ではない、iter3 [低]）。
- **圧力時の挙動（custody_lost トリガ封じ、iter2 [中] で story 修正）**: クラス①を出し切っても statvfs 高水位が続く場合、クラス④（**保存済み**未ack正本の削除+custody_lost）は MVE では**実装しない**＝保存済みデータを消さないので custody_lost が定義上発生しない。**能動的逆圧（水位→collector 抑制）も新設しない**（D1 はプロセス内逆圧を mpsc await と規定し `Deferred` を返さない [D1:111]）。**正確な劣化像**: 既存の front-door drop（`iotkit-ingest-client/src/lib.rs:184` / `bravepi event_loop:133` / `polling_loop:619`）は**スループット由来**（バースト時のバッファ溢れ）で **ディスク水位に連動しない**（`observe_watermark_latched` は監査+health フラグのみで取り込みを絞らない）。よって「archive 消費者ダウンで custody backlog がディスクを埋める」場面では front-door は発火せず、放置すると最終的に `ENOSPC` で**新規書込が明示的に失敗**する（保存済みデータは保持、無音損失なし、custody_lost でない）。MVE はこれを許容し、**R12 に水位・per-target 配送状態を事前公開して警報**する（能動 throttle とクラス④は後続）。水位閾値・R12 形式は writing-plans。

### 8.3 archive target 不在時 と outbox prune・Tx 原子性
- **floor-only へ縮退する条件を厳格化**: **target が1行も登録されていない、または archive_responsible=0 の時のみ** custody 約束が無いので floor-only（received_at < now - フロア）に縮退する。**archive_responsible=1 の target が登録済みだが cursor_epoch が未一致/NULL（未 ack）の場合は floor-only にしない**——effective cursor=0 として全 pub_seq 付き行を保護する（未 ack 正本の無音破棄は契約違反 [D2:30]）。
  - **「target 不在」に至る経路を限定（iter2 [高]）**: rotation は in-place `rotate-token`（§3.3）で target は消えない。target 不在は明示的 `remove`（未 ack 正本があれば拒否/`--abandon-custody` 監査、§11）でのみ発生する。＝floor-only は「運用者が監査付きで custody を放棄した後」だけに到達し、無音では起きない。
- **クラス①パージの原子性**: eligibility select → **outbox 行 prune → readings 行 delete** → 監査、を**単一 Immediate Tx** で行う（現 retention の purge 後別 Tx を改める）。電源断で dangling ref や半端状態を残さない。削除順は outbox→readings（FK 方向、§4.1）。
- **outbox prune 規則**: `publication_log` 行は配送 retry のため ack まで保持し、以下で prune（**いずれも対応 readings 削除と同一 Tx でペア**にし、outbox だけ/readings だけの片残り＝orphan を作らない。iter2 [中]）:
  - ack 済み（`epoch==cursor_epoch==current` かつ `pub_seq <= cursor_pub_seq`）行は、対応 readings のクラス①削除と同一 Tx で prune。
  - archive target 不在の floor-only 削除で readings を消す時は、対応 outbox 行も同時に prune。
  - **旧 epoch の残 outbox 行**（非 pristine restore 等）はフロア基準で、**対応 readings 行と同一 Tx で**削除する（outbox だけ消して readings を orphan 化しない）。
  - **例外（§9.2 遡及検疫）**: `replace-undo` は readings を**削除せず** quarantined=1 に UPDATE し、対応 outbox 行のみを同一 Tx で prune する。検疫行は元来 outbox に居るべきでない（§4.1 不変条件）ので、これは片残り=orphan ではない（readings は R11/floor 管理下に残る）。

---

## 9. 検疫遷移経路のガード（解除・付与の両方向、hard reject）

### 9.1 検疫解除（release）方向の hard reject（ユーザー裁定 2026-07-05）
Wave 0 の alias 定義（`registry::define_alias` → `release_series_quarantine_for_key_checked`）は series 検疫フラグを clear するが、過去 readings 行は触らず outbox 化もしない。MVE は renumber を封じるので、**無音の配送欠落を作らないよう解除を hard reject でガードする**（D7:33/D5:89 に契約忠実）:

- **ルール**: **archive_responsible target が登録されている間、検疫を解除する操作（＝未配送の検疫 readings を持つ series の解除）は、明示オーバーライドフラグ無しでは拒否**する。
- **実装**: 解除経路に「登録済み archive target があり、対象 series に未配送の検疫 readings があるか」のチェックを追加。該当すれば `gatewayctl` はエラーで中断し選択肢を提示（① archive target を remove ② renumber 実装（後続）を待つ ③ `--release-abandon-past` で過去分を放棄して解除、監査記録）。
- **保持**: 拒否されている間、過去検疫行は検疫のまま `readings` に残り R11 で読める（**floor purge まで**。§8.2 の非保護行として floor 超過で消える）。**黙って解除されない**。
- **解除後の未来データ**: 通常どおり非検疫として outbox に流れる。欠けるのは解除前 backlog のみ。
- **完全な検疫解除 renumber**（過去行の新規採番 + measurement 再配送 + 検疫遷移 annotation）は出口契約拡張（後続 sub-project）。導入後は archive target 登録中でも過去分ごと配送でき、本ガードを緩められる。

### 9.2 検疫付与（遡及）経路のガード（replace-undo、iter3 [高]／iter4 で簡素化）
Wave 0 の `device replace-undo`（`iotkit-gatewayctl/src/cmd/replace.rs` → `mark_readings_quarantined`、`core/timeseries/src/query.rs:242`）は既存 readings を遡及的に `quarantined=1` へ反転する。反転対象が既に pub_seq を持つ行だと、§8.2 保護から外れ floor purge で outbox が dangling、また push は POST を lock 外で行う（§6.2）ため**送信中の行が prune されても無通知で消費者へ届く**穴も生じる。D7決定1 は遡及検疫を annotation で扱うと規定するが MVE は annotation を封じるため、**§9（解除方向）と対称の hard-reject に一本化**する（iter4: 「機械 prune + 既 ack override」の二段は in-flight race を生むため簡素化）:
- **ルール**: **archive_responsible target が登録されている間、`replace-undo`（＝pub_seq 付き行を遡及検疫する操作）は `--abandon-custody` override 無しでは拒否**する。override は「配送済み/送信中の行は MVE では回収・通知できない（annotation 封鎖）」ことを運用者に監査付きで確認させる（§9 と対称）。
- **機械的（override 時 or archive target 不在時）**: `mark_readings_quarantined` は同一 Tx で対応 `publication_log` 行を prune し「検疫⇒pub_seq 無し」不変を回復（DB orphan/FK を作らない）。
- これにより **in-flight race は消える**——archive 稼働中は override 無しに replace-undo が走らないので送信中 prune が起きず、override 時は運用者が回収不可を承知済み。

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
  0. **既存 target が1行でもあれば `add` を拒否**（MVE は単一 target。2行目を作らせない。Sonnet iter2 [低]）。
  1. `endpoint_url` は `https://` のみ受理（平文拒否）。
  2. 登録を ledger 監査イベントに記録。
  3. 登録時**疎通スモーク**（空/ping バッチを POST し 2xx+ack を確認）。**スモーク成功まで archive_responsible=0**（誤設定 archive target へ配送→未受信のままパージ＝custody 損失を防ぐ）。
  4. `schema_version` 一致チェック（MVE=1、不一致は拒否）。
- **`target rotate-token`**: 行と cursor を保持したまま token を UPDATE + 再スモーク（§3.3）。**`archive_responsible` は変えない（終始 1、スモーク失敗時は token ロールバック）**。remove+add は使わない。
- **`target remove`**: 明示的 custody 放棄。**未 ack 検査→削除/拒否＋監査を単一 Immediate Tx**（別プロセス gatewayctl の TOCTOU 防止、iter3 [高]）。未 ack の pub_seq 付き正本が残る場合は拒否、`--abandon-custody` で監査付き強行のみ（§3.3）。
- **HTTP クライアント**: smoke-test POST のため gatewayctl に `reqwest`(blocking) を追加（gateway daemon の async reqwest とは別 feature）。
- 完全な R14 型付き操作（権限段階・dry-run・全操作カタログ）は sub-project E。

---

## 12. R22 連携

- **target_registry を平文 R22 snapshot に含めない**。理由: `credential_token` は秘密で、現 R22 snapshot は平文 JSON 書き出し（`iotkit-gatewayctl/src/cmd/snapshot.rs:129`）。[D2:75/98](../../redesign/decisions/D2-data-authority-topology-operations.md) は secrets 非空 snapshot の暗号化を必須とする。R22 暗号化は MVE スコープ外なので、**target/token を snapshot に入れない**。
- **publication_log（outbox）も data-plane** → readings 同様 snapshot に含めない。
- **restore 相互作用**: `run_restore` の空判定（`snapshot.rs:259`）は 5 SECTIONS のみを見る＝ publish 2表は判定にもリストアにも関与しない。pristine な交換箱では publish 2表は空で、運用者が `target add` で再登録（credential 再発行 + スモークで archive_responsible 再有効化）。非 pristine な箱へ restore して古い target が残っても、epoch guard が stale cursor を fail-closed に無効化する（§6.4）ので誤パージ・誤配送は起きない（推奨は restore 前後に `target remove`/再登録）。
- **非 pristine 残留の回収（iter2 [中]）**: 旧 epoch の outbox/readings 残存は §8.3 の「旧 epoch floor-prune（outbox+readings ペア、同一 Tx）」で回収する。pub_seq は AUTOINCREMENT 継続で 1 に戻らないが正しさは保つ（§4.1）。**writing-plans は restore 前提を確定する**: publish/readings 空を要求するか、restore Tx 内で publish/readings/sqlite_sequence を明示 cleanup するか。target config の暗号化退避は R22 暗号化と同時に後続 sub-project へ。

---

## 13. エラーハンドリング

- push 失敗（接続断/非2xx/タイムアウト/ack 不一致）: bounded exponential backoff（上限あり）。**shutdown 応答は net-new**: 既存の retention/health タスクは bare loop（`select!` shutdown 無し）で流用できないため、push タスクは main fan-in ループ（`main.rs:253` 系）の select! パターンを参考に**新規実装**する。
- スモーク失敗: `target add` はエラーで中断、archive_responsible を立てない。
- 消費者が長期死亡: cursor 進まず custody backlog がフロア超で滞留 → 水位上昇 → **R12 警報（事前）**。front-door drop はスループット由来でディスク水位に非連動なので（§8.2）この場面では発火せず、放置すれば最終的に `ENOSPC` で新規書込が明示失敗する（保存済み正本は保持、無音損失なし）。MVE は「単一の制御された archive 消費者」前提でこれを許容（能動 throttle とクラス④は後続）。
- 全体障害と個別 target 障害を混同しない（MVE は single-target だが、エラー型は target_id 文脈を持つ）。

---

## 14. テスト戦略

- **適合テスト消費者**: リポジトリ内フィクスチャ（バッチ POST を受け、pub_seq end を ack する極小 HTTP サーバ。tokio + 単純 listener で可）。
- **end-to-end custody ループ**: reading 挿入 → outbox enqueue → push → ack → cursor 前進 → retention クラス①で当該 readings 削除、を1本で検証。
- **クラッシュ冪等性**: push 後 ack 前 / ack 後 cursor 前 の擬似クラッシュで、再送が同一 publication_id・`(epoch,pub_seq)` で消費者 dedup されること。
- **検疫除外（配送）**: 検疫行は outbox に入らない（配送されない）。
- **検疫行の無限保持回避**: 行レベル検疫 readings が archive target 登録中でも **§8.2 の新規 floor branch（received_at 基準）**で purge され、保護されないこと（デバイス検疫期限失効=`devices.state` には依存しない）。
- **retention フロア**: ack 済みでもフロア(received_at)内は削除されない。未 ack 正本は received_at が古くても保護される。
- **epoch guard negative**: cursor_epoch != current_epoch（および NULL）の時、retention が新 epoch の未配送行を purge しない・push が effective cursor=0 で配送すること。
- **epoch 載り替え**: R22 restore 後、`epoch_start` が collector 開始前に enqueue され新 epoch 最初の pub_seq で配送される（初回 boot では出さない）。
- **outbox prune / no-dangling**: ack 済み行の同一 Tx prune、floor-only 時の prune、電源断（Tx 途中）で dangling ref が残らないこと。
- **byte-cap 単一超過**: 1レコードが byte cap を超えても空バッチにならず配送が進むこと。
- **hard reject（§9）**: archive target 登録中の検疫解除が override 無しで拒否され、`--release-abandon-past` で監査付き解除できること。
- **target 管理ガード**: http:// 拒否、スモーク失敗で archive_responsible が立たない、schema_version 不一致拒否、**既存 target ありで `add` 拒否**、`rotate-token` で cursor 保持しつつ token 更新、`remove` は未 ack 正本ありで拒否/`--abandon-custody` で監査付き。
- **rotation 窓の custody 保護（iter2 [高]）**: 4日超 backlog（>フロア）保持中に `rotate-token` しても floor-only パージが起きず backlog が保護されること。未 ack ありの `remove` が override 無しで拒否されること。
- **検疫 readings の floor purge（iter2 [高]）**: 検疫行（quarantined=1、pub_seq 無し）が archive target 登録中でもフロア超で削除される（無限保持しない）。同時に pub_seq 付き未 ack 正本は削除されない（新規 branch を実際に踏む）。
- **旧 epoch 残留の回収（iter2 [中]）**: 非 pristine 想定で、旧 epoch の outbox 行が対応 readings とペアで削除され orphan が残らないこと。
- **圧力時の無音損失なし（iter2 [中]）**: 消費者ダウンで水位上昇時、保存済み未 ack 正本が削除されず R12 が水位を公開すること（front-door drop は本経路で発火しない）。
- **遡及検疫（replace-undo、iter3 [高]/iter4）**: archive_responsible target 登録中の `replace-undo` は `--abandon-custody` 無しで**拒否**される（既 ack か否かに依らず、§9.2）。override 時 or archive 不在時は `mark_readings_quarantined` が対応 outbox を同一 Tx で prune し dangling/無音損失/FK エラーにならない。
- **remove の TOCTOU（iter3 [高]）**: `remove` の未 ack 検査中に別プロセス（gateway daemon）が新 pub_seq を commit しても、単一 Immediate Tx の検査+削除で未 ack 行を無音パージしない。
- **rotate-token の archive_responsible 不変（iter3 [中]）**: rotate-token 実行中（スモーク失敗含む）archive_responsible が終始 1 で floor-only が発火しないこと。
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
| `iotkit-gatewayctl/src/cmd/target.rs`（新規）+ `main.rs` | `target add|list|rotate-token|remove`、smoke に reqwest(blocking)、migration 連結 +1 |
| `iotkit-gatewayctl/Cargo.toml` / `iotkit-gateway/Cargo.toml` | reqwest 追加（gatewayctl=blocking / gateway=async、rustls-tls） |
| `iotkit-gatewayctl/src/cmd/snapshot.rs` | **変更なし**（§12: target/token・outbox とも snapshot 非含有） |
| `core/registry/src/store.rs`（or 呼出側） | 検疫解除 hard reject ガード（§9） |
| `iotkit-gatewayctl/src/cmd/replace.rs` / `core/timeseries/src/query.rs` | replace-undo 遡及検疫: `mark_readings_quarantined` が対応 outbox を同一 Tx prune、archive target 登録中は override（§9.2） |
| `core/timeseries/migrations/0004_readings_v3.sql` | stale コメント修正 |

---

## 16. 宿題ピン（writing-plans で確定する値・判断）

- 最低保持フロア既定値: 72h（[D1:155](../../redesign/decisions/D1-ingest-model.md)）。設定手段（config or ledger_meta）。
- 有界バッチ上限 N（件数）と byte cap。**単一レコード超過時は最低1件**（§6.2）。
- ack レスポンス形式（publication_id / epoch / cursor_end 到達の JSON 形、§6.2 [C]）。MVE は all-or-nothing バッチ ack（部分 ack は DEFERRED）。
- retry backoff パラメータ・push 間隔。
- `publication_log.reading_seq` の FK 宣言有無と `ON DELETE`（張らないなら削除順で担保、§4.1）。
- epoch_start trigger の実装場所（gateway 起動シーケンス内、collector spawn 前、§5.2/§6.4）。
- 水位→R12 公開の具体形と閾値（§8.2）。
- push タスクの `select!` shutdown 実装（net-new、§13）。
- rotate-token のスモーク失敗挙動（token ロールバック、archive_responsible 不変、§3.3）。
- remove / abandon-custody の単一 Immediate Tx 境界（§3.3/§11、§9.2）。
- replace-undo 遡及検疫の outbox prune と override 境界（§9.2）。

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
