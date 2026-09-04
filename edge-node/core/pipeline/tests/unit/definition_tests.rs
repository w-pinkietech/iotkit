use super::*;

fn input() -> PipelineInput {
    PipelineInput {
        adapter: "trial_sample".into(),
        subject: None,
        measurement_key: "contact_state".into(),
        channel_index: None,
        value_index: 0,
    }
}

fn detector() -> Detector {
    Detector {
        mode: DetectorMode::HighActive,
        rise_threshold: 0.5,
        fall_threshold: 0.5,
        rise_debounce_ms: 0,
        fall_debounce_ms: 0,
    }
}

pub(crate) fn count_definition() -> PipelineDefinition {
    PipelineDefinition {
        id: "press-01-cycle-count".parse().unwrap(),
        kind: PipelineKind::AccumulatedCount,
        input: input(),
        trigger: Some(Trigger::OnTransition),
        unit: None,
        display_name: Some("Press 01 cycles".into()),
        calibration: Calibration::default(),
        detector: Some(detector()),
    }
}

pub(crate) fn measurement_definition() -> PipelineDefinition {
    PipelineDefinition {
        id: "press-01-temperature".parse().unwrap(),
        kind: PipelineKind::Measurement,
        input: PipelineInput {
            measurement_key: "illuminance_lux".into(),
            ..input()
        },
        trigger: None,
        unit: Some("lx".into()),
        display_name: None,
        calibration: Calibration::default(),
        detector: None,
    }
}

#[test]
fn valid_definitions_for_each_kind_pass() {
    count_definition().validate().unwrap();
    measurement_definition().validate().unwrap();
    let state = PipelineDefinition {
        id: "press-01-temperature-high".parse().unwrap(),
        kind: PipelineKind::State,
        trigger: None,
        display_name: None,
        ..count_definition()
    };
    state.validate().unwrap();
}

#[test]
fn kind_specific_fields_are_required_or_forbidden() {
    let mut measurement_with_detector = measurement_definition();
    measurement_with_detector.detector = Some(detector());
    assert!(measurement_with_detector.validate().is_err());

    let mut measurement_without_unit = measurement_definition();
    measurement_without_unit.unit = None;
    assert!(measurement_without_unit.validate().is_err());

    let mut count_without_trigger = count_definition();
    count_without_trigger.trigger = None;
    assert!(count_without_trigger.validate().is_err());

    let mut count_with_unit = count_definition();
    count_with_unit.unit = Some("count".into());
    assert!(count_with_unit.validate().is_err());

    let mut state_with_trigger = count_definition();
    state_with_trigger.kind = PipelineKind::State;
    assert!(state_with_trigger.validate().is_err());

    let mut state_without_detector = count_definition();
    state_without_detector.kind = PipelineKind::State;
    state_without_detector.trigger = None;
    state_without_detector.detector = None;
    assert!(state_without_detector.validate().is_err());
}

#[test]
fn detector_ranges_follow_the_contract() {
    let mut definition = count_definition();
    definition.detector = Some(Detector {
        rise_debounce_ms: 300_001,
        ..detector()
    });
    assert!(definition.validate().is_err());
    definition.detector = Some(Detector {
        fall_debounce_ms: -1,
        ..detector()
    });
    assert!(definition.validate().is_err());
    definition.detector = Some(Detector {
        rise_threshold: 1.0,
        fall_threshold: 2.0,
        ..detector()
    });
    assert!(definition.validate().is_err());
    definition.detector = Some(Detector {
        rise_threshold: f64::NAN,
        ..detector()
    });
    assert!(definition.validate().is_err());
    definition.detector = Some(Detector {
        rise_debounce_ms: 300_000,
        fall_debounce_ms: 300_000,
        ..detector()
    });
    definition.validate().unwrap();
}

#[test]
fn calibration_and_names_are_validated() {
    let mut definition = measurement_definition();
    definition.calibration = Calibration {
        scale: 0.0,
        offset: 0.0,
    };
    assert!(definition.validate().is_err());
    definition.calibration = Calibration::default();
    definition.display_name = Some("x".repeat(129));
    assert!(definition.validate().is_err());
    definition.display_name = Some("   ".into());
    assert!(definition.validate().is_err());
    definition.display_name = Some("あ".repeat(128));
    definition.validate().unwrap();
}

#[test]
fn structural_hash_ignores_tuning_and_tracks_structure() {
    let base = count_definition();
    let hash = base.structural_hash();
    assert_eq!(hash.len(), 64);

    let mut tuned = base.clone();
    tuned.display_name = Some("renamed".into());
    tuned.calibration = Calibration {
        scale: 2.0,
        offset: 1.0,
    };
    tuned.detector = Some(Detector {
        rise_threshold: 0.9,
        fall_threshold: 0.1,
        rise_debounce_ms: 100,
        fall_debounce_ms: 200,
        mode: DetectorMode::LowActive,
    });
    assert_eq!(
        tuned.structural_hash(),
        hash,
        "tuning items keep the series"
    );

    let mut other_input = base.clone();
    other_input.input.channel_index = Some(1);
    assert_ne!(other_input.structural_hash(), hash);

    let mut other_subject = base.clone();
    other_subject.input.subject = Some("dev:1".into());
    assert_ne!(other_subject.structural_hash(), hash);

    let mut other_id = base.clone();
    other_id.id = "press-02-cycle-count".parse().unwrap();
    assert_ne!(other_id.structural_hash(), hash);

    let mut other_unit = measurement_definition();
    let measurement_hash = other_unit.structural_hash();
    other_unit.unit = Some("mm".into());
    assert_ne!(other_unit.structural_hash(), measurement_hash);
}

#[test]
fn definition_json_round_trips_and_rejects_unknown_fields() {
    let definition = count_definition();
    let json = serde_json::to_string(&definition).unwrap();
    assert!(json.contains("\"kind\":\"accumulated-count\""));
    assert!(json.contains("\"trigger\":\"on-transition\""));
    assert!(json.contains("\"mode\":\"high-active\""));
    let decoded: PipelineDefinition = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, definition);

    assert!(
        serde_json::from_str::<PipelineDefinition>(&json.replace("\"kind\"", "\"kinds\"")).is_err()
    );
    assert!(
        serde_json::from_str::<PipelineDefinition>(&json.replace("press-01", "Press-01")).is_err()
    );
}

#[test]
fn input_matching_treats_subject_as_optional() {
    let any_subject = input();
    assert!(any_subject.matches("trial_sample", Some("ns:state"), "contact_state", None));
    assert!(any_subject.matches("trial_sample", None, "contact_state", None));
    assert!(!any_subject.matches("other", None, "contact_state", None));
    assert!(!any_subject.matches("trial_sample", None, "contact_state", Some(0)));

    let pinned = PipelineInput {
        subject: Some("ns:state".into()),
        ..input()
    };
    assert!(pinned.matches("trial_sample", Some("ns:state"), "contact_state", None));
    assert!(!pinned.matches("trial_sample", Some("ns:other"), "contact_state", None));
    assert!(!pinned.matches("trial_sample", None, "contact_state", None));
}
