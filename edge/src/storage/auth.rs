use serde_json::{Value, json};
use sqlx::{Postgres, Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::auth::{
    password::{PasswordHash, normalize_login_id},
    principal::{AccountRole, AccountState},
    session::{SecretDigest, SessionWindow},
};

use super::{Storage, StorageError, StorageInner};

const ACCOUNT_LOCK_KEY: i64 = 4_968_354_283;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub account_ref: String,
    pub login_id: String,
    pub display_name: String,
    pub role: AccountRole,
    pub state: AccountState,
    pub must_change_password: bool,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub disabled_at: Option<i64>,
}

#[derive(Clone)]
pub struct AccountCredential {
    pub account: Account,
    pub password_hash: PasswordHash,
}

impl std::fmt::Debug for AccountCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccountCredential")
            .field("account", &self.account)
            .field("password_hash", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone)]
pub struct AccountProvision {
    pub login_id: String,
    pub display_name: String,
    pub role: AccountRole,
    pub password_hash: PasswordHash,
    pub must_change_password: bool,
    pub require_unowned: bool,
}

impl std::fmt::Debug for AccountProvision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccountProvision")
            .field("login_id", &self.login_id)
            .field("display_name", &self.display_name)
            .field("role", &self.role)
            .field("password_hash", &"[REDACTED]")
            .field("must_change_password", &self.must_change_password)
            .field("require_unowned", &self.require_unowned)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditActor {
    Account(String),
    LocalCli,
    System(String),
}

impl AuditActor {
    #[must_use]
    pub fn account(account_ref: impl Into<String>) -> Self {
        Self::Account(account_ref.into())
    }

