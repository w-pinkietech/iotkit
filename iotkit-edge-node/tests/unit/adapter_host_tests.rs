use super::*;
use iotkit_core_types::{DeviceKey, SensorReading, SensorType};

fn stub_event() -> AdapterEvent {
    AdapterEvent::SensorData {
        device_key: DeviceKey::new("test:0"),
        reading: SensorReading::empty(SensorType::Temperature),
        rssi: None,
        battery_pct: None,
        ingested_at: std::time::SystemTime::now(),
    }
}

#[tokio::test]
async fn single_adapter_events() {
    let mut host = AdapterHost::new();
    let (tx, rx) = mpsc::channel(16);
    host.register(AdapterId::new("a"), rx, || Box::pin(async { Ok(()) }))
        .unwrap();

    tx.send(stub_event()).await.unwrap();
    tx.send(stub_event()).await.unwrap();
    drop(tx);

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
    host.register(AdapterId::new("a"), rx_a, || Box::pin(async { Ok(()) }))
        .unwrap();
    host.register(AdapterId::new("b"), rx_b, || Box::pin(async { Ok(()) }))
        .unwrap();

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
    host.register(AdapterId::new("x"), rx, || Box::pin(async { Ok(()) }))
        .unwrap();
    drop(tx);

    let ev = host.next_event().await;
    assert!(matches!(ev, Some(AdapterHostEvent::AdapterClosed(id)) if id.as_str() == "x"));
    assert!(host.next_event().await.is_none());
}

#[tokio::test]
async fn all_closed_returns_none() {
    let mut host = AdapterHost::new();
    assert!(host.next_event().await.is_none());
}

#[tokio::test]
async fn duplicate_id_rejected() {
    let mut host = AdapterHost::new();
    let (_tx1, rx1) = mpsc::channel::<AdapterEvent>(1);
    let (_tx2, rx2) = mpsc::channel::<AdapterEvent>(1);
    host.register(AdapterId::new("dup"), rx1, || Box::pin(async { Ok(()) }))
        .unwrap();
    let result = host.register(AdapterId::new("dup"), rx2, || Box::pin(async { Ok(()) }));
    assert!(result.is_err());
}

#[tokio::test]
async fn shutdown_all_calls_closures() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut host = AdapterHost::new();
    let (tx, rx) = mpsc::channel(1);
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = called.clone();
    host.register(AdapterId::new("s"), rx, move || {
        Box::pin(async move {
            called_clone.store(true, Ordering::SeqCst);
            Ok(())
        })
    })
    .unwrap();
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
        host.register(AdapterId::new(name), rx, move || {
            Box::pin(async move {
                order_clone.lock().unwrap().push(name_owned);
                Ok(())
            })
        })
        .unwrap();
    }

    host.shutdown_all().await;
    let recorded = order.lock().unwrap().clone();
    assert_eq!(recorded, vec!["third", "second", "first"]);
}

#[tokio::test]
async fn shutdown_timeout_bounds_a_stuck_adapter() {
    let mut host = AdapterHost::new();
    let (_tx, rx) = mpsc::channel::<AdapterEvent>(1);
    host.register(AdapterId::new("stuck"), rx, || {
        Box::pin(std::future::pending())
    })
    .unwrap();

    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        host.shutdown_all_with_timeout(std::time::Duration::from_millis(1)),
    )
    .await
    .expect("host-owned shutdown deadline must complete");
}

#[tokio::test]
async fn shutdown_all_after_early_adapter_exit() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut host = AdapterHost::new();

    let (tx_early, rx_early) = mpsc::channel(1);
    drop(tx_early);
    let early_called = Arc::new(AtomicBool::new(false));
    let early_clone = early_called.clone();
    host.register(AdapterId::new("early"), rx_early, move || {
        Box::pin(async move {
            early_clone.store(true, Ordering::SeqCst);
            Ok(())
        })
    })
    .unwrap();

    let (_tx_alive, rx_alive) = mpsc::channel(1);
    let alive_called = Arc::new(AtomicBool::new(false));
    let alive_clone = alive_called.clone();
    host.register(AdapterId::new("alive"), rx_alive, move || {
        Box::pin(async move {
            alive_clone.store(true, Ordering::SeqCst);
            Ok(())
        })
    })
    .unwrap();

    let ev = host.next_event().await;
    assert!(matches!(ev, Some(AdapterHostEvent::AdapterClosed(id)) if id.as_str() == "early"));

    host.shutdown_all().await;
    assert!(early_called.load(Ordering::SeqCst));
    assert!(alive_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn duplicate_id_after_close_rejected() {
    let mut host = AdapterHost::new();
    let (tx, rx) = mpsc::channel::<AdapterEvent>(1);
    host.register(AdapterId::new("r"), rx, || Box::pin(async { Ok(()) }))
        .unwrap();

    // Adapter closes
    drop(tx);
    let ev = host.next_event().await;
    assert!(matches!(ev, Some(AdapterHostEvent::AdapterClosed(_))));

    // Re-registering the same ID should be rejected (shutdown closure still exists)
    let (_tx2, rx2) = mpsc::channel::<AdapterEvent>(1);
    let result = host.register(AdapterId::new("r"), rx2, || Box::pin(async { Ok(()) }));
    assert!(
        result.is_err(),
        "should reject duplicate ID even after adapter closed"
    );
}

#[tokio::test]
async fn deregister_allows_reregistration_of_same_id() {
    let mut host = AdapterHost::new();
    let (tx, rx) = mpsc::channel::<AdapterEvent>(4);
    host.register(AdapterId::new("a"), rx, || Box::pin(async { Ok(()) }))
        .unwrap();
    drop(tx); // チャネルを閉じる

    // AdapterClosedを消費
    while let Some(ev) = host.next_event().await {
        if matches!(ev, AdapterHostEvent::AdapterClosed(_)) {
            break;
        }
    }

    assert!(host.deregister(&AdapterId::new("a")));
    let (_tx2, rx2) = mpsc::channel::<AdapterEvent>(4);
    assert!(
        host.register(AdapterId::new("a"), rx2, || Box::pin(async { Ok(()) }))
            .is_ok()
    );
}

#[tokio::test]
async fn deregister_unknown_id_returns_false() {
    let mut host = AdapterHost::new();
    assert!(!host.deregister(&AdapterId::new("nonexistent")));
}

#[tokio::test]
async fn shutdown_all_unblocks_buffered_sender() {
    let mut host = AdapterHost::new();

    // Capacity-1 channel: fill it, then block a sender
    let (tx, rx) = mpsc::channel::<AdapterEvent>(1);
    tx.send(stub_event()).await.unwrap(); // fills the buffer

    host.register(AdapterId::new("buf"), rx, || Box::pin(async { Ok(()) }))
        .unwrap();

    // Spawn a sender that will block on the full channel
    let sender = tokio::spawn(async move {
        // This send will block until the receiver is dropped
        let _ = tx.send(stub_event()).await;
    });

    // Give the sender a moment to block
    tokio::task::yield_now().await;

    // shutdown_all removes the stream (drops receiver), which unblocks the sender
    host.shutdown_all().await;

    // The blocked sender task should complete without hanging
    tokio::time::timeout(std::time::Duration::from_secs(1), sender)
        .await
        .expect("sender should complete within timeout")
        .expect("sender task should not panic");
}
