use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_core_ops::{INGRESS_READY, IngressListenerConfig, IngressListenerMode};
use iotkit_core_storage::DbHandle;
use iotkit_ingest_http::{
    ApplyError, ExposureSnapshot, Listener, ListenerConfig, ListenerMode, ListenerTransition,
    SiteCidr, TlsMaterial, ValidatedListenerConfig,
};

use crate::health::{HealthState, IngressListenerHealth, IngressQueryState};

pub fn spawn_ingress_supervisor(
    db: DbHandle,
    data_dir: PathBuf,
    health: Arc<Mutex<HealthState>>,
    interval: Duration,
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
        let mut transition = ListenerTransition::<Listener>::default();
        let recovered = if INGRESS_READY && custody_reconciled {
            recover_last_applied(&db, &data_dir).await
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
                    if transition
                        .disable_generation(generation, |_| Ok::<_, ()>(()))
                        .is_err()
                    {
                        record_apply_error(&db, generation, "generation_conflict").await;
                        IngressListenerHealth::blocked(config, "generation_conflict")
                    } else {
                        match publish_applied_if_authorized(
                            &db, &data_dir, &config, generation, None,
                        )
                        .await
                        {
                            Ok(()) => {
                                IngressListenerHealth::disabled(generation, config.last_action)
                            }
                            Err(_) => IngressListenerHealth::unknown("applied_state_write_failed"),
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
                    if transition.active().is_some()
                        && transition.applied_generation() == desired_generation
                    {
                        match validate_runtime_config(&config, &data_dir) {
                            Ok(degraded) => IngressListenerHealth::listening(config, degraded),
                            Err(code) => {
                                invalidate_runtime(&db, &mut transition, code).await;
                                record_apply_error(&db, desired_generation, code).await;
                                IngressListenerHealth::invalidated(config, code)
                            }
                        }
                    } else {
                        let tls_generation = match config.desired.mode {
                            IngressListenerMode::Tls => config.desired.tls_generation,
                            IngressListenerMode::PrivatePlaintext => None,
                        };
                        let degraded = config.desired.mode == IngressListenerMode::PrivatePlaintext;
                        let prior_generation = transition.applied_generation();
                        let post_bind_db = db.clone();
                        let post_bind_dir = data_dir.clone();
                        let post_bind_config = config.clone();
                        let pre_switchover_db = db.clone();
                        let pre_switchover_dir = data_dir.clone();
                        let pre_switchover_config = config.clone();
                        let applied = transition
                            .apply_generation_async_checked(
                                desired_generation,
                                || build_validated_config(&config, &data_dir),
                                Ok,
                                move |(validated, _)| async move {
                                    let listener = Listener::bind(validated)
                                        .await
                                        .map_err(|_| "bind_failed")?;
                                    recheck_authority(
                                        &post_bind_db,
                                        &post_bind_dir,
                                        &post_bind_config,
                                        &post_bind_config,
                                    )
                                    .await?;
                                    Ok(listener)
                                },
                                move || async move {
                                    recheck_authority(
                                        &pre_switchover_db,
                                        &pre_switchover_dir,
                                        &pre_switchover_config,
                                        &pre_switchover_config,
                                    )
                                    .await
                                },
                            )
                            .await;
                        match applied {
                            Ok(old) => {
                                let written = publish_applied_if_authorized(
                                    &db,
                                    &data_dir,
                                    &config,
                                    desired_generation,
                                    tls_generation,
                                )
                                .await;
                                if written.is_ok() {
                                    drop(old);
                                    IngressListenerHealth::listening(config, degraded)
                                } else {
                                    drop(transition.rollback_switchover(old, prior_generation));
                                    record_apply_error(
                                        &db,
                                        desired_generation,
                                        "applied_state_write_failed",
                                    )
                                    .await;
                                    IngressListenerHealth::blocked(
                                        config,
                                        "applied_state_write_failed",
                                    )
                                }
                            }
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
            health.lock().expect("health state mutex poisoned").ingress = next;
            tokio::time::sleep(interval).await;
        }
    })
}

async fn recover_last_applied(db: &DbHandle, data_dir: &Path) -> Option<(Listener, u64, u64)> {
    let config = observe_authority(db, data_dir).await.ok()?;
    if !config.enabled {
        return None;
    }
    let applied = config.applied.clone()?;
    let mut recovery = config.clone();
    recovery.desired = applied.clone();
    let (validated, _) = build_validated_config(&recovery, data_dir).ok()?;
    let listener = Listener::bind(validated).await.ok()?;
    recheck_authority(db, data_dir, &config, &recovery)
        .await
        .ok()?;
    recheck_authority(db, data_dir, &config, &recovery)
        .await
        .ok()?;
    Some((listener, config.desired.generation, applied.generation))
}

async fn recheck_authority(
    db: &DbHandle,
    data_dir: &Path,
    expected: &IngressListenerConfig,
    runtime: &IngressListenerConfig,
) -> Result<(), &'static str> {
    let data_dir = data_dir.to_path_buf();
    let expected = expected.clone();
    let runtime = runtime.clone();
    db.with_conn(move |conn| {
        Ok(recheck_authority_on_conn(
            conn, &data_dir, &expected, &runtime,
        ))
    })
    .await
    .map_err(|_| "database_query_failed")?
}

async fn publish_applied_if_authorized(
    db: &DbHandle,
    data_dir: &Path,
    expected: &IngressListenerConfig,
    generation: u64,
    tls_generation: Option<u64>,
) -> Result<(), &'static str> {
    let data_dir = data_dir.to_path_buf();
    let expected = expected.clone();
    db.with_conn(move |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
        let checked = (|| {
            recheck_authority_on_conn(&tx, &data_dir, &expected, &expected)?;
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
        validate_runtime_config(runtime, data_dir).map(|_| ())?;
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
) -> Result<bool, &'static str> {
    build_validated_config(config, data_dir).map(|(validated, _)| validated.is_degraded())
}

fn build_validated_config(
    config: &IngressListenerConfig,
    data_dir: &Path,
) -> Result<(ValidatedListenerConfig, Option<u64>), &'static str> {
    let state = &config.desired;
    let bind = state.bind_addr.parse().map_err(|_| "invalid_bind")?;
    let site_local_cidrs = state
        .site_local_cidrs
        .iter()
        .map(|cidr| {
            cidr.parse::<SiteCidr>()
                .map_err(|_| "invalid_site_local_cidr")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let exposure = ExposureSnapshot::from_os(&state.interface).map_err(|_| "inventory_invalid")?;
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
        site_local_cidrs,
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
    transition: &mut ListenerTransition<Listener>,
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
mod tests {
    use super::*;

    fn migrations() -> Vec<iotkit_core_storage::Migration> {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
        all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
        all.sort_by_key(|migration| migration.version);
        all
    }

    #[tokio::test]
    async fn applied_publication_rejects_same_generation_configuration_change() {
        let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        db.with_conn_sync(|conn| {
            let hash = iotkit_core_ops::hash_passphrase("test-passphrase-long-enough").unwrap();
            iotkit_core_ops::reset_passphrase_with_hash(conn, &hash, "local_cli").unwrap();
            crate::api::tls::ensure_tls_material(conn, dir.path())
                .map(|_| ())
                .map_err(|error| {
                    iotkit_core_storage::StorageError::Sqlite(
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error)),
                    )
                })
        })
        .unwrap();
        let expected = db
            .with_conn(|conn| {
                iotkit_core_ops::load_ingress_listener_config(conn).map_err(|error| {
                    iotkit_core_storage::StorageError::Sqlite(
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error)),
                    )
                })
            })
            .await
            .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE ingress_listener_config SET bind_addr='192.168.1.9:8444' WHERE id=1",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        assert_eq!(
            publish_applied_if_authorized(&db, dir.path(), &expected, 0, None).await,
            Err("desired_generation_changed")
        );
        let applied_generation = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT applied_generation FROM ingress_listener_config WHERE id=1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(Into::into)
            })
            .await
            .unwrap();
        assert_eq!(applied_generation, 0);
    }
}
