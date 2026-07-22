use super::*;

#[test]
fn enqueue_and_select_batch_is_ordered_and_exclusive() {
    let conn = crate::tests_support::open();
    let e = "epoch-A";
    let s1 = enqueue_measurement(&conn, e, 100, 1).unwrap();
    let s2 = enqueue_measurement(&conn, e, 101, 2).unwrap();
    let _s3 = enqueue_measurement(&conn, e, 102, 3).unwrap();
    assert!(s2 > s1);
    let batch = select_batch(&conn, e, s1, 10).unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].pub_seq, s2);
    assert_eq!(batch[0].reading_seq, Some(101));
}

#[test]
fn enqueue_annotation_idempotent_on_epoch_subtype() {
    let conn = crate::tests_support::open();
    let a = enqueue_annotation(&conn, "epoch-A", "epoch_start", "{}", 1).unwrap();
    assert!(a.is_some());
    let b = enqueue_annotation(&conn, "epoch-A", "epoch_start", "{}", 2).unwrap();
    assert!(b.is_none(), "二重 enqueue は UNIQUE で None");
}

#[test]
fn enqueue_commissioning_smoke_keeps_test_identity_in_the_outbox() {
    let conn = crate::tests_support::open();

    let pub_seq = enqueue_commissioning_smoke(
        &conn,
        "epoch-A",
        "smoke-0123456789abcdef0123456789abcdef",
        42,
    )
    .unwrap();

    let rows = select_batch(&conn, "epoch-A", 0, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].pub_seq, pub_seq);
    assert_eq!(rows[0].kind, "commissioning_smoke");
    assert_eq!(rows[0].subtype, None);
    assert_eq!(rows[0].reading_seq, None);
    assert_eq!(
        rows[0].annotation_json.as_deref(),
        Some(r#"{"test_id":"smoke-0123456789abcdef0123456789abcdef"}"#)
    );
}

#[test]
fn enqueue_commissioning_smoke_rejects_invalid_test_identity_without_writing() {
    let conn = crate::tests_support::open();
    for test_id in [
        "0123456789abcdef0123456789abcdef",
        "smoke-short",
        "smoke-0123456789ABCDEF0123456789ABCDEF",
    ] {
        assert!(enqueue_commissioning_smoke(&conn, "epoch-A", test_id, 42).is_err());
    }
    assert!(select_batch(&conn, "epoch-A", 0, 10).unwrap().is_empty());
}

#[test]
fn discovery_only_rejects_every_direct_publication_enqueue() {
    let conn = crate::tests_support::open();
    crate::activation::install_edge_target(
        &conn,
        &TargetRow {
            target_id: "edge".into(),
            endpoint_url: "mqtts://broker.example.test:8883".into(),
            credential_token: String::new(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: None,
            cursor_pub_seq: 0,
        },
        1,
    )
    .unwrap();

    assert!(enqueue_measurement(&conn, "epoch-A", 1, 2).is_err());
    assert!(enqueue_annotation(&conn, "epoch-A", "epoch_start", "{}", 2).is_err());
    assert!(
        enqueue_commissioning_smoke(
            &conn,
            "epoch-A",
            "smoke-0123456789abcdef0123456789abcdef",
            2
        )
        .is_err()
    );
    assert!(select_batch(&conn, "epoch-A", 0, 10).unwrap().is_empty());
}

#[test]
fn prune_outbox_by_reading_seqs_removes_only_matching_measurements() {
    let conn = crate::tests_support::open();
    let e = "epoch-A";
    enqueue_measurement(&conn, e, 200, 1).unwrap();
    let keep = enqueue_measurement(&conn, e, 201, 2).unwrap();
    enqueue_annotation(&conn, e, "epoch_start", "{}", 3).unwrap();

    assert_eq!(prune_outbox_by_reading_seqs(&conn, &[200, 999]).unwrap(), 1);

    let batch = select_batch(&conn, e, 0, 10).unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].pub_seq, keep);
    assert_eq!(batch[0].reading_seq, Some(201));
    assert_eq!(batch[1].kind, "annotation");
}

#[test]
fn prune_outbox_by_reading_seqs_empty_slice_deletes_zero_rows() {
    let conn = crate::tests_support::open();

    assert_eq!(prune_outbox_by_reading_seqs(&conn, &[]).unwrap(), 0);
}

