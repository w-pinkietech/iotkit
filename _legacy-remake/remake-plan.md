# iotkit-next Remake Plan

## 1. Architecture Strategy

**Modular Monolith**

- 1プロセスで全モジュールを動作、モジュール間は Rust crate 境界で分離
- ESP32 対応を見据え、no_std 互換のコア層を設ける
- 全操作は API 経由 (UI・CLI・外部連携は同一 API を叩く)

## 2. Tech Stack

### Backend

- **Language**: Rust (stable)
- **HTTP**: axum (tokio ベース、軽量)
- **Async**: tokio (RPi) / embassy (ESP32、将来)
- **Serial/HW**: serialport + rppal (GPIO/I2C)
- **MQTT Client**: rumqttc (TLS 対応)
- **Script Engine**: Lua (mlua crate) — トランスフォーム用
- **Logging**: tracing crate (構造化 JSON ログ)

### Frontend

- 組み込み Web: Rust サーバーから静的 HTML/JS 配信
- チャート: 軽量 JS (uPlot or Chart.js) — transform 後の値をリアルタイム + 履歴表示
- フレームワーク不使用 (Vanilla JS + fetch)
- API 経由でのみデータアクセス

### CLI

- 別バイナリ `iotkit-cli` crate
- AI agent friendly: JSON 入出力がデフォルト、`--human` で人間用表示
- stdin パイプ対応、非対話、冪等
- 構造化 exit code (0=成功, 1=一般エラー, 2=バリデーション, 3=接続, 4=認証, 5=未発見)
- stderr にログ/進捗、stdout にデータ
- 全 API 操作に対応するサブコマンド

### Database

- **SQLite** (rusqlite) — 設定・マスタ・時系列・監査すべて統合
- **WAL モード** で読み書き並行処理
- **Repository trait** で永続化を抽象化 (RPi=SQLite / ESP32=NVS)
- **InfluxDB・MariaDB・Docker 廃止**

### MQTT

- **内蔵ブローカー廃止** — 外部ブローカーにクライアント接続のみ
- **TLS ON/OFF 可能** — テスト時は平文、本番は暗号化
- **mTLS 対応** (将来版) — Transport 層全体で相互認証
- **接続設定はファイルベース** (TOML) — DB ではなく設定ファイルで管理

## 3. DB Schema Redesign

### テーブル増殖問題の解決

レガシーではアクセスタイプごとに `*_device_configs` テーブル (6+)、センサー種別ごとに
拡張テーブル (`gpio_inputs`, `gpio_outputs`, `temperatures`, `adcs`) が増殖していた。

**Remake**: JSON 設定カラム + Rust enum バリデーションで統合。
新しいアクセスタイプやセンサー種別を追加してもテーブルは増えない。

### トランスポート + プロトコル 2層モデル

レガシーでは物理トランスポートと論理プロトコルが混同されていた (BLE = BravePI 固定等)。

**Remake**: 明確に分離。

- `transport`: enum (serial, i2c, gpio, ble, tcp, udp, ...)
  - `transport_config`: JSON — {port, baud, address, pin, ...}
- `protocol`: enum (bravepi, bravejig, modbus_rtu, modbus_tcp, mqtt, http, raw, ...)
  - `protocol_config`: JSON — {device_number, unit_id, register_map, ...}

Rust 側:
- `trait Transport { send/recv }` — 物理層
- `trait Protocol { encode_command / decode_response }` — 論理層
- 新プロトコル追加 = trait 実装のみ、テーブル変更不要
- 同じプロトコルが複数トランスポート上で動作可能 (例: Modbus RTU on Serial, Modbus TCP on TCP)

### デバイス識別 3層モデル

レガシーではデバイスIDの形式がテーブルごとにバラバラだった。

**Remake**: 3層で統一。

- `system_id`: UUID v7 — PK、自動生成、不変。全デバイスが必ず持つ
- `hardware_id`: TEXT NULL — `"{transport}:{固有部分}"` の正規化文字列
  - 例: `"ble:AA:BB:CC:DD:EE:FF"`, `"i2c:1:0x48"`, `"gpio:18"`
  - 物理IDがないデバイスは NULL (手動バインド)
- `user_label`: TEXT NULL — 人間用の名前 ("1F北側 温度センサー")

再接続時: hardware_id マッチ → ポート推定 → ユーザー確認 の順で解決。

### BravePI/BraveJIG の疎結合化

レガシーでは BravePI/BraveJIG のプロトコル詳細がコア全体に浸透していた (Design Defect D3-2)。

**Remake**: コアシステムは Transport/Protocol trait のみを知る。
BravePI/BraveJIG は独立したアダプター crate として実装。
コアのコード・型定義・DB スキーマに BravePI/BraveJIG 固有の記述を含めない。

