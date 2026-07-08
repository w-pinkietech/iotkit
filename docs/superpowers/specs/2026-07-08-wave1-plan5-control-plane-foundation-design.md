# Wave 1 計画5: 制御プレーンの土台（R14操作カタログ + 認証の座席 + APIサーバー）設計仕様

> **For agentic workers:** この spec は brainstorming（設計正本 D1〜D13、2026-07-08 確定）の成果物。実装は writing-plans → subagent-driven-development で行う。本文書は「契約」ではなく「Wave 1 実装 spec」——契約正本は設計リポジトリの [D3決定5](../../../../docs/redesign/decisions/D3-process-and-wave-decisions.md) / [D12決定3](../../../../docs/redesign/decisions/D12-southbound-contract.md) / [D13](../../../../docs/redesign/decisions/D13-ui-scope.md) / 責務台帳 R11/R12/R14。

**Goal:** ゲートウェイ内に初の HTTPS API サーバー（自己署名 TLS + フィンガープリント）を立て、R14 型付き操作カタログ（権限3分類+read-only スコープ、dry-run、全操作監査）と認証の座席（管理者パスフレーズ・setupモード・operatorトークン）を作る。以降の全計画（R2入口・南向き・UI）はこの土台の上に乗る。

**Architecture:** 新クレート `core/ops`（操作カタログ框組 + 標準カタログ組み立て + 認証ストア + 監査、migration 0012。依存は storage/ledger/registry への一方向）+ `iotkit-gateway/src/api/`（axum + rustls の常駐 API タスク）+ `iotkit-gatewayctl` 追加コマンド（passphrase reset / fingerprint / token）。UI・CLI・AI は同じ操作カタログを叩く（R14「AI/人間共用」、D13前提1）。

**Tech Stack:** Rust 1.95 / tokio / rusqlite(WAL) / axum 0.8 + **axum-server 0.8（`tls-rustls-no-provider`）+
`rustls = { version = "0.23", default-features = false, features = ["ring","std","tls12"] }` + 起動時 ring provider install**（§5.4）/
rcgen 0.13 / argon2 0.5（salt は password-hash 同梱の rand_core OsRng）/ sha2 0.10（core/registry で既使用）/
getrandom **0.4**（トークン乱数。0.2/0.4 が既にツリーに居るため 0.3 で3本目を作らない）/ base64 0.22（`URL_SAFE_NO_PAD`）。

---

## 1. スコープ

### 1.1 位置づけ

- 出口契約 MVE spec（2026-07-05）が **sub-project E（R14 型付き操作フレームワーク）/ sub-project B の制御プレーン片（operator 認証）** として繰り延べたものの回収。
- progress.md の Wave 1 ブロッカー「R2 は R19 認証（トークン）が前提」の**前提側**を作る。ただし本計画は**制御プレーン（operator/管理者）認証のみ**——デバイストークン（入口認証）は計画6。
- Wave 1 分割（2026-07-08 合意）: **計画5（本書）→ 計画6 入口 R2 → 計画7 出口完成 D9/D7 → 計画8 R15+R9+D12南向き → 計画9 UI D13**。

### 1.2 実装する（IN）

1. `core/ops` クレート: 操作カタログ框組（descriptor + dispatch + tier 執行 + dry-run + 監査）、**標準カタログの単一組み立て点** `standard_catalog()`（§6.5）、認証ストア（`admin_credential` / `operator_tokens`、migration 0012）、setupモード判定
2. 権限モデル: `Tier { ReadOnly, Routine, Daily, Construction }`。**ReadOnly はトークン ceiling 専用値**であり、OpDescriptor の tier には使えない（dispatch が assert。D12決定3「照会は層ではなくスコープ」）。AI トークンは Routine 上限（構造的: `tier_ceiling` + DB CHECK）
3. 管理者パスフレーズ（argon2id、単一行）+ **setupモード**（= admin_credential 行が無い状態。閉集合 gate + actor=`setup_mode` 監査 + 常時 setup フラグ表出）
4. operatorトークン（発行時のみ平文表示・保存はSHA-256ハッシュ・失効・last_used）+ ログインセッション（パスフレーズ→セッショントークン）
5. **step-up**: **実効 tier**（bulk 昇格後）が Construction の操作はログイン済みでもパスフレーズ再入力必須（§4.4）
6. ログイン試行スロットリング（per-source 逓増遅延 + グローバル上限 + 監査。**照合前に適用**）
7. HTTPS API サーバー（axum、既定有効、初回起動時に rcgen 自己署名証明書生成・SHA-256フィンガープリント表出、**非プライベート発の接続は常時拒否**=§7.6）: `/api/v1/box`（無認証・最小）/ `session` / `setup/passphrase` / `health`（R12）/ `series`+`live`+`readings`（R11）/ `ops`（R14 catalog + dispatch）
8. R14 初期カタログ（移管元の正確な所在は§6.2）: `registry.resolve_unknown_key`、`device.approve_sighting`、`device.retire`、`operator_token.issue`、`operator_token.revoke`
9. R11 API 化: **既存の** `core/timeseries::query::{query_readings_v3, aggregate_readings_v3, latest_by_series}` を呼ぶ（共有関数は Wave 0 計画4 で既に core 側にある——本計画で SQL 抽出はしない。`/live` も既存 `latest_by_series`(query.rs:209) の組み立てのみ）。新規作業は series_key の合成/分解の core/ledger への移設と解決 fn（§7.3）+ API 応答スキーマ
10. R12 統合（HealthState に api セクション追加、`/api/v1/health` は**既存の手書きレンダラ** `render_health_json` を再利用——serde 化はしない）
11. gatewayctl 追加: `passphrase reset`（回復経路 = D13決定2「物理/SSHアクセス者はroot相当」）、`fingerprint`、`token issue|revoke|list`（監査 actor=`local_cli`）
12. **`gatewayctl target add` に setupモード拒否を前倒し追加**（D13 閉じ圧力3。admin_credential 行が無ければ拒否。R14 への本移管は計画7 のまま）
13. 監査: 全 dispatch を `ledger_events` に D12 粒度で記録（§6.4）。認証系イベントも記録

