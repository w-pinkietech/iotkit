//! Production composition adapter for the browser boundary.
//!
//! Axum handlers depend only on [`WebApplication`]. This adapter composes the
//! storage- and application-owned operations that exist today; Task 6 can add
//! semantic/output implementations here without adding persistence knowledge
//! under `web/`.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use axum::http::StatusCode;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, Semaphore};

use crate::{
    application::{
        accounts::AccountService,
        output_profiles::{OutputProfiles, ProfileState, PublicationProvenance},
        profiles::{DeviceProfileInput, InventoryProfiles, SignalProfileInput},
        semantics::{SemanticRuleDraft, Semantics},
    },
    auth::{
        password::{Password, PasswordCandidate, PasswordHash, verify_password},
        principal::{AccountRole, AccountState, Principal as ApplicationPrincipal},
        session::{IDLE_SESSION_LIFETIME_MS, SecretDigest, SessionSecrets, SessionWindow},
    },
    composition::registered_output_adapters,
    diagnostics,
    semantics::{Detector, DetectorMode, RuleSpec, SemanticKind, TriggerMode},
    storage::{
        AuditActor, DescriptorSignal, EdgeNodeState, RawHistoryQuery, Storage, StorageError,
    },
    web::{
        ApiMutation, ApiQuery, ConsoleAccount, ConsoleAudit, ConsoleBinding, ConsoleDevice,
        ConsoleEdgeNode, ConsoleOutput, ConsoleRequest, ConsoleRule, ConsoleSignal, ConsoleStorage,
        ConsoleView, HistoryPage, HistoryQuery, LoginSession, MutationOutput, Principal,
        RawHistoryRow, SemanticHistoryPage, SemanticHistoryRow, WebApplication, WebError,
        console::commissioning::commissioning_view,
    },
};

const DUMMY_PASSWORD_PHC: &str = "$argon2id$v=19$m=65536,t=3,p=1$knuL0IBLO4j6mUvzLJIPUA$3nE/zZlE1o4o0jpG+2c0KByiUBDKpUCxUQKYcDNxHMY";

#[derive(Debug, Clone)]
pub struct LoginPolicy {
    pub max_failures: u32,
    pub failure_window: Duration,
    pub max_concurrent: usize,
    pub max_tracked_accounts: usize,
}

impl Default for LoginPolicy {
    fn default() -> Self {
        Self {
            max_failures: 5,
            failure_window: Duration::from_secs(60),
            max_concurrent: 4,
            max_tracked_accounts: 1_024,
        }
    }
}

#[derive(Debug)]
struct LoginFailures {
    count: u32,
    started_at: Instant,
}

#[derive(Clone)]
struct LoginAdmission {
    policy: LoginPolicy,
    concurrent: Arc<Semaphore>,
    failures: Arc<Mutex<HashMap<String, LoginFailures>>>,
}

impl LoginAdmission {
    fn new(policy: LoginPolicy) -> Self {
        Self {
            concurrent: Arc::new(Semaphore::new(policy.max_concurrent.max(1))),
            failures: Arc::new(Mutex::new(HashMap::new())),
            policy,
        }
    }

    fn begin(&self, login_id: &str) -> Result<OwnedSemaphorePermit, WebError> {
        let now = Instant::now();
        let mut failures = self.failures.lock().map_err(internal)?;
        failures.retain(|_, failure| {
            now.duration_since(failure.started_at) < self.policy.failure_window
        });
        if failures
            .get(login_id)
            .is_some_and(|failure| failure.count >= self.policy.max_failures.max(1))
            || (!failures.contains_key(login_id)
                && failures.len() >= self.policy.max_tracked_accounts.max(1))
        {
            return Err(login_rate_limited());
        }
        drop(failures);
        self.concurrent
            .clone()
            .try_acquire_owned()
            .map_err(|_| login_rate_limited())
    }

    fn failed(&self, login_id: &str) {
        let Ok(mut failures) = self.failures.lock() else {
            return;
        };
        let failure = failures.entry(login_id.into()).or_insert(LoginFailures {
            count: 0,
            started_at: Instant::now(),
        });
        failure.count = failure.count.saturating_add(1);
    }

    fn succeeded(&self, login_id: &str) {
        if let Ok(mut failures) = self.failures.lock() {
            failures.remove(login_id);
        }
    }
}

#[derive(Clone)]
pub struct StorageWebApplication {
    storage: Storage,
    login_admission: LoginAdmission,
    mutation_lock: Arc<AsyncMutex<()>>,
    storage_warning_percent: i32,
    broker_certificate_file: Option<PathBuf>,
}

