//! Production composition adapter for the browser boundary.
//!
//! Axum handlers depend only on [`WebApplication`]. This adapter composes the
//! storage- and application-owned operations that exist today; Task 6 can add
//! semantic/output implementations here without adding persistence knowledge
//! under `web/`.

use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use axum::http::StatusCode;
use iotkit_output_adapter_api::ObservationValue;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

use crate::{
    application::{
        accounts::AccountService,
        output_profiles::{OutputProfiles, ProfileState},
        semantics::{SemanticRuleDraft, Semantics},
    },
    auth::{
        password::{Password, PasswordCandidate, verify_password},
        principal::{AccountRole, AccountState, Principal as ApplicationPrincipal},
        session::{IDLE_SESSION_LIFETIME_MS, SecretDigest, SessionSecrets, SessionWindow},
    },
    composition::registered_output_adapters,
    diagnostics,
    semantics::{Detector, DetectorMode, RuleSpec, SemanticKind, TriggerMode},
    storage::{AuditActor, DescriptorSignal, EdgeNodeState, Storage, StorageError},
    web::{
        ApiMutation, ApiQuery, ConsoleAccount, ConsoleAudit, ConsoleBinding, ConsoleEdgeNode,
        ConsoleOutput, ConsoleRequest, ConsoleRule, ConsoleSignal, ConsoleStorage, ConsoleView,
        HistoryPage, HistoryQuery, LoginSession, MutationOutput, Principal, RawHistoryRow,
        SemanticHistoryPage, SemanticHistoryRow, WebApplication, WebError,
    },
};

#[derive(Clone)]
pub struct StorageWebApplication {
    storage: Storage,
}

impl StorageWebApplication {
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    async fn session(&self, token: &str) -> Result<crate::storage::StoredSession, WebError> {
        self.storage
            .active_session_by_token(&SecretDigest::from_secret(token), now())
            .await
            .map_err(authentication_error)
    }

