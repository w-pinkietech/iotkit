use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_core_publish::store::{
    TargetRow, effective_cursor, select_batch, target_advance_cursor, target_get, target_insert,
};
use iotkit_core_publish::wire::{
    AcceptedThrough, EGRESS_SCHEMA_VERSION, MAX_BATCH_BYTES, MAX_BATCH_RECORDS, RecordBatch,
    publication_id,
};
use iotkit_core_storage::{DbHandle, StorageError};
use iotkit_edge::config::MqttExitConfig;
use iotkit_edge::health::HealthState;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS, Transport};
use rusqlite::Connection;

const TARGET_ID: &str = "site";
const RETRY_INTERVAL: Duration = Duration::from_secs(30);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MQTT_PACKET_OVERHEAD_BYTES: usize = 16;

struct RuntimeConfig {
    connection: MqttExitConfig,
    edge_node_id: String,
    password: String,
    ca: Option<Vec<u8>>,
}

struct PreparedBatch {
    batch: RecordBatch,
    prior_cursor: i64,
}

pub(crate) async fn spawn_mqtt_publish_task(
    db: DbHandle,
    health: Arc<Mutex<HealthState>>,
    config: MqttExitConfig,
) -> Result<tokio::task::JoinHandle<()>, String> {
    let password = read_password(&config)?;
    let ca = read_ca(&config)?;
    let expected_endpoint = endpoint(&config);
    let (edge_node_id, ()) = db
        .with_conn(move |conn| {
            let identity = iotkit_core_ledger::edge_node_id(conn)
                .map_err(|error| storage_error(error.to_string()))?;
            ensure_target(conn, &expected_endpoint).map_err(storage_error)?;
            Ok((identity, ()))
        })
        .await
        .map_err(|error| error.to_string())?;

    let runtime = RuntimeConfig {
        connection: config,
        edge_node_id,
        password,
        ca,
    };
    Ok(tokio::spawn(run(db, health, runtime)))
}

async fn run(db: DbHandle, health: Arc<Mutex<HealthState>>, runtime: RuntimeConfig) {
    let records_topic = records_topic(&runtime.edge_node_id);
    let ack_topic = ack_topic(&runtime.edge_node_id);
    let mut options = MqttOptions::new(
        client_id(&runtime.edge_node_id),
        runtime.connection.host.clone(),
        runtime.connection.port,
    );
    configure_packet_limits(&mut options, &records_topic, &ack_topic);
    options.set_keep_alive(Duration::from_secs(30));
    options.set_clean_session(true);
    options.set_credentials(&runtime.edge_node_id, &runtime.password);
    options.set_transport(if runtime.connection.allow_insecure {
        Transport::tcp()
    } else if let Some(ca) = runtime.ca {
        Transport::tls(ca, None, None)
    } else {
        Transport::tls_with_default_config()
    });

    let (client, mut event_loop) = AsyncClient::new(options, 10);
    let mut retry = tokio::time::interval(RETRY_INTERVAL);
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut subscribed = false;
    let mut inflight: Option<PreparedBatch> = None;

    loop {
        tokio::select! {
            event = event_loop.poll() => {
                match event {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        subscribed = false;
                        if let Err(error) = client.subscribe(&ack_topic, QoS::AtLeastOnce).await {
                            tracing::warn!(error = %error, "failed to queue MQTT acknowledgement subscription");
                        }
                    }
                    Ok(Event::Incoming(Incoming::SubAck(_))) => {
                        subscribed = true;
                        if let Err(error) = publish_current_or_next(
                            &db,
                            &client,
                            &records_topic,
                            &runtime.edge_node_id,
                            &mut inflight,
                        ).await {
                            tracing::warn!(error = %error, "MQTT publish attempt failed");
                            super::publish_task::refresh_publish_health(&db, &health, &Err(error)).await;
                        }
                    }
                    Ok(Event::Incoming(Incoming::Publish(message))) => {
                        let result = handle_ack(
                            &db,
                            &ack_topic,
                            &message.topic,
                            &message.payload,
                            &mut inflight,
                        ).await;
                        match result {
                            Ok(true) => {
                                super::publish_task::refresh_publish_health(&db, &health, &Ok(())).await;
                                if let Err(error) = publish_current_or_next(
                                    &db,
                                    &client,
                                    &records_topic,
                                    &runtime.edge_node_id,
                                    &mut inflight,
                                ).await {
                                    tracing::warn!(error = %error, "MQTT publish attempt failed");
                                }
                            }
                            Ok(false) => {}
                            Err(error) => {
                                tracing::warn!(error = %error, "invalid MQTT application acknowledgement");
                                super::publish_task::refresh_publish_health(&db, &health, &Err(error)).await;
                            }
                        }
                    }
                    Ok(_) => {
                        // MQTT PUBACK is deliberately transport-only. Cursor changes are handled
                        // exclusively by the application acknowledgement branch above.
                    }
                    Err(error) => {
                        subscribed = false;
                        tracing::warn!(error = %error, "MQTT connection event loop failed; reconnecting");
                        tokio::time::sleep(RECONNECT_DELAY).await;
                    }
                }
            }
            _ = retry.tick(), if subscribed => {
                if let Err(error) = publish_current_or_next(
                    &db,
                    &client,
                    &records_topic,
                    &runtime.edge_node_id,
                    &mut inflight,
                ).await {
                    tracing::warn!(error = %error, "MQTT publish retry failed");
                    super::publish_task::refresh_publish_health(&db, &health, &Err(error)).await;
                }
            }
        }
    }
}

