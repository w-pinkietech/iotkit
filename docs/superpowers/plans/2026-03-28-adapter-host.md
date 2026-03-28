# AdapterHost Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Gateway の fan-in とアダプターライフサイクル管理を `AdapterHost` に一元化し、手書き `select!` ブランチを不要にする。

**Architecture:** `tokio_stream::StreamMap` で各アダプターの event receiver を multiplex。`WrappedStream` が stream 終了を検知して `AdapterClosed` を yield。`into_parts()` で Handle を分解し、shutdown ロジックは `ShutdownHandle` に分離。

**Tech Stack:** Rust, tokio, tokio-stream (StreamMap, ReceiverStream)

**Spec:** `docs/superpowers/specs/2026-03-28-adapter-host-design.md`

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `iotkit-gateway/src/adapter_host.rs` | AdapterHost, WrappedStream, AdapterHostEvent |
| Modify | `iotkit-gateway/src/main.rs` | Use AdapterHost instead of manual select! |
| Modify | `iotkit-gateway/Cargo.toml` | Add tokio-stream dependency, enable tokio sync feature |
| Modify | `iotkit-polling-adapter-runtime/src/lib.rs` | Add ShutdownHandle, AdapterParts, into_parts() |
| Modify | `bravepi-mainboard-adapter/src/task/handle.rs` | Add ShutdownHandle, AdapterParts, into_parts() |

---

### Task 1: Add into_parts() to polling-adapter-runtime

**Files:**
- Modify: `iotkit-polling-adapter-runtime/src/lib.rs:67-94`

- [ ] **Step 1: Write failing test for into_parts**

Add to the existing `#[cfg(test)] mod tests` block in `iotkit-polling-adapter-runtime/src/lib.rs`:

```rust
#[tokio::test]
async fn into_parts_preserves_id_and_channels() {
    use iotkit_core_types::SensorType;

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(1);
    let (command_tx, mut command_rx) = mpsc::channel::<AdapterCommand>(1);
    let handle = AdapterHandle {
        id: AdapterId::new("test:into-parts"),
        event_rx,
        command_tx,
        task_handle: None,
    };
    let parts = handle.into_parts();

    // ID preserved
    assert_eq!(parts.id.as_str(), "test:into-parts");

    // event_rx works: send an event, receive it from parts.event_rx
    let mut event_rx = parts.event_rx;
    event_tx
        .send(AdapterEvent::SensorData {
            device_key: iotkit_core_types::DeviceKey::new("test:0"),
            reading: SensorReading::empty(SensorType::Temperature),
            rssi: None,
            battery_pct: None,
        })
        .await
        .unwrap();
    let received = event_rx.recv().await;
    assert!(received.is_some(), "event_rx should receive the sent event");

    // ShutdownHandle sends Shutdown command via command_tx
    parts.shutdown.shutdown().await.ok();
    let cmd = command_rx.recv().await;
    assert!(
        matches!(cmd, Some(AdapterCommand::Shutdown)),
        "shutdown should send Shutdown command"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p iotkit-polling-adapter-runtime into_parts_preserves_id_and_channels`
Expected: FAIL — `into_parts` method not found, `AdapterParts` / `ShutdownHandle` not defined

- [ ] **Step 3: Implement ShutdownHandle, AdapterParts, into_parts()**

Add after the existing `AdapterHandle` impl block (after line 94):

```rust
/// Parts returned by [`AdapterHandle::into_parts`].
pub struct AdapterParts {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub shutdown: ShutdownHandle,
}

/// Opaque handle for shutting down a polling adapter.
///
/// Does NOT close the event receiver — that is the caller's responsibility
/// (e.g. by dropping the `event_rx` or the stream wrapping it).
/// ShutdownHandle only sends `Shutdown` and awaits the background task.
pub struct ShutdownHandle {
    command_tx: mpsc::Sender<AdapterCommand>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ShutdownHandle {
    pub async fn shutdown(mut self) -> Result<(), String> {
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.task_handle.take() {
            handle
                .await
                .map_err(|e| format!("polling task panicked: {e}"))?;
        }
        Ok(())
    }
}

impl AdapterHandle {
    /// Decompose this handle into parts for use with an adapter host.
    ///
    /// The existing [`AdapterHandle::shutdown`] method remains available
    /// for direct use — `into_parts` is an additive API.
    pub fn into_parts(self) -> AdapterParts {
        AdapterParts {
            id: self.id,
            event_rx: self.event_rx,
            shutdown: ShutdownHandle {
                command_tx: self.command_tx,
                task_handle: self.task_handle,
            },
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p iotkit-polling-adapter-runtime into_parts_preserves_id_and_channels`
Expected: PASS