### 1.3 繰り延べる/封じる（DEFERRED / SEALED）

| 項目 | 扱い | 封じ方（無音の穴を作らない） |
|---|---|---|
| デバイストークン・入口リスナー（R2/D11） | DEFERRED（計画6） | 本計画の API サーバーは**制御プレーン専用リスナー**。取り込み経路は inproc のまま（collector は Envelope.source 信頼のまま=現状維持、計画6で閉じる） |
| `listener.enable` 工事層操作（D11決定7） | DEFERRED（計画6） | 入口リスナー自体が無いので操作も無い |
| target 操作の R14 移管 | DEFERRED（計画7） | 既存 CLI 経路は ledger 監査つき + **setupモード拒否は本計画で前倒し**（§1.2-12）。R14 稼働後、新規の変更系操作を R14 を通さず作ることを禁止（plan-review 基準へ追記） |
| R15 desired/reported・R9・南向き動詞 | DEFERRED（計画8） | tier は descriptor の属性であり追加=行追加 |
| UI 静的配信・mDNS 公開（D13/D2 Phase 2） | DEFERRED（計画9） | ルーターは `/api/v1` 配下に閉じ、`/` は 404 |
| R22 snapshot への秘密投入（TLS鍵・パスフレーズ/トークンハッシュ）+ 暗号化コンテナ | DEFERRED（計画6、デバイストークンと同時） | snapshot に秘密を**入れない**（Wave 0 の secrets 空=平文可を維持、D2 §3.5-3）。帰結: **fresh DB への復元**は setupモード再突入+証明書再生成（既存 DB への上書き restore なら admin_credential は残る——前提を§9に明記）。target_registry も snapshot 非対象（実測）なので「setup窓+上流接続済み」は復元では発生しない。復元後の再setup 必要性は README/runbook に明示 |
| 個人アカウント・外部認証サービス | DEFERRED（将来） | トークン発行側の差し替えで対応（D13決定2） |
| 物理アクション権限段階 | SEALED | tier enum に**存在しない**（D12決定1）。AI 昇格不可は tier_ceiling で構造化（D1監査追記） |
| レート上限の具体値（照会予算・操作レート） | DEFERRED（Wave 1 実測） | ログインスロットリングのみ本計画（§4.5）。操作レート上限は descriptor にフィールドだけ持たせ既定=無制限 |

## 2. 設計正本との対応（Global Constraints への全掃引）

| # | 正本 | 本計画での実装 |
|---|---|---|
| G1 | D3決定5: 制御プレーン=サーバー側TLS(自己署名+ピン)+operatorトークン。mTLS不採用 | §5 + §4。クライアント証明書は作らない |
| G2 | D3決定5例外（D13決定2）: setupモード窓は閉集合・パスフレーズ設定で閉鎖・actor=`setup_mode` | §4.3 |
| G3 | D13決定2: 認証オフスイッチを作らない | 設定に auth 無効化キーを**存在させない** |
| G4 | D13決定2: step-up・セッション長め・個人アカウントなし | §4.4 / §4.2 |
| G5 | D12決定3: 3分類+read-only。一括操作は昇格。表にない動詞は Daily 既定（新規追加規範） | §6.2/§6.3 |
| G6 | D12決定3: 監査粒度=actor+層・動詞・宛先とimpact_set・ID・dry-run結果・実行結果 | §6.4 |
| G7 | D1監査追記: operatorトークンは物理アクション段階へ昇格不可。攻撃者可制御文字列はデータとしてタグ付け | tier enum に物理段階なし。監査 detail は JSON 値として格納 |
| G8 | credential/トークンを Debug/ログ/エラー/監査に出さない | `Debug` 手書き `[REDACTED]`。step-up パスフレーズは **body 直下（params の外）**（§4.4）。発行応答でのみ平文1回 |
| G9 | R14: dry-run・事前条件・権限段階・レート制限・全操作監査。AI/人間共用 | §6 |
| G10 | D11決定6同型: 無認証面には試行逓増遅延+監査 | §4.5 |
| G11 | D13決定1: 静的配布物のみ・API は JSON | §7 |
| G12 | D13決定2: 無認証で見えるのは箱の識別とヘルス要約のみ | §7.1 `/api/v1/box` の応答を閉じた列挙で固定 |
| G13 | D2 §3.5 復元時トークン引き継ぎ（Wave 1+）→ 本計画は DEFERRED（§1.3/§9） | §9 |
| G14 | UI/AI/CLI が同一 R14 カタログを叩く | `standard_catalog()` 単一組み立て点（§6.5）。gatewayctl も同一関数 |
| G15 | D11決定7/D10決定7: 制御面をインターネットへ素面公開しない | **非プライベート source の常時拒否**（§7.6、設定で外せない） |

## 2.5 査読ラウンド（2026-07-08, codex gpt-5.5 xhigh + Claude review-max 並行・同一プロンプト）

現実照合: 起草者主張9件中、**反証2件**（①R11 範囲クエリは既に core/timeseries の pub fn——「SQL抽出」前提崩れ、起草者も独立に自己検出済み。②sha2 は core/registry に既存）、**精密化2件**（registry の R14 マーカーは policy.rs:40 が該当・:344 はテスト模擬 / HealthState は手書きレンダラで serde 不可）。他は確認。両者の指摘を全裁定・反映:

