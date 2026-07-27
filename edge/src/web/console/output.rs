//! Presentation-only output delivery state classification.

use crate::web::{ConsoleOutput, ConsoleOutputSummary};

#[derive(Clone, Debug)]
pub struct ConsoleBindingState {
    pub label: &'static str,
    pub class_name: &'static str,
    pub target: bool,
    pub needs_configuration: bool,
    pub delivery_problem: bool,
    pub waiting_registration: bool,
}

pub fn binding_state(
    active: bool,
    needs_configuration: bool,
    ineligible: bool,
    prepared: bool,
    delivery_state: Option<&str>,
    preview_failed: bool,
    delivery_unavailable: bool,
) -> ConsoleBindingState {
    if preview_failed {
        state("変換エラー", "error", true, true, false, false)
    } else if needs_configuration {
        state("設定が必要", "needs-action", false, true, false, false)
    } else if delivery_unavailable {
        state(
            "配送状態を確認できません",
            "error",
            true,
            false,
            true,
            false,
        )
    } else if delivery_state == Some("possible_delivery_stall") {
        state("配送停止の可能性", "error", true, false, true, false)
    } else if prepared {
        state("外部登録待ち", "needs-action", true, false, false, true)
    } else if ineligible {
        state("対象外", "muted", false, false, false, false)
    } else if active && delivery_state == Some("delivering") {
        state("配送中", "delivering", true, false, false, false)
    } else if active && delivery_state == Some("published") {
        state("正常に送信中", "healthy", true, false, false, false)
    } else if active {
        state(
            "最初の値を待っています",
            "waiting",
            true,
            false,
            false,
            false,
        )
    } else {
        state("開始待ち", "waiting", false, false, false, false)
    }
}

fn state(
    label: &'static str,
    class_name: &'static str,
    target: bool,
    needs_configuration: bool,
    delivery_problem: bool,
    waiting_registration: bool,
) -> ConsoleBindingState {
    ConsoleBindingState {
        label,
        class_name,
        target,
        needs_configuration,
        delivery_problem,
        waiting_registration,
    }
}

pub fn apply_destination_state(output: &mut ConsoleOutput) {
    output.target_count = output
        .bindings
        .iter()
        .filter(|binding| binding.target)
        .count();
    output.pending_count = output
        .bindings
        .iter()
        .map(|binding| binding.pending_count)
        .sum();
    output.oldest_pending_at = output
        .bindings
        .iter()
        .filter_map(|binding| binding.oldest_pending_at)
        .min();
    output.last_published_at = output
        .bindings
        .iter()
        .filter_map(|binding| binding.last_published_at)
        .max();

    output.needs_configuration = output
        .bindings
        .iter()
        .any(|binding| binding.needs_configuration);
    output.delivery_problem = !output.needs_configuration
        && output
            .bindings
            .iter()
            .any(|binding| binding.delivery_problem);
    output.delivery_unavailable = output.delivery_problem
        && output
            .bindings
            .iter()
            .any(|binding| binding.delivery_unavailable);
    output.waiting_registration = !output.needs_configuration
        && !output.delivery_problem
        && output
            .bindings
            .iter()
            .any(|binding| binding.waiting_registration);

    let (label, class_name) = if output.needs_configuration {
        ("設定が必要", "needs-action")
    } else if output.delivery_unavailable {
        ("配送状態を確認できません", "error")
    } else if output.delivery_problem {
        ("配送停止の可能性", "error")
    } else if output.waiting_registration {
        ("外部登録待ち", "needs-action")
    } else if output.draining {
        ("停止処理中", "draining")
    } else if output
        .bindings
        .iter()
        .any(|binding| binding.state_class == "delivering")
    {
        ("配送中", "delivering")
    } else if output.active {
        ("正常に送信中", "healthy")
    } else {
        ("開始待ち", "waiting")
    };
    output.status_label = label.into();
    output.status_class = class_name.into();
}

pub fn summarize(outputs: &[ConsoleOutput]) -> ConsoleOutputSummary {
    let mut summary = ConsoleOutputSummary::default();
    for output in outputs
        .iter()
        .filter(|output| output.active || output.draining)
    {
        if output.needs_configuration || output.waiting_registration {
            summary.needs_configuration_count += 1;
        } else if output.delivery_problem {
            summary.delivery_problem_count += 1;
        } else {
            summary.sending_count += 1;
        }
    }
    summary
}
