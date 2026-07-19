use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct AdapterHealth {
    pub id: String,
    pub alive: bool,
    pub status: AdapterRuntimeStatus,
    pub last_event_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterRuntimeStatus {
    Running,
    Restarting,
    Exhausted,
    Stopped,
}

impl AdapterRuntimeStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Restarting => "restarting",
            Self::Exhausted => "exhausted",
            Self::Stopped => "stopped",
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressQueryState {
    Ok,
    Degraded,
    Unknown,
}

impl IngressQueryState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Degraded => "degraded",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct IngressListenerHealth {
    pub query_state: IngressQueryState,
    pub status: &'static str,
    pub desired_generation: Option<u64>,
    pub applied_generation: Option<u64>,
    pub bind: Option<&'static str>,
    pub local_addr: Option<String>,
    pub mode: Option<&'static str>,
    pub desired_mode: Option<&'static str>,
    pub applied_mode: Option<&'static str>,
    pub plaintext_warning: bool,
    pub last_error: Option<String>,
    pub last_action: String,
    pub gate_reason: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct IngressBoundsHealth {
    pub query_state: IngressQueryState,
    pub throttled_drop_count: u64,
    pub throttle_active: bool,
    pub queue_current: usize,
    pub queue_high_water: usize,
    pub queue_pressure_percent: u8,
    pub auth_pressure_percent: u8,
    pub global_flow_pressure_percent: u8,
    pub principal_pressure_percent: u8,
    pub request_pressure_percent: u8,
    pub connection_pressure_percent: u8,
    pub staging_rows: Option<u64>,
    pub staging_bytes: Option<u64>,
    pub staging_pinned_rows: Option<u64>,
    pub dedup_rows: Option<u64>,
    pub dedup_max_principal_rows: Option<u64>,
    pub dedup_degraded: Option<bool>,
}

impl IngressBoundsHealth {
    fn unknown() -> Self {
        Self {
            query_state: IngressQueryState::Unknown,
            throttled_drop_count: 0,
            throttle_active: false,
            queue_current: 0,
            queue_high_water: 0,
            queue_pressure_percent: 0,
            auth_pressure_percent: 0,
            global_flow_pressure_percent: 0,
            principal_pressure_percent: 0,
            request_pressure_percent: 0,
            connection_pressure_percent: 0,
            staging_rows: None,
            staging_bytes: None,
            staging_pinned_rows: None,
            dedup_rows: None,
            dedup_max_principal_rows: None,
            dedup_degraded: None,
        }
    }
}

impl IngressListenerHealth {
    pub fn listening(config: iotkit_core_ops::IngressListenerConfig, degraded: bool) -> Self {
        Self::listening_at(config, degraded, None)
    }

    pub fn listening_at(
        config: iotkit_core_ops::IngressListenerConfig,
        degraded: bool,
        local_addr: Option<std::net::SocketAddr>,
    ) -> Self {
        let mode = ingress_mode(config.desired.mode);
        Self {
            query_state: if degraded {
                IngressQueryState::Degraded
            } else {
                IngressQueryState::Ok
            },
            status: if degraded { "degraded" } else { "listening" },
            desired_generation: Some(config.desired.generation),
            applied_generation: Some(config.desired.generation),
            bind: Some("private_site"),
            local_addr: local_addr.map(|address| address.to_string()),
            mode: Some(mode),
            desired_mode: Some(mode),
            applied_mode: Some(mode),
            plaintext_warning: degraded,
            last_error: None,
            last_action: "listening".into(),
            gate_reason: None,
        }
    }
    pub fn disabled(generation: u64, last_action: String) -> Self {
        Self {
            query_state: IngressQueryState::Ok,
            status: "disabled",
            desired_generation: Some(generation),
            applied_generation: Some(generation),
            bind: None,
            local_addr: None,
            mode: None,
            desired_mode: None,
            applied_mode: None,
            plaintext_warning: false,
            last_error: None,
            last_action,
            gate_reason: None,
        }
    }
    pub fn blocked(config: iotkit_core_ops::IngressListenerConfig, reason: &str) -> Self {
        let desired_mode = ingress_mode(config.desired.mode);
        let applied_generation = config.applied.as_ref().map(|state| state.generation);
        let applied_mode = config
            .applied
            .as_ref()
            .map(|state| ingress_mode(state.mode));
        Self {
            query_state: IngressQueryState::Degraded,
            status: "error",
            desired_generation: Some(config.desired.generation),
            applied_generation,
            bind: applied_generation.map(|_| "private_site"),
            local_addr: None,
            mode: applied_mode,
            desired_mode: Some(desired_mode),
            applied_mode,
            plaintext_warning: applied_mode == Some("private_plaintext"),
            last_error: Some(reason.into()),
            last_action: config.last_action,
            gate_reason: Some(reason.into()),
        }
    }
    /// Runtime authority was invalidated and the transport was dropped. Desired state remains
    /// useful for repair, but no formerly applied bind/mode may survive in health.
    pub fn invalidated(config: iotkit_core_ops::IngressListenerConfig, reason: &str) -> Self {
        Self {
            query_state: IngressQueryState::Degraded,
            status: "unbound",
            desired_generation: Some(config.desired.generation),
            applied_generation: None,
            bind: None,
            local_addr: None,
            mode: None,
            desired_mode: Some(ingress_mode(config.desired.mode)),
            applied_mode: None,
            plaintext_warning: false,
            last_error: Some(reason.into()),
            last_action: "runtime_invalidated".into(),
            gate_reason: Some(reason.into()),
        }
    }
    pub fn blocked_unknown(reason: String) -> Self {
        Self {
            query_state: IngressQueryState::Degraded,
            status: "unbound",
            desired_generation: None,
            applied_generation: None,
            bind: None,
            local_addr: None,
            mode: None,
            desired_mode: None,
            applied_mode: None,
            plaintext_warning: false,
            last_error: Some(reason.clone()),
            last_action: "gate_blocked".into(),
            gate_reason: Some(reason),
        }
    }
    pub fn unknown(reason: &str) -> Self {
        Self {
            query_state: IngressQueryState::Unknown,
            status: "unknown",
            desired_generation: None,
            applied_generation: None,
            bind: None,
            local_addr: None,
            mode: None,
            desired_mode: None,
            applied_mode: None,
            plaintext_warning: false,
            last_error: Some(reason.into()),
            last_action: "query_failed".into(),
            gate_reason: Some("unknown".into()),
        }
    }
}

fn ingress_mode(mode: iotkit_core_ops::IngressListenerMode) -> &'static str {
    match mode {
        iotkit_core_ops::IngressListenerMode::Tls => "tls",
        iotkit_core_ops::IngressListenerMode::PrivatePlaintext => "private_plaintext",
    }
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
    pub ingress: IngressListenerHealth,
    pub ingress_bounds: IngressBoundsHealth,
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
            ingress: IngressListenerHealth::unknown("not_observed"),
            ingress_bounds: IngressBoundsHealth::unknown(),
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

    pub fn apply_ingress_persistence_health(
        &mut self,
        staging: iotkit_core_timeseries::StagingHealth,
        dedup: iotkit_core_timeseries::DedupHealth,
        maintenance: iotkit_core_timeseries::DedupMaintenanceHealth,
    ) {
        self.ingress_bounds.query_state = if maintenance.degraded {
            IngressQueryState::Degraded
        } else {
            IngressQueryState::Ok
        };
        self.ingress_bounds.staging_rows = Some(staging.rows);
        self.ingress_bounds.staging_bytes = Some(staging.bytes);
        self.ingress_bounds.staging_pinned_rows = Some(staging.pinned_rows);
        self.ingress_bounds.dedup_rows = Some(dedup.rows);
        self.ingress_bounds.dedup_max_principal_rows = Some(dedup.max_principal_rows);
        self.ingress_bounds.dedup_degraded = Some(maintenance.degraded);
    }

    pub fn mark_ingress_persistence_health_failed(&mut self) {
        self.ingress_bounds.query_state = match self.ingress_bounds.query_state {
            IngressQueryState::Ok | IngressQueryState::Degraded => IngressQueryState::Degraded,
            IngressQueryState::Unknown => IngressQueryState::Unknown,
        };
        if self.ingress_bounds.dedup_degraded == Some(false) {
            self.ingress_bounds.dedup_degraded = None;
        }
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
                adapter.status = AdapterRuntimeStatus::Running;
                adapter.last_event_at = Some(at_ms);
            }
            None => self.adapters.push(AdapterHealth {
                id: id.to_string(),
                alive: true,
                status: AdapterRuntimeStatus::Running,
                last_event_at: Some(at_ms),
            }),
        }
    }

    pub fn note_adapter_running(&mut self, id: &str) {
        self.note_adapter_status(id, AdapterRuntimeStatus::Running);
    }

    pub fn note_adapter_restarting(&mut self, id: &str) {
        self.note_adapter_status(id, AdapterRuntimeStatus::Restarting);
    }

    pub fn note_adapter_exhausted(&mut self, id: &str) {
        self.note_adapter_status(id, AdapterRuntimeStatus::Exhausted);
    }

    pub fn note_adapter_closed(&mut self, id: &str) {
        self.note_adapter_status(id, AdapterRuntimeStatus::Stopped);
    }

    fn note_adapter_status(&mut self, id: &str, status: AdapterRuntimeStatus) {
        let alive = status == AdapterRuntimeStatus::Running;
        match self.adapters.iter_mut().find(|a| a.id == id) {
            Some(adapter) => {
                adapter.alive = alive;
                adapter.status = status;
            }
            None => self.adapters.push(AdapterHealth {
                id: id.to_string(),
                alive,
                status,
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
                        iotkit_core_timeseries::staging_health(
                            conn,
                            iotkit_core_timeseries::StagingLimits::default(),
                        ),
                        iotkit_core_timeseries::dedup_health(
                            conn,
                            iotkit_core_timeseries::DedupLimits::default(),
                        ),
                        iotkit_core_timeseries::dedup_maintenance_health(conn),
                    ))
                })
                .await
            {
                Ok((
                    Ok(stale),
                    Ok(capacity),
                    Ok(backup),
                    Ok(staging),
                    Ok(dedup),
                    Ok(maintenance),
                )) => {
                    snapshot.apply_device_credential_health(stale, capacity, backup);
                    snapshot.apply_ingress_persistence_health(staging, dedup, maintenance);
                    let mut shared = state.lock().expect("health state mutex poisoned");
                    shared.device_credentials = snapshot.device_credentials;
                    shared.ingress_bounds = snapshot.ingress_bounds;
                }
                Ok(_) | Err(_) => {
                    snapshot.mark_device_credential_health_failed();
                    snapshot.mark_ingress_persistence_health_failed();
                    let mut shared = state.lock().expect("health state mutex poisoned");
                    shared.mark_device_credential_health_failed();
                    shared.mark_ingress_persistence_health_failed();
                    tracing::error!("device credential or ingress bound health query failed");
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
        .take(64)
        .map(|adapter| {
            serde_json::json!({"id":bounded_health_text(&adapter.id),"alive":adapter.alive,
            "status":adapter.status.as_str(),"last_event_at":adapter.last_event_at})
        })
        .collect::<Vec<_>>();
    let publish = state
        .publish
        .iter()
        .take(64)
        .map(|target| serde_json::json!({"target_id":bounded_health_text(&target.target_id),"cursor_pub_seq":target.cursor_pub_seq,
            "backlog":target.backlog,"last_push_at":target.last_push_at,
            "last_error":target.last_error.as_deref().map(bounded_health_text)}))
        .collect::<Vec<_>>();
    let api = state.api.as_ref().map(|api| {
        serde_json::json!({"bind":bounded_health_text(&api.bind),
            "tls_fingerprint":bounded_health_text(&api.tls_fingerprint)})
    });
    let replacement_action = match state.device_credentials.replacement_backup_unavailable {
        Some(true) => Some(iotkit_core_ops::device_credentials::REPLACEMENT_BACKUP_ACTION),
        Some(false) => None,
        None => Some(
            "Credential health is unknown; do not rely on replacement backup safety. Install Plan 6.5 encrypted replacement backup support, then create a complete encrypted replacement backup.",
        ),
    };
    let ingress_recovery_action = match state.ingress_bounds.dedup_degraded {
        Some(true) => Some(
            "Check database storage and run retention maintenance; verify dedup health returns to ok before relying on the full duplicate-suppression window.",
        ),
        Some(false) => None,
        None => Some(
            "Inspect database health and run retention maintenance; ingress staging/dedup state is unknown.",
        ),
    };
    let capacity_debt_action = (state.device_credentials.capacity_debt == Some(true)).then_some(
        "Run the construction-tier device capacity dry-run, then increase ingress capacity or reduce assigned device flow classes before clearing capacity debt.",
    );
    serde_json::to_string(&serde_json::json!({
        "schema":1,"written_at":now_ms(),"epoch":bounded_health_text(epoch),"uptime_s":state.started_at.elapsed().as_secs(),
        "collector_alive":state.collector_alive,"adapters":adapters,
        "db":{"size_bytes":state.db.size_bytes,"disk_available_bytes":state.db.disk_available_bytes,"watermark_exceeded":state.db.watermark_exceeded},
        "retention":{"days":state.retention.days,"last_purge_at":state.retention.last_purge_at,"last_purged_rows":state.retention.last_purged_rows},
        "publish":publish,"api":api,
        "ingress_listener":{"query_state":state.ingress.query_state.as_str(),"status":state.ingress.status,
            "desired_generation":state.ingress.desired_generation,"applied_generation":state.ingress.applied_generation,
            "bind":state.ingress.bind,"mode":state.ingress.mode,
            "local_addr":state.ingress.local_addr.as_deref(),
            "desired_mode":state.ingress.desired_mode,"applied_mode":state.ingress.applied_mode,
            "plaintext_warning":state.ingress.plaintext_warning,
            "last_error":state.ingress.last_error.as_deref().map(bounded_health_text),
            "last_action":bounded_health_text(&state.ingress.last_action),
            "gate_reason":state.ingress.gate_reason.as_deref().map(bounded_health_text)},
        "ingress_bounds":{"query_state":state.ingress_bounds.query_state.as_str(),
            "throttled_drop_count":state.ingress_bounds.throttled_drop_count,
            "throttle_active":state.ingress_bounds.throttle_active,
            "queue_current":state.ingress_bounds.queue_current,
            "queue_high_water":state.ingress_bounds.queue_high_water,
            "class_pressure_percent":{"auth":state.ingress_bounds.auth_pressure_percent,
                "global_flow":state.ingress_bounds.global_flow_pressure_percent,
                "principal":state.ingress_bounds.principal_pressure_percent,
                "queue":state.ingress_bounds.queue_pressure_percent,
                "request":state.ingress_bounds.request_pressure_percent,
                "connection":state.ingress_bounds.connection_pressure_percent},
            "staging":{"rows":state.ingress_bounds.staging_rows,"bytes":state.ingress_bounds.staging_bytes,
                "pinned_rows":state.ingress_bounds.staging_pinned_rows},
            "dedup":{"rows":state.ingress_bounds.dedup_rows,
                "max_principal_rows":state.ingress_bounds.dedup_max_principal_rows,
                "degraded":state.ingress_bounds.dedup_degraded},
            "recovery_action":ingress_recovery_action},
        "clock_trust":{"trusted":state.clock.trusted,"source":state.clock.source,"observed_at_ms":state.clock.observed_at_ms,"recovery_command":"iotkit-edgectl time confirm"},
        "device_credentials":{"query_state":state.device_credentials.query_state.as_str(),
            "active_count":state.device_credentials.active_count,"stale_count":state.device_credentials.stale_count,
            "counts_capped":state.device_credentials.counts_capped,
            "capacity":{"required_steady_units":state.device_credentials.capacity_required_steady,
                "required_burst_units":state.device_credentials.capacity_required_burst,
                "steady_units":state.device_credentials.capacity_steady,"burst_units":state.device_credentials.capacity_burst,
                "debt":state.device_credentials.capacity_debt,"debt_action":capacity_debt_action},
            "replacement_backup_unavailable":state.device_credentials.replacement_backup_unavailable,
            "recovery_action":replacement_action}
    })).expect("health document contains only JSON-safe values")
}

fn bounded_health_text(value: &str) -> &str {
    if value.len() <= 256 {
        value
    } else {
        "[TRUNCATED]"
    }
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
        all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
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
                status: AdapterRuntimeStatus::Running,
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
            ingress: IngressListenerHealth::disabled(0, "disabled".into()),
            ingress_bounds: IngressBoundsHealth::unknown(),
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
            "iotkit-edgectl time confirm"
        );
        assert!(json.contains(r#""uptime_s":10"#) || json.contains(r#""uptime_s":11"#));
    }

    #[test]
    fn adapter_health_separates_runtime_state_from_last_activity() {
        let mut state = HealthState::new(90);
        state.note_adapter_running("line_a");
        assert_eq!(state.adapters[0].status, AdapterRuntimeStatus::Running);
        assert_eq!(state.adapters[0].last_event_at, None);

        state.note_adapter_event("line_a", 42);
        state.note_adapter_restarting("line_a");
        assert_eq!(state.adapters[0].status, AdapterRuntimeStatus::Restarting);
        assert_eq!(state.adapters[0].last_event_at, Some(42));

        state.note_adapter_exhausted("line_a");
        assert_eq!(state.adapters[0].status, AdapterRuntimeStatus::Exhausted);
        assert!(!state.adapters[0].alive);
    }

    #[test]
    fn ingress_health_is_stable_coarse_and_secret_free() {
        let mut state = HealthState::new(90);
        state.ingress = IngressListenerHealth {
            query_state: IngressQueryState::Degraded,
            status: "degraded",
            desired_generation: Some(7),
            applied_generation: Some(6),
            bind: Some("private_site"),
            local_addr: None,
            mode: Some("private_plaintext"),
            desired_mode: Some("tls"),
            applied_mode: Some("private_plaintext"),
            plaintext_warning: true,
            last_error: Some("bind_failed".into()),
            last_action: "retain_last_safe".into(),
            gate_reason: Some("tls_not_ready".into()),
        };
        let rendered = render_health_json("epoch", &state);
        let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(json["ingress_listener"]["desired_generation"], 7);
        assert_eq!(json["ingress_listener"]["applied_generation"], 6);
        assert_eq!(json["ingress_listener"]["bind"], "private_site");
        assert_eq!(json["ingress_listener"]["plaintext_warning"], true);
        assert_eq!(json["ingress_listener"]["gate_reason"], "tls_not_ready");
        assert!(!rendered.contains("PRIVATE KEY"));
        assert!(!rendered.contains("192.168."));
    }

    #[test]
    fn failed_tls_desired_with_plaintext_applied_reports_active_plaintext() {
        let config = ingress_config(
            iotkit_core_ops::IngressListenerMode::Tls,
            iotkit_core_ops::IngressListenerMode::PrivatePlaintext,
        );
        let health = IngressListenerHealth::blocked(config, "bind_failed");
        assert_eq!(health.desired_mode, Some("tls"));
        assert_eq!(health.applied_mode, Some("private_plaintext"));
        assert!(health.plaintext_warning);
    }

    #[test]
    fn failed_plaintext_desired_with_tls_applied_reports_active_tls() {
        let config = ingress_config(
            iotkit_core_ops::IngressListenerMode::PrivatePlaintext,
            iotkit_core_ops::IngressListenerMode::Tls,
        );
        let health = IngressListenerHealth::blocked(config, "bind_failed");
        assert_eq!(health.desired_mode, Some("private_plaintext"));
        assert_eq!(health.applied_mode, Some("tls"));
        assert!(!health.plaintext_warning);
    }

    #[test]
    fn invalidated_plaintext_and_tls_applied_states_report_desired_only_unbound_health() {
        for mode in [
            iotkit_core_ops::IngressListenerMode::PrivatePlaintext,
            iotkit_core_ops::IngressListenerMode::Tls,
        ] {
            let health = IngressListenerHealth::invalidated(
                ingress_config(mode, mode),
                "authority_invalidated",
            );
            assert_eq!(health.status, "unbound");
            assert_eq!(health.desired_generation, Some(2));
            assert_eq!(health.desired_mode, Some(ingress_mode(mode)));
            assert_eq!(health.applied_generation, None);
            assert_eq!(health.bind, None);
            assert_eq!(health.mode, None);
            assert_eq!(health.applied_mode, None);
            assert!(!health.plaintext_warning);
        }
    }

    fn ingress_config(
        desired_mode: iotkit_core_ops::IngressListenerMode,
        applied_mode: iotkit_core_ops::IngressListenerMode,
    ) -> iotkit_core_ops::IngressListenerConfig {
        let state = |generation, mode| iotkit_core_ops::IngressListenerState {
            generation,
            bind_addr: "192.168.1.2:8444".into(),
            interface: "eth0".into(),
            site_local_cidrs: vec!["192.168.1.0/24".into()],
            mode,
            tls_generation: (mode == iotkit_core_ops::IngressListenerMode::Tls)
                .then_some(generation),
            tls_fingerprint: (mode == iotkit_core_ops::IngressListenerMode::Tls)
                .then(|| "fingerprint".into()),
        };
        iotkit_core_ops::IngressListenerConfig {
            enabled: true,
            desired: state(2, desired_mode),
            applied: Some(state(1, applied_mode)),
            last_error: None,
            last_action: "apply_failed".into(),
        }
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

    #[test]
    fn ingress_bound_health_is_aggregate_actionable_and_serialized_size_is_bounded() {
        let mut state = HealthState::new(90);
        for i in 0_i64..10_000 {
            state.adapters.push(AdapterHealth {
                id: if i == 0 {
                    format!("HOSTILE_MARKER{}", "x".repeat(100_000))
                } else {
                    format!("adapter-{i}")
                },
                alive: true,
                status: AdapterRuntimeStatus::Running,
                last_event_at: Some(i),
            });
            state.publish.push(TargetDeliveryHealth {
                target_id: format!("target-{i}"),
                cursor_pub_seq: i,
                backlog: i,
                last_push_at: None,
                last_error: (i == 0).then(|| format!("HOSTILE_ERROR{}", "y".repeat(100_000))),
            });
        }
        state.ingress_bounds = IngressBoundsHealth {
            query_state: IngressQueryState::Degraded,
            throttled_drop_count: u64::MAX,
            throttle_active: true,
            queue_current: 15,
            queue_high_water: 16,
            queue_pressure_percent: 93,
            auth_pressure_percent: 75,
            global_flow_pressure_percent: 80,
            principal_pressure_percent: 90,
            request_pressure_percent: 93,
            connection_pressure_percent: 87,
            staging_rows: Some(10_000),
            staging_bytes: Some(64 * 1024 * 1024),
            staging_pinned_rows: Some(9_744),
            dedup_rows: Some(100_000),
            dedup_max_principal_rows: Some(10_000),
            dedup_degraded: Some(true),
        };
        state.device_credentials.capacity_debt = Some(true);

        let rendered = render_health_json("epoch", &state);
        let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert!(rendered.len() < 64 * 1024);
        assert_eq!(json["adapters"].as_array().unwrap().len(), 64);
        assert_eq!(json["publish"].as_array().unwrap().len(), 64);
        assert_eq!(json["ingress_bounds"]["throttled_drop_count"], u64::MAX);
        assert_eq!(json["ingress_bounds"]["throttle_active"], true);
        assert_eq!(json["ingress_bounds"]["queue_current"], 15);
        assert_eq!(
            json["ingress_bounds"]["class_pressure_percent"]["principal"],
            90
        );
        assert_eq!(
            json["ingress_bounds"]["class_pressure_percent"]["queue"],
            93
        );
        assert!(
            json["device_credentials"]["capacity"]["debt_action"]
                .as_str()
                .unwrap()
                .contains("construction-tier")
        );
        assert_eq!(json["ingress_bounds"]["dedup"]["degraded"], true);
        assert!(
            json["ingress_bounds"]["recovery_action"]
                .as_str()
                .unwrap()
                .contains("retention")
        );
        assert!(!rendered.contains("token_id"));
        assert!(!rendered.contains("source_id"));
        assert!(!rendered.contains("HOSTILE_MARKER"));
        assert!(!rendered.contains("HOSTILE_ERROR"));
    }

    #[test]
    fn persisted_staging_and_dedup_health_populate_aggregate_state() {
        let mut state = HealthState::new(90);
        state.apply_ingress_persistence_health(
            iotkit_core_timeseries::StagingHealth {
                rows: 7,
                bytes: 700,
                pinned_rows: 2,
                pinned_bytes: 200,
                principals: 3,
            },
            iotkit_core_timeseries::DedupHealth {
                rows: 9,
                max_principal_rows: 4,
                oldest_age_ms: 50,
            },
            iotkit_core_timeseries::DedupMaintenanceHealth {
                degraded: true,
                episode_started_at: Some(10),
                last_failure_at: Some(20),
                last_success_at: None,
            },
        );
        assert_eq!(
            state.ingress_bounds.query_state,
            IngressQueryState::Degraded
        );
        assert_eq!(state.ingress_bounds.staging_rows, Some(7));
        assert_eq!(state.ingress_bounds.dedup_rows, Some(9));
        assert_eq!(state.ingress_bounds.dedup_degraded, Some(true));
    }
}
