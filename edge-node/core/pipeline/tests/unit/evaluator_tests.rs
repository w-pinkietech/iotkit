use super::*;
use crate::Calibration;
use crate::definition::PipelineInput;

fn definition(
    kind: PipelineKind,
    detector: Option<Detector>,
    trigger: Option<Trigger>,
) -> PipelineDefinition {
    PipelineDefinition {
        id: "p".parse().unwrap(),
        kind,
        input: PipelineInput {
            adapter: "a".into(),
            subject: None,
            measurement_key: "k".into(),
            channel_index: None,
            value_index: 0,
        },
        trigger,
        unit: (kind == PipelineKind::Measurement).then(|| "lx".to_string()),
        display_name: None,
        calibration: Calibration::default(),
        detector,
    }
}

fn high_active(rise: f64, fall: f64, rise_ms: i64, fall_ms: i64) -> Detector {
    Detector {
        mode: DetectorMode::HighActive,
        rise_threshold: rise,
        fall_threshold: fall,
        rise_debounce_ms: rise_ms,
        fall_debounce_ms: fall_ms,
    }
}

fn low_active(rise: f64, fall: f64, rise_ms: i64, fall_ms: i64) -> Detector {
    Detector {
        mode: DetectorMode::LowActive,
        ..high_active(rise, fall, rise_ms, fall_ms)
    }
}

fn run(definition: &PipelineDefinition, inputs: &[(f64, i64)]) -> Vec<Evaluation> {
    let mut state = EvaluationState::default();
    inputs
        .iter()
        .map(|(value, at)| {
            let (evaluation, next) = evaluate(definition, state, *value, *at).unwrap();
            state = next;
            evaluation
        })
        .collect()
}

fn integers(evaluations: &[Evaluation]) -> Vec<i64> {
    evaluations.iter().filter_map(|e| e.integer).collect()
}

fn booleans(evaluations: &[Evaluation]) -> Vec<bool> {
    evaluations.iter().filter_map(|e| e.boolean).collect()
}

#[test]
fn measurement_emits_the_calibrated_value_on_every_input() {
    let mut definition = definition(PipelineKind::Measurement, None, None);
    definition.calibration = Calibration {
        scale: 2.0,
        offset: 1.0,
    };
    let evaluations = run(&definition, &[(1.0, 0), (1.0, 1), (2.5, 2)]);
    assert!(evaluations.iter().all(|e| e.emitted));
    assert_eq!(
        evaluations
            .iter()
            .map(|e| e.number.unwrap())
            .collect::<Vec<_>>(),
        vec![3.0, 3.0, 6.0]
    );
}

#[test]
fn accumulated_count_uses_the_first_sample_only_as_baseline() {
    let definition = definition(
        PipelineKind::AccumulatedCount,
        Some(high_active(0.5, 0.5, 0, 0)),
        Some(Trigger::OnTransition),
    );
    // Baseline already active: no count until it falls and rises again.
    let evaluations = run(
        &definition,
        &[(1.0, 0), (1.0, 1), (0.0, 2), (1.0, 3), (0.0, 4), (1.0, 5)],
    );
    assert!(!evaluations[0].emitted);
    assert_eq!(integers(&evaluations), vec![1, 2]);
}

#[test]
fn state_emits_the_first_sample_and_confirmed_transitions_only() {
    let definition = definition(
        PipelineKind::State,
        Some(high_active(10.0, 4.0, 0, 0)),
        None,
    );
    let evaluations = run(
        &definition,
        &[(3.0, 0), (5.0, 1), (10.0, 2), (5.0, 3), (4.0, 4), (11.0, 5)],
    );
    assert_eq!(booleans(&evaluations), vec![false, true, false, true]);
    // Hysteresis: 5.0 is inside the band and keeps whatever the state was.
    assert!(!evaluations[1].emitted);
    assert!(!evaluations[3].emitted);
}

