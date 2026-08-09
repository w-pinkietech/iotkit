use super::*;
use iotkit_edge_custody_contract::{AcceptedThrough, SCHEMA_VERSION};
use rumqttc::Request;

#[test]
fn ingest_health_is_closed_and_keeps_only_the_last_ready_time() {
    let health = IngestHealth::default();
    assert_eq!(
        health.snapshot(),
        IngestRuntimeHealth {
            state: IngestConnectionState::Unknown,
            last_ready_at: None,
        }
    );

    health.connecting();
    assert_eq!(health.snapshot().state, IngestConnectionState::Connecting);
    health.ready(42);
    assert_eq!(
        health.snapshot(),
        IngestRuntimeHealth {
            state: IngestConnectionState::Ready,
            last_ready_at: Some(42),
        }
    );
    health.disconnected();
    assert_eq!(
        health.snapshot(),
        IngestRuntimeHealth {
            state: IngestConnectionState::Disconnected,
            last_ready_at: Some(42),
        }
    );
    assert!(
        !serde_json::to_string(&health.snapshot())
            .unwrap()
            .contains("error")
    );
}

#[test]
fn ingest_becomes_ready_only_after_one_complete_qos1_suback() {
    assert!(subscriptions_confirmed(&vec![
        SubscribeReasonCode::Success(
            QoS::AtLeastOnce
        );
        SUBSCRIPTIONS.len()
    ]));
    assert!(!subscriptions_confirmed(&vec![
        SubscribeReasonCode::Success(
            QoS::AtLeastOnce
        );
        SUBSCRIPTIONS.len() - 1
    ]));
    let mut rejected = vec![SubscribeReasonCode::Success(QoS::AtLeastOnce); SUBSCRIPTIONS.len()];
    rejected[2] = SubscribeReasonCode::Failure;
    assert!(!subscriptions_confirmed(&rejected));
    let mut wrong_qos = vec![SubscribeReasonCode::Success(QoS::AtLeastOnce); SUBSCRIPTIONS.len()];
    wrong_qos[4] = SubscribeReasonCode::Success(QoS::AtMostOnce);
    assert!(!subscriptions_confirmed(&wrong_qos));
}

