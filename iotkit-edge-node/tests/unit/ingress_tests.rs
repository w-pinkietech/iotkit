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
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
    })
    .unwrap();
    let expected = db
        .with_conn(|conn| {
            iotkit_core_ops::load_ingress_listener_config(conn).map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
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

#[tokio::test]
async fn throttle_episode_events_persist_without_identity_or_payload_text() {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    migrations.sort_by_key(|migration| migration.version);
    let db = iotkit_core_storage::init_db_memory(&migrations).unwrap();
    persist_throttle_episode_events(
        &db,
        vec![
            iotkit_ingest_http::ThrottleEpisodeEvent::Started { episode_id: 7 },
            iotkit_ingest_http::ThrottleEpisodeEvent::Recovered {
                episode_id: 7,
                drops: u64::MAX,
            },
        ],
    )
    .await;
    db.with_conn_sync(|conn| {
        let events = iotkit_core_ledger::list_recent_events(conn, 10).unwrap();
        assert_eq!(events.len(), 2);
        let rendered = events
            .iter()
            .map(|event| event.detail.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains(&u64::MAX.to_string()));
        assert!(!rendered.contains("principal"));
        assert!(!rendered.contains("token"));
        assert!(!rendered.contains("source"));
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn throttle_episode_persistence_failure_retries_without_duplicate_records() {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    migrations.sort_by_key(|migration| migration.version);
    let db = iotkit_core_storage::init_db_memory(&migrations).unwrap();
    let events = vec![
        iotkit_ingest_http::ThrottleEpisodeEvent::Started { episode_id: 9 },
        iotkit_ingest_http::ThrottleEpisodeEvent::Recovered {
            episode_id: 9,
            drops: 42,
        },
    ];
    db.with_conn_sync(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER fail_throttle_audit BEFORE INSERT ON ledger_events
                 WHEN new.kind IN ('ingress_throttle_started','ingress_throttle_recovered')
                 BEGIN SELECT RAISE(FAIL, 'injected throttle audit failure'); END;",
        )?;
        Ok(())
    })
    .unwrap();
    assert!(!persist_throttle_episode_events(&db, events.clone()).await);
    db.with_conn_sync(|conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ledger_events WHERE kind LIKE 'ingress_throttle_%'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 0);
        conn.execute_batch("DROP TRIGGER fail_throttle_audit")?;
        Ok(())
    })
    .unwrap();
    assert!(persist_throttle_episode_events(&db, events).await);
    db.with_conn_sync(|conn| {
        let counts: (i64, i64) = conn.query_row(
            "SELECT
                    SUM(kind='ingress_throttle_started'),
                    SUM(kind='ingress_throttle_recovered')
                 FROM ledger_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(counts, (1, 1));
        Ok(())
    })
    .unwrap();
}