```
コアシステム (BravePI/BraveJIG を知らない)
  │ trait 境界
  ▼
アダプター層 (独立 crate)
  ├── bravepi_adapter: impl Transport + Protocol
  └── bravejig_adapter: impl Transport + Protocol
```

初回リリースで BravePI/BraveJIG アダプターは提供するが、コアとは完全分離。

### その他のスキーマ改善

- FK と CHECK 制約の厳格化
- `sensor_gpio_output_pivots` → `sensors(sensor_id)` に修正 (レガシーバグ)
- ソフトデリート (`deleted_at`) + 監査タイムスタンプ (`created_at`, `updated_at`)
- マイグレーションシステム導入
- シードデータのバージョン管理、カスタムセンサータイプの ID 範囲分離
- `extra_mqtt` JSON カラム廃止 → 通知ルーティング正規化
- 命名規則の統一 (snake_case、`device_number` と `device_id` の明確な区別)

## 4. Transform Registry

### 概要

センサーデータは MQTT 送信前にトランスフォーム (変換処理) を通す。
センサータイプごとにデフォルト変換が登録されており、カスタム変換はスクリプトを追加登録する。

### アーキテクチャ

```
センサーデータ受信
  → Transform レジストリ適用
    → 変換後データ → MQTT パブリッシュ
    → 変換後データ → SQLite 時系列保存
    → 変換後データ → UI リアルタイムチャート表示
```

### スクリプト管理

```
transforms/
  ├── builtin/           # 組み込み (出荷時同梱)
  │   ├── thermocouple.lua
  │   ├── adc_linear.lua
  │   ├── gpio_debounce.lua
  │   ├── acceleration.lua
  │   └── passthrough.lua
  └── custom/            # ユーザー追加
      └── (ユーザーが登録)
```

### 適用ルール

```toml
# iotkit-next.toml
[transforms.registry]
thermocouple = "builtin/thermocouple"
adc          = "builtin/adc_linear"
gpio_input   = "builtin/gpio_debounce"
acceleration = "builtin/acceleration"
illuminance  = "builtin/passthrough"

[transforms.overrides]
"adc-press-03" = "custom/my_pressure"   # デバイス固有
"thermo-*"     = "custom/factory_format" # ワイルドカード
```

適用順序:
1. デバイス固有のオーバーライド → あればそれを使う
2. センサータイプのデフォルト → あればそれを使う
3. どちらもなければ passthrough → 生データそのまま

### スクリプト言語: Lua

- 軽量 (100KB ランタイム)
- RPi4B でも ESP32 でも動作
- サンドボックスが容易
- IoT/ゲーム業界で実績豊富

## 5. Logging

### 4層ログ設計

| ログ種類 | 保存先 | デフォルト | 保持期間 | フォーマット |
|---------|--------|----------|---------|------------|
| システムログ | stdout + ファイル | ON (INFO) | 30日 | JSON 構造化 |
| センサーデータ | SQLite 時系列 | ON | 90日 | API + CLI |
| 操作監査 | SQLite audit_log | ON | 365日 | API + CLI |
| 通信ログ | ファイル | OFF | 7日 | CLI で動的 ON/OFF |

### システムログ

- tracing crate (構造化 JSON)
- 出力: stdout (systemd journal) + ファイルローテーション
- レベル: ERROR / WARN / INFO / DEBUG / TRACE
- モジュール単位でレベル変更可能

### センサーデータログ

- transform 後の値を SQLite 時系列テーブルに保存
- 保持期間は設定ファイルで指定 (デバイスごとにオーバーライド可能)
- 自動パージ

### 操作監査ログ

- SQLite `audit_log` テーブル (append-only)
- 全書き込み API を記録、読み取り系は設定で ON/OFF
- フィールド: timestamp, source, actor, action, target, detail (JSON), result

### 通信ログ

- デフォルト OFF
- CLI で動的に ON/OFF (`iotkit debug transport on --device ... --duration 30m`)
- デバイス単位・トランスポート単位で絞り込み可能
- レベル: raw (全バイト) / frames (パース済み) / summary

## 6. Reimplementation Scope

全 11 モジュール実装。

| Module | Scope |
|--------|-------|
| core-domain | デバイスモデル (2層 transport/protocol, 3層 ID)、projection 型、Transform trait |
| api-service | 全操作対応 HTTP API。UI・CLI・外部連携の統一エントリポイント |
| sensor-ingest | I2C/GPIO ポーリング、データ正規化、Transform レジストリ適用 |
| provider-adapter | trait Transport + Protocol の実装群。bravepi/bravejig は独立 crate |
| device-command-orchestrator | コマンドライフサイクル (busy/timeout/retry/ACK) |
| device-config-service | CRUD + Repository trait + read-model rebuild |
| timeseries-service | SQLite 時系列テーブル + クエリ集計 + データ保持/パージ |
| notification-service | MQTT パブリッシュ + email 通知 (外部ブローカー接続) |
| ui-web | 全画面 (Vanilla JS)、transform 後の値でリアルタイム + 履歴チャート |
| ops-service | 時刻同期、再起動、カメラ、ストレージ |
| deployment | SQLite スキーマ init + migration + systemd unit |
| iotkit-cli | AI agent friendly CLI。全 API 操作対応、JSON デフォルト |