impl StorageWebApplication {
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self::with_login_policy(storage, LoginPolicy::default())
    }

    #[must_use]
    pub fn with_login_policy(storage: Storage, policy: LoginPolicy) -> Self {
        Self {
            storage,
            login_admission: LoginAdmission::new(policy),
            mutation_lock: Arc::new(AsyncMutex::new(())),
            storage_warning_percent: 85,
            broker_certificate_file: None,
        }
    }

    #[must_use]
    pub fn with_runtime_settings(
        storage: Storage,
        storage_warning_percent: i32,
        broker_certificate_file: Option<PathBuf>,
    ) -> Self {
        Self {
            storage,
            login_admission: LoginAdmission::new(LoginPolicy::default()),
            mutation_lock: Arc::new(AsyncMutex::new(())),
            storage_warning_percent,
            broker_certificate_file,
        }
    }

    async fn session(&self, token: &str) -> Result<crate::storage::StoredSession, WebError> {
        self.storage
            .active_session_by_token(&SecretDigest::from_secret(token), now())
            .await
            .map_err(authentication_error)
    }

    async fn storage_view(&self) -> Result<ConsoleStorage, WebError> {
        let status = diagnostics::storage_status(&self.storage, self.storage_warning_percent)
            .await
            .map_err(internal)?;
        let report = diagnostics::diagnostics_with_certificate(
            &self.storage,
            self.storage_warning_percent,
            now(),
            self.broker_certificate_file.as_deref(),
        )
        .await
        .map_err(internal)?;
        let mut messages: Vec<String> = report
            .issues
            .into_iter()
            .map(|issue| issue.summary)
            .collect();
        if messages.is_empty() {
            messages.push("確認が必要なことはありません".into());
        }
        Ok(ConsoleStorage {
            profile_label: if status.profile == "postgres" {
                "PostgreSQL".into()
            } else {
                "組み込みSQLite".into()
            },
            raw_count: status.raw_record_count,
            pending_output_count: status.pending_output_count,
            used_percent: status.disk_used_percent.clamp(0, 100) as u8,
            host_capacity_available: status.filesystem_available,
            retention_note: "rawの自動削除は無効".into(),
            diagnostic_messages: messages,
        })
    }

    async fn console_signals(&self) -> Result<Vec<ConsoleSignal>, WebError> {
        let inventory = self.storage.inventory_signals().await.map_err(internal)?;
        let rules = self.storage.list_semantic_rules().await.map_err(internal)?;
        let mut signals = Vec::with_capacity(inventory.len());
        for signal in inventory
            .into_iter()
            .filter(|signal| signal.presence == "current")
        {
            let latest = self
                .storage
                .recent_signal_inputs(&signal.signal_ref, 1)
                .await
                .map_err(internal)?
                .into_iter()
                .next();
            let value = latest
                .as_ref()
                .and_then(|item| serde_json::from_slice::<Value>(&item.record_json).ok())
                .and_then(|record| record.get("values").and_then(Value::as_array).cloned())
                .and_then(|values| values.into_iter().next())
                .map(|value| display_raw_value(&value, signal.decimal_places))
                .unwrap_or_else(|| "—".into());
            let has_received_value = latest.is_some();
            let sensor_type_code = signal.display_sensor_type.clone();
            let sensor_type = if sensor_type_code.is_empty() {
                format!("{}（種類未設定）", signal.measurement_key)
            } else {
                sensor_type_label(&sensor_type_code, &signal.display_sensor_type_label).into()
            };
            let input_is_boolean = matches!(
                if signal.display_value_kind.is_empty() {
                    signal.value_type.as_str()
                } else {
                    signal.display_value_kind.as_str()
                },
                "bool" | "boolean"
            );
            let calibration = self
                .storage
                .semantic_calibration(&signal.signal_ref)
                .await
                .ok();
            let signal_rules: Vec<ConsoleRule> = rules
                .iter()
                .filter(|rule| {
                    rule.edge_node_id == signal.edge_node_id
                        && rule.series_key == signal.series_key
                        && rule.active
                })
                .map(|rule| ConsoleRule {
                    rule_id: rule.rule_id.clone(),
                    display_name: rule.display_name.clone(),
                    kind: semantic_kind(rule.kind).into(),
                    kind_label: semantic_kind_label(rule.kind).into(),
                    count_summary: count_summary(rule.spec.trigger).into(),
                    revision: rule.revision,
                    detector_mode: detector_mode(rule.spec.detector.mode).into(),
                    detector_is_boolean: matches!(
                        rule.spec.detector.mode,
                        DetectorMode::BooleanHighActive | DetectorMode::BooleanLowActive
                    ),
                    rise_threshold: rule.spec.detector.rise_threshold,
                    fall_threshold: rule.spec.detector.fall_threshold,
                    rise_debounce_seconds: rule.spec.detector.rise_debounce_ms as f64 / 1_000.0,
                    fall_debounce_seconds: rule.spec.detector.fall_debounce_ms as f64 / 1_000.0,
                    trigger: trigger_mode(rule.spec.trigger).into(),
                })
                .collect();
            signals.push(ConsoleSignal {
                signal_ref: signal.signal_ref,
                device_ref: signal.device_ref,
                edge_node_id: signal.edge_node_id,
                name: if signal.display_name.is_empty() {
                    signal.measurement_key
                } else {
                    signal.display_name
                },
                sensor_type,
                sensor_type_code,
                value,
                unit: console_unit_label(&if signal.display_unit_mode == "dimensionless" {
                    String::new()
                } else if signal.display_unit.is_empty() {
                    signal.unit
                } else {
                    signal.display_unit
                }),
                value_kind: if signal.display_value_kind.is_empty() {
                    signal.value_type
                } else {
                    signal.display_value_kind
                },
                unit_mode: signal.display_unit_mode,
                decimal_places: signal.decimal_places,
                revision: signal.profile_revision.unwrap_or_default(),
                status_label: if has_received_value {
                    "受信中".into()
                } else {
                    "未受信".into()
                },
                status_class: if has_received_value {
                    "receiving".into()
                } else {
                    "never".into()
                },
                descriptor_current: signal.presence == "current",
                profile_complete: signal.profile_revision.is_some(),
                input_is_boolean,
                calibration_scale: calibration.map_or(1.0, |value| value.calibration.scale),
                calibration_offset: calibration.map_or(0.0, |value| value.calibration.offset),
                calibration_revision: calibration.map_or(1, |value| value.revision),
                has_alarm_rules: signal_rules.iter().any(|rule| rule.kind == "alarm"),
                rules: signal_rules,
            });
        }
        Ok(signals)
    }

    async fn console_outputs(&self) -> Result<Vec<ConsoleOutput>, WebError> {
        let profiles = OutputProfiles::new(self.storage.clone(), registered_output_adapters())
            .list()
            .await
            .map_err(internal)?;
        let rules = self.storage.list_semantic_rules().await.map_err(internal)?;
        Ok(registered_output_adapters()
            .iter()
            .map(|registration| {
                let descriptor = registration.adapter.descriptor();
                let profile = profiles
                    .iter()
                    .find(|profile| profile.adapter_id == descriptor.id);
                ConsoleOutput {
                    profile_id: profile.map_or_else(String::new, |item| item.profile_id.clone()),
                    adapter_id: descriptor.id.into(),
                    display_name: profile.map_or_else(
                        || descriptor.display_name.into(),
                        |item| item.display_name.clone(),
                    ),
                    description: format!(
                        "{} の意味づけ済みデータを送信します。",
                        descriptor.display_name
                    ),
                    active: profile.is_some_and(|item| {
                        matches!(item.state, ProfileState::Preparing | ProfileState::Active)
                    }),
                    bindings: profile
                        .map(|item| {
                            item.bindings
                                .iter()
                                .map(|binding| {
                                    let rule =
                                        rules.iter().find(|rule| rule.rule_id == binding.rule_id);
                                    ConsoleBinding {
                                        binding_id: binding.binding_id.clone(),
                                        sensor_name: rule.map_or_else(String::new, |rule| {
                                            rule.series_key.clone()
                                        }),
                                        rule_name: rule.map_or_else(
                                            || binding.rule_id.clone(),
                                            |rule| rule.display_name.clone(),
                                        ),
                                        state_label: if binding.active {
                                            "送信中"
                                        } else if binding.needs_configuration {
                                            "設定が必要"
                                        } else {
                                            "開始待ち"
                                        }
                                        .into(),
                                        prepared: !binding.active
                                            && !binding.needs_configuration
                                            && binding.ineligible_reason.is_empty(),
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect())
    }
}

#[async_trait]
impl WebApplication for StorageWebApplication {
    async fn login(&self, username: &str, password: &str) -> Result<LoginSession, WebError> {
        let admission_key = username.trim().to_ascii_lowercase();
        let _admission = self.login_admission.begin(&admission_key)?;
        let credential = self
            .storage
            .get_account_credential_by_login(username)
            .await
            .ok();
        let dummy_password_hash = PasswordHash::new(DUMMY_PASSWORD_PHC);
        let password_hash = credential
            .as_ref()
            .map_or(&dummy_password_hash, |credential| &credential.password_hash);
        let verification = verify_password(password_hash, &PasswordCandidate::new(password))
            .map_err(authentication_error)?;
        let Some(credential) = credential else {
            self.login_admission.failed(&admission_key);
            return Err(invalid_credentials());
        };
        if !verification.matches || credential.account.state != AccountState::Active {
            self.login_admission.failed(&admission_key);
            return Err(invalid_credentials());
        }
        self.login_admission.succeeded(&admission_key);
        let secrets = SessionSecrets::generate().map_err(internal)?;
        let issued_at = now();
        self.storage
            .create_session(
                &credential.account.account_ref,
                credential.account.revision,
                secrets.session_ref().as_str(),
                secrets.token_digest(),
                secrets.csrf_digest(),
                SessionWindow::issued(issued_at).map_err(internal)?,
                issued_at,
            )
            .await
            .map_err(authentication_error)?;
        Ok(LoginSession {
            token: secrets.token().expose_secret().into(),
            csrf: secrets.csrf().expose_secret().into(),
            principal: principal(&credential.account),
        })
    }

    async fn authenticate(&self, token: &str) -> Result<Principal, WebError> {
        let session = self.session(token).await?;
        self.storage
            .touch_session(
                &session.session_ref,
                now(),
                now().saturating_add(IDLE_SESSION_LIFETIME_MS),
            )
            .await
            .map_err(authentication_error)?;
        Ok(principal(&session.account))
    }

    async fn validate_csrf(&self, token: &str, csrf: &str) -> bool {
        let Ok(session) = self.session(token).await else {
            return false;
        };
        bool::from(
            session
                .csrf_digest
                .as_bytes()
                .ct_eq(SecretDigest::from_secret(csrf).as_bytes()),
        )
    }

    async fn logout(&self, token: &str) -> Result<(), WebError> {
        let session = self.session(token).await?;
        self.storage
            .revoke_session(
                &session.session_ref,
                AuditActor::account(&session.account.account_ref),
                now(),
            )
            .await
            .map_err(authentication_error)
    }

    async fn console(&self, request: ConsoleRequest) -> Result<ConsoleView, WebError> {
        let nodes = self.storage.list_edge_nodes(100).await.map_err(internal)?;
        let signals = self.console_signals().await?;
        let inventory_devices = self.storage.inventory_devices().await.map_err(internal)?;
        let mut devices = Vec::with_capacity(inventory_devices.len());
        for device in inventory_devices
            .into_iter()
            .filter(|device| device.presence == "current")
        {
            let edge_node_ref = nodes
                .iter()
                .find(|node| node.edge_node_id == device.edge_node_id)
                .map(|node| node.edge_node_ref.clone())
                .unwrap_or_default();
            let device_signals = signals
                .iter()
                .filter(|signal| signal.device_ref == device.device_ref)
                .cloned()
                .collect();
            devices.push(ConsoleDevice {
                device_ref: device.device_ref,
                edge_node_ref,
                edge_node_id: device.edge_node_id,
                name: if device.display_name.is_empty() {
                    if device.model_id.is_empty() {
                        "名前未設定のデバイス".into()
                    } else {
                        device.model_id.clone()
                    }
                } else {
                    device.display_name
                },
                location: if device.location.is_empty() {
                    "設置場所 未設定".into()
                } else {
                    device.location
                },
                state_label: if device.profile_revision.is_some() {
                    "登録済み".into()
                } else {
                    "設定が必要".into()
                },
                state_class: if device.profile_revision.is_some() {
                    "configured".into()
                } else {
                    "needs-setup".into()
                },
                identifier: device.identifier,
                model_id: device.model_id,
                descriptor_current: device.presence == "current",
                revision: device.profile_revision.unwrap_or_default(),
                signals: device_signals,
            });
        }
        let edge_nodes = nodes
            .iter()
            .map(|node| {
                let child_devices: Vec<_> = devices
                    .iter()
                    .filter(|device| device.edge_node_id == node.edge_node_id)
                    .cloned()
                    .collect();
                console_edge_node_with_devices(node, child_devices)
            })
            .collect::<Vec<_>>();
        let selected_edge_node = if let Some(reference) =
            request.path.strip_prefix("/equipment/edge-nodes/")
        {
            Some(
                edge_nodes
                    .iter()
                    .find(|node| node.edge_node_ref == reference || node.edge_node_id == reference)
                    .cloned()
                    .ok_or_else(not_found_error)?,
            )
        } else {
            None
        };
        let selected_device =
            if let Some(reference) = request.path.strip_prefix("/equipment/devices/") {
                let reference = reference
                    .split_once("/sensors/")
                    .map_or(reference, |(device_ref, _)| device_ref);
                Some(
                    devices
                        .iter()
                        .find(|device| device.device_ref == reference)
                        .cloned()
                        .ok_or_else(not_found_error)?,
                )
            } else {
                None
            };
        let selected_signal = request
            .path
            .strip_prefix("/sensors/")
            .or_else(|| {
                request
                    .path
                    .rsplit_once("/sensors/")
                    .map(|(_, value)| value)
            })
            .and_then(|reference| {
                signals
                    .iter()
                    .find(|signal| signal.signal_ref == reference)
                    .cloned()
            });
        if request.path.contains("/sensors/") && selected_signal.is_none() {
            return Err(not_found_error());
        }
        if let (Some(device), Some(signal)) = (&selected_device, &selected_signal)
            && signal.device_ref != device.device_ref
        {
            return Err(not_found_error());
        }
        let accounts = if request.principal.role == "system_admin" {
            self.storage
                .list_accounts(100)
                .await
                .map_err(internal)?
                .into_iter()
                .map(|account| ConsoleAccount {
                    account_ref: account.account_ref,
                    login_id: account.login_id,
                    display_name: account.display_name,
                    role: account.role.as_str().into(),
                    state: account.state.as_str().into(),
                    revision: account.revision,
                })
                .collect()
        } else {
            Vec::new()
        };
        let audit = self
            .storage
            .list_audit_events(100)
            .await
            .map_err(internal)?
            .into_iter()
            .map(|event| ConsoleAudit {
                occurred_at: event.occurred_at.to_string(),
                actor: event
                    .actor_display_name
                    .or(event.actor_login_id)
                    .unwrap_or(event.actor_class),
                action: event.operation,
                target: event.resource_ref,
            })
            .collect();
        let storage = self.storage_view().await?;
        let history = if request.path == "/logs" {
            self.raw_history(history_query(&request.query, &signals), false)
                .await?
                .rows
        } else {
            Vec::new()
        };
        let commissioning = commissioning_view(&edge_nodes, &devices, &signals);
        Ok(ConsoleView {
            commissioning,
            registered_edge_node_count: nodes
                .iter()
                .filter(|node| node.state == EdgeNodeState::Active)
                .count(),
            receiving_signal_count: signals
                .iter()
                .filter(|signal| signal.status_class == "receiving")
                .count(),
            edge_nodes,
            devices,
            signals,
            selected_edge_node,
            selected_device,
            selected_signal,
            history,
            outputs: self.console_outputs().await?,
            accounts,
            audit,
            storage,
            history_chart_path: String::new(),
            history_raw_export_url: history_url("/api/v1/history.csv", &request.query),
            history_processed_export_url: history_url(
                "/api/v1/semantic-history.csv",
                &request.query,
            ),
            ..ConsoleView::default()
        })
    }

    async fn query(&self, operation: ApiQuery) -> Result<Value, WebError> {
        let ApiQuery::Named { route, params } = operation else {
            return Err(WebError::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "session query is handled by the session endpoint",
            ));
        };
        match route.as_str() {
            "/api/v1/system/storage" => serde_json::to_value(
                diagnostics::storage_status(&self.storage, self.storage_warning_percent)
                    .await
                    .map_err(internal)?,
            )
            .map_err(internal),
            "/api/v1/system/diagnostics" => serde_json::to_value(
                diagnostics::diagnostics_with_certificate(
                    &self.storage,
                    self.storage_warning_percent,
                    now(),
                    self.broker_certificate_file.as_deref(),
                )
                .await
                .map_err(internal)?,
            )
            .map_err(internal),
            "/api/v1/edge-nodes" => Ok(json!({
                "items": self.storage.list_edge_nodes(100).await.map_err(internal)?
                    .iter().map(console_edge_node).map(|node| json!({
                        "edge_node_ref": node.edge_node_ref,
                        "edge_node_id": node.edge_node_id,
                        "state": node.state_class,
                    })).collect::<Vec<_>>()
            })),
            "/api/v1/accounts" => Ok(json!({
                "items": self.storage.list_accounts(100).await.map_err(internal)?
                    .into_iter().map(|account| json!({
                        "account_ref": account.account_ref,
                        "login_id": account.login_id,
                        "display_name": account.display_name,
                        "role": account.role.as_str(),
                        "state": account.state.as_str(),
                        "must_change_password": account.must_change_password,
                        "revision": account.revision,
                        "created_at": account.created_at,
                        "updated_at": account.updated_at,
                    })).collect::<Vec<_>>()
            })),
            "/api/v1/audit-events" => Ok(json!({
                "items": self.storage.list_audit_events(100).await.map_err(internal)?
            })),
            "/api/v1/devices" | "/api/v1/setup/devices" => Ok(json!({
                "items": self.storage.inventory_devices().await.map_err(internal)?
                    .into_iter().map(|item| json!({
                        "device_ref": item.device_ref,
                        "edge_node_id": item.edge_node_id, "system_id": item.system_id,
                        "display_name": if item.display_name.is_empty() {
                            item.identifier
                        } else {
                            item.display_name
                        },
                        "location": item.location, "revision": item.profile_revision,
                        "state": item.state,
                        "presence": item.presence, "model_id": item.model_id,
                    })).collect::<Vec<_>>()
            })),
            "/api/v1/signals" => Ok(json!({
                "items": self.console_signals().await?.into_iter().map(signal_json).collect::<Vec<_>>()
            })),
            "/api/v1/semantic-definitions" => Ok(json!({
                "items": self.storage.list_semantic_rules().await.map_err(internal)?
                    .into_iter().map(rule_json).collect::<Vec<_>>()
            })),
            "/api/v1/output-adapters" => Ok(json!({
                "items": registered_output_adapters().iter().map(|item| {
                    let descriptor = item.adapter.descriptor();
                    json!({
                        "adapter_id": descriptor.id,
                        "display_name": descriptor.display_name,
                        "config_schema_version": descriptor.config_schema_version,
                        "modes": descriptor.modes.iter().map(|mode| json!({
                            "key": mode.key, "display_name": mode.display_name
                        })).collect::<Vec<_>>()
                    })
                }).collect::<Vec<_>>()
            })),
            "/api/v1/export-profiles" => Ok(json!({
                "items": OutputProfiles::new(self.storage.clone(), registered_output_adapters())
                    .list().await.map_err(internal)?.into_iter().map(profile_json).collect::<Vec<_>>()
            })),
            "/api/v1/output-routes" => {
                let profiles =
                    OutputProfiles::new(self.storage.clone(), registered_output_adapters())
                        .list()
                        .await
                        .map_err(internal)?;
                Ok(json!({"items": profiles.into_iter().flat_map(|profile| {
                    let profile_id = profile.profile_id;
                    profile.bindings.into_iter().map(move |binding| json!({
                        "route_id": binding.binding_id, "profile_id": profile_id,
                        "rule_id": binding.rule_id, "mode": binding.mode,
                        "active": binding.active, "external_id": binding.external_id,
                    }))
                }).collect::<Vec<_>>() }))
            }
            route if route.ends_with("/semantic-configuration") => {
                let reference = params.get("signal_ref").cloned().unwrap_or_default();
                let rules: Vec<_> = self
                    .storage
                    .list_semantic_rules()
                    .await
                    .map_err(internal)?
                    .into_iter()
                    .filter(|rule| rule.signal_ref == reference)
                    .collect();
                let revision = rules.iter().map(|rule| rule.revision).max().unwrap_or(1);
                Ok(json!({
                    "revision": revision,
                    "items": rules.into_iter().map(rule_json).collect::<Vec<_>>()
                }))
            }
            route if route.ends_with("/publication") => {
                let binding_id = params.get("binding_id").cloned().unwrap_or_default();
                let publication =
                    OutputProfiles::new(self.storage.clone(), registered_output_adapters())
                        .publication(&binding_id, now())
                        .await
                        .map_err(operation_error)?;
                Ok(json!({
                    "binding_id":publication.binding_id,
                    "provenance":match publication.provenance {
                        PublicationProvenance::Actual => "actual",
                        PublicationProvenance::LatestObservation => "latest_observation",
                        PublicationProvenance::Sample => "sample",
                    },
                    "topic":publication.topic,
                    "qos":publication.qos,
                    "retain":publication.retain,
                    "payload":publication.payload,
                    "delivery":{
                        "state":publication.delivery.state,
                        "pending_count":publication.delivery.pending_count,
                        "published_count":publication.delivery.published_count,
                        "oldest_pending_at":publication.delivery.oldest_pending_at,
                        "last_published_at":publication.delivery.last_published_at,
                    }
                }))
            }
            _ => Err(WebError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "application operation was not found",
            )),
        }
    }

    async fn mutate(
        &self,
        principal: &Principal,
        operation: ApiMutation,
        body: Value,
    ) -> Result<MutationOutput, WebError> {
        let ApiMutation::Named {
            method,
            route,
            params,
            expected_revision,
        } = operation;
        let _mutation = self.mutation_lock.lock().await;
        if let Some(expected_revision) = expected_revision {
            let current_revision = self.resource_revision(&route, &params).await?;
            if current_revision != expected_revision {
                return Err(revision_mismatch());
            }
        }
        let application_principal = application_principal(principal)?;
        let accounts = AccountService::new(self.storage.clone());
        if route == "/password" || route == "/api/v1/session/password" {
            let account = accounts
                .change_own_password(
                    &application_principal,
                    PasswordCandidate::new(required_text(&body, "current_password")?),
                    Password::new(required_text(&body, "new_password")?).map_err(bad_request)?,
                    now(),
                )
                .await
                .map_err(operation_error)?;
            return Ok(MutationOutput::ok(account_json(&account)));
        }
        if route == "/console/accounts" || route == "/api/v1/accounts" {
            let account = accounts
                .create_account(
                    &application_principal,
                    required_text(&body, "login_id")?,
                    required_text(&body, "display_name")?,
                    parse_role(required_text(&body, "role")?)?,
                    Password::new(required_text(&body, "temporary_password")?)
                        .map_err(bad_request)?,
                    now(),
                )
                .await
                .map_err(operation_error)?;
            let output = account_json(&account);
            return Ok(if route.starts_with("/api/") {
                MutationOutput::created(output)
            } else {
                MutationOutput::ok(output)
            });
        }
        if let Some(account_ref) = params.get("account_ref") {
            let revision = required_i64(&body, "revision")?;
            let account = if route.ends_with("/disable") {
                accounts
                    .disable_account(&application_principal, account_ref, revision, now())
                    .await
            } else if route.ends_with("/password") {
                accounts
                    .reset_password(
                        &application_principal,
                        account_ref,
                        revision,
                        Password::new(required_text(&body, "temporary_password")?)
                            .map_err(bad_request)?,
                        now(),
                    )
                    .await
            } else {
                accounts
                    .update_account(
                        &application_principal,
                        account_ref,
                        revision,
                        required_text(&body, "display_name")?,
                        parse_role(required_text(&body, "role")?)?,
                        now(),
                    )
                    .await
            }
            .map_err(operation_error)?;
            return Ok(MutationOutput::ok(account_json(&account)));
        }
        if let Some(reference) = route
            .strip_prefix("/api/v1/edge-nodes/")
            .and_then(|value| value.strip_suffix("/activation"))
            .or_else(|| {
                route
                    .strip_prefix("/console/edge-nodes/")
                    .and_then(|value| value.strip_suffix("/activation"))
            })
        {
            let node = self
                .storage
                .list_edge_nodes(100)
                .await
                .map_err(internal)?
                .into_iter()
                .find(|node| node.edge_node_ref == reference || node.edge_node_id == reference)
                .ok_or_else(|| {
                    WebError::new(
                        StatusCode::NOT_FOUND,
                        "not_found",
                        "Edge Node was not found",
                    )
                })?;
            self.storage
                .request_activation_as(
                    AuditActor::account(&principal.account_ref),
                    &node.edge_node_id,
                    now(),
                )
                .await
                .map_err(internal)?;
            return Ok(MutationOutput::accepted(json!({
                "edge_node_ref": node.edge_node_ref,
                "state": "activating",
            })));
        }
        let semantics = Semantics::new(self.storage.clone());
        if let Some(signal_ref) = params.get("signal_ref") {
            if method == axum::http::Method::DELETE && route.ends_with("/semantic-definition") {
                let rules = self.storage.list_semantic_rules().await.map_err(internal)?;
                let mut retired = 0;
                for rule in rules
                    .into_iter()
                    .filter(|rule| rule.signal_ref == *signal_ref && rule.active)
                {
                    semantics
                        .retire_rule(&rule.rule_id, now())
                        .await
                        .map_err(operation_error)?;
                    retired += 1;
                }
                if retired == 0 {
                    return Err(not_found_error());
                }
                return Ok(MutationOutput::ok(
                    json!({"signal_ref": signal_ref, "active": false}),
                ));
            }
            if route.ends_with("/calibration") {
                let revision = semantics
                    .update_calibration_as(
                        AuditActor::account(&principal.account_ref),
                        signal_ref,
                        required_f64(&body, "scale")?,
                        required_f64(&body, "offset")?,
                        expected_revision,
                        now(),
                    )
                    .await
                    .map_err(operation_error)?;
                return Ok(MutationOutput::ok(json!({"revision": revision})));
            }
            if route.ends_with("/semantic-counter/reset") {
                let rules = self.storage.list_semantic_rules().await.map_err(internal)?;
                let rule = rules
                    .into_iter()
                    .find(|rule| rule.signal_ref == *signal_ref && rule.active)
                    .ok_or_else(not_found_error)?;
                let reset_id = semantics
                    .reset_counter_as(
                        AuditActor::account(&principal.account_ref),
                        &rule.rule_id,
                        now(),
                    )
                    .await
                    .map_err(operation_error)?;
                return Ok(MutationOutput::accepted(json!({"reset_id": reset_id})));
            }
            if route.ends_with("/semantic-rules")
                || route.ends_with("/semantic-definition")
                || route.ends_with("/semantic")
            {
                let signal = resolve_signal(&self.storage, signal_ref).await?;
                let rule = semantics
                    .create_rule_as(
                        AuditActor::account(&principal.account_ref),
                        SemanticRuleDraft {
                            edge_node_id: signal.edge_node_id,
                            series_key: signal.series_key,
                            display_name: required_text(&body, "display_name")?.into(),
                            spec: rule_spec(&body)?,
                        },
                        now(),
                    )
                    .await
                    .map_err(operation_error)?;
                return Ok(MutationOutput::created(rule_json(rule)));
            }
            if route.ends_with("/profile") {
                let unit_mode = required_text(&body, "display_unit_mode")?;
                let profile = InventoryProfiles::new(self.storage.clone())
                    .update_signal(
                        AuditActor::account(&principal.account_ref),
                        signal_ref,
                        SignalProfileInput {
                            display_name: required_text(&body, "display_name")?.into(),
                            display_sensor_type: required_text(&body, "display_sensor_type")?
                                .into(),
                            display_sensor_type_label: body
                                .get("display_sensor_type_label")
                                .and_then(Value::as_str)
                                .unwrap_or_else(|| {
                                    body.get("display_sensor_type")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                })
                                .into(),
                            display_value_kind: required_text(&body, "display_value_kind")?.into(),
                            display_unit_mode: if unit_mode == "custom" {
                                "unit".into()
                            } else {
                                unit_mode.into()
                            },
                            display_unit: body
                                .get("display_unit")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .into(),
                            decimal_places: required_nonnegative_i64(&body, "decimal_places")?
                                .try_into()
                                .map_err(|_| bad_request("decimal_places is out of range"))?,
                        },
                        optional_i64(&body, "revision")?,
                        now(),
                    )
                    .await
                    .map_err(operation_error)?;
                return Ok(MutationOutput::ok(json!({
                    "signal_ref": profile.signal_ref,
                    "display_name": profile.display_name,
                    "revision": profile.revision,
                })));
            }
        }
        if let Some(rule_id) = params.get("rule_id") {
            if route.ends_with("/retire") || method == axum::http::Method::DELETE {
                semantics
                    .retire_rule_as(AuditActor::account(&principal.account_ref), rule_id, now())
                    .await
                    .map_err(operation_error)?;
                return Ok(MutationOutput::ok(
                    json!({"rule_id": rule_id, "active": false}),
                ));
            }
            if route.ends_with("/counter-resets") {
                let reset_id = semantics
                    .reset_counter_as(AuditActor::account(&principal.account_ref), rule_id, now())
                    .await
                    .map_err(operation_error)?;
                return Ok(MutationOutput::accepted(json!({"reset_id": reset_id})));
            }
            let rule = semantics
                .revise_rule_as(
                    AuditActor::account(&principal.account_ref),
                    rule_id,
                    required_text(&body, "display_name")?,
                    rule_spec(&body)?,
                    now(),
                )
                .await
                .map_err(operation_error)?;
            return Ok(MutationOutput::ok(rule_json(rule)));
        }
        let profiles = OutputProfiles::new(self.storage.clone(), registered_output_adapters());
        if route == "/console/export-profiles" || route == "/api/v1/export-profiles" {
            let display_name = body
                .get("display_name")
                .and_then(Value::as_str)
                .unwrap_or_else(|| {
                    body.get("adapter_id")
                        .and_then(Value::as_str)
                        .unwrap_or("Output")
                });
            let profile = profiles
                .activate_as(
                    AuditActor::account(&principal.account_ref),
                    display_name,
                    required_text(&body, "adapter_id")?,
                    serde_json::Map::new(),
                    now(),
                )
                .await
                .map_err(operation_error)?;
            return Ok(MutationOutput::created(profile_json(profile)));
        }
        if let Some(profile_id) = params.get("profile_id")
            && route.ends_with("/stop")
        {
            profiles
                .stop_as(
                    AuditActor::account(&principal.account_ref),
                    profile_id,
                    now(),
                )
                .await
                .map_err(operation_error)?;
            return Ok(MutationOutput::accepted(
                json!({"profile_id": profile_id, "state": "draining"}),
            ));
        }
        if let Some(binding_id) = params.get("binding_id") {
            if route.ends_with("/start") {
                profiles
                    .confirm_as(
                        AuditActor::account(&principal.account_ref),
                        binding_id,
                        now(),
                    )
                    .await
                    .map_err(operation_error)?;
                return Ok(MutationOutput::accepted(
                    json!({"binding_id": binding_id, "active": true}),
                ));
            }
            let binding = profiles
                .configure_as(
                    AuditActor::account(&principal.account_ref),
                    binding_id,
                    body.get("mode")
                        .and_then(Value::as_str)
                        .unwrap_or("observation"),
                    serde_json::Map::new(),
                    now(),
                )
                .await
                .map_err(operation_error)?;
            return Ok(MutationOutput::ok(json!({
                "binding_id": binding.binding_id, "rule_id": binding.rule_id,
                "mode": binding.mode, "active": binding.active
            })));
        }
        if let Some(device_ref) = params.get("device_ref")
            && route.ends_with("/profile")
        {
            let profile = InventoryProfiles::new(self.storage.clone())
                .update_device(
                    AuditActor::account(&principal.account_ref),
                    device_ref,
                    DeviceProfileInput {
                        display_name: required_text(&body, "display_name")?.into(),
                        location: required_text(&body, "location")?.into(),
                    },
                    optional_i64(&body, "revision")?,
                    now(),
                )
                .await
                .map_err(operation_error)?;
            return Ok(MutationOutput::ok(json!({
                "device_ref":profile.device_ref,
                "display_name":profile.display_name,
                "location":profile.location,
                "revision":profile.revision,
            })));
        }
        if route == "/api/v1/export-profiles/preview" {
            let preview = profiles
                .preview_activation(required_text(&body, "adapter_id")?)
                .await
                .map_err(operation_error)?;
            return Ok(MutationOutput::ok(json!({
                "adapter_id":preview.adapter_id,
                "automatic_count":preview.automatic_count,
                "needs_configuration_count":preview.needs_configuration_count,
                "ineligible_count":preview.ineligible_count,
                "rules":preview.rules.into_iter().map(|rule| json!({
                    "rule_id":rule.rule_id,
                    "state":rule.state,
                    "compatible_modes":rule.compatible_modes,
                })).collect::<Vec<_>>(),
            })));
        }
        if route == "/api/v1/mapping-previews" {
            let request = serde_json::from_value(body).map_err(bad_request)?;
            let preview = semantics.preview(request).await.map_err(operation_error)?;
            return Ok(MutationOutput::ok(json!({
                "calibration":{"scale":preview.calibration.scale,"offset":preview.calibration.offset},
                "window_start":preview.window_start,
                "window_end":preview.window_end,
                "rules":preview.rules.into_iter().map(|rule| json!({
                    "rule_id":rule.rule_id,
                    "display_name":rule.display_name,
                    "kind":semantic_kind(rule.kind),
                    "input_count":rule.input_count,
                    "plot_count":rule.plot_count,
                    "error":rule.error,
                    "test_result":rule.test_result.map(|result| json!({
                        "emitted":result.emitted,
                        "number":result.number,
                        "boolean":result.boolean,
                        "integer":result.integer,
                        "calibrated":result.calibrated,
                    })),
                    "points":rule.points.into_iter().map(|point| json!({
                        "received_at":point.received_at,
                        "input":point.input,
                        "input_min":point.input_min,
                        "input_max":point.input_max,
                        "calibrated":point.calibrated,
                        "calibrated_min":point.calibrated_min,
                        "calibrated_max":point.calibrated_max,
                        "active":point.active,
                        "counter":point.counter,
                        "sample_count":point.sample_count,
                        "active_samples":point.active_samples,
                        "transitions":point.transitions,
                        "increment":point.increment,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })));
        }
        Err(WebError::new(
            StatusCode::BAD_REQUEST,
            "unsupported_operation",
            "the requested operation is not available for this resource",
        ))
    }

    async fn raw_history(
        &self,
        query: HistoryQuery,
        export: bool,
    ) -> Result<HistoryPage, WebError> {
        let from = query
            .from
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or_default();
        let to = query
            .to
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(i64::MAX);
        let limit = if export {
            crate::web::MAX_HISTORY_EXPORT_ROWS
        } else {
            usize::from(query.limit.unwrap_or(200))
        };
        let page = self
            .storage
            .query_raw_history(RawHistoryQuery {
                from,
                to,
                limit,
                cursor: query.cursor,
                signal_ref: query.signal_ref,
                edge_node_id: query.edge_node_id,
            })
            .await
            .map_err(internal)?;
        let mut rows = Vec::with_capacity(page.rows.len());
        for record in page.rows {
            let value: Value = serde_json::from_slice(&record.record_json).map_err(internal)?;
            let synthetic_ref = format!("{}:{}", record.edge_node_id, record.series_key);
            rows.push(RawHistoryRow {
                received_at: record.received_at.to_string(),
                observed_at: value
                    .get("event_time")
                    .or_else(|| value.get("observed_at"))
                    .or_else(|| value.get("observed_at_unix_ms"))
                    .and_then(Value::as_i64)
                    .unwrap_or(record.received_at)
                    .to_string(),
                edge_node_id: record.edge_node_id,
                ledger_epoch: record.ledger_epoch,
                pub_seq: record.pub_seq,
                signal_ref: if record.signal_ref.is_empty() {
                    synthetic_ref
                } else {
                    record.signal_ref
                },
                series_key: record.series_key,
                sensor_name: record.display_name,
                values: value
                    .get("values")
                    .cloned()
                    .unwrap_or_else(|| value.clone())
                    .to_string(),
                value_type: text(&value, &["value_type"]).into(),
                unit: record.unit,
                decimal_places: record.decimal_places,
                display_value_kind: record.display_value_kind,
            });
        }
        Ok(HistoryPage {
            rows,
            next_cursor: page.next_cursor,
            has_more: page.has_more,
        })
    }

    async fn history_series(&self, query: HistoryQuery) -> Result<Value, WebError> {
        let signal_ref = query.signal_ref.as_deref().unwrap_or_default();
        let from = query
            .from
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or_default();
        let to = query
            .to
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or_default();
        let buckets = self
            .storage
            .query_history_series(signal_ref, from, to, query.bucket_ms.unwrap_or_default())
            .await
            .map_err(internal)?;
        let signal = self
            .storage
            .inventory_signals()
            .await
            .map_err(internal)?
            .into_iter()
            .find(|signal| signal.signal_ref == signal_ref)
            .ok_or_else(not_found_error)?;
        let sample_count: i64 = buckets.iter().map(|bucket| bucket.count).sum();
        Ok(json!({
            "signal_ref": signal_ref,
            "display_name": if signal.display_name.is_empty() {
                signal.measurement_key
            } else {
                signal.display_name
            },
            "unit": signal.display_unit,
            "value_type": signal.value_type,
            "sample_count": sample_count,
            "points": buckets.into_iter().map(|bucket| json!({
                "bucket_start":bucket.bucket_start,
                "minimum":bucket.minimum,
                "average":bucket.average,
                "maximum":bucket.maximum,
                "sample_count":bucket.count,
            })).collect::<Vec<_>>(),
        }))
    }

    async fn semantic_history(&self, query: HistoryQuery) -> Result<SemanticHistoryPage, WebError> {
        let from = query
            .from
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or_default();
        let to = query
            .to
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(i64::MAX);
        let stored = self
            .storage
            .query_semantic_history(
                from,
                to,
                crate::web::MAX_HISTORY_EXPORT_ROWS + 1,
                query.signal_ref.as_deref(),
                query.edge_node_id.as_deref(),
            )
            .await
            .map_err(internal)?;
        let has_more = stored.len() > crate::web::MAX_HISTORY_EXPORT_ROWS;
        let rows = stored
            .into_iter()
            .take(crate::web::MAX_HISTORY_EXPORT_ROWS)
            .map(|observation| SemanticHistoryRow {
                observed_at: observation.observed_at.to_string(),
                processed_at: observation.processed_at.to_string(),
                edge_node_id: observation.edge_node_id,
                signal_ref: observation.signal_ref,
                sensor_name: observation.signal_name,
                rule_name: observation.rule_name,
                kind: observation.kind,
                value: serde_json::from_slice::<Value>(&observation.value_json)
                    .map_or_else(|_| String::new(), |value| value.to_string()),
                unit: observation.unit,
                series_id: observation.series_id,
                sequence: observation.sequence,
                observation_id: observation.observation_id,
                rule_revision: observation.rule_revision,
                calibration_revision: observation.calibration_revision,
                source_pub_seq: observation.source_pub_seq,
            })
            .collect();
        Ok(SemanticHistoryPage { rows, has_more })
    }
}

impl StorageWebApplication {
    async fn resource_revision(
        &self,
        route: &str,
        params: &HashMap<String, String>,
    ) -> Result<i64, WebError> {
        if let Some(rule_id) = params.get("rule_id") {
            return self
                .storage
                .list_semantic_rules()
                .await
                .map_err(internal)?
                .into_iter()
                .find(|rule| rule.rule_id == *rule_id)
                .map(|rule| rule.revision)
                .ok_or_else(not_found_error);
        }
        if let Some(signal_ref) = params.get("signal_ref") {
            if route.ends_with("/calibration") {
                return self
                    .storage
                    .semantic_calibration_revision(signal_ref)
                    .await
                    .map_err(operation_error);
            }
            return Ok(self
                .storage
                .list_semantic_rules()
                .await
                .map_err(internal)?
                .into_iter()
                .filter(|rule| rule.signal_ref == *signal_ref)
                .map(|rule| rule.revision)
                .max()
                .unwrap_or(1));
        }
        if let Some(profile_id) = params.get("profile_id") {
            return OutputProfiles::new(self.storage.clone(), registered_output_adapters())
                .list()
                .await
                .map_err(internal)?
                .into_iter()
                .find(|profile| profile.profile_id == *profile_id)
                .map(|profile| profile.revision)
                .ok_or_else(not_found_error);
        }
        if let Some(binding_id) = params.get("binding_id") {
            return OutputProfiles::new(self.storage.clone(), registered_output_adapters())
                .list()
                .await
                .map_err(internal)?
                .into_iter()
                .find(|profile| {
                    profile
                        .bindings
                        .iter()
                        .any(|binding| binding.binding_id == *binding_id)
                })
                .map(|profile| profile.revision)
                .ok_or_else(not_found_error);
        }
        Ok(1)
    }
}

fn principal(account: &crate::storage::Account) -> Principal {
    Principal {
        account_ref: account.account_ref.clone(),
        login_id: account.login_id.clone(),
        display_name: account.display_name.clone(),
        role: account.role.as_str().into(),
        state: account.state.as_str().into(),
        must_change_password: account.must_change_password,
        revision: account.revision,
        created_at: account.created_at,
        updated_at: account.updated_at,
    }
}

fn application_principal(principal: &Principal) -> Result<ApplicationPrincipal, WebError> {
    ApplicationPrincipal::authenticated_account(
        &principal.account_ref,
        &principal.login_id,
        &principal.display_name,
        parse_role(&principal.role)?,
        match principal.state.as_str() {
            "active" => AccountState::Active,
            "disabled" => AccountState::Disabled,
            _ => return Err(authentication_error("invalid account state")),
        },
        principal.must_change_password,
        "web-session",
    )
    .map_err(authentication_error)
}

fn parse_role(value: &str) -> Result<AccountRole, WebError> {
    match value {
        "viewer" => Ok(AccountRole::Viewer),
        "admin" => Ok(AccountRole::Admin),
        "system_admin" => Ok(AccountRole::SystemAdmin),
        _ => Err(WebError::new(
            StatusCode::BAD_REQUEST,
            "invalid_role",
            "account role is invalid",
        )),
    }
}

fn required_text<'a>(body: &'a Value, field: &'static str) -> Result<&'a str, WebError> {
    body.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            WebError::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                format!("{field} is required"),
            )
            .field(field)
        })
}

fn required_i64(body: &Value, field: &'static str) -> Result<i64, WebError> {
    body.get(field)
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            WebError::new(
                StatusCode::PRECONDITION_FAILED,
                "revision_required",
                format!("{field} is required"),
            )
            .field(field)
        })
}

fn required_nonnegative_i64(body: &Value, field: &'static str) -> Result<i64, WebError> {
    body.get(field)
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .filter(|value| *value >= 0)
        .ok_or_else(|| {
            WebError::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                format!("{field} must be zero or greater"),
            )
            .field(field)
        })
}

fn optional_i64(body: &Value, field: &'static str) -> Result<Option<i64>, WebError> {
    let Some(value) = body.get(field) else {
        return Ok(None);
    };
    if value.as_str().is_some_and(str::is_empty) {
        return Ok(None);
    }
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        .filter(|value| *value > 0)
        .map(Some)
        .ok_or_else(|| bad_request(format!("{field} must be a positive integer")))
}

fn account_json(account: &crate::storage::Account) -> Value {
    json!({
        "account_ref": account.account_ref,
        "login_id": account.login_id,
        "display_name": account.display_name,
        "role": account.role.as_str(),
        "state": account.state.as_str(),
        "must_change_password": account.must_change_password,
        "revision": account.revision,
        "created_at": account.created_at,
        "updated_at": account.updated_at,
    })
}

fn console_edge_node(node: &crate::storage::EdgeNode) -> ConsoleEdgeNode {
    console_edge_node_with_devices(node, Vec::new())
}

fn console_edge_node_with_devices(
    node: &crate::storage::EdgeNode,
    devices: Vec<ConsoleDevice>,
) -> ConsoleEdgeNode {
    let (state_label, state_class, can_activate) = match node.state {
        EdgeNodeState::Discovered => ("未登録", "needs-setup", true),
        EdgeNodeState::Activating => ("登録処理中", "stale", false),
        EdgeNodeState::Active => ("登録済み", "configured", false),
        EdgeNodeState::RecoveryHold => ("復旧確認待ち", "stale", false),
    };
    let descriptor_device_count = devices
        .iter()
        .filter(|device| device.descriptor_current)
        .count();
    let descriptor_signal_count = devices
        .iter()
        .flat_map(|device| &device.signals)
        .filter(|signal| signal.descriptor_current)
        .count();
    ConsoleEdgeNode {
        edge_node_ref: node.edge_node_ref.clone(),
        edge_node_id: node.edge_node_id.clone(),
        ledger_epoch: node.ledger_epoch.clone(),
        first_detected_at: node.first_detected_at.to_string(),
        name: node.edge_node_id.clone(),
        location: "設置場所 未設定".into(),
        state: node.state,
        state_label: state_label.into(),
        state_class: state_class.into(),
        can_activate,
        needs_recovery_review: node.state == EdgeNodeState::RecoveryHold,
        devices,
        descriptor_device_count,
        descriptor_signal_count,
        signal_count: descriptor_signal_count,
    }
}

fn console_unit_label(unit: &str) -> String {
    match unit {
        "Cel" => "°C".into(),
        _ => unit.into(),
    }
}

fn history_url(base: &str, query: &HashMap<String, String>) -> String {
    let mut values = query.clone();
    values.entry("from".into()).or_insert_with(|| "0".into());
    values
        .entry("to".into())
        .or_insert_with(|| now().to_string());
    format!(
        "{base}?{}",
        serde_urlencoded::to_string(values).unwrap_or_default()
    )
}

fn text<'a>(value: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .unwrap_or("")
}

fn descriptor_signal_ref(signal: &DescriptorSignal) -> String {
    format!("{}:{}", signal.edge_node_id, signal.series_key)
}

async fn resolve_signal(storage: &Storage, signal_ref: &str) -> Result<DescriptorSignal, WebError> {
    let inventory = storage.inventory_signals().await.map_err(internal)?;
    let inventory_identity = inventory
        .iter()
        .find(|signal| signal.signal_ref == signal_ref)
        .map(|signal| (signal.edge_node_id.as_str(), signal.series_key.as_str()));
    let rules = storage.list_semantic_rules().await.map_err(internal)?;
    let identity = inventory_identity.or_else(|| {
        rules
            .iter()
            .find(|rule| rule.signal_ref == signal_ref)
            .map(|rule| (rule.edge_node_id.as_str(), rule.series_key.as_str()))
    });
    storage
        .list_descriptor_signals()
        .await
        .map_err(internal)?
        .into_iter()
        .find(|signal| {
            identity.is_some_and(|(edge, series)| {
                edge == signal.edge_node_id && series == signal.series_key
            }) || descriptor_signal_ref(signal) == signal_ref
        })
        .ok_or_else(not_found_error)
}

fn signal_json(signal: ConsoleSignal) -> Value {
    json!({
        "signal_ref": signal.signal_ref, "device_ref": signal.device_ref,
        "edge_node_id": signal.edge_node_id, "display_name": signal.name,
        "sensor_type": signal.sensor_type, "value": signal.value, "unit": signal.unit,
        "status": signal.status_class, "profile_complete": signal.profile_complete,
        "semantic_rules": signal.rules.into_iter().map(|rule| json!({
            "rule_id": rule.rule_id, "display_name": rule.display_name, "kind": rule.kind
        })).collect::<Vec<_>>()
    })
}

fn semantic_kind(kind: SemanticKind) -> &'static str {
    match kind {
        SemanticKind::Numeric => "numeric",
        SemanticKind::Boolean => "boolean",
        SemanticKind::CumulativeCounter => "cumulative_counter",
        SemanticKind::Alarm => "alarm",
    }
}

fn semantic_kind_label(kind: SemanticKind) -> &'static str {
    match kind {
        SemanticKind::Numeric => "測定値",
        SemanticKind::Boolean => "ON / OFF",
        SemanticKind::CumulativeCounter => "累積値",
        SemanticKind::Alarm => "異常検知",
    }
}

fn detector_mode(mode: DetectorMode) -> &'static str {
    match mode {
        DetectorMode::None => "",
        DetectorMode::BooleanHighActive => "boolean_high_active",
        DetectorMode::BooleanLowActive => "boolean_low_active",
        DetectorMode::HighActive => "high_active",
        DetectorMode::LowActive => "low_active",
    }
}

fn trigger_mode(mode: TriggerMode) -> &'static str {
    match mode {
        TriggerMode::None => "",
        TriggerMode::OnTransition => "on_transition",
        TriggerMode::OnNotification => "on_notification",
    }
}