- [ ] **Step 5: Run full crate tests to verify no regression**

Run: `cargo test -p iotkit-polling-adapter-runtime`
Expected: All existing tests pass

- [ ] **Step 6: Commit**

```bash
git add iotkit-polling-adapter-runtime/src/lib.rs
git commit -m "feat(polling-adapter-runtime): add into_parts() and ShutdownHandle"
```

---

### Task 2: Add into_parts() to bravepi-mainboard-adapter

**Files:**
- Modify: `bravepi-mainboard-adapter/src/task/handle.rs`
- Modify: `bravepi-mainboard-adapter/src/task/mod.rs` (re-export)

- [ ] **Step 1: Write failing test for into_parts**

Add to the existing `#[cfg(test)] mod tests` block in `bravepi-mainboard-adapter/src/task/handle.rs`:

```rust
#[tokio::test]
async fn into_parts_preserves_id_and_channels() {
    use iotkit_core_types::{DeviceKey, SensorReading, SensorType};

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(1);
    let (command_tx, mut command_rx) = mpsc::channel::<AdapterCommand>(1);
    let handle = AdapterHandle {
        id: AdapterId::new("test:into-parts"),
        event_rx,
        command_tx,
        source_handle: None,
        event_loop_handle: None,
    };
    let parts = handle.into_parts();

    // ID preserved
    assert_eq!(parts.id.as_str(), "test:into-parts");

    // event_rx works
    let mut event_rx = parts.event_rx;
    event_tx
        .send(AdapterEvent::SensorData {
            device_key: DeviceKey::new("test:0"),
            reading: SensorReading::empty(SensorType::Temperature),
            rssi: None,
            battery_pct: None,
        })
        .await
        .unwrap();
    let received = event_rx.recv().await;
    assert!(received.is_some(), "event_rx should receive the sent event");

    // ShutdownHandle sends Shutdown command
    parts.shutdown.shutdown().await.ok();
    let cmd = command_rx.recv().await;
    assert!(
        matches!(cmd, Some(AdapterCommand::Shutdown)),
        "shutdown should send Shutdown command"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bravepi-mainboard-adapter into_parts_preserves_id_and_channels`
Expected: FAIL — `into_parts` method not found, `AdapterParts` / `ShutdownHandle` not defined

- [ ] **Step 3: Implement ShutdownHandle, AdapterParts, into_parts()**

Add to `bravepi-mainboard-adapter/src/task/handle.rs` after the existing `AdapterHandle` impl:

```rust
/// Parts returned by [`AdapterHandle::into_parts`].
pub struct AdapterParts {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub shutdown: ShutdownHandle,
}

/// Opaque handle for shutting down the BravePI adapter.
///
/// Does NOT close the event receiver — that is the caller's responsibility.
/// ShutdownHandle sends `Shutdown` and joins both the event loop task and
/// the reader thread.
pub struct ShutdownHandle {
    command_tx: mpsc::Sender<AdapterCommand>,
    source_handle: Option<SerialSourceHandle>,
    event_loop_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ShutdownHandle {
    pub async fn shutdown(mut self) -> Result<(), String> {
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.event_loop_handle.take() {
            handle
                .await
                .map_err(|e| format!("event_loop panicked: {e}"))?;
        }
        if let Some(source) = self.source_handle.take() {
            source.join().await?;
        }
        Ok(())
    }
}

impl AdapterHandle {
    /// Decompose this handle into parts for use with an adapter host.
    ///
    /// The existing [`AdapterHandle::shutdown`] method remains available
    /// for direct use — `into_parts` is an additive API.
    pub fn into_parts(self) -> AdapterParts {
        AdapterParts {
            id: self.id,
            event_rx: self.event_rx,
            shutdown: ShutdownHandle {
                command_tx: self.command_tx,
                source_handle: self.source_handle,
                event_loop_handle: self.event_loop_handle,
            },
        }
    }
}
```