    #[must_use]
    pub fn local_cli() -> Self {
        Self::LocalCli
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditEvent {
    pub audit_row_id: i64,
    pub occurred_at: i64,
    pub actor_class: String,
    pub actor_ref: String,
    pub actor_login_id: Option<String>,
    pub actor_display_name: Option<String>,
    pub operation: String,
    pub resource_ref: String,
    pub outcome: String,
    pub summary: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSession {
    pub session_ref: String,
    pub token_digest: SecretDigest,
    pub csrf_digest: SecretDigest,
    pub account: Account,
    pub issued_at: i64,
    pub last_seen_at: i64,
    pub idle_expires_at: i64,
    pub absolute_expires_at: i64,
    pub revoked_at: Option<i64>,
}

impl Storage {
    pub async fn create_account(
        &self,
        provision: AccountProvision,
        actor: AuditActor,
        now: i64,
    ) -> Result<Account, StorageError> {
        let login_id = normalize_login_id(&provision.login_id)
            .map_err(|error| StorageError::InvalidAccount(error.to_string()))?;
        validate_display_name(&provision.display_name)?;
        validate_now(now)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                if provision.require_unowned {
                    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM edge_accounts")
                        .fetch_one(&mut *tx)
                        .await?;
                    if count != 0 {
                        return Err(StorageError::AccountConflict);
                    }
                }
                let account_ref = new_account_ref();
                let result = sqlx::query(
                    "INSERT INTO edge_accounts(account_ref, login_id, login_id_normalized, \
                     display_name, password_phc, role, state, must_change_password, revision, \
                     created_at, updated_at, disabled_at) \
                     VALUES(?, ?, ?, ?, ?, ?, 'active', ?, 1, ?, ?, NULL)",
                )
                .bind(&account_ref)
                .bind(&login_id)
                .bind(&login_id)
                .bind(provision.display_name.trim())
                .bind(provision.password_hash.expose_secret())
                .bind(provision.role.as_str())
                .bind(provision.must_change_password)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await;
                if let Err(error) = result {
                    if is_unique_violation(&error) {
                        return Err(StorageError::AccountConflict);
                    }
                    return Err(error.into());
                }
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "account.create",
                    &account_ref,
                    json!({
                        "login_id": login_id,
                        "display_name": provision.display_name.trim(),
                        "role": provision.role.as_str(),
                        "must_change_password": provision.must_change_password
                    }),
                )
                .await?;
                let account = load_account_sqlite(&mut tx, &account_ref).await?;
                tx.commit().await?;
                Ok(account)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                lock_accounts_postgres(&mut tx).await?;
                if provision.require_unowned {
                    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM edge_accounts")
                        .fetch_one(&mut *tx)
                        .await?;
                    if count != 0 {
                        return Err(StorageError::AccountConflict);
                    }
                }
                let account_ref = new_account_ref();
                let result = sqlx::query(
                    "INSERT INTO edge_accounts(account_ref, login_id, login_id_normalized, \
                     display_name, password_phc, role, state, must_change_password, revision, \
                     created_at, updated_at, disabled_at) \
                     VALUES($1,$2,$3,$4,$5,$6,'active',$7,1,$8,$8,NULL)",
                )
                .bind(&account_ref)
                .bind(&login_id)
                .bind(&login_id)
                .bind(provision.display_name.trim())
                .bind(provision.password_hash.expose_secret())
                .bind(provision.role.as_str())
                .bind(provision.must_change_password)
                .bind(now)
                .execute(&mut *tx)
                .await;
                if let Err(error) = result {
                    if is_unique_violation(&error) {
                        return Err(StorageError::AccountConflict);
                    }
                    return Err(error.into());
                }
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "account.create",
                    &account_ref,
                    json!({
                        "login_id": login_id,
                        "display_name": provision.display_name.trim(),
                        "role": provision.role.as_str(),
                        "must_change_password": provision.must_change_password
                    }),
                )
                .await?;
                let account = load_account_postgres(&mut tx, &account_ref).await?;
                tx.commit().await?;
                Ok(account)
            }
        }
    }

    pub async fn get_account(&self, account_ref: &str) -> Result<Account, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let row = account_select_sqlite(account_ref)
                    .fetch_optional(pool)
                    .await?;
                row.map(|row| decode_account(&row))
                    .transpose()?
                    .ok_or(StorageError::AccountNotFound)
            }
            StorageInner::Postgres { pool, .. } => {
                let row = account_select_postgres(account_ref)
                    .fetch_optional(pool)
                    .await?;
                row.map(|row| decode_account(&row))
                    .transpose()?
                    .ok_or(StorageError::AccountNotFound)
            }
        }
    }

    pub async fn get_account_credential_by_login(
        &self,
        login_id: &str,
    ) -> Result<AccountCredential, StorageError> {
        let login_id = normalize_login_id(login_id).map_err(|_| StorageError::AccountNotFound)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let row = sqlx::query(
                    "SELECT account_ref, login_id, display_name, role, state, \
                 must_change_password, revision, created_at, updated_at, disabled_at, password_phc \
                 FROM edge_accounts WHERE login_id_normalized = ?",
                )
                .bind(login_id)
                .fetch_optional(pool)
                .await?
                .ok_or(StorageError::AccountNotFound)?;
                let account = decode_account(&row)?;
                let password_phc: String = row.try_get("password_phc")?;
                Ok(AccountCredential {
                    account,
                    password_hash: PasswordHash::new(password_phc),
                })
            }
            StorageInner::Postgres { pool, .. } => {
                let row = sqlx::query(
                    "SELECT account_ref, login_id, display_name, role, state, \
                 must_change_password, revision, created_at, updated_at, disabled_at, password_phc \
                 FROM edge_accounts WHERE login_id_normalized = $1",
                )
                .bind(login_id)
                .fetch_optional(pool)
                .await?
                .ok_or(StorageError::AccountNotFound)?;
                let account = decode_account(&row)?;
                let password_phc: String = row.try_get("password_phc")?;
                Ok(AccountCredential {
                    account,
                    password_hash: PasswordHash::new(password_phc),
                })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_account(
        &self,
        account_ref: &str,
        expected_revision: i64,
        display_name: &str,
        role: AccountRole,
        actor: AuditActor,
        now: i64,
    ) -> Result<Account, StorageError> {
        validate_revision(expected_revision)?;
        validate_display_name(display_name)?;
        validate_now(now)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let current = load_account_sqlite(&mut tx, account_ref).await?;
                validate_mutation(&current, expected_revision)?;
                guard_last_admin_sqlite(&mut tx, &current, role, false).await?;
                let result = sqlx::query(
                    "UPDATE edge_accounts SET display_name=?, role=?, revision=revision+1, \
                     updated_at=? WHERE account_ref=? AND revision=? AND state='active'",
                )
                .bind(display_name.trim())
                .bind(role.as_str())
                .bind(now)
                .bind(account_ref)
                .bind(expected_revision)
                .execute(&mut *tx)
                .await?;
                ensure_one_revision(result.rows_affected())?;
                if current.role != role {
                    revoke_all_sqlite(&mut tx, account_ref, now).await?;
                }
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "account.update",
                    account_ref,
                    json!({
                        "display_name": display_name.trim(),
                        "role": role.as_str(),
                        "revision": expected_revision + 1
                    }),
                )
                .await?;
                let account = load_account_sqlite(&mut tx, account_ref).await?;
                tx.commit().await?;
                Ok(account)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                lock_accounts_postgres(&mut tx).await?;
                let current = load_account_postgres_for_update(&mut tx, account_ref).await?;
                validate_mutation(&current, expected_revision)?;
                guard_last_admin_postgres(&mut tx, &current, role, false).await?;
                let result = sqlx::query(
                    "UPDATE edge_accounts SET display_name=$1, role=$2, revision=revision+1, \
                     updated_at=$3 WHERE account_ref=$4 AND revision=$5 AND state='active'",
                )
                .bind(display_name.trim())
                .bind(role.as_str())
                .bind(now)
                .bind(account_ref)
                .bind(expected_revision)
                .execute(&mut *tx)
                .await?;
                ensure_one_revision(result.rows_affected())?;
                if current.role != role {
                    revoke_all_postgres(&mut tx, account_ref, now).await?;
                }
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "account.update",
                    account_ref,
                    json!({
                        "display_name": display_name.trim(),
                        "role": role.as_str(),
                        "revision": expected_revision + 1
                    }),
                )
                .await?;
                let account = load_account_postgres(&mut tx, account_ref).await?;
                tx.commit().await?;
                Ok(account)
            }
        }
    }

    pub async fn disable_account(
        &self,
        account_ref: &str,
        expected_revision: i64,
        actor: AuditActor,
        now: i64,
    ) -> Result<Account, StorageError> {
        validate_revision(expected_revision)?;
        validate_now(now)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let current = load_account_sqlite(&mut tx, account_ref).await?;
                validate_mutation(&current, expected_revision)?;
                guard_last_admin_sqlite(&mut tx, &current, current.role, true).await?;
                let result = sqlx::query(
                    "UPDATE edge_accounts SET state='disabled', disabled_at=?, updated_at=?, \
                     revision=revision+1 WHERE account_ref=? AND revision=? AND state='active'",
                )
                .bind(now)
                .bind(now)
                .bind(account_ref)
                .bind(expected_revision)
                .execute(&mut *tx)
                .await?;
                ensure_one_revision(result.rows_affected())?;
                revoke_all_sqlite(&mut tx, account_ref, now).await?;
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "account.disable",
                    account_ref,
                    json!({"revision": expected_revision + 1}),
                )
                .await?;
                let account = load_account_sqlite(&mut tx, account_ref).await?;
                tx.commit().await?;
                Ok(account)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                lock_accounts_postgres(&mut tx).await?;
                let current = load_account_postgres_for_update(&mut tx, account_ref).await?;
                validate_mutation(&current, expected_revision)?;
                guard_last_admin_postgres(&mut tx, &current, current.role, true).await?;
                let result = sqlx::query(
                    "UPDATE edge_accounts SET state='disabled', disabled_at=$1, updated_at=$1, \
                     revision=revision+1 WHERE account_ref=$2 AND revision=$3 AND state='active'",
                )
                .bind(now)
                .bind(account_ref)
                .bind(expected_revision)
                .execute(&mut *tx)
                .await?;
                ensure_one_revision(result.rows_affected())?;
                revoke_all_postgres(&mut tx, account_ref, now).await?;
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "account.disable",
                    account_ref,
                    json!({"revision": expected_revision + 1}),
                )
                .await?;
                let account = load_account_postgres(&mut tx, account_ref).await?;
                tx.commit().await?;
                Ok(account)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn replace_account_password(
        &self,
        account_ref: &str,
        expected_revision: i64,
        password_hash: PasswordHash,
        must_change_password: bool,
        actor: AuditActor,
        now: i64,
    ) -> Result<Account, StorageError> {
        validate_revision(expected_revision)?;
        validate_now(now)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let current = load_account_sqlite(&mut tx, account_ref).await?;
                validate_mutation(&current, expected_revision)?;
                let result = sqlx::query(
                    "UPDATE edge_accounts SET password_phc=?, must_change_password=?, \
                     updated_at=?, revision=revision+1 \
                     WHERE account_ref=? AND revision=? AND state='active'",
                )
                .bind(password_hash.expose_secret())
                .bind(must_change_password)
                .bind(now)
                .bind(account_ref)
                .bind(expected_revision)
                .execute(&mut *tx)
                .await?;
                ensure_one_revision(result.rows_affected())?;
                revoke_all_sqlite(&mut tx, account_ref, now).await?;
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "account.password_replace",
                    account_ref,
                    json!({
                        "must_change_password": must_change_password,
                        "revision": expected_revision + 1
                    }),
                )
                .await?;
                let account = load_account_sqlite(&mut tx, account_ref).await?;
                tx.commit().await?;
                Ok(account)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let current = load_account_postgres_for_update(&mut tx, account_ref).await?;
                validate_mutation(&current, expected_revision)?;
                let result = sqlx::query(
                    "UPDATE edge_accounts SET password_phc=$1, must_change_password=$2, \
                     updated_at=$3, revision=revision+1 \
                     WHERE account_ref=$4 AND revision=$5 AND state='active'",
                )
                .bind(password_hash.expose_secret())
                .bind(must_change_password)
                .bind(now)
                .bind(account_ref)
                .bind(expected_revision)
                .execute(&mut *tx)
                .await?;
                ensure_one_revision(result.rows_affected())?;
                revoke_all_postgres(&mut tx, account_ref, now).await?;
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "account.password_replace",
                    account_ref,
                    json!({
                        "must_change_password": must_change_password,
                        "revision": expected_revision + 1
                    }),
                )
                .await?;
                let account = load_account_postgres(&mut tx, account_ref).await?;
                tx.commit().await?;
                Ok(account)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_session(
        &self,
        account_ref: &str,
        expected_account_revision: i64,
        session_ref: &str,
        token_digest: SecretDigest,
        csrf_digest: SecretDigest,
        window: SessionWindow,
        now: i64,
    ) -> Result<StoredSession, StorageError> {
        validate_now(now)?;
        validate_revision(expected_account_revision)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let result = sqlx::query(
                    "INSERT INTO edge_sessions(session_ref, token_sha256, csrf_sha256, \
                     account_ref, issued_at, last_seen_at, idle_expires_at, \
                     absolute_expires_at, revoked_at) \
                     SELECT ?,?,?,?,?,?,?,?,NULL FROM edge_accounts \
                     WHERE account_ref=? AND revision=? AND state='active'",
                )
                .bind(session_ref)
                .bind(token_digest.as_bytes().as_slice())
                .bind(csrf_digest.as_bytes().as_slice())
                .bind(account_ref)
                .bind(window.issued_at())
                .bind(window.last_seen_at())
                .bind(window.idle_expires_at())
                .bind(window.absolute_expires_at())
                .bind(account_ref)
                .bind(expected_account_revision)
                .execute(&mut *tx)
                .await?;
                if result.rows_affected() != 1 {
                    let current = account_select_sqlite(account_ref)
                        .fetch_optional(&mut *tx)
                        .await?
                        .map(|row| decode_account(&row))
                        .transpose()?;
                    return match current {
                        Some(account) if account.state == AccountState::Active => {
                            Err(StorageError::RevisionMismatch)
                        }
                        _ => Err(StorageError::AccountNotFound),
                    };
                }
                insert_audit_sqlite(
                    &mut tx,
                    &AuditActor::account(account_ref),
                    now,
                    "session.login",
                    session_ref,
                    json!({}),
                )
                .await?;
                let session =
                    load_session_sqlite(&mut tx, session_ref, token_digest, csrf_digest).await?;
                tx.commit().await?;
                Ok(session)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let account = load_account_postgres_for_update(&mut tx, account_ref).await?;
                validate_mutation(&account, expected_account_revision)?;
                sqlx::query(
                    "INSERT INTO edge_sessions(session_ref, token_sha256, csrf_sha256, \
                     account_ref, issued_at, last_seen_at, idle_expires_at, \
                     absolute_expires_at, revoked_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,NULL)",
                )
                .bind(session_ref)
                .bind(token_digest.as_bytes().as_slice())
                .bind(csrf_digest.as_bytes().as_slice())
                .bind(account_ref)
                .bind(window.issued_at())
                .bind(window.last_seen_at())
                .bind(window.idle_expires_at())
                .bind(window.absolute_expires_at())
                .execute(&mut *tx)
                .await?;
                insert_audit_postgres(
                    &mut tx,
                    &AuditActor::account(account_ref),
                    now,
                    "session.login",
                    session_ref,
                    json!({}),
                )
                .await?;
                let session =
                    load_session_postgres(&mut tx, session_ref, token_digest, csrf_digest).await?;
                tx.commit().await?;
                Ok(session)
            }
        }
    }

    pub async fn active_session_by_token(
        &self,
        token_digest: &SecretDigest,
        now: i64,
    ) -> Result<StoredSession, StorageError> {
        validate_now(now)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let row = sqlx::query(
                    "SELECT s.session_ref,s.token_sha256,s.csrf_sha256,s.issued_at,s.last_seen_at, \
                 s.idle_expires_at,s.absolute_expires_at,s.revoked_at, \
                 a.account_ref,a.login_id,a.display_name,a.role,a.state,a.must_change_password, \
                 a.revision,a.created_at,a.updated_at,a.disabled_at \
                 FROM edge_sessions s JOIN edge_accounts a ON a.account_ref=s.account_ref \
                 WHERE s.token_sha256=? AND s.revoked_at IS NULL AND s.idle_expires_at>? \
                 AND s.absolute_expires_at>? AND a.state='active'",
                )
                .bind(token_digest.as_bytes().as_slice())
                .bind(now)
                .bind(now)
                .fetch_optional(pool)
                .await?
                .ok_or(StorageError::SessionNotFound)?;
                decode_session(&row)
            }
            StorageInner::Postgres { pool, .. } => {
                let row = sqlx::query(
                    "SELECT s.session_ref,s.token_sha256,s.csrf_sha256,s.issued_at,s.last_seen_at, \
                 s.idle_expires_at,s.absolute_expires_at,s.revoked_at, \
                 a.account_ref,a.login_id,a.display_name,a.role,a.state,a.must_change_password, \
                 a.revision,a.created_at,a.updated_at,a.disabled_at \
                 FROM edge_sessions s JOIN edge_accounts a ON a.account_ref=s.account_ref \
                 WHERE s.token_sha256=$1 AND s.revoked_at IS NULL AND s.idle_expires_at>$2 \
                 AND s.absolute_expires_at>$2 AND a.state='active'",
                )
                .bind(token_digest.as_bytes().as_slice())
                .bind(now)
                .fetch_optional(pool)
                .await?
                .ok_or(StorageError::SessionNotFound)?;
                decode_session(&row)
            }
        }
    }

    pub async fn touch_session(
        &self,
        session_ref: &str,
        now: i64,
        idle_expires_at: i64,
    ) -> Result<(), StorageError> {
        validate_now(now)?;
        let affected = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => sqlx::query(
                "UPDATE edge_sessions SET last_seen_at=?, \
                 idle_expires_at=min(?, absolute_expires_at) \
                 WHERE session_ref=? AND revoked_at IS NULL AND idle_expires_at>? \
                 AND absolute_expires_at>? AND EXISTS(SELECT 1 FROM edge_accounts a \
                 WHERE a.account_ref=edge_sessions.account_ref AND a.state='active')",
            )
            .bind(now)
            .bind(idle_expires_at)
            .bind(session_ref)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await?
            .rows_affected(),
            StorageInner::Postgres { pool, .. } => sqlx::query(
                "UPDATE edge_sessions SET last_seen_at=$1, \
                 idle_expires_at=least($2, absolute_expires_at) \
                 WHERE session_ref=$3 AND revoked_at IS NULL AND idle_expires_at>$1 \
                 AND absolute_expires_at>$1 AND EXISTS(SELECT 1 FROM edge_accounts a \
                 WHERE a.account_ref=edge_sessions.account_ref AND a.state='active')",
            )
            .bind(now)
            .bind(idle_expires_at)
            .bind(session_ref)
            .execute(pool)
            .await?
            .rows_affected(),
        };
        if affected == 1 {
            Ok(())
        } else {
            Err(StorageError::SessionNotFound)
        }
    }

    pub async fn revoke_session(
        &self,
        session_ref: &str,
        actor: AuditActor,
        now: i64,
    ) -> Result<(), StorageError> {
        validate_now(now)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let affected = sqlx::query(
                    "UPDATE edge_sessions SET revoked_at=? \
                     WHERE session_ref=? AND revoked_at IS NULL",
                )
                .bind(now)
                .bind(session_ref)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if affected != 1 {
                    return Err(StorageError::SessionNotFound);
                }
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "session.logout",
                    session_ref,
                    json!({}),
                )
                .await?;
                tx.commit().await?;
                Ok(())
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let affected = sqlx::query(
                    "UPDATE edge_sessions SET revoked_at=$1 \
                     WHERE session_ref=$2 AND revoked_at IS NULL",
                )
                .bind(now)
                .bind(session_ref)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if affected != 1 {
                    return Err(StorageError::SessionNotFound);
                }
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "session.logout",
                    session_ref,
                    json!({}),
                )
                .await?;
                tx.commit().await?;
                Ok(())
            }
        }
    }

    pub async fn list_audit_events(&self, limit: i64) -> Result<Vec<AuditEvent>, StorageError> {
        if !(1..=100).contains(&limit) {
            return Err(StorageError::InvalidAccount(
                "audit limit must be between 1 and 100".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT audit_row_id,occurred_at,actor_class,actor_ref,actor_login_id, \
                     actor_display_name,operation,resource_ref,outcome,summary_json \
                     FROM audit_events ORDER BY audit_row_id DESC LIMIT ?",
                )
                .bind(limit)
                .fetch_all(pool)
                .await?;
                rows.iter().map(decode_audit_sqlite).collect()
            }
            StorageInner::Postgres { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT audit_row_id,occurred_at,actor_class,actor_ref,actor_login_id, \
                     actor_display_name,operation,resource_ref,outcome,summary_json \
                     FROM audit_events ORDER BY audit_row_id DESC LIMIT $1",
                )
                .bind(limit)
                .fetch_all(pool)
                .await?;
                rows.iter().map(decode_audit_postgres).collect()
            }
        }
    }
}