## 7. Quality Criteria

- テストカバレッジ: 80%+ 全体
  - core-domain, provider-adapter, device-config-service: 90%+
  - ui-web, ops-service: 70%+ 許容
- レガシー等価テスト: HTTP API / プロトコル互換性テスト
- CI: cargo test + clippy + fmt
- パフォーマンス: RPi4B 起動 5秒以内、メモリ 100MB 以下

## 8. Migration Strategy

- **ビッグバン切替** — 全モジュール完成後に一括切替
- MariaDB → SQLite 変換スクリプト (スキーマ再設計に対応)
- InfluxDB → SQLite 時系列テーブルへのデータ移行
- 切替手順書を deployment モジュールに含める

## 9. Implementation Order

1. **WS1**: core-domain — 基盤型定義 (2層/3層モデル、Transform trait)
2. **WS2**: device-config-service + deployment — 永続化層、スキーマ、マイグレーション
3. **WS3**: provider-adapter + sensor-ingest — Transport/Protocol trait 実装、BravePI/BraveJIG アダプター
4. **WS4**: device-command-orchestrator — コマンドライフサイクル
5. **WS5**: timeseries-service + notification-service — 時系列保存、MQTT/email 通知
6. **WS6**: api-service — 全操作対応 HTTP API
7. **WS7**: ui-web — フロントエンド (transform 後チャート含む)
8. **WS8**: ops-service + iotkit-cli — 運用機能 + CLI ツール

## 10. ESP32 Portability Strategy

- core-domain, provider-adapter は no_std 互換で設計
- Repository trait で永続化抽象化 (SQLite / NVS)
- Transport/Protocol trait は ESP32 でもそのまま使用
- Transform (Lua) は ESP32 でも動作可能
- axum → embassy-net への HTTP 抽象化
- tokio 依存は RPi 専用クレートに隔離

## 11. Directory Structure

**レイヤー分離型 Cargo workspace**

```
iotkit-next/
├── Cargo.toml              # workspace root
├── core/
│   └── core-domain/        # エンティティ層: ドメインモデル、trait 定義
├── adapters/
│   ├── provider-adapter/   # Transport/Protocol trait の共通ユーティリティ
│   ├── bravepi-adapter/    # BravePI 固有の impl (独立 crate)
│   └── bravejig-adapter/   # BraveJIG 固有の impl (独立 crate)
├── services/
│   ├── api-service/        # HTTP ハンドラー、ルーティング
│   ├── sensor-ingest/      # センサーデータ収集・正規化
│   ├── device-config/      # デバイス CRUD、read-model
│   ├── device-command/     # コマンドライフサイクル
│   ├── timeseries/         # 時系列保存・クエリ
│   ├── notification/       # MQTT/email 通知
│   └── ops-service/        # システム管理
├── apps/
│   ├── iotkit-server/      # バイナリ crate (main): DI で全層を結合
│   └── iotkit-cli/         # CLI バイナリ crate
├── ui/
│   └── web/                # 静的 HTML/JS/CSS
├── config/                 # デフォルト設定ファイル (iotkit-next.toml)
├── transforms/             # Lua スクリプト (builtin/ + custom/)
├── migrations/             # DB マイグレーション
└── _legacy-remake/         # ハーネスナレッジベース
```

### crate 内ファイル構成 (標準)

```
crates/<name>/
├── Cargo.toml
└── src/
    ├── lib.rs              # crate ルート (pub mod 宣言)
    ├── domain.rs           # ドメインモデル (エンティティ、値オブジェクト)
    ├── port.rs             # trait 定義 (Repository, 外部サービス境界)
    ├── service.rs          # ユースケース実装
    ├── error.rs            # crate 固有エラー型 (thiserror)
    └── tests/              # 結合テスト
        └── *.rs
```

mod.rs は使用しない (ファイル名 = モジュール名)。

## 12. Clean Architecture

### 4層分離

```
┌─────────────────────────────────────────────┐
│  apps/ (フレームワーク・ドライバー層)           │
│  axum, clap, rumqttc, serialport, rusqlite   │
├─────────────────────────────────────────────┤
│  adapters/ (インターフェースアダプター層)        │
│  bravepi/bravejig impl, SQLite Repository    │
├─────────────────────────────────────────────┤
│  services/ (ユースケース層)                    │
│  アプリケーション固有のビジネスルール            │
├─────────────────────────────────────────────┤
│  core/ (エンティティ層)                        │
│  ドメインモデル, trait 定義, 外部依存ゼロ       │
└─────────────────────────────────────────────┘
```

