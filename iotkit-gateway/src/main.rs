//! iotkit-gateway: composition root。
//! adapter を起動し、core/engine に event を渡す。

mod adapter_host;
mod bridge;
mod config;
mod supervision;

use std::collections::HashMap;

use adapter_host::{AdapterHost, AdapterHostEvent};
use iotkit_core_engine::Engine;
use iotkit_core_types::{AdapterEvent, AdapterId};
use tracing_subscriber::EnvFilter;

fn main() {
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
    all_migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS); // v3, v5
    all_migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS); // v2, v4
    all_migrations.extend_from_slice(iotkit_core_registry::MIGRATIONS); // v6
    all_migrations.sort_by_key(|m| m.version); // 1,2,3,4,5,6
    let db = match iotkit_core_storage::init_db(std::path::Path::new(&config.db_path), &all_migrations) {
        Ok(handle) => handle,
        Err(e) => {
            tracing::error!(error = %e, db_path = %config.db_path, "failed to initialize database");
            std::process::exit(1);
        }
    };
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

/// フォールインループを実行する。戻り値は「コレクタタスクが生きたまま終了したか」
/// (true=正常終了・ctrl_c/全アダプタclose、false=コレクタ死亡によるfail-fast終了)。
/// `main`はfalseを非ゼロexitに変換し、systemdのプロセス再起動に委ねる(R20と同じ設計方針)。
async fn run(config: config::GatewayConfig, db: iotkit_core_storage::DbHandle) -> bool {
    let engine = Engine::new();
    let mut host = AdapterHost::new();

    // Ingest collector: fan-inループのSensorData分岐が経由する耐久点(D1)。
    // 受理判定はD6判別表(SqliteRegistry=現場レジストリ参照、計画2)。
    let (collector, _collector_handle) = iotkit_core_collector::Collector::spawn(
        db.clone(),
        std::sync::Arc::new(iotkit_core_registry::SqliteRegistry),
        256,
    );

    // rpi_local有効時、位置型デバイスを起動時に登録する(D5経路B: 定義=登録)。
    // hardcoded_rpi_local_targets()と同じ2アドレス(0x60, 0x44)。冪等: 既にalive登録済みならスキップ。
    if config.rpi_local.is_some() {
        db.with_conn(|conn| {
            for (addr, label) in [(0x60u8, "MCP9600 thermocouple"), (0x44u8, "OPT3001 illuminance")] {
                let hw = format!("rpi-local:default:i2c:0x{addr:02x}");
                if iotkit_core_ledger::find_alive_by_hardware_id(conn, &hw)
                    .map_err(ledger_to_storage_err)?
                    .is_none()
                {
                    iotkit_core_ledger::insert_device(conn, &iotkit_core_ledger::NewDevice {
                        hardware_id: hw,
                        user_label: Some(label.to_string()),
                        parent: None,
                        kind: iotkit_core_ledger::DeviceKind::Positional,
                        initial_state: iotkit_core_ledger::DeviceState::Active,
                    }).map_err(ledger_to_storage_err)?;
                }
            }
            Ok(())
        }).await.expect("positional device registration");
    }

    // R20: 再起動可能な公式アダプタ(BravePI/rpi-local)のみを記録する(D4: 再起動権限は形態①のみ)。
    // 他のAdapterClosedはログのみで再起動しない。
    let mut restart_specs: HashMap<AdapterId, RestartSpec> = HashMap::new();

    // BravePI mainboard adapter
    if let Some(bp) = &config.bravepi {
        match start_bravepi(&mut host, &bp.port) {
            Ok(id) => {
                restart_specs.insert(id, RestartSpec::BravePi { port: bp.port.clone() });
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
        match start_rpi_local(&mut host, adapter_config.clone()) {
            Ok(id) => {
                restart_specs.insert(id, RestartSpec::RpiLocal { config: adapter_config });
            }
            Err(e) => {
                tracing::error!(error = %e, bus_path = %rpi.bus_path, "Failed to start RPi local adapter");
                std::process::exit(1);
            }
        }
    } else {
        tracing::info!("RPi local adapter disabled");
    }

    // R20: アプリレベル監督(責務台帳)。プロセスレベルはsystemdに委譲。
    let mut tracker = supervision::RestartTracker::new(supervision::RestartPolicy::default());

    // コレクタタスク死亡時、fan-inループをbreakした後にfalseを返して非ゼロexitへ導くフラグ
    // (正常終了=ctrl_c/全アダプタclose はtrueのまま=exit 0)。プロセスレベルの再起動は
    // systemdの責務(R20コメント参照)であり、ここでは「健康でないまま動き続けない」ことだけ担保する。
    let mut collector_alive = true;

    // Unified fan-in loop
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
                break;
            }
            event = host.next_event() => {
                match event {
                    Some(AdapterHostEvent::Event(ev)) => {
                        tracing::debug!(
                            adapter = %ev.adapter_id,
                            event = ?ev.event,
                            "Adapter event"
                        );

                        // Extract sensor data BEFORE engine consumes the event
                        let sensor_data = match &ev.event {
                            AdapterEvent::SensorData {
                                device_key, reading, rssi, battery_pct, ..
                            } => Some((
                                ev.adapter_id.clone(),
                                device_key.clone(),
                                reading.clone(),
                                *rssi,
                                *battery_pct,
                            )),
                            _ => None,
                        };

                        // engine.apply(ev) は従来どおり(projectionは旧語彙のまま=D5「engineはWave 0無改修」)
                        engine.apply(ev).await;

                        if let Some((adapter_id, device_key, reading, rssi, battery_pct)) = sensor_data {
                            // R20: 正常受信のたびに再起動カウンタをリセットする(簡略化: HashMap::removeは冪等で安価)。
                            tracker.note_healthy(&adapter_id);

                            if let Some(envelope) = bridge::adapter_event_to_envelope(&adapter_id, &device_key, &reading, rssi, battery_pct) {
                                match tokio::time::timeout(std::time::Duration::from_secs(5), collector.submit(envelope)).await {
                                    Ok(Ok(ack)) => {
                                        if !matches!(ack.status, iotkit_ingest_contract::AckStatus::Accepted { .. }
                                            | iotkit_ingest_contract::AckStatus::Duplicate)
                                        {
                                            tracing::warn!(?ack.status, "ingest not accepted");
                                        }
                                    }
                                    Ok(Err(_)) => {
                                        // コレクタタスクが死んでいる = 取り込み全損。プロセスを
                                        // 「健康」なまま動かし続けるとサイレントにデータを失い続ける
                                        // ので、fan-inループをbreakして非ゼロexitへ倒す(プロセス
                                        // レベルの再起動はsystemdの責務)。
                                        tracing::error!("collector closed; aborting fan-in loop for process restart (systemd)");
                                        collector_alive = false;
                                        break;
                                    }
                                    Err(_) => tracing::error!("collector ack timeout (5s)"), // D1: ackタイムアウト必須
                                }
                            }
                        }
                    }
                    Some(AdapterHostEvent::AdapterClosed(id)) => {
                        // Closed済みアダプタをまず登録簿から除去する(除去しないと同一IDでの
                        // 再registerがadapter_host::registerの重複拒否に阻まれる)。
                        host.deregister(&id);

                        match restart_specs.get(&id).cloned() {
                            Some(spec) => match tracker.next_delay(&id) {
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
                                    tokio::time::sleep(sleep_for).await;

                                    let restart_result = match &spec {
                                        RestartSpec::BravePi { port } => start_bravepi(&mut host, port),
                                        RestartSpec::RpiLocal { config } => {
                                            start_rpi_local(&mut host, config.clone())
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
                        tracing::info!("All adapter channels closed");
                        break;
                    }
                }
            }
        }
    }

    host.shutdown_all().await;

    let devices = engine.devices().await;
    tracing::info!(device_count = devices.len(), "Engine state at shutdown");

    collector_alive
}

/// R20: 再起動を許可される公式アダプタ(D4: 再起動権限は形態①のみ)の起動パラメータ。
/// AdapterClosed時、host.deregister後にこのspecから同じ起動パスを再実行する。
#[derive(Clone)]
enum RestartSpec {
    BravePi { port: String },
    RpiLocal { config: rpi_local_adapter::RpiLocalConfig },
}

/// BravePI mainboard adapterを起動し、hostへ登録する。
/// 起動時と再起動時の両方から呼ばれる共用コードパス。
fn start_bravepi(host: &mut AdapterHost, port: &str) -> Result<AdapterId, String> {
    let handle = bravepi_mainboard_adapter::task::start(port.to_string())
        .map_err(|e| format!("Failed to start BravePI mainboard adapter on {port}: {e}"))?;
    tracing::info!(adapter_id = %handle.id, port = %port, "BravePI mainboard adapter started");
    let parts = handle.into_parts();
    let id = parts.id.clone();
    host.register(
        parts.id,
        parts.event_rx,
        {
            let sh = parts.shutdown;
            move || Box::pin(async move { sh.shutdown().await })
        },
    )?;
    Ok(id)
}

/// RPi local I2C adapterを検証・起動し、hostへ登録する。
/// 起動時と再起動時の両方から呼ばれる共用コードパス。
fn start_rpi_local(
    host: &mut AdapterHost,
    adapter_config: rpi_local_adapter::RpiLocalConfig,
) -> Result<AdapterId, String> {
    // Preflight: catch driver-level validation before spawning background tasks
    rpi_local_adapter::validate(&adapter_config)
        .map_err(|e| format!("RPi local adapter config validation failed: {e}"))?;
    let handle = rpi_local_adapter::start(adapter_config)
        .map_err(|e| format!("Failed to start RPi local adapter: {e}"))?;
    tracing::info!(adapter_id = %handle.id, "RPi local adapter started");
    let parts = handle.into_parts();
    let id = parts.id.clone();
    host.register(
        parts.id,
        parts.event_rx,
        {
            let sh = parts.shutdown;
            move || Box::pin(async move { sh.shutdown().await })
        },
    )?;
    Ok(id)
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
        rpi_local_adapter::RpiLocalTarget::OPT3001 {
            address: 0x44,
        },
    ]
}
