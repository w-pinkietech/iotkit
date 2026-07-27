use iotkit_edge::semantics::{
    Calibration, DefinitionSpec, Detector, DetectorMode, EvaluationState, PreviewInput, RuleSpec,
    SemanticKind, TriggerMode, build_preview, evaluate_at, evaluate_rule,
};

fn boolean_detector() -> Detector {
    Detector {
        mode: DetectorMode::BooleanHighActive,
        rise_threshold: 0.0,
        fall_threshold: 0.0,
        rise_debounce_ms: 0,
        fall_debounce_ms: 0,
    }
}

#[test]
fn cumulative_transition_uses_the_first_sample_only_as_baseline() {
    let spec = DefinitionSpec {
        kind: SemanticKind::CumulativeCounter,
        scale: 1.0,
        offset: 0.0,
        detector: boolean_detector(),
        trigger: TriggerMode::OnTransition,
    };
    let mut state = EvaluationState::default();
    let mut values = Vec::new();
    for (time, input) in [0.0, 1.0, 1.0, 0.0, 1.0].into_iter().enumerate() {
        let (result, next) = evaluate_at(spec, state, input, time as i64).expect("evaluate");
        state = next;
        values.extend(result.integer);
    }
    assert_eq!(values, [1, 2]);
}

#[test]
fn cumulative_notification_counts_each_active_sample_after_baseline() {
    let spec = DefinitionSpec {
        kind: SemanticKind::CumulativeCounter,
        scale: 1.0,
        offset: 0.0,
        detector: Detector {
            mode: DetectorMode::HighActive,
            rise_threshold: 40.0,
            fall_threshold: 39.0,
            ..Detector::default()
        },
        trigger: TriggerMode::OnNotification,
    };
    let mut state = EvaluationState::default();
    let mut values = Vec::new();
    for (time, input) in [43.0, 44.5, 45.0, 46.5].into_iter().enumerate() {
        let (result, next) = evaluate_at(spec, state, input, time as i64).expect("evaluate");
        state = next;
        values.extend(result.integer);
    }
    assert_eq!(values, [1, 2, 3]);
}

#[test]
fn high_active_detector_applies_hysteresis_and_independent_debounce() {
    let spec = DefinitionSpec {
        kind: SemanticKind::Boolean,
        scale: 1.0,
        offset: 0.0,
        detector: Detector {
            mode: DetectorMode::HighActive,
            rise_threshold: 10.0,
            fall_threshold: 4.0,
            rise_debounce_ms: 2_000,
            fall_debounce_ms: 3_000,
        },
        trigger: TriggerMode::None,
    };
    let (_, state) = evaluate_at(spec, EvaluationState::default(), 0.0, 1_000).expect("baseline");
    let (pending, state) = evaluate_at(spec, state, 11.0, 2_000).expect("rise starts");
    assert!(!pending.emitted && state.pending);
    let (rise, state) = evaluate_at(spec, state, 11.0, 4_000).expect("rise confirms");
    assert_eq!(rise.boolean, Some(true));
    let (_, state) = evaluate_at(spec, state, 3.0, 5_000).expect("fall starts");
    let (fall, _) = evaluate_at(spec, state, 3.0, 8_000).expect("fall confirms");
    assert_eq!(fall.boolean, Some(false));
}

#[test]
fn preview_downsamples_after_evaluation_without_losing_spikes_or_counts() {
    let mut inputs: Vec<_> = (0..1_000)
        .map(|index| PreviewInput {
            received_at: index,
            observed_at: None,
            value: (index % 10) as f64,
        })
        .collect();
    inputs[517].value = 1_000.0;
    let preview = build_preview(
        DefinitionSpec {
            kind: SemanticKind::Numeric,
            scale: 2.0,
            offset: 1.0,
            detector: Detector::default(),
            trigger: TriggerMode::None,
        },
        &inputs,
        300,
        None,
    )
    .expect("preview");
    assert_eq!(preview.input_count, 1_000);
    assert_eq!(preview.plot_count, preview.points.len());
    assert!(preview.points.len() <= 300);
    assert!(
        preview
            .points
            .iter()
            .any(|point| point.input_max == 1_000.0 && point.calibrated_max == 2_001.0)
    );
}

#[test]
fn calibration_rejects_intermediate_overflow_and_rules_use_calibrated_input_once() {
    assert!(
        Calibration {
            scale: 2.0,
            offset: -f64::MAX
        }
        .apply(f64::MAX)
        .is_err()
    );
    let (result, _) = evaluate_rule(
        RuleSpec {
            kind: SemanticKind::Numeric,
            detector: Detector::default(),
            trigger: TriggerMode::None,
        },
        EvaluationState::default(),
        21.5,
        1,
    )
    .expect("evaluate calibrated rule");
    assert_eq!(result.number, Some(21.5));
}

#[test]
fn canonical_detector_defaults_decode_without_repeating_zero_fields() {
    let numeric: DefinitionSpec =
        serde_json::from_str(r#"{"kind":"numeric","scale":1,"detector":{},"trigger":""}"#)
            .expect("numeric defaults");
    numeric.validate().expect("valid numeric defaults");
    let boolean: RuleSpec = serde_json::from_str(
        r#"{"kind":"boolean","detector":{"mode":"boolean_high_active"},"trigger":""}"#,
    )
    .expect("boolean detector defaults");
    assert_eq!(boolean.detector.rise_debounce_ms, 0);
    assert_eq!(boolean.detector.fall_threshold, 0.0);
}
