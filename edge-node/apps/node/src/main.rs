//! iotkit-edge-node: composition root。
//! adapter を起動し、core/engine に event を渡す。

mod adapter_host;
mod mqtt_publish_task;
#[allow(dead_code)]
mod publish_task;
mod retention;
mod supervision;

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adapter_host::{AdapterHost, AdapterHostEvent};
use iotkit_core_engine::Engine;
use iotkit_core_recovery::RecoveryStartupMode;
use iotkit_core_supervision::AdapterEvent;
use iotkit_core_types::AdapterId;
use iotkit_edge_node::api::{ApiHandle, spawn_api_task};
use iotkit_edge_node::{config, epoch_start, health};
use iotkit_ingest_client::IngestClient;
use iotkit_input_adapter_host_api::{
    AdapterCompletion, AdapterStartContext, RunningInputAdapter, SourceBoundIngest,
};
use tracing_subscriber::EnvFilter;

// Deployment profiles give Docker 15 seconds. Keep cleanup bounded to ten
// seconds so runtime teardown can finish without Docker escalating to SIGKILL.
const SHUTDOWN_CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

type ShutdownSignal = Pin<Box<dyn Future<Output = Result<(), std::io::Error>> + Send>>;

fn version_requested(args: &[String]) -> bool {
    matches!(args, [_, value] if value == "--version")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if version_requested(&args) {
        println!("iotkit-edge-node {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("ring provider install");

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // R20: パニックしたタスクのbacktraceを確実にログへ残す(D1)。
    supervision::install_panic_hook();

    // Register process signals before any recovery/configuration work can start
    // a service or mutate durable state. The bare runtime only owns Tokio's
    // signal driver here; application tasks are still behind the recovery fence.
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let mut shutdown_signal = match rt.block_on(async { install_shutdown_signal() }) {
        Ok(signal) => signal,
        Err(error) => {
            tracing::error!(error = %error, "failed to install shutdown signal handlers");
            rt.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
            std::process::exit(1);
        }
    };

    // Parse TOML and environment overrides only.  Adapter catalog validation
    // and effective-value construction happen after the process-wide recovery
    // fence, including for a syntactically valid config with an invalid
    // adapter.
    let bootstrap = match config::load_bootstrap(&args) {
        Ok(config) => config,
        Err(e) => {
            tracing::error!(error = %e, "failed to load config");
            std::process::exit(1);
        }
    };
    let db_path = match bootstrap.db_path() {
        Ok(path) => path,
        Err(e) => {
            tracing::error!(error = %e, "failed to load config");
            std::process::exit(1);
        }
    };
    // Recovery state is the process-wide startup fence.  Probe with a read-only
    // connection before catalog validation, effective-config logging, migration,
    // identity/provenance mutation, or any application service setup.
    match iotkit_core_recovery::probe_startup_path(Path::new(db_path)) {
        Ok(RecoveryStartupMode::Normal | RecoveryStartupMode::Recovered { .. }) => {}
        Ok(
            RecoveryStartupMode::FencedCandidate { .. }
            | RecoveryStartupMode::AwaitingCompletion { .. },
        ) => {
            eprintln!("fenced recovery candidate; normal runtime is disabled");
            std::process::exit(3);
        }
        Err(_) => {
            eprintln!("Edge Node recovery startup state is invalid");
            std::process::exit(3);
        }
    }
    let unresolved = match bootstrap.load_full() {
        Ok(config) => config,
        Err(e) => {
            tracing::error!(error = %e, "failed to load config");
            std::process::exit(1);
        }
    };
    let config = match unresolved.resolve() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to load config");
            std::process::exit(1);
        }
    };
    if let Err(error) = iotkit_edge_node::input_adapters::validate_catalog() {
        tracing::error!(%error, "invalid built-in input adapter catalog");
        std::process::exit(1);
    }

    tracing::info!(source = ?config.config_source, "config loaded");
    tracing::info!(
        edge_node_id = %config.edge_node_id,
        db_path = %config.db_path,
        retention_days = config.retention_days,
        quarantine_ttl_days = config.quarantine_ttl_days,
        health_json_path = %config.health_json_path.display(),
        disk_high_watermark_pct = config.disk_high_watermark_pct,
        api_enabled = config.api.enabled,
        api_bind = %config.api.bind,
        input_adapter_instances = config.adapter_instances.len(),
        mqtt_output_enabled = config.mqtt_output.is_some(),
        heartbeat_interval_ms = config.status.heartbeat_interval.as_millis(),
        pipelines_export_path = %config.pipelines.export_path.display(),
        "effective config"
    );
    if let Some(bp) = &config.bravepi {
        tracing::info!(port = %bp.port, "bravepi config");
    }
    if let Some(rpi) = &config.rpi_local {
        tracing::info!(bus_path = %rpi.bus_path, poll_interval_ms = rpi.poll_interval_ms, "rpi_local config");
    }
    for instance in &config.adapter_instances {
        tracing::info!(
            instance = %instance.instance_id(),
            adapter_type = instance.adapter_type(),
            source = %instance.source(),
            "input adapter instance configured"
        );
    }
    if let Some(mqtt) = &config.mqtt_output {
        tracing::info!(
            host = %mqtt.host,
            port = mqtt.port,
            tls = !mqtt.allow_insecure,
            ca_file = ?mqtt.ca_file,
            "MQTT output config"
        );
    }

    let all_migrations = iotkit_core_recovery::all_edge_node_migrations();
    let db_path_for_init = std::path::Path::new(&config.db_path);
    let database_existed_before_open = db_path_for_init.exists();
    if let Err(error) = iotkit_core_storage::preflight_edge_node_database(db_path_for_init) {
        tracing::error!(error = %error, db_path = %config.db_path, "Edge Node database cutover preflight failed");
        std::process::exit(1);
    }
    let db = match iotkit_core_storage::init_db(db_path_for_init, &all_migrations) {
        Ok(handle) => handle,
        Err(e) => {
            tracing::error!(error = %e, db_path = %config.db_path, "failed to initialize database");
            std::process::exit(1);
        }
    };
    // Defense in depth: a concurrent or otherwise unexpected state change must
    // still be observed before identity/provenance writes begin.
    let startup_is_normal = db.with_conn_sync(|conn| {
        Ok(matches!(
            iotkit_core_recovery::startup_mode(conn),
            Ok(RecoveryStartupMode::Normal | RecoveryStartupMode::Recovered { .. })
        ))
    });
    if !matches!(startup_is_normal, Ok(true)) {
        eprintln!("Edge Node recovery startup state is invalid");
        std::process::exit(3);
    }
    if let Err(error) = db.with_conn_sync(|conn| {
        iotkit_core_ledger::edge_node_id(conn)
            .map(|_| ())
            .map_err(ledger_to_storage_err)
    }) {
        tracing::error!(error = %error, db_path = %config.db_path, "failed to initialize Edge Node identity");
        std::process::exit(1);
    }
    if let Err(error) = db.with_conn_sync(|conn| {
        iotkit_core_ops::reconcile_database_initialization_provenance(
            conn,
            db_path_for_init,
            database_existed_before_open,
        )
        .map_err(|error| {
            iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                Box::new(error),
            ))
        })
    }) {
        tracing::error!(error = %error, db_path = %config.db_path, "failed to reconcile database initialization provenance");
        std::process::exit(1);
    }
    tracing::info!(db_path = %config.db_path, "database initialized");

    let collector_alive = rt.block_on(run(config, db, &mut shutdown_signal));
    // Periodic tasks are detached from the fan-in loop. Do not let an in-flight
    // blocking task consume Docker's remaining shutdown grace.
    rt.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
    if !collector_alive {
        // R20コメントと同じ方針: プロセスレベルの再起動はsystemdの責務。ここでは非ゼロexitで
        // 「死んでいる」ことを伝えるだけ(要求されたshutdownはexit 0のまま区別する)。
        std::process::exit(1);
    }
}

