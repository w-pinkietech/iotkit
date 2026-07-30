use rusqlite::params;

use super::*;

const RECOVERY_ID: &str = "recovery-0123456789abcdef0123456789abcdef";
const EDGE_ID: &str = "edge-0123456789abcdef0123456789abcdef";
const EDGE_NODE_ID: &str = "edge-node-01";
const CANDIDATE_ID: &str = "candidate-0123456789abcdef0123456789abcdef";
const BACKUP_ID: &str = "backup-0123456789abcdef0123456789abcdef";
const OLD_EPOCH: &str = "01JOLDLEDGEREPOCH";
const NEW_EPOCH: &str = "01JNEWLEDGEREPOCH";

fn candidate_database() -> rusqlite::Connection {
    let conn = crate::tests_support::complete_database();
    conn.execute(
        "INSERT INTO ledger_meta(key,value) VALUES
             ('edge_node_id',?1),('epoch',?2),('generation','1')",
        params![EDGE_NODE_ID, OLD_EPOCH],
    )
    .unwrap();
    conn.execute(
        "UPDATE auth_state SET device_credential_generation=4 WHERE id=1",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO target_registry(
             target_id,endpoint_url,credential_token,archive_responsible,
             schema_version,cursor_epoch,cursor_pub_seq,created_at
         ) VALUES('edge','mqtts://broker.example.test:8883','',1,1,?1,40,1)",
        [OLD_EPOCH],
    )
    .unwrap();
    for pub_seq in 41..=50 {
        conn.execute(
            "INSERT INTO publication_log(
                 pub_seq,epoch,kind,reading_seq,created_at
             ) VALUES(?1,?2,'measurement',?1,1)",
            params![pub_seq, OLD_EPOCH],
        )
        .unwrap();
    }
    conn.execute(
        "UPDATE edge_node_activation
         SET state='active',edge_id=?1,
             activation_id='act-0123456789abcdef0123456789abcdef',
             ledger_epoch=?2,discard_through_reading_seq=0,
             cleanup_through_reading_seq=0,request_json='{}',
             result_json='{}',activated_at=1
         WHERE singleton=1",
        params![EDGE_ID, OLD_EPOCH],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO edge_node_recovery_candidate(
             singleton,state,recovery_id,candidate_instance_id,backup_id,
             source_database_length,source_database_sha256,artifact_length,
             artifact_sha256,edge_id,edge_node_id,old_ledger_epoch,
             proposed_new_epoch,credential_generation,handoff_schema_version,
             installed_at_ms
         ) VALUES(
             1,'durably_fenced_candidate',?1,?2,?3,
             1,'0000000000000000000000000000000000000000000000000000000000000000',
             1,'1111111111111111111111111111111111111111111111111111111111111111',
             ?4,?5,?6,?7,2,1,1
         )",
        params![
            RECOVERY_ID,
            CANDIDATE_ID,
            BACKUP_ID,
            EDGE_ID,
            EDGE_NODE_ID,
            OLD_EPOCH,
            NEW_EPOCH
        ],
    )
    .unwrap();
    conn
}