async fn publish_current_or_next(
    db: &DbHandle,
    client: &AsyncClient,
    topic: &str,
    edge_node_id: &str,
    inflight: &mut Option<PreparedBatch>,
) -> Result<(), String> {
    if inflight.is_none() {
        let identity = edge_node_id.to_string();
        *inflight = db
            .with_conn(move |conn| Ok::<_, StorageError>(prepare_batch(conn, &identity)))
            .await
            .map_err(|error| error.to_string())??;
    }
    let Some(prepared) = inflight else {
        return Ok(());
    };
    let payload = serde_json::to_vec(&prepared.batch).map_err(|error| error.to_string())?;
    client
        .publish(topic, QoS::AtLeastOnce, false, payload)
        .await
        .map_err(|error| error.to_string())
}

async fn handle_ack(
    db: &DbHandle,
    expected_topic: &str,
    actual_topic: &str,
    payload: &[u8],
    inflight: &mut Option<PreparedBatch>,
) -> Result<bool, String> {
    if actual_topic != expected_topic {
        return Err("application acknowledgement arrived on an unexpected topic".to_string());
    }
    let Some(prepared) = inflight.as_ref() else {
        tracing::debug!(
            topic = actual_topic,
            "ignoring acknowledgement without an inflight batch"
        );
        return Ok(false);
    };
    let ack: AcceptedThrough = serde_json::from_slice(payload)
        .map_err(|error| format!("accepted-through decode failed: {error}"))?;
    ack.validate_for(&prepared.batch, prepared.prior_cursor)
        .map_err(|error| error.to_string())?;

    let epoch = prepared.batch.ledger_epoch.clone();
    let cursor_end = prepared.batch.cursor_end;
    let prior_cursor = prepared.prior_cursor;
    db.with_conn(move |conn| {
        apply_ack(conn, &epoch, prior_cursor, cursor_end).map_err(storage_error)
    })
    .await
    .map_err(|error| error.to_string())?;
    *inflight = None;
    Ok(true)
}

fn prepare_batch(conn: &Connection, edge_node_id: &str) -> Result<Option<PreparedBatch>, String> {
    let current_epoch =
        iotkit_core_ledger::ledger_epoch(conn).map_err(|error| error.to_string())?;
    let Some(target) = target_get(conn).map_err(|error| error.to_string())? else {
        return Err("MQTT exit target is not initialized".to_string());
    };
    if target.target_id != TARGET_ID || !target.archive_responsible {
        return Err("MQTT exit target is not the active archive target".to_string());
    }
    let prior_cursor = effective_cursor(&current_epoch, &target);
    let rows = select_batch(conn, &current_epoch, prior_cursor, MAX_BATCH_RECORDS as u32)
        .map_err(|error| error.to_string())?;
    if rows.is_empty() {
        return Ok(None);
    }
    for (offset, row) in rows.iter().enumerate() {
        let expected = prior_cursor + 1 + offset as i64;
        if row.pub_seq != expected {
            return Err(format!(
                "outbox is not contiguous at pub_seq {} (expected {expected})",
                row.pub_seq
            ));
        }
    }

    let mut records = iotkit_edge::record::materialize_batch(conn, &rows)?;
    while !records.is_empty() {
        let cursor_start = prior_cursor + 1;
        let cursor_end = cursor_start + records.len() as i64 - 1;
        let batch = RecordBatch {
            schema_version: EGRESS_SCHEMA_VERSION,
            edge_node_id: edge_node_id.to_string(),
            ledger_epoch: current_epoch.clone(),
            publication_id: publication_id(edge_node_id, &current_epoch, cursor_start, cursor_end),
            cursor_start,
            cursor_end,
            records,
        };
        let encoded_len = serde_json::to_vec(&batch)
            .map_err(|error| error.to_string())?
            .len();
        if encoded_len <= MAX_BATCH_BYTES {
            batch.validate().map_err(|error| error.to_string())?;
            return Ok(Some(PreparedBatch {
                batch,
                prior_cursor,
            }));
        }
        records = batch.records;
        records.pop();
    }

    Err("one canonical record exceeds the MQTT batch byte limit".to_string())
}

