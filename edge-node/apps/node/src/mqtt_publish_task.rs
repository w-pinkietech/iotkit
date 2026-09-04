use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_core_publish::activation::{
    ActivationRequest, apply_activation, cleanup_pre_activation_batch, install_edge_target,
    publication_admitted,
};
use iotkit_core_publish::mqtt::MqttBinding;
use iotkit_core_publish::store::{
    TargetRow, effective_cursor, outbox_backlog_count, select_batch, target_advance_cursor,
    target_get,
};
use iotkit_core_publish::wire::{
    AcceptedThrough, AdapterState, CollectorState, EGRESS_SCHEMA_VERSION, MAX_BATCH_BYTES,
    MAX_BATCH_RECORDS, MAX_STATUS_BYTES, RecordBatch, StatusAdapter, StatusHeartbeat,
    publication_id,
};
use iotkit_core_storage::{DbHandle, StorageError};
use iotkit_edge_node::config::{MqttOutputConfig, MqttTrustMode};
use iotkit_edge_node::health::HealthState;
use rumqttc::{
    AsyncClient, Event, Incoming, MqttOptions, QoS, SubscribeFilter, SubscribeReasonCode, Transport,
};
use rusqlite::Connection;
use uuid::Uuid;

const TARGET_ID: &str = "edge";
const RETRY_INTERVAL: Duration = Duration::from_secs(30);
const IDLE_PROBE_INTERVAL: Duration = Duration::from_secs(1);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MQTT_PACKET_OVERHEAD_BYTES: usize = 16;
const STATUS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct RuntimeConfig {
    connection: MqttOutputConfig,
    binding: MqttBinding,
    password: String,
    ca: Option<Vec<u8>>,
}

struct PreparedBatch {
    batch: RecordBatch,
    prior_cursor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DescriptorIdentity {
    ledger_epoch: String,
    descriptor_revision: u64,
}

struct PreparedDescriptor {
    identity: DescriptorIdentity,
    payload: Vec<u8>,
}

/// Installs the durable publication target before an adapter can create a
/// standalone outbox. Starting the MQTT network task remains a later lifecycle
/// decision so its first status heartbeat reflects the running collector.
pub(crate) async fn prepare_mqtt_publish_runtime(
    db: DbHandle,
    config: MqttOutputConfig,
) -> Result<RuntimeConfig, String> {
    let password = read_password(&config)?;
    let ca = read_ca(&config)?;
    let expected_endpoint = endpoint(&config);
    let (binding, ()) = db
        .with_conn(move |conn| {
            let identity = iotkit_core_ledger::edge_node_id(conn)
                .map_err(|error| storage_error(error.to_string()))?;
            let binding = MqttBinding::for_edge_node(&identity)
                .map_err(|error| storage_error(error.to_string()))?;
            ensure_target(conn, &expected_endpoint).map_err(storage_error)?;
            Ok((binding, ()))
        })
        .await
        .map_err(|error| error.to_string())?;

    Ok(RuntimeConfig {
        connection: config,
        binding,
        password,
        ca,
    })
}

pub(crate) fn spawn_mqtt_publish_task(
    db: DbHandle,
    health: Arc<Mutex<HealthState>>,
    runtime: RuntimeConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(db, health, runtime))
}