fn count_summary(mode: TriggerMode) -> &'static str {
    match mode {
        TriggerMode::OnTransition => "OFF→ONで +1",
        TriggerMode::OnNotification => "条件一致の受信ごとに +1",
        TriggerMode::None => "",
    }
}

fn rule_json(rule: crate::application::semantics::SemanticRule) -> Value {
    json!({
        "rule_id": rule.rule_id, "signal_ref": rule.signal_ref,
        "edge_node_id": rule.edge_node_id, "series_key": rule.series_key,
        "display_name": rule.display_name, "kind": semantic_kind(rule.kind),
        "spec": rule.spec,
        "series_id": rule.series_id, "revision": rule.revision, "active": rule.active
    })
}

fn profile_json(profile: crate::application::output_profiles::ExportProfile) -> Value {
    let state = match profile.state {
        ProfileState::Preparing => "preparing",
        ProfileState::Active => "active",
        ProfileState::Draining => "draining",
        ProfileState::Stopped => "stopped",
    };
    json!({
        "profile_id": profile.profile_id, "display_name": profile.display_name,
        "adapter_id": profile.adapter_id, "state": state, "revision": profile.revision,
        "bindings": profile.bindings.into_iter().map(|binding| json!({
            "binding_id": binding.binding_id, "rule_id": binding.rule_id,
            "external_id": binding.external_id, "mode": binding.mode,
            "active": binding.active, "needs_configuration": binding.needs_configuration,
            "ineligible_reason": binding.ineligible_reason
        })).collect::<Vec<_>>()
    })
}