fn validate_display_name(display_name: &str) -> Result<(), StorageError> {
    let display_name = display_name.trim();
    if display_name.is_empty()
        || display_name.len() > 128
        || display_name.chars().any(char::is_control)
    {
        return Err(StorageError::InvalidAccount(
            "display name must contain 1 to 128 UTF-8 bytes and no control characters".into(),
        ));
    }
    Ok(())
}

fn validate_now(now: i64) -> Result<(), StorageError> {
    if now < 0 {
        Err(StorageError::InvalidAccount(
            "timestamp must not be negative".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_revision(revision: i64) -> Result<(), StorageError> {
    if revision > 0 {
        Ok(())
    } else {
        Err(StorageError::RevisionMismatch)
    }
}

fn validate_mutation(account: &Account, expected_revision: i64) -> Result<(), StorageError> {
    if account.state != AccountState::Active {
        return Err(StorageError::AccountNotFound);
    }
    if account.revision != expected_revision {
        return Err(StorageError::RevisionMismatch);
    }
    Ok(())
}

fn ensure_one_revision(affected: u64) -> Result<(), StorageError> {
    if affected == 1 {
        Ok(())
    } else {
        Err(StorageError::RevisionMismatch)
    }
}

fn new_account_ref() -> String {
    format!("acct_{}", Uuid::new_v4().simple())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
}

fn parse_role(value: &str) -> Result<AccountRole, StorageError> {
    match value {
        "viewer" => Ok(AccountRole::Viewer),
        "admin" => Ok(AccountRole::Admin),
        "system_admin" => Ok(AccountRole::SystemAdmin),
        _ => Err(StorageError::InvalidAccount(
            "database contains an invalid role".into(),
        )),
    }
}

fn parse_state(value: &str) -> Result<AccountState, StorageError> {
    match value {
        "active" => Ok(AccountState::Active),
        "disabled" => Ok(AccountState::Disabled),
        _ => Err(StorageError::InvalidAccount(
            "database contains an invalid account state".into(),
        )),
    }
}

fn decode_account<R>(row: &R) -> Result<Account, StorageError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    for<'a> String: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> i64: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> bool: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> Option<i64>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
{
    let role: String = row.try_get("role")?;
    let state: String = row.try_get("state")?;
    Ok(Account {
        account_ref: row.try_get("account_ref")?,
        login_id: row.try_get("login_id")?,
        display_name: row.try_get("display_name")?,
        role: parse_role(&role)?,
        state: parse_state(&state)?,
        must_change_password: row.try_get("must_change_password")?,
        revision: row.try_get("revision")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        disabled_at: row.try_get("disabled_at")?,
    })
}

// Row is not object-safe across database backends; these wrappers let inference choose the row.
fn decode_session<R>(row: &R) -> Result<StoredSession, StorageError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    for<'a> String: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> Vec<u8>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> i64: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> bool: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> Option<i64>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
{
    let token: Vec<u8> = row.try_get("token_sha256")?;
    let csrf: Vec<u8> = row.try_get("csrf_sha256")?;
    let token_digest = digest_from_bytes(&token)?;
    let csrf_digest = digest_from_bytes(&csrf)?;
    Ok(StoredSession {
        session_ref: row.try_get("session_ref")?,
        token_digest,
        csrf_digest,
        account: decode_account(row)?,
        issued_at: row.try_get("issued_at")?,
        last_seen_at: row.try_get("last_seen_at")?,
        idle_expires_at: row.try_get("idle_expires_at")?,
        absolute_expires_at: row.try_get("absolute_expires_at")?,
        revoked_at: row.try_get("revoked_at")?,
    })
}

fn digest_from_bytes(value: &[u8]) -> Result<SecretDigest, StorageError> {
    let bytes: [u8; 32] = value.try_into().map_err(|_| {
        StorageError::InvalidAccount("database contains an invalid secret digest".into())
    })?;
    Ok(SecretDigest::from_digest(bytes))
}

fn account_select_sqlite(
    account_ref: &str,
) -> sqlx::query::Query<'_, Sqlite, sqlx::sqlite::SqliteArguments<'_>> {
    sqlx::query(
        "SELECT account_ref,login_id,display_name,role,state,must_change_password,revision, \
         created_at,updated_at,disabled_at FROM edge_accounts WHERE account_ref=?",
    )
    .bind(account_ref)
}

fn account_select_postgres(
    account_ref: &str,
) -> sqlx::query::Query<'_, Postgres, sqlx::postgres::PgArguments> {
    sqlx::query(
        "SELECT account_ref,login_id,display_name,role,state,must_change_password,revision, \
         created_at,updated_at,disabled_at FROM edge_accounts WHERE account_ref=$1",
    )
    .bind(account_ref)
}

async fn load_account_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    account_ref: &str,
) -> Result<Account, StorageError> {
    let row = account_select_sqlite(account_ref)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StorageError::AccountNotFound)?;
    decode_account(&row)
}

async fn load_account_postgres(
    tx: &mut Transaction<'_, Postgres>,
    account_ref: &str,
) -> Result<Account, StorageError> {
    let row = account_select_postgres(account_ref)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StorageError::AccountNotFound)?;
    decode_account(&row)
}

