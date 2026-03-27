# Sub-project A: Transport 抽象化 — 設計 Spec

## 目的

`bravepi-adapter/src/task/handle.rs` が現在まとめて持っている3つの責務
(ポートオープン / reader thread + reconnect / adapter 起動) を分離し、
adapter crate を「byte stream を受けて event を返す側」に寄せる。

これにより:
- event_loop のテスタビリティを維持したまま、transport 層を差し替え可能にする
- mock/replay テストが channel 境界だけで完結する状態を明示的な設計にする
- 将来の USB/TCP 化、共通 supervisor、base adapter 化への準備を整える

## 方針

### channel 境界を adapter の入力契約にする (ByteStream trait は導入しない)

event_loop は既に `mpsc::Receiver<Result<Vec<u8>, String>>` で閉じており、
テスト (`event_loop_test.rs`) もこの境界だけで成立している。

ByteStream trait を導入しない理由:
- `SerialTransport::read()` は blocking なので、trait を入れても async の
  event_loop には直接刺さらず、結局 reader thread か spawn_blocking と channel が残る
- reconnect は read/write だけでは表現できず、factory/session 抽象が追加で必要になる
- 現段階では過剰

ByteStream / TransportSession が必要になるのは「複数 transport を同じ双方向
session API で扱う」「adapter が active write/request-response を多用する」段階。

### TransportError は terminal failure 専用

`recoverable: bool` は持たない。recoverable を判断して再接続する責務は
serial_source 側にあり、event_loop に見せると transport policy が逆流する。

event_loop に渡すのは「bytes」か「source が継続不能になった理由」のどちらか。
将来 degraded 状態を扱いたくなったら、別の status channel か
TransportMessage enum を足す方が筋が良い。

## 型定義

### TransportError

```rust
// bravepi-adapter/src/transport.rs

/// Transport source が回復不能な障害で停止した理由。
#[derive(Debug, Clone)]
pub struct TransportError {
    pub message: String,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// event_loop が受け取る byte stream の型。
pub type BytesReceiver = mpsc::Receiver<Result<Vec<u8>, TransportError>>;
```

core 側の `AdapterError.error` はまだ `String` なので、
event_loop から core へは `error.to_string()` で流す。

## 責務分割

| 責務 | モジュール | 依存 |
|------|-----------|------|
| port open + reader thread + reconnect + shutdown 協調 | `task::serial_source` | `rpi4b_transport::SerialTransport` |
| bytes → codec → AdapterEvent | `task::event_loop` (変更なし) | `bravepi_codec`, `iotkit_core_types` |
| channel wiring と起動 | `task::start()` (薄くなる) | `serial_source` + `event_loop` |

## API (全て crate 内部)

### serial_source

```rust
// bravepi-adapter/src/task/serial_source.rs

pub(crate) struct SerialSource {
    pub bytes_rx: BytesReceiver,
    pub handle: SerialSourceHandle,
}

pub(crate) struct SerialSourceHandle {
    thread_handle: std::thread::JoinHandle<()>,
}

impl SerialSourceHandle {
    pub async fn join(self) -> Result<(), String> {
        tokio::task::spawn_blocking(|| self.thread_handle.join())
            .await
            .map_err(|_| "spawn_blocking failed".to_string())?
            .map_err(|_| "Reader thread panicked".to_string())
    }
}

/// SerialTransport を開き、reader thread を起動する。
/// reconnect ロジックもこの中に閉じる。
pub fn start(port_path: &str) -> Result<SerialSource, io::Error> {
    let config = serial_config();
    let transport = SerialTransport::open(port_path, &config)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let (bytes_tx, bytes_rx) = mpsc::channel(64);
    let owned_path = port_path.to_string();
    let thread_handle = std::thread::Builder::new()
        .name(format!("bravepi-serial-{}", port_path))
        .spawn(move || serial_reader_thread(owned_path, transport, bytes_tx))?;
    Ok(SerialSource {
        bytes_rx,
        handle: SerialSourceHandle { thread_handle },
    })
}
```

`serial_reader_thread` は現在の `reader.rs` の中身をそのまま移動する。
ただし `bytes_tx` の型が `mpsc::Sender<Result<Vec<u8>, String>>` から
`mpsc::Sender<Result<Vec<u8>, TransportError>>` に変わるため、
エラー送信箇所で `TransportError { message: msg }` を構築する。
`reader.rs` は削除する。

### task::start() (唯一の public API)

```rust
pub fn start(port_path: String) -> Result<AdapterHandle, io::Error> {
    let source = serial_source::start(&port_path)?;

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(256);
    let (command_tx, command_rx) = mpsc::channel::<AdapterCommand>(32);
    let id = AdapterId::new(format!("bravepi:{}", port_path));

    let event_loop_handle = tokio::spawn(
        event_loop(port_path, source.bytes_rx, event_tx, command_rx)
    );

    Ok(AdapterHandle {
        id,
        event_rx,
        command_tx,
        source_handle: Some(source.handle),
        event_loop_handle: Some(event_loop_handle),
    })
}
```