| 出所 | 重大度 | 指摘 | 反映 |
|---|---|---|---|
| both | 高 | setupモード閉集合の `device.retire` は D13 逸脱（復元後 setup 再突入で LAN から無認証 retire） | §4.3: retire を閉集合から**除外** |
| codex | 高 | setup中 `GET /readings`（履歴範囲）は D13「ライブ値」より広い | §7.1: `GET /live`（series毎の最新値のみ）を新設し setup 閉集合はこちら。`/readings` は Bearer 固定 |
| Fable | 高 | axum-server 0.7 `tls-rustls` は rustls の aws-lc-rs を引き込み、既存 ring（reqwest 経由）とプロバイダ衝突→実行時 panic + RPi ビルド摩擦 | §5.4: axum-server 0.8 `tls-rustls-no-provider` + ring provider 明示 install。計画に cargo check スパイク |
| both | 高 | OpDescriptor `&mut Connection` は DbHandle（`&Connection`）と不整合 | §6.1: op は `&rusqlite::Transaction` を受け、dispatch が `unchecked_transaction()` で作る（前例 migrate.rs） |
| Fable | 高 | dispatch 全体を単一Txにしないと TOCTOU（precondition 古読み） | §6.1: tier判定の token 読み→precondition→execute→監査を**1回の with_conn+1 Immediate Tx** |
| codex | 高 | 失敗監査と rollback の矛盾 | §6.1: 外側Tx + operation SAVEPOINT。失敗時は savepoint rollback→失敗監査→外側 commit |
| Fable | 高 | dry_run「書き込みゼロ」×「dry_run も監査」の矛盾 | §6.1: 「**ledger_events を除き**書き込みゼロ」に精密化（テストも同様） |
| both | 高 | R11「SQL 抽出」タスクの前提崩れ | §1.2-9/§7.3 書き換え（既存 fn 再利用 + series_key 解決 + 応答スキーマ） |
| Fable | 高 | カタログ組み立てモデル未解決（2箇所組み立てのドリフト） | §6.5: `core/ops` が storage/ledger/registry に依存し `standard_catalog()` を単一提供（依存は一方向のまま。registry→collector 依存は存在しない） |
| both | 中 | bind 0.0.0.0 と「LAN限定」が未接続（無認証 setup 窓が公的経路に出る） | §7.6: 非プライベート source 常時拒否（ハードコード・設定で外せない）+ 起動ログで bind/インターフェース表出 |
| both | 中 | step_up_passphrase が params 経由だと監査に平文が残る | §4.4: body 直下（params 外）に固定。監査は params のみ記録 |
| Fable | 中 | bulk×step-up×setup の相互作用未定義 | §4.4/§4.3: step-up は**実効 tier**基準。setupモードでは bulk（昇格発生）を一律拒否（パスフレーズ不在で step-up 不能のため） |
| both | 中 | target 登録の setupモード拒否が計画7まで未接続 | §1.2-12: gatewayctl target add に前倒し |
| codex | 中 | スロットリングと argon2 の順序・タイミング攻撃 | §4.5: throttle→ハッシュ読み→**ロック外で** argon2 照合。トークン照合は SHA-256+index（前像不可）で定数時間比較不要（Fable 検証）だが応答は均一化 |
| Fable | 中 | OpDescriptor に宛先抽出の口が無い（bulk 判定・監査 targets が導けない） | §6.1: `targets: fn(&Value) -> Vec<String>` を追加 |
| Fable | 中 | argon2 0.5 と rand 0.9 の rand_core 不一致 | Tech Stack: salt=password-hash 同梱 OsRng、トークン=getrandom 0.3（rand 依存を持たない） |
| Fable | 小 | `local_cli` トークン kind は発行経路が無い（型だけの概念） | §4.1: kind は `human\|ai` のみ。`local_cli` は監査 actor_kind 専用 |
| codex | 小 | Tier enum に ReadOnly を混ぜる語彙ねじれ | §1.2-2: ReadOnly は ceiling 専用値、descriptor には使用不可（assert） |
| Fable | 小 | last_used_at 毎回 UPDATE は全 GET を書き込み化 | §4.1: 前回更新から 60 秒超のときのみ UPDATE |
| Fable | 小 | TLS ファイルの原子性・片方欠損・ディレクトリ権限 | §5.2: temp+rename、片方欠損は両方再生成、tls/ 0700。100年証明書の Apple 825日ルール注意を明記（対象端末は Android=D13、受け入れ） |
| Fable | 小 | グローバル 10req/s は正規ログイン締め出し DoS を許す / per-source は IP 差し替えで迂回可 | §4.5: 意図したトレードオフとして明記（グローバル上限が下支え） |
| Fable | 小 | setup/passphrase 同時 POST の競合 | §4.3: id=1 INSERT の先勝ち、後着は 409 |
| Fable | 小 | §6.2 移管元の行番号誤り | §6.2: policy.rs:40（warnログ経路）に修正 |
| Fable | 小 | /readings 応答スキーマ・series 解決手段の未指定 | §7.3 |
| Fable | 小 | 「全ルート Bearer 必須（box除く）」が session と矛盾 | §4.3 文言修正（box/session/setup を除く） |
| Fable | 小 | D13 保留（復元と setup 再突入の意味論）を本 spec が具体化した旨の還流が無い | §13 設計正本への還流 |

### iter2（着地確認、2026-07-08、同2系統並行）

裁定表24行の本文着地は両者が確認（古い本文の残り無し）。新規指摘を全採用:

