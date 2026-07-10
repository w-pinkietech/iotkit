# Wave 1 計画5: 繰り延べた強化項目（deferred hardening list）

**目的**: 初期開発では素早く柔軟に進める方針（ユーザー裁定 2026-07-08）のもと、「今はテストしない/実装しないと決めた」制約を append-only で記録する。全体像が固まった時点の**強化パス（独立計画）**を「考古学」でなく「チェックリスト消化」にするための保険。Wave 0 の `phase1-hardening` と同じ流儀。

各項目: 何を・なぜ後回し・どこで拾うか。

---

## D-1: r14_op 監査の system_id 列は plan5 では常に None

- **内容**: `dispatch` は `record_event(&tx, "r14_op", None, &detail)` で、ledger_events.system_id 列を常に NULL にする。spec §6.4 は「宛先が system_id である単一宛先 op のみ system_id 設定」を望むが、plan5 では設定しない。
- **なぜ後回し**: 宛先は detail JSON の `targets` 配列に**完全に記録済み**——監査は情報を失っていない。system_id 列は「system_id で監査をクエリする」将来の利便のためのもので、完全性・正しさの要件ではない。OpDescriptor に system_id 抽出フックを足すのは、対象が `device.retire`（単一 system_id 宛）にほぼ限られ、初期に足すと framework が太る。
- **どこで拾うか**: 監査クエリ面（R13 インシデントバンドル）を実装する強化パス。OpDescriptor に `audit_system_id: Option<fn(&Value)->Option<Vec<u8>>>` を足し、retire 等で設定。独立レビュー Task 3 [Important] 由来。

---

## D-2: dispatch の監査 INSERT 失敗パスの直接テストが無い

- **内容**: 「execute 成功後に監査 INSERT が失敗したら操作ごと rollback される」ことはコード構造上は成立（commit 前に Err → Tx drop で rollback）だが、これを強制する直接テストが無い（監査 INSERT を故意に失敗させる手段＝trigger 等が要る）。
- **なぜ後回し**: 発生条件が極めて稀（同一 conn・同一 DB で監査 INSERT だけが失敗）で、テスト用に SQLite trigger を仕込むのは初期にはコスト過大。
- **どこで拾うか**: 強化パスで `ledger_events` に一時 trigger を張って INSERT 失敗を注入するテストを追加。spec §11「監査 INSERT 失敗で操作ごと rollback」由来。

## D-3〜D-6: Fable掃引レビュー(2026-07-08)の繰り延べMinor

Fable review-max が制御プレーンの土台(core/ops+api)全体を掃引した際の Minor 発見のうち、初期には後回しにして強化パスで拾うもの。Important(I-1〜I-4)+ 高価値Minor(M-2/4/6/7/8, log level)は T8 マージ時に反映済み。

- **D-3 (M-1)**: registry_ops の `optional_string` が明示 `null` を Validation エラーにする一方、`channel_roles`/`physical_min` は null 許容。同一オブジェクト内で null 規約が不統一。→ 強化パスで null 規約を統一。
- **D-4 (M-3)**: guard.rs の throttle `sources` HashMap が期限切れ掃除なしで堆積、`blocked_until` 経過後も `failures` カウントが減衰しない(指数が伸び続ける)。プライベート帯域限定で上限はあるが、長期稼働のメモリ堆積と失敗カウント減衰を強化パスで。
- **D-5 (M-9)**: catalog.rs の dispatch で SAVEPOINT cleanup(`ROLLBACK TO op; RELEASE op`)自体が失敗した場合、Err 後も外側で監査 INSERT→commit を試みる理論経路。接続破損時は commit も失敗するため実害ほぼ無いが、cleanup Err 時は commit せず Tx drop(rollback)に倒すのが安全。
- **D-6 (M-10)**: fingerprint.rs が複数 CERTIFICATE ブロックの base64 を連結して1 DER と解釈。現状は常に単一自己署名で実害なし。将来チェーン対応時の地雷。
- **I-5 (Task 10 引き継ぎ、繰り延べではない)**: API タスク終了時に `health.api` を None へ戻す機構(Drop ガード or join 監視)が api モジュールに無い。**Task 10 で main が spawn_api_task の join を監視し、予期せぬ終了時に health から api セクションを消す配線を必須実装**すること(spec §7.4)。放置すると死んだ API が health 上で生き続ける。

## D-7〜D-8: Task 9 クロスベンダーレビュー(2026-07-08)の繰り延べMinor

Fable review-max + codex の T9 レビュー。Important/Critical なし。高価値Minor(ネットワーク依存テスト・fingerprint exit code・token_id表示・最小長順序)は T9 マージ時に反映済み。以下は強化パスで拾う。