fn epoch_start_candidate(with_measurement: bool) -> rusqlite::Connection {
    let conn = candidate_database();
    conn.execute("DELETE FROM publication_log", []).unwrap();
    conn.execute(
        "DELETE FROM sqlite_sequence WHERE name='publication_log'",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO publication_log(
             epoch,kind,subtype,annotation_json,created_at
         ) VALUES(?1,'annotation','epoch_start',?2,1)",
        params![OLD_EPOCH, format!(r#"{{"prior_epoch":"{OLD_EPOCH}"}}"#)],
    )
    .unwrap();
    if with_measurement {
        conn.execute(
            "INSERT INTO publication_log(
                 epoch,kind,reading_seq,created_at
             ) VALUES(?1,'measurement',2,1)",
            [OLD_EPOCH],
        )
        .unwrap();
    }
    conn.execute(
        "UPDATE target_registry SET cursor_pub_seq=0 WHERE target_id='edge'",
        [],
    )
    .unwrap();
    conn
}

fn request() -> RecoveryActivationRequest {
    RecoveryActivationRequest {
        schema_version: 1,
        recovery_id: RECOVERY_ID.into(),
        edge_id: EDGE_ID.into(),
        edge_node_id: EDGE_NODE_ID.into(),
        candidate_instance_id: CANDIDATE_ID.into(),
        backup_id: BACKUP_ID.into(),
        old_ledger_epoch: OLD_EPOCH.into(),
        new_ledger_epoch: NEW_EPOCH.into(),
        broker_credential_generation: 2,
        device_auth_generation: 4,
        snapshot_accepted_through: 40,
        snapshot_allocation_high_water: 50,
        snapshot_epoch_start_publication_seq: None,
        edge_accepted_through: 45,
        grant_revision: 1,
        issued_at: 10,
    }
}

fn completion() -> RecoveryCompletion {
    RecoveryCompletion {
        schema_version: 1,
        recovery_id: RECOVERY_ID.into(),
        edge_id: EDGE_ID.into(),
        edge_node_id: EDGE_NODE_ID.into(),
        candidate_instance_id: CANDIDATE_ID.into(),
        new_ledger_epoch: NEW_EPOCH.into(),
        status: "committed".into(),
        accepted_through: 0,
        committed_at: 12,
    }
}

#[test]
fn applying_and_completing_recovery_is_idempotent_and_keeps_runtime_fenced_until_completion() {
    let conn = candidate_database();
    let request = request();

    let result = apply_recovery_activation(&conn, &request, 11).unwrap();
    assert_eq!(result.replayed_records, 5);
    assert_eq!(result.last_new_publication_seq, 6);
    assert_eq!(
        apply_recovery_activation(&conn, &request, 999).unwrap(),
        result
    );
    assert_eq!(
        startup_mode(&conn).unwrap(),
        RecoveryStartupMode::AwaitingCompletion {
            recovery_id: RECOVERY_ID.into(),
            candidate_instance_id: CANDIDATE_ID.into(),
            new_ledger_epoch: NEW_EPOCH.into(),
        }
    );

    complete_recovery_activation(&conn, &completion(), 3).unwrap();
    complete_recovery_activation(&conn, &completion(), 3).unwrap();
    assert_eq!(
        startup_mode(&conn).unwrap(),
        RecoveryStartupMode::Recovered {
            recovery_id: RECOVERY_ID.into(),
            candidate_instance_id: CANDIDATE_ID.into(),
            new_ledger_epoch: NEW_EPOCH.into(),
        }
    );
}

#[test]
fn completed_recovery_allows_only_monotonic_post_recovery_device_authority_changes() {
    let conn = candidate_database();
    apply_recovery_activation(&conn, &request(), 11).unwrap();

    conn.execute(
        "UPDATE auth_state SET device_credential_generation=5 WHERE id=1",
        [],
    )
    .unwrap();
    assert_eq!(startup_mode(&conn), Err(RecoveryError::InvalidStartupState));

    conn.execute(
        "UPDATE auth_state SET device_credential_generation=4 WHERE id=1",
        [],
    )
    .unwrap();
    complete_recovery_activation(&conn, &completion(), 3).unwrap();
    conn.execute(
        "UPDATE auth_state SET device_credential_generation=5 WHERE id=1",
        [],
    )
    .unwrap();
    assert!(matches!(
        startup_mode(&conn).unwrap(),
        RecoveryStartupMode::Recovered { .. }
    ));

    conn.execute(
        "UPDATE auth_state SET device_credential_generation=3 WHERE id=1",
        [],
    )
    .unwrap();
    assert_eq!(startup_mode(&conn), Err(RecoveryError::InvalidStartupState));
}

#[test]
fn conflicting_recovery_request_rolls_back_without_changing_the_candidate() {
    let conn = candidate_database();
    let mut conflicting = request();
    conflicting.broker_credential_generation = 3;

    assert_eq!(
        apply_recovery_activation(&conn, &conflicting, 11),
        Err(RecoveryError::RecoveryConflict)
    );
    assert_eq!(iotkit_core_ledger::ledger_epoch(&conn).unwrap(), OLD_EPOCH);
    assert!(matches!(
        startup_mode(&conn).unwrap(),
        RecoveryStartupMode::FencedCandidate { .. }
    ));
}

#[test]
fn completion_for_a_different_candidate_fails_closed() {
    let conn = candidate_database();
    apply_recovery_activation(&conn, &request(), 11).unwrap();
    let mut conflicting = completion();
    conflicting.candidate_instance_id = "candidate-ffffffffffffffffffffffffffffffff".into();

    assert_eq!(
        complete_recovery_activation(&conn, &conflicting, 3),
        Err(RecoveryError::RecoveryConflict)
    );
    assert!(matches!(
        startup_mode(&conn).unwrap(),
        RecoveryStartupMode::AwaitingCompletion { .. }
    ));
}

#[test]
fn an_unaccepted_snapshot_epoch_start_is_replaced_not_counted_as_replay() {
    for (with_measurement, expected_replayed, expected_last) in [(false, 0, 1), (true, 1, 2)] {
        let conn = epoch_start_candidate(with_measurement);
        let mut request = request();
        request.snapshot_accepted_through = 0;
        request.snapshot_allocation_high_water = if with_measurement { 2 } else { 1 };
        request.snapshot_epoch_start_publication_seq = Some(1);
        request.edge_accepted_through = 0;

        let result = apply_recovery_activation(&conn, &request, 11).unwrap();
        assert_eq!(result.replayed_records, expected_replayed);
        assert_eq!(result.last_new_publication_seq, expected_last);
        let rows: Vec<(i64, String, Option<String>)> = conn
            .prepare(
                "SELECT pub_seq,kind,subtype FROM publication_log
                 WHERE epoch=?1 ORDER BY pub_seq",
            )
            .unwrap()
            .query_map([NEW_EPOCH], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), expected_last as usize);
        assert_eq!(
            rows[0],
            (1, "annotation".into(), Some("epoch_start".into()))
        );
    }
}
