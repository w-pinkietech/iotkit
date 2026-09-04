use super::*;
use crate::definition::tests::{count_definition, measurement_definition};
use crate::definition::{Detector, DetectorMode, PipelineInput};
use crate::outbox::{self, OutboxRow};
use crate::{Calibration, evaluator};
use iotkit_core_storage::DbHandle;
use rusqlite::{Connection, TransactionBehavior};

fn open() -> DbHandle {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(crate::MIGRATIONS);
    iotkit_core_storage::init_db_memory(&migrations).unwrap()
}

fn engine() -> PipelineEngine {
    PipelineEngine::new("rpi1".parse().unwrap())
}

fn with<T>(db: &DbHandle, f: impl FnOnce(&Connection) -> T) -> T {
    db.with_conn_sync(|conn| Ok(f(conn))).unwrap()
}

fn payload_json(row: &OutboxRow) -> serde_json::Value {
    serde_json::from_slice(&row.payload).unwrap()
}

fn state_definition() -> PipelineDefinition {
    PipelineDefinition {
        id: "press-01-high".parse().unwrap(),
        kind: PipelineKind::State,
        trigger: None,
        display_name: None,
        detector: Some(Detector {
            mode: DetectorMode::HighActive,
            rise_threshold: 10.0,
            fall_threshold: 4.0,
            rise_debounce_ms: 0,
            fall_debounce_ms: 0,
        }),
        ..count_definition()
    }
}

#[test]
fn creating_an_accumulated_count_publishes_sequence_one_value_zero_immediately() {
    let db = open();
    with(&db, |conn| {
        let start = engine().create(conn, &count_definition(), 1_000).unwrap();
        let published = start
            .published
            .expect("accumulated-count publishes at series start");
        assert_eq!(published.sequence, 1);
        assert_eq!(published.value, ObservationValue::AccumulatedCount(0));
        assert_eq!(published.timestamp, 1_000);
        assert_eq!(published.series_id, start.series_id);

        let rows = outbox::all(conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].topic,
            "iotkit/v1/edge-node/rpi1/observation/press-01-cycle-count/accumulated-count"
        );
        assert!(rows[0].retain);
        let payload = payload_json(&rows[0]);
        assert_eq!(payload["sequence"], 1);
        assert_eq!(payload["value"], 0);
        assert_eq!(payload["series_id"], start.series_id);

        let state = store::get_state(conn, &count_definition().id)
            .unwrap()
            .unwrap();
        assert_eq!(state.next_sequence, 2);
        assert_eq!(state.series_id, start.series_id);
        assert!(!state.evaluation.initialized);
    });
}

#[test]
fn measurement_and_state_publish_nothing_at_series_start() {
    let db = open();
    with(&db, |conn| {
        let engine = engine();
        assert!(
            engine
                .create(conn, &measurement_definition(), 0)
                .unwrap()
                .published
                .is_none()
        );
        assert!(
            engine
                .create(conn, &state_definition(), 0)
                .unwrap()
                .published
                .is_none()
        );
        assert_eq!(outbox::count(conn).unwrap(), 0);
        assert_eq!(
            store::get_state(conn, &state_definition().id)
                .unwrap()
                .unwrap()
                .next_sequence,
            1
        );
    });
}

#[test]
fn creating_twice_or_updating_a_missing_pipeline_fails() {
    let db = open();
    with(&db, |conn| {
        let engine = engine();
        engine.create(conn, &count_definition(), 0).unwrap();
        assert!(matches!(
            engine.create(conn, &count_definition(), 0),
            Err(EngineError::AlreadyExists(_))
        ));
        assert!(matches!(
            engine.update(conn, &measurement_definition(), 0),
            Err(EngineError::NotFound(_))
        ));
        assert!(matches!(
            engine.reset(conn, &measurement_definition().id, 0),
            Err(EngineError::NotFound(_))
        ));
    });
}

