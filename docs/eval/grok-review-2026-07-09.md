# IoTKit-next 総合レビュー

| 項目 | 内容 |
|------|------|
| 対象 | `w-pinkietech/iotkit-next` |
| リビジョン | `a4ae911`（`master` / `origin/master` 同期時点） |
| 実施日 | 2026-07-09 |
| レビュア | Grok（xAI） |
| 視点 | ①プロダクト／運用成熟度 ②現場適合 ③コーディング／アーキテクチャ ④サイトサーバー役割 |

> **位置づけ:** 差分レビューではなく、未完成（pre-1.0 / Wave 1 途中）を前提とした **システム全体の評価メモ**。  
> **Historical (2026-07-09):** 当時、設計正本は別cloneの `../docs/redesign/` にあり、本リポジトリには同梱されていなかった。2026-07-12以降の正本は本リポジトリの `docs/redesign/`。

---

## 0. 総括（1 ページ要約）

**結論:**  
コア思想（custody・ack 意味論・バッファとしてのゲートウェイ）は実装とテストにまで落ちており、**自サイト運用の骨格としては成熟度が高い**。  
第三者配布・OSS 公開には、既定セキュリティ・運用契約の明示・Wave 1 後半がまだ足りない。

| 軸 | 評価 |
|----|------|
| 現場に刺さる思想 | 強い（「黙って消さない」「電源断は普通」） |
| エンジニアリング水準 | 業務プロダクション品質（industrial systems） |
| アーキテクチャ | 制約駆動の modular monolith。核心は正しい |
| OSS / 配布準備 | 未成熟（ライセンス・既定値・公開 ingress） |
| 今日の自サイトデプロイ | 条件付きで可（§1.5） |

**残すべき中核:**  
同一 Immediate Tx での reading + outbox、NoAck ≠ Rejected、generation 無効化、custody-aware purge、publish が HTTP 中に DB ロックを持たない、migration set-difference + schema-ahead、R14 dispatch、adapters が engine に依存しない。

---

## 1. プロダクト／運用成熟度レビュー

### 1.1 何ができているか（現状）

```
 sensor adapters ──▶ collector ──▶ SQLite (readings + outbox) ──▶ push task ──▶ archive consumer
   (BravePI,          (dedup,        (durable, crash-safe)         (HTTPS,        (acks a cursor;
    rpi-local)         normalize,                                   per-target     ack authorizes
                       quarantine,                                  token, at-      purge)
                       series id)                                   least-once)
```

- Wave 0（自サイトで動く最小）: 完了  
- Wave 1: 出口契約 MVE・制御プレーン土台まで到達、残り進行中  
- 規模感: Rust 約 3.3 万行、~460 テスト想定、設計／レビュー文化（watchpoint・deferred list）が厚い

### 1.2 強み

1. **取り込み ack 意味論がコードと一貫**  
   ストレージ失敗は `Rejected` にせず `ack_tx` ドロップ → `SubmitError::NoAck`。決定的契約違反のみ終端拒否。

2. **エンベロープ全体の単一 Tx + 非検疫時 outbox 同 Tx エンキュー**  
   reading と outbox の一体性を構造で保証。ロールバック時の幻 series_id 対策（キャッシュ破棄）あり。

3. **レジストリの決定的拒否 vs 検疫の切り分け**  
   NaN/Inf を検疫ではなく `ValueTypeMismatch` 終端にし、恒久 NoAck ループを潰している。

4. **保管責任付き retention**  
   `archive_responsible` 時は unacked を保護。sightings 等を致命経路から分離。

5. **publish の ack 検証が厳格**  
   `publication_id` / `acked_pub_seq` 不一致でカーソルを進めない。POST 中に DB ロックなし。

6. **秘密情報の露出抑制**  
   `Secret` の Debug 赤塗り、credential マスク、監査に平文トークンを載せない。

7. **マイグレーション harness**  
   クレート分割番号空間に対する set-difference 適用と schema-ahead 拒否。

8. **制御プレーンの fail-closed 要素**  
   setup allowlist、private source guard、TLS、throttle、Construction の step-up、AI トークンの tier 二重遮断。

