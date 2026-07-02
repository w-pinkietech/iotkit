pub mod ids;
pub mod store;

pub use ids::SystemId;
pub use store::{
    activate_device, approve_sighting, ensure_series, find_alive_by_hardware_id, insert_device,
    ledger_epoch, record_sighting, DeviceKind, DeviceRow, DeviceState, LedgerError, NewDevice,
};

use iotkit_core_storage::Migration;

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 3,
    label: "ledger",
    sql: include_str!("../migrations/0003_ledger.sql"),
}];