#[test]
fn accumulated_count_increments_publish_in_sequence_within_one_series() {
    let db = open();
    with(&db, |conn| {
        let engine = engine();
        let definition = count_definition();
        let start = engine.create(conn, &definition, 0).unwrap();
        let inputs = [(0.0, 10), (1.0, 20), (1.0, 30), (0.0, 40), (1.0, 50)];
        let mut published = Vec::new();
        for (value, at) in inputs {
            match engine.process(conn, &definition, value, at).unwrap() {
                DeliveryOutcome::Published(observation) => published.push(observation),
                DeliveryOutcome::Silent => {}
            }
        }
        assert_eq!(published.len(), 2);
        assert_eq!(published[0].sequence, 2);
        assert_eq!(published[0].value, ObservationValue::AccumulatedCount(1));
        assert_eq!(published[0].timestamp, 20);
        assert_eq!(published[1].sequence, 3);
        assert_eq!(published[1].value, ObservationValue::AccumulatedCount(2));
        assert!(published.iter().all(|o| o.series_id == start.series_id));
        assert_eq!(outbox::count(conn).unwrap(), 3);
        let state = store::get_state(conn, &definition.id).unwrap().unwrap();
        assert_eq!(state.evaluation.counter, 2);
        assert_eq!(state.next_sequence, 4);
        assert_eq!(state.last_timestamp, Some(50));
    });
}

#[test]
fn measurement_publishes_only_when_the_calibrated_value_changes() {
    let db = open();
    with(&db, |conn| {
        let engine = engine();
        let mut definition = measurement_definition();
        definition.calibration = Calibration {
            scale: 0.5,
            offset: 0.0,
        };
        engine.create(conn, &definition, 0).unwrap();
        let outcomes: Vec<_> = [(48.0, 1), (48.0, 2), (49.0, 3), (49.0, 4), (48.0, 5)]
            .into_iter()
            .map(|(value, at)| engine.process(conn, &definition, value, at).unwrap())
            .collect();
        let sequences: Vec<(u64, f64)> = outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                DeliveryOutcome::Published(o) => match o.value {
                    ObservationValue::Measurement(v) => Some((o.sequence, v)),
                    _ => None,
                },
                DeliveryOutcome::Silent => None,
            })
            .collect();
        assert_eq!(sequences, vec![(1, 24.0), (2, 24.5), (3, 24.0)]);
        let rows = outbox::all(conn).unwrap();
        assert_eq!(payload_json(&rows[0])["value"], 24);
        assert_eq!(payload_json(&rows[1])["value"], 24.5);
    });
}

#[test]
fn state_publishes_the_first_input_and_each_transition() {
    let db = open();
    with(&db, |conn| {
        let engine = engine();
        let definition = state_definition();
        engine.create(conn, &definition, 0).unwrap();
        let values: Vec<bool> = [(3.0, 1), (5.0, 2), (12.0, 3), (5.0, 4), (3.0, 5)]
            .into_iter()
            .filter_map(
                |(value, at)| match engine.process(conn, &definition, value, at).unwrap() {
                    DeliveryOutcome::Published(Observation {
                        value: ObservationValue::State(v),
                        ..
                    }) => Some(v),
                    _ => None,
                },
            )
            .collect();
        assert_eq!(values, vec![false, true, false]);
        assert_eq!(payload_json(&outbox::all(conn).unwrap()[1])["value"], true);
    });
}

