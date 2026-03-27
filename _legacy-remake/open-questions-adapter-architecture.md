# Adapter アーキテクチャ — 未決事項と議論メモ

## 決まったこと

### 3層分離モデル（改訂）

```
Core（ドメイン、no_std）
  ↑ SensorReading（統一出力）
Sensor Driver（sensor_type ごとに1つ、入力ソースを抽象化）
  ↑ 物理値
Transport / ProtocolCodec（通信経路ごとの生データ取得）
```

### 核心的な設計方針：センサーは「どこにいても同じ」

同じセンサーIC の値は、通信経路が異なっても同じ SensorDriver が処理する。

```
GPIO pin 18 ──────→┐
I2C /dev/i2c-1 ───→├──→ SensorDriver(261) ──→ SensorReading { 22.5℃ }
UART (BravePI) ───→┤
将来: BraveJIG ───→┤
将来: Modbus ─────→┘
```

PoC で確認した実例：MCP9600（熱電対 sensor_type=261）
- I2C 直結: レジスタ読み → Int16 BE × 0.0625 → ℃
- UART (BravePI): メインボードが変換済み → Float32LE → ℃
- 入口は違うが出口は同じ SensorReading

### Driver の2つの責務

**1. Transport / Codec（通信経路ごと、ステートレス）:**
- ポートを開く、バイトを読む/書く（OS依存）
- バイト列 ↔ メッセージの変換（no_std 可能）

**2. Sensor Driver（sensor_type ごと、入力ソースを吸収）:**
- 生データ → 物理値への変換（センサーIC 固有の計算）
- 複数の入力ソース（I2C, UART, GPIO 等）に対応
- 出力は統一された SensorReading

| センサーIC | sensor_type | I2C での生データ | UART での生データ | 共通出力 |
|---|---|---|---|---|
| OPT3001 | 264 | 指数+仮数 → Lux | Float32LE (Lux) | SensorReading { Lux } |
| MCP9600 | 261 | Int16 BE × 0.0625 | Float32LE (℃) | SensorReading { ℃ } |
| MCP3427 | 259 | MCP342x lib → V | Int16LE × 2ch (mV) | SensorReading { mV } |
| VL53L1X | 260 | qwiic lib → mm | UInt16LE (mm) | SensorReading { mm } |
| SDP810 | 263 | raw/scale → Pa | Float32LE (Pa) | SensorReading { Pa } |
| LIS2DUXS12 | 262 | Int16LE × 3 × 0.244 | Float32LE × 3 | SensorReading { mG } |
| 接点入出力 | 257/258 | GPIO ピン状態 | 1byte ON/OFF | SensorReading { bool } |

### RPi に直接つながっているデバイスの整理

BravePI メインボードは BLE 中継器だが、RPi から見ると UART デバイス。
RPi は BLE のことを一切知らない。

```
RPi のローカルハードウェア
├── UART (/dev/ttyAMA0) ← BravePI メインボード（BLE の先のリモートセンサー）
├── I2C (/dev/i2c-1)    ← ボード上の直結センサー（同じセンサー IC）
└── GPIO (BCM pins)      ← 接点入出力
```

### Adapter の責務（状態を持つ）

- Driver を使ってハードウェアと通信
- デバイス発見、セッション維持（keep-alive）
- ペアリング、DFU、スキャンモードなどの管理操作（ステートマシン）
- 自前の SQLite DB を持つ（core.db とは分離）
- 管理用 API を提供
- core には SensorValue / DeviceInfo だけを渡す

### Adapter = ミニアプリケーション

- adapter は自分自身の中に完結した世界を持つ
  - 自分のドメインロジック
  - 自分の DB
  - 自分の API
  - 自分の UI 用データ
- core から見ると adapter はブラックボックス
- adapter から見ると core はセンサー値の受け口

### DB レベルの分離

```
data/
├── core.db       ← デバイス、センサー値、閾値、通知
├── bravepi.db    ← ペアリング、通信状態、DFU履歴
└── bravejig.db   ← ルーター、モジュール、ペアリング
```

adapter → core: DeviceDiscovered, SensorValue
core は adapter 固有の情報を一切持たない

## 未決事項

### 1. Adapter の UI をどうするか

議論の経緯:
- core が adapter の画面を描画する → core が adapter を知ることになるので NG
- adapter が独立ポートを持つ → ユーザー（設備管理者）にポートを覚えさせるのは NG
- adapter が core に route を登録する → 結局 BravePI/BraveJIG 専用になる恐れ

現時点の方向性:
- core の `/adapters` ページに adapter 一覧を表示
- 各 adapter の中身は adapter の API を叩いて描画
- UI は同居（1つの Web アプリ）、内部的に剥がせる設計

未決:
- adapter の API からどういう形式で UI データを返すか（JSON? HTML片? 宣言的スキーマ?）
- シンプルな adapter（Modbus 等）と複雑な adapter（BraveJIG の DFU 等）で UI の提供方法を分けるか
- フロントエンド側で adapter の画面をどうレンダリングするか

### 2. Base Adapter の設計

議論の経緯:
- adapter を作るたびに DB 初期化、設定読み込み、API ルーター、core 接続を毎回書くのは無駄
- base adapter が共通基盤を提供すれば、新しい adapter は固有ロジックだけ書けばいい

未決:
- base adapter が提供する具体的な機能の範囲
- base adapter は crate として提供？trait として提供？struct として提供？
- マイグレーション管理はどうするか

### 2.1 2026-03-27 メモ: Driver Base / Adapter Base の切り分け

