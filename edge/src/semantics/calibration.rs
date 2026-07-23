use super::SemanticError;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Calibration {
    pub scale: f64,
    pub offset: f64,
}

impl Calibration {
    pub fn validate(self) -> Result<(), SemanticError> {
        if !self.scale.is_finite() || self.scale == 0.0 {
            return Err(SemanticError::Invalid(
                "calibration scale must be finite and non-zero".into(),
            ));
        }
        if !self.offset.is_finite() {
            return Err(SemanticError::Invalid(
                "calibration offset must be finite".into(),
            ));
        }
        Ok(())
    }

    pub fn apply(self, input: f64) -> Result<f64, SemanticError> {
        self.validate()?;
        if !input.is_finite() {
            return Err(SemanticError::Invalid("input must be finite".into()));
        }
        let value = input.mul_add(self.scale, self.offset);
        if !value.is_finite() {
            return Err(SemanticError::Invalid(
                "calibrated input must be finite".into(),
            ));
        }
        Ok(value)
    }
}