| 出所 | 重大度 | 指摘 | 反映 |
|---|---|---|---|
| both | 高 | `unchecked_transaction()` の既定は Deferred——Immediate は `Transaction::new_unchecked(conn, Immediate)`（前例は devices.rs:86/retention.rs:142/actor.rs:180。migrate.rs は Deferred） | §6.1 修正 |
| both | 高 | rusqlite `Savepoint` 型は `&mut Transaction` 要求で `fn(&Transaction)` と両立しない | §6.1: SQL 直発行（SAVEPOINT/ROLLBACK TO/RELEASE。実機動作確認済み）。dry_run は成功時も rollback |
| codex | 中 | `define_custom` は実在せず、custom 新設の入力（semantic_class/channel_mode/entry_revision）未指定 | §6.2: `define_custom_entry` 新設を全スキーマで指定 |
| codex | 中 | setup中 `/health` full JSON は D13「ヘルス要約」より広い（adapters/target 詳細を含む） | §7.1: setup中 401、要約は /box |
| both | 中 | IPv6: mapped アドレスの unmap 必須 / IPv6 GUA の LAN は許可リスト外 / `[::]` bind で全滅 | §7.6: IPv4 bind のみ許容+unmap+GUA 非対応明記 |
| codex | 中 | rustls 直接依存の feature 指定がないと aws-lc-rs 既定が混入 | Tech Stack/§3: `default-features=false, features=["ring","std","tls12"]` |
| Fable | 中 | §3 の依存方向の事実が逆（registry→collector が実物。ops に collector が推移的に入る） | §3 修正（直接依存の規律として言い直し） |
| Fable | 中 | `/series` 応答の series_id が §7.3 の「外に出すのは CLI のみ」と矛盾 | §7.1: series_id を応答から除去 |
| Fable | 中 | series_key は列でなく派生値（合成 fn は gateway record.rs:32 の私有） | §7.3: 合成/分解を core/ledger へ移設 |
| codex | 小 | base64 依存の欠落 | Tech Stack/§3: base64 0.22 URL_SAFE_NO_PAD |
| codex | 小 | setup中 bulk のステータス未固定 | §4.3: 閉集合外=401 / bulk=403 |
| Fable | 小 | getrandom 0.3 は木に3本目の重複 | Tech Stack: 0.4 へ |
| Fable | 小 | system_id は hardware_id/token_id 宛の op で成立しない | §6.4 精密化 |
| Fable | 小 | `/live` の実装源・render_health_json の pub 化・api=None 復帰機構の未記載 | §1.2-9/§7.4 |
| Fable | 小 | Tailscale(CGNAT) 直アクセスが塞がる旨の案内 | §7.6 |

確認済み（OK）: §5.4 の axum-server 0.8 feature 構成と rustls API 名は実物どおり / series_key の
4分割 parse は D6 コロン禁止により一意 / 正規ユースケース（LAN スマホ・Docker・SSH フォワード）は
§7.6 ガードを通る / `cargo test --workspace` 466 passed 実測一致。

## 3. クレートと依存

```
core/ops (新規 iotkit-core-ops)
  依存: core/storage, core/ledger, core/registry, serde/serde_json,
        argon2, sha2, getrandom 0.4, base64 0.22, thiserror
  提供: catalog（OpDescriptor/dispatch/standard_catalog）, auth（パスフレーズ/トークン/setup判定）,
        MIGRATIONS（0012_ops.sql）
  ※依存は一方向: ops→{storage,ledger,registry}。**注意（iter2 現実照合）: registry が collector に依存する向き**
    （registry/Cargo.toml:7、policy.rs が RegistryVerdict を使用）のため、ops には collector が**推移的に**入る。
    循環はなく、ops が collector/publish/timeseries を**直接**知らないことが規律（推移的依存は許容と明記）。

iotkit-gateway
  追加依存: axum 0.8, axum-server 0.8 (features=["tls-rustls-no-provider"]),
            rustls（ring provider 明示）, rcgen 0.13, core/ops
  追加モジュール: src/api/{mod.rs, tls.rs, auth_layer.rs, routes.rs}
  ※readings ルートが core/timeseries::query を呼ぶのは gateway 側の合成（composition root）

iotkit-gatewayctl
  追加依存: core/ops
  追加コマンド: passphrase, fingerprint, token（+ target add への setup 拒否1チェック）
```

- **やらないこと**: `core/types` への追加なし（3語彙分離の維持=D4）。engine/AdapterEvent 無改修。

## 4. 認証モデル

### 4.1 データ（migration `core/ops/migrations/0012_ops.sql`）

```sql
CREATE TABLE admin_credential (
  id INTEGER PRIMARY KEY CHECK (id = 1),   -- 単一行
  passphrase_hash TEXT NOT NULL,           -- argon2id PHC 文字列
  set_at INTEGER NOT NULL,                 -- unix ms
  updated_at INTEGER NOT NULL
);
CREATE TABLE operator_tokens (
  token_id TEXT PRIMARY KEY,               -- "tok_" + base64url(16byte乱数)=22文字（表示・監査用の公開ID）
  name TEXT NOT NULL,
  token_hash BLOB NOT NULL,                -- SHA-256(トークン平文)
  kind TEXT NOT NULL CHECK (kind IN ('human','ai')),
  tier_ceiling TEXT NOT NULL CHECK (tier_ceiling IN ('read_only','routine','daily','construction')),
  is_session INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  expires_at INTEGER,                      -- NULL=無期限（AI ハーネス用長命）
  revoked_at INTEGER,
  last_used_at INTEGER,
  CHECK (kind != 'ai' OR tier_ceiling IN ('read_only','routine'))
);
CREATE UNIQUE INDEX idx_operator_tokens_hash ON operator_tokens(token_hash);
```

- **setupモード判定 = `admin_credential` に行が無いこと**（状態の二重管理をしない）。
- トークン平文は `iko_` + 32byte 乱数（getrandom）の base64url（43文字）。応答で1回だけ返し、以後 SHA-256 で照合
  （hash の UNIQUE index 引き当て。256bit 乱数の前像は詰められないため定数時間比較は不要——応答・遅延は成功/失敗で均一化する）。
- `last_used_at` は前回更新から 60 秒超のときのみ UPDATE（全 GET を書き込みにしない）。
- **AI トークンの構造遮断**: `operator_token.issue` の事前条件 + DB CHECK の二重（defense-in-depth）。
- `local_cli` は**トークン kind ではなく監査 actor_kind**（gatewayctl 直実行の記録用）。

