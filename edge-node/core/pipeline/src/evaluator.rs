//! Evaluation semantics moved from the IoTKit Edge `semantics/evaluator.rs`:
//! calibration, thresholding with hysteresis, debounce, and accumulated
//! counting. Pure functions over owned state so that the store can persist
//! the state in the same transaction as the outbox row.

use crate::definition::{Detector, DetectorMode, PipelineDefinition, PipelineKind, Trigger};

/// Largest exactly representable integer of a JSON number consumer (2^53 - 1).
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EvaluationState {
    pub initialized: bool,
    pub active: bool,
    pub counter: i64,
    pub pending: bool,
    pub pending_active: bool,
    pub pending_since: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Evaluation {
    pub emitted: bool,
    pub number: Option<f64>,
    pub boolean: Option<bool>,
    pub integer: Option<i64>,
    pub calibrated: f64,
}

impl Evaluation {
    fn silent(calibrated: f64) -> Self {
        Self {
            emitted: false,
            number: None,
            boolean: None,
            integer: None,
            calibrated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvaluatorError {
    #[error("invalid evaluation input: {0}")]
    Invalid(String),
    #[error("accumulated count reached the safe integer limit")]
    CounterLimit,
}

/// Evaluates one input at `received_at` (Unix epoch ms).
///
/// - `measurement` emits the calibrated value on every input; the engine
///   decides whether the value changed since the last publication.
/// - `state` emits the detector state on the first input and then only on a
///   confirmed transition.
/// - `accumulated-count` uses the first input as the baseline and emits the
///   counter each time the trigger fires.
pub fn evaluate(
    definition: &PipelineDefinition,
    mut state: EvaluationState,
    input: f64,
    received_at: i64,
) -> Result<(Evaluation, EvaluationState), EvaluatorError> {
    if received_at < 0 {
        return Err(EvaluatorError::Invalid(
            "received time must be non-negative".into(),
        ));
    }
    let calibrated = definition.calibration.apply(input)?;
    let mut result = Evaluation::silent(calibrated);
    if definition.kind == PipelineKind::Measurement {
        result.emitted = true;
        result.number = Some(calibrated);
        state.initialized = true;
        return Ok((result, state));
    }

    let detector = definition.detector.ok_or_else(|| {
        EvaluatorError::Invalid("detector is required for state and accumulated-count".into())
    })?;
    let candidate = detector_active(detector, state, calibrated);
    if !state.initialized {
        state.initialized = true;
        state.active = candidate;
        if definition.kind == PipelineKind::State {
            result.emitted = true;
            result.boolean = Some(candidate);
        }
        return Ok((result, state));
    }
    let previous = state.active;
    if candidate == state.active {
        state.pending = false;
    } else {
        let debounce = transition_debounce(detector, candidate);
        if debounce == 0 {
            state.active = candidate;
            state.pending = false;
        } else if !state.pending || state.pending_active != candidate {
            state.pending = true;
            state.pending_active = candidate;
            state.pending_since = received_at;
            return Ok((result, state));
        } else if received_at < state.pending_since {
            // Clock went backwards: restart the debounce window from here.
            state.pending_since = received_at;
            return Ok((result, state));
        } else if received_at - state.pending_since < debounce {
            return Ok((result, state));
        } else {
            state.active = candidate;
            state.pending = false;
        }
    }

    match definition.kind {
        PipelineKind::State => {
            if previous != state.active {
                result.emitted = true;
                result.boolean = Some(state.active);
            }
        }
        PipelineKind::AccumulatedCount => {
            let increments = match definition.trigger {
                Some(Trigger::OnTransition) => !previous && state.active,
                None => {
                    return Err(EvaluatorError::Invalid(
                        "trigger is required for accumulated-count".into(),
                    ));
                }
            };
            if increments {
                if state.counter >= MAX_SAFE_INTEGER {
                    return Err(EvaluatorError::CounterLimit);
                }
                state.counter += 1;
                result.emitted = true;
                result.integer = Some(state.counter);
            }
        }
        PipelineKind::Measurement => unreachable!("handled above"),
    }
    Ok((result, state))
}

fn detector_active(detector: Detector, state: EvaluationState, value: f64) -> bool {
    match detector.mode {
        DetectorMode::HighActive => {
            if state.initialized && state.active {
                value > detector.fall_threshold
            } else {
                value >= detector.rise_threshold
            }
        }
        DetectorMode::LowActive => {
            if state.initialized && state.active {
                value < detector.rise_threshold
            } else {
                value <= detector.fall_threshold
            }
        }
    }
}

fn transition_debounce(detector: Detector, target_active: bool) -> i64 {
    // For low-active detectors the physical signal rises when the state
    // becomes inactive, so the rise debounce applies to that direction.
    let rising_signal = match detector.mode {
        DetectorMode::HighActive => target_active,
        DetectorMode::LowActive => !target_active,
    };
    if rising_signal {
        detector.rise_debounce_ms
    } else {
        detector.fall_debounce_ms
    }
}

#[cfg(test)]
#[path = "../tests/unit/evaluator_tests.rs"]
mod tests;