- **D-7 (Fable Minor2)**: gatewayctl passphrase reset の argon2 KDF(数百ms)が Immediate Tx 内で実行され、RESERVED ロック保持中に gateway の書き込みがブロックされる。稀な手動操作で実害小。→ core/ops に `reset_passphrase_with_hash`(UPSERT版、KDF をロック外で)を足し、CLI がロック外で hash してから呼ぶ。
- **D-8 (Fable Minor3)**: パスフレーズ最小長がバイト長(`len()<8`)判定で、3文字の日本語(9バイト)が通る。API(routes.rs)と CLI(passphrase.rs)は両面 `len()<8` で**一貫**しているため I-3 の一貫性要件は満たすが、文字数意味論としては弱い。→ 強化パスで `chars().count()` ベースへ統一するか、文言を「bytes」に正すか裁定。

## Task 10 への引き継ぎ(Fable Minor7)

- **spawn_api_task の data_dir**: fingerprint CLI コマンドは `{db_path 親}/tls/cert.pem` を前提とする。Task 10 で main が `spawn_api_task(..., data_dir=db_path.parent())` を渡し、gateway が実際に生成する cert.pem のパスと CLI の前提が一致することを必須確認する(不一致だと fingerprint コマンドが常に「未生成」を返す)。

## D-9〜D-15: 構造監査(2026-07-10、クロスベンダー+ユーザー裁定)の繰り延べ項目

監査の DO-NOW 分(耐久性 FULL 化・毒性回復・リネーム2件・core/supervision 分離・契約 rustdoc)は
96c34d5〜 で実施済み。以下は台帳送り(Minor または トリガー待ち)。

- **D-9**: `iotkit-polling-adapter-runtime/src/polling_loop.rs` の約1,490行のインラインテストを
  `polling_loop_test.rs` へ分離(house pattern。実コードは638行で責務違反なし=監査確定)。
- **D-10**: `core/ledger/src/store.rs`(実コード853行)のモジュール分割(device/series/sighting/
  event/meta、ルート re-export 維持)。挙動不変の内部整理。crate 分割はしない(トランザクション
  所有が壊れるため=監査裁定、正本の例外に記載)。
- **D-11**: `iotkit-gateway/src/epoch_start.rs` の ledger_events 生読みを
  `core/ledger::last_epoch_renewal()` ヘルパーへ(enqueue 側は既に core/publish 経由)。
- **D-12**: `rpi-local-adapter/Cargo.toml` が edition 2021(workspace は 2024)→ 統一。
- **D-13**: `iotkit-gateway/src/main.rs` のモジュールレベル `#[allow(dead_code)]`(publish_task)を
  項目単位へスコープ縮小(本当に死んだ項目を隠している)。
- **D-14**: `docs/install.md` + systemd unit + `iotkit.toml.example` + 初回パスフレーズ/fingerprint
  手順(設置者ペルソナの一枚紙)。**トリガー: 初の外部配布前**(Wave 1 出口条件)。
- **D-15**: typed StartupError(config/TLS エラーにファイルパスと次に確認すべきことを付与)。
  起動系を触る計画のついでに。Fable 監査はエラー文言を「現状 actionable」と判定済み=急がない。

### 昇格トリガー(最初の2件は正本 architecture.md の Deliberate exceptions と対。event_loop と OSS メタデータは台帳のみで管理)

- `core/retention` 新設 = retention の次機能(active back-pressure)着手時。
- `record.rs` → `core/publish` = D9 MQTT 出口バインディングが共有 materialization を要した時
  (docs/exit-contract.md:4-6 の実装参照の同時更新を忘れない)。
- BravePI `event_loop` 分割 = 世話サービサ移行が旧南向き経路を削除する時(それまでの分割は
  捨てられる労力=監査裁定・ユーザー承認済み)。
- LICENSE / CONTRIBUTING / SECURITY / 公開メタデータ = Wave 2(公開 OSS)入口。

## D-16〜D-18: Grok 総合レビュー(2026-07-09)トリアージの繰り延べ項目

第三ベンダー(Grok/xAI)による全体レビュー `docs/eval/grok-review-2026-07-09.md`(対象 a4ae911)を
2026-07-10 に実物照合でトリアージ(初回照合の誤り2件をクロスベンダーレビューが訂正済み)。

- 解消済み: A1(二重データ面の文書)・未使用 ReasonCode の明記は構造ラン(0916ded/f23b1b3)。
- 重複: I1/I4/I5/I7/I8/A4(polling・store)は既存の決定・台帳(D1軽量プロファイル・昇格トリガー・
  D-9/D-10)と重複。
- 不採用(設計どおり): I6 の検疫TTL自動active化は**実在する**(retention 駆動
  `expire_quarantined_devices`、`core/ledger/src/store.rs:744`、既定7日=`quarantine_ttl_days`
  で設定可)が、D5:198「時限自動失効+CLI解除のみ」・D1:83 の決定どおりの挙動。当初「機構なし」
  と誤記帳→両ベンダーレビューが訂正。
- 不採用(実装済み): I9 の `last_used_at` 書き込みは既に60秒間引き実装済み
  (`core/ops/src/auth.rs:20,258`。Grok 対象の a4ae911 時点でも存在)。
- C1(配布時セキュリティ既定)は計画6持ち込み 8 へ。