fn rule_spec(body: &Value) -> Result<RuleSpec, WebError> {
    let kind = match required_text(body, "kind")? {
        "numeric" => SemanticKind::Numeric,
        "boolean" => SemanticKind::Boolean,
        "cumulative_counter" => SemanticKind::CumulativeCounter,
        "alarm" => SemanticKind::Alarm,
        _ => return Err(bad_request("unknown semantic kind")),
    };
    let default_detector = match kind {
        SemanticKind::Numeric => "",
        SemanticKind::Boolean | SemanticKind::CumulativeCounter => "boolean_high_active",
        SemanticKind::Alarm => "high_active",
    };
    let detector_mode = match body
        .get("detector_mode")
        .and_then(Value::as_str)
        .unwrap_or(default_detector)
    {
        "" => DetectorMode::None,
        "boolean_high_active" => DetectorMode::BooleanHighActive,
        "boolean_low_active" => DetectorMode::BooleanLowActive,
        "high_active" => DetectorMode::HighActive,
        "low_active" => DetectorMode::LowActive,
        _ => return Err(bad_request("unknown detector mode")),
    };
    let default_trigger = if kind == SemanticKind::CumulativeCounter {
        "on_transition"
    } else {
        ""
    };
    let trigger = match body
        .get("trigger")
        .and_then(Value::as_str)
        .unwrap_or(default_trigger)
    {
        "" => TriggerMode::None,
        "on_transition" => TriggerMode::OnTransition,
        "on_notification" => TriggerMode::OnNotification,
        _ => return Err(bad_request("unknown trigger")),
    };
    Ok(RuleSpec {
        kind,
        detector: Detector {
            mode: detector_mode,
            rise_threshold: optional_f64(body, "rise_threshold", 0.0)?,
            fall_threshold: optional_f64(body, "fall_threshold", 0.0)?,
            rise_debounce_ms: (optional_f64(body, "rise_debounce_seconds", 0.0)? * 1_000.0) as i64,
            fall_debounce_ms: (optional_f64(body, "fall_debounce_seconds", 0.0)? * 1_000.0) as i64,
        },
        trigger,
    })
}

