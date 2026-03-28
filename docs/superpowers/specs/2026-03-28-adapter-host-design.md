# AdapterHost Design Spec

## Goal

Gateway がアダプターの内部実装（polling / streaming）を知らずに均質に管理できるようにする。`iotkit-gateway` 内に `AdapterHost` モジュールを作り、fan-in とライフサイクル管理を一元化する。

## Motivation

現状の `main.rs` は各アダプターごとに `select!` ブランチを手書きしている。アダプターが増えるたびにブランチ追加・open フラグ管理・shutdown 分岐が必要で、スケールしない。

両アダプターは既に `AdapterEvent` / `AdapterCommand` というメッセージ契約を共有している。差異を「吸収」するのではなく、差異を持ったまま「統一的に扱える」インターフェースを gateway 側に作る。

## Scope

**In:**
- `iotkit-gateway/src/adapter_host.rs` — 新規モジュール（gateway 内部のみ）
- `iotkit-gateway/src/main.rs` — AdapterHost を使うようにリファクタ
- `tokio-stream` 依存を `iotkit-gateway/Cargo.toml` に追加

**Minimal adapter changes (public API addition only, no internal logic change):**
- `iotkit-polling-adapter-runtime`: `AdapterHandle::into_parts()` を追加
- `bravepi-mainboard-adapter`: `AdapterHandle::into_parts()` を追加
- 両アダプターの既存 `shutdown()` ロジックを `ShutdownHandle` に移動（中身は同一）

**Out:**
- 新クレートは作らない
- `core/` への変更なし
- adapter の runtime failure policy（`FatalOnExit` / `Optional`）は将来スコープ

## Design

### Fan-in: StreamMap 方式

forwarder タスク + merged channel ではなく、`tokio_stream::StreamMap` で各アダプターの `event_rx` を直接 multiplex する。

**なぜ StreamMap か:**
- 余分な forwarder タスクと中間チャネルが不要
- all_closed 検出が自然（StreamMap が空になると `poll_next` が `None` を返す）
- backpressure が各アダプターの元のチャネルに直接かかる（中間バッファによる公平性問題なし）
- shutdown 時の deadlock リスクなし（forwarder が中間チャネルで詰まる問題が存在しない）

### AdapterHost struct

```rust
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamMap;

pub struct AdapterHost {
    streams: StreamMap<AdapterId, ReceiverStream<AdapterEvent>>,
    adapters: Vec<ManagedAdapter>,
}

struct ManagedAdapter {
    id: AdapterId,
    shutdown_fn: Option<Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>>,
}
```

`command_tx` は v1 では AdapterHost に保持しない。コマンドルーティングが必要になったタイミングで `send_command(adapter_id, cmd)` API とともに追加する。

### register メソッド

```rust
impl AdapterHost {
    pub fn new() -> Self { ... }

    pub fn register(
        &mut self,
        id: AdapterId,
        event_rx: mpsc::Receiver<AdapterEvent>,
        shutdown_fn: impl FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static,
    )
```

`event_rx` を `ReceiverStream` でラップし、`StreamMap` に `(id, stream)` として挿入する。shutdown クロージャは `ManagedAdapter` に保存。

### next_event メソッド

```rust
    pub async fn next_event(&mut self) -> Option<EngineEvent>
```

`StreamMap::next()` を呼び、`(adapter_id, event)` を `EngineEvent { adapter_id, event }` に包んで返す。全ストリームが終了したら `None` を返す。

### shutdown_all メソッド

```rust
    pub async fn shutdown_all(&mut self)
```

登録の逆順で各アダプターの shutdown クロージャを呼び出す。

**Shutdown シーケンス:**
1. shutdown クロージャ内で、各アダプター固有の shutdown 処理を実行（receiver close → Shutdown cmd → task join）
2. StreamMap のストリームは、アダプターの event_tx がドロップされた時点で自然に終了する

各 shutdown クロージャの結果は per-adapter でログ出力する。1つのアダプターの shutdown 失敗が他のアダプターの shutdown をブロックしないようにする。

### all_closed 検出

`next_event()` が `None` を返す = StreamMap 内の全ストリームが終了 = 全アダプターのチャネルが閉じた。forwarder + merged channel 方式と違い、AdapterHost 自身が sender を保持する問題がない。

## Handle 分解: into_parts()

register は `event_rx` を引き取り、shutdown クロージャは残りの Handle 部品を所有する。現状の Handle は private フィールドを含むため、部分 move ができない。

**解決策:** 両アダプターに `into_parts()` を追加する。

```rust
// bravepi-mainboard-adapter
pub struct AdapterParts {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
    pub shutdown: ShutdownHandle,
}

pub struct ShutdownHandle {
    event_rx_close: mpsc::Receiver<AdapterEvent>,  // ← 不要、後述
    source_handle: Option<SerialSourceHandle>,
    event_loop_handle: Option<JoinHandle<()>>,
    command_tx: mpsc::Sender<AdapterCommand>,
}
```

**Shutdown と receiver-close の問題:**

現在の bravepi `shutdown()` は `event_rx.close()` を最初に呼ぶ。これは event_loop の `event_tx.send()` を即座に失敗させ、バッファ詰まりによる shutdown hang を防ぐ。