async fn load_account_postgres_for_update(
    tx: &mut Transaction<'_, Postgres>,
    account_ref: &str,
) -> Result<Account, StorageError> {
    let row = sqlx::query(
        "SELECT account_ref,login_id,display_name,role,state,must_change_password,revision, \
         created_at,updated_at,disabled_at FROM edge_accounts \
         WHERE account_ref=$1 FOR UPDATE",
    )
    .bind(account_ref)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StorageError::AccountNotFound)?;
    decode_account(&row)
}

async fn lock_accounts_postgres(tx: &mut Transaction<'_, Postgres>) -> Result<(), StorageError> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ACCOUNT_LOCK_KEY)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn guard_last_admin_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    current: &Account,
    new_role: AccountRole,
    disabling: bool,
) -> Result<(), StorageError> {
    if current.role == AccountRole::SystemAdmin
        && current.state == AccountState::Active
        && (disabling || new_role != AccountRole::SystemAdmin)
    {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM edge_accounts WHERE role='system_admin' AND state='active'",
        )
        .fetch_one(&mut **tx)
        .await?;
        if count <= 1 {
            return Err(StorageError::LastSystemAdmin);
        }
    }
    Ok(())
}

async fn guard_last_admin_postgres(
    tx: &mut Transaction<'_, Postgres>,
    current: &Account,
    new_role: AccountRole,
    disabling: bool,
) -> Result<(), StorageError> {
    if current.role == AccountRole::SystemAdmin
        && current.state == AccountState::Active
        && (disabling || new_role != AccountRole::SystemAdmin)
    {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM edge_accounts WHERE role='system_admin' AND state='active'",
        )
        .fetch_one(&mut **tx)
        .await?;
        if count <= 1 {
            return Err(StorageError::LastSystemAdmin);
        }
    }
    Ok(())
}

