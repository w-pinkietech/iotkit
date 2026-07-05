use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_core_publish::store::{select_batch, target_advance_cursor, target_get};
use iotkit_core_storage::{DbHandle, StorageError};
use rusqlite::Connection;
use serde_json::Value;

use crate::health::HealthState;

const BATCH_LIMIT: u32 = 256;
const BYTE_CAP: usize = 1024 * 1024;
const POST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(5 * 60);

struct PreparedBatch {
    records: Vec<Value>,
    cursor_start: i64,
    cursor_end: i64,
    endpoint_url: String,
    credential_token: String,
    target_id: String,
    current_epoch: String,
}

#[derive(serde::Deserialize)]
struct AckResponse {
    publication_id: String,
    acked_pub_seq: i64,
}

pub(crate) async fn run_publish_cycle(db: &DbHandle) -> Result<(), String> {
    let prepared = db
        .with_conn(|conn| Ok::<_, StorageError>(prepare_batch(conn)))
        .await
        .map_err(|e| e.to_string())??;
    let Some(prepared) = prepared else {
        return Ok(());
    };

    let publication_id = format!(
        "{}:{}:{}:{}",
        prepared.target_id, prepared.current_epoch, prepared.cursor_start, prepared.cursor_end
    );
    let body = serde_json::json!({
        "publication_id": publication_id.clone(),
        "records": prepared.records,
    });

    let client = reqwest::Client::new();
    let ack = tokio::time::timeout(POST_TIMEOUT, async {
        let response = client
            .post(&prepared.endpoint_url)
            .bearer_auth(&prepared.credential_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("publish POST failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "publish POST returned non-success status: {}",
                response.status()
            ));
        }
        response
            .json::<AckResponse>()
            .await
            .map_err(|e| format!("publish ack decode failed: {e}"))
    })
    .await
    .map_err(|_| "publish POST timed out".to_string())??;

    let target_id = prepared.target_id;
    let current_epoch = prepared.current_epoch;
    let cursor_end = prepared.cursor_end;
    db.with_conn(move |conn| {
        Ok::<_, StorageError>(advance_cursor_after_ack(
            conn,
            &target_id,
            &current_epoch,
            cursor_end,
            &publication_id,
            &ack,
        ))
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(())
}

pub(crate) fn spawn_publish_task(
    db: DbHandle,
    _health: Arc<Mutex<HealthState>>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut delay = Duration::ZERO;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(delay) => {
                    match run_publish_cycle(&db).await {
                        Ok(()) => {
                            delay = interval;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "publish task failed");
                            delay = next_backoff(delay, interval);
                        }
                    }
                }
                shutdown = tokio::signal::ctrl_c() => {
                    if let Err(e) = shutdown {
                        tracing::warn!(error = %e, "publish task shutdown signal listener failed");
                    }
                    break;
                }
            }
        }
    })
}

fn prepare_batch(conn: &Connection) -> Result<Option<PreparedBatch>, String> {
    let current_epoch = iotkit_core_ledger::ledger_epoch(conn).map_err(|e| e.to_string())?;
    let Some(target) = target_get(conn).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let cursor = if target.cursor_epoch.as_deref() == Some(current_epoch.as_str()) {
        target.cursor_pub_seq
    } else {
        0
    };
    let rows =
        select_batch(conn, &current_epoch, cursor, BATCH_LIMIT).map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(None);
    }

    let mut records = crate::record::materialize_batch(conn, &rows)?;
    if records.is_empty() {
        return Err("materialized empty records for non-empty outbox batch".to_string());
    }
    let selected_count = selected_record_count(&records)?;
    records.truncate(selected_count);
    let cursor_end = rows
        .get(selected_count - 1)
        .map(|row| row.pub_seq)
        .ok_or_else(|| "selected batch cursor missing".to_string())?;

    Ok(Some(PreparedBatch {
        records,
        cursor_start: cursor + 1,
        cursor_end,
        endpoint_url: target.endpoint_url,
        credential_token: target.credential_token,
        target_id: target.target_id,
        current_epoch,
    }))
}

fn selected_record_count(records: &[Value]) -> Result<usize, String> {
    let mut total_bytes = 0usize;
    let mut selected = 0usize;
    for record in records {
        let len = serde_json::to_string(record)
            .map_err(|e| e.to_string())?
            .len();
        if selected > 0 && total_bytes.saturating_add(len) > BYTE_CAP {
            break;
        }
        total_bytes = total_bytes.saturating_add(len);
        selected += 1;
    }
    Ok(selected)
}

fn advance_cursor_after_ack(
    conn: &Connection,
    target_id: &str,
    current_epoch: &str,
    cursor_end: i64,
    expected_publication_id: &str,
    ack: &AckResponse,
) -> Result<(), String> {
    if ack.publication_id != expected_publication_id {
        return Err(format!(
            "publish ack publication_id mismatch: expected {expected_publication_id}, got {}",
            ack.publication_id
        ));
    }
    if ack.acked_pub_seq < cursor_end {
        return Err(format!(
            "publish acked_pub_seq {} is before cursor_end {}",
            ack.acked_pub_seq, cursor_end
        ));
    }

    let Some(target) = target_get(conn).map_err(|e| e.to_string())? else {
        return Err("publish target disappeared before cursor advance".to_string());
    };
    if target.target_id != target_id {
        return Err(format!(
            "publish target changed before cursor advance: expected {target_id}, got {}",
            target.target_id
        ));
    }
    if target.cursor_epoch.as_deref() == Some(current_epoch) && target.cursor_pub_seq > cursor_end {
        return Ok(());
    }

    target_advance_cursor(conn, target_id, current_epoch, cursor_end).map_err(|e| e.to_string())
}

fn next_backoff(current_delay: Duration, interval: Duration) -> Duration {
    let next = if current_delay.is_zero() {
        interval
    } else {
        current_delay.saturating_mul(2)
    };
    std::cmp::min(next, MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use iotkit_core_publish::store::{
        TargetRow, target_advance_cursor, target_get, target_insert,
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint_url = format!("http://{}", listener.local_addr().unwrap());
        let received = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
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
        });

        TestConsumer {
            endpoint_url,
            received,
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
                        time_source: "gateway".into(),
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
        let (epoch, pub_seqs) = seed_target_and_measurements(
            &db,
            consumer.endpoint_url,
            None,
            0,
            vec![oversized_values],
        );

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
        assert!(
            err.contains("acked_pub_seq"),
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
            "http://127.0.0.1:1".into(),
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
            prepared.target_id,
            prepared.current_epoch,
            prepared.cursor_start,
            prepared.cursor_end
        );
        let cursor_start_segment = publication_id.split(':').nth(2).unwrap();
        assert_eq!(cursor_start_segment, (pub_seqs[0] + 1).to_string());
    }
}