### 4.2 セッション（ログイン）

- `POST /api/v1/session {passphrase}` → argon2id 照合（§4.5 の順序）→ `kind='human',
  tier_ceiling='construction', is_session=1, expires_at=now+30日` のトークンを発行して返す。
  更新は re-login（スライディングは Wave 1 実測後）。
- セッションは `operator_token.revoke` で個別失効可能（パスフレーズ変更時の全セッション失効は D13 保留どおり後続）。

### 4.3 setupモード gate

- setupモード中に**認証なしで**許される API の閉集合:
  `GET /api/v1/box|health|series|live|ops`、
  `POST /api/v1/ops/registry.resolve_unknown_key`、`POST /api/v1/ops/device.approve_sighting`、
  `POST /api/v1/setup/passphrase`（モードの出口）。
  **`device.retire`・`operator_token.*`・`GET /readings`（履歴範囲）は setupモードでは不可**
  （401。D13 の閉集合=登録・検疫解決・ライブ値のみ。retire はデバイスの沈黙化であり D11 失効系「人間のみ」と同族——
  無認証窓に置かない）。
- setupモード中は **bulk（複数宛先）を一律拒否**（昇格先 Construction の step-up がパスフレーズ不在で不可能なため。
  一括解除はパスフレーズ設定後に行う）。**ステータス固定: setup中の閉集合外=401、setup中の bulk=403**。
- 監査 actor は `setup_mode`（token_id なし・接続元アドレスを detail に記録）。
- `POST /setup/passphrase` の同時競合は id=1 INSERT の**先勝ち**（後着 409）。
- パスフレーズ設定後は、`/api/v1/box`・`POST /session`・`POST /setup/passphrase`（常に409になる）を除く
  全ルートが Bearer 必須。
- **閉じ圧力**の本計画分: `/api/v1/box` と health JSON に `setup_mode: true` を常時表出 +
  `gatewayctl target add` の setup 拒否（§1.2-12）。R23 表示・Phase 6 不合格は各担当計画で参照。

### 4.4 step-up（Construction）

- **実効 tier**（descriptor.tier を bulk 昇格で1段上げた後の値）が Construction の操作は、リクエスト body の
  **`step_up_passphrase` フィールド（トップレベル。`params` の外）**必須。照合失敗は 403 + 監査（dispatch されない）。
- 監査 detail に記録するのは `params` のみ——step_up_passphrase は構造的に監査へ到達しない（G8）。
- 初期カタログで素の Construction は `operator_token.issue` のみ。Daily の bulk（approve_sighting/retire/revoke の
  複数宛先）も実効 Construction となり step-up 必須。

### 4.5 ログイン試行スロットリング

- 対象: `POST /session` と `step_up_passphrase` 照合。**順序: throttle 判定（照合前）→ ハッシュ読み（with_conn）→
  ロック解放 → argon2 照合（別 spawn_blocking。DB Mutex を握って KDF を回さない）**。
- per-source（接続元IP）逓増遅延（失敗 n 回目 → min(2^n, 60) 秒 429）+ グローバル上限（10 req/s 超 429）。
  in-memory（再起動でリセット。永続ロックアウトは作らない）。
- **明記するトレードオフ**: per-source は LAN 内 IP 差し替えで迂回可能であり、グローバル上限が下支え。
  グローバル上限は LAN 内攻撃者による正規ログインの締め出し DoS を許すが、「ロックアウトで現場が入れない」
  より安全側（攻撃者が LAN にいる時点で D11 の脅威モデル外）。
- 失敗は `auth_failed` 監査（source、対象=session|step_up。パスフレーズ本文なし）。

## 5. TLS（自己署名 + フィンガープリント）

### 5.1 生成

初回起動時、`{db_path 親}/tls/cert.pem` + `key.pem` が無ければ rcgen で生成（CN=`iotkit-gateway`,
SAN=`iotkit-gateway.local`+ホスト名, 有効期限 **100年**——信頼の実体はピン=フィンガープリントであり
X.509 期限ではない。D3決定5/D10決定1 と同思想。※Apple 系ブラウザの 825日ルールで警告バイパス不能に
なり得る既知の互換性注意——D13 の対象端末は廉価 Android であり受け入れ、必要なら将来短期化+自動再生成)。

### 5.2 ファイル規律

- tls/ ディレクトリ 0700、key.pem 0600。書き込みは temp+rename（health JSON と同じ流儀）。
- **cert/key の片方欠損時は両方を再生成**（不完全ペアで起動しない）。

### 5.3 フィンガープリント

SHA-256(証明書DER) 16進コロン区切り。表出3経路: 起動ログ / `GET /api/v1/box` / `gatewayctl fingerprint`。

### 5.4 rustls プロバイダ（査読反映）

ワークスペースには reqwest 経由で **rustls 0.23 + ring** が既に居る。axum-server の `tls-rustls` feature は
aws-lc-rs を引き込み**プロバイダ二重化で実行時 panic / RPi ビルド摩擦**を生むため:
`axum-server 0.8` を `features=["tls-rustls-no-provider"]` で採用し、起動時に
`rustls::crypto::ring::default_provider().install_default()` を1回呼ぶ。計画の最初のタスクで
**cargo check スパイク**（依存が解決し hello-TLS が立つこと）を行う。

- API サーバーは HTTPS のみ（平文 HTTP リスナーなし・リダイレクトなし）。
- 証明書再生成 = ファイル削除+再起動のみ（操作カタログに入れない）。

## 6. R14 操作カタログ

### 6.1 形と dispatch 規律