- [ ] **Step 4: Update mod.rs re-exports**

In `bravepi-mainboard-adapter/src/task/mod.rs`, add re-exports for the new types:

```rust
pub use handle::{start, AdapterHandle, AdapterParts, ShutdownHandle};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p bravepi-mainboard-adapter into_parts_preserves_id_and_channels`
Expected: PASS

- [ ] **Step 6: Run full crate tests**

Run: `cargo test -p bravepi-mainboard-adapter`
Expected: All existing tests pass

- [ ] **Step 7: Commit**

```bash
git add bravepi-mainboard-adapter/src/task/handle.rs bravepi-mainboard-adapter/src/task/mod.rs
git commit -m "feat(bravepi-mainboard-adapter): add into_parts() and ShutdownHandle"
```

---

### Task 3: Create AdapterHost with WrappedStream and register

**Files:**
- Create: `iotkit-gateway/src/adapter_host.rs`
- Modify: `iotkit-gateway/Cargo.toml`
- Modify: `iotkit-gateway/src/main.rs` (add module declaration)

- [ ] **Step 1: Add tokio-stream dependency**

Update `iotkit-gateway/Cargo.toml` `[dependencies]`:

```toml
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal", "sync"] }
tokio-stream = { version = "0.1", features = ["sync"] }
```

The `sync` feature on tokio is needed for `mpsc` in adapter_host. The `sync` feature on tokio-stream is needed for `ReceiverStream`.

- [ ] **Step 2: Write failing tests**

Create `iotkit-gateway/src/adapter_host.rs` with tests first:

```rust
//! AdapterHost: unified fan-in and lifecycle management for adapters.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt, StreamMap};

use iotkit_core_engine::EngineEvent;
use iotkit_core_types::{AdapterEvent, AdapterId};

#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::{DeviceKey, SensorReading, SensorType};

    fn stub_event() -> AdapterEvent {
        AdapterEvent::SensorData {
            device_key: DeviceKey::new("test:0"),
            reading: SensorReading::empty(SensorType::Temperature),
            rssi: None,
            battery_pct: None,
        }
    }

    #[tokio::test]
    async fn single_adapter_events() {
        let mut host = AdapterHost::new();
        let (tx, rx) = mpsc::channel(16);
        host.register(
            AdapterId::new("a"),
            rx,
            || Box::pin(async { Ok(()) }),
        ).unwrap();

        tx.send(stub_event()).await.unwrap();
        tx.send(stub_event()).await.unwrap();
        drop(tx);

        // Should receive 2 events, then AdapterClosed, then None
        let e1 = host.next_event().await;
        assert!(matches!(e1, Some(AdapterHostEvent::Event(_))));
        let e2 = host.next_event().await;
        assert!(matches!(e2, Some(AdapterHostEvent::Event(_))));
        let closed = host.next_event().await;
        assert!(matches!(closed, Some(AdapterHostEvent::AdapterClosed(id)) if id.as_str() == "a"));
        let done = host.next_event().await;
        assert!(done.is_none());
    }

    #[tokio::test]
    async fn multiple_adapters_interleaved() {
        let mut host = AdapterHost::new();
        let (tx_a, rx_a) = mpsc::channel(16);
        let (tx_b, rx_b) = mpsc::channel(16);
        host.register(AdapterId::new("a"), rx_a, || Box::pin(async { Ok(()) })).unwrap();
        host.register(AdapterId::new("b"), rx_b, || Box::pin(async { Ok(()) })).unwrap();

        tx_a.send(stub_event()).await.unwrap();
        tx_b.send(stub_event()).await.unwrap();
        drop(tx_a);
        drop(tx_b);

        let mut event_count = 0;
        let mut closed_count = 0;
        while let Some(ev) = host.next_event().await {
            match ev {
                AdapterHostEvent::Event(_) => event_count += 1,
                AdapterHostEvent::AdapterClosed(_) => closed_count += 1,
            }
        }
        assert_eq!(event_count, 2);
        assert_eq!(closed_count, 2);
    }

    #[tokio::test]
    async fn adapter_closed_notification() {
        let mut host = AdapterHost::new();
        let (tx, rx) = mpsc::channel(16);
        host.register(AdapterId::new("x"), rx, || Box::pin(async { Ok(()) })).unwrap();
        drop(tx); // close immediately

        let ev = host.next_event().await;
        assert!(matches!(ev, Some(AdapterHostEvent::AdapterClosed(id)) if id.as_str() == "x"));
        assert!(host.next_event().await.is_none());
    }

    #[tokio::test]
    async fn all_closed_returns_none() {
        let mut host = AdapterHost::new();
        // No adapters registered
        assert!(host.next_event().await.is_none());
    }

    #[tokio::test]
    async fn duplicate_id_rejected() {
        let mut host = AdapterHost::new();
        let (_tx1, rx1) = mpsc::channel::<AdapterEvent>(1);
        let (_tx2, rx2) = mpsc::channel::<AdapterEvent>(1);
        host.register(AdapterId::new("dup"), rx1, || Box::pin(async { Ok(()) })).unwrap();
        let result = host.register(AdapterId::new("dup"), rx2, || Box::pin(async { Ok(()) }));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_all_calls_closures() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let mut host = AdapterHost::new();
        let (tx, rx) = mpsc::channel(1);
        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();
        host.register(
            AdapterId::new("s"),
            rx,
            move || Box::pin(async move { called_clone.store(true, Ordering::SeqCst); Ok(()) }),
        ).unwrap();
        drop(tx);

        host.shutdown_all().await;
        assert!(called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn shutdown_order_is_reverse() {
        use std::sync::{Arc, Mutex};

        let mut host = AdapterHost::new();
        let order = Arc::new(Mutex::new(Vec::new()));

        for name in ["first", "second", "third"] {
            let (_tx, rx) = mpsc::channel::<AdapterEvent>(1);
            let order_clone = order.clone();
            let name_owned = name.to_string();
            host.register(
                AdapterId::new(name),
                rx,
                move || Box::pin(async move {
                    order_clone.lock().unwrap().push(name_owned);
                    Ok(())
                }),
            ).unwrap();
        }

        host.shutdown_all().await;
        let recorded = order.lock().unwrap().clone();
        assert_eq!(recorded, vec!["third", "second", "first"]);
    }

    #[tokio::test]
    async fn shutdown_all_after_early_adapter_exit() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let mut host = AdapterHost::new();

        // Adapter "early" closes immediately
        let (tx_early, rx_early) = mpsc::channel(1);
        drop(tx_early);
        let early_called = Arc::new(AtomicBool::new(false));
        let early_clone = early_called.clone();
        host.register(
            AdapterId::new("early"),
            rx_early,
            move || Box::pin(async move { early_clone.store(true, Ordering::SeqCst); Ok(()) }),
        ).unwrap();

        // Adapter "alive" stays open
        let (_tx_alive, rx_alive) = mpsc::channel(1);
        let alive_called = Arc::new(AtomicBool::new(false));
        let alive_clone = alive_called.clone();
        host.register(
            AdapterId::new("alive"),
            rx_alive,
            move || Box::pin(async move { alive_clone.store(true, Ordering::SeqCst); Ok(()) }),
        ).unwrap();

        // Drain the early close event
        let ev = host.next_event().await;
        assert!(matches!(ev, Some(AdapterHostEvent::AdapterClosed(id)) if id.as_str() == "early"));

        // shutdown_all should still work for both
        host.shutdown_all().await;
        assert!(early_called.load(Ordering::SeqCst));
        assert!(alive_called.load(Ordering::SeqCst));
    }
}
```

- [ ] **Step 3: Add module declaration to main.rs**

Add at the top of `iotkit-gateway/src/main.rs` (before the existing `use` statements):

```rust
mod adapter_host;
```

This is needed BEFORE running tests so that the test file is actually compiled.

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p iotkit-gateway`
Expected: FAIL — `AdapterHost`, `AdapterHostEvent`, etc. not defined (the module exists but types are not yet implemented)

- [ ] **Step 5: Implement AdapterHost**

Add the implementation above the `#[cfg(test)]` block in `adapter_host.rs`:

```rust
/// Event yielded by [`AdapterHost::next_event`].
pub enum AdapterHostEvent {
    /// A data event from an adapter.
    Event(EngineEvent),
    /// An adapter's event stream has closed.
    AdapterClosed(AdapterId),
}

/// Unified fan-in and lifecycle manager for adapters.
pub struct AdapterHost {
    streams: StreamMap<AdapterId, WrappedStream>,
    adapters: Vec<ManagedAdapter>,
}

struct ManagedAdapter {
    id: AdapterId,
    shutdown_fn: Option<
        Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send>,
    >,
}

impl AdapterHost {
    pub fn new() -> Self {
        Self {
            streams: StreamMap::new(),
            adapters: Vec::new(),
        }
    }

    /// Register an adapter. Returns `Err` if the adapter ID is already registered.
    pub fn register(
        &mut self,
        id: AdapterId,
        event_rx: mpsc::Receiver<AdapterEvent>,
        shutdown_fn: impl FnOnce() -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
            + Send
            + 'static,
    ) -> Result<(), String> {
        if self.streams.contains_key(&id) {
            return Err(format!("duplicate adapter ID: {id}"));
        }
        let stream = WrappedStream {
            inner: ReceiverStream::new(event_rx),
            closed_yielded: false,
        };
        self.streams.insert(id.clone(), stream);
        self.adapters.push(ManagedAdapter {
            id,
            shutdown_fn: Some(Box::new(shutdown_fn)),
        });
        Ok(())
    }

    /// Returns the next event from any registered adapter, or `None` if all
    /// adapters have closed.
    pub async fn next_event(&mut self) -> Option<AdapterHostEvent> {
        let (id, item) = self.streams.next().await?;
        match item {
            WrappedItem::Event(event) => Some(AdapterHostEvent::Event(EngineEvent {
                adapter_id: id,
                event,
            })),
            WrappedItem::Closed => Some(AdapterHostEvent::AdapterClosed(id)),
        }
    }

    /// Shut down all adapters in reverse registration order.
    ///
    /// For each adapter: removes its stream (closing the receiver), then
    /// invokes its shutdown closure, before moving to the next adapter.
    /// Errors are logged, not propagated.
    pub async fn shutdown_all(&mut self) {
        // Per-adapter: remove stream then shutdown, in reverse order.
        for adapter in self.adapters.iter_mut().rev() {
            // Remove stream → drops ReceiverStream → closes receiver
            self.streams.remove(&adapter.id);
            // Invoke shutdown closure (Shutdown cmd → task/thread join)
            if let Some(shutdown_fn) = adapter.shutdown_fn.take() {
                if let Err(e) = shutdown_fn().await {
                    tracing::error!(
                        adapter = %adapter.id, error = %e,
                        "Adapter shutdown error"
                    );
                }
            }
        }
    }
}

// ── WrappedStream ────────────────────────────────────────

enum WrappedItem {
    Event(AdapterEvent),
    Closed,
}

/// Wraps a `ReceiverStream<AdapterEvent>` to yield a `Closed` sentinel
/// when the inner stream ends, before the stream itself terminates.
///
/// This ensures `StreamMap` delivers a notification for each adapter that
/// closes, rather than silently removing finished streams.
struct WrappedStream {
    inner: ReceiverStream<AdapterEvent>,
    closed_yielded: bool,
}

impl Stream for WrappedStream {
    type Item = WrappedItem;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.closed_yielded {
            return Poll::Ready(None);
        }
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(event)) => Poll::Ready(Some(WrappedItem::Event(event))),
            Poll::Ready(None) => {
                this.closed_yielded = true;
                Poll::Ready(Some(WrappedItem::Closed))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p iotkit-gateway`
Expected: All 8 tests pass

- [ ] **Step 7: Commit**

```bash
git add iotkit-gateway/Cargo.toml iotkit-gateway/src/adapter_host.rs iotkit-gateway/src/main.rs
git commit -m "feat(gateway): add AdapterHost with StreamMap fan-in and WrappedStream"
```

---

### Task 4: Refactor gateway main.rs to use AdapterHost

**Files:**
- Modify: `iotkit-gateway/src/main.rs`

- [ ] **Step 1: Rewrite run() to use AdapterHost**

Replace the entire `run()` function in `iotkit-gateway/src/main.rs`:

```rust
use adapter_host::{AdapterHost, AdapterHostEvent};

async fn run(port_path: String) {
    let engine = Engine::new();
    let mut host = AdapterHost::new();

    // BravePI mainboard adapter — required: start failure is fatal.
    let bravepi = match bravepi_mainboard_adapter::task::start(port_path) {
        Ok(h) => {
            tracing::info!(adapter_id = %h.id, "BravePI mainboard adapter started");
            h
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to start BravePI mainboard adapter");
            std::process::exit(1);
        }
    };
    let bravepi_parts = bravepi.into_parts();
    host.register(
        bravepi_parts.id,
        bravepi_parts.event_rx,
        {
            let sh = bravepi_parts.shutdown;
            move || Box::pin(async move { sh.shutdown().await })
        },
    )
    .expect("duplicate adapter ID");

    // RPi local adapter — optional: disabled by default, enable with RPI_LOCAL_ENABLED=1.
    let rpi_local_enabled = std::env::var("RPI_LOCAL_ENABLED")
        .map(|v| v == "1")
        .unwrap_or(false);

    if rpi_local_enabled {
        match rpi_local_adapter::start(rpi_local_config()) {
            Ok(rpi) => {
                tracing::info!(adapter_id = %rpi.id, "RPi local adapter started");
                let rpi_parts = rpi.into_parts();
                host.register(
                    rpi_parts.id,
                    rpi_parts.event_rx,
                    {
                        let sh = rpi_parts.shutdown;
                        move || Box::pin(async move { sh.shutdown().await })
                    },
                )
                .expect("duplicate adapter ID");
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "Failed to start RPi local adapter (enabled but failed)"
                );
                std::process::exit(1);
            }
        }
    } else {
        tracing::info!("RPi local adapter disabled (set RPI_LOCAL_ENABLED=1 to enable)");
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
                        tracing::debug!(
                            adapter = %ev.adapter_id,
                            event = ?ev.event,
                            "Adapter event"
                        );
                        engine.apply(ev).await;
                    }
                    Some(AdapterHostEvent::AdapterClosed(id)) => {
                        tracing::warn!(
                            adapter = %id,
                            "Adapter channel closed unexpectedly"
                        );
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

- [ ] **Step 2: Remove unused imports**

The `use iotkit_core_engine::EngineEvent` import in main.rs is no longer needed directly (AdapterHost creates EngineEvent internally). Remove it if unused. Keep `Engine` import.

Update the top of main.rs:

```rust
//! iotkit-gateway: composition root。
//! adapter を起動し、core/engine に event を渡す。

mod adapter_host;

use adapter_host::{AdapterHost, AdapterHostEvent};
use iotkit_core_engine::Engine;
use tracing_subscriber::EnvFilter;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo build -p iotkit-gateway`
Expected: Compiles without errors

- [ ] **Step 4: Run all tests**

Run: `cargo test --workspace`
Expected: All tests pass (adapter_host tests + existing tests)

- [ ] **Step 5: Commit**

```bash
git add iotkit-gateway/src/main.rs
git commit -m "refactor(gateway): replace manual select! fan-in with AdapterHost"
```

---

### Task 5: Verify rpi-local-adapter re-export

**Files:**
- Modify: `rpi-local-adapter/src/lib.rs` (if needed)

- [ ] **Step 1: Check if AdapterParts and ShutdownHandle are re-exported**

Currently `rpi-local-adapter/src/lib.rs` line 7:
```rust
pub use iotkit_polling_adapter_runtime::AdapterHandle;
```

The gateway calls `rpi_local_adapter::start()` which returns `AdapterHandle`, then calls `.into_parts()`. Since `AdapterParts` and `ShutdownHandle` are returned by `into_parts()`, they need to be accessible. They are already public in `iotkit_polling_adapter_runtime`, so the return type works without additional re-exports.

Verify by checking compilation succeeded in Task 4. If additional re-exports are needed, add:

```rust
pub use iotkit_polling_adapter_runtime::{AdapterHandle, AdapterParts, ShutdownHandle};
```

- [ ] **Step 2: Run rpi-local-adapter tests**

Run: `cargo test -p rpi-local-adapter`
Expected: All existing tests pass

- [ ] **Step 3: Commit (only if changes were needed)**

```bash
git add rpi-local-adapter/src/lib.rs
git commit -m "fix(rpi-local-adapter): re-export AdapterParts and ShutdownHandle"
```

---

### Task 6: Final workspace verification

- [ ] **Step 1: Run full workspace tests**

Run: `cargo test --workspace`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Commit any clippy fixes**

If clippy reports issues, fix and commit:
```bash
git commit -m "fix: address clippy warnings"
```
