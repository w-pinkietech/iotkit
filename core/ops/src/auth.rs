use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::OpsError;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOutcome {
    FirstSet,
    AlreadySet,
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

pub fn is_setup_mode(conn: &Connection) -> Result<bool, OpsError> {
    let exists: i64 = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM admin_credential WHERE id = 1)",
        [],
        |row| row.get(0),
    )?;
    Ok(exists == 0)
}

/// Sets the initial admin passphrase if it has not already been set.
///
/// Mutation and audit INSERT atomicity is guaranteed by the caller's transaction:
/// dispatch (Task 3) uses one Immediate Tx, while session/setup routes (Task 6)
/// and gatewayctl (Task 9) create a Tx inside `with_conn`. This function does not
/// open its own transaction, so it does not nest with the caller's Tx.
pub fn set_passphrase(
    conn: &Connection,
    plaintext: &str,
    audit_actor: &str,
) -> Result<SetOutcome, OpsError> {
    let now = now_ms();
    let hash = hash_passphrase(plaintext)?;
    let changed = conn.execute(
        "INSERT OR IGNORE INTO admin_credential (id, passphrase_hash, set_at, updated_at)
         VALUES (1, ?1, ?2, ?2)",
        params![hash, now],
    )?;
    if changed == 0 {
        return Ok(SetOutcome::AlreadySet);
    }
    record_auth_event(
        conn,
        "admin_passphrase_set",
        json!({ "actor": audit_actor }),
    )?;
    Ok(SetOutcome::FirstSet)
}