async fn run(db: DbHandle, health: Arc<Mutex<HealthState>>, runtime: RuntimeConfig) {
    let records_topic = runtime.binding.records_topic.clone();
    let status_topic = runtime.binding.status_topic.clone();
    let ack_topic = runtime.binding.accepted_through_topic.clone();
    let descriptor_topic = runtime.binding.descriptor_topic.clone();
    let activation_request_topic = runtime.binding.activation_request_topic.clone();
    let activation_result_topic = runtime.binding.activation_result_topic.clone();
    let qos = mqtt_qos(&runtime.binding);
    let mut options = MqttOptions::new(
        runtime.binding.client_id.clone(),
        runtime.connection.host.clone(),
        runtime.connection.port,
    );
    configure_packet_limits(
        &mut options,
        &records_topic,
        &ack_topic,
        &descriptor_topic,
        &status_topic,
    );
    options.set_keep_alive(Duration::from_secs(30));
    options.set_clean_session(true);
    options.set_credentials(&runtime.binding.username, &runtime.password);
    options.set_transport(if runtime.connection.allow_insecure {
        Transport::tcp()
    } else if let Some(ca) = runtime.ca {
        Transport::tls(ca, None, None)
    } else {
        Transport::tls_with_default_config()
    });

    let (client, mut event_loop) = AsyncClient::new(options, 10);
    let timer_started_at = tokio::time::Instant::now();
    let mut retry = tokio::time::interval_at(timer_started_at + RETRY_INTERVAL, RETRY_INTERVAL);
    // Start after a full interval; a delayed task then performs one bounded retry.
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut heartbeat = tokio::time::interval_at(
        timer_started_at + STATUS_HEARTBEAT_INTERVAL,
        STATUS_HEARTBEAT_INTERVAL,
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut idle_probe =
        tokio::time::interval_at(timer_started_at + IDLE_PROBE_INTERVAL, IDLE_PROBE_INTERVAL);
    // Start after a full interval. One indexed no-inflight probe is enough after a busy interval.
    idle_probe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut subscribed = false;
    let mut inflight: Option<PreparedBatch> = None;
    let mut published_descriptor: Option<DescriptorIdentity> = None;
    let boot_id = format!("boot-{}", Uuid::new_v4().simple());
    let mut status_seq = 0_u64;

    loop {
        tokio::select! {
            event = event_loop.poll() => {
                match event {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        subscribed = false;
                        published_descriptor = None;
                        if let Err(error) = client
                            .subscribe_many(subscription_filters(&runtime.binding, qos))
                            .await
                        {
                            tracing::warn!(error = %error, "failed to queue MQTT control subscriptions");
                        }
                        if let Err(error) = publish_descriptor_if_changed(
                            &db,
                            &client,
                            &descriptor_topic,
                            &runtime.binding.edge_node_id,
                            qos,
                            runtime.binding.descriptor_retain,
                            &mut published_descriptor,
                        ).await {
                            tracing::warn!(error = %error, "MQTT descriptor publish attempt failed");
                        }
                    }
                    Ok(Event::Incoming(Incoming::SubAck(ack))) => {
                        subscribed = ack.return_codes.len() == 2
                            && ack.return_codes.iter().all(|code| {
                                matches!(code, SubscribeReasonCode::Success(QoS::AtLeastOnce))
                            });
                        if !subscribed {
                            tracing::warn!("MQTT Broker rejected one or more control subscriptions");
                            continue;
                        }
                        if let Err(error) = publish_status_heartbeat(
                            &db,
                            &health,
                            &client,
                            &runtime.binding,
                            &boot_id,
                            &mut status_seq,
                        ).await {
                            tracing::warn!(error = %error, "MQTT status heartbeat publish attempt failed");
                        }
                        // The reconnect-path heartbeat above is the initial status for this
                        // confirmed subscription. Do not immediately emit a second one when
                        // an overdue interval tick is selected next.
                        heartbeat.reset();
                        if let Err(error) = publish_current_or_next(
                            &db,
                            &client,
                            &records_topic,
                            &runtime.binding.edge_node_id,
                            qos,
                            runtime.binding.retain,
                            &mut inflight,
                        ).await {
                            tracing::warn!(error = %error, "MQTT publish attempt failed");
                            super::publish_task::refresh_publish_health(&db, &health, &Err(error)).await;
                        }
                    }
                    Ok(Event::Incoming(Incoming::Publish(message))) => {
                        if message.topic == activation_request_topic {
                            match handle_activation_request(
                                &db,
                                &activation_request_topic,
                                &message.topic,
                                &message.payload,
                            ).await {
                                Ok(payload) => {
                                    if let Err(error) = client
                                        .publish(
                                            &activation_result_topic,
                                            qos,
                                            false,
                                            payload,
                                        )
                                        .await
                                    {
                                        tracing::warn!(error = %error, "MQTT activation result publish attempt failed");
                                    }
                                    if let Err(error) = publish_current_or_next(
                                        &db,
                                        &client,
                                        &records_topic,
                                        &runtime.binding.edge_node_id,
                                        qos,
                                        runtime.binding.retain,
                                        &mut inflight,
                                    ).await {
                                        tracing::warn!(error = %error, "MQTT post-activation publish attempt failed");
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        topic = %message.topic,
                                        error = %error,
                                        "invalid MQTT activation request"
                                    );
                                }
                            }
                            continue;
                        }
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
                                    &runtime.binding.edge_node_id,
                                    qos,
                                    runtime.binding.retain,
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
                        published_descriptor = None;
                        tracing::warn!(error = %error, "MQTT connection event loop failed; reconnecting");
                        tokio::time::sleep(RECONNECT_DELAY).await;
                    }
                }
            }
            _ = retry.tick(), if subscribed => {
                if let Err(error) = publish_descriptor_if_changed(
                    &db,
                    &client,
                    &descriptor_topic,
                    &runtime.binding.edge_node_id,
                    qos,
                    runtime.binding.descriptor_retain,
                    &mut published_descriptor,
                ).await {
                    tracing::warn!(error = %error, "MQTT descriptor publish retry failed");
                }
                if inflight.is_some()
                    && let Err(error) = publish_current_or_next(
                        &db,
                        &client,
                        &records_topic,
                        &runtime.binding.edge_node_id,
                        qos,
                        runtime.binding.retain,
                        &mut inflight,
                    ).await
                {
                    tracing::warn!(error = %error, "MQTT publish retry failed");
                    super::publish_task::refresh_publish_health(&db, &health, &Err(error)).await;
                }
                if let Err(error) = cleanup_activation_prefix(&db).await {
                    tracing::warn!(error = %error, "pre-activation reading cleanup failed");
                }
            }
            _ = heartbeat.tick(), if subscribed => {
                if let Err(error) = publish_status_heartbeat(
                    &db,
                    &health,
                    &client,
                    &runtime.binding,
                    &boot_id,
                    &mut status_seq,
                ).await {
                    tracing::warn!(error = %error, "MQTT status heartbeat publish retry failed");
                }
            }
            _ = idle_probe.tick(), if subscribed && inflight.is_none() => {
                if let Err(error) = publish_current_or_next(
                    &db,
                    &client,
                    &records_topic,
                    &runtime.binding.edge_node_id,
                    qos,
                    runtime.binding.retain,
                    &mut inflight,
                ).await {
                    tracing::warn!(error = %error, "MQTT idle publish probe failed");
                    super::publish_task::refresh_publish_health(&db, &health, &Err(error)).await;
                }
            }
        }
    }
}

