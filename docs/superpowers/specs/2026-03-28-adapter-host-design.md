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

**Minimal adapter changes (public API addition only):**
- `iotkit-polling-adapter-runtime`: `AdapterHandle::into_parts()` を追加、既存 `shutdown()` はそのまま残す
- `bravepi-mainboard-adapter`: `AdapterHandle::into_parts()` を追加、既存 `shutdown()` はそのまま残す

**Out:**
- 新クレートは作らない
- `core/` への変更なし
- adapter の runtime failure policy（`FatalOnExit` / `Optional`）は将来スコープ
- command routing API は将来スコープ
- gateway config 統一は将来スコープ

## Design

### Fan-in: StreamMap 方式

`tokio_stream::StreamMap` で各アダプターの `event_rx` を直接 multiplex する。

**なぜ StreamMap か:**
- 余分な forwarder タスクと中間チャネルが不要
- backpressure が各アダプターの元のチャネルに直接かかる（中間バッファによる公平性問題なし）
- shutdown 時の deadlock リスクなし

**スケーラビリティについて:** StreamMap は Vec-backed で少数のストリーム向け。現在のアダプター数（2-5）では十分。アダプター数が大幅に増える場合は shared-channel fan-in に移行する。

### AdapterHostEvent

```rust
pub enum AdapterHostEvent {
    Event(EngineEvent),
    AdapterClosed(AdapterId),
}
```

`next_event()` はデータイベントだけでなくアダプターのライフサイクルイベントも返す。これにより：
- gateway が個別アダプターの終了を検知・ログできる（現 main.rs の「BravePI channel closed」と同等）
- 将来の runtime failure policy（`FatalOnExit` / `Optional`）の土台になる

### AdapterHost struct

```rust
use tokio_stream::StreamMap;

pub struct AdapterHost {
    streams: StreamMap<AdapterId, WrappedStream>,
    adapters: Vec<ManagedAdapter>,
}

struct ManagedAdapter {
    id: AdapterId,
    shutdown_fn: Option<Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>>,
}
```

v1 では `command_tx` を AdapterHost に保持しない。コマンドルーティングが必要になったタイミングで追加する。

### WrappedStream（終了検知付き）

素の `ReceiverStream` は終了時に StreamMap から暗黙的に消え、個別アダプターの死亡が見えなくなる。これを防ぐため、ストリーム終了時に `AdapterClosed` イベントを1回 yield するラッパーを使う。

```rust
/// ReceiverStream をラップし、内部ストリーム終了後に None ではなく
/// sentinel 値を1回 yield してからストリーム終了する。
struct WrappedStream {
    inner: ReceiverStream<AdapterEvent>,
    closed_yielded: bool,
}
```

`WrappedStream` は `Stream<Item = WrappedItem>` を実装：
- inner が `Some(event)` → `WrappedItem::Event(event)` を yield
- inner が `None`（終了）かつ `!closed_yielded` → `WrappedItem::Closed` を yield、`closed_yielded = true`
- inner が `None` かつ `closed_yielded` → `None`（ストリーム終了、StreamMap から除去）

`next_event()` は `WrappedItem` を `AdapterHostEvent` に変換する。

### register メソッド

```rust
impl AdapterHost {
    pub fn new() -> Self { ... }

    pub fn register(
        &mut self,
        id: AdapterId,
        event_rx: mpsc::Receiver<AdapterEvent>,
        shutdown_fn: impl FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static,
    ) -> Result<(), String>
```

- `event_rx` を `WrappedStream` でラップし、`StreamMap` に `(id, stream)` として挿入
- shutdown クロージャを `ManagedAdapter` に保存
- **duplicate AdapterId を reject**: 既に同じ ID が登録されていれば `Err` を返す

### next_event メソッド

```rust
    pub async fn next_event(&mut self) -> Option<AdapterHostEvent>
```

`StreamMap::next()` を呼び、`WrappedItem` に応じて：
- `WrappedItem::Event(event)` → `Some(AdapterHostEvent::Event(EngineEvent { adapter_id, event }))`
- `WrappedItem::Closed` → `Some(AdapterHostEvent::AdapterClosed(adapter_id))`
- StreamMap が空 → `None`（全アダプター終了）

### shutdown_all メソッド

```rust
    pub async fn shutdown_all(&mut self)
```

**Shutdown シーケンス（shutdown_all が全責任を持つ）:**

登録の逆順で、各アダプターに対して：
1. `streams.remove(&id)` で StreamMap からストリームを除去 → ReceiverStream ドロップ → receiver close
2. `shutdown_fn` を呼び出し（Shutdown cmd → task/thread join）
3. 結果を per-adapter でログ出力

1つのアダプターの shutdown 失敗が他のアダプターをブロックしないよう、エラーはログして続行する。