### 1.3 Critical

#### C1. API 既定が「有効 + `0.0.0.0:8443`」、setup 中は Bearer なしで一部変異 op が通る

- 根拠: `iotkit-gateway/src/config.rs` の `enabled` 既定 `true`、bind 既定 `0.0.0.0:8443`。  
- setup mode では `device.approve_sighting` / `registry.resolve_unknown_key` 等が認証なしで可能。  
- `private_source_guard` でインターネット直公開は弾かれるが、**同一 LAN の任意ホストは setup 完了まで操作可能**。  
- **提案:** 既定 bind を `127.0.0.1`、または setup 完了まで API 無効／ワンタイム setup token。README に初回チェックリストを固定。

### 1.4 Important

| ID | 内容 | 影響 |
|----|------|------|
| **I1** | ingest client spool が **メモリのみ**（溢れは drop-oldest）。アダプタは Full 時に読み取りを捨てる | 電源断・プロセス死で「アダプタ〜コレクタ間」は揮発 |
| **I2** | archive target 未登録 / `archive_responsible=false` では retention が **floor-only** | target 設定忘れで custody 保証が成立しない |
| **I3** | 承認前 `staged_readings` は **本流（readings/outbox）へ昇格しない**。リング 1000 件/hw | 承認前ヒストリは時系列として残らない |
| **I4** | ディスク水位は観測のみ。緊急 purge / 取り込み逆圧は未実装 | 安全だが graceful でない（ENOSPC まで走る） |
| **I5** | R14 HTTP カタログが CLI より狭い | API だけでは onboard 完結しない |
| **I6** | 検疫デバイスの TTL 自動 active 化 | 現場ポリシーによっては意図しない本流化 |
| **I7** | ライセンス未設定・公開 ingress 未実装 | OSS 配布はまだ不可 |
| **I8** | throttle `sources` の無制限成長・失敗カウント非減衰 | deferred hardening D-4 と一致 |
| **I9** | 認証の `last_used_at` 更新が読取経路でも DB 書き込み | 高頻度 poll で collector とロック競合しうる |

### 1.5 Suggestions（抜粋）

- wire 上の未使用 `ReasonCode` の明記  
- `new_envelope` が空 values を黙って落とすことの文書化  
- architecture 図が AdapterEvent → collector に見え、実体（IngestClient Envelope）とズレやすい  
- plan5 deferred list（D-1〜D-8）は健全な「後回し台帳」— 考古学にしない  
- series 級検疫の運用ドキュメントが必要  

### 1.6 Wave 成熟度

| 領域 | 状態 |
|------|------|
| Ingest 契約 + collector + registry | **自サイト production-ready** |
| Adapter → ingest client | 運用可、損失モードは明示が必要 |
| Exit contract (publish + retention) | **条件付き ready**（target + 正しい consumer ack） |
| Ledger / gatewayctl | ローカル運用 ready |
| Control plane API | 基盤は動くが配布向け未成熟 |
| Storage / migrations | ready |
| 公開ネットワーク取り込み | **未実装** |
| OSS リリース | **not ready** |

#### 今日デプロイしてよい条件（自サイト）

1. 起動直後にパスフレーズを設定する  
2. API を `127.0.0.1` か FW 下に閉じる  
3. archive target を `archive_responsible=true` で登録し、consumer が厳密 ack する  
4. 承認前データは捨ててよいと割り切る、または事前 `device add --active`  
5. health の backlog / watermark を監視する  

#### まだやってはいけないこと

- setup 放置で不特定が LAN から API に届く環境  
- target 未設定のまま custody を期待する  
- プロセス再起動ゼロ損失を期待する（メモリ spool）  
- インターネットからのセンサ HTTP/MQTT 取り込み  

### 1.7 推奨フォーカス（優先順）

