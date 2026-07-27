use iotkit_edge::web::{
    ConsoleBinding, ConsoleOutput,
    console::output::{apply_destination_state, binding_state, summarize},
};

#[test]
fn binding_priority_preserves_actionable_configuration_and_delivery_states() {
    let cases = [
        (
            "preview failure wins over a delivery state",
            true,
            true,
            false,
            true,
            Some("possible_delivery_stall"),
            true,
            "変換エラー",
            "error",
            true,
            true,
            false,
            false,
        ),
        (
            "configuration keeps its actionable state",
            false,
            true,
            false,
            false,
            None,
            false,
            "設定が必要",
            "needs-action",
            false,
            true,
            false,
            false,
        ),
        (
            "stall wins over external-registration wait",
            true,
            false,
            false,
            true,
            Some("possible_delivery_stall"),
            false,
            "配送停止の可能性",
            "error",
            true,
            false,
            true,
            false,
        ),
        (
            "external-registration wait remains in configuration",
            true,
            false,
            false,
            true,
            None,
            false,
            "外部登録待ち",
            "needs-action",
            true,
            true,
            false,
            true,
        ),
        (
            "ineligible binding is muted",
            true,
            false,
            true,
            false,
            Some("published"),
            false,
            "対象外",
            "muted",
            false,
            false,
            false,
            false,
        ),
        (
            "active delivery is reported",
            true,
            false,
            false,
            false,
            Some("delivering"),
            false,
            "配送中",
            "delivering",
            true,
            false,
            false,
            false,
        ),
        (
            "published delivery is healthy",
            true,
            false,
            false,
            false,
            Some("published"),
            false,
            "正常に送信中",
            "healthy",
            true,
            false,
            false,
            false,
        ),
        (
            "active binding waits for its first value",
            true,
            false,
            false,
            false,
            None,
            false,
            "最初の値を待っています",
            "waiting",
            true,
            false,
            false,
            false,
        ),
        (
            "inactive binding waits to start",
            false,
            false,
            false,
            false,
            None,
            false,
            "開始待ち",
            "waiting",
            false,
            false,
            false,
            false,
        ),
    ];

    for (
        case,
        active,
        needs_configuration,
        ineligible,
        prepared,
        delivery_state,
        preview_failed,
        label,
        class_name,
        target,
        expected_needs_configuration,
        delivery_problem,
        waiting_registration,
    ) in cases
    {
        let state = binding_state(
            active,
            needs_configuration,
            ineligible,
            prepared,
            delivery_state,
            preview_failed,
        );
        assert_eq!(state.label, label, "{case}");
        assert_eq!(state.class_name, class_name, "{case}");
        assert_eq!(state.target, target, "{case}");
        assert_eq!(
            state.needs_configuration, expected_needs_configuration,
            "{case}"
        );
        assert_eq!(state.delivery_problem, delivery_problem, "{case}");
        assert_eq!(state.waiting_registration, waiting_registration, "{case}");
    }
}

#[test]
fn destination_state_aggregates_each_binding_and_keeps_flags_exclusive() {
    let mut output = ConsoleOutput {
        active: true,
        bindings: vec![
            ConsoleBinding {
                needs_configuration: true,
                pending_count: 3,
                oldest_pending_at: Some(300),
                last_published_at: Some(600),
                ..ConsoleBinding::default()
            },
            ConsoleBinding {
                target: true,
                delivery_problem: true,
                pending_count: 2,
                oldest_pending_at: Some(100),
                last_published_at: Some(900),
                ..ConsoleBinding::default()
            },
        ],
        ..ConsoleOutput::default()
    };

    apply_destination_state(&mut output);

    assert!(output.needs_configuration);
    assert!(!output.delivery_problem);
    assert_eq!(output.status_label, "設定が必要");
    assert_eq!(output.status_class, "needs-action");
    assert_eq!(output.target_count, 1);
    assert_eq!(output.pending_count, 5);
    assert_eq!(output.oldest_pending_at, Some(100));
    assert_eq!(output.last_published_at, Some(900));
}

#[test]
fn destination_summary_counts_live_destinations_once_by_actionability() {
    let mut configuration = ConsoleOutput {
        active: true,
        bindings: vec![ConsoleBinding {
            needs_configuration: true,
            ..ConsoleBinding::default()
        }],
        ..ConsoleOutput::default()
    };
    let mut delivery_problem = ConsoleOutput {
        active: true,
        bindings: vec![ConsoleBinding {
            delivery_problem: true,
            ..ConsoleBinding::default()
        }],
        ..ConsoleOutput::default()
    };
    let mut draining = ConsoleOutput {
        draining: true,
        ..ConsoleOutput::default()
    };
    let mut inactive = ConsoleOutput {
        bindings: vec![ConsoleBinding {
            needs_configuration: true,
            ..ConsoleBinding::default()
        }],
        ..ConsoleOutput::default()
    };

    for output in [
        &mut configuration,
        &mut delivery_problem,
        &mut draining,
        &mut inactive,
    ] {
        apply_destination_state(output);
    }

    let summary = summarize(&[configuration, delivery_problem, draining, inactive]);
    assert_eq!(summary.needs_configuration_count, 1);
    assert_eq!(summary.delivery_problem_count, 1);
    assert_eq!(summary.sending_count, 1);
}