### AdapterHandle

```rust
pub struct AdapterHandle {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
    source_handle: Option<SerialSourceHandle>,
    event_loop_handle: Option<tokio::task::JoinHandle<()>>,
}
```

## Shutdown シーケンス

```rust
pub async fn shutdown(mut self) -> Result<(), String> {
    // 1. event_rx を close → event_loop の send() が Err で抜ける (buffer 詰まり対策)
    self.event_rx.close();

    // 2. Shutdown コマンド送信 → event_loop が select で観測して return
    let _ = self.command_tx.send(AdapterCommand::Shutdown).await;

    // 3. event_loop の完了を待つ
    if let Some(handle) = self.event_loop_handle.take() {
        handle.await.map_err(|e| format!("event_loop panicked: {}", e))?;
    }

    // 4. reader thread の join
    //    event_loop 終了 → bytes_rx drop → bytes_tx.is_closed() = true
    //    → reader thread が次の is_closed() チェックで終了
    if let Some(source) = self.source_handle.take() {
        source.join().await?;
    }

    Ok(())
}
```

**因果関係:**
```
event_rx.close()
    ↓
event_loop の send() が Err → loop 脱出 (buffer 詰まりケース)
    ↓
Shutdown cmd → event_loop が select で観測 → return (通常ケース)
    ↓
event_loop 終了 → bytes_rx drop
    ↓
bytes_tx.is_closed() = true
    ↓
reader thread 終了 (read timeout 500ms か retry backoff 1秒刻み以内)
    ↓
source.join() 完了
```

## ファイル構成と変更範囲

### 新規
- `bravepi-adapter/src/transport.rs` — `TransportError`, `BytesReceiver` 型定義
- `bravepi-adapter/src/task/serial_source.rs` — `SerialSource`, `SerialSourceHandle`, `start()`, `serial_reader_thread()` (reader.rs から移動)

### 変更
- `bravepi-adapter/src/task/handle.rs` — `start()` を薄く、`AdapterHandle` に `event_loop_handle` + `source_handle` 追加、`shutdown()` に `event_rx.close()` 追加
- `bravepi-adapter/src/task/mod.rs` — `mod reader;` → `mod serial_source;` に置き換え。`pub use event_loop::event_loop;` を削除 (event_loop は pub(crate) に)
- `bravepi-adapter/src/lib.rs` — `pub(crate) mod transport;` 追加 (crate 内部境界)

### 削除
- `bravepi-adapter/src/task/reader.rs` — 内容は `serial_source.rs` に完全移動

### 変更
- `bravepi-adapter/src/task/event_loop.rs` — `pub async fn` → `pub(crate) async fn` に変更。シグネチャは変更なし (引数の型が alias になるだけ)
- `bravepi-adapter/src/task/convert.rs` — `pub fn frame_to_event` → `pub(crate) fn frame_to_event` に変更 (内容は変更なし)

### 変更なし
- `bravepi-adapter/codec/` 全体
- `bravepi-adapter/sensors/` 全体
- `core/types/src/lib.rs`
- `rpi4b-driver/`

### テスト変更
- `bravepi-adapter/tests/event_loop_test.rs` → `bravepi-adapter/src/task/event_loop_test.rs` に移動 (crate 内 unit test 化)。`pub(crate)` の event_loop と TransportError に直接アクセスできる。`Err(String)` → `Err(TransportError)` に変更。`event_rx.close()` で event_loop が抜けるテストを追加可能
- `bravepi-adapter/tests/frame_to_event_test.rs` → `bravepi-adapter/src/task/convert_test.rs` に移動 (frame_to_event も pub(crate) になるため)
- 両テストファイルは `#[cfg(test)] mod tests` ではなく `#[cfg(test)] mod <name>_test;` で隣接ファイルとして配置

### スコープ外
- `serial_source` の単体テストは不要 (実ポートが必要なため)
- `AdapterHandle::shutdown()` の end-to-end 順序保証テストは今回はスコープ外 (実ポートなしでは直接取りにくい)

## 将来の拡張点

### Downlink (adapter → device)

現在 `BravePiCodec::encode_downlink()` は存在するが、event_loop から serial port に書く経路がない。
transport 抽象化後の拡張方針:

- `serial_source` に `bytes_tx` (adapter → serial) を追加するか、
  `AdapterCommand` に downlink variant を足して event_loop 経由で送るかは、
  Sub-project D (command/query boundary) で決める
- 今回は read-only の channel 境界のみ

### 他の transport source

USB/TCP/replay 等を追加する場合、`serial_source` と同じ
`(BytesReceiver, SourceHandle)` パターンで別の source モジュールを作る。
`task::start()` が source を選ぶ形になる。
