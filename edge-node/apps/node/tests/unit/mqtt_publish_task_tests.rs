use super::*;
use iotkit_core_publish::activation::{
    ActivationRequest, ActivationResult, ActivationState, activation_state,
};

fn test_db() -> DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    all.sort_by_key(|migration| migration.version);
    iotkit_core_storage::init_db_memory(&all).unwrap()
}

fn seed_annotation(conn: &Connection, edge_node_id: &str) -> (String, PreparedBatch) {
    conn.execute(
        "INSERT INTO ledger_meta(key, value) VALUES('edge_node_id', ?1)",
        [edge_node_id],
    )
    .unwrap();
    ensure_target(conn, "mqtt://broker:1883").unwrap();
    let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
    let request = ActivationRequest {
        schema_version: 1,
        activation_id: "act-0123456789abcdef0123456789abcdef".into(),
        edge_id: "edge-0123456789abcdef0123456789abcdef".into(),
        edge_node_id: edge_node_id.into(),
        expected_ledger_epoch: epoch.clone(),
        grant_revision: 1,
        issued_at: 1,
    };
    apply_activation(conn, &request, 2).unwrap();
    iotkit_core_publish::store::enqueue_annotation(
        conn,
        &epoch,
        "epoch_start",
        r#"{"prior_epoch":"old-epoch"}"#,
        1,
    )
    .unwrap();
    let prepared = prepare_batch(conn, edge_node_id).unwrap().unwrap();
    (epoch, prepared)
}

#[test]
fn prepares_versioned_contiguous_batch_for_edge_node_topic() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let (_epoch, prepared) = seed_annotation(conn, "edge-01");
        assert_eq!(prepared.prior_cursor, 0);
        assert_eq!(prepared.batch.cursor_start, 1);
        assert_eq!(prepared.batch.cursor_end, 1);
        assert_eq!(prepared.batch.edge_node_id, "edge-01");
        let binding = MqttBinding::for_edge_node("edge-01").unwrap();
        assert_eq!(
            binding.records_topic,
            "iotkit/v1/edge-nodes/edge-01/records"
        );
        assert_eq!(
            binding.accepted_through_topic,
            "iotkit/v1/edge-nodes/edge-01/accepted-through"
        );
        assert_eq!(binding.client_id, "iotkit-edge-node-edge-01");
        prepared.batch.validate().unwrap();
        Ok(())
    })
    .unwrap();
}

#[test]
fn mqtt_client_accepts_the_wire_batch_limit_plus_protocol_overhead() {
    let binding = MqttBinding::for_edge_node("edge-01").unwrap();
    let records_topic = binding.records_topic;
    let status_topic = binding.status_topic;
    let ack_topic = binding.accepted_through_topic;
    let descriptor_topic = binding.descriptor_topic;
    let mut options = MqttOptions::new("test-client", "localhost", 1883);

    configure_packet_limits(
        &mut options,
        &records_topic,
        &ack_topic,
        &descriptor_topic,
        &status_topic,
    );

    assert!(options.max_packet_size() > MAX_BATCH_BYTES);
    assert_eq!(
        options.max_packet_size(),
        mqtt_packet_limit(&records_topic, &ack_topic, &descriptor_topic, &status_topic)
    );
}

#[test]
fn status_heartbeat_maps_only_bounded_operational_health_and_separates_custody() {
    let mut health = iotkit_edge_node::health::HealthState::new(7);
    health.collector_alive = false;
    health.db.watermark_exceeded = true;
    health.note_adapter_running("running-adapter");
    health.note_adapter_restarting("restarting-adapter");
    health.note_adapter_exhausted("exhausted-adapter");
    health.note_adapter_closed("stopped-adapter");
    health
        .publish
        .push(iotkit_edge_node::health::TargetDeliveryHealth {
            target_id: "edge".into(),
            cursor_pub_seq: 999,
            backlog: 777,
            last_push_at: Some(1),
            last_error: Some("must-not-leave-the-host".into()),
        });

    let heartbeat = build_status_heartbeat(
        &health,
        "edge-01".into(),
        "epoch-01".into(),
        "boot-0123456789abcdef0123456789abcdef",
        4,
        42,
        3,
    );

    assert_eq!(heartbeat.collector_state, CollectorState::Stopped);
    assert_eq!(heartbeat.accepted_through, 42);
    assert_eq!(heartbeat.pending_publications, 3);
    assert!(heartbeat.storage_pressure);
    assert_eq!(
        heartbeat
            .adapters
            .iter()
            .map(|adapter| (adapter.adapter_id.as_str(), adapter.state))
            .collect::<Vec<_>>(),
        vec![
            ("running-adapter", AdapterState::Running),
            ("restarting-adapter", AdapterState::Restarting),
            ("exhausted-adapter", AdapterState::Exhausted),
            ("stopped-adapter", AdapterState::Stopped),
        ]
    );
    let encoded = serde_json::to_string(&heartbeat).unwrap();
    assert!(!encoded.contains("must-not-leave-the-host"));
    assert!(!encoded.contains("last_error"));
    heartbeat.validate().unwrap();
}