async fn publish_status_heartbeat(
    db: &DbHandle,
    health: &Arc<Mutex<HealthState>>,
    client: &AsyncClient,
    binding: &MqttBinding,
    boot_id: &str,
    status_seq: &mut u64,
) -> Result<(), String> {
    let next_seq = next_status_sequence(*status_seq)?;
    let edge_node_id = binding.edge_node_id.clone();
    let (ledger_epoch, accepted_through, pending_publications) = db
        .with_conn(move |conn| {
            let ledger_epoch = iotkit_core_ledger::ledger_epoch(conn)
                .map_err(|error| storage_error(error.to_string()))?;
            let target = target_get(conn).map_err(|error| storage_error(error.to_string()))?;
            let Some(target) = target else {
                return Err(storage_error("MQTT exit target is not initialized".into()));
            };
            if target.target_id != TARGET_ID || !target.archive_responsible {
                return Err(storage_error("MQTT exit target is not active".into()));
            }
            let accepted_through = effective_cursor(&ledger_epoch, &target);
            let pending_publications = outbox_backlog_count(conn, &ledger_epoch, &target)
                .map_err(|error| storage_error(error.to_string()))?;
            Ok((ledger_epoch, accepted_through, pending_publications))
        })
        .await
        .map_err(|error| error.to_string())?;
    let snapshot = health.lock().expect("health state mutex poisoned").clone();
    let heartbeat = build_status_heartbeat(
        &snapshot,
        edge_node_id,
        ledger_epoch,
        boot_id,
        next_seq,
        accepted_through,
        pending_publications,
    );
    heartbeat.validate().map_err(|error| error.to_string())?;
    let payload = serde_json::to_vec(&heartbeat).map_err(|error| error.to_string())?;
    if payload.len() > MAX_STATUS_BYTES {
        return Err("status heartbeat exceeds encoded byte limit".into());
    }
    client
        .publish(
            &binding.status_topic,
            mqtt_qos(binding),
            binding.status_retain,
            payload,
        )
        .await
        .map_err(|error| error.to_string())?;
    *status_seq = next_seq;
    Ok(())
}

