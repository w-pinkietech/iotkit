use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_core_ops::{INGRESS_READY, IngressListenerConfig, IngressListenerMode};
use iotkit_core_storage::DbHandle;
use iotkit_ingest_http::{
    ApplyError, ExposureSnapshot, HttpIngestService, Listener, ListenerConfig, ListenerError,
    ListenerMode, ListenerTransition, LocalIngressCidr, ServingListener, SystemMonotonicClock,
    TlsMaterial, ValidatedListenerConfig,
};

use crate::health::{HealthState, IngressListenerHealth, IngressQueryState};

pub type IngressBindFuture =
    Pin<Box<dyn Future<Output = Result<Listener, ListenerError>> + Send + 'static>>;

/// Composition boundary for the listener's trusted interface inventory and socket ownership.
///
/// The default implementation reads the host inventory and binds the validated local address.
/// Alternate implementations must still return a strictly validated configuration; the socket
/// boundary exists for socket activation and deterministic supervisor composition, not to bypass
/// the private-network validation performed by [`ValidatedListenerConfig::new`].
pub trait IngressComposition: Send + Sync {
    fn exposure(&self, interface: &str) -> Result<ExposureSnapshot, ListenerError>;

    fn bind(&self, config: ValidatedListenerConfig) -> IngressBindFuture;
}

struct OsIngressComposition;

impl IngressComposition for OsIngressComposition {
    fn exposure(&self, interface: &str) -> Result<ExposureSnapshot, ListenerError> {
        ExposureSnapshot::from_os(interface)
    }

    fn bind(&self, config: ValidatedListenerConfig) -> IngressBindFuture {
        Box::pin(Listener::bind(config))
    }
}

fn os_ingress_composition() -> Arc<dyn IngressComposition> {
    Arc::new(OsIngressComposition)
}

pub fn spawn_ingress_supervisor(
    db: DbHandle,
    data_dir: PathBuf,
    health: Arc<Mutex<HealthState>>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    spawn_ingress_supervisor_inner(
        db,
        data_dir,
        health,
        interval,
        None,
        os_ingress_composition(),
    )
}

pub fn spawn_ingress_supervisor_serving(
    db: DbHandle,
    data_dir: PathBuf,
    health: Arc<Mutex<HealthState>>,
    interval: Duration,
    service: HttpIngestService<SystemMonotonicClock>,
) -> tokio::task::JoinHandle<()> {
    spawn_ingress_supervisor_inner(
        db,
        data_dir,
        health,
        interval,
        Some(service),
        os_ingress_composition(),
    )
}

pub fn spawn_ingress_supervisor_serving_with_composition(
    db: DbHandle,
    data_dir: PathBuf,
    health: Arc<Mutex<HealthState>>,
    interval: Duration,
    service: HttpIngestService<SystemMonotonicClock>,
    composition: Arc<dyn IngressComposition>,
) -> tokio::task::JoinHandle<()> {
    spawn_ingress_supervisor_inner(db, data_dir, health, interval, Some(service), composition)
}

