use std::fmt;

#[derive(Debug)]
pub enum EncodeError {
    /// Event type not supported for MQTT encoding (e.g. DeviceConfig)
    UnsupportedEvent(String),
    /// JSON serialization failed
    Json(serde_json::Error),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedEvent(msg) => write!(f, "unsupported event: {msg}"),
            Self::Json(e) => write!(f, "json encode: {e}"),
        }
    }
}

impl std::error::Error for EncodeError {}

impl From<serde_json::Error> for EncodeError {
    fn from(e: serde_json::Error) -> Self { Self::Json(e) }
}

#[derive(Debug)]
pub enum DecodeError {
    /// JSON deserialization failed
    Json(serde_json::Error),
    /// Unknown or unsupported envelope version
    UnknownVersion(u32),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "json decode: {e}"),
            Self::UnknownVersion(v) => write!(f, "unknown envelope version: {v}"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<serde_json::Error> for DecodeError {
    fn from(e: serde_json::Error) -> Self { Self::Json(e) }
}