#[test]
fn high_active_detector_applies_independent_debounce_with_inclusive_boundary() {
    let definition = definition(
        PipelineKind::State,
        Some(high_active(10.0, 4.0, 2_000, 3_000)),
        None,
    );
    let evaluations = run(
        &definition,
        &[
            (0.0, 0),
            (12.0, 1_000), // rise pending since 1000
            (12.0, 2_999), // 1999 ms: not yet
            (12.0, 3_000), // exactly 2000 ms: confirmed
            (2.0, 4_000),  // fall pending since 4000
            (2.0, 6_999),  // 2999 ms: not yet
            (2.0, 7_000),  // exactly 3000 ms: confirmed
        ],
    );
    assert_eq!(
        evaluations.iter().map(|e| e.emitted).collect::<Vec<_>>(),
        vec![true, false, false, true, false, false, true]
    );
    assert_eq!(booleans(&evaluations), vec![false, true, false]);
}

#[test]
fn debounce_restarts_when_the_candidate_flips_or_the_clock_goes_backwards() {
    let definition = definition(
        PipelineKind::State,
        Some(high_active(10.0, 4.0, 2_000, 0)),
        None,
    );
    // Candidate flips back inside the window: no transition.
    let evaluations = run(
        &definition,
        &[
            (0.0, 0),
            (12.0, 1_000),
            (0.0, 2_000),
            (12.0, 2_500),
            (12.0, 4_000),
        ],
    );
    assert_eq!(booleans(&evaluations), vec![false]);
    let evaluations = run(
        &definition,
        &[
            (0.0, 0),
            (12.0, 1_000),
            (0.0, 2_000),
            (12.0, 2_500),
            (12.0, 4_500),
        ],
    );
    assert_eq!(booleans(&evaluations), vec![false, true]);

    // Clock correction backwards restarts the window from the new time.
    let evaluations = run(
        &definition,
        &[
            (0.0, 5_000),
            (12.0, 6_000),
            (12.0, 4_000),
            (12.0, 5_999),
            (12.0, 6_000),
        ],
    );
    assert_eq!(
        evaluations.iter().map(|e| e.emitted).collect::<Vec<_>>(),
        vec![true, false, false, false, true]
    );
}

#[test]
fn low_active_detector_mirrors_thresholds_and_debounce_direction() {
    // Active when the signal is low. rise_debounce applies when the signal
    // rises (state becomes inactive); fall_debounce when it falls (active).
    let definition = definition(
        PipelineKind::State,
        Some(low_active(10.0, 4.0, 1_000, 3_000)),
        None,
    );
    let evaluations = run(
        &definition,
        &[
            (20.0, 0),    // inactive baseline
            (4.0, 1_000), // <= fall: candidate active, fall debounce 3000
            (4.0, 3_999),
            (4.0, 4_000),  // active
            (5.0, 5_000),  // inside band: stays active
            (10.0, 6_000), // >= rise: candidate inactive, rise debounce 1000
            (10.0, 7_000), // inactive
        ],
    );
    assert_eq!(booleans(&evaluations), vec![false, true, false]);
    assert!(evaluations[3].emitted && evaluations[6].emitted);
}

#[test]
fn accumulated_count_stops_at_the_safe_integer_limit() {
    let definition = definition(
        PipelineKind::AccumulatedCount,
        Some(high_active(0.5, 0.5, 0, 0)),
        Some(Trigger::OnTransition),
    );
    let state = EvaluationState {
        initialized: true,
        active: false,
        counter: MAX_SAFE_INTEGER,
        ..EvaluationState::default()
    };
    assert_eq!(
        evaluate(&definition, state, 1.0, 0).unwrap_err(),
        EvaluatorError::CounterLimit
    );
    let (evaluation, next) = evaluate(
        &definition,
        EvaluationState {
            counter: MAX_SAFE_INTEGER - 1,
            ..state
        },
        1.0,
        0,
    )
    .unwrap();
    assert_eq!(evaluation.integer, Some(MAX_SAFE_INTEGER));
    assert_eq!(next.counter, MAX_SAFE_INTEGER);
}

#[test]
fn calibration_rejects_non_finite_results_and_negative_time() {
    let mut definition = definition(PipelineKind::Measurement, None, None);
    definition.calibration = Calibration {
        scale: f64::MAX,
        offset: 0.0,
    };
    assert!(evaluate(&definition, EvaluationState::default(), f64::MAX, 0).is_err());
    assert!(evaluate(&definition, EvaluationState::default(), f64::NAN, 0).is_err());
    definition.calibration = Calibration::default();
    assert!(evaluate(&definition, EvaluationState::default(), 1.0, -1).is_err());
}