fn spawn_ingress_supervisor_inner(
    db: DbHandle,
    data_dir: PathBuf,
    health: Arc<Mutex<HealthState>>,
    interval: Duration,
    service: Option<HttpIngestService<SystemMonotonicClock>>,
    composition: Arc<dyn IngressComposition>,
) -> tokio::task::JoinHandle<()> {
    struct ExitGuard(Arc<Mutex<HealthState>>);
    impl Drop for ExitGuard {
        fn drop(&mut self) {
            note_ingress_task_exit(&self.0, "supervisor_exited");
        }
    }
    tokio::spawn(async move {
        let _exit_guard = ExitGuard(health.clone());
        let custody_dir = data_dir.clone();
        let mut custody_reconciled = db
            .with_conn(move |conn| {
                storage(iotkit_core_ops::reconcile_ingress_tls_custody(
                    conn,
                    &custody_dir,
                ))
            })
            .await
            .is_ok();
        if !custody_reconciled {
            health.lock().expect("health state mutex poisoned").ingress =
                IngressListenerHealth::unknown("tls_custody_reconciliation_failed");
        }
        let mut transition = ListenerTransition::<ServingListener>::default();
        let recovered = if INGRESS_READY && custody_reconciled {
            recover_last_applied(&db, &data_dir, service.as_ref(), composition.clone()).await
        } else {
            None
        };
        if let Some((listener, desired_generation, applied_generation)) = recovered {
            if transition
                .restore_applied(desired_generation, applied_generation, listener)
                .is_err()
            {
                let _ = db
                    .with_conn(|conn| {
                        storage(iotkit_core_ops::mark_ingress_runtime_unbound(
                            conn,
                            "runtime_start",
                        ))
                    })
                    .await;
            }
        } else {
            let _ = db
                .with_conn(|conn| {
                    storage(iotkit_core_ops::mark_ingress_runtime_unbound(
                        conn,
                        "runtime_start",
                    ))
                })
                .await;
        }

        loop {
            if !custody_reconciled {
                let custody_dir = data_dir.clone();
                custody_reconciled = db
                    .with_conn(move |conn| {
                        storage(iotkit_core_ops::reconcile_ingress_tls_custody(
                            conn,
                            &custody_dir,
                        ))
                    })
                    .await
                    .is_ok();
                if !custody_reconciled {
                    health.lock().expect("health state mutex poisoned").ingress =
                        IngressListenerHealth::unknown("tls_custody_reconciliation_failed");
                    tokio::time::sleep(interval).await;
                    continue;
                }
            }
            let observed = observe_authority(&db, &data_dir).await;
            let next = match observed {
                Ok(config) if !config.enabled => {
                    let generation = config.desired.generation;
                    match disable_generation_safely(
                        &db,
                        &data_dir,
                        &config,
                        &mut transition,
                        composition.clone(),
                    )
                    .await
                    {
                        Ok(()) => IngressListenerHealth::disabled(generation, config.last_action),
                        Err("generation_conflict") => {
                            record_apply_error(&db, generation, "generation_conflict").await;
                            IngressListenerHealth::blocked(config, "generation_conflict")
                        }
                        Err(code) => {
                            record_apply_error(&db, generation, code).await;
                            IngressListenerHealth::blocked(config, code)
                        }
                    }
                }
                Ok(config) if !INGRESS_READY => {
                    invalidate_runtime(&db, &mut transition, "ingress_not_ready").await;
                    record_apply_error(&db, config.desired.generation, "ingress_not_ready").await;
                    IngressListenerHealth::invalidated(config, "ingress_not_ready")
                }
                Ok(config) => {
                    let desired_generation = config.desired.generation;
                    if transition
                        .active()
                        .is_some_and(ServingListener::is_finished)
                    {
                        invalidate_runtime(&db, &mut transition, "http_service_exited").await;
                        record_apply_error(&db, desired_generation, "http_service_exited").await;
                        IngressListenerHealth::invalidated(config, "http_service_exited")
                    } else if transition.active().is_some()
                        && transition.applied_generation() == desired_generation
                    {
                        match validate_runtime_config(&config, &data_dir, composition.as_ref()) {
                            Ok(degraded) => IngressListenerHealth::listening_at(
                                config,
                                degraded,
                                transition.active().map(ServingListener::local_addr),
                            ),
                            Err(code) => {
                                invalidate_runtime(&db, &mut transition, code).await;
                                record_apply_error(&db, desired_generation, code).await;
                                IngressListenerHealth::invalidated(config, code)
                            }
                        }
                    } else {
                        let degraded = config.desired.mode == IngressListenerMode::PrivatePlaintext;
                        match apply_generation_safely(
                            &db,
                            &data_dir,
                            &config,
                            &mut transition,
                            service.as_ref(),
                            composition.clone(),
                        )
                        .await
                        {
                            Ok(()) => IngressListenerHealth::listening_at(
                                config,
                                degraded,
                                transition.active().map(ServingListener::local_addr),
                            ),
                            Err(ApplyError::External(code)) => {
                                record_apply_error(&db, desired_generation, code).await;
                                IngressListenerHealth::blocked(config, code)
                            }
                            Err(ApplyError::State(_)) => {
                                record_apply_error(&db, desired_generation, "generation_conflict")
                                    .await;
                                IngressListenerHealth::blocked(config, "generation_conflict")
                            }
                        }
                    }
                }
                Err(reason) => {
                    invalidate_runtime(&db, &mut transition, "authority_invalidated").await;
                    IngressListenerHealth::blocked_unknown(reason)
                }
            };
            if let Some(service) = service.as_ref() {
                let admission = service.admission_health();
                let events = service.pending_throttle_episode_events();
                if !events.is_empty() && persist_throttle_episode_events(&db, events.clone()).await
                {
                    let _ = service.acknowledge_throttle_episode_events(&events);
                }
                let mut state = health.lock().expect("health state mutex poisoned");
                state.ingress_bounds.throttled_drop_count = admission.throttled_drop_count;
                state.ingress_bounds.throttle_active = admission.throttle_active;
                state.ingress_bounds.queue_current = admission.queue_current;
                state.ingress_bounds.queue_high_water = admission.queue_high_water;
                state.ingress_bounds.queue_pressure_percent = admission.queue_pressure_percent;
                state.ingress_bounds.auth_pressure_percent = admission.auth_pressure_percent;
                state.ingress_bounds.global_flow_pressure_percent =
                    admission.global_flow_pressure_percent;
                state.ingress_bounds.principal_pressure_percent =
                    admission.principal_pressure_percent;
                state.ingress_bounds.request_pressure_percent = admission.request_pressure_percent;
                state.ingress_bounds.connection_pressure_percent =
                    admission.connection_pressure_percent;
            }
            health.lock().expect("health state mutex poisoned").ingress = next;
            tokio::time::sleep(interval).await;
        }
    })
}

