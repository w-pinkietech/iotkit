use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::OpsError;
use crate::clock::{ClockTrust, ClockTrustError};
use crate::tier::{Actor, ActorKind, Tier, TokenKind};

const TOKEN_RANDOM_BYTES: usize = 32;
const TOKEN_ID_RANDOM_BYTES: usize = 16;
const PASSPHRASE_SALT_BYTES: usize = 16;
const TOKEN_PREFIX: &str = "iko_";
const TOKEN_ID_PREFIX: &str = "tok_";
const LAST_USED_UPDATE_INTERVAL_MS: i64 = 60_000;

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedToken {
    pub token_id: String,
    pub plaintext: Secret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOperatorToken {
    pub name: String,
    pub kind: TokenKind,
    pub ceiling: Tier,
    pub is_session: bool,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRow {
    pub token_id: String,
    pub name: String,
    pub kind: TokenKind,
    pub tier_ceiling: Tier,
    pub is_session: bool,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipState {
    Unowned,
    LocalRecoveryRequired,
    Owned,
}

pub fn ownership_state(conn: &Connection) -> Result<OwnershipState, OpsError> {
    let (recovery_required, ownership_ever_established): (bool, bool) = conn.query_row(
        "SELECT recovery_required, ownership_ever_established FROM auth_state WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let credential = load_passphrase_hash(conn)?;
    if recovery_required {
        return Ok(OwnershipState::LocalRecoveryRequired);
    }
    match credential {
        None if ownership_ever_established => Ok(OwnershipState::LocalRecoveryRequired),
        None => Ok(OwnershipState::Unowned),
        Some(hash) if PasswordHash::new(&hash).is_ok() => Ok(OwnershipState::Owned),
        Some(_) => Ok(OwnershipState::LocalRecoveryRequired),
    }
}

pub fn database_initialization_marker_path(db_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.initialized", db_path.display()))
}

pub fn reconcile_database_initialization_provenance(
    conn: &Connection,
    db_path: &Path,
    database_existed_before_open: bool,
) -> Result<(), OpsError> {
    let marker = database_initialization_marker_path(db_path);
    let marker_existed = marker.exists();
    if marker_existed && !database_existed_before_open {
        conn.execute(
            "UPDATE auth_state
             SET recovery_required = 1,
                 ownership_ever_established = 1
             WHERE id = 1",
            [],
        )?;
        record_auth_event(
            conn,
            "database_reinitialized_after_loss",
            json!({ "actor": "local_startup" }),
        )?;
    }
    if !marker_existed {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)?;
        file.write_all(b"iotkit-database-initialized-v1\n")?;
        file.sync_all()?;
        let parent = marker
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Resets the admin passphrase, creating it first if needed.
///
/// The caller must hash before the SQLite write transaction. This local-maintenance boundary owns the
/// Immediate transaction that updates the credential, generation, revocations, and audit.
pub fn reset_passphrase_with_hash(
    conn: &Connection,
    phc: &str,
    audit_actor: &str,
) -> Result<(), OpsError> {
    PasswordHash::new(phc).map_err(|_| OpsError::CredentialHash)?;
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let now = now_ms();
    tx.execute(
        "INSERT INTO admin_credential (id, passphrase_hash, set_at, updated_at)
         VALUES (1, ?1, ?2, ?2)
         ON CONFLICT(id) DO UPDATE SET
           passphrase_hash = excluded.passphrase_hash,
           updated_at = excluded.updated_at",
        params![phc, now],
    )?;
    tx.execute(
        "UPDATE auth_state
         SET auth_generation = auth_generation + 1,
             device_credential_generation = device_credential_generation + 1,
             recovery_required = 0,
             ownership_ever_established = 1
         WHERE id = 1",
        [],
    )?;
    tx.execute(
        "UPDATE operator_tokens SET revoked_at = ?1 WHERE revoked_at IS NULL",
        [now],
    )?;
    record_auth_event(
        &tx,
        "admin_passphrase_reset",
        json!({ "actor": audit_actor }),
    )?;
    tx.commit()?;
    Ok(())
}

pub fn auth_generation(conn: &Connection) -> Result<i64, OpsError> {
    conn.query_row(
        "SELECT auth_generation FROM auth_state WHERE id = 1",
        [],
        |row| row.get(0),
    )
    .map_err(OpsError::from)
}

pub fn auth_epoch(conn: &Connection) -> Result<String, OpsError> {
    conn.query_row(
        "SELECT auth_epoch FROM auth_state WHERE id = 1",
        [],
        |row| row.get(0),
    )
    .map_err(OpsError::from)
}

pub fn new_auth_epoch() -> Result<String, OpsError> {
    random_prefixed("auth_", TOKEN_ID_RANDOM_BYTES)
}

pub fn enter_restored_local_recovery(
    tx: &rusqlite::Transaction<'_>,
    new_epoch: &str,
) -> Result<(), OpsError> {
    tx.execute_batch("PRAGMA defer_foreign_keys = ON")?;
    let prior_device_generation = crate::device_auth_generation(tx)?;
    tx.execute("DELETE FROM admin_credential", [])?;
    tx.execute("DELETE FROM operator_tokens", [])?;
    tx.execute(
        "UPDATE auth_state
         SET auth_generation = auth_generation + 1,
             device_credential_generation = device_credential_generation + 1,
             auth_epoch = ?1,
             recovery_required = 1,
             ownership_ever_established = 1,
             clock_evidence_source = NULL,
             clock_evidence_at_ms = NULL,
             manual_evidence_seq = manual_evidence_seq + 1
         WHERE id = 1",
        [new_epoch],
    )?;
    tx.execute("UPDATE device_credentials SET auth_epoch = ?1", [new_epoch])?;
    // A restored desired enable flag is retained for operator diagnosis, but applied network
    // authority is always fenced. Local recovery alone cannot silently reopen ingress.
    tx.execute(
        "UPDATE ingress_listener_config SET applied_generation=0,
          applied_bind_addr=NULL,applied_interface=NULL,applied_site_local_cidrs=NULL,
          applied_mode=NULL,applied_tls_generation=NULL,applied_tls_fingerprint=NULL,
          last_error='restore_reapply_required',last_action='restore_fenced' WHERE id=1",
        [],
    )?;
    tx.execute(
        "UPDATE auth_state SET device_credential_generation=?1 WHERE id=1",
        [prior_device_generation.saturating_add(1)],
    )?;
    record_auth_event(
        tx,
        "restore_authority_cleared",
        json!({ "actor": "local_cli", "recovery": "local_recovery_required" }),
    )?;
    Ok(())
}

pub fn load_passphrase_hash(conn: &Connection) -> Result<Option<String>, OpsError> {
    conn.query_row(
        "SELECT passphrase_hash FROM admin_credential WHERE id = 1",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(OpsError::from)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassphraseAuthority {
    pub phc: String,
    pub auth_generation: i64,
}

pub fn load_passphrase_authority(
    conn: &Connection,
) -> Result<Option<PassphraseAuthority>, OpsError> {
    conn.query_row(
        "SELECT admin_credential.passphrase_hash, auth_state.auth_generation
         FROM admin_credential CROSS JOIN auth_state
         WHERE admin_credential.id = 1 AND auth_state.id = 1",
        [],
        |row| {
            Ok(PassphraseAuthority {
                phc: row.get(0)?,
                auth_generation: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(OpsError::from)
}

pub fn require_passphrase_authority_unchanged(
    conn: &Connection,
    expected: &PassphraseAuthority,
) -> Result<(), OpsError> {
    let unchanged: bool = conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM admin_credential CROSS JOIN auth_state
           WHERE admin_credential.id = 1
             AND auth_state.id = 1
             AND admin_credential.passphrase_hash = ?1
             AND auth_state.auth_generation = ?2
         )",
        params![expected.phc, expected.auth_generation],
        |row| row.get(0),
    )?;
    if unchanged {
        Ok(())
    } else {
        Err(OpsError::Forbidden)
    }
}

pub fn verify_passphrase(phc: &str, plaintext: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(plaintext.as_bytes(), &parsed)
        .is_ok()
}

/// Issues an operator token or auth session token.
///
/// Mutation and audit INSERT atomicity is guaranteed by the caller's transaction:
/// dispatch (Task 3) uses one Immediate Tx, while session/setup routes (Task 6)
/// and gatewayctl (Task 9) create a Tx inside `with_conn`. This function does not
/// open its own transaction, so it does not nest with the caller's Tx.
pub fn issue_token(
    conn: &Connection,
    token: &NewOperatorToken,
    audit_actor: &str,
    audit_source: Option<&str>,
    clock_trust: Option<&ClockTrust>,
) -> Result<IssuedToken, OpsError> {
    if token.kind == TokenKind::Ai && token.ceiling > Tier::Routine {
        return Err(OpsError::Validation(
            "ai token tier ceiling cannot exceed routine".to_string(),
        ));
    }

    let now = if token.expires_at.is_some() {
        clock_trust
            .ok_or(OpsError::ClockUntrusted)?
            .trusted_now_and_advance(conn)
            .map_err(clock_error)?
    } else {
        now_ms()
    };
    issue_token_at(conn, token, audit_actor, audit_source, now)
}

pub fn issue_session_token(
    conn: &Connection,
    name: &str,
    ceiling: Tier,
    ttl_ms: i64,
    audit_actor: &str,
    audit_source: Option<&str>,
    clock_trust: &ClockTrust,
) -> Result<IssuedToken, OpsError> {
    if ttl_ms <= 0 {
        return Err(OpsError::Validation("session TTL must be positive".into()));
    }
    let now = clock_trust
        .trusted_now_and_advance(conn)
        .map_err(clock_error)?;
    let expires_at = now
        .checked_add(ttl_ms)
        .ok_or_else(|| OpsError::Validation("session expiry overflow".into()))?;
    let token = NewOperatorToken {
        name: name.to_string(),
        kind: TokenKind::Human,
        ceiling,
        is_session: true,
        expires_at: Some(expires_at),
    };
    issue_token_at(conn, &token, audit_actor, audit_source, now)
}

fn issue_token_at(
    conn: &Connection,
    token: &NewOperatorToken,
    audit_actor: &str,
    audit_source: Option<&str>,
    now: i64,
) -> Result<IssuedToken, OpsError> {
    let generation = auth_generation(conn)?;
    let token_id = random_prefixed(TOKEN_ID_PREFIX, TOKEN_ID_RANDOM_BYTES)?;
    let plaintext = random_prefixed(TOKEN_PREFIX, TOKEN_RANDOM_BYTES)?;
    let token_hash = hash_token(&plaintext);
    conn.execute(
        "INSERT INTO operator_tokens (
           token_id, name, token_hash, kind, tier_ceiling, is_session,
           created_at, expires_at, auth_generation
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            token_id,
            token.name.as_str(),
            token_hash,
            token.kind.as_str(),
            token.ceiling.as_str(),
            if token.is_session { 1_i64 } else { 0_i64 },
            now,
            token.expires_at,
            generation,
        ],
    )?;

    let event_kind = if token.is_session {
        "auth_session_issued"
    } else {
        "operator_token_issued"
    };
    record_auth_event(
        conn,
        event_kind,
        json!({
            "token_id": token_id,
            "name": token.name.as_str(),
            "source": audit_source,
            "actor": audit_actor,
        }),
    )?;
    Ok(IssuedToken {
        token_id,
        plaintext: Secret(plaintext),
    })
}

pub fn authenticate(
    conn: &Connection,
    plaintext: &str,
    clock_trust: &ClockTrust,
) -> Result<Option<Actor>, OpsError> {
    let token_hash = hash_token(plaintext);
    let row = conn
        .query_row(
            "SELECT token_id, kind, tier_ceiling, expires_at, revoked_at, last_used_at,
                    auth_generation
             FROM operator_tokens WHERE token_hash = ?1",
            params![token_hash],
            |row| {
                Ok(TokenAuthRow {
                    token_id: row.get(0)?,
                    kind: row.get(1)?,
                    tier_ceiling: row.get(2)?,
                    expires_at: row.get(3)?,
                    revoked_at: row.get(4)?,
                    last_used_at: row.get(5)?,
                    auth_generation: row.get(6)?,
                })
            },
        )
        .optional()?;

    let Some(row) = row else {
        return Ok(None);
    };
    if row.revoked_at.is_some() || row.auth_generation != auth_generation(conn)? {
        return Ok(None);
    }
    let observed_now = if let Some(expires) = row.expires_at {
        let now = clock_trust
            .trusted_now_and_advance(conn)
            .map_err(clock_error)?;
        if expires <= now {
            return Ok(None);
        }
        now
    } else {
        clock_trust.wall_time_ms()
    };
    if row
        .last_used_at
        .is_none_or(|last| observed_now.saturating_sub(last) > LAST_USED_UPDATE_INTERVAL_MS)
    {
        conn.execute(
            "UPDATE operator_tokens SET last_used_at = ?1 WHERE token_id = ?2",
            params![observed_now, row.token_id],
        )?;
    }

    let kind = match TokenKind::parse(&row.kind)? {
        TokenKind::Human => ActorKind::Human,
        TokenKind::Ai => ActorKind::Ai,
    };
    Ok(Some(Actor {
        actor_id: row.token_id,
        actor_kind: kind,
        tier_ceiling: Tier::parse(&row.tier_ceiling)?,
    }))
}

/// Revokes an operator token.
///
/// Mutation and audit INSERT atomicity is guaranteed by the caller's transaction:
/// dispatch (Task 3) uses one Immediate Tx, while session/setup routes (Task 6)
/// and gatewayctl (Task 9) create a Tx inside `with_conn`. This function does not
/// open its own transaction, so it does not nest with the caller's Tx.
pub fn revoke_token(conn: &Connection, token_id: &str, audit_actor: &str) -> Result<(), OpsError> {
    let changed = conn.execute(
        "UPDATE operator_tokens SET revoked_at = ?1 WHERE token_id = ?2",
        params![now_ms(), token_id],
    )?;
    if changed == 0 {
        return Err(OpsError::NotFound);
    }
    record_auth_event(
        conn,
        "operator_token_revoked",
        json!({ "token_id": token_id, "actor": audit_actor }),
    )?;
    Ok(())
}

pub fn list_tokens(conn: &Connection) -> Result<Vec<TokenRow>, OpsError> {
    let mut stmt = conn.prepare(
        "SELECT token_id, name, kind, tier_ceiling, is_session,
                created_at, expires_at, revoked_at, last_used_at
         FROM operator_tokens
         ORDER BY created_at, token_id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<i64>>(8)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    rows.into_iter()
        .map(
            |(
                token_id,
                name,
                kind,
                tier_ceiling,
                is_session,
                created_at,
                expires_at,
                revoked_at,
                last_used_at,
            )| {
                Ok(TokenRow {
                    token_id,
                    name,
                    kind: TokenKind::parse(&kind)?,
                    tier_ceiling: Tier::parse(&tier_ceiling)?,
                    is_session: is_session != 0,
                    created_at,
                    expires_at,
                    revoked_at,
                    last_used_at,
                })
            },
        )
        .collect()
}

struct TokenAuthRow {
    token_id: String,
    kind: String,
    tier_ceiling: String,
    expires_at: Option<i64>,
    revoked_at: Option<i64>,
    last_used_at: Option<i64>,
    auth_generation: i64,
}

fn clock_error(error: ClockTrustError) -> OpsError {
    match error {
        ClockTrustError::Untrusted => OpsError::ClockUntrusted,
        ClockTrustError::Ops(error) => error,
    }
}

pub fn hash_passphrase(plaintext: &str) -> Result<String, OpsError> {
    let mut salt_bytes = [0_u8; PASSPHRASE_SALT_BYTES];
    getrandom::fill(&mut salt_bytes).map_err(|_| OpsError::Random)?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| OpsError::Random)?;
    Argon2::default()
        .hash_password(plaintext.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| OpsError::CredentialHash)
}

fn random_prefixed(prefix: &str, random_bytes: usize) -> Result<String, OpsError> {
    let mut bytes = vec![0_u8; random_bytes];
    getrandom::fill(&mut bytes).map_err(|_| OpsError::Random)?;
    Ok(format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes)))
}

fn hash_token(plaintext: &str) -> Vec<u8> {
    Sha256::digest(plaintext.as_bytes()).to_vec()
}

fn record_auth_event(
    conn: &Connection,
    kind: &str,
    detail: serde_json::Value,
) -> Result<(), OpsError> {
    iotkit_core_ledger::record_event(conn, kind, None, &detail.to_string())?;
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
    use std::time::Duration;

    use rusqlite::{OptionalExtension, params};

    use super::*;
    use crate::tier::{ActorKind, Tier, TokenKind};
    use iotkit_core_storage::Migration;

    #[derive(Default)]
    struct TestClock {
        wall: AtomicI64,
        monotonic: AtomicU64,
        synchronized: AtomicBool,
    }

    impl TestClock {
        fn set_wall(&self, value: i64) {
            self.wall.store(value, Ordering::SeqCst);
        }
    }

    impl crate::Clock for TestClock {
        fn wall_time_ms(&self) -> i64 {
            self.wall.load(Ordering::SeqCst)
        }

        fn monotonic_ms(&self) -> u64 {
            self.monotonic.load(Ordering::SeqCst)
        }

        fn kernel_synchronized(&self) -> bool {
            self.synchronized.load(Ordering::SeqCst)
        }
    }

    fn test_clock(conn: &Connection, now: i64) -> (Arc<TestClock>, ClockTrust) {
        let clock = Arc::new(TestClock::default());
        clock.set_wall(now);
        clock.synchronized.store(true, Ordering::SeqCst);
        let trust = ClockTrust::load(
            conn,
            clock.clone(),
            Duration::from_millis(10),
            Duration::from_secs(60),
        )
        .unwrap();
        (clock, trust)
    }

    fn all_migrations() -> Vec<Migration> {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
        all.extend_from_slice(crate::MIGRATIONS);
        all.sort_by_key(|m| m.version);
        all
    }

    fn event_details(conn: &rusqlite::Connection, kind: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT detail FROM ledger_events WHERE kind = ?1 ORDER BY event_id")
            .unwrap();
        stmt.query_map([kind], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn token_last_used(conn: &rusqlite::Connection, token_id: &str) -> Option<i64> {
        conn.query_row(
            "SELECT last_used_at FROM operator_tokens WHERE token_id = ?1",
            [token_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn local_passphrase_reset_establishes_ownership_and_audit_does_not_expose_secret() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            assert_eq!(ownership_state(conn).unwrap(), OwnershipState::Unowned);
            let phc = hash_passphrase("correct horse battery staple").unwrap();
            reset_passphrase_with_hash(conn, &phc, "local_cli").unwrap();
            assert_eq!(ownership_state(conn).unwrap(), OwnershipState::Owned);

            let stored = load_passphrase_hash(conn).unwrap().unwrap();
            assert_eq!(stored, phc);
            assert!(verify_passphrase(&stored, "correct horse battery staple"));
            assert!(!verify_passphrase(&stored, "wrong passphrase"));

            let new_hash = hash_passphrase("new passphrase").unwrap();
            reset_passphrase_with_hash(conn, &new_hash, "local_cli").unwrap();
            let updated = load_passphrase_hash(conn).unwrap().unwrap();
            assert!(verify_passphrase(&updated, "new passphrase"));
            assert!(!verify_passphrase(&updated, "correct horse battery staple"));

            let reset_details = event_details(conn, "admin_passphrase_reset");
            assert_eq!(reset_details.len(), 2);
            let reset_detail: serde_json::Value = serde_json::from_str(&reset_details[1]).unwrap();
            assert_eq!(reset_detail["actor"], "local_cli");
            for detail in reset_details {
                assert!(!detail.contains("new passphrase"));
                assert!(!detail.contains("correct horse battery staple"));
                assert!(!detail.contains("passphrase_hash"));
                assert!(!detail.contains("$argon2"));
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn passphrase_reset_revokes_all_operator_and_session_authority() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let (_clock, trust) = test_clock(conn, 1_000);
            let operator = issue_token(
                conn,
                &NewOperatorToken {
                    name: "operator".to_string(),
                    kind: TokenKind::Human,
                    ceiling: Tier::Daily,
                    is_session: false,
                    expires_at: None,
                },
                "local_cli",
                None,
                None,
            )
            .unwrap();
            let session = issue_token(
                conn,
                &NewOperatorToken {
                    name: "session".to_string(),
                    kind: TokenKind::Human,
                    ceiling: Tier::Construction,
                    is_session: true,
                    expires_at: Some(10_000),
                },
                "local_cli",
                None,
                Some(&trust),
            )
            .unwrap();

            let hash = hash_passphrase("replacement passphrase").unwrap();
            reset_passphrase_with_hash(conn, &hash, "local_cli").unwrap();

            assert!(
                authenticate(conn, operator.plaintext.expose(), &trust)
                    .unwrap()
                    .is_none()
            );
            assert!(
                authenticate(conn, session.plaintext.expose(), &trust)
                    .unwrap()
                    .is_none()
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn passphrase_reset_audit_failure_preserves_prior_authority_atomically() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let (_clock, trust) = test_clock(conn, 1_000);
            let original_hash = hash_passphrase("original passphrase").unwrap();
            reset_passphrase_with_hash(conn, &original_hash, "local_cli").unwrap();
            let issued = issue_token(
                conn,
                &NewOperatorToken {
                    name: "surviving operator".into(),
                    kind: TokenKind::Human,
                    ceiling: Tier::Routine,
                    is_session: false,
                    expires_at: None,
                },
                "local_cli",
                None,
                None,
            )
            .unwrap();
            let generation = auth_generation(conn).unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_reset_audit BEFORE INSERT ON ledger_events
                 WHEN NEW.kind = 'admin_passphrase_reset'
                 BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;",
            )
            .unwrap();

            let replacement_hash = hash_passphrase("replacement passphrase").unwrap();
            assert!(reset_passphrase_with_hash(conn, &replacement_hash, "local_cli").is_err());

            assert_eq!(load_passphrase_hash(conn).unwrap().unwrap(), original_hash);
            assert_eq!(auth_generation(conn).unwrap(), generation);
            assert!(
                authenticate(conn, issued.plaintext.expose(), &trust)
                    .unwrap()
                    .is_some()
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn concurrent_passphrase_resets_serialize_and_leave_one_committed_credential() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let db = iotkit_core_storage::init_db(&path, &all_migrations()).unwrap();
        let first = hash_passphrase("first concurrent passphrase").unwrap();
        let second = hash_passphrase("second concurrent passphrase").unwrap();
        let mut workers = Vec::new();
        for hash in [first.clone(), second.clone()] {
            let path = path.clone();
            workers.push(std::thread::spawn(move || {
                let conn = Connection::open(path).unwrap();
                conn.busy_timeout(Duration::from_secs(5)).unwrap();
                reset_passphrase_with_hash(&conn, &hash, "local_cli").unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        db.with_conn_sync(|conn| {
            let committed = load_passphrase_hash(conn).unwrap().unwrap();
            assert!(committed == first || committed == second);
            assert_eq!(auth_generation(conn).unwrap(), 2);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn token_issue_authenticate_expire_revoke_and_audit_do_not_expose_secret() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let (clock, trust) = test_clock(conn, 1_000);
            let issued = issue_token(
                conn,
                &NewOperatorToken {
                    name: "daily human".to_string(),
                    kind: TokenKind::Human,
                    ceiling: Tier::Daily,
                    is_session: false,
                    expires_at: None,
                },
                "local_cli",
                Some("127.0.0.1"),
                None,
            )
            .unwrap();
            assert_eq!(format!("{:?}", issued.plaintext), "[REDACTED]");
            assert_eq!(issued.token_id.len(), 26);
            assert!(issued.token_id.starts_with("tok_"));
            assert_eq!(issued.plaintext.expose().len(), 47);
            assert!(issued.plaintext.expose().starts_with("iko_"));

            let actor = authenticate(conn, issued.plaintext.expose(), &trust)
                .unwrap()
                .unwrap();
            assert_eq!(actor.actor_id, issued.token_id);
            assert_eq!(actor.actor_kind, ActorKind::Human);
            assert_eq!(actor.tier_ceiling, Tier::Daily);

            let expired = issue_token(
                conn,
                &NewOperatorToken {
                    name: "expired".to_string(),
                    kind: TokenKind::Human,
                    ceiling: Tier::Routine,
                    is_session: true,
                    expires_at: Some(5_000),
                },
                "local_cli",
                None,
                Some(&trust),
            )
            .unwrap();
            clock.set_wall(5_001);
            assert!(
                authenticate(conn, expired.plaintext.expose(), &trust)
                    .unwrap()
                    .is_none()
            );

            revoke_token(conn, &issued.token_id, "local_cli").unwrap();
            assert!(
                authenticate(conn, issued.plaintext.expose(), &trust)
                    .unwrap()
                    .is_none()
            );

            let issue_details = event_details(conn, "operator_token_issued");
            let session_details = event_details(conn, "auth_session_issued");
            let revoke_details = event_details(conn, "operator_token_revoked");
            assert_eq!(issue_details.len(), 1);
            assert_eq!(session_details.len(), 1);
            assert_eq!(revoke_details.len(), 1);
            assert!(issue_details[0].contains(&issued.token_id));
            assert!(issue_details[0].contains("daily human"));
            assert!(issue_details[0].contains("127.0.0.1"));
            assert!(session_details[0].contains(&expired.token_id));
            assert!(session_details[0].contains("expired"));
            assert!(revoke_details[0].contains(&issued.token_id));
            let issue_detail: serde_json::Value = serde_json::from_str(&issue_details[0]).unwrap();
            let session_detail: serde_json::Value =
                serde_json::from_str(&session_details[0]).unwrap();
            let revoke_detail: serde_json::Value =
                serde_json::from_str(&revoke_details[0]).unwrap();
            assert_eq!(issue_detail["actor"], "local_cli");
            assert_eq!(issue_detail["source"], "127.0.0.1");
            assert_eq!(session_detail["actor"], "local_cli");
            assert!(session_detail["source"].is_null());
            assert_eq!(revoke_detail["actor"], "local_cli");
            for detail in issue_details
                .into_iter()
                .chain(session_details)
                .chain(revoke_details)
            {
                assert!(!detail.contains(issued.plaintext.expose()));
                assert!(!detail.contains(expired.plaintext.expose()));
                assert!(!detail.contains("token_hash"));
                assert!(!detail.contains("hash"));
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn authenticate_rejects_token_expiring_at_now() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let (clock, trust) = test_clock(conn, 4_000);
            let issued = issue_token(
                conn,
                &NewOperatorToken {
                    name: "boundary".to_string(),
                    kind: TokenKind::Human,
                    ceiling: Tier::Routine,
                    is_session: false,
                    expires_at: Some(5_000),
                },
                "local_cli",
                None,
                Some(&trust),
            )
            .unwrap();

            clock.set_wall(5_000);
            assert!(
                authenticate(conn, issued.plaintext.expose(), &trust)
                    .unwrap()
                    .is_none()
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn finite_auth_floor_failure_rejects_without_updating_session_state() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let (clock, trust) = test_clock(conn, 4_000);
            let issued = issue_token(
                conn,
                &NewOperatorToken {
                    name: "finite".into(),
                    kind: TokenKind::Human,
                    ceiling: Tier::Routine,
                    is_session: true,
                    expires_at: Some(10_000),
                },
                "local_cli",
                None,
                Some(&trust),
            )
            .unwrap();
            let floor = ClockTrust::persisted_floor(conn).unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_auth_floor BEFORE UPDATE OF clock_floor_ms ON auth_state
                 WHEN NEW.clock_floor_ms > OLD.clock_floor_ms
                 BEGIN SELECT RAISE(ABORT, 'injected auth floor failure'); END;",
            )
            .unwrap();
            clock.set_wall(5_000);
            let tx =
                rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate).unwrap();
            assert!(authenticate(&tx, issued.plaintext.expose(), &trust).is_err());
            drop(tx);
            assert_eq!(ClockTrust::persisted_floor(conn).unwrap(), floor);
            assert_eq!(token_last_used(conn, &issued.token_id), None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn concurrent_finite_auth_transactions_share_one_clock_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("concurrent-auth.db");
        let db = iotkit_core_storage::init_db(&path, &all_migrations()).unwrap();
        let (plaintext, trust) = db
            .with_conn_sync(|conn| {
                let (_clock, trust) = test_clock(conn, 7_000);
                let trust = Arc::new(trust);
                let issued = issue_session_token(
                    conn,
                    "concurrent session",
                    Tier::Routine,
                    60_000,
                    "local_cli",
                    None,
                    &trust,
                )
                .unwrap();
                Ok((issued.plaintext.expose().to_string(), trust))
            })
            .unwrap();

        let mut workers = Vec::new();
        for _ in 0..8 {
            let path = path.clone();
            let plaintext = plaintext.clone();
            let trust = trust.clone();
            workers.push(std::thread::spawn(move || {
                let conn = Connection::open(path).unwrap();
                conn.busy_timeout(Duration::from_secs(5)).unwrap();
                let tx =
                    rusqlite::Transaction::new_unchecked(&conn, TransactionBehavior::Immediate)
                        .unwrap();
                assert!(authenticate(&tx, &plaintext, &trust).unwrap().is_some());
                tx.commit().unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        db.with_conn_sync(|conn| {
            assert_eq!(ClockTrust::persisted_floor(conn).unwrap(), 7_000);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn ai_tokens_above_routine_are_rejected_before_database_insert() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let result = issue_token(
                conn,
                &NewOperatorToken {
                    name: "ai daily".to_string(),
                    kind: TokenKind::Ai,
                    ceiling: Tier::Daily,
                    is_session: false,
                    expires_at: None,
                },
                "local_cli",
                None,
                None,
            );
            assert!(matches!(result, Err(crate::OpsError::Validation(_))));
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM operator_tokens", [], |row| row.get(0))
                .unwrap();
            let audit_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM ledger_events WHERE kind = 'operator_token_issued'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
            assert_eq!(audit_count, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn authenticate_throttles_last_used_updates_to_sixty_seconds() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let (clock, trust) = test_clock(conn, 1_000);
            let issued = issue_token(
                conn,
                &NewOperatorToken {
                    name: "routine".to_string(),
                    kind: TokenKind::Human,
                    ceiling: Tier::Routine,
                    is_session: false,
                    expires_at: None,
                },
                "local_cli",
                None,
                None,
            )
            .unwrap();

            assert!(
                authenticate(conn, issued.plaintext.expose(), &trust)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(token_last_used(conn, &issued.token_id), Some(1_000));

            assert!(
                authenticate(conn, issued.plaintext.expose(), &trust)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(token_last_used(conn, &issued.token_id), Some(1_000));

            clock.set_wall(61_001);
            assert!(
                authenticate(conn, issued.plaintext.expose(), &trust)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(token_last_used(conn, &issued.token_id), Some(61_001));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn list_tokens_omits_hash_and_plaintext() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let issued = issue_token(
                conn,
                &NewOperatorToken {
                    name: "listed".to_string(),
                    kind: TokenKind::Ai,
                    ceiling: Tier::Routine,
                    is_session: false,
                    expires_at: None,
                },
                "local_cli",
                None,
                None,
            )
            .unwrap();
            let rows = list_tokens(conn).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].token_id, issued.token_id);
            assert_eq!(rows[0].name, "listed");
            assert_eq!(rows[0].kind, TokenKind::Ai);
            assert_eq!(rows[0].tier_ceiling, Tier::Routine);

            let leaked_hash: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT token_hash FROM operator_tokens WHERE token_id = ?1",
                    params![rows[0].token_id],
                    |row| row.get(0),
                )
                .optional()
                .unwrap();
            assert!(
                leaked_hash.is_some(),
                "test must prove the DB row has a hash"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn old_passphrase_verification_cannot_issue_after_concurrent_reset() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let old_hash = hash_passphrase("old-passphrase").unwrap();
            reset_passphrase_with_hash(conn, &old_hash, "local_cli").unwrap();
            let pre_hash_authority = load_passphrase_authority(conn).unwrap().unwrap();
            assert!(verify_passphrase(&pre_hash_authority.phc, "old-passphrase"));

            let new_hash = hash_passphrase("new-passphrase").unwrap();
            reset_passphrase_with_hash(conn, &new_hash, "local_cli").unwrap();

            let tx =
                rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate).unwrap();
            assert!(matches!(
                require_passphrase_authority_unchanged(&tx, &pre_hash_authority),
                Err(OpsError::Forbidden)
            ));
            let sessions: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM operator_tokens WHERE is_session = 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(sessions, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn losing_credential_after_ownership_requires_local_recovery() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let hash = hash_passphrase("owned-passphrase").unwrap();
            reset_passphrase_with_hash(conn, &hash, "local_cli").unwrap();
            conn.execute("DELETE FROM admin_credential", []).unwrap();
            assert_eq!(
                ownership_state(conn).unwrap(),
                OwnershipState::LocalRecoveryRequired
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn external_initialization_marker_distinguishes_first_init_from_database_loss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gateway.db");
        let first = iotkit_core_storage::init_db(&path, &all_migrations()).unwrap();
        first
            .with_conn_sync(|conn| {
                reconcile_database_initialization_provenance(conn, &path, false).unwrap();
                assert_eq!(ownership_state(conn).unwrap(), OwnershipState::Unowned);
                Ok(())
            })
            .unwrap();
        drop(first);
        std::fs::remove_file(&path).unwrap();

        let recreated = iotkit_core_storage::init_db(&path, &all_migrations()).unwrap();
        recreated
            .with_conn_sync(|conn| {
                reconcile_database_initialization_provenance(conn, &path, false).unwrap();
                assert_eq!(
                    ownership_state(conn).unwrap(),
                    OwnershipState::LocalRecoveryRequired
                );
                Ok(())
            })
            .unwrap();
    }
}