```rust
pub struct OpDescriptor {
    pub name: &'static str,
    pub tier: Tier,                  // Routine | Daily | Construction（ReadOnly 不可、assert）
    pub bulk_escalates: bool,
    pub params_schema: fn() -> serde_json::Value,
    /// params から宛先集合を抽出（bulk 判定と監査 targets/impact_set の源。単一宛先なら len=1）
    pub targets: fn(&serde_json::Value) -> Vec<String>,
    pub preconditions: fn(&rusqlite::Transaction<'_>, &serde_json::Value) -> Result<(), OpError>,
    pub dry_run: fn(&rusqlite::Transaction<'_>, &serde_json::Value) -> Result<serde_json::Value, OpError>,
    pub execute: fn(&rusqlite::Transaction<'_>, &serde_json::Value) -> Result<serde_json::Value, OpError>,
}
```

- **dispatch 全体を 1 回の `with_conn` + 1 つの Immediate Tx に入れる**（TOCTOU 排除。単一 Mutex 前提を利用）:
  **`rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)`** で外側 Tx を開く
  （iter2 修正: `conn.unchecked_transaction()` の既定は Deferred。Immediate の前例=gatewayctl devices.rs:86 /
  gateway retention.rs:142 / collector actor.rs:180）。
  ①トークン読み+tier 判定 → ②params validate → ③preconditions → ④**operation SAVEPOINT** 内で
  dry_run または execute → ⑤失敗なら savepoint rollback → ⑥監査 INSERT → ⑦外側 commit。
  これにより「操作は成ったが監査が無い」も「失敗監査が rollback で消える」も構造的に起きない。
- **SAVEPOINT は SQL 直発行**（`tx.execute_batch("SAVEPOINT op")` → 失敗/dry_run 時
  `ROLLBACK TO op; RELEASE op` / 成功時 `RELEASE op`）。rusqlite の `Savepoint` 型は `&mut Transaction`
  を要求し `fn(&Transaction)` コールバックと両立しないため使わない（iter2 で SQLite 実機動作確認済み）。
  `Transaction` は `Deref<Target=Connection>` なので既存 fn（approve_sighting 等 `&Connection` 引数）は
  そのまま `&tx` で呼べる。
- dry_run は **savepoint を成功時も必ず rollback** し、**ledger_events（監査）を除き書き込みゼロ**を
  構造で保証する（テストは ledger_events 以外のテーブルの前後比較で検証）。
- argon2 照合（step-up）だけは Tx の**外**（§4.5 の順序。KDF で DB Mutex を握らない）。照合成功後に
  with_conn へ入る。照合と dispatch の間の TOCTOU は「パスフレーズが変わった」ケースのみで、
  管理者自身の操作なので受け入れる（一文明記）。

### 6.2 初期カタログ

| name | tier | bulk | 中身（移管元） |
|---|---|---|---|
| `registry.resolve_unknown_key` | Daily | — | 検疫キーの解決（D6決定9 の2経路。移管元=`core/registry/src/policy.rs:40` の「明示解決(R14)…Wave 0 は warn ログ」経路）。**エイリアス枝**: 既存 `define_alias(alias=申告キー, target, AliasKind::SiteMapping)`。params: `{"key":"temp","resolution":{"alias_to":"temperature_c"}}`。**custom枝**: 新設 fn `define_custom_entry`（`define_custom` は現存しない——iter2 反映）: `registry_entries` に origin='custom'・catalog_version=NULL・entry_revision=内容ハッシュ（CatalogEntry::revision() と同レシピ）で INSERT + 申告キー→custom キーの alias。params: `{"key":"temp","resolution":{"custom":{"measurement_key":"custom.tank_temp","unit_ucum":"Cel","unit_display":"°C","value_type":"float","semantic_class":"sensor","channel_mode":"single","channel_roles":null,"physical_min":null,"physical_max":null}}}`（measurement_key は `custom.` 接頭辞必須=D6決定9、channel_mode='fixed' のときのみ channel_roles 必須）。衝突検査は enable_entry/define_alias と同一（entry 既存・alias 既存・名前空間衝突）。**op は registry の変異のみ**——既存検疫行の遡及解除は行わない（検疫遷移の配送=D5/計画7 領分。以後の受信が alias 経由で解決される） |
| `device.approve_sighting` | Daily | true | `core/ledger::approve_sighting`（store.rs:594、sighting_approved 監査は既存のまま二重化しない——§6.4 注記）。params: `{"hardware_ids":["..."]}` |
| `device.retire` | Daily | true | `core/ledger` retire 系 fn のラップ。params: `{"system_ids":["..."]}` |
| `operator_token.issue` | Construction | — | §4.1。kind='ai' は tier_ceiling≤routine を事前条件+CHECK で強制。params: `{"name":"ai-harness","kind":"ai","tier_ceiling":"routine","expires_at":null}` |
| `operator_token.revoke` | Daily | true | revoked_at 設定（セッション含む）。params: `{"token_ids":["tok_..."]}` |

- 上記以外の既存 CLI 操作（target 系・snapshot 系・replace 系）は移管しない（計画7/8）。
  ただし本計画マージ後、**新規の変更系操作を R14 を通さず作ることを禁止**（plan-review 基準へ追記）。

### 6.3 Tier

```rust
pub enum Tier { ReadOnly, Routine, Daily, Construction }   // 序列あり
```

- ReadOnly はトークン ceiling 専用（照会系 GET は「有効なトークンなら tier 不問」の実体）。
- 実効 tier = descriptor.tier を `targets(params).len() > 1 && bulk_escalates` なら1段昇格。
- 新規操作の追加規範:「表にない動詞は Daily 既定、引き下げは spec 改訂で」（D12決定3=G5）。

### 6.4 監査（G6 粒度）

- kind: `r14_op`（dispatch 1回=1行、dry_run 含む）。system_id: **宛先が system_id である op の単一宛先のみ**設定
  （hardware_id 宛の approve_sighting・token_id 宛の revoke は NULL——targets 配列が正）。