fn next_status_sequence(status_seq: u64) -> Result<u64, String> {
    status_seq
        .checked_add(1)
        .ok_or_else(|| "status heartbeat sequence exhausted".to_string())
}

#[allow(clippy::too_many_arguments)]
fn build_status_heartbeat(
    snapshot: &HealthState,
    edge_node_id: String,
    ledger_epoch: String,
    boot_id: &str,
    status_seq: u64,
    accepted_through: i64,
    pending_publications: i64,
) -> StatusHeartbeat {
    StatusHeartbeat {
        schema_version: EGRESS_SCHEMA_VERSION,
        edge_node_id,
        ledger_epoch,
        boot_id: boot_id.into(),
        status_seq,
        collector_state: if snapshot.collector_alive {
            CollectorState::Running
        } else {
            CollectorState::Stopped
        },
        // Do not truncate: omitting a failed adapter would make the Console
        // claim a bounded but incomplete health view. The wire validator
        // rejects an invalid local configuration before anything is sent.
        adapters: snapshot
            .adapters
            .iter()
            .map(|adapter| StatusAdapter {
                adapter_id: adapter.id.clone(),
                state: match adapter.status {
                    iotkit_edge_node::health::AdapterRuntimeStatus::Running => {
                        AdapterState::Running
                    }
                    iotkit_edge_node::health::AdapterRuntimeStatus::Restarting => {
                        AdapterState::Restarting
                    }
                    iotkit_edge_node::health::AdapterRuntimeStatus::Exhausted => {
                        AdapterState::Exhausted
                    }
                    iotkit_edge_node::health::AdapterRuntimeStatus::Stopped => {
                        AdapterState::Stopped
                    }
                },
            })
            .collect(),
        accepted_through,
        pending_publications,
        storage_pressure: snapshot.db.watermark_exceeded,
    }
}

fn subscription_filters(binding: &MqttBinding, qos: QoS) -> Vec<SubscribeFilter> {
    vec![
        SubscribeFilter::new(binding.accepted_through_topic.clone(), qos),
        SubscribeFilter::new(binding.activation_request_topic.clone(), qos),
    ]
}

async fn handle_activation_request(
    db: &DbHandle,
    expected_topic: &str,
    actual_topic: &str,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
    let expected_topic = expected_topic.to_string();
    let actual_topic = actual_topic.to_string();
    let payload = payload.to_vec();
    db.with_conn(move |conn| {
        apply_activation_request(conn, &expected_topic, &actual_topic, &payload, now_ms())
            .map_err(storage_error)
    })
    .await
    .map_err(|error| error.to_string())
}

fn apply_activation_request(
    conn: &Connection,
    expected_topic: &str,
    actual_topic: &str,
    payload: &[u8],
    applied_at: i64,
) -> Result<Vec<u8>, String> {
    if actual_topic != expected_topic {
        return Err("activation request arrived on an unexpected topic".into());
    }
    let request = ActivationRequest::decode(payload).map_err(|error| error.to_string())?;
    let result = apply_activation(conn, &request, applied_at).map_err(|error| error.to_string())?;
    cleanup_pre_activation_batch(conn, 5_000).map_err(|error| error.to_string())?;
    result.encode().map_err(|error| error.to_string())
}

async fn cleanup_activation_prefix(db: &DbHandle) -> Result<(), String> {
    db.with_conn(|conn| {
        cleanup_pre_activation_batch(conn, 5_000)
            .map(|_| ())
            .map_err(|error| storage_error(error.to_string()))
    })
    .await
    .map_err(|error| error.to_string())
}

async fn publish_descriptor_if_changed(
    db: &DbHandle,
    client: &AsyncClient,
    topic: &str,
    edge_node_id: &str,
    qos: QoS,
    retain: bool,
    published: &mut Option<DescriptorIdentity>,
) -> Result<(), String> {
    let identity = edge_node_id.to_string();
    let previous = published.clone();
    let prepared = db
        .with_conn(move |conn| {
            Ok::<_, StorageError>(prepare_descriptor(conn, &identity, previous.as_ref()))
        })
        .await
        .map_err(|error| error.to_string())??;
    let Some(prepared) = prepared else {
        return Ok(());
    };
    client
        .publish(topic, qos, retain, prepared.payload)
        .await
        .map_err(|error| error.to_string())?;
    *published = Some(prepared.identity);
    Ok(())
}

