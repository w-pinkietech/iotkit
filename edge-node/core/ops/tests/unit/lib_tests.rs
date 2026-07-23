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
