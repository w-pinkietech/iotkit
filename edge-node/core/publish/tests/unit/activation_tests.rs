use super::{
    ActivationRequest, ActivationResult, ActivationState, activation_state, apply_activation,
    cleanup_pre_activation_batch, install_edge_target,
};
use crate::store::TargetRow;
use rusqlite::{Connection, params};

const EDGE_ID: &str = "edge-node-01";
const EPOCH: &str = "01JTESTEPOCH";

fn initialize_identity(conn: &Connection) {
    conn.execute(
        "INSERT INTO ledger_meta(key, value) VALUES('edge_node_id', ?1)",
        [EDGE_ID],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ledger_meta(key, value) VALUES('epoch', ?1)",
        [EPOCH],
    )
    .unwrap();
}

fn target() -> TargetRow {
    TargetRow {
        target_id: "edge".into(),
        endpoint_url: "mqtts://broker.example.test:8883".into(),
        credential_token: "secret".into(),
        archive_responsible: true,
        schema_version: 1,
        cursor_epoch: None,
        cursor_pub_seq: 0,
    }
}

fn request(activation_id: &str) -> ActivationRequest {
    ActivationRequest {
        schema_version: 1,
        activation_id: activation_id.into(),
        edge_id: "edge-0123456789abcdef0123456789abcdef".into(),
        edge_node_id: EDGE_ID.into(),
        expected_ledger_epoch: EPOCH.into(),
        grant_revision: 1,
        issued_at: 1_720_000_000_000,
    }
}

fn seed_readings(conn: &Connection, count: i64) {
    conn.execute(
        "INSERT INTO devices(system_id, hardware_id, kind, state, created_at)
             VALUES(zeroblob(16), 'test-device', 'individual', 'active', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO series(system_id, measurement_key, created_at)
             VALUES(zeroblob(16), 'temperature', 1)",
        [],
    )
    .unwrap();
    for received_at in 1..=count {
        conn.execute(
            "INSERT INTO readings(
                     series_id, received_at, time_source, values_json
                 ) VALUES(1, ?1, 'edge', '[20.0]')",
            [received_at],
        )
        .unwrap();
    }
}

fn discovery_only() -> Connection {
    let conn = crate::tests_support::open();
    initialize_identity(&conn);
    assert_eq!(
        activation_state(&conn).unwrap(),
        ActivationState::Standalone
    );
    install_edge_target(&conn, &target(), 1).unwrap();
    assert_eq!(
        activation_state(&conn).unwrap(),
        ActivationState::DiscoveryOnly
    );
    conn
}

#[test]
fn fresh_target_waits_for_activation_and_exact_duplicate_replays_result() {
    let conn = discovery_only();
    seed_readings(&conn, 2);
    let original = request("act-0123456789abcdef0123456789abcdef");

    let result = apply_activation(&conn, &original, 1_720_000_001_000).unwrap();

    assert_eq!(result.first_publication_seq, 1);
    assert_eq!(result.discard_through_reading_seq, 2);
    assert_eq!(result.status, "applied");
    assert_eq!(activation_state(&conn).unwrap(), ActivationState::Active);
    assert_eq!(
        apply_activation(&conn, &original, 1_720_000_009_999).unwrap(),
        result
    );
    assert!(
        apply_activation(
            &conn,
            &request("act-fedcba9876543210fedcba9876543210"),
            1_720_000_010_000
        )
        .is_err()
    );
    assert_eq!(
        crate::store::enqueue_measurement(&conn, EPOCH, 3, 1_720_000_010_001).unwrap(),
        1
    );
}

