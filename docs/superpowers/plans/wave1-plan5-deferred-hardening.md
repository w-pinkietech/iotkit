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