#[test]
fn prune_outbox_for_quarantined_range_removes_only_quarantined_readings_in_window() {
    let conn = crate::tests_support::open();
    let e = "epoch-A";
    let system_id = vec![1_u8; 16];
    conn.execute(
        "INSERT INTO devices (system_id, hardware_id, kind, state, created_at)
             VALUES (?1, 'hw:test', 'individual', 'active', 1)",
        params![&system_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO series
                (series_id, system_id, measurement_key, channel_index, variant, created_at)
             VALUES
                (10, ?1, 'temperature', -1, 'primary', 1),
                (20, ?1, 'humidity', -1, 'primary', 1)",
        params![&system_id],
    )
    .unwrap();
    let matching = iotkit_core_timeseries::insert_reading_v3(
        &conn,
        &iotkit_core_timeseries::NewReading {
            series_id: 10,
            received_at_ms: 1_200,
            device_time_ms: None,
            time_source: "edge_node".into(),
            values: vec![1.0],
            rssi: None,
            battery_pct: None,
            quarantined: false,
        },
    )
    .unwrap();
    let outside_range = iotkit_core_timeseries::insert_reading_v3(
        &conn,
        &iotkit_core_timeseries::NewReading {
            series_id: 10,
            received_at_ms: 2_200,
            device_time_ms: None,
            time_source: "edge_node".into(),
            values: vec![2.0],
            rssi: None,
            battery_pct: None,
            quarantined: false,
        },
    )
    .unwrap();
    let other_series = iotkit_core_timeseries::insert_reading_v3(
        &conn,
        &iotkit_core_timeseries::NewReading {
            series_id: 20,
            received_at_ms: 1_300,
            device_time_ms: None,
            time_source: "edge_node".into(),
            values: vec![3.0],
            rssi: None,
            battery_pct: None,
            quarantined: false,
        },
    )
    .unwrap();
    enqueue_measurement(&conn, e, matching, 1).unwrap();
    let keep_outside_range = enqueue_measurement(&conn, e, outside_range, 2).unwrap();
    let keep_other_series = enqueue_measurement(&conn, e, other_series, 3).unwrap();
    conn.execute(
        "UPDATE readings SET quarantined = 1
             WHERE series_id = ?1 AND received_at BETWEEN ?2 AND ?3",
        params![10, 1_000, 2_000],
    )
    .unwrap();

    assert_eq!(
        prune_outbox_for_quarantined_range(&conn, &[10], 1_000, 2_000).unwrap(),
        1
    );

    let batch = select_batch(&conn, e, 0, 10).unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].pub_seq, keep_outside_range);
    assert_eq!(batch[0].reading_seq, Some(outside_range));
    assert_eq!(batch[1].pub_seq, keep_other_series);
    assert_eq!(batch[1].reading_seq, Some(other_series));
}

#[test]
fn prune_acked_outbox_removes_up_to_cursor_in_epoch_only() {
    let conn = crate::tests_support::open();
    let s1 = enqueue_measurement(&conn, "E", 300, 1).unwrap();
    let s2 = enqueue_measurement(&conn, "E", 301, 2).unwrap();
    enqueue_measurement(&conn, "OTHER", 302, 3).unwrap();

    assert_eq!(prune_acked_outbox(&conn, "E", s1).unwrap(), 1);

    let current = select_batch(&conn, "E", 0, 10).unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].pub_seq, s2);
    assert_eq!(select_batch(&conn, "OTHER", 0, 10).unwrap().len(), 1);
}

#[test]
fn target_crud_and_cursor_update_round_trip() {
    let conn = crate::tests_support::open();
    assert_eq!(target_count(&conn).unwrap(), 0);
    let t = TargetRow {
        target_id: "target-1".into(),
        endpoint_url: "https://example.test/push".into(),
        credential_token: "token-a".into(),
        archive_responsible: true,
        schema_version: 1,
        cursor_epoch: None,
        cursor_pub_seq: 0,
    };

    target_insert(&conn, &t, 10).unwrap();
    assert_eq!(target_count(&conn).unwrap(), 1);
    assert!(archive_target_registered(&conn).unwrap());

    target_set_token(&conn, "target-1", "token-b").unwrap();
    target_set_archive_responsible(&conn, "target-1", false).unwrap();
    target_advance_cursor(&conn, "target-1", "epoch-B", 42).unwrap();
    let got = target_get(&conn).unwrap().unwrap();
    assert_eq!(got.target_id, "target-1");
    assert_eq!(got.credential_token, "token-b");
    assert!(!got.archive_responsible);
    assert_eq!(got.cursor_epoch.as_deref(), Some("epoch-B"));
    assert_eq!(got.cursor_pub_seq, 42);
    assert!(!archive_target_registered(&conn).unwrap());

    target_delete(&conn, "target-1").unwrap();
    assert_eq!(target_get(&conn).unwrap().map(|t| t.target_id), None);
}

