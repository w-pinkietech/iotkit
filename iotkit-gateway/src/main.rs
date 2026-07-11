//! iotkit-gateway: composition root。
//! adapter を起動し、core/engine に event を渡す。

mod adapter_host;
#[allow(dead_code)]
mod publish_task;
mod retention;
mod supervision;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adapter_host::{AdapterHost, AdapterHostEvent};
use iotkit_core_engine::Engine;
use iotkit_core_supervision::AdapterEvent;
use iotkit_core_types::AdapterId;
use iotkit_gateway::api::{ApiHandle, spawn_api_task};
use iotkit_gateway::{config, epoch_start, health};
use iotkit_ingest_client::IngestClient;
use tracing_subscriber::EnvFilter;

fn main() {
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

    let args: Vec<String> = std::env::args().collect();
    let config = match config::load(&args) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to load config");
            std::process::exit(1);
        }
    };

    tracing::info!(source = ?config.config_source, "config loaded");
    tracing::info!(
        db_path = %config.db_path,
        retention_days = config.retention_days,
        quarantine_ttl_days = config.quarantine_ttl_days,
        health_json_path = %config.health_json_path.display(),
        disk_high_watermark_pct = config.disk_high_watermark_pct,
        api_enabled = config.api.enabled,
        api_bind = %config.api.bind,
        api_gateway_name = %config.api.gateway_name,
        bravepi_enabled = config.bravepi.is_some(),
        rpi_local_enabled = config.rpi_local.is_some(),
        "effective config"
    );
    if let Some(bp) = &config.bravepi {
        tracing::info!(port = %bp.port, "bravepi config");
    }
    if let Some(rpi) = &config.rpi_local {
        tracing::info!(bus_path = %rpi.bus_path, poll_interval_ms = rpi.poll_interval_ms, "rpi_local config");
    }

    let mut all_migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    all_migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS); // v3, v5, v9, v11
    all_migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS); // v4, v7, v8
    all_migrations.extend_from_slice(iotkit_core_registry::MIGRATIONS); // v6
    all_migrations.extend_from_slice(iotkit_core_publish::MIGRATIONS); // v10
    all_migrations.extend_from_slice(iotkit_core_ops::MIGRATIONS); // v12
    all_migrations.sort_by_key(|m| m.version); // 1,3,4,5,6,7,8,9,10,11,12
    let db_path_for_init = std::path::Path::new(&config.db_path);
    let database_existed_before_open = db_path_for_init.exists();
    let db = match iotkit_core_storage::init_db(db_path_for_init, &all_migrations) {
        Ok(handle) => handle,
        Err(e) => {
            tracing::error!(error = %e, db_path = %config.db_path, "failed to initialize database");
            std::process::exit(1);
        }
    };
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

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let collector_alive = rt.block_on(run(config, db));
    if !collector_alive {
        // R20コメントと同じ方針: プロセスレベルの再起動はsystemdの責務。ここでは非ゼロexitで
        // 「死んでいる」ことを伝えるだけ(正常なctrl_c終了はexit 0のまま区別する)。
        std::process::exit(1);
    }
}