fn apply_ack(
    conn: &Connection,
    epoch: &str,
    prior_cursor: i64,
    cursor_end: i64,
) -> Result<(), String> {
    let Some(target) = target_get(conn).map_err(|error| error.to_string())? else {
        return Err("MQTT exit target disappeared before cursor advance".to_string());
    };
    if target.target_id != TARGET_ID {
        return Err("MQTT exit target changed before cursor advance".to_string());
    }
    let current = effective_cursor(epoch, &target);
    if current == cursor_end {
        return Ok(());
    }
    if current != prior_cursor {
        return Err("MQTT exit cursor changed while a batch was inflight".to_string());
    }
    target_advance_cursor(conn, TARGET_ID, epoch, cursor_end).map_err(|error| error.to_string())
}

fn ensure_target(conn: &Connection, expected_endpoint: &str) -> Result<(), String> {
    match target_get(conn).map_err(|error| error.to_string())? {
        None => target_insert(
            conn,
            &TargetRow {
                target_id: TARGET_ID.to_string(),
                endpoint_url: expected_endpoint.to_string(),
                credential_token: String::new(),
                archive_responsible: true,
                schema_version: EGRESS_SCHEMA_VERSION as i64,
                cursor_epoch: None,
                cursor_pub_seq: 0,
            },
            now_ms(),
        )
        .map_err(|error| error.to_string()),
        Some(target)
            if target.target_id == TARGET_ID
                && target.endpoint_url == expected_endpoint
                && target.credential_token.is_empty()
                && target.archive_responsible
                && target.schema_version == EGRESS_SCHEMA_VERSION as i64 =>
        {
            Ok(())
        }
        Some(_) => {
            Err("existing exit target does not match the configured MQTT Site target".to_string())
        }
    }
}

fn read_password(config: &MqttExitConfig) -> Result<String, String> {
    let contents = std::fs::read_to_string(&config.password_file).map_err(|error| {
        format!(
            "failed to read MQTT password file {}: {error}",
            config.password_file.display()
        )
    })?;
    let password = contents.trim_end_matches(['\r', '\n']);
    if password.is_empty() {
        return Err("MQTT password file is empty".to_string());
    }
    Ok(password.to_string())
}

fn read_ca(config: &MqttExitConfig) -> Result<Option<Vec<u8>>, String> {
    config
        .ca_file
        .as_ref()
        .map(|path| {
            std::fs::read(path)
                .map_err(|error| format!("failed to read MQTT CA file {}: {error}", path.display()))
        })
        .transpose()
}

fn endpoint(config: &MqttExitConfig) -> String {
    let scheme = if config.allow_insecure {
        "mqtt"
    } else {
        "mqtts"
    };
    format!("{scheme}://{}:{}", config.host, config.port)
}

fn records_topic(edge_node_id: &str) -> String {
    format!("iotkit/v1/edge-nodes/{edge_node_id}/records")
}

fn ack_topic(edge_node_id: &str) -> String {
    format!("iotkit/v1/edge-nodes/{edge_node_id}/accepted-through")
}

fn client_id(edge_node_id: &str) -> String {
    format!("iotkit-edge-{edge_node_id}")
}

fn configure_packet_limits(options: &mut MqttOptions, records_topic: &str, ack_topic: &str) {
    let limit = mqtt_packet_limit(records_topic, ack_topic);
    options.set_max_packet_size(limit, limit);
}

fn mqtt_packet_limit(records_topic: &str, ack_topic: &str) -> usize {
    MAX_BATCH_BYTES + records_topic.len().max(ack_topic.len()) + MQTT_PACKET_OVERHEAD_BYTES
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn storage_error(message: String) -> StorageError {
    StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
        std::io::Error::other(message).into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert_eq!(
                records_topic("edge-01"),
                "iotkit/v1/edge-nodes/edge-01/records"
            );
            assert_eq!(
                ack_topic("edge-01"),
                "iotkit/v1/edge-nodes/edge-01/accepted-through"
            );
            assert_eq!(client_id("edge-01"), "iotkit-edge-edge-01");
            prepared.batch.validate().unwrap();
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn mqtt_client_accepts_the_wire_batch_limit_plus_protocol_overhead() {
        let records_topic = records_topic("edge-01");
        let ack_topic = ack_topic("edge-01");
        let mut options = MqttOptions::new("test-client", "localhost", 1883);

        configure_packet_limits(&mut options, &records_topic, &ack_topic);

        assert!(options.max_packet_size() > MAX_BATCH_BYTES);
        assert_eq!(
            options.max_packet_size(),
            mqtt_packet_limit(&records_topic, &ack_topic)
        );
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

    #[test]
    fn existing_legacy_target_is_not_silently_rewritten() {
        let db = test_db();
        db.with_conn_sync(|conn| {
            target_insert(
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
}