async fn apply_generation_safely(
    db: &DbHandle,
    data_dir: &Path,
    config: &IngressListenerConfig,
    transition: &mut ListenerTransition<ServingListener>,
    service: Option<&HttpIngestService<SystemMonotonicClock>>,
    composition: Arc<dyn IngressComposition>,
) -> Result<(), ApplyError<&'static str>> {
    let generation = config.desired.generation;
    transition
        .prepare_generation(generation)
        .map_err(ApplyError::State)?;
    let (validated, installed_tls_generation) =
        build_validated_config(config, data_dir, composition.as_ref())
            .map_err(ApplyError::External)?;
    let same_bind = transition.active().is_some_and(|active| {
        let requested = validated.bind_addr();
        let current = active.configured_addr();
        requested.ip() == current.ip()
            && (requested.port() == 0 || requested.port() == current.port())
    });

    if same_bind {
        let active = transition.active().ok_or(ApplyError::State(
            iotkit_ingest_http::TransitionError::GenerationRollback,
        ))?;
        let policy = Listener::stage_policy_for_local_addr(&validated, active.local_addr())
            .map_err(|_| ApplyError::External("http_service_start_failed"))?;
        active
            .pause()
            .await
            .map_err(|_| ApplyError::External("http_service_exited"))?;
        if let Err(code) = publish_applied_if_authorized_with_composition(
            db,
            data_dir,
            config,
            generation,
            installed_tls_generation,
            composition.clone(),
        )
        .await
        {
            active
                .resume()
                .await
                .map_err(|_| ApplyError::External("http_service_exited"))?;
            return Err(ApplyError::External(code));
        }
        let old_policy = active
            .replace_policy(policy)
            .await
            .map_err(|_| ApplyError::External("http_service_exited"))?;
        active
            .resume()
            .await
            .map_err(|_| ApplyError::External("http_service_exited"))?;
        transition
            .commit_reused_generation(generation)
            .map_err(ApplyError::State)?;
        drop(old_policy);
        return Ok(());
    }

    let service = service
        .cloned()
        .ok_or(ApplyError::External("http_service_unavailable"))?;
    let staged = composition
        .bind(validated)
        .await
        .map_err(|_| ApplyError::External("bind_failed"))?;
    let staged = staged
        .serve_paused(service)
        .map_err(|_| ApplyError::External("http_service_start_failed"))?;
    let old_was_paused = if let Some(active) = transition.active() {
        active
            .pause()
            .await
            .map_err(|_| ApplyError::External("http_service_exited"))?;
        true
    } else {
        false
    };

    if let Err(code) = publish_applied_if_authorized_with_composition(
        db,
        data_dir,
        config,
        generation,
        installed_tls_generation,
        composition.clone(),
    )
    .await
    {
        if old_was_paused && let Some(active) = transition.active() {
            active
                .resume()
                .await
                .map_err(|_| ApplyError::External("http_service_exited"))?;
        }
        staged.shutdown().await;
        return Err(ApplyError::External(code));
    }

    if staged.resume().await.is_err() {
        staged.shutdown().await;
        invalidate_runtime(db, transition, "http_service_start_failed").await;
        return Err(ApplyError::External("http_service_start_failed"));
    }
    let old = transition
        .commit_replaced_generation(generation, staged)
        .map_err(ApplyError::State)?;
    if let Some(old) = old {
        old.shutdown().await;
    }
    Ok(())
}