1. **配布時セキュリティ既定**（bind / setup 窓）  
2. **取り込み耐久の次段階**（disk spool または損失メトリクスを health に）  
3. **staged の契約を正面に出す**（昇格実装 or「ヒストリ非保証」明示）  
4. **target 未登録時の強い degraded / 警告**  
5. **deferred hardening の消化**  
6. **R14 カタログ拡張**（activate / target など）  
7. **OSS ゲート**（ライセンス、schema 安定方針、consumer 参照実装）  

---

## 2. 現場適合レビュー（「愛される箱」か）

### 2.1 結論

**「現場に愛される素地はかなりある。今は“設計思想として愛される”段階で、“毎日触る箱として愛される”にはもう一段。」**

- 玄人・保全・エッジ寄りの人には信頼されやすい  
- 誰が置いても初対面で安心、まではまだ尖っている  

### 2.2 現場に刺さる点

1. **約束が現場語**  
   - 電源断を異常ではなく普通のイベントにする  
   - ゲートウェイは倉庫ではなくバッファ  
   - consumer ack まで purge しない  
   - ディスクが詰まったら黙って捨てず ENOSPC で止まる  

2. **現場運用の現実を知っている**  
   - series identity が rename / HW 交換で切れない  
   - gatewayctl というローカル操作面  
   - 1 binary + SQLite + systemd（退屈＝信頼性）  

3. **壊れたときの姿勢が誠実**  
   - ストレージ失敗を Rejected にしない  
   - quarantine と reject の分離  
   - health / 監査 / epoch フェンス  
   - watchpoint / deferred list  

現場は完璧さより **「嘘をつかない箱」** を好む。この点は強い。

### 2.3 まだ愛され切らない理由

| 現場の感覚 | 現状 |
|------------|------|
| 箱を置いたら安全側に倒れてほしい | API 既定 `0.0.0.0` + setup 窓 |
| 「送れてるはず」が欲しい | target 未設定だと floor-only で消える |
| 再起動しても途中まで残っててほしい | ingest spool がメモリのみ |
| 承認前のデータも少しは残してほしい | staged は本流に昇格しない |
| 詰まったらどうなるか分かる | 水位は観測のみ、graceful ではない |
| 手順が少ない | 操作の多くが gatewayctl / 文書依存 |

正しさの哲学は現場向きだが、**初期設定を間違えたときの体験**がまだエンジニア寄り。

### 2.4 愛される製品の 3 類型との対応

1. とにかく簡単 → **まだ Wave 1 後半〜 Wave 2**  
2. とにかく壊れない → **コアは本気で取りにいっている**  
3. 壊れたとき説明できる → **近い**  

愛される瞬間は機能追加より:

- 初回起動が安全側  
- target 未設定がはっきり赤になる  
- 再起動・詰まり・承認待ちが運用言語で説明される  
- 「これだけやれば現場で回る」チェックリストが短い  

---

## 3. コーディング水準・アーキテクチャ設計レビュー

### 3.1 水準感

| 軸 | 評価 |
|----|------|
| アーキテクチャ | 実務 mid〜senior / staff 手前。意図がはっきりした modular monolith |
| コーディング | 業務プロダクション品質。不変条件をコードに落とす力が強い |
| 抽象化 | 過剰でも過小でもなく、契約境界中心 |
| 一貫性 | コア（ingest / ledger / publish）は高い。周辺と composition root は成長の跡 |

> **「きれいなクリーンアーキテクチャ教材」ではなく、「制約を知った人間が、壊れない箱を Rust で組んだ設計」**

### 3.2 アーキテクチャの上手さ

#### レイヤが依存方向で切れている

```
iotkit-ingest-contract   ← ワイヤ契約（安定意図）
        ↑
core/{storage,ledger,timeseries,publish,collector,registry,ops}
        ↑
adapters / ingest-client
        ↑
iotkit-gateway (composition root) + gatewayctl
```

- adapters は engine に依存しない  
- 取り込み正本は IngestClient → Collector  
- AdapterEvent は監督・投影用に並走  

#### 核心の不変条件が構造になっている