#[test]
fn has_unacked_true_when_cursor_behind_same_epoch() {
    let conn = crate::tests_support::open();
    let s = enqueue_measurement(&conn, "E", 500, 1).unwrap();
    let t = TargetRow {
        target_id: "t".into(),
        endpoint_url: "https://x".into(),
        credential_token: "k".into(),
        archive_responsible: true,
        schema_version: 1,
        cursor_epoch: Some("E".into()),
        cursor_pub_seq: s - 1,
    };
    assert!(has_unacked_pubseq_rows(&conn, "E", &t, &[500]).unwrap());
}

#[test]
fn has_unacked_false_when_cursor_epoch_mismatch_means_effective_zero_but_no_current_epoch_rows() {
    let conn = crate::tests_support::open();
    enqueue_measurement(&conn, "OLD", 500, 1).unwrap();
    let t = TargetRow {
        target_id: "t".into(),
        endpoint_url: "https://x".into(),
        credential_token: "k".into(),
        archive_responsible: true,
        schema_version: 1,
        cursor_epoch: Some("OLD".into()),
        cursor_pub_seq: 9999,
    };
    assert!(!has_unacked_pubseq_rows(&conn, "NEW", &t, &[500]).unwrap());
}

#[test]
fn has_unacked_pubseq_rows_empty_reading_seqs_returns_false() {
    let conn = crate::tests_support::open();
    let t = TargetRow {
        target_id: "t".into(),
        endpoint_url: "https://x".into(),
        credential_token: "k".into(),
        archive_responsible: true,
        schema_version: 1,
        cursor_epoch: Some("E".into()),
        cursor_pub_seq: 0,
    };

    assert!(!has_unacked_pubseq_rows(&conn, "E", &t, &[]).unwrap());
}

#[test]
fn any_unacked_for_target_uses_effective_cursor_for_current_epoch() {
    let conn = crate::tests_support::open();
    let s1 = enqueue_measurement(&conn, "E", 600, 1).unwrap();
    let _s2 = enqueue_measurement(&conn, "E", 601, 2).unwrap();
    let current = TargetRow {
        target_id: "t".into(),
        endpoint_url: "https://x".into(),
        credential_token: "k".into(),
        archive_responsible: true,
        schema_version: 1,
        cursor_epoch: Some("E".into()),
        cursor_pub_seq: s1,
    };
    assert!(any_unacked_for_target(&conn, "E", &current).unwrap());

    let all_acked = TargetRow {
        cursor_pub_seq: i64::MAX,
        ..current.clone()
    };
    assert!(!any_unacked_for_target(&conn, "E", &all_acked).unwrap());

    let old_epoch_cursor = TargetRow {
        cursor_epoch: Some("OLD".into()),
        cursor_pub_seq: i64::MAX,
        ..current
    };
    assert!(any_unacked_for_target(&conn, "E", &old_epoch_cursor).unwrap());
}

#[test]
fn outbox_backlog_count_uses_effective_cursor_for_current_epoch() {
    let conn = crate::tests_support::open();
    let s1 = enqueue_measurement(&conn, "E", 700, 1).unwrap();
    let s2 = enqueue_measurement(&conn, "E", 701, 2).unwrap();
    enqueue_measurement(&conn, "OTHER", 702, 3).unwrap();
    let current = TargetRow {
        target_id: "t".into(),
        endpoint_url: "https://x".into(),
        credential_token: "k".into(),
        archive_responsible: true,
        schema_version: 1,
        cursor_epoch: Some("E".into()),
        cursor_pub_seq: s1,
    };

    assert_eq!(outbox_backlog_count(&conn, "E", &current).unwrap(), 1);

    let all_acked = TargetRow {
        cursor_pub_seq: s2,
        ..current.clone()
    };
    assert_eq!(outbox_backlog_count(&conn, "E", &all_acked).unwrap(), 0);

    let old_epoch_cursor = TargetRow {
        cursor_epoch: Some("OLD".into()),
        cursor_pub_seq: i64::MAX,
        ..current
    };
    assert_eq!(
        outbox_backlog_count(&conn, "E", &old_epoch_cursor).unwrap(),
        2
    );
}
