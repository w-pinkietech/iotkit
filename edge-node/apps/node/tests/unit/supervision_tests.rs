use iotkit_core_types::AdapterId;
use std::time::Duration;

#[test]
fn backoff_grows_exponentially_with_cap_and_exhausts() {
    let policy = super::RestartPolicy {
        max_restarts: 3,
        base_backoff: Duration::from_secs(1),
        max_backoff: Duration::from_secs(4),
    };
    let mut t = super::RestartTracker::new(policy);
    let id = AdapterId::new("bravepi-mainboard:/dev/ttyAMA0");
    assert_eq!(t.next_delay(&id), Some(Duration::from_secs(1)));
    assert_eq!(t.next_delay(&id), Some(Duration::from_secs(2)));
    assert_eq!(t.next_delay(&id), Some(Duration::from_secs(4))); // cap
    assert_eq!(t.next_delay(&id), None); // exhausted → 永続degraded
}

#[test]
fn healthy_note_resets_counter() {
    let mut t = super::RestartTracker::new(super::RestartPolicy::default());
    let id = AdapterId::new("a");
    t.next_delay(&id);
    t.note_healthy(&id);
    assert_eq!(
        t.next_delay(&id),
        Some(super::RestartPolicy::default().base_backoff)
    );
}

#[tokio::test]
async fn restart_notification_is_delayed_without_blocking_caller() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (other_tx, mut other_rx) = tokio::sync::mpsc::unbounded_channel();
    let id = AdapterId::new("a");

    super::schedule_restart_notification(id.clone(), Duration::from_millis(100), tx);
    other_tx.send("processed").unwrap();

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), other_rx.recv())
            .await
            .expect("unrelated work should not be blocked by restart delay"),
        Some("processed")
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("notification should arrive")
            .expect("channel should stay open"),
        id
    );
}

#[test]
fn workspace_does_not_use_panic_abort() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .unwrap();
    let toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        !toml.contains("panic = \"abort\""),
        "panic=abort breaks task supervision (D1)"
    );
}