| 不変条件 | 構造的な支え |
|----------|----------------|
| ack なし = 未耐久 | oneshot ドロップ → `NoAck` |
| reading と outbox の一体性 | 同一 Immediate Tx |
| 世代境界 | epoch + generation counter |
| キャッシュ無効化 | generation 不一致で全捨て |
| ダウングレード耐性 | migration set-difference + schema-ahead |
| 単一 writer | `DbHandle` + reentrancy panic |

#### 制御プレーンのカタログ化（R14）

`OpDescriptor` + `dispatch` 単一入口で tier / bulk / step-up / setup allowlist を強制。SQL 直書き変更経路を増やさない方針と整合。

#### Adapter 境界

BravePI（codec / sensors / transport）、polling runtime（`SensorDriver`）、固有型 → `ConnectionInfo` の片方向変換。プロトコル詳細が core に染み出していない。

### 3.3 アーキテクチャの歪み

| ID | 内容 | コメント |
|----|------|----------|
| **A1** | 二重のデータ面（Envelope 正本 vs AdapterEvent 投影） | 意図的並走だが認知負荷が高い。図・README を固定すべき |
| **A2** | `RegistryPolicy` が collector 定義・registry が impl | 動くがポート置き場として純度減点 |
| **A3** | composition root（`main.rs`）が太い | bootstrap 分割で改善可能 |
| **A4** | 巨大モジュール | `polling_loop` ~2100、`ledger/store` ~2000、`collector/actor` ~1200 等 |
| **A5** | エラー型が境界で崩れる | ホットパス `Result<_, String>`、`ToSqlConversionFailure` 橋渡し |
| **A6** | migration 番号空間の人間プロセス依存 | 小チーム向き、OSS 多人だと衝突しやすい |

### 3.4 コーディングの上手さ

- 不変条件ファースト（NaN 終端の理由まで書かれている）  
- SQLite 単一 Mutex + `spawn_blocking` の現実的判断  
- TOCTOU / Immediate Tx / SAVEPOINT への意識  
- 契約を見るテスト中心  
- `Secret` の赤塗り  
- 過剰 DI・空レイヤが少ない  

### 3.5 コーディングの減点点

- Stringly errors  
- god file 傾向  
- コメントが仕様書同期で長く、参入障壁になりうる  
- rpi-local アドレス等のハードコード二重管理  
- API と CLI のパリティ不足（段階公開としては正しい）  
- `SensorReading` と `ReadingItem` の二重語彙  

### 3.6 採用パターン

| パターン | 適用 |
|----------|------|
| Modular monolith | crate 分割 + 単一バイナリ |
| Ports & adapters（軽量） | RegistryPolicy, SensorDriver, IngestClient |
| Outbox / custody | publication_log + ack cursor |
| Command catalog | R14 OpDescriptor |
| Event-carried projection | Engine（非耐久） |
| Single-writer datastore | DbHandle |

流行りフル装備ではなく、**Pi 一台で壊れない制約からの最小十分**。良い判断。

### 3.7 相対スコア（主観・10 点満点）

| 観点 | 点 | コメント |
|------|----|----------|
| 境界設計 | 8.5 | 契約・依存方向が明確 |
| ドメインモデルの一貫性 | 8 | D1/D5/D6 がコードに生きている |
| 拡張性 | 7.5 | 伸びるが巨大ファイルが足枷 |
| エラー設計 | 6 | 意味論は強いが型が弱い |
| 可読性・参入容易性 | 6.5 | 正本依存・二重パス・コメント量 |
| テスト設計 | 8.5 | 契約テスト中心で優秀 |
| 運用を知った設計判断 | 9 | 最大の差別化 |
| コード美学 / 洗練 | 7 | 美しく整っているというより「正しく重い」 |

**総合:** 信頼できる industrial systems コード。骨格を作り直す必要はない。

### 3.8 設計を磨くなら

1. データ面の物語を一本化（Envelope 正、AdapterEvent は投影）  
2. エラー型の再導入（`CollectorError` 等）  
3. 大きな store / actor の分割  
4. Policy ポートの移動  
5. composition root の bootstrap 化  

---

## 4. サイトサーバーの役割

### 4.1 用語