現状の理解:

- `rpi4b-driver/transport` は protocol 非依存の I/O 層としてかなり分離できている
- ただし `driver base` と呼べる共通 trait / factory / session 抽象はまだ薄い
- `bravepi-adapter` は core との channel 契約 (`AdapterEvent` / `AdapterCommand`) は共有できている
- ただし `adapter base` と呼べる共通 runtime / DB / API / registration 基盤はまだ無い

要するに:

- driver は「責務分離はできているが、base API はまだ小さい」
- adapter は「boundary はあるが、base framework はまだ無い」

#### Driver Base に将来入りそうな責務

- resource lifecycle (`open / close / reopen / enumerate`)
- 共通 I/O 契約 (`read / write / write_all / timeout`)
- 共通 config と validation
- 共通 error taxonomy (`missing / timeout / disconnected / permission denied`)
- mock / replay / fake device を差し込むためのテスト seam

Driver Base に入れない方がよいもの:

- protocol codec
- sensor decode
- device discovery
- adapter 側の retry policy や lifecycle state

#### Adapter Base に将来入りそうな責務

- adapter task の起動 / 停止 / supervision
- 設定読込、instance registration、core との接続
- adapter 自前 DB の bootstrap / migration / connection 管理
- health / status / metrics / debug dump の共通 surface
- 管理 API / UI が必要な adapter のための共通 runtime

Adapter Base に入れない方がよいもの:

- BravePI / BraveJIG 固有の protocol 変換
- sensor registry や downlink encode
- pairing / DFU / scan の具体 state machine
- command busy / timeout / retry / ACK の業務ルール

#### 抽出タイミングの目安

先回りして大きい base を作るのではなく、「2つ目の concrete 実装で重複が見えた時に抜く」を原則にする。

具体的には:

1. 今の段階では BravePI をもう少し concrete に進める
2. 2つ目の adapter (`BraveJIG` や直結 I2C/GPIO path) を作る
3. 起動 / 停止 / 設定 / DB / API / 監視の重複が見えたら `adapter base` を抽出する
4. 同じ上位ロジックが serial 以外の transport (`USB`, `TCP`, `I2C` など) を同じ形で扱いたくなったら `driver base` を trait 化する

補足:

- `busy / timeout / retry / ACK` は `adapter base` に混ぜず、必要なら `device-command-orchestrator` のような上位層として分離する
- `core/types` の境界はすでに共通 contract として機能しているので、今すぐ base 化すべき最優先はそこではない

### 3. Core 側でデバイス削除したとき adapter 側はどうなるか

選択肢:
- a) core がイベントを adapter に通知 → adapter が自分の DB からも消す
- b) core で消しても adapter のペアリング情報は残る（再登録で復活）
- c) 削除は adapter 側からのみ可能にする（core はデバイスの無効化だけ）

### 4. Adapter の登録・発見メカニズム

- core は起動時にどうやって adapter を発見するか
- adapter の追加・削除は再起動が必要か、動的か
- 設定ファイルで adapter を列挙するか、自動検出か

### 5. Adapter 間通信の必要性

- BraveJIG のルーターが複数のモジュールタイプを扱う場合、adapter 間で連携が必要か
- 基本は「adapter 間は独立」で良いはずだが、エッジケースがあるか

## 背景メモ

### レガシーでの Node-RED の役割

Node-RED が担っていたのは本質的には adapter 層のフレームワークだった:
- シリアル通信ノード → transport
- MQTT ノード（port 51883）→ BraveJIG 内部制御メッセージング
- UI テンプレート → adapter 固有の管理画面
- フロー → adapter 内のロジック

レガシーの問題は、Node-RED が adapter だけでなく core まで全部やっていたこと。

### remake で内蔵 MQTT ブローカー（port 51883）は廃止

BraveJIG LAN ルーターとの通信方式は adapter 内で解決する必要がある。
これも未決事項の一つ。

## 参考: Crate 構成案（改訂）

```
crates/
├── iotkit-core/              # ドメインモデル + SensorReading 型 (no_std)
│
├── transport-serial/         # UART Transport
├── transport-i2c/            # I2C Transport
├── transport-gpio/           # GPIO Transport
│
├── codec-bravepi/            # BravePI UART フレーム codec (no_std 可能)
├── codec-bravejig/           # BraveJIG codec (no_std 可能)
├── codec-modbus/             # Modbus codec (将来)
│
├── sensor-opt3001/           # 照度 — from_i2c() / from_uart_frame()
├── sensor-mcp9600/           # 熱電対 — from_i2c() / from_uart_frame()
├── sensor-mcp3427/           # ADC — from_i2c() / from_uart_frame()
├── sensor-vl53l1x/           # 測距 — from_i2c() / from_uart_frame()
├── sensor-sdp810/            # 差圧 — from_i2c() / from_uart_frame()
├── sensor-lis2duxs12/        # 加速度 — from_i2c() / from_uart_frame()
├── sensor-contact/           # 接点入出力 — from_gpio() / from_uart_frame()
│
├── adapter-bravepi/          # BravePI adapter (管理: ペアリング, DFU, スキャン)
├── adapter-bravejig/         # BraveJIG adapter (管理: ルーター, DFU)
│
└── iotkit-gateway/           # Gateway アプリケーション

apps/
└── iotkit-rpi/               # RPi4B 向けバイナリ（adapter群を組み合わせ）
```

ポイント: sensor-* crate が入力ソースを吸収する。
同じ sensor_type なら I2C でも UART でも同じ crate が処理する。
