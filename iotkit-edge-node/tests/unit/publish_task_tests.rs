use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use iotkit_core_publish::store::{
    TargetRow, target_advance_cursor, target_get, target_insert, target_set_archive_responsible,
};
use iotkit_core_timeseries::NewReading;
use rusqlite::params;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

static TEST_ID: AtomicUsize = AtomicUsize::new(1);

struct TestConsumer {
    endpoint_url: String,
    received: tokio::task::JoinHandle<ReceivedRequest>,
}

struct TestConsumerBatch {
    endpoint_url: String,
    received: tokio::task::JoinHandle<Vec<ReceivedRequest>>,
}

struct ReceivedRequest {
    authorization: Option<String>,
    publication_id: String,
    records: Vec<Value>,
}

#[derive(Clone, Copy)]
enum AckMode {
    EchoMax,
    EchoLow,
    WrongPublicationId,
}

async fn spawn_consumer(mode: AckMode) -> TestConsumer {
    let batch = spawn_consumers(mode, 1).await;
    let endpoint_url = batch.endpoint_url;
    let received = tokio::spawn(async move {
        let mut requests = batch.received.await.unwrap();
        assert_eq!(requests.len(), 1);
        requests.remove(0)
    });

    TestConsumer {
        endpoint_url,
        received,
    }
}

async fn spawn_consumers(mode: AckMode, request_count: usize) -> TestConsumerBatch {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint_url = format!("https://{}", listener.local_addr().unwrap());
    let received = tokio::spawn(async move {
        let mut requests = Vec::with_capacity(request_count);
        for _ in 0..request_count {
            let (mut stream, _) = listener.accept().await.unwrap();
            requests.push(read_request_and_ack(&mut stream, mode).await);
        }
        requests
    });

    TestConsumerBatch {
        endpoint_url,
        received,
    }
}