fn prepare_descriptor(
    conn: &Connection,
    edge_node_id: &str,
    previous: Option<&DescriptorIdentity>,
) -> Result<Option<PreparedDescriptor>, String> {
    let snapshot =
        iotkit_edge_node::descriptor_snapshot::build_descriptor_snapshot(conn, edge_node_id)
            .map_err(|error| error.to_string())?;
    let identity = DescriptorIdentity {
        ledger_epoch: snapshot.ledger_epoch.clone(),
        descriptor_revision: snapshot.descriptor_revision,
    };
    if previous == Some(&identity) {
        return Ok(None);
    }
    let payload = snapshot
        .encode_bounded()
        .map_err(|error| error.to_string())?;
    Ok(Some(PreparedDescriptor { identity, payload }))
}

async fn publish_current_or_next(
    db: &DbHandle,
    client: &AsyncClient,
    topic: &str,
    edge_node_id: &str,
    qos: QoS,
    retain: bool,
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
        .publish(topic, qos, retain, payload)
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
    let cursor_end = match ack.validate_for(&prepared.batch, prepared.prior_cursor) {
        Ok(()) => prepared.batch.cursor_end,
        Err(error) => {
            if ack
                .validate_stale_for(&prepared.batch, prepared.prior_cursor)
                .is_ok()
            {
                return Ok(false);
            }
            if ack
                .validate_prior_prefix_for(&prepared.batch, prepared.prior_cursor)
                .is_ok()
            {
                ack.accepted_through
            } else {
                return Err(error.to_string());
            }
        }
    };

    let epoch = prepared.batch.ledger_epoch.clone();
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
    if !publication_admitted(conn).map_err(|error| error.to_string())? {
        return Ok(None);
    }
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

    let mut records = iotkit_edge_node::record::materialize_batch(conn, &rows)?;
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
        None => install_edge_target(
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
        Some(_) => Err(
            "existing exit target does not match the configured MQTT IoTKit Edge target"
                .to_string(),
        ),
    }
}

fn read_password(config: &MqttOutputConfig) -> Result<String, String> {
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

fn read_ca(config: &MqttOutputConfig) -> Result<Option<Vec<u8>>, String> {
    match (&config.trust_mode, &config.ca_file) {
        (MqttTrustMode::SystemRoots, None) => Ok(None),
        (MqttTrustMode::BundleOnly, Some(path)) => std::fs::read(path)
            .map(Some)
            .map_err(|error| format!("failed to read MQTT CA file {}: {error}", path.display())),
        _ => Err("invalid resolved MQTT trust configuration".to_string()),
    }
}

fn endpoint(config: &MqttOutputConfig) -> String {
    let scheme = if config.allow_insecure {
        "mqtt"
    } else {
        "mqtts"
    };
    format!("{scheme}://{}:{}", config.host, config.port)
}

fn mqtt_qos(binding: &MqttBinding) -> QoS {
    debug_assert_eq!(binding.qos, 1);
    mqtt_qos_for_v1()
}

fn mqtt_qos_for_v1() -> QoS {
    QoS::AtLeastOnce
}

fn configure_packet_limits(
    options: &mut MqttOptions,
    records_topic: &str,
    ack_topic: &str,
    descriptor_topic: &str,
    status_topic: &str,
) {
    let limit = mqtt_packet_limit(records_topic, ack_topic, descriptor_topic, status_topic);
    options.set_max_packet_size(limit, limit);
}

fn mqtt_packet_limit(
    records_topic: &str,
    ack_topic: &str,
    descriptor_topic: &str,
    status_topic: &str,
) -> usize {
    MAX_BATCH_BYTES
        + records_topic
            .len()
            .max(ack_topic.len())
            .max(descriptor_topic.len())
            .max(status_topic.len())
        + MQTT_PACKET_OVERHEAD_BYTES
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
#[path = "../tests/unit/mqtt_publish_task_tests.rs"]
mod tests;