async fn revoke_all_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    account_ref: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE edge_sessions SET revoked_at=? \
         WHERE account_ref=? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(account_ref)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn revoke_all_postgres(
    tx: &mut Transaction<'_, Postgres>,
    account_ref: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE edge_sessions SET revoked_at=$1 \
         WHERE account_ref=$2 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(account_ref)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn actor_snapshot_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    actor: &AuditActor,
) -> Result<(String, String, Option<String>, Option<String>), StorageError> {
    match actor {
        AuditActor::Account(account_ref) => {
            let row =
                sqlx::query("SELECT login_id, display_name FROM edge_accounts WHERE account_ref=?")
                    .bind(account_ref)
                    .fetch_optional(&mut **tx)
                    .await?
                    .ok_or(StorageError::AccountNotFound)?;
            Ok((
                "account".into(),
                account_ref.clone(),
                Some(row.try_get("login_id")?),
                Some(row.try_get("display_name")?),
            ))
        }
        AuditActor::LocalCli => Ok(("local_cli".into(), "local_cli".into(), None, None)),
        AuditActor::System(reference) => Ok(("system".into(), reference.clone(), None, None)),
    }
}

async fn actor_snapshot_postgres(
    tx: &mut Transaction<'_, Postgres>,
    actor: &AuditActor,
) -> Result<(String, String, Option<String>, Option<String>), StorageError> {
    match actor {
        AuditActor::Account(account_ref) => {
            let row = sqlx::query(
                "SELECT login_id, display_name FROM edge_accounts WHERE account_ref=$1",
            )
            .bind(account_ref)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or(StorageError::AccountNotFound)?;
            Ok((
                "account".into(),
                account_ref.clone(),
                Some(row.try_get("login_id")?),
                Some(row.try_get("display_name")?),
            ))
        }
        AuditActor::LocalCli => Ok(("local_cli".into(), "local_cli".into(), None, None)),
        AuditActor::System(reference) => Ok(("system".into(), reference.clone(), None, None)),
    }
}

