//! AdapterHost: unified fan-in and lifecycle management for adapters.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt, StreamMap};

use iotkit_core_engine::EngineEvent;
use iotkit_core_supervision::AdapterEvent;
use iotkit_core_types::AdapterId;

const ADAPTER_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

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

type ShutdownFn =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send>;

struct ManagedAdapter {
    id: AdapterId,
    shutdown_fn: Option<ShutdownFn>,
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
        if self.streams.contains_key(&id) || self.adapters.iter().any(|a| a.id == id) {
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

    /// Closed済みアダプタを登録簿から除去し、同一IDでの再registerを可能にする。
    /// 戻り値: 除去したらtrue。streamsに残っていれば併せて除去する。
    pub fn deregister(&mut self, id: &AdapterId) -> bool {
        self.streams.remove(id);
        let before = self.adapters.len();
        self.adapters.retain(|a| a.id != *id);
        before != self.adapters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    pub fn contains(&self, id: &AdapterId) -> bool {
        self.streams.contains_key(id) || self.adapters.iter().any(|adapter| adapter.id == *id)
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
        self.shutdown_all_with_timeout(ADAPTER_SHUTDOWN_TIMEOUT)
            .await;
    }

    async fn shutdown_all_with_timeout(&mut self, timeout: std::time::Duration) {
        for adapter in self.adapters.iter_mut().rev() {
            self.streams.remove(&adapter.id);
            if let Some(shutdown_fn) = adapter.shutdown_fn.take() {
                match tokio::time::timeout(timeout, shutdown_fn()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        tracing::error!(
                            adapter = %adapter.id,
                            %error,
                            "Adapter shutdown error"
                        );
                    }
                    Err(_) => {
                        tracing::error!(
                            adapter = %adapter.id,
                            timeout_ms = timeout.as_millis() as u64,
                            "Adapter shutdown timed out"
                        );
                    }
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

#[cfg(test)]
#[path = "../tests/unit/adapter_host_tests.rs"]
mod tests;