### 依存方向ルール (Cargo.toml で強制)

- `core/` は `std` 以外の外部 crate に依存しない (`serde` のみ例外許容)
- `services/` は `core/` の trait にのみ依存。`adapters/` や `apps/` を import しない
- `adapters/` は `core/` の trait を実装。`services/` を import しない
- `apps/` が DI コンテナとして全層を結合
- 層をまたぐ逆方向の依存は **コンパイルエラー** で検出

## 13. Coding Conventions

### Rust 標準 (rustfmt / clippy が強制)

| 対象 | スタイル | 例 |
|------|---------|-----|
| 型名 | PascalCase | `Device`, `SensorReading` |
| trait 名 | PascalCase | `DeviceRepository`, `Transport` |
| 関数/メソッド | snake_case | `find_by_id`, `encode_command` |
| 変数 | snake_case | `device_name`, `sensor_id` |
| 定数 | SCREAMING_SNAKE_CASE | `MAX_RETRY_COUNT`, `DEFAULT_TIMEOUT_MS` |
| モジュール | snake_case | `device_config`, `sensor_ingest` |
| crate 名 | kebab-case | `core-domain`, `bravepi-adapter` |
| enum variant | PascalCase | `AccessType::Ble`, `Protocol::ModbusRtu` |

### ドメイン用語の統一 (レガシーの混乱を引き継がない)

| レガシー | Remake | 意味 |
|---------|--------|------|
| `device_number` | `hardware_id` | 物理ID |
| `device_id` | `system_id` | システム内部ID (UUID) |
| `device_name` | `user_label` | 人間用ラベル |
| `access_type` | `transport` + `protocol` | 2層分離 |
| `hysteresis_*` | `threshold_*` | 閾値 |

### 型の接尾辞ルール

| 接尾辞 | 用途 |
|--------|------|
| `*Repository` | trait のみ (永続化境界) |
| `*Service` | ユースケース層のサービス型 |
| `*Adapter` | 外部システムとの接続実装 |
| `*Handler` | HTTP ハンドラー |
| `*Command` | 書き込み操作の入力 DTO |
| `*Query` | 読み取り操作の入力 DTO |
| `*Response` | API レスポンス DTO |
| `*Error` | エラー型 (thiserror) |
| `*Config` | 設定構造体 |

### エラーハンドリング

- ライブラリ crate: `thiserror` で型付きエラー
- アプリケーション層: `anyhow` でエラー集約

### テスト命名

```rust
#[test]
fn should_create_device_with_valid_config() { }
fn should_reject_duplicate_hardware_id() { }
fn should_timeout_after_10_seconds() { }
// should_<期待動作>_<条件> のパターン
```

### 日本語の扱い

- コード上は英語のみ
- 日本語はドキュメント・コメント・glossary.md でのみ使用
- 対照表を glossary.md に維持

## 14. Test Strategy

### 3層テスト

| 層 | 対象 | ツール | 実行タイミング |
|----|------|--------|--------------|
| ユニットテスト | 個別関数・メソッド、ドメインロジック | `#[cfg(test)]` + mock | `cargo test` (常時) |
| 結合テスト | Repository + SQLite 実物、サービス層 | `tests/` ディレクトリ、インメモリ SQLite | `cargo test` (常時) |
| E2E テスト | API エンドポイント、CLI コマンド | reqwest + テストサーバー起動 | CI / 手動 |

### テストルール

- ユニットテスト: 外部依存を mock。core/ のテストは I/O なし
- 結合テスト: SQLite 実物 (`:memory:`) を使用。DB mock は禁止
- E2E テスト: 実際の HTTP リクエストで API を検証
- テストデータ: 各テストが自前で作成、共有状態なし
- カバレッジ目標: core 90%+、services 80%+、adapters 80%+、ui/ops 70%+

### ドキュメント

- pub API には doc comment 必須
- OpenAPI spec を axum から自動生成 (utoipa crate)
- `cargo doc --no-deps` がクリーンにビルドされること

## 15. Configuration Strategy

- **設定ファイル**: TOML 形式 (`iotkit-next.toml`)
- **管理対象**: MQTT 接続、TLS 設定、ログレベル、Transform レジストリ、データ保持期間
- **DB には入れないもの**: インフラ接続情報、暗号化設定、ログ設定
- **DB に入れるもの**: デバイス登録、センサー設定、通知ルーティング
- **TLS 全般**: ON/OFF 切替可能 (テスト時は平文、本番は暗号化)
- **将来**: mTLS (相互認証) を Transport 層全体で対応