/// Resets the admin passphrase, creating it first if needed.
///
/// Mutation and audit INSERT atomicity is guaranteed by the caller's transaction:
/// dispatch (Task 3) uses one Immediate Tx, while session/setup routes (Task 6)
/// and gatewayctl (Task 9) create a Tx inside `with_conn`. This function does not
/// open its own transaction, so it does not nest with the caller's Tx.
pub fn reset_passphrase(
    conn: &Connection,
    plaintext: &str,
    audit_actor: &str,
) -> Result<(), OpsError> {
    let now = now_ms();
    let hash = hash_passphrase(plaintext)?;
    conn.execute(
        "INSERT INTO admin_credential (id, passphrase_hash, set_at, updated_at)
         VALUES (1, ?1, ?2, ?2)
         ON CONFLICT(id) DO UPDATE SET
           passphrase_hash = excluded.passphrase_hash,
           updated_at = excluded.updated_at",
        params![hash, now],
    )?;
    record_auth_event(
        conn,
        "admin_passphrase_reset",
        json!({ "actor": audit_actor }),
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
) -> Result<IssuedToken, OpsError> {
    if token.kind == TokenKind::Ai && token.ceiling > Tier::Routine {
        return Err(OpsError::Validation(
            "ai token tier ceiling cannot exceed routine".to_string(),
        ));
    }

    let now = now_ms();
    let token_id = random_prefixed(TOKEN_ID_PREFIX, TOKEN_ID_RANDOM_BYTES)?;
    let plaintext = random_prefixed(TOKEN_PREFIX, TOKEN_RANDOM_BYTES)?;
    let token_hash = hash_token(&plaintext);
    conn.execute(
        "INSERT INTO operator_tokens (
           token_id, name, token_hash, kind, tier_ceiling, is_session,
           created_at, expires_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            token_id,
            token.name.as_str(),
            token_hash,
            token.kind.as_str(),
            token.ceiling.as_str(),
            if token.is_session { 1_i64 } else { 0_i64 },
            now,
            token.expires_at
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
    now_ms: i64,
) -> Result<Option<Actor>, OpsError> {
    let token_hash = hash_token(plaintext);
    let row = conn
        .query_row(
            "SELECT token_id, kind, tier_ceiling, expires_at, revoked_at, last_used_at
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
                })
            },
        )
        .optional()?;

    let Some(row) = row else {
        return Ok(None);
    };
    if row.revoked_at.is_some() || row.expires_at.is_some_and(|expires| expires <= now_ms) {
        return Ok(None);
    }
    if row
        .last_used_at
        .is_none_or(|last| now_ms - last > LAST_USED_UPDATE_INTERVAL_MS)
    {
        conn.execute(
            "UPDATE operator_tokens SET last_used_at = ?1 WHERE token_id = ?2",
            params![now_ms, row.token_id],
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
}

fn hash_passphrase(plaintext: &str) -> Result<String, OpsError> {
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
    use rusqlite::{OptionalExtension, params};

    use super::*;
    use crate::tier::{ActorKind, Tier, TokenKind};
    use iotkit_core_storage::Migration;

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
    fn passphrase_setup_set_verify_reset_and_audit_do_not_expose_secret() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            assert!(is_setup_mode(conn).unwrap());

            assert_eq!(
                set_passphrase(conn, "correct horse battery staple", "setup_mode").unwrap(),
                SetOutcome::FirstSet
            );
            assert!(!is_setup_mode(conn).unwrap());
            assert_eq!(
                set_passphrase(conn, "losing concurrent writer", "local_cli").unwrap(),
                SetOutcome::AlreadySet
            );

            let phc = load_passphrase_hash(conn).unwrap().unwrap();
            assert!(verify_passphrase(&phc, "correct horse battery staple"));
            assert!(!verify_passphrase(&phc, "wrong passphrase"));

            reset_passphrase(conn, "new passphrase", "local_cli").unwrap();
            let updated = load_passphrase_hash(conn).unwrap().unwrap();
            assert!(verify_passphrase(&updated, "new passphrase"));
            assert!(!verify_passphrase(&updated, "correct horse battery staple"));

            let set_details = event_details(conn, "admin_passphrase_set");
            let reset_details = event_details(conn, "admin_passphrase_reset");
            assert_eq!(set_details.len(), 1);
            assert_eq!(reset_details.len(), 1);
            let set_detail: serde_json::Value = serde_json::from_str(&set_details[0]).unwrap();
            let reset_detail: serde_json::Value = serde_json::from_str(&reset_details[0]).unwrap();
            assert_eq!(set_detail["actor"], "setup_mode");
            assert_eq!(reset_detail["actor"], "local_cli");
            for detail in set_details.into_iter().chain(reset_details) {
                assert!(!detail.contains("correct horse battery staple"));
                assert!(!detail.contains("new passphrase"));
                assert!(!detail.contains("passphrase_hash"));
                assert!(!detail.contains("$argon2"));
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn token_issue_authenticate_expire_revoke_and_audit_do_not_expose_secret() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
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
            )
            .unwrap();
            assert_eq!(format!("{:?}", issued.plaintext), "[REDACTED]");
            assert_eq!(issued.token_id.len(), 26);
            assert!(issued.token_id.starts_with("tok_"));
            assert_eq!(issued.plaintext.expose().len(), 47);
            assert!(issued.plaintext.expose().starts_with("iko_"));

            let actor = authenticate(conn, issued.plaintext.expose(), 1_000)
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
                "setup_mode",
                None,
            )
            .unwrap();
            assert!(
                authenticate(conn, expired.plaintext.expose(), 5_001)
                    .unwrap()
                    .is_none()
            );

            revoke_token(conn, &issued.token_id, "local_cli").unwrap();
            assert!(
                authenticate(conn, issued.plaintext.expose(), 2_000)
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
            assert_eq!(session_detail["actor"], "setup_mode");
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
            )
            .unwrap();

            assert!(
                authenticate(conn, issued.plaintext.expose(), 5_000)
                    .unwrap()
                    .is_none()
            );
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
            )
            .unwrap();

            assert!(
                authenticate(conn, issued.plaintext.expose(), 1_000)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(token_last_used(conn, &issued.token_id), Some(1_000));

            assert!(
                authenticate(conn, issued.plaintext.expose(), 2_000)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(token_last_used(conn, &issued.token_id), Some(1_000));

            assert!(
                authenticate(conn, issued.plaintext.expose(), 61_001)
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
}