`into_parts()` で `event_rx` を AdapterHost に渡した後は、ShutdownHandle が `event_rx` を持たない。しかし StreamMap 方式では問題にならない：
- StreamMap は `event_rx` をラップした ReceiverStream を所有
- shutdown 時に `AdapterHost` が StreamMap からストリームを remove すれば、ReceiverStream がドロップされ、内部の Receiver が close される
- これは従来の `event_rx.close()` と同じ効果

**Shutdown シーケンス（per-adapter）:**
1. AdapterHost が StreamMap から該当ストリームを remove（= receiver close）
2. ShutdownHandle が Shutdown コマンド送信
3. ShutdownHandle がタスク/スレッドの join を await

```rust
// bravepi-mainboard-adapter
impl ShutdownHandle {
    /// Shutdown the adapter. Caller must close the event receiver first
    /// (e.g. by dropping the ReceiverStream).
    pub async fn shutdown(mut self) -> Result<(), String> {
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.event_loop_handle.take() {
            handle.await.map_err(|e| format!("event_loop panicked: {e}"))?;
        }
        if let Some(source) = self.source_handle.take() {
            source.join().await?;
        }
        Ok(())
    }
}

// iotkit-polling-adapter-runtime
impl ShutdownHandle {
    pub async fn shutdown(mut self) {
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }
    }
}
```

### into_parts の具体シグネチャ

```rust
// bravepi-mainboard-adapter
impl AdapterHandle {
    pub fn into_parts(self) -> AdapterParts {
        AdapterParts {
            id: self.id,
            event_rx: self.event_rx,
            command_tx: self.command_tx.clone(),
            shutdown: ShutdownHandle {
                source_handle: self.source_handle,
                event_loop_handle: self.event_loop_handle,
                command_tx: self.command_tx,
            },
        }
    }
}

// iotkit-polling-adapter-runtime — 同様のパターン
```

`command_tx` は shutdown にも必要なため、clone して両方に渡す。

## gateway main.rs の変更

```rust
async fn run(port_path: String) {
    let engine = Engine::new();
    let mut host = AdapterHost::new();

    // BravePI — required
    let bravepi = bravepi_mainboard_adapter::task::start(port_path)
        .expect("Failed to start BravePI adapter");
    let bravepi_parts = bravepi.into_parts();
    host.register(
        bravepi_parts.id,
        bravepi_parts.event_rx,
        {
            let mut sh = bravepi_parts.shutdown;
            move || Box::pin(async move {
                if let Err(e) = sh.shutdown().await {
                    tracing::error!(error = %e, "BravePI shutdown error");
                }
            })
        },
    );

    // RPi local — optional
    let rpi_local_enabled = std::env::var("RPI_LOCAL_ENABLED")
        .map(|v| v == "1")
        .unwrap_or(false);

    if rpi_local_enabled {
        let rpi = rpi_local_adapter::start(rpi_local_config())
            .expect("Failed to start RPi local adapter");
        let rpi_parts = rpi.into_parts();
        host.register(
            rpi_parts.id,
            rpi_parts.event_rx,
            {
                let mut sh = rpi_parts.shutdown;
                move || Box::pin(async move { sh.shutdown().await; })
            },
        );
    }

    // Unified fan-in loop
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
                break;
            }
            event = host.next_event() => {
                match event {
                    Some(ev) => engine.apply(ev).await,
                    None => {
                        tracing::info!("All adapter channels closed");
                        break;
                    }
                }
            }
        }
    }

    host.shutdown_all().await;

    let devices = engine.devices().await;
    tracing::info!(device_count = devices.len(), "Engine state at shutdown");
}
```

## Testing

### adapter_host unit tests

StubAdapter（`mpsc::channel` で即座に N 個のイベントを送って close する）を使う：

- **single_adapter_events** — 1つの adapter 登録 → next_event で全イベント受信 → None で all_closed
- **multiple_adapters_interleaved** — 2つの adapter 登録 → 両方のイベントが受信される（順序は非決定的）
- **all_closed_detection** — 全 adapter の sender ドロップ後に next_event が None を返す
- **shutdown_all_calls_closures** — shutdown クロージャが呼ばれたことを AtomicBool で確認
- **shutdown_order_is_reverse** — 登録逆順で shutdown されることを Vec<usize> の記録で確認

### into_parts tests（各アダプタークレート内）

- **into_parts_preserves_id** — parts.id が元の handle.id と一致
- **shutdown_handle_works** — ShutdownHandle.shutdown() が正常に完了（tokio::test で stub transport を使用）

### 既存テスト

- 両アダプターの既存テストは変更不要（`into_parts()` は追加 API）
- `cargo test --workspace` が全通することを確認

## Adapter Taxonomy との関係

この設計は polling-adapter-runtime-design.md Section 12 の adapter taxonomy と整合する：
- `runtime_model` の違い（polling / stream_ingress）は AdapterHost の関心外
- `liveness_owner` の違い（adapter / orchestrator）も AdapterHost の関心外
- AdapterHost が知るのは「`AdapterEvent` ストリームを出し、shutdown できるもの」だけ

将来 orchestrator 層が追加された場合も、orchestrator → AdapterHost → Engine の階層で自然に組み合わせられる。