#[test]
fn ingest_enqueues_one_subscription_packet_for_the_complete_contract() {
    let (client, mut event_loop) = AsyncClient::new(MqttOptions::new("test", "localhost", 1883), 1);
    assert!(try_subscribe_all(&client));
    event_loop.clean();
    let subscriptions = event_loop
        .pending
        .iter()
        .filter_map(|request| match request {
            Request::Subscribe(subscribe) => Some(subscribe),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(subscriptions.len(), 1);
    assert_eq!(subscriptions[0].filters.len(), SUBSCRIPTIONS.len());
    assert_eq!(
        subscriptions[0]
            .filters
            .iter()
            .map(|filter| filter.path.as_str())
            .collect::<Vec<_>>(),
        SUBSCRIPTIONS
    );
    assert!(
        subscriptions[0]
            .filters
            .iter()
            .all(|filter| filter.qos == QoS::AtLeastOnce)
    );
}

#[test]
fn mqtt_packet_limit_covers_the_largest_custody_payload_and_topic() {
    let mut options = MqttOptions::new("test", "localhost", 1883);

    configure_packet_limits(&mut options);

    assert!(
        options.max_packet_size()
            >= MAX_BATCH_BYTES + MAX_MQTT_TOPIC_BYTES + MQTT_PACKET_OVERHEAD_BYTES
    );
}

#[test]
fn mqtt_ingest_runtime_retries_newer_pending_ack_after_a_stale_replay() {
    let (client, mut event_loop) = AsyncClient::new(MqttOptions::new("test", "localhost", 1883), 1);
    client
        .try_publish(
            "iotkit/v1/edge-nodes/edge-node-01/descriptors",
            QoS::AtLeastOnce,
            false,
            b"fill the bounded request queue".as_slice(),
        )
        .expect("fill request queue");
    let acknowledgement = acknowledgement_for("epoch-01", "edge-node-01:epoch-01:2:2", 2);
    let stale_acknowledgement = acknowledgement_for("epoch-01", "edge-node-01:epoch-01:1:1", 1);
    let mut pending = PendingCustodyAcks::default();

    assert!(
        pending
            .try_enqueue(&client, acknowledgement.clone())
            .is_err()
    );
    assert_eq!(
        pending.by_topic.len(),
        1,
        "failed acknowledgement is retained"
    );
    assert!(pending.retry(&client).is_err());
    assert_eq!(
        pending.by_topic.len(),
        1,
        "a repeated retry failure does not add a duplicate acknowledgement"
    );
    assert!(pending.try_enqueue(&client, stale_acknowledgement).is_ok());
    let retained = pending
        .by_topic
        .get(&acknowledgement.topic)
        .expect("current acknowledgement remains pending");
    assert_eq!(
        retained.accepted.accepted_through, 2,
        "a stale replay cannot replace the current acknowledgement"
    );
    assert!(
        pending
            .try_enqueue(&client, acknowledgement.clone())
            .is_ok()
    );
    assert_eq!(
        pending.by_topic.len(),
        1,
        "same-topic exact replay coalesces while the queue remains full"
    );

    event_loop.clean();
    pending
        .retry(&client)
        .expect("retry once the request queue has capacity");
    assert!(
        pending.by_topic.is_empty(),
        "successfully enqueued retry is no longer pending"
    );

    event_loop.clean();
    let published = event_loop
        .pending
        .iter()
        .filter_map(|request| match request {
            Request::Publish(publish) if publish.topic == acknowledgement.topic => Some(publish),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        published.len(),
        1,
        "only one coalesced acknowledgement is queued"
    );
    assert_eq!(published[0].qos, QoS::AtLeastOnce);
    assert!(!published[0].retain);
    assert_eq!(
        published[0].payload.as_ref(),
        acknowledgement.payload.as_slice()
    );
}

#[test]
fn mqtt_ingest_runtime_rejects_a_pending_acknowledgement_for_a_different_epoch() {
    let (client, _event_loop) = AsyncClient::new(MqttOptions::new("test", "localhost", 1883), 1);
    client
        .try_publish(
            "iotkit/v1/edge-nodes/edge-node-01/descriptors",
            QoS::AtLeastOnce,
            false,
            b"fill the bounded request queue".as_slice(),
        )
        .expect("fill request queue");
    let prior_epoch = acknowledgement_for("epoch-01", "edge-node-01:epoch-01:2:2", 2);
    let current_epoch = acknowledgement_for("epoch-02", "edge-node-01:epoch-02:1:1", 1);
    let mut pending = PendingCustodyAcks::default();

    assert!(pending.try_enqueue(&client, prior_epoch.clone()).is_err());
    assert!(matches!(
        pending.try_enqueue(&client, current_epoch),
        Err(RuntimeError::PendingAcknowledgementCorrelation)
    ));
    assert_eq!(
        pending.by_topic.len(),
        1,
        "a different epoch cannot expand the per-topic pending acknowledgement bound"
    );
    assert_eq!(
        pending
            .by_topic
            .get(&prior_epoch.topic)
            .expect("prior acknowledgement remains pending")
            .accepted
            .ledger_epoch,
        "epoch-01",
        "a different epoch cannot overwrite the retained acknowledgement"
    );
}

#[test]
fn mqtt_ingest_runtime_rejects_a_pending_acknowledgement_with_conflicting_publication_id() {
    let (client, _event_loop) = AsyncClient::new(MqttOptions::new("test", "localhost", 1883), 1);
    client
        .try_publish(
            "iotkit/v1/edge-nodes/edge-node-01/descriptors",
            QoS::AtLeastOnce,
            false,
            b"fill the bounded request queue".as_slice(),
        )
        .expect("fill request queue");
    let acknowledgement = acknowledgement_for("epoch-01", "edge-node-01:epoch-01:1:1", 1);
    let conflicting = acknowledgement_for("epoch-01", "edge-node-01:epoch-01:1:2", 1);
    let mut pending = PendingCustodyAcks::default();

    assert!(
        pending
            .try_enqueue(&client, acknowledgement.clone())
            .is_err()
    );
    assert!(matches!(
        pending.try_enqueue(&client, conflicting),
        Err(RuntimeError::PendingAcknowledgementCorrelation)
    ));
    assert_eq!(
        pending.by_topic.len(),
        1,
        "a conflicting publication ID cannot add another pending acknowledgement"
    );
    assert_eq!(
        pending
            .by_topic
            .get(&acknowledgement.topic)
            .expect("original acknowledgement remains pending")
            .accepted
            .publication_id,
        "edge-node-01:epoch-01:1:1"
    );
}

fn acknowledgement_for(
    ledger_epoch: &str,
    publication_id: &str,
    accepted_through: i64,
) -> AckPublication {
    AckPublication {
        topic: "iotkit/v1/edge-nodes/edge-node-01/accepted-through".into(),
        retain: false,
        payload: serde_json::to_vec(&AcceptedThrough {
            schema_version: SCHEMA_VERSION,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: ledger_epoch.into(),
            publication_id: publication_id.into(),
            accepted_through,
        })
        .expect("encode acknowledgement"),
    }
}