#[test]
fn activation_rejects_any_prior_publication_allocation_or_cursor() {
    for poison in ["row", "sequence", "cursor"] {
        let conn = discovery_only();
        match poison {
            "row" => {
                conn.execute(
                    "INSERT INTO publication_log(epoch, kind, created_at)
                         VALUES(?1, 'annotation', 1)",
                    [EPOCH],
                )
                .unwrap();
            }
            "sequence" => {
                conn.execute(
                    "INSERT INTO publication_log(epoch, kind, created_at)
                         VALUES(?1, 'annotation', 1)",
                    [EPOCH],
                )
                .unwrap();
                conn.execute("DELETE FROM publication_log", []).unwrap();
            }
            "cursor" => {
                conn.execute("UPDATE target_registry SET cursor_pub_seq = 1", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }

        let error = apply_activation(&conn, &request("act-0123456789abcdef0123456789abcdef"), 2)
            .unwrap_err();
        assert!(
            error.to_string().contains("unused publication stream"),
            "{poison}: {error}"
        );
    }
}

#[test]
fn activation_requires_the_initialized_edge_and_exact_epoch() {
    let conn = discovery_only();
    for mut invalid in [
        request("act-0123456789abcdef0123456789abcdef"),
        request("act-fedcba9876543210fedcba9876543210"),
    ] {
        if invalid.activation_id.starts_with("act-0") {
            invalid.edge_node_id = "edge-node-other".into();
        } else {
            invalid.expected_ledger_epoch = "01JOTHER".into();
        }
        assert!(apply_activation(&conn, &invalid, 2).is_err());
    }
    assert_eq!(
        activation_state(&conn).unwrap(),
        ActivationState::DiscoveryOnly
    );
}

#[test]
fn cleanup_deletes_only_the_frozen_prefix_in_bounded_batches() {
    let conn = discovery_only();
    seed_readings(&conn, 3);
    apply_activation(&conn, &request("act-0123456789abcdef0123456789abcdef"), 10).unwrap();
    conn.execute(
        "INSERT INTO readings(series_id, received_at, time_source, values_json)
             VALUES(1, 4, 'edge', '[21.0]')",
        [],
    )
    .unwrap();

    assert_eq!(cleanup_pre_activation_batch(&conn, 2).unwrap(), 2);
    assert_eq!(cleanup_pre_activation_batch(&conn, 2).unwrap(), 1);
    assert_eq!(cleanup_pre_activation_batch(&conn, 2).unwrap(), 0);
    let remaining: Vec<i64> = conn
        .prepare("SELECT seq FROM readings ORDER BY seq")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(remaining, vec![4]);
}

#[test]
fn request_decoder_is_strict_about_contract_fields() {
    let valid = serde_json::to_vec(&request("act-0123456789abcdef0123456789abcdef")).unwrap();
    assert_eq!(
        ActivationRequest::decode(&valid).unwrap(),
        request("act-0123456789abcdef0123456789abcdef")
    );

    for invalid in [
            br#"{"schema_version":2,"activation_id":"act-0123456789abcdef0123456789abcdef","edge_id":"edge-0123456789abcdef0123456789abcdef","edge_node_id":"edge-node-01","expected_ledger_epoch":"01JTESTEPOCH","grant_revision":1,"issued_at":1}"#.as_slice(),
            br#"{"schema_version":1,"activation_id":"act-0123456789ABCDEF0123456789ABCDEF","edge_id":"edge-0123456789abcdef0123456789abcdef","edge_node_id":"edge-node-01","expected_ledger_epoch":"01JTESTEPOCH","grant_revision":1,"issued_at":1}"#.as_slice(),
            br#"{"schema_version":1,"activation_id":"act-0123456789abcdef0123456789abcdef","edge_id":"edge-0123456789abcdef0123456789abcdef","edge_node_id":"edge-node-01","expected_ledger_epoch":"01JTESTEPOCH","grant_revision":1,"issued_at":1,"unknown":true}"#.as_slice(),
        ] {
            assert!(ActivationRequest::decode(invalid).is_err());
        }
}

#[test]
fn shared_activation_fixtures_match_the_edge_contract() {
    let valid_request = ActivationRequest::decode(include_bytes!(
        "../../../../../testdata/egress/v1/activation-request.json"
    ))
    .unwrap();
    let valid_result = ActivationResult::decode(include_bytes!(
        "../../../../../testdata/egress/v1/activation-result.json"
    ))
    .unwrap();
    assert_eq!(valid_request.edge_node_id, valid_result.edge_node_id);
    assert_eq!(
        valid_request.expected_ledger_epoch,
        valid_result.ledger_epoch
    );
    for invalid in [
        include_bytes!("../../../../../testdata/egress/v1/activation-request-malformed-id.json")
            .as_slice(),
        include_bytes!("../../../../../testdata/egress/v1/activation-request-unknown-field.json")
            .as_slice(),
    ] {
        assert!(ActivationRequest::decode(invalid).is_err());
    }
    assert!(
        ActivationResult::decode(include_bytes!(
            "../../../../../testdata/egress/v1/activation-result-first-seq-2.json"
        ))
        .is_err()
    );

    let conn = discovery_only();
    for contextual_mismatch in [
        include_bytes!("../../../../../testdata/egress/v1/activation-request-wrong-edge-node.json")
            .as_slice(),
        include_bytes!("../../../../../testdata/egress/v1/activation-request-wrong-epoch.json")
            .as_slice(),
    ] {
        let request = ActivationRequest::decode(contextual_mismatch).unwrap();
        assert!(apply_activation(&conn, &request, 1_720_000_001_000).is_err());
    }
    let original = ActivationRequest::decode(include_bytes!(
        "../../../../../testdata/egress/v1/activation-request.json"
    ))
    .unwrap();
    apply_activation(&conn, &original, 1_720_000_001_000).unwrap();
    let conflicting = ActivationRequest::decode(include_bytes!(
        "../../../../../testdata/egress/v1/activation-request-conflicting-id.json"
    ))
    .unwrap();
    assert!(apply_activation(&conn, &conflicting, 1_720_000_002_000).is_err());
}

#[test]
fn existing_target_migrates_as_active() {
    let conn = Connection::open_in_memory().unwrap();
    let mut before_activation = Vec::new();
    before_activation.extend_from_slice(iotkit_core_storage::MIGRATIONS);
    before_activation.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    before_activation.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    before_activation.push(crate::MIGRATIONS[0]);
    before_activation.sort_by_key(|migration| migration.version);
    iotkit_core_storage::run_migrations(&conn, &before_activation).unwrap();
    conn.execute(
        "INSERT INTO target_registry(
                 target_id, endpoint_url, credential_token, archive_responsible,
                 schema_version, cursor_pub_seq, created_at
             ) VALUES('edge', 'mqtts://broker', 'secret', 1, 1, 0, 1)",
        [],
    )
    .unwrap();

    let mut after_activation = before_activation;
    after_activation.push(crate::MIGRATIONS[1]);
    after_activation.sort_by_key(|migration| migration.version);
    iotkit_core_storage::run_migrations(&conn, &after_activation).unwrap();

    assert_eq!(activation_state(&conn).unwrap(), ActivationState::Active);
}

#[test]
fn target_install_rejects_a_used_standalone_outbox() {
    let conn = crate::tests_support::open();
    initialize_identity(&conn);
    conn.execute(
        "INSERT INTO publication_log(epoch, kind, created_at)
             VALUES(?1, 'annotation', 1)",
        params![EPOCH],
    )
    .unwrap();

    let error = install_edge_target(&conn, &target(), 2).unwrap_err();

    assert!(error.to_string().contains("standalone outbox adoption"));
    assert_eq!(crate::store::target_count(&conn).unwrap(), 0);
    assert_eq!(
        activation_state(&conn).unwrap(),
        ActivationState::Standalone
    );
}