#[test]
fn status_heartbeat_refuses_to_hide_an_unbounded_adapter_set() {
    let mut health = iotkit_edge_node::health::HealthState::new(7);
    for number in 0..65 {
        health.note_adapter_running(&format!("adapter-{number}"));
    }
    let heartbeat = build_status_heartbeat(
        &health,
        "edge-01".into(),
        "epoch-01".into(),
        "boot-0123456789abcdef0123456789abcdef",
        1,
        0,
        0,
    );
    assert!(heartbeat.validate().is_err());
}

#[tokio::test]
async fn failed_status_enqueue_does_not_consume_the_sequence() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        seed_annotation(conn, "edge-01");
        Ok(())
    })
    .unwrap();
    let health = Arc::new(Mutex::new(iotkit_edge_node::health::HealthState::new(7)));
    let binding = MqttBinding::for_edge_node("edge-01").unwrap();
    let (client, event_loop) =
        AsyncClient::new(MqttOptions::new("test-client", "localhost", 1883), 1);
    drop(event_loop);
    let mut sequence = 7;

    assert!(
        publish_status_heartbeat(
            &db,
            &health,
            &client,
            &binding,
            "boot-0123456789abcdef0123456789abcdef",
            &mut sequence,
        )
        .await
        .is_err()
    );
    assert_eq!(sequence, 7);
}

#[test]
fn descriptor_preparation_is_revision_aware_and_reconnect_safe() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        conn.execute(
            "INSERT INTO ledger_meta(key, value) VALUES
                    ('edge_node_id', 'edge-01'), ('epoch', 'epoch-01')",
            [],
        )
        .unwrap();
        let first = prepare_descriptor(conn, "edge-01", None).unwrap().unwrap();
        assert_eq!(first.identity.ledger_epoch, "epoch-01");
        assert!(
            prepare_descriptor(conn, "edge-01", Some(&first.identity))
                .unwrap()
                .is_none()
        );

        conn.execute(
            "INSERT INTO devices (system_id, hardware_id, kind, state, created_at)
                 VALUES (?1, 'test-device', 'individual', 'active', 1)",
            [vec![1_u8; 16]],
        )
        .unwrap();
        let changed = prepare_descriptor(conn, "edge-01", Some(&first.identity))
            .unwrap()
            .unwrap();
        assert!(changed.identity.descriptor_revision > first.identity.descriptor_revision);

        let reconnect = prepare_descriptor(conn, "edge-01", None).unwrap().unwrap();
        assert_eq!(reconnect.identity, changed.identity);
        Ok(())
    })
    .unwrap();
}

