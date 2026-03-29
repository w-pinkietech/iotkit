//! iotkit-core-storage: SQLite persistence infrastructure for the IoT gateway.

mod error;
mod handle;
mod migrate;

pub use error::StorageError;
pub use handle::DbHandle;
