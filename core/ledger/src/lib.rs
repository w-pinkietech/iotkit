pub mod ids;
pub mod store;

pub use ids::SystemId;
pub use store::{
    CHANNEL_NA, DEFAULT_VARIANT, DeviceKind, DeviceRow, DeviceState, EventRow, LedgerError,
    NewDevice, ReplaceOutcome, SeriesMeta, SeriesRow, SightingRow, activate_device,
    approve_sighting, bump_generation, current_generation, ensure_series, find_alive_by_hardware_id,
    find_series_meta, get_device, insert_device, ledger_epoch, list_devices, list_recent_events,
    list_series_for_device, list_sightings, record_event, record_sighting,
    release_series_quarantine_for_key_checked, replace_hardware, retire_device,
    series_exists_for_key, set_calibration_review,
};

use iotkit_core_storage::Migration;

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 3,
        label: "ledger",
        sql: include_str!("../migrations/0003_ledger.sql"),
    },
    Migration {
        version: 5,
        label: "series_quarantine_reason",
        sql: include_str!("../migrations/0005_series_quarantine_reason.sql"),
    },
    Migration {
        version: 9,
        label: "calibration_review",
        sql: include_str!("../migrations/0009_calibration_review.sql"),
    },
];
