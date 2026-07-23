/// All errors from iotkit-core-storage.
#[derive(Debug)]
pub enum StorageError {
    /// SQLite operation failure.
    Sqlite(rusqlite::Error),
    /// Filesystem error (e.g., parent directory missing).
    Io(std::io::Error),
    /// A specific migration failed.
    MigrationFailed {
        version: u32,
        source: Box<StorageError>,
    },
    /// On-disk schema is newer than this binary knows about.
    SchemaVersionAhead { on_disk: u32, latest_known: u32 },
    /// Migration versions are not strictly ascending.
    InvalidMigrationOrder { first: u32, second: u32 },
    /// Existing pre-release database predates the Edge Node identity cutover.
    UnsupportedPreReleaseEdgeDatabase,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::MigrationFailed { version, source } => {
                write!(f, "migration v{version} failed: {source}")
            }
            Self::SchemaVersionAhead {
                on_disk,
                latest_known,
            } => {
                write!(
                    f,
                    "schema version {on_disk} is ahead of latest known {latest_known}; upgrade the binary"
                )
            }
            Self::InvalidMigrationOrder { first, second } => {
                write!(
                    f,
                    "migration versions not strictly ascending: v{first} >= v{second}"
                )
            }
            Self::UnsupportedPreReleaseEdgeDatabase => write!(
                f,
                "unsupported pre-release Edge Node database; recreate the Edge Node database"
            ),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::MigrationFailed { source, .. } => Some(source.as_ref()),
            Self::SchemaVersionAhead { .. } => None,
            Self::InvalidMigrationOrder { .. } => None,
            Self::UnsupportedPreReleaseEdgeDatabase => None,
        }
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