fn required_f64(body: &Value, field: &'static str) -> Result<f64, WebError> {
    optional_f64(body, field, f64::NAN).and_then(|value| {
        if value.is_finite() {
            Ok(value)
        } else {
            Err(bad_request(format!("{field} is required")))
        }
    })
}

fn optional_f64(body: &Value, field: &'static str, default: f64) -> Result<f64, WebError> {
    match body.get(field) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(value)) => value
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| bad_request(format!("{field} must be a finite number"))),
        Some(Value::String(value)) if value.is_empty() => Ok(default),
        Some(Value::String(value)) => value
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| bad_request(format!("{field} must be a finite number"))),
        _ => Err(bad_request(format!("{field} must be a number"))),
    }
}

fn history_query(params: &HashMap<String, String>, signals: &[ConsoleSignal]) -> HistoryQuery {
    let signal_ref = params.get("signal_ref").cloned();
    let edge_node_id = signal_ref.as_ref().and_then(|reference| {
        signals
            .iter()
            .find(|signal| signal.signal_ref == *reference)
            .map(|signal| signal.edge_node_id.clone())
    });
    HistoryQuery {
        from: params.get("from").cloned(),
        to: params.get("to").cloned(),
        limit: Some(200),
        cursor: None,
        signal_ref,
        edge_node_id,
        bucket_ms: None,
    }
}