- detail JSON: `{"op":"...","actor":"tok_…|setup_mode|local_cli","actor_kind":"human|ai|local_cli|setup_mode","tier":"daily","effective_tier":"construction","dry_run":false,"params":{...},"result":"ok|error:<code>","targets":[...],"source":"192.168.1.20"}`
- 既存 fn（approve_sighting 等）が内部で自前監査（sighting_approved）を書く場合は**そのまま残す**
  （r14_op 行は dispatch の記録、既存行はドメインイベントの記録——役割が違う。二重を仕様として明記）。
- 認証系: `auth_session_issued` / `auth_failed` / `operator_token_issued` / `operator_token_revoked` /
  `admin_passphrase_set` / `admin_passphrase_reset`。パスフレーズ・トークン平文/ハッシュは detail に入れない。

### 6.5 カタログの組み立て（G14）

`core/ops` が storage/ledger/registry に依存し、`pub fn standard_catalog() -> &'static [OpDescriptor]`
を**単一の組み立て点**として提供する。gateway（API dispatch）と gatewayctl（token/将来の ops CLI）は
これだけを参照する——2箇所組み立てのドリフトを作らない。

## 7. API サーフェス（/api/v1）

### 7.1 エンドポイントと認証マトリクス

| Method/Path | 通常時 | setupモード中 | 応答 |
|---|---|---|---|
| GET `/api/v1/box` | **無認証** | 無認証 | `{gateway_name, epoch, version, setup_mode, tls_fingerprint, health_summary:{status, adapters_alive}}`（この列挙で固定=G12） |
| POST `/api/v1/setup/passphrase` | 409（設定済み） | **無認証**（モードの出口） | セッショントークン返却 |
| POST `/api/v1/session` | パスフレーズ | 409（未設定） | セッショントークン返却 |
| GET `/api/v1/health` | Bearer | **401**（setup中は /box の要約のみ——full health は adapters/target 詳細を含み D13「ヘルス要約」より広い。iter2 反映） | render_health_json と同一出力（§7.4） |
| GET `/api/v1/series` | Bearer | 無認証 | series 一覧 `[{series_key, system_id, user_label}]`（**series_id は出さない**——外部表現は series_key=D5決定3。iter2 反映） |
| GET `/api/v1/live` | Bearer | 無認証 | **series ごとの最新値のみ** `[{series_key, event_time, event_time_source, quarantined, values}]`（D13「ライブ値確認」の実体） |
| GET `/api/v1/readings?series_key=&from_ms=&to_ms=&limit=&include_quarantined=` | Bearer | **401** | R11 範囲クエリ（§7.3） |
| GET `/api/v1/ops` | Bearer | 無認証 | カタログ列挙（name/tier/bulk/params_schema） |
| POST `/api/v1/ops/{name}` | Bearer（+step-up） | §4.3 の閉集合のみ | `{params, dry_run, step_up_passphrase?}` → dispatch 結果 |

- エラー形: `{error:{code,message}}`。401/403/404/409/422/429。Body 上限 64KB・タイムアウト（D1 の流儀）。

### 7.3 readings（R11）

- **既存の** `core/timeseries::query::query_readings_v3(conn, series_id, from, to, limit, include_quarantined)`
  を呼ぶ（SQL 抽出はしない——Wave 0 計画4 で共有化済み。CSV も core 側 `export_csv` が既存）。
- 新規: `core/ledger` に `find_series_by_key(conn, &str) -> Result<Option<i64>>` を追加。
  **series_key は列ではなく派生値**（series 実表は system_id/measurement_key/channel_index/variant の
  UNIQUE。iter2 確認）: 合成関数 `series_key_of` は現在 iotkit-gateway/src/record.rs:32 の私有——
  **合成/分解を core/ledger へ移設**し、record.rs と API がそれを参照する。分解はコロン4分割で一意
  （measurement_key はコロン禁止=D6決定2）。API の外部表現は series_key=D5決定3
  （series_id を外に出すのは CLI のみの既存事情として維持）。
- from_ms/to_ms **必須**（無制限スキャン禁止=D7決定9。時間軸は event_time 基準）。応答:
  `{"series_key":"...","rows":[{"seq":n,"event_time":n,"event_time_source":"...","quarantined":false,"values":[...]}]}`。
- `include_quarantined` 既定 false。

### 7.4 R12 統合

- `HealthState` に `api: Option<ApiHealth {bind, tls_fingerprint}>` を追加し、**手書きレンダラ
  `render_health_json` にセクションを足す**（serde 化はしない——HealthState は `Instant` を含み serde 不可。
  現行の手書き JSON 方式を維持。レンダラは現在**私有 fn なので pub 化**する——iter2 反映）。
- `GET /api/v1/health` はファイル書き出しと**同じレンダラ関数**の出力を返す（二重実装しない）。
- API タスクは終了時（JoinHandle 消費側の監視 or Drop ガード）に health_state の `api` を None に戻す
  （「api セクション消失で表出」の機構を明示——iter2 反映）。

### 7.5 設定（config.rs 追加）

```toml
[api]
enabled = true          # 既定 true。false は保守用（リスナーを立てないだけ。認証は弱めない=G3）
bind = "0.0.0.0:8443"
```

### 7.6 プライベート発ガード（G15。査読反映）

- 全リクエストに対し、接続元アドレスが **loopback / RFC1918 / IPv6 ULA(fc00::/7) / link-local** の
  いずれでもなければ **403**（ハードコード。設定で外せない——認証オフスイッチと同じ理由で「公開スイッチ」も
  作らない。インターネットに晒された箱の setup 窓・/box が世界に開く事故を構造で防ぐ。D11決定7/D10決定7 と同思想）。
