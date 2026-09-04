use serde::{Deserialize, Serialize};

use crate::evaluator::EvaluatorError;

/// Linear calibration applied to the raw input before evaluation:
/// `value = input * scale + offset`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Calibration {
    pub scale: f64,
    pub offset: f64,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset: 0.0,
        }
    }
}

impl Calibration {
    pub fn validate(self) -> Result<(), EvaluatorError> {
        if !self.scale.is_finite() || self.scale == 0.0 {
            return Err(EvaluatorError::Invalid(
                "calibration scale must be finite and non-zero".into(),
            ));
        }
        if !self.offset.is_finite() {
            return Err(EvaluatorError::Invalid(
                "calibration offset must be finite".into(),
            ));
        }
        Ok(())
    }

    pub fn apply(self, input: f64) -> Result<f64, EvaluatorError> {
        self.validate()?;
        if !input.is_finite() {
            return Err(EvaluatorError::Invalid("input must be finite".into()));
        }
        let value = input * self.scale + self.offset;
        if !value.is_finite() {
            return Err(EvaluatorError::Invalid(
                "calibrated input must be finite".into(),
            ));
        }
        Ok(value)
    }
}