fn not_found_error() -> WebError {
    WebError::new(StatusCode::NOT_FOUND, "not_found", "resource was not found")
}

fn display_raw_value(value: &Value, decimal_places: i32) -> String {
    if let Some(number) = value.as_f64() {
        let places = usize::try_from(decimal_places.clamp(0, 6)).unwrap_or_default();
        return format!("{number:.places$}");
    }
    match value {
        Value::Bool(state) => {
            if *state {
                "ON".into()
            } else {
                "OFF".into()
            }
        }
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

fn sensor_type_label<'a>(code: &'a str, custom_label: &'a str) -> &'a str {
    match code {
        "thermocouple" => "熱電対",
        "temperature" => "温度（方式未確認）",
        "contact" => "接点入力",
        "illuminance" => "照度",
        "distance" => "距離",
        "voltage" => "電圧",
        "current" => "電流",
        "pressure" => "圧力",
        "humidity" => "湿度",
        "acceleration" => "加速度",
        "custom" if !custom_label.is_empty() => custom_label,
        _ => code,
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

fn authentication_error(error: impl std::fmt::Display) -> WebError {
    let _ = error;
    invalid_credentials()
}

fn invalid_credentials() -> WebError {
    WebError::new(
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
        "login ID or password is invalid",
    )
}

fn login_rate_limited() -> WebError {
    WebError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "login_rate_limited",
        "login cannot be attempted again yet",
    )
}

fn revision_mismatch() -> WebError {
    WebError::new(
        StatusCode::PRECONDITION_FAILED,
        "revision_mismatch",
        "resource revision does not match",
    )
}

fn internal(error: impl std::fmt::Display) -> WebError {
    let _ = error;
    WebError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "internal server error",
    )
}

fn bad_request(error: impl std::fmt::Display) -> WebError {
    WebError::new(
        StatusCode::BAD_REQUEST,
        "invalid_request",
        error.to_string(),
    )
}

fn operation_error(error: impl std::fmt::Display) -> WebError {
    let message = error.to_string();
    if message.contains("authorization") || message.contains("not authorized") {
        WebError::new(StatusCode::FORBIDDEN, "forbidden", "operation is forbidden")
    } else if message.contains("revision") {
        WebError::new(
            StatusCode::CONFLICT,
            "revision_mismatch",
            "resource revision does not match",
        )
    } else if message.contains("not found") {
        WebError::new(StatusCode::NOT_FOUND, "not_found", "resource was not found")
    } else {
        bad_request(message)
    }
}

impl serde::Serialize for crate::storage::AuditEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        json!({
            "audit_row_id": self.audit_row_id,
            "occurred_at": self.occurred_at,
            "actor_class": self.actor_class,
            "actor_ref": self.actor_ref,
            "actor_login_id": self.actor_login_id,
            "actor_display_name": self.actor_display_name,
            "operation": self.operation,
            "resource_ref": self.resource_ref,
            "outcome": self.outcome,
            "summary": self.summary,
        })
        .serialize(serializer)
    }
}

impl From<StorageError> for WebError {
    fn from(error: StorageError) -> Self {
        internal(error)
    }
}