async fn read_request_and_ack(
    stream: &mut tokio::net::TcpStream,
    mode: AckMode,
) -> ReceivedRequest {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0, "client closed before headers");
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
    };

    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let content_length = content_length(&headers);
    let authorization = header_value(&headers, "authorization");
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0, "client closed before body");
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);

    let request_json: Value = serde_json::from_slice(&body).unwrap();
    let publication_id = request_json
        .get("publication_id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    let records = request_json
        .get("records")
        .and_then(|v| v.as_array())
        .unwrap()
        .clone();
    let max_pub_seq = records
        .iter()
        .filter_map(|r| r.get("pub_seq").and_then(|v| v.as_i64()))
        .max()
        .unwrap_or(0);
    let ack_publication_id = match mode {
        AckMode::EchoMax => publication_id.clone(),
        AckMode::EchoLow => publication_id.clone(),
        AckMode::WrongPublicationId => format!("{publication_id}:wrong"),
    };
    let acked_pub_seq = match mode {
        AckMode::EchoMax | AckMode::WrongPublicationId => max_pub_seq,
        AckMode::EchoLow => max_pub_seq - 1,
    };
    let ack = serde_json::json!({
        "publication_id": ack_publication_id,
        "acked_pub_seq": acked_pub_seq,
    });
    let ack_body = serde_json::to_vec(&ack).unwrap();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        ack_body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    stream.write_all(&ack_body).await.unwrap();

    ReceivedRequest {
        authorization,
        publication_id,
        records,
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn content_length(headers: &str) -> usize {
    header_value(headers, "content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .expect("content-length header")
}

fn header_value(headers: &str, name: &str) -> Option<String> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.eq_ignore_ascii_case(name) {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

fn test_db() -> iotkit_core_storage::DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    iotkit_core_storage::init_db_memory(&all).unwrap()
}

fn seed_target_and_measurements(
    db: &iotkit_core_storage::DbHandle,
    endpoint_url: String,
    cursor_epoch: Option<String>,
    cursor_pub_seq: i64,
    values: Vec<Vec<f64>>,
) -> (String, Vec<i64>) {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    db.with_conn_sync(move |conn| {
        let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
        let sid = iotkit_core_ledger::insert_device(
            conn,
            &iotkit_core_ledger::NewDevice {
                hardware_id: format!("hw:publish-task:{id}"),
                user_label: None,
                parent: None,
                kind: iotkit_core_ledger::DeviceKind::Individual,
                initial_state: iotkit_core_ledger::DeviceState::Active,
            },
        )
        .unwrap();
        let series_id = iotkit_core_ledger::ensure_series(
            conn,
            &sid,
            "temperature_c",
            iotkit_core_ledger::CHANNEL_NA,
            iotkit_core_ledger::DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();
        let mut pub_seqs = Vec::new();
        for (offset, sample) in values.into_iter().enumerate() {
            let reading_seq = iotkit_core_timeseries::insert_reading_v3(
                conn,
                &NewReading {
                    series_id,
                    received_at_ms: 1_000 + offset as i64,
                    device_time_ms: None,
                    time_source: "edge_node".into(),
                    values: sample,
                    rssi: None,
                    battery_pct: None,
                    quarantined: false,
                },
            )
            .unwrap();
            pub_seqs.push(
                iotkit_core_publish::store::enqueue_measurement(
                    conn,
                    &epoch,
                    reading_seq,
                    2_000 + offset as i64,
                )
                .unwrap(),
            );
        }
        target_insert(
            conn,
            &TargetRow {
                target_id: "target-1".into(),
                endpoint_url,
                credential_token: "token-1".into(),
                archive_responsible: true,
                schema_version: 1,
                cursor_epoch,
                cursor_pub_seq,
            },
            3_000,
        )
        .unwrap();
        Ok((epoch, pub_seqs))
    })
    .unwrap()
}

fn target_cursor(db: &iotkit_core_storage::DbHandle) -> (Option<String>, i64) {
    db.with_conn_sync(|conn| {
        let target = target_get(conn).unwrap().unwrap();
        Ok((target.cursor_epoch, target.cursor_pub_seq))
    })
    .unwrap()
}

fn count_rows(db: &iotkit_core_storage::DbHandle, table: &str) -> i64 {
    db.with_conn_sync(|conn| {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        Ok(conn
            .query_row(&sql, [], |row| row.get::<_, i64>(0))
            .unwrap())
    })
    .unwrap()
}

fn orphan_outbox_count(db: &iotkit_core_storage::DbHandle) -> i64 {
    db.with_conn_sync(|conn| {
        Ok(conn
            .query_row(
                "SELECT COUNT(*)
                     FROM publication_log p
                     LEFT JOIN readings r ON p.reading_seq = r.seq
                     WHERE p.kind = 'measurement'
                       AND p.reading_seq IS NOT NULL
                       AND r.seq IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap())
    })
    .unwrap()
}

#[tokio::test]
async fn push_cycle_delivers_batch_and_advances_cursor() {
    let consumer = spawn_consumer(AckMode::EchoMax).await;
    let db = test_db();
    let (epoch, pub_seqs) = seed_target_and_measurements(
        &db,
        consumer.endpoint_url,
        None,
        0,
        vec![vec![21.5], vec![22.0]],
    );

    super::run_publish_cycle(&db).await.unwrap();

    let received = consumer.received.await.unwrap();
    let expected_publication_id = format!(
        "target-1:{}:{}:{}",
        epoch,
        pub_seqs[0],
        pub_seqs[pub_seqs.len() - 1]
    );
    assert_eq!(received.publication_id, expected_publication_id);
    assert_eq!(received.authorization.as_deref(), Some("Bearer token-1"));
    assert_eq!(received.records.len(), 2);
    assert_eq!(
        received.records[0].get("pub_seq").and_then(|v| v.as_i64()),
        Some(pub_seqs[0])
    );
    assert_eq!(target_cursor(&db), (Some(epoch), pub_seqs[1]));
}

#[tokio::test]
async fn byte_cap_single_oversized_record_still_delivers_one() {
    let consumer = spawn_consumer(AckMode::EchoMax).await;
    let db = test_db();
    let oversized_values = vec![1.234567; 160_000];
    let (epoch, pub_seqs) =
        seed_target_and_measurements(&db, consumer.endpoint_url, None, 0, vec![oversized_values]);

    super::run_publish_cycle(&db).await.unwrap();

    let received = consumer.received.await.unwrap();
    assert_eq!(received.records.len(), 1);
    let serialized = serde_json::to_string(&received.records[0]).unwrap();
    assert!(serialized.len() > 1024 * 1024);
    assert_eq!(target_cursor(&db), (Some(epoch), pub_seqs[0]));
}

#[tokio::test]
async fn byte_cap_truncates_mid_batch_and_advances_to_last_included() {
    let consumer = spawn_consumer(AckMode::EchoMax).await;
    let db = test_db();
    let modest_large_values = vec![1.234567; 34_000];
    let (epoch, pub_seqs) = seed_target_and_measurements(
        &db,
        consumer.endpoint_url,
        None,
        0,
        vec![
            modest_large_values.clone(),
            modest_large_values.clone(),
            modest_large_values.clone(),
            modest_large_values.clone(),
            modest_large_values,
        ],
    );
    let prepared = db
        .with_conn_sync(|conn| Ok(super::prepare_batch(conn).unwrap().unwrap()))
        .unwrap();
    assert_eq!(prepared.records.len(), 3);
    assert_eq!(prepared.cursor_end, pub_seqs[2]);

    super::run_publish_cycle(&db).await.unwrap();

    let received = consumer.received.await.unwrap();
    assert_eq!(received.records.len(), 3);
    assert_eq!(
        received.records[0].get("pub_seq").and_then(|v| v.as_i64()),
        Some(pub_seqs[0])
    );
    assert_eq!(
        received.records[2].get("pub_seq").and_then(|v| v.as_i64()),
        Some(pub_seqs[2])
    );
    assert_eq!(target_cursor(&db), (Some(epoch), pub_seqs[2]));
}

#[tokio::test]
async fn ack_validation_failure_does_not_advance_cursor() {
    let consumer = spawn_consumer(AckMode::WrongPublicationId).await;
    let db = test_db();
    seed_target_and_measurements(
        &db,
        consumer.endpoint_url,
        None,
        0,
        vec![vec![21.5], vec![22.0]],
    );

    let err = super::run_publish_cycle(&db).await.unwrap_err();

    let _received = consumer.received.await.unwrap();
    assert!(err.contains("ack"), "unexpected error: {err}");
    assert_eq!(target_cursor(&db), (None, 0));
}

#[tokio::test]
async fn ack_with_low_acked_pub_seq_does_not_advance() {
    let consumer = spawn_consumer(AckMode::EchoLow).await;
    let db = test_db();
    seed_target_and_measurements(
        &db,
        consumer.endpoint_url,
        None,
        0,
        vec![vec![21.5], vec![22.0]],
    );

    let err = super::run_publish_cycle(&db).await.unwrap_err();

    let _received = consumer.received.await.unwrap();
    assert!(err.contains("acked_pub_seq"), "unexpected error: {err}");
    assert_eq!(target_cursor(&db), (None, 0));
}

#[test]
fn ack_with_high_acked_pub_seq_does_not_advance() {
    let db = test_db();
    let (epoch, pub_seqs) = seed_target_and_measurements(
        &db,
        "https://archive.example/publish".into(),
        None,
        0,
        vec![vec![21.5], vec![22.0]],
    );
    let expected_publication_id = format!(
        "target-1:{}:{}:{}",
        epoch,
        pub_seqs[0],
        pub_seqs[pub_seqs.len() - 1]
    );
    let ack = super::AckResponse {
        publication_id: expected_publication_id.clone(),
        acked_pub_seq: pub_seqs[pub_seqs.len() - 1] + 1,
    };

    let err = db
        .with_conn_sync(|conn| {
            Ok(super::advance_cursor_after_ack(
                conn,
                "target-1",
                &epoch,
                pub_seqs[pub_seqs.len() - 1],
                &expected_publication_id,
                &ack,
            ))
        })
        .unwrap()
        .unwrap_err();

    assert!(
        err.contains("does not match batch cursor_end"),
        "unexpected error: {err}"
    );
    assert_eq!(target_cursor(&db), (None, 0));
}

#[tokio::test]
async fn push_skips_when_target_not_archive_responsible() {
    let db = test_db();
    seed_target_and_measurements(
        &db,
        "https://127.0.0.1:1".into(),
        None,
        0,
        vec![vec![21.5], vec![22.0]],
    );
    db.with_conn_sync(|conn| {
        target_set_archive_responsible(conn, "target-1", false).unwrap();
        Ok(())
    })
    .unwrap();

    super::run_publish_cycle(&db).await.unwrap();

    assert_eq!(target_cursor(&db), (None, 0));
}

#[tokio::test]
async fn push_refuses_non_https_endpoint() {
    let db = test_db();
    seed_target_and_measurements(
        &db,
        "http://127.0.0.1:1".into(),
        None,
        0,
        vec![vec![21.5], vec![22.0]],
    );

    let err = super::run_publish_cycle(&db).await.unwrap_err();

    assert!(
        err.contains("refusing to deliver to non-HTTPS endpoint: http://127.0.0.1:1"),
        "unexpected error: {err}"
    );
    assert_eq!(target_cursor(&db), (None, 0));
}

#[tokio::test]
async fn push_epoch_mismatch_redelivers_from_effective_cursor_zero() {
    let consumer = spawn_consumer(AckMode::EchoMax).await;
    let db = test_db();
    let (epoch, pub_seqs) = seed_target_and_measurements(
        &db,
        consumer.endpoint_url,
        Some("OLD".into()),
        i64::MAX,
        vec![vec![21.5], vec![22.0]],
    );
    assert_ne!(epoch, "OLD");

    super::run_publish_cycle(&db).await.unwrap();

    let received = consumer.received.await.unwrap();
    assert_eq!(received.records.len(), 2);
    assert_eq!(
        received.records[0].get("pub_seq").and_then(|v| v.as_i64()),
        Some(pub_seqs[0])
    );
    assert_eq!(target_cursor(&db), (Some(epoch), pub_seqs[1]));
}

#[test]
fn publication_id_cursor_start_is_prev_cursor_plus_one() {
    let db = test_db();
    let (epoch, pub_seqs) = seed_target_and_measurements(
        &db,
        "https://127.0.0.1:1".into(),
        None,
        0,
        vec![vec![21.5], vec![22.0], vec![22.5]],
    );
    db.with_conn_sync(|conn| {
        target_advance_cursor(conn, "target-1", &epoch, pub_seqs[0]).unwrap();
        conn.execute(
            "DELETE FROM publication_log WHERE pub_seq = ?1",
            params![pub_seqs[1]],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();

    let prepared = db
        .with_conn_sync(|conn| Ok(super::prepare_batch(conn).unwrap().unwrap()))
        .unwrap();
    assert_eq!(prepared.cursor_start, pub_seqs[0] + 1);
    assert_eq!(prepared.cursor_end, pub_seqs[2]);
    assert_eq!(
        prepared.records[0].get("pub_seq").and_then(|v| v.as_i64()),
        Some(pub_seqs[2])
    );
    assert_ne!(prepared.cursor_start, pub_seqs[2]);

    let publication_id = format!(
        "{}:{}:{}:{}",
        prepared.target_id, prepared.current_epoch, prepared.cursor_start, prepared.cursor_end
    );
    let cursor_start_segment = publication_id.split(':').nth(2).unwrap();
    assert_eq!(cursor_start_segment, (pub_seqs[0] + 1).to_string());
}

#[tokio::test]
async fn health_json_reports_per_target_delivery_state() {
    let db = test_db();
    let (epoch, pub_seqs) = seed_target_and_measurements(
        &db,
        "http://127.0.0.1:1".into(),
        Some("previous-epoch".into()),
        999,
        vec![vec![21.5], vec![22.0]],
    );
    let health = Arc::new(Mutex::new(iotkit_edge_node::health::HealthState::new(90)));
    let cycle = Ok(());

    super::refresh_publish_health(&db, &health, &cycle).await;

    {
        let snapshot = health.lock().unwrap();
        assert_eq!(snapshot.publish.len(), 1);
        assert_eq!(snapshot.publish[0].target_id, "target-1");
        assert_eq!(snapshot.publish[0].cursor_pub_seq, 999);
        assert_eq!(snapshot.publish[0].backlog, pub_seqs.len() as i64);
        assert!(snapshot.publish[0].last_push_at.is_some());
        assert_eq!(snapshot.publish[0].last_error, None);
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("health.json");
    let snapshot = health.lock().unwrap().clone();
    iotkit_edge_node::health::write_health_json(&path, &epoch, &snapshot).unwrap();
    let json = std::fs::read_to_string(path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["publish"][0]["target_id"], "target-1");
    assert_eq!(json["publish"][0]["cursor_pub_seq"], 999);
    assert_eq!(json["publish"][0]["backlog"], 2);
    assert!(json["publish"][0]["last_error"].is_null());
}

#[tokio::test]
async fn end_to_end_custody_loop() {
    let consumer = spawn_consumer(AckMode::EchoMax).await;
    let db = test_db();
    let (epoch, pub_seqs) = seed_target_and_measurements(
        &db,
        consumer.endpoint_url,
        None,
        0,
        vec![vec![21.5], vec![22.0]],
    );

    super::run_publish_cycle(&db).await.unwrap();

    let received = consumer.received.await.unwrap();
    assert_eq!(received.records.len(), 2);
    assert_eq!(target_cursor(&db), (Some(epoch.clone()), pub_seqs[1]));
    let survivor_pub_seq = db
        .with_conn_sync(|conn| {
            let series_id = conn
                .query_row(
                    "SELECT series_id FROM readings ORDER BY seq LIMIT 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap();
            let reading_seq = iotkit_core_timeseries::insert_reading_v3(
                conn,
                &NewReading {
                    series_id,
                    received_at_ms: 1_002,
                    device_time_ms: None,
                    time_source: "edge_node".into(),
                    values: vec![22.5],
                    rssi: None,
                    battery_pct: None,
                    quarantined: false,
                },
            )
            .unwrap();
            let pub_seq =
                iotkit_core_publish::store::enqueue_measurement(conn, &epoch, reading_seq, 2_002)
                    .unwrap();
            Ok(pub_seq)
        })
        .unwrap();
    assert!(survivor_pub_seq > pub_seqs[1]);

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let config = crate::retention::RetentionConfig {
        retention_days: 1,
        quarantine_ttl_days: 30,
        disk_high_watermark_pct: 101,
    };
    let health = Arc::new(Mutex::new(iotkit_edge_node::health::HealthState::new(
        config.retention_days,
    )));
    let mut latch = crate::retention::WatermarkLatch::default();

    crate::retention::run_retention_once_with_latch(&db, &db_path, config, health, &mut latch)
        .await
        .unwrap();

    assert_eq!(count_rows(&db, "readings"), 1);
    assert_eq!(count_rows(&db, "publication_log"), 1);
    assert_eq!(orphan_outbox_count(&db), 0);
}

#[tokio::test]
async fn crash_between_post_and_cursor_is_idempotent() {
    let consumer = spawn_consumers(AckMode::EchoMax, 2).await;
    let db = test_db();
    let (epoch, pub_seqs) = seed_target_and_measurements(
        &db,
        consumer.endpoint_url,
        None,
        0,
        vec![vec![21.5], vec![22.0]],
    );

    super::run_publish_cycle(&db).await.unwrap();
    db.with_conn_sync(|conn| {
        target_advance_cursor(conn, "target-1", &epoch, 0).unwrap();
        Ok(())
    })
    .unwrap();
    super::run_publish_cycle(&db).await.unwrap();

    let received = consumer.received.await.unwrap();
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].records.len(), 2);
    assert_eq!(received[1].records.len(), 2);
    assert_eq!(
        received[0].records[0]
            .get("pub_seq")
            .and_then(|v| v.as_i64()),
        Some(pub_seqs[0])
    );
    assert_eq!(received[1].publication_id, received[0].publication_id);
    assert_eq!(target_cursor(&db), (Some(epoch), pub_seqs[1]));
}