#[test]
fn tuning_changes_keep_the_series_and_structural_changes_start_a_new_one() {
    let db = open();
    with(&db, |conn| {
        let engine = engine();
        let definition = count_definition();
        let first = engine.create(conn, &definition, 0).unwrap();
        engine.process(conn, &definition, 0.0, 1).unwrap();
        engine.process(conn, &definition, 1.0, 2).unwrap();

        let mut tuned = definition.clone();
        tuned.display_name = Some("renamed".into());
        tuned.detector = Some(Detector {
            rise_threshold: 0.8,
            fall_threshold: 0.2,
            ..definition.detector.unwrap()
        });
        assert!(engine.update(conn, &tuned, 3).unwrap().is_none());
        let state = store::get_state(conn, &definition.id).unwrap().unwrap();
        assert_eq!(state.series_id, first.series_id);
        assert_eq!(state.evaluation.counter, 1);
        assert_eq!(
            store::get_definition(conn, &definition.id)
                .unwrap()
                .unwrap(),
            tuned
        );

        let mut restructured = tuned.clone();
        restructured.input.channel_index = Some(2);
        let second = engine
            .update(conn, &restructured, 4)
            .unwrap()
            .expect("new series");
        assert_ne!(second.series_id, first.series_id);
        let state = store::get_state(conn, &definition.id).unwrap().unwrap();
        assert_eq!(state.series_id, second.series_id);
        assert_eq!(state.evaluation.counter, 0);
        assert_eq!(state.next_sequence, 2);
        let last = outbox::all(conn).unwrap().pop().unwrap();
        let payload = payload_json(&last);
        assert_eq!(payload["sequence"], 1);
        assert_eq!(payload["value"], 0);
        assert_eq!(payload["series_id"], second.series_id);

        let mut other_kind = restructured.clone();
        other_kind.kind = PipelineKind::State;
        other_kind.trigger = None;
        assert!(matches!(
            engine.update(conn, &other_kind, 5),
            Err(EngineError::KindChanged)
        ));
    });
}

#[test]
fn reset_starts_a_new_series_and_delete_clears_the_retained_value() {
    let db = open();
    with(&db, |conn| {
        let engine = engine();
        let definition = count_definition();
        let first = engine.create(conn, &definition, 0).unwrap();
        engine.process(conn, &definition, 0.0, 1).unwrap();
        engine.process(conn, &definition, 1.0, 2).unwrap();

        let reset = engine.reset(conn, &definition.id, 3).unwrap();
        assert_ne!(reset.series_id, first.series_id);
        assert_eq!(
            reset.published.unwrap().value,
            ObservationValue::AccumulatedCount(0)
        );
        assert_eq!(
            store::get_state(conn, &definition.id)
                .unwrap()
                .unwrap()
                .evaluation
                .counter,
            0
        );

        engine.delete(conn, &definition.id, 4).unwrap();
        assert!(
            store::get_definition(conn, &definition.id)
                .unwrap()
                .is_none()
        );
        assert!(
            store::get_state(conn, &definition.id).unwrap().is_none(),
            "state cascades"
        );
        let last = outbox::all(conn).unwrap().pop().unwrap();
        assert_eq!(
            last.topic,
            "iotkit/v1/edge-node/rpi1/observation/press-01-cycle-count/accumulated-count"
        );
        assert!(last.payload.is_empty());
        assert!(last.retain);
        assert!(matches!(
            engine.delete(conn, &definition.id, 5),
            Err(EngineError::NotFound(_))
        ));
    });
}

#[test]
fn reconcile_starts_series_only_where_state_is_missing_or_structurally_stale() {
    let db = open();
    with(&db, |conn| {
        let engine = engine();
        let count = count_definition();
        let measurement = measurement_definition();
        let created = engine.create(conn, &count, 0).unwrap();
        engine.create(conn, &measurement, 0).unwrap();
        assert!(
            engine.reconcile(conn, 1).unwrap().is_empty(),
            "matching hashes continue"
        );

        // Simulate a definition edited without the engine (for example a
        // restored database whose state row is from another structure).
        let mut restructured = measurement.clone();
        restructured.unit = Some("mm".into());
        store::update_definition(conn, &restructured, 2).unwrap();
        conn.execute(
            "DELETE FROM pipeline_state WHERE pipeline_id = ?1",
            [count.id.as_str()],
        )
        .unwrap();

        let started = engine.reconcile(conn, 3).unwrap();
        let ids: Vec<&str> = started.iter().map(|s| s.pipeline_id.as_str()).collect();
        assert_eq!(ids, vec![count.id.as_str(), measurement.id.as_str()]);
        assert_ne!(started[0].series_id, created.series_id);
        assert!(started[0].published.is_some());
        assert!(started[1].published.is_none());
    });
}