/// ledger::LedgerError → StorageError の橋渡し(Edge Node起動シーケンス専用ヘルパ)。
/// ledgerクレートはStorageErrorを直接返さないため、ここで包む。起動時失敗はexpectで落とす方針(brief参照)。
fn ledger_to_storage_err(e: iotkit_core_ledger::LedgerError) -> iotkit_core_storage::StorageError {
    // rusqlite::Error::ModuleError requires the "vtab" feature (not enabled here),
    // so ToSqlConversionFailure is used as a generic non-gated carrier (brief: variant
    // name adjusted to whatever the build accepts; intent is "fail loudly at startup").
    iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

/// フォールインループを実行する。戻り値は「プロセスとして正常終了してよいか」
/// (true=正常終了・要求されたshutdown/全アダプタclose、false=コレクタ死亡/API bind失敗/
/// API専用モードでのAPI異常終了によるfail-fast終了)。
/// `main`はfalseを非ゼロexitに変換し、systemdのプロセス再起動に委ねる(R20と同じ設計方針)。
async fn run(
    config: config::EdgeNodeConfig,
    db: iotkit_core_storage::DbHandle,
    shutdown_signal: &mut ShutdownSignal,
) -> bool {
    let engine = Engine::new();
    let mut host = AdapterHost::new();
    let db_path = std::path::PathBuf::from(&config.db_path);
    let health_state = Arc::new(Mutex::new(health::HealthState::new(config.retention_days)));
    let clock_trust = db
        .with_conn(|conn| {
            iotkit_core_ops::ClockTrust::load(
                conn,
                Arc::new(iotkit_core_ops::SystemClock::default()),
                Duration::from_secs(2),
                Duration::from_secs(5 * 60),
            )
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
        })
        .await
        .expect("clock trust state");
    let clock_trust = Arc::new(clock_trust);
    let epoch = db
        .with_conn(|conn| {
            iotkit_core_ledger::ledger_epoch(conn).map_err(|e| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(e),
                ))
            })
        })
        .await
        .expect("ledger epoch");
    let prepared_mqtt_runtime = if let Some(mqtt_config) = config.mqtt_output.clone() {
        match mqtt_publish_task::prepare_mqtt_publish_runtime(db.clone(), mqtt_config).await {
            Ok(runtime) => {
                tracing::info!("MQTT exit publication target prepared");
                Some(runtime)
            }
            Err(error) => {
                tracing::error!(error = %error, "failed to prepare MQTT exit publication target");
                return false;
            }
        }
    } else {
        None
    };
    let _retention_task = retention::spawn_retention_task(
        db.clone(),
        db_path.clone(),
        retention::RetentionConfig {
            retention_days: config.retention_days,
            quarantine_ttl_days: config.quarantine_ttl_days,
            disk_high_watermark_pct: config.disk_high_watermark_pct,
        },
        health_state.clone(),
        Duration::from_secs(24 * 60 * 60),
    );
    let _health_task = health::spawn_health_writer(
        config.health_json_path.clone(),
        epoch.clone(),
        health_state.clone(),
        clock_trust.clone(),
        db.clone(),
        Duration::from_secs(60),
    );
    let mut api_shutdown = None;
    let mut api_join = None;
    if config.api.enabled {
        let data_dir = Path::new(&config.db_path)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let handle = match spawn_api_task(
            db.clone(),
            health_state.clone(),
            config.api.clone(),
            epoch.clone(),
            data_dir,
            clock_trust.clone(),
        )
        .await
        {
            Ok(handle) => handle,
            Err(e) => {
                tracing::error!(error = %e, bind = %config.api.bind, "failed to start control-plane API");
                return false;
            }
        };
        let ApiHandle {
            local_addr,
            fingerprint,
            shutdown,
            join,
        } = handle;
        tracing::info!(
            bind = %local_addr,
            tls_fingerprint = %fingerprint,
            interfaces = "box,session,health,series,live,readings,ops",
            "control-plane API started"
        );
        api_shutdown = Some(shutdown);
        api_join = Some(join);
    } else {
        tracing::info!("control-plane API disabled");
    }
    let mut api_task_running = api_join.is_some();

    // Ingest collector: fan-inループのSensorData分岐が経由する耐久点(D1)。
    // 受理判定はD6判別表(SqliteRegistry=現場レジストリ参照、計画2)。
    let (collector, principal_issuer, device_issuer, _collector_handle) =
        iotkit_core_collector::Collector::spawn_fully_composed(
            db.clone(),
            std::sync::Arc::new(iotkit_core_registry::SqliteRegistry),
            256,
        );
    health_state
        .lock()
        .expect("health state mutex poisoned")
        .collector_alive = true;
    let ingress_service = iotkit_ingest_http::HttpIngestService::new(
        db.clone(),
        collector.clone(),
        device_issuer,
        iotkit_ingest_http::HttpIngestConfig::default(),
        iotkit_ingest_http::SystemMonotonicClock::default(),
    )
    .expect("finite HTTP ingress configuration");
    // R2 listener ownership is supervised independently from control API and collection. Task 6
    // connects the bounded Task 5 HTTP service before the first possible enabled bind.
    let data_dir = Path::new(&config.db_path)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let _ingress_task = iotkit_edge_node::ingress::spawn_ingress_supervisor_serving(
        db.clone(),
        data_dir,
        health_state.clone(),
        Duration::from_secs(1),
        ingress_service,
    );
    // 取り込みクライアント(D4の第3部品、inproc)はアダプタごとにreceiver-created
    // principalを束縛する。アダプタ側が送れるのはEnvelopeだけ。
    let (ingest_exit_tx, mut ingest_exit_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut ingest_client_task_count = 0usize;

    // Validate-all has completed in config::resolve. Reconcile the combined positional
    // inventory before starting any runtime.
    let positional_inventory: Vec<_> = config
        .adapter_instances
        .iter()
        .flat_map(|instance| instance.positional_inventory())
        .collect();
    if !positional_inventory.is_empty() {
        db.with_conn(move |conn| {
            let devices = positional_inventory
                .iter()
                .map(|item| {
                    serde_json::json!({
                        "hardware_id": item.hardware_id,
                        "model_id": item.model_id,
                        "user_label": item.label,
                    })
                })
                .collect::<Vec<_>>();
            iotkit_core_ops::dispatch(
                conn,
                iotkit_core_ops::standard_catalog(),
                iotkit_core_ops::DispatchRequest {
                    op: iotkit_core_ops::POSITIONAL_INVENTORY_RECONCILE_OP.into(),
                    params: serde_json::json!({ "devices": devices }),
                    dry_run: false,
                    actor: iotkit_core_ops::Actor {
                        actor_id: "system:iotkit-edge-node".into(),
                        actor_kind: iotkit_core_ops::ActorKind::System,
                        tier_ceiling: iotkit_core_ops::Tier::Daily,
                    },
                    source: Some("input_adapter_inventory".into()),
                    step_up_verified: false,
                    clock_trust: None,
                },
            )
            .and_then(iotkit_core_ops::DispatchResult::into_public)
            .map(|_| ())
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
        })
        .await
        .expect("positional device registration");
    }

    // Every built-in instance uses the same generic host lifecycle and restart record.
    let mut restart_specs: HashMap<AdapterId, RestartSpec> = HashMap::new();
    let mut active_generations: HashMap<AdapterId, u64> = HashMap::new();
    let mut generation_counters: HashMap<AdapterId, u64> = HashMap::new();
    let (healthy_tx, mut healthy_rx) =
        tokio::sync::mpsc::channel::<ActivityNotice>((config.adapter_instances.len() * 2).max(1));
    for instance in &config.adapter_instances {
        let source = instance.source().as_str().to_owned();
        let principal =
            principal_issuer.official_adapter(format!("principal:{source}"), source.clone());
        let (ingest, ingest_handle) = iotkit_ingest_client::spawn_inproc(
            collector.clone(),
            principal,
            iotkit_ingest_client::DEFAULT_QUEUE_CAP,
            iotkit_ingest_client::DEFAULT_SPOOL_CAP,
        );
        ingest_client_task_count += 1;
        let exit_tx = ingest_exit_tx.clone();
        tokio::spawn(async move {
            let _ = ingest_handle.await;
            let _ = exit_tx.send(());
        });
        match start_input_adapter(
            &mut host,
            instance.clone(),
            ingest.clone(),
            healthy_tx.clone(),
            1,
        ) {
            Ok(id) => {
                active_generations.insert(id.clone(), 1);
                generation_counters.insert(id.clone(), 1);
                health_state
                    .lock()
                    .expect("health state mutex poisoned")
                    .note_adapter_running(&id.to_string());
                restart_specs.insert(
                    id,
                    RestartSpec {
                        instance: instance.clone(),
                        ingest,
                    },
                );
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    instance = %instance.instance_id(),
                    adapter_type = instance.adapter_type(),
                    "failed to start input adapter"
                );
                host.shutdown_all().await;
                return false;
            }
        }
    }
    drop(ingest_exit_tx);

    db.with_conn(|conn| Ok(epoch_start::maybe_enqueue_epoch_start(conn)))
        .await
        .expect("epoch_start annotation")
        .expect("epoch_start annotation");
    if !may_start_status_publisher(&health_state, config.adapter_instances.len()) {
        tracing::error!(
            "collector or configured input adapters were not ready before MQTT startup"
        );
        host.shutdown_all().await;
        return false;
    }
    let mut publish_task = if let Some(runtime) = prepared_mqtt_runtime {
        let task =
            mqtt_publish_task::spawn_mqtt_publish_task(db.clone(), health_state.clone(), runtime);
        tracing::info!("MQTT exit publisher started");
        Some(task)
    } else {
        tracing::info!("MQTT exit publisher disabled");
        None
    };

    // R20: アプリレベル監督(責務台帳)。プロセスレベルはsystemdに委譲。
    let mut tracker = supervision::RestartTracker::new(supervision::RestartPolicy::default());

    // コレクタタスク死亡時、fan-inループをbreakした後にfalseを返して非ゼロexitへ導くフラグ
    // (正常終了=要求されたshutdown/全アダプタclose はtrueのまま=exit 0)。プロセスレベルの再起動は
    // systemdの責務(R20コメント参照)であり、ここでは「健康でないまま動き続けない」ことだけ担保する。
    let mut collector_alive = true;
    let mut api_failed = false;
    let mut mqtt_failed = false;
    let mut api_shutdown_requested = false;
    let mqtt_task_running = publish_task.is_some();
    let service_only_mode = host.is_empty() && (api_task_running || mqtt_task_running);
    let (tx_restart, mut rx_restart) = tokio::sync::mpsc::unbounded_channel::<AdapterId>();
    let mut pending_restart_count = 0usize;
    let mut exhausted_adapter_count = 0usize;
    // Unified fan-in loop
    loop {
        if host.is_empty()
            && should_stop_after_all_adapter_streams_closed(
                pending_restart_count,
                exhausted_adapter_count,
                api_task_running || mqtt_task_running,
                service_only_mode,
            )
        {
            log_fan_in_stop(service_only_mode, api_failed);
            break;
        }

        tokio::select! {
            shutdown_result = &mut *shutdown_signal => {
                if let Err(error) = shutdown_result {
                    tracing::warn!(error = %error, "shutdown signal listener failed");
                } else {
                    tracing::info!("Shutdown signal received");
                }
                if let Some(shutdown) = api_shutdown.take() {
                    api_shutdown_requested = true;
                    let _ = shutdown.send(());
                }
                break;
            }
            api_result = wait_for_api_task(&mut api_join), if api_task_running => {
                api_task_running = false;
                api_shutdown = None;
                health_state
                    .lock()
                    .expect("health state mutex poisoned")
                    .api = None;
                if api_shutdown_requested {
                    match api_result {
                        Ok(()) => tracing::info!("control-plane API task exited"),
                        Err(e) => tracing::error!(error = %e, "control-plane API task panicked during requested shutdown"),
                    }
                } else {
                    api_failed = true;
                    match api_result {
                        Ok(()) => tracing::error!("control-plane API task exited unexpectedly"),
                        Err(e) => tracing::error!(error = %e, "control-plane API task panicked"),
                    }
                }
            }
            mqtt_result = wait_for_mqtt_task(&mut publish_task), if mqtt_task_running => {
                publish_task = None;
                mqtt_failed = true;
                match mqtt_result {
                    Ok(()) => tracing::error!("MQTT exit publisher exited unexpectedly"),
                    Err(error) => tracing::error!(error = %error, "MQTT exit publisher panicked"),
                }
                break;
            }
            Some(()) = ingest_exit_rx.recv(), if ingest_client_task_count > 0 => {
                // クライアントタスク退出=コレクタ死亡(Closed)。取り込み全損なのでfail-fast
                tracing::error!("ingest client exited (collector closed); aborting fan-in loop");
                collector_alive = false;
                health_state
                    .lock()
                    .expect("health state mutex poisoned")
                    .collector_alive = false;
                break;
            }
            Some(notice) = healthy_rx.recv() => {
                if activity_notice_is_current(&active_generations, &notice) {
                    tracker.note_healthy(&notice.adapter_id);
                    health_state
                        .lock()
                        .expect("health state mutex poisoned")
                        .note_adapter_event(&notice.adapter_id.to_string(), health::now_ms());
                }
            }
            Some(id) = rx_restart.recv() => {
                pending_restart_count = pending_restart_count.saturating_sub(1);
                let Some(spec) = restart_specs.get(&id).cloned() else {
                    tracing::warn!(
                        adapter = %id,
                        "Restart timer fired for adapter without restart spec"
                    );
                    continue;
                };
                let generation = *generation_counters
                    .entry(id.clone())
                    .and_modify(|generation| *generation = generation.saturating_add(1))
                    .or_insert(1);
                let restart_result = start_input_adapter(
                    &mut host,
                    spec.instance.clone(),
                    spec.ingest.clone(),
                    healthy_tx.clone(),
                    generation,
                );
                match restart_result {
                    Ok(new_id) => {
                        active_generations.insert(new_id.clone(), generation);
                        health_state
                            .lock()
                            .expect("health state mutex poisoned")
                            .note_adapter_running(&new_id.to_string());
                        tracing::info!(adapter = %new_id, "Adapter restarted successfully");
                    }
                    Err(e) => {
                        tracing::error!(
                            adapter = %id, error = %e,
                            "Adapter restart attempt failed"
                        );
                        if let Some(delay) = tracker.next_delay(&id) {
                            health_state
                                .lock()
                                .expect("health state mutex poisoned")
                                .note_adapter_restarting(&id.to_string());
                            pending_restart_count = pending_restart_count.saturating_add(1);
                            supervision::schedule_restart_notification(
                                id,
                                delay,
                                tx_restart.clone(),
                            );
                        } else {
                            exhausted_adapter_count = exhausted_adapter_count.saturating_add(1);
                            health_state
                                .lock()
                                .expect("health state mutex poisoned")
                                .note_adapter_exhausted(&id.to_string());
                            tracing::error!(
                                adapter = %id,
                                "Adapter permanently degraded after restart-start failures"
                            );
                        }
                    }
                }
            }
            event = host.next_event(), if !host.is_empty() => {
                match event {
                    Some(AdapterHostEvent::Event(ev)) => {
                        tracing::debug!(
                            adapter = %ev.adapter_id,
                            event = ?ev.event,
                            "Adapter event"
                        );

                        // Extract sensor health BEFORE engine consumes the event.
                        let healthy_adapter = match &ev.event {
                            AdapterEvent::SensorData { .. } => Some(ev.adapter_id.clone()),
                            _ => None,
                        };

                        // engine.apply(ev) は従来どおり(projectionは旧語彙のまま=D5「engineはWave 0無改修」)
                        engine.apply(ev).await;

                        if let Some(adapter_id) = healthy_adapter {
                            // R20: 正常受信のたびに再起動カウンタをリセットする(簡略化: HashMap::removeは冪等で安価)。
                            tracker.note_healthy(&adapter_id);
                            health_state
                                .lock()
                                .expect("health state mutex poisoned")
                                .note_adapter_event(&adapter_id.to_string(), health::now_ms());
                        }
                    }
                    Some(AdapterHostEvent::AdapterClosed(id)) => {
                        active_generations.remove(&id);
                        health_state
                            .lock()
                            .expect("health state mutex poisoned")
                            .note_adapter_closed(&id.to_string());
                        // Closed済みアダプタをまず登録簿から除去する(除去しないと同一IDでの
                        // 再registerがadapter_host::registerの重複拒否に阻まれる)。
                        host.deregister(&id);

                        match restart_specs.get(&id).cloned() {
                            Some(_) => match tracker.next_delay(&id) {
                                Some(delay) => {
                                    // ジッタ: 同時再送ストーム対策(D1)。単一プロセス内では
                                    // プロセスIDから導く決定的オフセットで簡易に足りる。
                                    let jitter = std::time::Duration::from_millis(
                                        u64::from(std::process::id() % 1000),
                                    );
                                    let sleep_for = delay + jitter;
                                    tracing::warn!(
                                        adapter = %id,
                                        delay_ms = sleep_for.as_millis() as u64,
                                        "Adapter channel closed, restarting after backoff"
                                    );
                                    pending_restart_count = pending_restart_count.saturating_add(1);
                                    health_state
                                        .lock()
                                        .expect("health state mutex poisoned")
                                        .note_adapter_restarting(&id.to_string());
                                    supervision::schedule_restart_notification(
                                        id,
                                        sleep_for,
                                        tx_restart.clone(),
                                    );
                                }
                                None => {
                                    exhausted_adapter_count =
                                        exhausted_adapter_count.saturating_add(1);
                                    health_state
                                        .lock()
                                        .expect("health state mutex poisoned")
                                        .note_adapter_exhausted(&id.to_string());
                                    tracing::error!(
                                        adapter = %id,
                                        "Adapter permanently degraded (restart budget exhausted)"
                                    );
                                }
                            },
                            None => {
                                tracing::warn!(
                                    adapter = %id,
                                    "Adapter channel closed unexpectedly (not eligible for restart)"
                                );
                            }
                        }
                    }
                    None => {
                        if should_stop_after_all_adapter_streams_closed(
                            pending_restart_count,
                            exhausted_adapter_count,
                            api_task_running || mqtt_task_running,
                            service_only_mode,
                        ) {
                            log_fan_in_stop(service_only_mode, api_failed);
                            break;
                        }
                    }
                }
            }
        }
    }

    let cleanup_completed = shutdown_with_timeout(SHUTDOWN_CLEANUP_TIMEOUT, async {
        host.shutdown_all().await;
        if let Some(shutdown) = api_shutdown.take() {
            api_shutdown_requested = true;
            let _ = shutdown.send(());
        }
        if let Some(join) = api_join.take()
            && api_task_running
        {
            match join.await {
                Ok(()) if api_shutdown_requested => {
                    tracing::info!("control-plane API task exited during requested shutdown");
                }
                Ok(()) => {
                    tracing::info!("control-plane API task exited during shutdown");
                }
                Err(e) => {
                    tracing::error!(error = %e, "control-plane API task panicked during shutdown");
                }
            }
        }
        if let Some(task) = publish_task.take() {
            task.abort();
            let _ = task.await;
        }
    })
    .await;
    if !cleanup_completed {
        tracing::error!(
            timeout_ms = SHUTDOWN_CLEANUP_TIMEOUT.as_millis() as u64,
            "graceful shutdown cleanup timed out"
        );
    }
    {
        let mut health = health_state.lock().expect("health state mutex poisoned");
        health.api = None;
        health.collector_alive = collector_alive;
    }

    let devices = engine.devices().await;
    tracing::info!(device_count = devices.len(), "Engine state at shutdown");

    !should_exit_nonzero(collector_alive, api_failed, mqtt_failed, cleanup_completed)
}

