#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("unsupported event variant: {0}")]
    UnsupportedEvent(String),

    #[error("json encode error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("json decode error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("unknown envelope version: {0}")]
    UnknownVersion(u32),

    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(i64),

    #[error("invalid payload: {0}")]
    InvalidPayload(String),
}