shutdown 順序は `shutdown_all` にエンコードされ、呼び出し側がコメント規約に依存しない。

## Handle 分解: into_parts()

### 方針

- `into_parts()` を追加 API として両アダプターに追加
- 既存の `AdapterHandle::shutdown()` はそのまま残す（後方互換）
- `into_parts()` は `{id, event_rx, shutdown_handle}` の3つに分解
- `command_tx` は v1 の into_parts には含めない（AdapterHost が使わないため）

### ShutdownHandle

ShutdownHandle は receiver close を行わない。その責務は AdapterHost にある。ShutdownHandle は「Shutdown cmd 送信 → task/thread join」のみ。

```rust
// bravepi-mainboard-adapter
pub struct ShutdownHandle {
    command_tx: mpsc::Sender<AdapterCommand>,
    source_handle: Option<SerialSourceHandle>,
    event_loop_handle: Option<JoinHandle<()>>,
}

impl ShutdownHandle {
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
```

```rust
// iotkit-polling-adapter-runtime
pub struct ShutdownHandle {
    command_tx: mpsc::Sender<AdapterCommand>,
    task_handle: Option<JoinHandle<()>>,
}

impl ShutdownHandle {
    pub async fn shutdown(mut self) {
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }
    }
}
```

### into_parts シグネチャ

```rust
// 両アダプター共通パターン
pub struct AdapterParts {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub shutdown: ShutdownHandle,
}

impl AdapterHandle {
    /// Decompose the handle for use with AdapterHost.
    /// Existing AdapterHandle::shutdown() remains available for direct use.
    pub fn into_parts(self) -> AdapterParts { ... }
}
```

`command_tx` は ShutdownHandle 内に move される（shutdown cmd 送信に必要）。into_parts 後に command を送りたい場合は、into_parts 前に `command_tx.clone()` しておく。

### 既存 shutdown() との関係

`AdapterHandle::shutdown()` は既存コードのまま残す。into_parts() は additive API。

- gateway の新コード: `into_parts()` → AdapterHost に登録
- テストや直接利用: 従来通り `handle.shutdown().await`

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
            let sh = bravepi_parts.shutdown;
            move || Box::pin(async move {
                if let Err(e) = sh.shutdown().await {
                    tracing::error!(error = %e, "BravePI shutdown error");
                }
            })
        },
    ).expect("Failed to register BravePI adapter");

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
                let sh = rpi_parts.shutdown;
                move || Box::pin(async move { sh.shutdown().await; })
            },
        ).expect("Failed to register RPi local adapter");
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
                    Some(AdapterHostEvent::Event(ev)) => {
                        engine.apply(ev).await;
                    }
                    Some(AdapterHostEvent::AdapterClosed(id)) => {
                        tracing::info!(adapter = %id, "Adapter channel closed");
                    }
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

- **single_adapter_events** — 1 adapter 登録 → next_event で全イベント受信 → AdapterClosed → None
- **multiple_adapters_interleaved** — 2 adapter 登録 → 両方のイベントが受信される
- **adapter_closed_notification** — adapter の sender ドロップ後に AdapterClosed(id) が返る
- **all_closed_returns_none** — 全 adapter 終了後に next_event が None を返す
- **shutdown_all_calls_closures** — shutdown クロージャが呼ばれたことを AtomicBool で確認
- **shutdown_order_is_reverse** — 登録逆順で shutdown されることを記録で確認
- **duplicate_id_rejected** — 同じ AdapterId で2回 register すると Err
- **shutdown_all_after_early_adapter_exit** — 1つが先に死んでも shutdown_all が残りを正常停止
- **ctrl_c_with_buffered_events** — イベントがバッファにある状態で shutdown_all が詰まらない

### into_parts tests（各アダプタークレート内）

- **into_parts_preserves_id** — parts.id が元の handle.id と一致
- **original_shutdown_still_works** — into_parts() 追加後も従来の handle.shutdown() が動く

### 既存テスト

- 両アダプターの既存テストは変更不要（into_parts は追加 API、shutdown は互換維持）
- `cargo test --workspace` が全通することを確認

## Adapter Taxonomy との関係

この設計は polling-adapter-runtime-design.md Section 12 の adapter taxonomy と整合する：
- `runtime_model` の違い（polling / stream_ingress）は AdapterHost の関心外
- `liveness_owner` の違い（adapter / orchestrator）も AdapterHost の関心外
- AdapterHost が知るのは「`AdapterEvent` ストリームを出し、shutdown できるもの」だけ

将来 orchestrator 層が追加された場合も、orchestrator → AdapterHost → Engine の階層で自然に組み合わせられる。`AdapterHostEvent::AdapterClosed` は orchestrator の判断材料になる。