#[test]
fn application_ack_advances_cursor_but_mismatch_does_not() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let (epoch, prepared) = seed_annotation(conn, "edge-01");
        let mut wrong = AcceptedThrough {
            schema_version: EGRESS_SCHEMA_VERSION,
            edge_node_id: "edge-other".to_string(),
            ledger_epoch: epoch.clone(),
            publication_id: prepared.batch.publication_id.clone(),
            accepted_through: prepared.batch.cursor_end,
        };
        assert!(
            wrong
                .validate_for(&prepared.batch, prepared.prior_cursor)
                .is_err()
        );
        assert_eq!(
            effective_cursor(&epoch, &target_get(conn).unwrap().unwrap()),
            0
        );

        wrong.edge_node_id = "edge-01".to_string();
        wrong
            .validate_for(&prepared.batch, prepared.prior_cursor)
            .unwrap();
        apply_ack(
            conn,
            &epoch,
            prepared.prior_cursor,
            prepared.batch.cursor_end,
        )
        .unwrap();
        assert_eq!(
            effective_cursor(&epoch, &target_get(conn).unwrap().unwrap()),
            1
        );
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn delayed_prior_ack_keeps_the_next_batch_inflight() {
    let db = test_db();
    let (epoch, first) = db
        .with_conn_sync(|conn| {
            let (epoch, first) = seed_annotation(conn, "edge-01");
            let second = iotkit_core_publish::store::enqueue_commissioning_smoke(
                conn,
                &epoch,
                "smoke-0123456789abcdef0123456789abcdea",
                2,
            )
            .unwrap();
            assert_eq!(second, 2);
            Ok((epoch, first))
        })
        .unwrap();
    let binding = MqttBinding::for_edge_node("edge-01").unwrap();
    let prior_ack = AcceptedThrough {
        schema_version: EGRESS_SCHEMA_VERSION,
        edge_node_id: "edge-01".into(),
        ledger_epoch: epoch.clone(),
        publication_id: first.batch.publication_id.clone(),
        accepted_through: first.batch.cursor_end,
    };
    let prior_payload = serde_json::to_vec(&prior_ack).unwrap();
    let mut inflight = Some(first);

    assert!(
        handle_ack(
            &db,
            &binding.accepted_through_topic,
            &binding.accepted_through_topic,
            &prior_payload,
            &mut inflight,
        )
        .await
        .unwrap()
    );
    assert!(inflight.is_none());

    let next = db
        .with_conn_sync(|conn| Ok(prepare_batch(conn, "edge-01").unwrap().unwrap()))
        .unwrap();
    assert_eq!(next.prior_cursor, 1);
    assert_eq!(next.batch.cursor_start, 2);
    let mut inflight = Some(next);

    assert!(
        !handle_ack(
            &db,
            &binding.accepted_through_topic,
            &binding.accepted_through_topic,
            &prior_payload,
            &mut inflight,
        )
        .await
        .unwrap()
    );
    let current = inflight.as_ref().unwrap();
    assert_eq!(current.prior_cursor, 1);
    assert_eq!(current.batch.cursor_start, 2);

    let malformed = AcceptedThrough {
        schema_version: EGRESS_SCHEMA_VERSION,
        edge_node_id: "edge-01".into(),
        ledger_epoch: epoch.clone(),
        publication_id: "edge-01:epoch:bad".into(),
        accepted_through: 1,
    };
    assert!(
        handle_ack(
            &db,
            &binding.accepted_through_topic,
            &binding.accepted_through_topic,
            &serde_json::to_vec(&malformed).unwrap(),
            &mut inflight,
        )
        .await
        .is_err()
    );
    assert_eq!(inflight.as_ref().unwrap().batch.cursor_start, 2);
    assert_eq!(
        db.with_conn_sync(|conn| {
            Ok(effective_cursor(
                &epoch,
                &target_get(conn).unwrap().unwrap(),
            ))
        })
        .unwrap(),
        1,
    );
}

#[tokio::test]
async fn delayed_prior_prefix_ack_advances_only_the_proven_prefix() {
    let db = test_db();
    let (epoch, prior) = db
        .with_conn_sync(|conn| Ok(seed_annotation(conn, "edge-01")))
        .unwrap();
    let binding = MqttBinding::for_edge_node("edge-01").unwrap();
    let prefix_ack = AcceptedThrough {
        schema_version: EGRESS_SCHEMA_VERSION,
        edge_node_id: "edge-01".into(),
        ledger_epoch: epoch.clone(),
        publication_id: prior.batch.publication_id.clone(),
        accepted_through: prior.batch.cursor_end,
    };
    let widened = db
        .with_conn_sync(|conn| {
            let next = iotkit_core_publish::store::enqueue_commissioning_smoke(
                conn,
                &epoch,
                "smoke-0123456789abcdef0123456789abcdea",
                2,
            )
            .unwrap();
            assert_eq!(next, 2);
            Ok(prepare_batch(conn, "edge-01").unwrap().unwrap())
        })
        .unwrap();
    assert_eq!(widened.prior_cursor, 0);
    assert_eq!(widened.batch.cursor_start, 1);
    assert_eq!(widened.batch.cursor_end, 2);
    let mut inflight = Some(widened);

    assert!(
        handle_ack(
            &db,
            &binding.accepted_through_topic,
            &binding.accepted_through_topic,
            &serde_json::to_vec(&prefix_ack).unwrap(),
            &mut inflight,
        )
        .await
        .unwrap()
    );
    assert!(inflight.is_none());
    assert_eq!(
        db.with_conn_sync(|conn| {
            Ok(effective_cursor(
                &epoch,
                &target_get(conn).unwrap().unwrap(),
            ))
        })
        .unwrap(),
        1,
    );

    let remainder = db
        .with_conn_sync(|conn| Ok(prepare_batch(conn, "edge-01").unwrap().unwrap()))
        .unwrap();
    assert_eq!(remainder.prior_cursor, 1);
    assert_eq!(remainder.batch.cursor_start, 2);
    assert_eq!(remainder.batch.cursor_end, 2);
    assert_eq!(remainder.batch.records.len(), 1);
    assert_eq!(
        remainder.batch.records[0]["family"],
        serde_json::json!("commissioning_smoke"),
    );
}