`iotkit-next` 文書では **「サイトサーバー」という語はほぼ使われない**。  
同じ役割の正式用語は **archive consumer（アーカイブ責任消費者）**。

### 4.2 位置づけ（一言）

**サイト側の「正本保管庫」＝ゲートウェイからデータを受け取り、保管責任を引き取る相手。**

```
[現場センサ]
    → [IoTKit Gateway (Pi)]  … 収集・一時保持・push
        → HTTPS POST + cursor ack
            → [Archive consumer / サイトサーバー]  … 耐久保管・履歴の正本
```

ゲートウェイは **倉庫ではなくバッファ**。長く持つ・分析する・ダッシュボードを回すのは出口の向こう側。

### 4.3 やる／やらない

#### やること

1. 測定ストリームを受け取る（`measurement` + 最小 `annotation`）  
2. 耐久に書く  
3. 厳密に ack（`publication_id` echo、`acked_pub_seq == cursor_end`）  
4. `(epoch, pub_seq)` で冪等 upsert  
5. 意味付け・派生・閾値は消費者側（ゲートウェイは生レコードストリーム）  

#### やらないこと

| やらない | 理由 |
|----------|------|
| ゲートウェイ内データの直接削除指示 | purge は ack 済み ∧ floor のゲートウェイ側ルール |
| デバイス台帳・identity の正 | ledger はゲートウェイ側 |
| センサ取り込み経路の制御 | エッジ側 |
| コマンド中枢／設定マスタ | 出口契約の消費者にすぎない |
| ゲートウェイ制御 API そのもの | 制御プレーンはゲートウェイ自身の LAN API |

### 4.4 custody 境界

| ゲートウェイ | サイトサーバー |
|--------------|----------------|
| 未 ack 正本を消さない | ack した分の長期保管 |
| outbox / cursor の送り側 | cursor の受け側（冪等） |
| retention floor 内バッファ | 分析・可視化・長期アーカイブ |
| デバイス承認・レジストリ | （任意）意味付け・アプリ |

- ack 停止 → ゲートウェイは purge 停止、最終的に ENOSPC  
- 正しい ack 継続 → バッファを薄く保てる  

### 4.5 旧実装との読み替え

旧 IoTKit は Pi 上に MariaDB / InfluxDB が乗り、ゲートウェイ≒小型サイト DB に近かった。  
`iotkit-next` では:

- **エッジ:** 壊れにくいバッファ + 出口契約  
- **サイトサーバー:** 長期保管の正本（契約を満たせば実装は自由）  

**リポジトリ内にサイトサーバー本体の実装はなく、契約の相手として定義されている。**  
MVE は単一 target。マルチ消費者・replay・backfill は将来。

---

## 5. アーキテクチャとして残すべきもの（まとめ）

- **ack なし = 未耐久、Rejected = 終端** の分離  
- **同一 Immediate Tx での reading + outbox**  
- **generation counter によるキャッシュ無効化**  
- **custody-aware purge（cursor と epoch フェンス）**  
- **publish がネットワーク中に DB ロックを持たない**  
- **レジストリの series 級 / 行級検疫分類と NaN 終端拒否**  
- **マイグレーションの set-difference + schema-ahead**  
- **R14 dispatch 単一入口**  
- **adapters が engine を知らない依存則**  
- **契約クレート（ingest-contract）の分離**  

これらは実装の癖ではなくプロダクトの中核価値。Wave 2 でも壊さない方がよい。

---

## 6. 率直な一文集

| 視点 | 一文 |
|------|------|
| プロダクト | 自サイトでは骨格が強い。配布・OSS はまだ早い。 |
| 現場 | 現場向きの思想。今は玄人に信頼される箱。誰でも安心な箱にはもう一歩。 |
| コード／設計 | 信頼できる業務品質。流行りより制約駆動。骨格の作り直しは不要。 |
| サイトサーバー | コマンド中枢ではなく、履歴正本を引き取る出口の相手。 |

---

## 7. 改訂履歴

| 日付 | 内容 |
|------|------|
| 2026-07-09 | 初版。同一セッションの 4 視点レビューを 1 文書に統合 |
