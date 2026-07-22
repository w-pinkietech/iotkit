#[cfg(not(test))]
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_core_publish::store::{
    outbox_backlog_count, select_batch, target_advance_cursor, target_get,
};
use iotkit_core_storage::{DbHandle, StorageError};
use rusqlite::Connection;
use serde_json::Value;

use iotkit_edge_node::health::{HealthState, TargetDeliveryHealth, now_ms};

#[cfg(test)]
use tests::delivery_endpoint_url;

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
    let endpoint_url = delivery_endpoint_url(&prepared.endpoint_url);

    let client = reqwest::Client::new();
    let ack = tokio::time::timeout(POST_TIMEOUT, async {
        let response = client
            .post(endpoint_url.as_ref())
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

#[cfg(not(test))]
fn delivery_endpoint_url(endpoint_url: &str) -> Cow<'_, str> {
    Cow::Borrowed(endpoint_url)
}

pub(crate) fn spawn_publish_task(
    db: DbHandle,
    health: Arc<Mutex<HealthState>>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut delay = Duration::ZERO;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(delay) => {
                    let result = run_publish_cycle(&db).await;
                    match &result {
                        Ok(()) => {
                            delay = interval;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "publish task failed");
                            delay = next_backoff(delay, interval);
                        }
                    }
                    refresh_publish_health(&db, &health, &result).await;
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

pub(crate) async fn refresh_publish_health(
    db: &DbHandle,
    health: &Arc<Mutex<HealthState>>,
    cycle: &Result<(), String>,
) {
    let read = db
        .with_conn(|conn| {
            Ok::<_, StorageError>((|| -> Result<Option<(String, i64, i64)>, String> {
                let current_epoch =
                    iotkit_core_ledger::ledger_epoch(conn).map_err(|e| e.to_string())?;
                let Some(target) = target_get(conn).map_err(|e| e.to_string())? else {
                    return Ok(None);
                };
                let backlog = outbox_backlog_count(conn, &current_epoch, &target)
                    .map_err(|e| e.to_string())?;
                Ok(Some((target.target_id, target.cursor_pub_seq, backlog)))
            })())
        })
        .await;

    let target = match read {
        Ok(Ok(target)) => target,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "publish health refresh failed");
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "publish health refresh failed");
            return;
        }
    };

    let mut state = health.lock().expect("health state mutex poisoned");
    let Some((target_id, cursor_pub_seq, backlog)) = target else {
        state.publish = Vec::new();
        return;
    };

    let previous_last_push_at = state
        .publish
        .iter()
        .find(|entry| entry.target_id == target_id)
        .and_then(|entry| entry.last_push_at);
    let last_push_at = if cycle.is_ok() {
        Some(now_ms())
    } else {
        previous_last_push_at
    };
    let last_error = match cycle {
        Ok(()) => None,
        Err(e) => Some(e.clone()),
    };
    state.publish = vec![TargetDeliveryHealth {
        target_id,
        cursor_pub_seq,
        backlog,
        last_push_at,
        last_error,
    }];
}

fn prepare_batch(conn: &Connection) -> Result<Option<PreparedBatch>, String> {
    let current_epoch = iotkit_core_ledger::ledger_epoch(conn).map_err(|e| e.to_string())?;
    let Some(target) = target_get(conn).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    if !target.archive_responsible {
        return Ok(None);
    }
    if !target.endpoint_url.starts_with("https://") {
        return Err(format!(
            "refusing to deliver to non-HTTPS endpoint: {}",
            target.endpoint_url
        ));
    }
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

    let mut records = iotkit_edge_node::record::materialize_batch(conn, &rows)?;
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
    if ack.acked_pub_seq != cursor_end {
        return Err(format!(
            "publish ack acked_pub_seq {} does not match batch cursor_end {}",
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
#[path = "../tests/unit/publish_task_tests.rs"]
mod tests;
