# Sub-project B: Device Lifecycle — 設計 Spec

## 目的

DeviceDiscovered の契約を閉じる。全デバイスで DeviceDiscovered を出し、
DeviceKey を logical sensor endpoint に統一する。
DeviceLost は型だけ残し、発行ロジックは後続に defer する。

これにより:
- core/UI が全デバイスを inventory に載せられる
- DeviceKey の意味が「transmitter」から「logical sensor endpoint」に統一される
- contact 系デバイスも DeviceDiscovered を出せるようになる
- 将来の timeout-based DeviceLost を自然に差し込める構造になる

## 設計判断

### DeviceDiscovered の定義

「adapter が stable な device_key を観測し、inventory に載せるための最小限の
identity を確定できた」ときに発行するイベント。

- 全デバイスで出す
- `SensorIdentity` は必須 (Optional にしない)
- contact 系は IC レベルでなく module-level の identity を返す
- Config frame は discovery の前提ではなく、将来の enrich source

### DeviceKey の composite key 化

BravePI の DeviceKey は常に logical sensor endpoint を指す。

```
bravepi:{transmitter_id}:{suffix}
```

例:
- `bravepi:246880020140018b:temperature`
- `bravepi:246880020140018b:contact_input`
- `bravepi:246880020140018b:contact_output`

physical transmitter と logical device は分けて考える。
同一 transmitter でも sensor_type が異なれば別 logical device。
raw の transmitter ID は `identity.connection.parameters["transmitter_id"]` に残る。

core は key の文字列を parse しない。意味づけは SensorIdentity と connection で持つ。

### suffix の生成

suffix は adapter-local の明示 mapping で管理する。
`SensorType` の Display や core 側のメソッドには依存しない。

```rust
// bravepi-adapter/src/task/convert.rs (private helper)

fn device_key_suffix(sensor_type: &SensorType) -> Option<&'static str> {
    match sensor_type {
        SensorType::ContactInput => Some("contact_input"),
        SensorType::ContactOutput => Some("contact_output"),
        SensorType::Adc => Some("adc"),
        SensorType::Ranging => Some("ranging"),
        SensorType::Temperature => Some("temperature"),
        SensorType::Acceleration => Some("acceleration"),
        SensorType::DifferentialPressure => Some("differential_pressure"),
        SensorType::Illuminance => Some("illuminance"),
        SensorType::Unknown(_) => None,
    }
}
```

`Unknown(_)` は suffix を返せないため、`frame_to_event()` は早期 return する
(現状の挙動と同じ)。

### contact 系の module-level identity

ContactInput / ContactOutput は IC ベースの identity を持たないが、
module-level identity を返す。

```rust
// bravepi-adapter/src/task/convert.rs (private helper)

fn contact_identity(
    sensor_type: &SensorType,
    conn_info: ConnectionInfo,
) -> SensorIdentity {
    SensorIdentity {
        manufacturer: "Braveridge".to_string(),
        ic_part_number: match sensor_type {
            SensorType::ContactInput => "Contact Input Module".to_string(),
            SensorType::ContactOutput => "Contact Output Module".to_string(),
            _ => unreachable!("contact_identity called with non-contact sensor type"),
        },
        sensor_type: sensor_type.clone(),
        connection: conn_info,
    }
}
```

`ic_part_number` は実 IC ではなく module/model 名を入れる前提。
フィールド名の変更は将来の横断リファクタに defer する。

### DeviceLost

DeviceLost は「過去に DeviceDiscovered を出した device_key に対してのみ出す」
と定義する。ただし Sub-project B では発行ロジックを実装しない。

発行しない理由:
- timeout-based の判定にはデバイスごとの uplink_interval が必要
- uplink_interval は Config frame から取得するが、Config の活用はスコープ外
- 暫定ルールを event_loop に固定化すると後で外しにくい
- transport terminal error は AdapterError で表すのが自然で、DeviceLost とは混ぜない

### DeviceState

`event_loop.rs` の `HashSet<DeviceKey>` を `HashMap<DeviceKey, DeviceState>` に
置き換える。

```rust
struct DeviceState {
    last_seen: tokio::time::Instant,
}
```

- 新規 key + identity あり → DeviceDiscovered 送信 → insert
- 新規 key + identity なし → warn、insert しない、SensorData も送信しない (early return)。
  DeviceDiscovered 未発行のデバイスから SensorData を流すのは契約違反。
  Sub-project B 後の BravePI では Unknown は frame_to_event() で早期 return、
  それ以外は全て identity=Some なので、この分岐は実質到達不能だが防御として残す。
- 既知 key → last_seen 更新のみ

insert は DeviceDiscovered 成立後に行う。
「map に key がある = DeviceDiscovered 済み」という不変条件を保つ。

`last_seen` は今回使わないが、DeviceState の最小限のフィールドとして先に持つ。
後続で timeout-based lost を入れる際に構造変更なしで差し込める。

`tokio::time::Instant` を使う理由: 将来 timeout テストで
`tokio::time::pause` / `tokio::time::advance` が使える。

## ファイル構成と変更範囲

### 変更

- `bravepi-adapter/src/task/convert.rs`
  - `device_key_suffix()` private helper 追加
  - `contact_identity()` private helper 追加
  - `frame_to_event()` の device_key 生成を composite key に変更
  - ContactInput/ContactOutput が `Some(identity)` を返すよう変更
  - DecodeError の device_key も composite key に揃える (suffix が引ける場合)
- `bravepi-adapter/src/task/event_loop.rs`
  - `HashSet<DeviceKey>` → `HashMap<DeviceKey, DeviceState>`
  - `DeviceState { last_seen: tokio::time::Instant }` 追加
  - insert 順序を DeviceDiscovered 成立後に変更
  - identity=None の新規デバイスは warn して insert しない、SensorData も送信しない (early return)

### テスト変更

- `bravepi-adapter/src/task/convert_test.rs`
  - 全テストの `device_key.as_str()` アサーションを composite key に更新
  - `contact_input_has_no_identity` → identity が Some であることをテスト (反転)
  - contact 系 identity の manufacturer, ic_part_number, sensor_type, connection をアサーション
  - DecodeError の composite key テスト追加 (既存の "unknown" → device_key: None ケースも維持)
- `bravepi-adapter/src/task/event_loop_test.rs`
  - `device_key.as_str()` アサーションを composite key に更新
  - contact 系デバイスの DeviceDiscovered が出ることを確認するケース追加
  - **同一 transmitter で sensor_type 違い = 別 logical device テスト追加**: 同じ transmitter_id で Temperature と ContactInput を流し、2 つの別 key で DeviceDiscovered が 2 回出ること、以降は再 discover されないことを確認

### 変更なし

- `core/types/src/lib.rs` — AdapterEvent, SensorIdentity, DeviceKey は変更なし
- `bravepi-adapter/codec/` 全体
- `bravepi-adapter/sensors/` 全体
- `bravepi-adapter/src/transport.rs`
- `bravepi-adapter/src/task/serial_source.rs`
- `bravepi-adapter/src/task/handle.rs`

### スコープ外

- DeviceLost の発行ロジック (timeout-based) — 後続 sub-project
- Config frame の活用 (uplink_interval の取得) — 後続 sub-project
- `ic_part_number` フィールドの名前変更 — 将来の横断リファクタ
- DeviceState への identity 保持 — DeviceLost 発行時に必要になったら追加