- **D-16 (Grok A5)**: ホットパスの stringly error が未裁定——collector の
  `Result<EnvelopeAck, String>`(`core/collector/src/actor.rs:164`)、`ToSqlConversionFailure`
  への橋渡し(`iotkit-gateway/src/publish_task.rs:200`・`api/auth_layer.rs:30-34`・retention.rs)。
  D-15(起動系 StartupError)はこれを覆わない。→ 強化パスで typed error(CollectorError 等)へ。
- **D-17 (Grok I2)**: archive target 未登録 / `archive_responsible=false` だと retention が
  floor-only になり custody 保証が成立しないが、現状その状態は health に現れない。ただし D8 の
  Standalone は上流任意なので target 不在は正当な構成——無条件 degraded にはしない。→ custody
  を期待する構成(Site-managed、または custody 前提と明示された設定)に限定して health に明示
  表示を追加し、D-14(docs/install.md)の初回チェックリストと対にする。
- **D-18 (Grok §1.5 + A4 残り)**: `iotkit-ingest-client::new_envelope` が空 values の item を
  黙って落とす仕様が doc コメントに無い(テスト `new_envelope_drops_empty_value_items` で意図は
  固定済み)→ doc コメント追記。あわせて `core/collector/src/actor.rs` のインラインテスト
  約840行の分離(実コードは約375行で責務違反なし=2026-07-10 実測。D-9 と同じ house pattern)。

## 計画6への持ち込み(構造監査+計画6メニュー検証 2026-07-10)

計画6(R2 入口)の brainstorming/spec は以下を Global Constraints へ全掃引すること。

1. **IngestPrincipal**: 認証送信者identityを `IngestRequest` にエンベロープと別載せ。dedup・subject
   認可・流量会計・監査はこちらを使う。`envelope.source` は診断メタデータ扱い、principal との
   不一致は reject+侵害シグナル監査(D11決定2、D1:198-201「実装はWave 1のHTTP ingress」)。
   **同時に D5決定1 の「トークン1:1 送信者は subject_hint 省略可」の解決経路を実装する**——
   現行コレクタは無条件必須(欠落=終端 UnknownSubject)で、1:1 トークンが登場する計画6までに
   契約どおりの省略解決が要る(2026-07-10 最終レビュー codex 指摘の裁定)。
2. **ack 契約の完成**: 拒否詳細に `field_path`(JSON pointer)+期待スキーマヒント追加、
   `ReasonCode::Internal` 削除(未使用・D1準拠の生成者なし=T4監査で文書化済み)。D1:90/93。
   ワイヤ適合テストと同時に。
3. **docs/ingest-contract.md** 正規文書(exit-contract.md の対)+ curl 3行体験を受け入れ基準に。
4. **入口は別crate**: `iotkit-ingest-http`(R2)。`gateway/src/api` は制御面専用——認証・レート
   制限境界が異なる(D1:142、R2台帳)。check-layers に INGRESS 分類を新設。
5. **鮮度ウィンドウ超の拒否**: D1:60-65 のユーザー裁定「外部送信者導入と同時」=計画6が該当。
6. **スコープ裁定済み(2026-07-10 メニュー検証)**: HTTP 先行は正本既決(D1:106)。MQTT ingest は
   別計画(先頭タスク=D11保留のMQTT絞りワイヤ表現の決定)。ペアリング窓経路は計画9へ(承認画面の
   突合が必要=D13)→計画6は無認証面ゼロ。流量クラス等の具体値は設定化した暫定値+実測確定は
   命名済み別計画(容量ベンチ・電源断リグも吸収)。絞りの執行は網リスナー限定(in-proc は有界
   チャネルが逆圧=D1:117)。token-bucket 採用は D11保留の解決として還流記録すること。
   アラーム基盤は未実装のため「騒がしく」=監査イベント+R12 水位+エピソード集約(R23はフック)。
7. **未決(計画6 brainstorming でユーザーに確認)**: R22 snapshot 秘密投入+暗号化コンテナを
   計画6に含めるか(計画5 spec §9の約束)vs 直後の小計画6.5に分割するか。
8. **配布時セキュリティ既定(Grok C1、2026-07-10 実物照合済み)**: API 既定が `enabled=true` +
   bind `0.0.0.0:8443`(`iotkit-gateway/src/config.rs:326-327`)。setup モード(パスフレーズ
   未設定)の間、同一 LAN の任意ホストから (a) `POST /api/v1/setup/passphrase` が認証外
   (`api/routes.rs:62`)のため**先にパスフレーズを設定した者が箱を掌握できる**(最強ベクター)、
   (b) Bearer なしで `SETUP_ALLOWED_OPS` の 2 op(`device.approve_sighting`=デバイス承認 /
   `registry.resolve_unknown_key`=測定キー解決、`core/ops/src/catalog.rs:8`)が実行可能
   (単一ターゲット限定・bulk 不可・private_source_guard はインターネット直公開のみ遮断)。
   R19 入口認証の設計と同じ議論で既定を裁定する: bind 既定 127.0.0.1 / setup 完了まで API
   閉鎖 / ワンタイム setup トークン等。採った既定は D-14(設置手順)の初回チェックリストと
   対にする。
