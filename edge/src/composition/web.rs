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
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

use crate::{
    application::accounts::AccountService,
    auth::{
        password::{Password, PasswordCandidate, verify_password},
        principal::{AccountRole, AccountState, Principal as ApplicationPrincipal},
        session::{IDLE_SESSION_LIFETIME_MS, SecretDigest, SessionSecrets, SessionWindow},
    },
    diagnostics,
    storage::{AuditActor, EdgeNodeState, Storage, StorageError},
    web::{
        ApiMutation, ApiQuery, ConsoleAccount, ConsoleAudit, ConsoleEdgeNode, ConsoleRequest,
        ConsoleStorage, ConsoleView, HistoryPage, HistoryQuery, LoginSession, MutationOutput,
        Principal, RawHistoryRow, SemanticHistoryPage, WebApplication, WebError,
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
        Ok(ConsoleView {
            edge_nodes,
            selected_edge_node,
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
        let ApiQuery::Named { route, .. } = operation else {
            return Err(not_implemented(
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
            // Task 6 owns the application-service implementation for these
            // views. Keep their wire shape stable until that adapter is joined.
            "/api/v1/devices"
            | "/api/v1/signals"
            | "/api/v1/setup/devices"
            | "/api/v1/semantic-definitions"
            | "/api/v1/output-adapters"
            | "/api/v1/export-profiles"
            | "/api/v1/output-routes" => Ok(json!({"items": []})),
            _ => Err(not_implemented("application operation is not connected")),
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
        let _ = principal;
        Err(not_implemented(
            "Task 6 application mutation adapter is not connected",
        ))
    }

    async fn raw_history(
        &self,
        query: HistoryQuery,
        export: bool,
    ) -> Result<HistoryPage, WebError> {
        let Some(edge_node_id) = query.edge_node_id.as_deref() else {
            return Ok(HistoryPage {
                rows: Vec::new(),
                next_cursor: None,
                has_more: false,
            });
        };
        let node = self
            .storage
            .edge_node(edge_node_id)
            .await
            .map_err(internal)?;
        let records = self
            .storage
            .raw_records(edge_node_id, &node.ledger_epoch)
            .await
            .map_err(internal)?;
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
        let mut rows = Vec::new();
        for record in records.into_iter().rev() {
            if record.received_at < from || record.received_at >= to {
                continue;
            }
            let value: Value = serde_json::from_slice(&record.record_json).map_err(internal)?;
            let signal_ref = text(&value, &["signal_ref", "series_key"]);
            if query
                .signal_ref
                .as_deref()
                .is_some_and(|expected| expected != signal_ref)
            {
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
                edge_node_id: edge_node_id.into(),
                ledger_epoch: node.ledger_epoch.clone(),
                pub_seq: record.pub_seq,
                signal_ref: signal_ref.into(),
                series_key: text(&value, &["series_key"]).into(),
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

    async fn semantic_history(
        &self,
        _query: HistoryQuery,
    ) -> Result<SemanticHistoryPage, WebError> {
        Ok(SemanticHistoryPage {
            rows: Vec::new(),
            has_more: false,
        })
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

fn not_implemented(message: &'static str) -> WebError {
    WebError::new(StatusCode::NOT_IMPLEMENTED, "not_implemented", message)
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
