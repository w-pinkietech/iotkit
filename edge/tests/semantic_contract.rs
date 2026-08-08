use iotkit_edge::semantics::{
    Calibration, DefinitionSpec, Detector, DetectorMode, EvaluationState, PreviewInput, RuleSpec,
    SemanticKind, TriggerMode, build_preview, build_preview_window, evaluate_at, evaluate_rule,
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
fn preview_window_keeps_state_history_but_plots_only_recent_points() {
    let mut inputs = Vec::with_capacity(2_000);
    inputs.push(PreviewInput {
        received_at: 0,
        observed_at: None,
        value: 0.0,
    });
    for index in 1..2_000 {
        inputs.push(PreviewInput {
            received_at: index * 1_000,
            observed_at: None,
            value: 2.0,
        });
    }
    let preview = build_preview_window(
        DefinitionSpec {
            kind: SemanticKind::CumulativeCounter,
            scale: 1.0,
            offset: 0.0,
            detector: Detector {
                mode: DetectorMode::HighActive,
                rise_threshold: 1.0,
                fall_threshold: 0.5,
                ..Detector::default()
            },
            trigger: TriggerMode::OnNotification,
        },
        &inputs,
        200,
        None,
        Some(1_940_000),
    )
    .expect("preview");

    assert_eq!(preview.input_count, 2_000);
    assert_eq!(preview.points.first().unwrap().received_at, 1_940_000);
    assert_eq!(preview.points.last().unwrap().received_at, 1_999_000);
    assert_eq!(preview.points.first().unwrap().counter, Some(1_940));
}

#[test]
fn preview_window_buckets_same_second_samples_after_full_history_evaluation() {
    let inputs = vec![
        PreviewInput {
            received_at: 0,
            observed_at: None,
            value: 0.0,
        },
        PreviewInput {
            received_at: 80_100,
            observed_at: None,
            value: 2.0,
        },
        PreviewInput {
            received_at: 80_900,
            observed_at: None,
            value: 4.0,
        },
        PreviewInput {
            received_at: 81_100,
            observed_at: None,
            value: 6.0,
        },
    ];
    let preview = build_preview_window(
        DefinitionSpec {
            kind: SemanticKind::CumulativeCounter,
            scale: 1.0,
            offset: 0.0,
            detector: Detector {
                mode: DetectorMode::HighActive,
                rise_threshold: 1.0,
                fall_threshold: 0.5,
                ..Detector::default()
            },
            trigger: TriggerMode::OnNotification,
        },
        &inputs,
        200,
        None,
        Some(80_000),
    )
    .expect("preview");

    assert_eq!(preview.input_count, 4);
    assert_eq!(preview.points.len(), 2);
    let first = preview.points.first().expect("first bucket");
    assert_eq!(first.received_at, 80_900);
    assert_eq!(first.plot_at, 80_000);
    assert_eq!(first.sample_count, 2);
    assert_eq!(first.input, 3.0);
    assert_eq!(first.input_min, 2.0);
    assert_eq!(first.input_max, 4.0);
    assert_eq!(first.counter, Some(2));
    assert_eq!(first.increment, 2);
    assert_eq!(preview.latest_point.unwrap().received_at, 81_100);
    assert_eq!(preview.latest_point.unwrap().plot_at, 81_100);
}

#[test]
fn preview_window_uses_absolute_second_boundaries_for_rolling_windows() {
    let inputs = vec![
        PreviewInput {
            received_at: 0,
            observed_at: None,
            value: 0.0,
        },
        PreviewInput {
            received_at: 80_300,
            observed_at: None,
            value: 2.0,
        },
        PreviewInput {
            received_at: 80_600,
            observed_at: None,
            value: 4.0,
        },
        PreviewInput {
            received_at: 80_900,
            observed_at: None,
            value: 6.0,
        },
        PreviewInput {
            received_at: 81_100,
            observed_at: None,
            value: 8.0,
        },
    ];
    let spec = DefinitionSpec {
        kind: SemanticKind::CumulativeCounter,
        scale: 1.0,
        offset: 0.0,
        detector: Detector {
            mode: DetectorMode::HighActive,
            rise_threshold: 1.0,
            fall_threshold: 0.5,
            ..Detector::default()
        },
        trigger: TriggerMode::OnNotification,
    };
    let preview = build_preview_window(spec, &inputs, 200, None, Some(80_250)).expect("preview");

    assert_eq!(
        preview
            .points
            .iter()
            .map(|point| point.plot_at)
            .collect::<Vec<_>>(),
        vec![80_000, 81_000]
    );
    let shifted =
        build_preview_window(spec, &inputs, 200, None, Some(80_500)).expect("shifted preview");
    assert_eq!(
        shifted
            .points
            .iter()
            .map(|point| point.plot_at)
            .collect::<Vec<_>>(),
        vec![80_000, 81_000]
    );
    let first = preview.points.first().expect("first bucket");
    let shifted_first = shifted.points.first().expect("shifted first bucket");
    assert_eq!(first.plot_at, 80_000);
    assert_eq!(first.sample_count, 3);
    assert_eq!(first.input_min, 2.0);
    assert_eq!(first.input_max, 6.0);
    assert_eq!(first.increment, 3);
    assert_eq!(
        (
            shifted_first.sample_count,
            shifted_first.input_min,
            shifted_first.input_max,
            shifted_first.increment,
        ),
        (
            first.sample_count,
            first.input_min,
            first.input_max,
            first.increment,
        )
    );
}

#[test]
fn preview_plot_uses_observed_time_for_batched_receipts() {
    let inputs: Vec<_> = (0..20)
        .map(|index| {
            let distance = if index < 10 { index } else { 19 - index };
            PreviewInput {
                received_at: 500_000,
                observed_at: Some(100_000 + index * 1_000),
                value: distance as f64,
            }
        })
        .collect();
    let preview = build_preview_window(
        DefinitionSpec {
            kind: SemanticKind::Numeric,
            scale: 1.0,
            offset: 0.0,
            detector: Detector::default(),
            trigger: TriggerMode::None,
        },
        &inputs,
        200,
        None,
        Some(60_000),
    )
    .expect("preview");

    assert_eq!(preview.input_count, 20);
    assert_eq!(preview.points.len(), 20);
    assert_eq!(preview.points.first().unwrap().received_at, 500_000);
    assert_eq!(preview.points.first().unwrap().plot_at, 100_000);
    assert_eq!(preview.points[9].input, 9.0);
    assert_eq!(preview.points.last().unwrap().received_at, 500_000);
    assert_eq!(preview.points.last().unwrap().plot_at, 119_000);
    assert_eq!(preview.points.last().unwrap().input, 0.0);
    assert_eq!(preview.latest_point.unwrap().received_at, 500_000);
    assert_eq!(preview.latest_point.unwrap().plot_at, 119_000);

    let safe_fallback = build_preview_window(
        DefinitionSpec {
            kind: SemanticKind::Numeric,
            scale: 1.0,
            offset: 0.0,
            detector: Detector::default(),
            trigger: TriggerMode::None,
        },
        &[
            PreviewInput {
                received_at: 500,
                observed_at: Some(0),
                value: 1.0,
            },
            PreviewInput {
                received_at: 600,
                observed_at: Some(-1),
                value: 2.0,
            },
        ],
        200,
        None,
        None,
    )
    .expect("fallback preview");
    assert_eq!(
        safe_fallback
            .points
            .iter()
            .map(|point| point.received_at)
            .collect::<Vec<_>>(),
        [500, 600]
    );

    let sorted = build_preview_window(
        DefinitionSpec {
            kind: SemanticKind::CumulativeCounter,
            scale: 1.0,
            offset: 0.0,
            detector: boolean_detector(),
            trigger: TriggerMode::OnTransition,
        },
        &[
            PreviewInput {
                received_at: 500_000,
                observed_at: Some(102_000),
                value: 1.0,
            },
            PreviewInput {
                received_at: 500_000,
                observed_at: Some(100_000),
                value: 0.0,
            },
            PreviewInput {
                received_at: 500_000,
                observed_at: Some(101_000),
                value: 1.0,
            },
        ],
        200,
        None,
        Some(99_000),
    )
    .expect("out-of-order preview");
    assert_eq!(
        sorted
            .points
            .iter()
            .map(|point| point.received_at)
            .collect::<Vec<_>>(),
        [500_000, 500_000, 500_000]
    );
    assert_eq!(
        sorted
            .points
            .iter()
            .map(|point| point.plot_at)
            .collect::<Vec<_>>(),
        [100_000, 101_000, 102_000]
    );
    let latest = sorted.latest_point.expect("latest evaluated point");
    assert_eq!(latest.received_at, 500_000);
    assert_eq!(latest.plot_at, 101_000);
    assert_eq!(latest.counter, Some(1));
    assert_eq!(latest.active, Some(true));
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