async fn disable_generation_safely(
    db: &DbHandle,
    data_dir: &Path,
    config: &IngressListenerConfig,
    transition: &mut ListenerTransition<ServingListener>,
    composition: Arc<dyn IngressComposition>,
) -> Result<(), &'static str> {
    let generation = config.desired.generation;
    if transition.active().is_none() && transition.applied_generation() == generation {
        return Ok(());
    }
    transition
        .prepare_generation(generation)
        .map_err(|_| "generation_conflict")?;
    let old_was_paused = if let Some(active) = transition.active() {
        active.pause().await.map_err(|_| "http_service_exited")?;
        true
    } else {
        false
    };
    if let Err(code) = publish_applied_if_authorized_with_composition(
        db,
        data_dir,
        config,
        generation,
        None,
        composition,
    )
    .await
    {
        if old_was_paused && let Some(active) = transition.active() {
            active.resume().await.map_err(|_| "http_service_exited")?;
        }
        return Err(code);
    }
    let old = transition
        .commit_disabled_generation(generation)
        .map_err(|_| "generation_conflict")?;
    if let Some(old) = old {
        old.shutdown().await;
    }
    Ok(())
}

async fn recover_last_applied(
    db: &DbHandle,
    data_dir: &Path,
    service: Option<&HttpIngestService<SystemMonotonicClock>>,
    composition: Arc<dyn IngressComposition>,
) -> Option<(ServingListener, u64, u64)> {
    let service = service?.clone();
    let config = observe_authority(db, data_dir).await.ok()?;
    if !config.enabled {
        return None;
    }
    let applied = config.applied.clone()?;
    let mut recovery = config.clone();
    recovery.desired = applied.clone();
    let (validated, _) = build_validated_config(&recovery, data_dir, composition.as_ref()).ok()?;
    let listener = composition.bind(validated).await.ok()?;
    recheck_authority(db, data_dir, &config, &recovery, composition.clone())
        .await
        .ok()?;
    recheck_authority(db, data_dir, &config, &recovery, composition)
        .await
        .ok()?;
    let listener = listener.serve(service).ok()?;
    Some((listener, config.desired.generation, applied.generation))
}

async fn recheck_authority(
    db: &DbHandle,
    data_dir: &Path,
    expected: &IngressListenerConfig,
    runtime: &IngressListenerConfig,
    composition: Arc<dyn IngressComposition>,
) -> Result<(), &'static str> {
    let data_dir = data_dir.to_path_buf();
    let expected = expected.clone();
    let runtime = runtime.clone();
    db.with_conn(move |conn| {
        Ok(recheck_authority_on_conn(
            conn,
            &data_dir,
            &expected,
            &runtime,
            composition.as_ref(),
        ))
    })
    .await
    .map_err(|_| "database_query_failed")?
}

