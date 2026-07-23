use iotkit_core_storage::StorageError;

/// Errors from timeseries operations.
#[derive(Debug)]
pub enum TimeseriesError {
    /// Invalid reading data (NaN, Inf, pre-epoch timestamp, invalid range, etc.)
    InvalidReading(String),
    /// A finite staging/dedup limit or pin reserve would be violated.
    Limit(String),
    /// Underlying storage error.
    Storage(StorageError),
}

impl std::fmt::Display for TimeseriesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidReading(msg) => write!(f, "invalid reading: {msg}"),
            Self::Limit(msg) => write!(f, "configured retention limit: {msg}"),
            Self::Storage(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TimeseriesError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidReading(_) => None,
            Self::Limit(_) => None,
            Self::Storage(e) => Some(e),
        }
    }
}

impl From<StorageError> for TimeseriesError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for TimeseriesError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(StorageError::Sqlite(e))
    }
}