#[test]
fn existing_legacy_target_is_not_silently_rewritten() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        iotkit_core_publish::store::target_insert(
            conn,
            &TargetRow {
                target_id: "legacy".to_string(),
                endpoint_url: "https://legacy.invalid/push".to_string(),
                credential_token: "secret".to_string(),
                archive_responsible: true,
                schema_version: 1,
                cursor_epoch: None,
                cursor_pub_seq: 0,
            },
            1,
        )
        .unwrap();
        assert!(ensure_target(conn, "mqtt://broker:1883").is_err());
        let target = target_get(conn).unwrap().unwrap();
        assert_eq!(target.target_id, "legacy");
        assert_eq!(target.endpoint_url, "https://legacy.invalid/push");
        Ok(())
    })
    .unwrap();
}

#[test]
fn new_mqtt_target_enters_discovery_only_and_subscribes_to_both_control_topics() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        conn.execute(
            "INSERT INTO ledger_meta(key, value) VALUES
                    ('edge_node_id', 'edge-01'), ('epoch', 'epoch-01')",
            [],
        )
        .unwrap();

        ensure_target(conn, "mqtt://broker:1883").unwrap();

        assert_eq!(
            activation_state(conn).unwrap(),
            ActivationState::DiscoveryOnly
        );
        assert!(prepare_batch(conn, "edge-01").unwrap().is_none());
        let binding = MqttBinding::for_edge_node("edge-01").unwrap();
        let filters = subscription_filters(&binding, QoS::AtLeastOnce);
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].path, binding.accepted_through_topic);
        assert_eq!(filters[1].path, binding.activation_request_topic);
        assert!(filters.iter().all(|filter| filter.qos == QoS::AtLeastOnce));
        Ok(())
    })
    .unwrap();
}

#[test]
fn activation_request_is_durable_idempotent_and_opens_pub_seq_one() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        conn.execute(
            "INSERT INTO ledger_meta(key, value) VALUES
                    ('edge_node_id', 'edge-01'), ('epoch', 'epoch-01')",
            [],
        )
        .unwrap();
        ensure_target(conn, "mqtt://broker:1883").unwrap();
        conn.execute(
            "INSERT INTO devices(system_id, hardware_id, kind, state, created_at)
                 VALUES(zeroblob(16), 'preactivation-device', 'individual', 'active', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO series(system_id, measurement_key, created_at)
                 VALUES(zeroblob(16), 'temperature_c', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO readings(
                    series_id, received_at, time_source, values_json
                 ) VALUES(1, 1, 'edge', '[19.5]')",
            [],
        )
        .unwrap();
        let binding = MqttBinding::for_edge_node("edge-01").unwrap();
        let request = ActivationRequest {
            schema_version: 1,
            activation_id: "act-0123456789abcdef0123456789abcdef".into(),
            edge_id: "edge-0123456789abcdef0123456789abcdef".into(),
            edge_node_id: "edge-01".into(),
            expected_ledger_epoch: "epoch-01".into(),
            grant_revision: 1,
            issued_at: 10,
        };
        let payload = serde_json::to_vec(&request).unwrap();

        let first = apply_activation_request(
            conn,
            &binding.activation_request_topic,
            &binding.activation_request_topic,
            &payload,
            20,
        )
        .unwrap();
        let duplicate = apply_activation_request(
            conn,
            &binding.activation_request_topic,
            &binding.activation_request_topic,
            &payload,
            99,
        )
        .unwrap();

        assert_eq!(duplicate, first);
        let result = ActivationResult::decode(&first).unwrap();
        assert_eq!(result.applied_at, 20);
        assert_eq!(result.discard_through_reading_seq, 1);
        assert_eq!(result.first_publication_seq, 1);
        assert_eq!(activation_state(conn).unwrap(), ActivationState::Active);
        assert_eq!(
            conn.query_row("SELECT count(*) FROM readings", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0,
            "activation handling should start bounded prefix cleanup immediately"
        );
        assert!(
            apply_activation_request(
                conn,
                &binding.activation_request_topic,
                "iotkit/v1/edge-nodes/edge-other/activation/request",
                &payload,
                100,
            )
            .is_err()
        );

        let pub_seq = iotkit_core_publish::store::enqueue_annotation(
            conn,
            "epoch-01",
            "epoch_start",
            r#"{"prior_epoch":"old-epoch"}"#,
            21,
        )
        .unwrap()
        .unwrap();
        assert_eq!(pub_seq, 1);
        assert_eq!(
            prepare_batch(conn, "edge-01")
                .unwrap()
                .unwrap()
                .batch
                .cursor_start,
            1
        );
        Ok(())
    })
    .unwrap();
}
