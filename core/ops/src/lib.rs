//! iotkit-core-ops: control-plane operation foundation.

use iotkit_core_storage::Migration;

pub mod auth;
pub mod catalog;
pub mod fingerprint;
pub mod ops;
pub mod tier;

pub use auth::{
    IssuedToken, NewOperatorToken, Secret, SetOutcome, TokenRow, authenticate, is_setup_mode,
    issue_token, list_tokens, load_passphrase_hash, reset_passphrase, revoke_token, set_passphrase,
    verify_passphrase,
};
pub use catalog::{DispatchRequest, OpContext, OpDescriptor, OpError, SETUP_ALLOWED_OPS, dispatch};
pub use fingerprint::fingerprint_of_pem;
pub use ops::standard_catalog;
pub use tier::{Actor, ActorKind, Tier, TokenKind};

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 12,
    label: "ops",
    sql: include_str!("../migrations/0012_ops.sql"),
}];

#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    #[error(transparent)]
    Storage(#[from] iotkit_core_storage::StorageError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Ledger(#[from] iotkit_core_ledger::LedgerError),
    #[error("not found")]
    NotFound,
    #[error("conflict")]
    Conflict,
    #[error("forbidden")]
    Forbidden,
    #[error("validation: {0}")]
    Validation(String),
    #[error("credential hashing failed")]
    CredentialHash,
    #[error("random generation failed")]
    Random,
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::*;

    fn all_migrations() -> Vec<Migration> {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
        all.extend_from_slice(MIGRATIONS);
        all.sort_by_key(|m| m.version);
        all
    }

    #[test]
    fn ai_tokens_cannot_exceed_routine_ceiling() {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();

        db.with_conn_sync(|conn| {
            let err = conn
                .execute(
                    "INSERT INTO operator_tokens (
                        token_id, name, token_hash, kind, tier_ceiling, created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params!["tok_test", "ai daily", vec![0_u8; 32], "ai", "daily", 1_i64],
                )
                .expect_err("ai daily token must fail CHECK constraint");
            let message = err.to_string();
            assert!(
                message.contains("CHECK constraint failed"),
                "expected CHECK constraint failure, got {message}"
            );
            Ok(())
        })
        .unwrap();
    }
}