async fn wait_for_api_task(
    api_join: &mut Option<tokio::task::JoinHandle<()>>,
) -> Result<(), tokio::task::JoinError> {
    match api_join {
        Some(join) => join.await,
        None => std::future::pending().await,
    }
}

async fn wait_for_mqtt_task(
    publish_task: &mut Option<tokio::task::JoinHandle<()>>,
) -> Result<(), tokio::task::JoinError> {
    match publish_task {
        Some(task) => task.await,
        None => std::future::pending().await,
    }
}

async fn shutdown_with_timeout<F>(timeout: Duration, cleanup: F) -> bool
where
    F: Future<Output = ()>,
{
    tokio::time::timeout(timeout, cleanup).await.is_ok()
}

fn install_shutdown_signal() -> Result<ShutdownSignal, std::io::Error> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut interrupt = signal(SignalKind::interrupt())?;
        let mut terminate = signal(SignalKind::terminate())?;
        Ok(Box::pin(async move {
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
            Ok(())
        }))
    }
    #[cfg(not(unix))]
    {
        Ok(Box::pin(tokio::signal::ctrl_c()))
    }
}

/// R20: 再起動を許可される公式アダプタ(D4: 再起動権限は形態①のみ)の起動パラメータ。
/// AdapterClosed時、host.deregister後にこのspecから同じ起動パスを再実行する。
#[derive(Clone)]
struct RestartSpec {
    instance: iotkit_edge_node::input_adapters::PreparedInputAdapter,
    ingest: IngestClient,
}

