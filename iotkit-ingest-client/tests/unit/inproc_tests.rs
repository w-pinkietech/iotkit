use super::*;
use iotkit_ingest_contract::TimeSource;

fn item(values: Vec<f64>) -> ReadingItem {
    ReadingItem {
        subject_hint: Some("hw".into()),
        measurement_key: "temperature_c".into(),
        channel_index: None,
        series_variant: None,
        values,
        device_time_ms: None,
        time_source: TimeSource::EdgeNode,
        age_ms: None,
        rssi: None,
        battery_pct: None,
    }
}

#[test]
fn new_envelope_drops_empty_value_items() {
    let envelope = new_envelope("test", vec![item(vec![]), item(vec![1.0])]);
    assert_eq!(envelope.items.len(), 1);
    assert_eq!(envelope.items[0].values, vec![1.0]);
}

#[tokio::test]
async fn try_submit_distinguishes_full_from_closed() {
    let (client, mut rx) = channel_for_test(1);
    client
        .try_submit(new_envelope("test", vec![item(vec![1.0])]))
        .unwrap();
    assert_eq!(
        client
            .try_submit(new_envelope("test", vec![item(vec![2.0])]))
            .unwrap_err(),
        IngestClientError::Full
    );
    rx.recv().await.expect("first item should remain queued");
    drop(rx);
    assert_eq!(
        client
            .try_submit(new_envelope("test", vec![item(vec![3.0])]))
            .unwrap_err(),
        IngestClientError::Closed
    );
}

#[tokio::test]
async fn receipt_submit_returns_same_envelope_as_retry_handle_when_full_or_closed() {
    let (client, mut rx) = channel_for_test(1);
    client
        .try_submit(new_envelope("test", vec![item(vec![1.0])]))
        .unwrap();

    let full_envelope = new_envelope("test", vec![item(vec![2.0])]);
    let full_id = full_envelope.envelope_id.clone();
    let QueueSubmitError::Full(full_retry) = client
        .try_submit_with_receipt(full_envelope)
        .expect_err("second item must see the bounded queue as full")
    else {
        panic!("expected full");
    };
    assert_eq!(full_retry.envelope_id(), full_id);
    assert_eq!(full_retry.source(), "test");

    rx.recv().await.expect("first item should remain queued");
    drop(rx);

    let closed_envelope = new_envelope("test", vec![item(vec![3.0])]);
    let closed_id = closed_envelope.envelope_id.clone();
    let QueueSubmitError::Closed(closed_retry) = client
        .try_submit_with_receipt(closed_envelope)
        .expect_err("closed queue must return retry ownership")
    else {
        panic!("expected closed");
    };
    assert_eq!(closed_retry.envelope_id(), closed_id);
}

#[test]
fn abandonment_closes_front_door_before_draining_receipts() {
    let (client, mut rx) = channel_for_test(1);
    let mut spool = VecDeque::new();

    abandon_all(&mut spool, &mut rx.rx, AbandonReason::CollectorClosed);

    let envelope = new_envelope("test", vec![item(vec![1.0])]);
    let envelope_id = envelope.envelope_id.clone();
    let QueueSubmitError::Closed(retry) = client
        .try_submit_with_receipt(envelope)
        .expect_err("abandonment must close admission before its final drain")
    else {
        panic!("expected closed");
    };
    assert_eq!(retry.envelope_id(), envelope_id);
}