/// ledger::LedgerError → StorageError の橋渡し(gateway起動シーケンス専用ヘルパ)。
/// ledgerクレートはStorageErrorを直接返さないため、ここで包む。起動時失敗はexpectで落とす方針(brief参照)。
fn ledger_to_storage_err(e: iotkit_core_ledger::LedgerError) -> iotkit_core_storage::StorageError {
    // rusqlite::Error::ModuleError requires the "vtab" feature (not enabled here),
    // so ToSqlConversionFailure is used as a generic non-gated carrier (brief: variant
    // name adjusted to whatever the build accepts; intent is "fail loudly at startup").
    iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

/// フォールインループを実行する。戻り値は「プロセスとして正常終了してよいか」
/// (true=正常終了・ctrl_c/全アダプタclose、false=コレクタ死亡/API bind失敗/
/// API専用モードでのAPI異常終了によるfail-fast終了)。
/// `main`はfalseを非ゼロexitに変換し、systemdのプロセス再起動に委ねる(R20と同じ設計方針)。
async fn run(config: config::GatewayConfig, db: iotkit_core_storage::DbHandle) -> bool {
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
    db.with_conn(|conn| Ok(epoch_start::maybe_enqueue_epoch_start(conn)))
        .await
        .expect("epoch_start annotation")
        .expect("epoch_start annotation");
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
    let _publish_task =
        publish_task::spawn_publish_task(db.clone(), health_state.clone(), Duration::from_secs(30));

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
    let (collector, principal_issuer, _collector_handle) =
        iotkit_core_collector::Collector::spawn_composed(
            db.clone(),
            std::sync::Arc::new(iotkit_core_registry::SqliteRegistry),
            256,
        );
    // 取り込みクライアント(D4の第3部品、inproc)はアダプタごとにreceiver-created
    // principalを束縛する。アダプタ側が送れるのはEnvelopeだけ。
    let (ingest_exit_tx, mut ingest_exit_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut ingest_client_task_count = 0usize;

    // rpi_local有効時、位置型デバイスを起動時に登録する(D5経路B: 定義=登録)。
    // hardcoded_rpi_local_targets()と同じ2アドレス(0x60, 0x44)。冪等: 既にalive登録済みならスキップ。
    if config.rpi_local.is_some() {
        db.with_conn(|conn| {
            for (addr, label) in [
                (0x60u8, "MCP9600 thermocouple"),
                (0x44u8, "OPT3001 illuminance"),
            ] {
                let hw = format!("rpi-local:default:i2c:0x{addr:02x}");
                if iotkit_core_ledger::find_alive_by_hardware_id(conn, &hw)
                    .map_err(ledger_to_storage_err)?
                    .is_none()
                {
                    iotkit_core_ledger::insert_device(
                        conn,
                        &iotkit_core_ledger::NewDevice {
                            hardware_id: hw,
                            user_label: Some(label.to_string()),
                            parent: None,
                            kind: iotkit_core_ledger::DeviceKind::Positional,
                            initial_state: iotkit_core_ledger::DeviceState::Active,
                        },
                    )
                    .map_err(ledger_to_storage_err)?;
                }
            }
            Ok(())
        })
        .await
        .expect("positional device registration");
    }

    // R20: 再起動可能な公式アダプタ(BravePI/rpi-local)のみを記録する(D4: 再起動権限は形態①のみ)。
    // 他のAdapterClosedはログのみで再起動しない。
    let mut restart_specs: HashMap<AdapterId, RestartSpec> = HashMap::new();

    // BravePI mainboard adapter
    if let Some(bp) = &config.bravepi {
        let source = format!("bravepi-mainboard:{}", bp.port);
        let principal = principal_issuer.official_adapter(format!("principal:{source}"), source);
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
        match start_bravepi(&mut host, &bp.port, Some(ingest.clone())) {
            Ok(id) => {
                restart_specs.insert(
                    id,
                    RestartSpec::BravePi {
                        port: bp.port.clone(),
                        ingest,
                    },
                );
            }
            Err(e) => {
                tracing::error!(error = %e, port = %bp.port, "Failed to start BravePI mainboard adapter");
                std::process::exit(1);
            }
        }
    } else {
        tracing::info!("BravePI mainboard adapter disabled");
    }

    // RPi local I2C adapter
    if let Some(rpi) = &config.rpi_local {
        let targets = hardcoded_rpi_local_targets();
        tracing::info!(
            bus_path = %rpi.bus_path,
            poll_interval_ms = rpi.poll_interval_ms,
            target_count = targets.len(),
            "rpi-local using hardcoded targets: MCP9600@0x60(K-type), OPT3001@0x44 (until auto-detection #35)"
        );
        let adapter_config = rpi_local_adapter::RpiLocalConfig {
            bus_path: rpi.bus_path.clone(),
            poll_interval_ms: rpi.poll_interval_ms,
            targets,
        };
        let principal =
            principal_issuer.official_adapter("principal:rpi-local:default", "rpi-local:default");
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
        match start_rpi_local(&mut host, adapter_config.clone(), Some(ingest.clone())) {
            Ok(id) => {
                restart_specs.insert(
                    id,
                    RestartSpec::RpiLocal {
                        config: adapter_config,
                        ingest,
                    },
                );
            }
            Err(e) => {
                tracing::error!(error = %e, bus_path = %rpi.bus_path, "Failed to start RPi local adapter");
                std::process::exit(1);
            }
        }
    } else {
        tracing::info!("RPi local adapter disabled");
    }
    drop(ingest_exit_tx);

    // R20: アプリレベル監督(責務台帳)。プロセスレベルはsystemdに委譲。
    let mut tracker = supervision::RestartTracker::new(supervision::RestartPolicy::default());

    // コレクタタスク死亡時、fan-inループをbreakした後にfalseを返して非ゼロexitへ導くフラグ
    // (正常終了=ctrl_c/全アダプタclose はtrueのまま=exit 0)。プロセスレベルの再起動は
    // systemdの責務(R20コメント参照)であり、ここでは「健康でないまま動き続けない」ことだけ担保する。
    let mut collector_alive = true;
    let mut api_failed = false;
    let mut api_shutdown_requested = false;
    let api_only_mode = config.api.enabled && host.is_empty();
    let (tx_restart, mut rx_restart) = tokio::sync::mpsc::unbounded_channel::<AdapterId>();
    let mut pending_restart_count = 0usize;

    // Unified fan-in loop
    loop {
        if host.is_empty()
            && should_stop_after_all_adapter_streams_closed(
                pending_restart_count,
                api_task_running,
                api_only_mode,
            )
        {
            log_fan_in_stop(api_only_mode, api_failed);
            break;
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
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
            Some(id) = rx_restart.recv() => {
                pending_restart_count = pending_restart_count.saturating_sub(1);
                let Some(spec) = restart_specs.get(&id).cloned() else {
                    tracing::warn!(
                        adapter = %id,
                        "Restart timer fired for adapter without restart spec"
                    );
                    continue;
                };
                let restart_result = match &spec {
                    RestartSpec::BravePi { port, ingest } => {
                        start_bravepi(&mut host, port, Some(ingest.clone()))
                    }
                    RestartSpec::RpiLocal { config, ingest } => {
                        start_rpi_local(
                            &mut host,
                            config.clone(),
                            Some(ingest.clone()),
                        )
                    }
                };
                match restart_result {
                    Ok(new_id) => {
                        tracing::info!(adapter = %new_id, "Adapter restarted successfully");
                    }
                    Err(e) => {
                        tracing::error!(
                            adapter = %id, error = %e,
                            "Adapter restart attempt failed"
                        );
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
                                    supervision::schedule_restart_notification(
                                        id,
                                        sleep_for,
                                        tx_restart.clone(),
                                    );
                                }
                                None => {
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
                            api_task_running,
                            api_only_mode,
                        ) {
                            log_fan_in_stop(api_only_mode, api_failed);
                            break;
                        }
                    }
                }
            }
        }
    }

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
    {
        let mut health = health_state.lock().expect("health state mutex poisoned");
        health.api = None;
        health.collector_alive = collector_alive;
    }

    let devices = engine.devices().await;
    tracing::info!(device_count = devices.len(), "Engine state at shutdown");

    !should_exit_nonzero(collector_alive, api_failed)
}

async fn wait_for_api_task(
    api_join: &mut Option<tokio::task::JoinHandle<()>>,
) -> Result<(), tokio::task::JoinError> {
    match api_join {
        Some(join) => join.await,
        None => std::future::pending().await,
    }
}

/// R20: 再起動を許可される公式アダプタ(D4: 再起動権限は形態①のみ)の起動パラメータ。
/// AdapterClosed時、host.deregister後にこのspecから同じ起動パスを再実行する。
#[derive(Clone)]
enum RestartSpec {
    BravePi {
        port: String,
        ingest: IngestClient,
    },
    RpiLocal {
        config: rpi_local_adapter::RpiLocalConfig,
        ingest: IngestClient,
    },
}

/// BravePI mainboard adapterを起動し、hostへ登録する。
/// 起動時と再起動時の両方から呼ばれる共用コードパス。
fn start_bravepi(
    host: &mut AdapterHost,
    port: &str,
    ingest: Option<IngestClient>,
) -> Result<AdapterId, String> {
    let handle = bravepi_mainboard_adapter::task::start(port.to_string(), ingest)
        .map_err(|e| format!("Failed to start BravePI mainboard adapter on {port}: {e}"))?;
    tracing::info!(adapter_id = %handle.id, port = %port, "BravePI mainboard adapter started");
    let parts = handle.into_parts();
    let id = parts.id.clone();
    host.register(parts.id, parts.event_rx, {
        let sh = parts.shutdown;
        move || Box::pin(async move { sh.shutdown().await })
    })?;
    Ok(id)
}

/// RPi local I2C adapterを検証・起動し、hostへ登録する。
/// 起動時と再起動時の両方から呼ばれる共用コードパス。
fn start_rpi_local(
    host: &mut AdapterHost,
    adapter_config: rpi_local_adapter::RpiLocalConfig,
    ingest: Option<IngestClient>,
) -> Result<AdapterId, String> {
    // Preflight: catch driver-level validation before spawning background tasks
    rpi_local_adapter::validate(&adapter_config)
        .map_err(|e| format!("RPi local adapter config validation failed: {e}"))?;
    let handle = rpi_local_adapter::start(adapter_config, ingest)
        .map_err(|e| format!("Failed to start RPi local adapter: {e}"))?;
    tracing::info!(adapter_id = %handle.id, "RPi local adapter started");
    let parts = handle.into_parts();
    let id = parts.id.clone();
    host.register(parts.id, parts.event_rx, {
        let sh = parts.shutdown;
        move || Box::pin(async move { sh.shutdown().await })
    })?;
    Ok(id)
}

fn should_stop_after_all_adapter_streams_closed(
    pending_restart_count: usize,
    api_task_running: bool,
    api_only_mode: bool,
) -> bool {
    pending_restart_count == 0 && (!api_only_mode || !api_task_running)
}

fn should_exit_nonzero(collector_alive: bool, api_failed: bool) -> bool {
    !collector_alive || api_failed
}

fn log_fan_in_stop(api_only_mode: bool, api_failed: bool) {
    match (api_only_mode, api_failed) {
        (true, true) => tracing::error!(
            "control-plane API task exited unexpectedly (API-only mode); exiting for restart"
        ),
        (true, false) => tracing::info!("control-plane API task exited; API-only mode stopping"),
        (false, true) => tracing::error!(
            "All adapter channels closed after control-plane API task failure; exiting for restart"
        ),
        (false, false) => tracing::info!("All adapter channels closed"),
    }
}

/// Hardcoded sensor targets for the v1 RPi4B hardware profile.
///
/// Deployment inventory -- lives in the gateway composition root, not the
/// adapter crate. Replaced by #35 auto-detection or #23 device-config-service.
fn hardcoded_rpi_local_targets() -> Vec<rpi_local_adapter::RpiLocalTarget> {
    use rpi_local_adapter::ThermocoupleType;
    vec![
        rpi_local_adapter::RpiLocalTarget::MCP9600 {
            address: 0x60,
            thermocouple_type: ThermocoupleType::K,
        },
        rpi_local_adapter::RpiLocalTarget::OPT3001 { address: 0x44 },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_in_continues_while_restart_notification_is_pending() {
        assert!(
            !should_stop_after_all_adapter_streams_closed(1, false, false),
            "pending restart timers must keep the fan-in loop alive"
        );
        assert!(
            should_stop_after_all_adapter_streams_closed(0, true, false),
            "normal adapter closure should stop even while the API task is running"
        );
        assert!(
            !should_stop_after_all_adapter_streams_closed(0, true, true),
            "API-only mode must keep the fan-in loop alive until the API task exits"
        );
        assert!(
            should_stop_after_all_adapter_streams_closed(0, false, true),
            "the fan-in loop may stop only when no restart is pending"
        );
    }

    #[test]
    fn run_exit_status_reflects_collector_and_api_failures() {
        assert!(
            !should_exit_nonzero(true, false),
            "ctrl_c and normal adapter closure should exit successfully"
        );
        assert!(
            should_exit_nonzero(false, false),
            "collector death remains fail-fast"
        );
        assert!(
            should_exit_nonzero(true, true),
            "unexpected API task exit in API-only mode should be fail-fast"
        );
    }
}