fn start_input_adapter(
    host: &mut AdapterHost,
    instance: iotkit_edge_node::input_adapters::PreparedInputAdapter,
    ingest: IngestClient,
    healthy_tx: tokio::sync::mpsc::Sender<ActivityNotice>,
    generation: u64,
) -> Result<AdapterId, String> {
    let adapter_id = AdapterId::new(instance.instance_id().as_str());
    if host.contains(&adapter_id) {
        return Err(format!(
            "duplicate adapter instance ID: {}",
            instance.instance_id()
        ));
    }
    let context = AdapterStartContext::try_new(
        instance.instance_id().clone(),
        instance.source().clone(),
        SourceBoundIngest::new(instance.source().clone(), ingest),
    )
    .map_err(|error| format!("invalid adapter source binding: {error}"))?;
    let running = instance.start(context)?;
    register_running_adapter(host, running, healthy_tx, generation)
}

#[derive(Debug, Clone)]
struct ActivityNotice {
    adapter_id: AdapterId,
    generation: u64,
}

fn activity_notice_is_current(
    active_generations: &HashMap<AdapterId, u64>,
    notice: &ActivityNotice,
) -> bool {
    active_generations.get(&notice.adapter_id) == Some(&notice.generation)
}

fn may_start_status_publisher(
    health: &Arc<Mutex<health::HealthState>>,
    configured_adapter_count: usize,
) -> bool {
    let health = health.lock().expect("health state mutex poisoned");
    health.collector_alive
        && health.adapters.len() == configured_adapter_count
        && health
            .adapters
            .iter()
            .all(|adapter| adapter.status == health::AdapterRuntimeStatus::Running)
}

