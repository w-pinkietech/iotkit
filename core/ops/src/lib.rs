//! iotkit-core-ops: control-plane operation foundation.

use iotkit_core_storage::Migration;

pub mod auth;
pub mod catalog;
pub mod clock;
pub mod fingerprint;
pub mod ops;
pub mod tier;

pub use auth::{
    IssuedToken, NewOperatorToken, OwnershipState, PassphraseAuthority, Secret, TokenRow,
    auth_epoch, auth_generation, authenticate, database_initialization_marker_path,
    enter_restored_local_recovery, hash_passphrase, issue_session_token, issue_token, list_tokens,
    load_passphrase_authority, load_passphrase_hash, new_auth_epoch, ownership_state,
    reconcile_database_initialization_provenance, require_passphrase_authority_unchanged,
    reset_passphrase_with_hash, revoke_token, verify_passphrase,
};
pub use catalog::{DispatchRequest, OpContext, OpDescriptor, OpError, dispatch};
pub use clock::{
    Clock, ClockEvidence, ClockTrust, ClockTrustError, SystemClock, TrustSource,
    confirm_time_with_clock,
};
pub use fingerprint::fingerprint_of_pem;
pub use ops::standard_catalog;
pub use tier::{Actor, ActorKind, Tier, TokenKind};

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 12,
        label: "ops",
        sql: include_str!("../migrations/0012_ops.sql"),
    },
    Migration {
        version: 13,
        label: "auth_authority",
        sql: include_str!("../migrations/0013_auth_authority.sql"),
    },
    Migration {
        version: 14,
        label: "task1_provenance",
        sql: include_str!("../migrations/0014_task1_provenance.sql"),
    },
];

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
    #[error("trusted wall clock is required")]
    ClockUntrusted,
    #[error(transparent)]
    Io(#[from] std::io::Error),
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

    #[test]
    fn ownership_provenance_migration_seeds_existing_admin_compatibly() {
        let mut through_auth_authority = all_migrations();
        through_auth_authority.retain(|migration| migration.version <= 13);
        let db = iotkit_core_storage::init_db_memory(&through_auth_authority).unwrap();
        db.with_conn_sync(|conn| {
            let hash = hash_passphrase("existing-admin-passphrase").unwrap();
            conn.execute(
                "INSERT INTO admin_credential (id, passphrase_hash, set_at, updated_at)
                 VALUES (1, ?1, 1, 1)",
                [&hash],
            )?;
            iotkit_core_storage::run_migrations(conn, &all_migrations())?;
            assert_eq!(ownership_state(conn).unwrap(), OwnershipState::Owned);
            conn.execute("DELETE FROM admin_credential", [])?;
            assert_eq!(
                ownership_state(conn).unwrap(),
                OwnershipState::LocalRecoveryRequired
            );
            Ok(())
        })
        .unwrap();
    }
}