async fn insert_audit_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    actor: &AuditActor,
    now: i64,
    operation: &str,
    resource_ref: &str,
    summary: Value,
) -> Result<(), StorageError> {
    let (class, reference, login, display) = actor_snapshot_sqlite(tx, actor).await?;
    sqlx::query(
        "INSERT INTO audit_events(occurred_at,actor_class,actor_ref,actor_login_id, \
         actor_display_name,operation,resource_ref,outcome,summary_json) \
         VALUES(?,?,?,?,?,?,?,'success',?)",
    )
    .bind(now)
    .bind(class)
    .bind(reference)
    .bind(login)
    .bind(display)
    .bind(operation)
    .bind(resource_ref)
    .bind(serde_json::to_vec(&summary).map_err(StorageError::EncodeRecord)?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_audit_postgres(
    tx: &mut Transaction<'_, Postgres>,
    actor: &AuditActor,
    now: i64,
    operation: &str,
    resource_ref: &str,
    summary: Value,
) -> Result<(), StorageError> {
    let (class, reference, login, display) = actor_snapshot_postgres(tx, actor).await?;
    sqlx::query(
        "INSERT INTO audit_events(occurred_at,actor_class,actor_ref,actor_login_id, \
         actor_display_name,operation,resource_ref,outcome,summary_json) \
         VALUES($1,$2,$3,$4,$5,$6,$7,'success',$8)",
    )
    .bind(now)
    .bind(class)
    .bind(reference)
    .bind(login)
    .bind(display)
    .bind(operation)
    .bind(resource_ref)
    .bind(summary)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn load_session_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    session_ref: &str,
    token_digest: SecretDigest,
    csrf_digest: SecretDigest,
) -> Result<StoredSession, StorageError> {
    let row = sqlx::query(
        "SELECT s.session_ref,s.issued_at,s.last_seen_at,s.idle_expires_at, \
         s.absolute_expires_at,s.revoked_at,a.account_ref,a.login_id,a.display_name,a.role, \
         a.state,a.must_change_password,a.revision,a.created_at,a.updated_at,a.disabled_at \
         FROM edge_sessions s JOIN edge_accounts a ON a.account_ref=s.account_ref \
         WHERE s.session_ref=?",
    )
    .bind(session_ref)
    .fetch_one(&mut **tx)
    .await?;
    Ok(StoredSession {
        session_ref: row.try_get("session_ref")?,
        token_digest,
        csrf_digest,
        account: decode_account(&row)?,
        issued_at: row.try_get("issued_at")?,
        last_seen_at: row.try_get("last_seen_at")?,
        idle_expires_at: row.try_get("idle_expires_at")?,
        absolute_expires_at: row.try_get("absolute_expires_at")?,
        revoked_at: row.try_get("revoked_at")?,
    })
}

async fn load_session_postgres(
    tx: &mut Transaction<'_, Postgres>,
    session_ref: &str,
    token_digest: SecretDigest,
    csrf_digest: SecretDigest,
) -> Result<StoredSession, StorageError> {
    let row = sqlx::query(
        "SELECT s.session_ref,s.issued_at,s.last_seen_at,s.idle_expires_at, \
         s.absolute_expires_at,s.revoked_at,a.account_ref,a.login_id,a.display_name,a.role, \
         a.state,a.must_change_password,a.revision,a.created_at,a.updated_at,a.disabled_at \
         FROM edge_sessions s JOIN edge_accounts a ON a.account_ref=s.account_ref \
         WHERE s.session_ref=$1",
    )
    .bind(session_ref)
    .fetch_one(&mut **tx)
    .await?;
    Ok(StoredSession {
        session_ref: row.try_get("session_ref")?,
        token_digest,
        csrf_digest,
        account: decode_account(&row)?,
        issued_at: row.try_get("issued_at")?,
        last_seen_at: row.try_get("last_seen_at")?,
        idle_expires_at: row.try_get("idle_expires_at")?,
        absolute_expires_at: row.try_get("absolute_expires_at")?,
        revoked_at: row.try_get("revoked_at")?,
    })
}

fn decode_audit_sqlite(row: &sqlx::sqlite::SqliteRow) -> Result<AuditEvent, StorageError> {
    let summary: Vec<u8> = row.try_get("summary_json")?;
    Ok(AuditEvent {
        audit_row_id: row.try_get("audit_row_id")?,
        occurred_at: row.try_get("occurred_at")?,
        actor_class: row.try_get("actor_class")?,
        actor_ref: row.try_get("actor_ref")?,
        actor_login_id: row.try_get("actor_login_id")?,
        actor_display_name: row.try_get("actor_display_name")?,
        operation: row.try_get("operation")?,
        resource_ref: row.try_get("resource_ref")?,
        outcome: row.try_get("outcome")?,
        summary: serde_json::from_slice(&summary).map_err(StorageError::EncodeRecord)?,
    })
}

fn decode_audit_postgres(row: &sqlx::postgres::PgRow) -> Result<AuditEvent, StorageError> {
    Ok(AuditEvent {
        audit_row_id: row.try_get("audit_row_id")?,
        occurred_at: row.try_get("occurred_at")?,
        actor_class: row.try_get("actor_class")?,
        actor_ref: row.try_get("actor_ref")?,
        actor_login_id: row.try_get("actor_login_id")?,
        actor_display_name: row.try_get("actor_display_name")?,
        operation: row.try_get("operation")?,
        resource_ref: row.try_get("resource_ref")?,
        outcome: row.try_get("outcome")?,
        summary: row.try_get("summary_json")?,
    })
}
