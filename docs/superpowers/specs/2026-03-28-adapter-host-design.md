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

**Out:**
- 新クレートは作らない
- `core/` への変更なし

**Minimal adapter changes (public API addition only, no internal logic change):**
- `iotkit-polling-adapter-runtime`: `AdapterHandle::into_parts()` を追加
- `bravepi-mainboard-adapter`: `AdapterHandle::into_parts()` を追加
- 両アダプターの既存 `shutdown()` ロジックを `ShutdownHandle` に移動（中身は同一）

## Design

### AdapterHost struct

```rust
pub struct AdapterHost {
    merged_tx: mpsc::Sender<EngineEvent>,
    merged_rx: mpsc::Receiver<EngineEvent>,
    adapters: Vec<ManagedAdapter>,
}
```

`AdapterHost` は登録されたアダプターの forwarder タスクを管理し、全アダプターの `AdapterEvent` を `EngineEvent` に包んで1本の `merged_rx` に集約する。

### ManagedAdapter

```rust
struct ManagedAdapter {
    id: AdapterId,
    command_tx: mpsc::Sender<AdapterCommand>,
    shutdown_fn: Option<Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>>,
    forwarder_handle: JoinHandle<()>,
}
```

各アダプターの shutdown 方法を型消去したクロージャで保持する。これにより：
- bravepi の `shutdown(mut self) -> Result<(), String>`（consume）
- polling-runtime の `shutdown(&mut self)`（borrow）

という異なるシグネチャを統一的に扱える。

### register メソッド

```rust
impl AdapterHost {
    pub fn register(
        &mut self,
        id: AdapterId,
        event_rx: mpsc::Receiver<AdapterEvent>,
        command_tx: mpsc::Sender<AdapterCommand>,
        shutdown_fn: impl FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static,
    )
```

register 時に forwarder タスクを spawn する。forwarder は：
1. `event_rx.recv()` でアダプターからイベントを受信
2. `EngineEvent { adapter_id, event }` に包む
3. `merged_tx.send()` で共通チャネルに転送
4. `event_rx` が閉じたら（adapter 終了 or チャネルドロップ）forwarder も終了し、自身が持つ `merged_tx` clone をドロップする

### event_rx アクセス

```rust
    pub fn event_rx(&mut self) -> &mut mpsc::Receiver<EngineEvent>
```

gateway の main loop が `select!` で使う唯一のイベントソース。

### shutdown_all

```rust
    pub async fn shutdown_all(&mut self)
```

全アダプターの shutdown クロージャを呼び出し、forwarder タスクの完了を await する。順序は登録の逆順（後から登録したものを先に停止）。

### all_closed 検出

全 forwarder タスクが終了すると `merged_tx` の全 sender がドロップされ、`merged_rx.recv()` が `None` を返す。gateway はこれで「全アダプターが閉じた」ことを検出できる。

## gateway main.rs の変更

```rust
async fn run(port_path: String) {
    let engine = Engine::new();
    let mut host = AdapterHost::new();

    // BravePI — required
    let bravepi = bravepi_mainboard_adapter::task::start(port_path)?;
    host.register(bravepi.id, bravepi.event_rx, bravepi.command_tx, {
        // bravepi.shutdown() consumes self → move into closure
        move || Box::pin(async move {
            if let Err(e) = bravepi_handle.shutdown().await {
                tracing::error!(error = %e, "BravePI shutdown error");
            }
        })
    });

    // RPi local — optional
    if rpi_local_enabled {
        let rpi = rpi_local_adapter::start(config)?;
        host.register(rpi.id, rpi.event_rx, rpi.command_tx, {
            move || Box::pin(async move { rpi_handle.shutdown().await; })
        });
    }

    // Unified fan-in loop
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            event = host.event_rx().recv() => {
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
}
```

## register 時の Handle 分解

register は `event_rx` と `command_tx` を引き取る。残りの Handle フィールド（bravepi の `source_handle` + `event_loop_handle`、polling の `task_handle`）は shutdown クロージャに move される。

つまり、register の呼び出し側で Handle を分解する必要がある。現状の Handle は `event_rx` と `command_tx` が `pub` フィールドなので、構造体から直接取り出せる。ただし bravepi の private フィールド（`source_handle`, `event_loop_handle`）は shutdown クロージャ内でのみ使われるため、Handle ごとクロージャに move する形になる。

具体的には：
- **bravepi**: `event_rx` と `command_tx` を取り出した後、残りの Handle を shutdown クロージャに move。ただし現状の Handle は部分 move を許さない構造のため、`into_parts()` メソッドを追加するか、Handle ごと move して shutdown クロージャ内で `event_rx` / `command_tx` の重複を避ける設計が必要。
- **polling-runtime**: 同様の問題。`event_rx` と `command_tx` は pub だが、`task_handle` は private。

**解決策:** 両アダプターに `into_parts()` を追加して、Handle を `(AdapterId, Receiver, Sender, ShutdownHandle)` に分解可能にする。`ShutdownHandle` はアダプター固有の型で、`async fn shutdown(self)` を持つ。

```rust
// bravepi-mainboard-adapter
pub struct ShutdownHandle { source_handle, event_loop_handle }
impl ShutdownHandle { pub async fn shutdown(self) -> Result<(), String> { ... } }

// iotkit-polling-adapter-runtime
pub struct ShutdownHandle { task_handle }
impl ShutdownHandle { pub async fn shutdown(self) { ... } }
```

これは「アダプター内部の変更」に見えるが、public API の追加のみで内部ロジックの変更はない。既存の `shutdown()` メソッドの中身を `ShutdownHandle` に移すだけ。

## Testing

- **adapter_host unit tests**: StubAdapter（即座に N 個のイベントを送って閉じる）を使って：
  - 単一アダプター登録 → イベント受信 → all_closed 検出
  - 複数アダプター登録 → イベントが interleave される
  - shutdown_all → 全 forwarder 停止
- **既存テスト**: 変更なし（アダプター内部は触らないため）
- **gateway 統合**: main.rs のリファクタ後に `cargo test --workspace` が通ること

## Adapter Taxonomy との関係

この設計は polling-adapter-runtime-design.md Section 12 の adapter taxonomy と整合する：
- `runtime_model` の違い（polling / stream_ingress）は AdapterHost の関心外
- `liveness_owner` の違い（adapter / orchestrator）も AdapterHost の関心外
- AdapterHost が知るのは「`AdapterEvent` を出し、`AdapterCommand` を受け、shutdown できるもの」だけ

将来 orchestrator 層が追加された場合も、orchestrator → AdapterHost → Engine の階層で自然に組み合わせられる。