async fn publish_applied_if_authorized_with_composition(
    db: &DbHandle,
    data_dir: &Path,
    expected: &IngressListenerConfig,
    generation: u64,
    tls_generation: Option<u64>,
    composition: Arc<dyn IngressComposition>,
) -> Result<(), &'static str> {
    let data_dir = data_dir.to_path_buf();
    let expected = expected.clone();
    db.with_conn(move |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
        let checked = (|| {
            recheck_authority_on_conn(&tx, &data_dir, &expected, &expected, composition.as_ref())?;
            iotkit_core_ops::mark_ingress_applied_in_transaction(&tx, generation, tls_generation)
                .map_err(|_| "applied_state_write_failed")
        })();
        match checked {
            Ok(()) => {
                tx.commit()?;
                Ok(Ok(()))
            }
            Err(code) => Ok(Err(code)),
        }
    })
    .await
    .map_err(|_| "applied_state_write_failed")?
}

fn recheck_authority_on_conn(
    conn: &rusqlite::Connection,
    data_dir: &Path,
    expected: &IngressListenerConfig,
    runtime: &IngressListenerConfig,
    composition: &dyn IngressComposition,
) -> Result<(), &'static str> {
    crate::network_authority::require_common_network_authority(conn, data_dir)
        .map_err(|_| "authority_invalidated")?;
    crate::api::tls::validate_existing_tls_material(conn, data_dir).map_err(|_| "tls_not_ready")?;
    let current = iotkit_core_ops::load_ingress_listener_config(conn)
        .map_err(|_| "unsafe_ingress_generation_state")?;
    if current.enabled != expected.enabled || current.desired != expected.desired {
        return Err("desired_generation_changed");
    }
    if expected.enabled {
        validate_runtime_config(runtime, data_dir, composition).map(|_| ())?;
    }
    Ok(())
}

async fn observe_authority(
    db: &DbHandle,
    data_dir: &Path,
) -> Result<IngressListenerConfig, String> {
    let data_dir = data_dir.to_path_buf();
    db.with_conn(move |conn| {
        Ok((|| {
            crate::network_authority::require_common_network_authority(conn, &data_dir)
                .map_err(|error| error.reason().to_owned())?;
            crate::api::tls::validate_existing_tls_material(conn, &data_dir)
                .map_err(|_| "tls_not_ready".to_owned())?;
            iotkit_core_ops::load_ingress_listener_config(conn)
                .map_err(|_| "unsafe_ingress_generation_state".to_owned())
        })())
    })
    .await
    .map_err(|_| "database_query_failed".to_owned())?
}

fn validate_runtime_config(
    config: &IngressListenerConfig,
    data_dir: &Path,
    composition: &dyn IngressComposition,
) -> Result<bool, &'static str> {
    build_validated_config(config, data_dir, composition)
        .map(|(validated, _)| validated.is_degraded())
}