#[test]
fn reconcile_records_the_edge_node_id_for_tools_without_the_toml() {
    let db = open();
    with(&db, |conn| {
        assert!(PipelineEngine::load(conn).unwrap().is_none());
        engine().reconcile(conn, 0).unwrap();
        let loaded = PipelineEngine::load(conn)
            .unwrap()
            .expect("recorded at startup");
        assert_eq!(loaded.edge_node_id().as_str(), "rpi1");
        PipelineEngine::new("rpi2".parse().unwrap())
            .reconcile(conn, 1)
            .unwrap();
        assert_eq!(
            PipelineEngine::load(conn)
                .unwrap()
                .unwrap()
                .edge_node_id()
                .as_str(),
            "rpi2"
        );
    });
}

#[test]
fn state_and_outbox_commit_or_roll_back_together() {
    let db = open();
    let definition = count_definition();
    with(&db, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate).unwrap();
        engine().create(&tx, &definition, 0).unwrap();
        tx.commit().unwrap();
    });
    with(&db, |conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate).unwrap();
        engine().process(&tx, &definition, 0.0, 1).unwrap();
        let outcome = engine().process(&tx, &definition, 1.0, 2).unwrap();
        assert!(matches!(outcome, DeliveryOutcome::Published(_)));
        assert_eq!(outbox::count(&tx).unwrap(), 2);
        tx.rollback().unwrap();
    });
    with(&db, |conn| {
        let state = store::get_state(conn, &definition.id).unwrap().unwrap();
        assert!(
            !state.evaluation.initialized,
            "rolled back with the outbox row"
        );
        assert_eq!(state.next_sequence, 2);
        assert_eq!(outbox::count(conn).unwrap(), 1);

        let tx =
            rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate).unwrap();
        engine().process(&tx, &definition, 0.0, 1).unwrap();
        engine().process(&tx, &definition, 1.0, 2).unwrap();
        tx.commit().unwrap();
        let state = store::get_state(conn, &definition.id).unwrap().unwrap();
        assert_eq!(state.evaluation.counter, 1);
        assert_eq!(state.next_sequence, 3);
        assert_eq!(outbox::count(conn).unwrap(), 2);
    });
}

#[test]
fn an_evaluator_error_leaves_state_and_outbox_untouched() {
    let db = open();
    with(&db, |conn| {
        let engine = engine();
        let definition = count_definition();
        engine.create(conn, &definition, 0).unwrap();
        engine.process(conn, &definition, 0.0, 1).unwrap();
        let mut state = store::get_state(conn, &definition.id).unwrap().unwrap();
        state.evaluation.counter = evaluator::MAX_SAFE_INTEGER;
        store::put_state(conn, &definition.id, &state, 1).unwrap();

        let error = engine.process(conn, &definition, 1.0, 2).unwrap_err();
        assert!(matches!(
            error,
            EngineError::Evaluator(EvaluatorError::CounterLimit)
        ));
        let after = store::get_state(conn, &definition.id).unwrap().unwrap();
        assert_eq!(after, state);
        assert_eq!(outbox::count(conn).unwrap(), 1);
    });
}

fn reading<'a>(
    adapter: &'a str,
    subject: Option<&'a str>,
    measurement_key: &'a str,
    channel_index: Option<u16>,
    values: &'a [f64],
    received_at: i64,
) -> AcceptedReading<'a> {
    AcceptedReading {
        adapter,
        subject,
        measurement_key,
        channel_index,
        values,
        received_at,
    }
}

