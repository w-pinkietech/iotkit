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