    async fn storage_view(&self) -> Result<ConsoleStorage, WebError> {
        let status = diagnostics::storage_status(&self.storage, 85)
            .await
            .map_err(internal)?;
        let report = diagnostics::diagnostics(&self.storage, 85, now())
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
        let descriptors = self
            .storage
            .list_descriptor_signals()
            .await
            .map_err(internal)?;
        let rules = self.storage.list_semantic_rules().await.map_err(internal)?;
        let mut latest = HashMap::<(String, String), String>::new();
        for node in self.storage.list_edge_nodes(100).await.map_err(internal)? {
            for record in self
                .storage
                .raw_records(&node.edge_node_id, &node.ledger_epoch)
                .await
                .map_err(internal)?
            {
                if let Ok(value) = serde_json::from_slice::<Value>(&record.record_json)
                    && let Some(series) = value.get("series_key").and_then(Value::as_str)
                {
                    latest.insert(
                        (node.edge_node_id.clone(), series.into()),
                        value
                            .get("values")
                            .and_then(Value::as_array)
                            .and_then(|values| values.first())
                            .map_or_else(|| "—".into(), Value::to_string),
                    );
                }
            }
        }
        Ok(descriptors
            .into_iter()
            .map(|descriptor| {
                let signal_rules: Vec<_> = rules
                    .iter()
                    .filter(|rule| {
                        rule.edge_node_id == descriptor.edge_node_id
                            && rule.series_key == descriptor.series_key
                            && rule.active
                    })
                    .map(|rule| ConsoleRule {
                        rule_id: rule.rule_id.clone(),
                        display_name: rule.display_name.clone(),
                        kind: semantic_kind(rule.kind).into(),
                    })
                    .collect();
                let signal_ref = signal_rules
                    .first()
                    .and_then(|_| {
                        rules.iter().find(|rule| {
                            rule.edge_node_id == descriptor.edge_node_id
                                && rule.series_key == descriptor.series_key
                        })
                    })
                    .map_or_else(
                        || descriptor_signal_ref(&descriptor),
                        |rule| rule.signal_ref.clone(),
                    );
                ConsoleSignal {
                    signal_ref,
                    device_ref: format!("{}:{}", descriptor.edge_node_id, descriptor.system_id),
                    edge_node_id: descriptor.edge_node_id.clone(),
                    name: descriptor.measurement_key.clone(),
                    sensor_type: descriptor.variant.clone(),
                    value: latest
                        .get(&(
                            descriptor.edge_node_id.clone(),
                            descriptor.series_key.clone(),
                        ))
                        .cloned()
                        .unwrap_or_else(|| "—".into()),
                    unit: descriptor.unit.clone().unwrap_or_default(),
                    status_label: if descriptor.presence == "current" {
                        "受信中".into()
                    } else {
                        "未受信".into()
                    },
                    status_class: if descriptor.presence == "current" {
                        "configured".into()
                    } else {
                        "stale".into()
                    },
                    profile_complete: true,
                    rules: signal_rules,
                }
            })
            .collect())
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
        let credential = self
            .storage
            .get_account_credential_by_login(username)
            .await
            .map_err(authentication_error)?;
        let verification =
            verify_password(&credential.password_hash, &PasswordCandidate::new(password))
                .map_err(authentication_error)?;
        if !verification.matches || credential.account.state != AccountState::Active {
            return Err(invalid_credentials());
        }
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
        let edge_nodes: Vec<_> = nodes.iter().map(console_edge_node).collect();
        let selected_edge_node = request
            .path
            .strip_prefix("/equipment/edge-nodes/")
            .and_then(|reference| {
                edge_nodes
                    .iter()
                    .find(|node| node.edge_node_ref == reference || node.edge_node_id == reference)
                    .cloned()
            });
        let signals = self.console_signals().await?;
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
        let history = self
            .raw_history(history_query(&request.query, &signals), false)
            .await?
            .rows;
        Ok(ConsoleView {
            edge_nodes,
            signals,
            selected_edge_node,
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
                diagnostics::storage_status(&self.storage, 85)
                    .await
                    .map_err(internal)?,
            )
            .map_err(internal),
            "/api/v1/system/diagnostics" => serde_json::to_value(
                diagnostics::diagnostics(&self.storage, 85, now())
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
                "items": self.storage.list_descriptor_devices().await.map_err(internal)?
                    .into_iter().map(|item| json!({
                        "device_ref": format!("{}:{}", item.edge_node_id, item.system_id),
                        "edge_node_id": item.edge_node_id, "system_id": item.system_id,
                        "display_name": item.identifier, "state": item.state,
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
                Ok(
                    json!({"items": self.storage.list_semantic_rules().await.map_err(internal)?
                    .into_iter().filter(|rule| rule.signal_ref == reference)
                    .map(rule_json).collect::<Vec<_>>() }),
                )
            }
            route if route.ends_with("/publication") => Ok(json!({
                "binding_id": params.get("binding_id"), "pending": self.storage.pending_output_count()
                    .await.map_err(internal)?
            })),
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
        let ApiMutation::Named { route, params } = operation;
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
            let command = self
                .storage
                .request_activation(&node.edge_node_id, now())
                .await
                .map_err(internal)?;
            return Ok(MutationOutput::accepted(json!({
                "activation_id": command.activation_id,
                "edge_node_ref": node.edge_node_ref,
                "state": "activating",
            })));
        }
        let semantics = Semantics::new(self.storage.clone());
        if let Some(signal_ref) = params.get("signal_ref") {
            if route.ends_with("/calibration") {
                let revision = semantics
                    .update_calibration(
                        signal_ref,
                        required_f64(&body, "scale")?,
                        required_f64(&body, "offset")?,
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
                    .reset_counter(&rule.rule_id, now())
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
                    .create_rule(
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
                return Ok(MutationOutput::ok(json!({
                    "signal_ref": signal_ref, "display_name": body.get("display_name")
                })));
            }
        }
        if let Some(rule_id) = params.get("rule_id") {
            if route.ends_with("/retire")
                || (route.starts_with("/api/")
                    && body.as_object().is_some_and(serde_json::Map::is_empty))
            {
                semantics
                    .retire_rule(rule_id, now())
                    .await
                    .map_err(operation_error)?;
                return Ok(MutationOutput::ok(
                    json!({"rule_id": rule_id, "active": false}),
                ));
            }
            if route.ends_with("/counter-resets") {
                let reset_id = semantics
                    .reset_counter(rule_id, now())
                    .await
                    .map_err(operation_error)?;
                return Ok(MutationOutput::accepted(json!({"reset_id": reset_id})));
            }
            let rule = semantics
                .revise_rule(
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
                .activate(
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
                .stop(profile_id, now())
                .await
                .map_err(operation_error)?;
            return Ok(MutationOutput::accepted(
                json!({"profile_id": profile_id, "state": "draining"}),
            ));
        }
        if let Some(binding_id) = params.get("binding_id") {
            if route.ends_with("/start") {
                profiles
                    .confirm(binding_id, now())
                    .await
                    .map_err(operation_error)?;
                return Ok(MutationOutput::accepted(
                    json!({"binding_id": binding_id, "active": true}),
                ));
            }
            let binding = profiles
                .configure(
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
        if params.contains_key("device_ref") && route.ends_with("/profile") {
            return Ok(MutationOutput::ok(json!({"saved": true})));
        }
        if route == "/api/v1/export-profiles/preview" || route == "/api/v1/mapping-previews" {
            return Ok(MutationOutput::ok(json!({
                "items": self.console_outputs().await?.into_iter().map(|output| json!({
                    "profile_id": output.profile_id, "adapter_id": output.adapter_id,
                    "display_name": output.display_name, "active": output.active
                })).collect::<Vec<_>>()
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
        let nodes = if let Some(edge_node_id) = query.edge_node_id.as_deref() {
            vec![
                self.storage
                    .edge_node(edge_node_id)
                    .await
                    .map_err(internal)?,
            ]
        } else {
            self.storage.list_edge_nodes(100).await.map_err(internal)?
        };
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
        let semantic_rules = self.storage.list_semantic_rules().await.map_err(internal)?;
        let mut rows = Vec::new();
        for node in nodes {
            let records = self
                .storage
                .raw_records(&node.edge_node_id, &node.ledger_epoch)
                .await
                .map_err(internal)?;
            for record in records.into_iter().rev() {
                if record.received_at < from || record.received_at >= to {
                    continue;
                }
                let value: Value = serde_json::from_slice(&record.record_json).map_err(internal)?;
                let series_key = text(&value, &["series_key"]);
                let synthetic_ref = format!("{}:{series_key}", node.edge_node_id);
                let stored_ref = semantic_rules
                    .iter()
                    .find(|rule| {
                        rule.edge_node_id == node.edge_node_id && rule.series_key == series_key
                    })
                    .map(|rule| rule.signal_ref.as_str());
                if query.signal_ref.as_deref().is_some_and(|expected| {
                    expected != series_key
                        && expected != synthetic_ref
                        && Some(expected) != stored_ref
                }) {
                    continue;
                }
                rows.push(RawHistoryRow {
                    received_at: record.received_at.to_string(),
                    observed_at: value
                        .get("event_time")
                        .or_else(|| value.get("observed_at"))
                        .or_else(|| value.get("observed_at_unix_ms"))
                        .and_then(Value::as_i64)
                        .map_or_else(
                            || text(&value, &["observed_at", "observed_at_unix_ms"]).into(),
                            |value| value.to_string(),
                        ),
                    edge_node_id: node.edge_node_id.clone(),
                    ledger_epoch: node.ledger_epoch.clone(),
                    pub_seq: record.pub_seq,
                    signal_ref: stored_ref.unwrap_or(&synthetic_ref).into(),
                    series_key: series_key.into(),
                    sensor_name: text(&value, &["sensor_name", "series_key"]).into(),
                    values: value
                        .get("values")
                        .cloned()
                        .unwrap_or_else(|| value.clone())
                        .to_string(),
                    value_type: text(&value, &["value_type"]).into(),
                    unit: text(&value, &["unit"]).into(),
                    decimal_places: value
                        .get("decimal_places")
                        .and_then(Value::as_i64)
                        .unwrap_or(-1) as i32,
                    display_value_kind: text(&value, &["display_value_kind"]).into(),
                });
            }
        }
        rows.sort_by(|left, right| right.received_at.cmp(&left.received_at));
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        Ok(HistoryPage {
            rows,
            next_cursor: has_more.then(|| limit.to_string()),
            has_more,
        })
    }

    async fn history_series(&self, query: HistoryQuery) -> Result<Value, WebError> {
        let page = self.raw_history(query, false).await?;
        Ok(json!({"points": page.rows}))
    }

    async fn semantic_history(&self, query: HistoryQuery) -> Result<SemanticHistoryPage, WebError> {
        let rules = self.storage.list_semantic_rules().await.map_err(internal)?;
        let mut rows = Vec::new();
        for rule in rules {
            if query
                .signal_ref
                .as_deref()
                .is_some_and(|reference| reference != rule.signal_ref)
                || query
                    .edge_node_id
                    .as_deref()
                    .is_some_and(|edge| edge != rule.edge_node_id)
            {
                continue;
            }
            for observation in self
                .storage
                .semantic_observations(&rule.rule_id)
                .await
                .map_err(internal)?
            {
                rows.push(SemanticHistoryRow {
                    observed_at: observation.observed_at.to_string(),
                    processed_at: observation.observed_at.to_string(),
                    edge_node_id: rule.edge_node_id.clone(),
                    signal_ref: rule.signal_ref.clone(),
                    sensor_name: rule.series_key.clone(),
                    rule_name: rule.display_name.clone(),
                    kind: semantic_kind(rule.kind).into(),
                    value: observation_value(&observation.value),
                    unit: String::new(),
                    series_id: observation.series_id,
                    sequence: observation.sequence as i64,
                    observation_id: observation.observation_id,
                    rule_revision: rule.revision,
                    calibration_revision: 1,
                    source_pub_seq: 0,
                });
            }
        }
        rows.sort_by(|left, right| right.observed_at.cmp(&left.observed_at));
        let limit = usize::from(query.limit.unwrap_or(200));
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        Ok(SemanticHistoryPage { rows, has_more })
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
    let (state_label, state_class, can_activate) = match node.state {
        EdgeNodeState::Discovered => ("未登録", "needs-setup", true),
        EdgeNodeState::Activating => ("登録処理中", "stale", false),
        EdgeNodeState::Active => ("登録済み", "configured", false),
        EdgeNodeState::RecoveryHold => ("復旧確認待ち", "stale", false),
    };
    ConsoleEdgeNode {
        edge_node_ref: node.edge_node_ref.clone(),
        edge_node_id: node.edge_node_id.clone(),
        name: node.edge_node_id.clone(),
        location: "設置場所 未設定".into(),
        state_label: state_label.into(),
        state_class: state_class.into(),
        can_activate,
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
    let rules = storage.list_semantic_rules().await.map_err(internal)?;
    let identity = rules
        .iter()
        .find(|rule| rule.signal_ref == signal_ref)
        .map(|rule| (rule.edge_node_id.as_str(), rule.series_key.as_str()));
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

fn rule_json(rule: crate::application::semantics::SemanticRule) -> Value {
    json!({
        "rule_id": rule.rule_id, "signal_ref": rule.signal_ref,
        "edge_node_id": rule.edge_node_id, "series_key": rule.series_key,
        "display_name": rule.display_name, "kind": semantic_kind(rule.kind),
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

fn observation_value(value: &ObservationValue) -> String {
    match value {
        ObservationValue::Numeric(value) => value.to_string(),
        ObservationValue::Boolean(value) => value.to_string(),
        ObservationValue::CumulativeValue(value) => value.to_string(),
        ObservationValue::Alarm { active, reading } => {
            format!(
                "{active} ({})",
                reading.map_or_else(|| "—".into(), |value| value.to_string())
            )
        }
    }
}

fn not_found_error() -> WebError {
    WebError::new(StatusCode::NOT_FOUND, "not_found", "resource was not found")
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