fn build_validated_config(
    config: &IngressListenerConfig,
    data_dir: &Path,
    composition: &dyn IngressComposition,
) -> Result<(ValidatedListenerConfig, Option<u64>), &'static str> {
    let state = &config.desired;
    let bind = state.bind_addr.parse().map_err(|_| "invalid_bind")?;
    let local_ingress_cidrs = state
        .local_ingress_cidrs
        .iter()
        .map(|cidr| {
            cidr.parse::<LocalIngressCidr>()
                .map_err(|_| "invalid_local_ingress_cidr")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let exposure = composition
        .exposure(&state.interface)
        .map_err(|_| "inventory_invalid")?;
    let (mode, tls_generation) = match state.mode {
        IngressListenerMode::PrivatePlaintext => (ListenerMode::PrivatePlaintext, None),
        IngressListenerMode::Tls => {
            let generation = state.tls_generation.ok_or("tls_material_missing")?;
            let material =
                validate_ingress_tls_material(config, data_dir)?.ok_or("tls_material_missing")?;
            (ListenerMode::Tls(material), Some(generation))
        }
    };
    let listener = ListenerConfig {
        bind,
        interface: state.interface.clone(),
        local_ingress_cidrs,
        mode,
    };
    ValidatedListenerConfig::new(listener, &exposure)
        .map(|validated| (validated, tls_generation))
        .map_err(|_| "unsafe_ingress_exposure")
}

pub(crate) fn validate_ingress_tls_material(
    config: &IngressListenerConfig,
    data_dir: &Path,
) -> Result<Option<TlsMaterial>, &'static str> {
    let state = &config.desired;
    if state.mode == IngressListenerMode::PrivatePlaintext {
        return Ok(None);
    }
    let generation = state.tls_generation.ok_or("tls_material_missing")?;
    let fingerprint = state
        .tls_fingerprint
        .as_deref()
        .ok_or("tls_material_missing")?;
    let root = data_dir
        .join("ingress-tls")
        .join(format!("generation-{generation}"));
    let cert = std::fs::read(root.join("cert.pem")).map_err(|_| "tls_material_missing")?;
    let key = std::fs::read(root.join("key.pem")).map_err(|_| "tls_material_missing")?;
    TlsMaterial::validate(cert, key, fingerprint, generation)
        .map(Some)
        .map_err(|_| "tls_material_invalid")
}

async fn invalidate_runtime(
    db: &DbHandle,
    transition: &mut ListenerTransition<ServingListener>,
    action: &'static str,
) {
    let _ = transition.invalidate(|_| Ok::<_, ()>(()));
    let _ = db
        .with_conn(move |conn| storage(iotkit_core_ops::mark_ingress_runtime_unbound(conn, action)))
        .await;
}

async fn record_apply_error(db: &DbHandle, generation: u64, code: &'static str) {
    let _ = db
        .with_conn(move |conn| {
            storage(iotkit_core_ops::mark_ingress_apply_error(
                conn, generation, code,
            ))
        })
        .await;
}

async fn persist_throttle_episode_events(
    db: &DbHandle,
    events: Vec<iotkit_ingest_http::ThrottleEpisodeEvent>,
) -> bool {
    db.with_conn(move |conn| {
        let tx = rusqlite::Transaction::new_unchecked(
            conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        for event in events {
            let (kind, detail) = match event {
                iotkit_ingest_http::ThrottleEpisodeEvent::Started { episode_id } => (
                    "ingress_throttle_started",
                    serde_json::json!({"state":"throttled","episode_id":episode_id,"operator_action":"Check ingress capacity debt and queue pressure."}).to_string(),
                ),
                iotkit_ingest_http::ThrottleEpisodeEvent::Recovered { episode_id, drops } => (
                    "ingress_throttle_recovered",
                    serde_json::json!({"state":"recovered","episode_id":episode_id,"throttled_drop_count":drops}).to_string(),
                ),
            };
            iotkit_core_ledger::record_event(&tx, kind, None, &detail).map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(
                    rusqlite::Error::ToSqlConversionFailure(Box::new(error)),
                )
            })?;
        }
        tx.commit()?;
        Ok(())
    })
    .await
    .is_ok()
}

fn storage(
    result: Result<(), iotkit_core_ops::OpsError>,
) -> Result<(), iotkit_core_storage::StorageError> {
    result.map_err(|error| {
        iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
            Box::new(error),
        ))
    })
}

pub fn note_ingress_task_exit(health: &Arc<Mutex<HealthState>>, action: &'static str) {
    health.lock().expect("health state mutex poisoned").ingress = IngressListenerHealth {
        query_state: IngressQueryState::Degraded,
        status: "error",
        desired_generation: None,
        applied_generation: None,
        bind: None,
        local_addr: None,
        mode: None,
        desired_mode: None,
        applied_mode: None,
        plaintext_warning: false,
        last_error: Some("listener_task_exited".into()),
        last_action: action.into(),
        gate_reason: Some("listener_task_exited".into()),
    };
}

#[cfg(test)]
#[path = "../tests/unit/ingress_tests.rs"]
mod tests;
