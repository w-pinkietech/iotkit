/// The maximum accepted UTF-8 byte length of a measurement key.
///
/// [`validate_measurement_key`] returns [`MeasurementKeyError::TooLong`] when a
/// key exceeds this limit.
pub const MAX_MEASUREMENT_KEY_LEN: usize = 64;

/// A deterministic measurement-key grammar violation.
///
/// Callers can use the variant to correct input before submitting an envelope.
#[derive(Debug, Clone, PartialEq)]
pub enum MeasurementKeyError {
    /// No key text was provided.
    ///
    /// Supply at least one valid segment before retrying the observation.
    Empty,
    /// The key exceeds [`MAX_MEASUREMENT_KEY_LEN`] UTF-8 bytes.
    ///
    /// Shorten the key while preserving the required segment grammar.
    TooLong {
        /// The actual UTF-8 byte length that was validated.
        len: usize,
    },
    /// A dot-delimited segment does not match `[a-z][a-z0-9_]*`.
    ///
    /// This includes empty segments, uppercase text, colons, and any segment that
    /// does not begin with an ASCII lowercase letter.
    InvalidSegment {
        /// The offending segment, or an empty string for an empty segment.
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

/// Validates the stable version-1 measurement-key grammar.
///
/// A key contains one or more dot-separated `[a-z][a-z0-9_]*` segments,
/// excludes colons through that character set, and is at most
/// [`MAX_MEASUREMENT_KEY_LEN`] UTF-8 bytes. Receivers apply this validation before
/// registry lookup.
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

/// Builds the recommended external-device envelope identifier.
///
/// The stable representation is `sender_id-boot_epoch-seq`. The boot epoch must
/// distinguish restarts and the sequence must increase within it; the sender
/// stores the resulting identifier with a spooled envelope and reuses it unchanged
/// for every retry.
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