fn register_running_adapter(
    host: &mut AdapterHost,
    running: RunningInputAdapter,
    healthy_tx: tokio::sync::mpsc::Sender<ActivityNotice>,
    generation: u64,
) -> Result<AdapterId, String> {
    let id = AdapterId::new(running.instance_id.as_str());
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<AdapterEvent>(1);
    let shutdown = running.shutdown.clone();
    let instance_id = running.instance_id.clone();
    let activity_adapter_id = id.clone();
    let activity = running.activity.clone();
    let completion = running.completion;
    let mut diagnostics = running.diagnostics;
    let join = tokio::spawn(async move {
        let diagnostic_task = tokio::spawn(async move {
            while let Some(diagnostic) = diagnostics.recv().await {
                tracing::warn!(
                    instance = %instance_id,
                    kind = ?diagnostic.kind,
                    code = ?diagnostic.code,
                    message = %diagnostic.message,
                    "input adapter diagnostic"
                );
            }
        });
        let activity_task = tokio::spawn(async move {
            let mut last_seen = None;
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let snapshot = activity.snapshot();
                if snapshot.last_physical_decode.is_some()
                    && snapshot.last_physical_decode != last_seen
                {
                    last_seen = snapshot.last_physical_decode;
                    let _ = healthy_tx.try_send(ActivityNotice {
                        adapter_id: activity_adapter_id.clone(),
                        generation,
                    });
                }
            }
        });
        let outcome = completion.wait().await;
        diagnostic_task.abort();
        activity_task.abort();
        let _ = diagnostic_task.await;
        let _ = activity_task.await;
        drop(event_tx);
        outcome
    });
    host.register(id.clone(), event_rx, move || {
        Box::pin(async move {
            shutdown.request();
            match join.await {
                Ok(AdapterCompletion::RequestedStop) => Ok(()),
                Ok(other) => Err(format!("adapter completed unexpectedly: {other:?}")),
                Err(error) => Err(format!("adapter completion task panicked: {error}")),
            }
        })
    })?;
    tracing::info!(adapter = %id, "input adapter started through generic host");
    Ok(id)
}

fn should_stop_after_all_adapter_streams_closed(
    pending_restart_count: usize,
    exhausted_adapter_count: usize,
    background_service_running: bool,
    service_only_mode: bool,
) -> bool {
    pending_restart_count == 0
        && exhausted_adapter_count == 0
        && (!service_only_mode || !background_service_running)
}

fn should_exit_nonzero(
    collector_alive: bool,
    api_failed: bool,
    mqtt_failed: bool,
    cleanup_completed: bool,
) -> bool {
    !collector_alive || api_failed || mqtt_failed || !cleanup_completed
}

fn log_fan_in_stop(service_only_mode: bool, api_failed: bool) {
    match (service_only_mode, api_failed) {
        (true, true) => tracing::error!(
            "background service exited unexpectedly (service-only mode); exiting for restart"
        ),
        (true, false) => tracing::info!("background service exited; service-only mode stopping"),
        (false, true) => tracing::error!(
            "All adapter channels closed after control-plane API task failure; exiting for restart"
        ),
        (false, false) => tracing::info!("All adapter channels closed"),
    }
}

#[cfg(test)]
#[path = "../tests/unit/main_tests.rs"]
mod tests;
