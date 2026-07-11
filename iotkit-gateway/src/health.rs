use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct AdapterHealth {
    pub id: String,
    pub alive: bool,
    pub last_event_at: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct DbHealth {
    pub size_bytes: u64,
    pub disk_available_bytes: u64,
    pub watermark_exceeded: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct RetentionHealth {
    pub days: u64,
    pub last_purge_at: Option<i64>,
    pub last_purged_rows: u64,
}

#[derive(Debug, Clone)]
pub struct TargetDeliveryHealth {
    pub target_id: String,
    pub cursor_pub_seq: i64,
    pub backlog: i64,
    pub last_push_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiHealth {
    pub bind: String,
    pub tls_fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct ClockHealth {
    pub trusted: bool,
    pub source: Option<&'static str>,
    pub observed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialHealthQueryState {
    Ok,
    Degraded,
    Unknown,
}

impl CredentialHealthQueryState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Degraded => "degraded",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DeviceCredentialHealth {
    pub query_state: CredentialHealthQueryState,
    pub active_count: Option<u64>,
    pub stale_count: Option<u64>,
    pub counts_capped: Option<bool>,
    pub capacity_required_steady: Option<i64>,
    pub capacity_required_burst: Option<i64>,
    pub capacity_steady: Option<i64>,
    pub capacity_burst: Option<i64>,
    pub capacity_debt: Option<bool>,
    pub replacement_backup_unavailable: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct HealthState {
    pub started_at: Instant,
    pub collector_alive: bool,
    pub adapters: Vec<AdapterHealth>,
    pub db: DbHealth,
    pub retention: RetentionHealth,
    pub publish: Vec<TargetDeliveryHealth>,
    pub api: Option<ApiHealth>,
    pub clock: ClockHealth,
    pub device_credentials: DeviceCredentialHealth,
}

impl HealthState {
    pub fn new(retention_days: u64) -> Self {
        Self {
            started_at: Instant::now(),
            collector_alive: true,
            adapters: Vec::new(),
            db: DbHealth {
                size_bytes: 0,
                disk_available_bytes: 0,
                watermark_exceeded: false,
            },
            retention: RetentionHealth {
                days: retention_days,
                last_purge_at: None,
                last_purged_rows: 0,
            },
            publish: Vec::new(),
            api: None,
            clock: ClockHealth {
                trusted: false,
                source: None,
                observed_at_ms: None,
            },
            device_credentials: DeviceCredentialHealth {
                query_state: CredentialHealthQueryState::Unknown,
                active_count: None,
                stale_count: None,
                counts_capped: None,
                capacity_required_steady: None,
                capacity_required_burst: None,
                capacity_steady: None,
                capacity_burst: None,
                capacity_debt: None,
                replacement_backup_unavailable: None,
            },
        }
    }

    pub fn apply_device_credential_health(
        &mut self,
        stale: iotkit_core_ops::StaleCredentialHealth,
        capacity: iotkit_core_ops::CapacityHealth,
        backup: iotkit_core_ops::device_credentials::ReplacementBackupHealth,
    ) {
        self.device_credentials = DeviceCredentialHealth {
            query_state: CredentialHealthQueryState::Ok,
            active_count: Some(stale.active_count),
            stale_count: Some(stale.stale_count),
            counts_capped: Some(stale.counts_capped),
            capacity_required_steady: Some(capacity.status.required_steady_units),
            capacity_required_burst: Some(capacity.status.required_burst_units),
            capacity_steady: Some(capacity.status.capacity_steady_units),
            capacity_burst: Some(capacity.status.capacity_burst_units),
            capacity_debt: Some(capacity.active_debt),
            replacement_backup_unavailable: Some(backup.replacement_backup_unavailable),
        };
    }

    pub fn mark_device_credential_health_failed(&mut self) {
        self.device_credentials.query_state = match self.device_credentials.query_state {
            CredentialHealthQueryState::Ok | CredentialHealthQueryState::Degraded => {
                CredentialHealthQueryState::Degraded
            }
            CredentialHealthQueryState::Unknown => CredentialHealthQueryState::Unknown,
        };
        if self.device_credentials.capacity_debt == Some(false) {
            self.device_credentials.capacity_debt = None;
        }
        if self.device_credentials.replacement_backup_unavailable == Some(false) {
            self.device_credentials.replacement_backup_unavailable = None;
        }
    }

    pub fn apply_clock_evidence(&mut self, evidence: iotkit_core_ops::ClockEvidence) {
        self.clock = match evidence {
            iotkit_core_ops::ClockEvidence::Untrusted => ClockHealth {
                trusted: false,
                source: None,
                observed_at_ms: None,
            },
            iotkit_core_ops::ClockEvidence::Trusted {
                source,
                observed_at_ms,
            } => ClockHealth {
                trusted: true,
                source: Some(match source {
                    iotkit_core_ops::TrustSource::KernelSync => "kernel_sync",
                    iotkit_core_ops::TrustSource::ManualLocalRoot => "manual_local_root",
                }),
                observed_at_ms: Some(observed_at_ms),
            },
        };
    }

    pub fn note_adapter_event(&mut self, id: &str, at_ms: i64) {
        match self.adapters.iter_mut().find(|a| a.id == id) {
            Some(adapter) => {
                adapter.alive = true;
                adapter.last_event_at = Some(at_ms);
            }
            None => self.adapters.push(AdapterHealth {
                id: id.to_string(),
                alive: true,
                last_event_at: Some(at_ms),
            }),
        }
    }

    pub fn note_adapter_closed(&mut self, id: &str) {
        match self.adapters.iter_mut().find(|a| a.id == id) {
            Some(adapter) => adapter.alive = false,
            None => self.adapters.push(AdapterHealth {
                id: id.to_string(),
                alive: false,
                last_event_at: None,
            }),
        }
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn write_health_json(path: &Path, epoch: &str, state: &HealthState) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temp_path(path);
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(render_health_json(epoch, state).as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub fn spawn_health_writer(
    path: PathBuf,
    epoch: String,
    state: std::sync::Arc<std::sync::Mutex<HealthState>>,
    clock_trust: std::sync::Arc<iotkit_core_ops::ClockTrust>,
    db: iotkit_core_storage::DbHandle,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let mut snapshot = state.lock().expect("health state mutex poisoned").clone();
            snapshot.apply_clock_evidence(clock_trust.evidence());
            match db
                .with_conn(|conn| {
                    Ok((
                        iotkit_core_ops::configured_stale_after_ms(conn).and_then(
                            |stale_after_ms| {
                                iotkit_core_ops::stale_credential_health(
                                    conn,
                                    now_ms(),
                                    stale_after_ms,
                                )
                            },
                        ),
                        iotkit_core_ops::capacity_health(conn),
                        iotkit_core_ops::replacement_backup_health(conn),
                    ))
                })
                .await
            {
                Ok((Ok(stale), Ok(capacity), Ok(backup))) => {
                    snapshot.apply_device_credential_health(stale, capacity, backup);
                    state
                        .lock()
                        .expect("health state mutex poisoned")
                        .device_credentials = snapshot.device_credentials;
                }
                Ok(_) | Err(_) => {
                    snapshot.mark_device_credential_health_failed();
                    state
                        .lock()
                        .expect("health state mutex poisoned")
                        .mark_device_credential_health_failed();
                    tracing::error!("device credential health query failed");
                }
            }
            if let Err(e) = write_health_json(&path, &epoch, &snapshot) {
                tracing::error!(error = %e, path = %path.display(), "health json write failed");
            }
            tokio::time::sleep(interval).await;
        }
    })
}

fn temp_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.tmp", path.display()))
}

pub fn render_health_json(epoch: &str, state: &HealthState) -> String {
    let adapters = state
        .adapters
        .iter()
        .map(|adapter| serde_json::json!({"id":adapter.id,"alive":adapter.alive,"last_event_at":adapter.last_event_at}))
        .collect::<Vec<_>>();
    let publish = state
        .publish
        .iter()
        .map(|target| serde_json::json!({"target_id":target.target_id,"cursor_pub_seq":target.cursor_pub_seq,
            "backlog":target.backlog,"last_push_at":target.last_push_at,"last_error":target.last_error}))
        .collect::<Vec<_>>();
    let api = state
        .api
        .as_ref()
        .map(|api| serde_json::json!({"bind":api.bind,"tls_fingerprint":api.tls_fingerprint}));
    let replacement_action = match state.device_credentials.replacement_backup_unavailable {
        Some(true) => Some(iotkit_core_ops::device_credentials::REPLACEMENT_BACKUP_ACTION),
        Some(false) => None,
        None => Some(
            "Credential health is unknown; do not rely on replacement backup safety. Install Plan 6.5 encrypted replacement backup support, then create a complete encrypted replacement backup.",
        ),
    };
    serde_json::to_string(&serde_json::json!({
        "schema":1,"written_at":now_ms(),"epoch":epoch,"uptime_s":state.started_at.elapsed().as_secs(),
        "collector_alive":state.collector_alive,"adapters":adapters,
        "db":{"size_bytes":state.db.size_bytes,"disk_available_bytes":state.db.disk_available_bytes,"watermark_exceeded":state.db.watermark_exceeded},
        "retention":{"days":state.retention.days,"last_purge_at":state.retention.last_purge_at,"last_purged_rows":state.retention.last_purged_rows},
        "publish":publish,"api":api,
        "clock_trust":{"trusted":state.clock.trusted,"source":state.clock.source,"observed_at_ms":state.clock.observed_at_ms,"recovery_command":"gatewayctl time confirm"},
        "device_credentials":{"query_state":state.device_credentials.query_state.as_str(),
            "active_count":state.device_credentials.active_count,"stale_count":state.device_credentials.stale_count,
            "counts_capped":state.device_credentials.counts_capped,
            "capacity":{"required_steady_units":state.device_credentials.capacity_required_steady,
                "required_burst_units":state.device_credentials.capacity_required_burst,
                "steady_units":state.device_credentials.capacity_steady,"burst_units":state.device_credentials.capacity_burst,
                "debt":state.device_credentials.capacity_debt},
            "replacement_backup_unavailable":state.device_credentials.replacement_backup_unavailable,
            "recovery_action":replacement_action}
    })).expect("health document contains only JSON-safe values")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn all_migrations() -> Vec<iotkit_core_storage::Migration> {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
        all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
        all.sort_by_key(|migration| migration.version);
        all
    }

    #[test]
    fn write_health_json_uses_temp_file_then_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("health.json");
        let tmp = dir.path().join("health.json.tmp");
        let state = HealthState {
            started_at: Instant::now() - Duration::from_secs(10),
            collector_alive: true,
            adapters: vec![AdapterHealth {
                id: "bravepi-mainboard".to_string(),
                alive: true,
                last_event_at: Some(1234),
            }],
            db: DbHealth {
                size_bytes: 42,
                disk_available_bytes: 1024,
                watermark_exceeded: false,
            },
            retention: RetentionHealth {
                days: 90,
                last_purge_at: Some(4567),
                last_purged_rows: 3,
            },
            publish: Vec::new(),
            api: Some(ApiHealth {
                bind: "127.0.0.1:8443".to_string(),
                tls_fingerprint: "sha256:test".to_string(),
            }),
            clock: ClockHealth {
                trusted: false,
                source: None,
                observed_at_ms: None,
            },
            device_credentials: HealthState::new(90).device_credentials,
        };

        write_health_json(&path, "epoch-1", &state).unwrap();

        assert!(path.exists());
        assert!(!tmp.exists());
        let json = std::fs::read_to_string(path).unwrap();
        assert!(json.contains(r#""schema":1"#));
        assert!(json.contains(r#""epoch":"epoch-1""#));
        assert!(json.contains(r#""collector_alive":true"#));
        assert!(json.contains(r#""id":"bravepi-mainboard""#));
        assert!(json.contains(r#""size_bytes":42"#));
        assert!(json.contains(r#""days":90"#));
        assert!(
            json.contains(r#""api":{"bind":"127.0.0.1:8443","tls_fingerprint":"sha256:test"}"#)
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["clock_trust"]["trusted"], false);
        assert!(parsed["clock_trust"]["source"].is_null());
        assert_eq!(
            parsed["clock_trust"]["recovery_command"],
            "gatewayctl time confirm"
        );
        assert!(json.contains(r#""uptime_s":10"#) || json.contains(r#""uptime_s":11"#));
    }

    #[test]
    fn device_credential_health_is_bounded_and_names_plan_6_5_recovery() {
        let mut state = HealthState::new(90);
        state.device_credentials = DeviceCredentialHealth {
            query_state: CredentialHealthQueryState::Ok,
            active_count: Some(10_000),
            stale_count: Some(10_000),
            counts_capped: Some(true),
            capacity_required_steady: Some(120),
            capacity_required_burst: Some(130),
            capacity_steady: Some(100),
            capacity_burst: Some(100),
            capacity_debt: Some(true),
            replacement_backup_unavailable: Some(true),
        };
        let rendered = render_health_json("test-epoch", &state);
        serde_json::from_str::<serde_json::Value>(&rendered).unwrap();
        assert!(rendered.contains(r#""active_count":10000"#));
        assert!(rendered.contains(r#""counts_capped":true"#));
        assert!(rendered.contains(r#""debt":true"#));
        assert!(rendered.contains(r#""replacement_backup_unavailable":true"#));
        assert!(rendered.contains("Install Plan 6.5 encrypted replacement backup support"));
        assert!(!rendered.contains("token_hash"));
        assert!(!rendered.contains("principal_id"));
    }

    #[test]
    fn failed_health_refresh_is_loud_and_preserves_last_good_replacement_alert() {
        let mut state = HealthState::new(90);
        let unknown = render_health_json("epoch", &state);
        let unknown: serde_json::Value = serde_json::from_str(&unknown).unwrap();
        assert_eq!(unknown["device_credentials"]["query_state"], "unknown");
        assert!(unknown["device_credentials"]["replacement_backup_unavailable"].is_null());

        state.device_credentials = DeviceCredentialHealth {
            query_state: CredentialHealthQueryState::Ok,
            active_count: Some(1),
            stale_count: Some(0),
            counts_capped: Some(false),
            capacity_required_steady: Some(1),
            capacity_required_burst: Some(1),
            capacity_steady: Some(1),
            capacity_burst: Some(1),
            capacity_debt: Some(false),
            replacement_backup_unavailable: Some(true),
        };
        state.mark_device_credential_health_failed();
        let degraded: serde_json::Value =
            serde_json::from_str(&render_health_json("epoch", &state)).unwrap();
        assert_eq!(degraded["device_credentials"]["query_state"], "degraded");
        assert_eq!(
            degraded["device_credentials"]["replacement_backup_unavailable"],
            true
        );
        assert!(
            degraded["device_credentials"]["recovery_action"]
                .as_str()
                .unwrap()
                .contains("Plan 6.5")
        );
    }

    #[test]
    fn failed_refresh_drops_last_good_safe_booleans_but_keeps_unsafe_alerts() {
        let mut state = HealthState::new(90);
        state.device_credentials = DeviceCredentialHealth {
            query_state: CredentialHealthQueryState::Ok,
            active_count: Some(0),
            stale_count: Some(0),
            counts_capped: Some(false),
            capacity_required_steady: Some(0),
            capacity_required_burst: Some(0),
            capacity_steady: Some(1),
            capacity_burst: Some(1),
            capacity_debt: Some(false),
            replacement_backup_unavailable: Some(false),
        };
        state.mark_device_credential_health_failed();
        let degraded: serde_json::Value =
            serde_json::from_str(&render_health_json("epoch", &state)).unwrap();
        assert_eq!(degraded["device_credentials"]["query_state"], "degraded");
        assert!(degraded["device_credentials"]["capacity"]["debt"].is_null());
        assert!(degraded["device_credentials"]["replacement_backup_unavailable"].is_null());
        assert!(
            degraded["device_credentials"]["recovery_action"]
                .as_str()
                .unwrap()
                .contains("Plan 6.5")
        );

        state.device_credentials.capacity_debt = Some(true);
        state.device_credentials.replacement_backup_unavailable = Some(true);
        state.mark_device_credential_health_failed();
        let unsafe_snapshot: serde_json::Value =
            serde_json::from_str(&render_health_json("epoch", &state)).unwrap();
        assert_eq!(
            unsafe_snapshot["device_credentials"]["capacity"]["debt"],
            true
        );
        assert_eq!(
            unsafe_snapshot["device_credentials"]["replacement_backup_unavailable"],
            true
        );

        state.apply_device_credential_health(
            iotkit_core_ops::StaleCredentialHealth {
                active_count: 0,
                stale_count: 0,
                counts_capped: false,
            },
            iotkit_core_ops::CapacityHealth {
                status: iotkit_core_ops::CapacityStatus {
                    required_steady_units: 0,
                    required_burst_units: 0,
                    capacity_steady_units: 1,
                    capacity_burst_units: 1,
                },
                active_debt: false,
            },
            iotkit_core_ops::device_credentials::ReplacementBackupHealth {
                replacement_backup_unavailable: false,
                recovery_action: None,
            },
        );
        assert_eq!(state.device_credentials.capacity_debt, Some(false));
        assert_eq!(
            state.device_credentials.replacement_backup_unavailable,
            Some(false)
        );
    }

    #[tokio::test]
    async fn health_writer_database_failure_publishes_unknown_not_false_safe_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("health-failure.db");
        let health_path = dir.path().join("health.json");
        let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
        let trust = db
            .with_conn_sync(|conn| {
                Ok(Arc::new(
                    iotkit_core_ops::ClockTrust::load(
                        conn,
                        Arc::new(iotkit_core_ops::SystemClock::default()),
                        Duration::from_secs(1),
                        Duration::from_secs(60),
                    )
                    .unwrap(),
                ))
            })
            .unwrap();
        db.with_conn_sync(|conn| {
            conn.execute_batch("DROP TABLE device_capacity")?;
            Ok(())
        })
        .unwrap();
        let state = Arc::new(Mutex::new(HealthState::new(90)));
        let task = spawn_health_writer(
            health_path.clone(),
            "epoch".into(),
            state,
            trust,
            db,
            Duration::from_millis(10),
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            while !health_path.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(health_path).unwrap()).unwrap();
        assert_eq!(json["device_credentials"]["query_state"], "unknown");
        assert!(json["device_credentials"]["replacement_backup_unavailable"].is_null());
        assert!(
            json["device_credentials"]["recovery_action"]
                .as_str()
                .unwrap()
                .contains("unknown")
        );
    }
}
