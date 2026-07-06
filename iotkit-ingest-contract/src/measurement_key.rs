pub const MAX_MEASUREMENT_KEY_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum MeasurementKeyError {
    Empty,
    TooLong {
        len: usize,
    },
    /// コロン等の禁止文字、大文字、セグメント先頭が英小文字でない、空セグメント
    InvalidSegment {
        segment: String,
    },
}

impl std::fmt::Display for MeasurementKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "measurement_key is empty"),
            Self::TooLong { len } => {
                write!(
                    f,
                    "measurement_key length {len} exceeds {MAX_MEASUREMENT_KEY_LEN}"
                )
            }
            Self::InvalidSegment { segment } => {
                write!(
                    f,
                    "invalid measurement_key segment '{segment}': expected [a-z][a-z0-9_]*"
                )
            }
        }
    }
}
impl std::error::Error for MeasurementKeyError {}

/// D6決定2: セグメント=[a-z][a-z0-9_]*、区切りドット、コロン禁止(charsetで排除)、上限64。
pub fn validate_measurement_key(key: &str) -> Result<(), MeasurementKeyError> {
    if key.is_empty() {
        return Err(MeasurementKeyError::Empty);
    }
    if key.len() > MAX_MEASUREMENT_KEY_LEN {
        return Err(MeasurementKeyError::TooLong { len: key.len() });
    }
    for seg in key.split('.') {
        let mut chars = seg.chars();
        let valid = matches!(chars.next(), Some('a'..='z'))
            && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'));
        if !valid {
            return Err(MeasurementKeyError::InvalidSegment {
                segment: seg.to_string(),
            });
        }
    }
    Ok(())
}

/// D1推奨プロファイル `sender_id + boot_epoch + 単調seq` の正準文字列形式。
pub fn external_envelope_id(sender_id: &str, boot_epoch: u64, seq: u64) -> String {
    format!("{sender_id}-{boot_epoch}-{seq}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_standard_and_custom_keys() {
        for k in [
            "temperature_c",
            "voltage_mv",
            "custom.tank_level",
            "a",
            "x9_z.b_1",
        ] {
            assert!(validate_measurement_key(k).is_ok(), "{k} should be valid");
        }
    }

    #[test]
    fn rejects_colon_uppercase_and_bad_segments() {
        for k in [
            "custom:temp",
            "Temp",
            "9abc",
            "a..b",
            ".a",
            "a.",
            "",
            "温度",
        ] {
            assert!(
                validate_measurement_key(k).is_err(),
                "{k} should be invalid"
            );
        }
    }

    #[test]
    fn rejects_over_64_chars() {
        let k = "a".repeat(65);
        assert!(matches!(
            validate_measurement_key(&k),
            Err(MeasurementKeyError::TooLong { .. })
        ));
    }

    #[test]
    fn envelope_id_recipe_is_stable() {
        assert_eq!(external_envelope_id("dev1", 3, 42), "dev1-3-42");
    }
}