- **IPv6 の扱い（iter2 反映）**: bind は **IPv4 のみ許容**（`[::]` 等の IPv6 bind は config validate で拒否、
  Wave 1 非対応と明記）。判定前に IPv4-mapped アドレス（`::ffff:a.b.c.d`）は **unmap（to_canonical 相当）**
  してから照合する。IPv6 GUA しか持たない LAN は Wave 1 非対応（判明済みトレードオフとして受け入れ）。
- Tailscale 等 CGNAT（100.64.0.0/10）直アクセスは塞がる——SSH ポートフォワード（loopback）で代替
  （runbook に一言。プロキシ/NAT 越し利用は Wave 1 スコープ外——そういう構成は D10 のトンネルの領分）。
- 起動ログに bind と有効インターフェースを騒がしく表出。

## 8. gatewayctl 追加コマンド

| コマンド | 動作 | 監査 |
|---|---|---|
| `gatewayctl passphrase reset` | 対話で新パスフレーズ→argon2id 更新（行が無ければ作成=setup終了と同義） | `admin_passphrase_reset`, actor=`local_cli` |
| `gatewayctl fingerprint` | tls/cert.pem の SHA-256 表示（無ければ「未生成」） | なし（読み） |
| `gatewayctl token issue --name X --kind ai\|human --tier T` | `standard_catalog()` の同一 fn 経由（平文1回表示） | `operator_token_issued` |
| `gatewayctl token revoke --id tok_…` / `token list` | 同上 | `operator_token_revoked` |
| （既存改修）`gatewayctl target add` | **admin_credential 行が無ければ拒否**（D13 閉じ圧力3 の前倒し） | 既存監査のまま |

## 9. R22/復元との相互作用（本計画の割り切り）

- snapshot は Wave 0 のまま（secrets 空・平文可）。TLS 鍵・admin_credential・operator_tokens は含めない。
- 帰結（明文で受け入れ）: **fresh DB への復元**では (a) setupモードに戻る（パスフレーズ再設定が必要——
  README/runbook に明示）、(b) TLS 証明書が再生成されフィンガープリントが変わる（ピン留め相手は未存在）。
  既存 DB への上書きでない restore フローが対象。target_registry も snapshot 非対象（現実測）のため
  「setup 窓 + 上流接続済み」という状態は復元では発生しない。
- 計画6（デバイストークン導入）で D2 §3.5 どおり secrets セクション+暗号化コンテナに移行し、この割り切りを解消。
- エポックフェンスとの関係: 本計画は epoch を読むだけ（`/api/v1/box` 表出）。変更なし。

## 10. 障害と運用

- API タスクは `run()` 内で spawn。bind 失敗は**起動失敗**（fail fast、DB init と同格）。実行中の予期しない
  終了は tracing error + health `api` セクション消失で表出（プロセスは続行——取り込み・配送は独立に生きる）。
- graceful shutdown: ctrl_c で axum server へ shutdown シグナル。
- ログ: リクエストは trace レベル（method/path/status/latency。Authorization ヘッダ・body は出さない=G8）。

## 11. テスト計画（骨子。詳細は writing-plans で展開）

- **auth 単体**: argon2 照合往復 / トークン発行→SHA-256 照合→revoke 拒否 / expires_at 超過拒否 /
  AI ceiling 超え issue が事前条件+CHECK 両方で落ちる / setupモード判定 / last_used_at 間引き
- **tier 執行マトリクス**: ceiling 4種 × 初期カタログ 5op ×（単数/bulk）×（step-up 有無）の許可/403 表を
  そのままテスト化 / setupモード中の bulk 一律拒否
- **setupモード gate**: 閉集合の各ルート 200 / 閉集合外（retire・token 系・readings）401 /
  passphrase 設定後に gate が消え Bearer 必須化 / actor=`setup_mode` 監査 / 同時 POST 先勝ち 409
- **スロットリング**: 連続失敗で 429 と逓増 / 成功でリセット / `auth_failed` 監査 / throttle が argon2 照合より先
- **dispatch**: dry_run が ledger_events 以外書き込みゼロ（前後比較）/ execute 失敗時に savepoint rollback +
  失敗監査が残る / 監査 INSERT 失敗で操作ごと rollback / 存在しない op=404 / targets 抽出と bulk 昇格
- **HTTPS 統合**: ring provider install → 自己署名受け入れ reqwest で end-to-end（cert 生成→fingerprint 一致→
  setup→session→dry_run→execute→ledger_events 検証）/ cert・key 再起動再利用 / 片方欠損で両方再生成 /
  **非プライベート source 403**（ソケットレベルで擬似）
- **readings/live/series**: series_key 解決 / from/to 欠落 422 / CLI と API が同一入力で同一行 /
  live が「series ごと最新1件」のみ返す
- **回帰**: 既存 466 テスト全緑 + clippy -D warnings クリーン

## 12. 受け入れ基準

1. `cargo test --workspace` 全緑 + clippy クリーン（CI）
2. 開発機で: 初回起動→fingerprint 表示→`/api/v1/box` が setup_mode:true→passphrase 設定→
   setup_mode:false→session→`device.approve_sighting` dry_run→execute→ledger_events に G6 粒度の監査行
3. AI 用トークン（kind=ai）で Construction 相当（issue / bulk 操作）が 403、Routine 相当が成功
4. 認証オフ・公開許可の設定キーが**存在しない**こと（grep で確認）
5. progress.md 更新: 計画5 クローズ、計画6（R2 入口）のブロッカー解除を記録

## 13. 設計正本への還流（実装完了時）

- D13 保留「工場リセット=setupモード再突入の意味論」の一部を本 spec が具体化（fresh DB 復元=再突入、
  既存 DB 上書きは非該当）——D13 保留節へ「計画5 spec で部分確定」の注記。
- D13決定2 の回復経路（gatewayctl passphrase reset）実装確定の注記。
- 台帳 R14 行に「Wave 1 計画5 で骨格実装」注記（実装完了時の還流コミットで）。
