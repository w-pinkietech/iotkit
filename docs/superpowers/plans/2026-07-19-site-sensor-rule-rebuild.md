# Site sensor rule rebuild implementation plan

**Goal:** 旧IoTKitで現場が利用していた立ち上がり・立ち下がり・個別debounce・反転・累積値リセットを、現在の責務分離を保ったSiteセンサー判定として実装する。

**Architecture:** revisioned semantic definitionの`spec_json`は、future-only契約を保つため引き続き正本とする。単一threshold/hysteresisを、明示的な高側/低側しきい値、上昇/下降debounce、有効側へ置き換える。判定途中は`semantic_definition_state_v2`へ永続化し、評価器・preview・projectorで同じ実装を使う。外部出力と製品固有Adapter設定はsemantic DBへ混ぜない。

**Tech stack:** Go, SQLite, `net/http`, server-rendered HTML, vanilla JavaScript.

---

1. `internal/semantics`へ新しいdetector spec、validation、時刻付き評価テストを追加し、旧condition JSONの読込互換を持たせる。
2. DB migrationでpending transition stateを追加し、projectorが受信時刻と状態を永続化するテストを追加する。
3. `SemanticService`へ累積値リセット操作を追加し、store・監査・API・Console POSTをテスト先行で通す。
4. previewを新detectorへ移し、上昇/下降しきい値とdebounceをグラフ・試算へ反映する。
5. Console formをHigh/Low有効、立ち上がり/立ち下がり、各確定待ち時間へ変更し、接点ではしきい値を隠す。
6. package test、migration test、HTTP scenario testを実行し、実画面で熱電対・照度・接点入力を確認する。