#[test]
fn deliver_routes_a_reading_to_every_matching_pipeline_only() {
    let db = open();
    with(&db, |conn| {
        let engine = engine();
        let count = count_definition();
        let mut pinned = count.clone();
        pinned.id = "press-01-cycle-count-b".parse().unwrap();
        pinned.input.subject = Some("ns:state".into());
        let mut second_value = count.clone();
        second_value.id = "press-01-cycle-count-c".parse().unwrap();
        second_value.input.value_index = 1;
        for definition in [&count, &pinned, &second_value] {
            engine.create(conn, definition, 0).unwrap();
        }

        let outcomes = engine
            .deliver(
                conn,
                &reading(
                    "trial_sample",
                    Some("ns:state"),
                    "contact_state",
                    None,
                    &[1.0],
                    1,
                ),
            )
            .unwrap();
        let ids: Vec<&str> = outcomes.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                count.id.as_str(),
                pinned.id.as_str(),
                second_value.id.as_str()
            ]
        );
        assert!(matches!(outcomes[0].1, Ok(DeliveryOutcome::Silent)));
        assert!(matches!(outcomes[1].1, Ok(DeliveryOutcome::Silent)));
        assert!(matches!(outcomes[2].1, Err(EngineError::MissingValue(1))));

        let outcomes = engine
            .deliver(
                conn,
                &reading(
                    "trial_sample",
                    Some("ns:other"),
                    "contact_state",
                    None,
                    &[0.0, 0.0],
                    2,
                ),
            )
            .unwrap();
        let ids: Vec<&str> = outcomes.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec![count.id.as_str(), second_value.id.as_str()]);

        assert!(
            engine
                .deliver(
                    conn,
                    &reading("trial_sample", None, "illuminance_lux", None, &[1.0], 3)
                )
                .unwrap()
                .is_empty()
        );
        assert!(
            engine
                .deliver(
                    conn,
                    &reading("other", None, "contact_state", None, &[1.0], 3)
                )
                .unwrap()
                .is_empty()
        );
        assert!(
            engine
                .deliver(
                    conn,
                    &reading("trial_sample", None, "contact_state", Some(0), &[1.0], 3)
                )
                .unwrap()
                .is_empty()
        );
    });
}

#[test]
fn import_replaces_definitions_clears_vanished_topics_and_restarts_every_series() {
    let db = open();
    with(&db, |conn| {
        let engine = engine();
        let count = count_definition();
        let measurement = measurement_definition();
        let first = engine.create(conn, &count, 0).unwrap();
        engine.create(conn, &measurement, 0).unwrap();
        let before = outbox::count(conn).unwrap();

        let mut renamed = count.clone();
        renamed.display_name = Some("imported".into());
        let mut added = state_definition();
        added.input = PipelineInput {
            measurement_key: "illuminance_lux".into(),
            ..added.input
        };
        let started = engine
            .import(conn, &[renamed.clone(), added.clone()], 10)
            .unwrap();
        assert_eq!(started.len(), 2);
        assert_ne!(
            started[0].series_id, first.series_id,
            "import restarts every series"
        );

        let definitions = store::list_definitions(conn).unwrap();
        assert_eq!(definitions, vec![renamed, added]);
        assert!(store::get_state(conn, &measurement.id).unwrap().is_none());

        let rows = outbox::all(conn).unwrap();
        let new_rows = &rows[before as usize..];
        assert!(
            new_rows.iter().any(|row| row.payload.is_empty()
                && row.topic.ends_with("/press-01-temperature/measurement")),
            "vanished pipeline clears its retained value"
        );
        assert!(
            new_rows
                .iter()
                .filter(|row| !row.payload.is_empty())
                .any(|row| payload_json(row)["value"] == 0)
        );

        let mut duplicate = vec![count.clone(), count.clone()];
        duplicate[1].display_name = Some("dup".into());
        assert!(matches!(
            engine.import(conn, &duplicate, 11),
            Err(EngineError::Validation(_))
        ));
    });
}
