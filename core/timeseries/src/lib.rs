//! iotkit-core-timeseries: sensor reading persistence (INSERT/query/delete).

mod error;
mod model;

pub use error::TimeseriesError;
pub use model::{ReadingRow, TimeRange};

use iotkit_core_storage::Migration;

/// Timeseries migrations. Append to core/storage MIGRATIONS when assembling.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 2,
    label: "timeseries",
    sql: include_str!("../migrations/0002_timeseries.sql"),
}];
