use serde::{Deserialize, Serialize};

use super::Calibration;

const MAX_DEBOUNCE_MS: i64 = 300_000;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticKind {
    Numeric,
    Boolean,
    CumulativeCounter,
    Alarm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectorMode {
    BooleanHighActive,
    BooleanLowActive,
    HighActive,
    LowActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerMode {
    OnTransition,
    OnNotification,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detector {
    pub mode: DetectorMode,
    pub rise_threshold: f64,
    pub fall_threshold: f64,
    pub rise_debounce_ms: i64,
    pub fall_debounce_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionSpec {
    pub kind: SemanticKind,
    pub scale: f64,
    pub offset: f64,
    pub detector: Option<Detector>,
    pub trigger: Option<TriggerMode>,
}

impl DefinitionSpec {
    pub fn validate(self) -> Result<(), SemanticError> {
        Calibration {
            scale: self.scale,
            offset: self.offset,
        }
        .validate()?;
        match self.kind {
            SemanticKind::Numeric if self.detector.is_none() && self.trigger.is_none() => Ok(()),
            SemanticKind::Boolean | SemanticKind::Alarm
                if self.detector.is_some() && self.trigger.is_none() =>
            {
                self.detector.expect("checked").validate()
            }
            SemanticKind::CumulativeCounter
                if self.detector.is_some() && self.trigger.is_some() =>
            {
                self.detector.expect("checked").validate()
            }
            _ => Err(SemanticError::Invalid(
                "semantic kind, detector, and trigger are inconsistent".into(),
            )),
        }
    }
}

impl Detector {
    fn validate(self) -> Result<(), SemanticError> {
        if !self.rise_threshold.is_finite()
            || !self.fall_threshold.is_finite()
            || !(0..=MAX_DEBOUNCE_MS).contains(&self.rise_debounce_ms)
            || !(0..=MAX_DEBOUNCE_MS).contains(&self.fall_debounce_ms)
        {
            return Err(SemanticError::Invalid(
                "detector thresholds or debounce are invalid".into(),
            ));
        }
        if matches!(
            self.mode,
            DetectorMode::HighActive | DetectorMode::LowActive
        ) && self.fall_threshold > self.rise_threshold
        {
            return Err(SemanticError::Invalid(
                "fall threshold cannot exceed rise threshold".into(),
            ));
        }
        Ok(())
    }
}

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

pub fn evaluate_at(
    spec: DefinitionSpec,
    mut state: EvaluationState,
    input: f64,
    received_at: i64,
) -> Result<(Evaluation, EvaluationState), SemanticError> {
    spec.validate()?;
    if received_at < 0 {
        return Err(SemanticError::Invalid(
            "received time must be non-negative".into(),
        ));
    }
    let calibrated = Calibration {
        scale: spec.scale,
        offset: spec.offset,
    }
    .apply(input)?;
    let mut result = Evaluation::silent(calibrated);
    if spec.kind == SemanticKind::Numeric {
        result.emitted = true;
        result.number = Some(calibrated);
        state.initialized = true;
        return Ok((result, state));
    }

    let detector = spec.detector.expect("validated detector");
    let candidate = detector_active(detector, state, calibrated)?;
    if !state.initialized {
        state.initialized = true;
        state.active = candidate;
        if matches!(spec.kind, SemanticKind::Boolean | SemanticKind::Alarm) {
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
            state.pending_since = received_at;
            return Ok((result, state));
        } else if received_at - state.pending_since < debounce {
            return Ok((result, state));
        } else {
            state.active = candidate;
            state.pending = false;
        }
    }

    match spec.kind {
        SemanticKind::Boolean | SemanticKind::Alarm if previous != state.active => {
            result.emitted = true;
            result.boolean = Some(state.active);
        }
        SemanticKind::CumulativeCounter => {
            let increments = match spec.trigger.expect("validated trigger") {
                TriggerMode::OnNotification => state.active,
                TriggerMode::OnTransition => !previous && state.active,
            };
            if increments {
                if state.counter >= MAX_SAFE_INTEGER {
                    return Err(SemanticError::Invalid(
                        "cumulative counter reached safe integer limit".into(),
                    ));
                }
                state.counter += 1;
                result.emitted = true;
                result.integer = Some(state.counter);
            }
        }
        _ => {}
    }
    Ok((result, state))
}

fn detector_active(
    detector: Detector,
    state: EvaluationState,
    value: f64,
) -> Result<bool, SemanticError> {
    match detector.mode {
        DetectorMode::BooleanHighActive | DetectorMode::BooleanLowActive => {
            if value != 0.0 && value != 1.0 {
                return Err(SemanticError::Invalid(
                    "boolean input must be 0 or 1 after calibration".into(),
                ));
            }
            Ok((value == 1.0) == (detector.mode == DetectorMode::BooleanHighActive))
        }
        DetectorMode::HighActive => Ok(if state.initialized && state.active {
            value > detector.fall_threshold
        } else {
            value >= detector.rise_threshold
        }),
        DetectorMode::LowActive => Ok(if state.initialized && state.active {
            value < detector.rise_threshold
        } else {
            value <= detector.fall_threshold
        }),
    }
}

fn transition_debounce(detector: Detector, target_active: bool) -> i64 {
    let rising_signal = if matches!(
        detector.mode,
        DetectorMode::LowActive | DetectorMode::BooleanLowActive
    ) {
        !target_active
    } else {
        target_active
    };
    if rising_signal {
        detector.rise_debounce_ms
    } else {
        detector.fall_debounce_ms
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SemanticError {
    #[error("invalid semantic configuration: {0}")]
    Invalid(String),
}
