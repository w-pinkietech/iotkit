pub mod ids;
pub mod store;

pub use ids::SystemId;
pub use store::{
    CHANNEL_NA, DEFAULT_VARIANT, DeviceKind, DeviceRow, DeviceState, EdgeIdentity, EventRow,
    LedgerError, NewDevice, ParsedSeriesKey, ReplaceOutcome, SeriesListRow, SeriesMeta, SeriesRow,
    SightingRow, activate_device, approve_sighting, bind_positional_model, bump_generation,
    current_generation, descriptor_revision, edge_node_id, ensure_series,
    expire_quarantined_devices, find_alive_by_hardware_id, find_series_by_key, find_series_meta,
    get_device, insert_device, is_valid_model_id, ledger_epoch, list_devices, list_recent_events,
    list_series, list_series_for_device, list_sightings, load_edge_identity, parse_series_key,
    positional_model_id, purge_sightings, record_event, record_sighting,
    release_series_quarantine_for_key_checked, renew_epoch, replace_hardware, retire_device,
    series_exists_for_key, series_key_of, set_calibration_review, set_presentation_identifier,
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
    Migration {
        version: 11,
        label: "sightings_bound",
        sql: include_str!("../migrations/0011_sightings_bound.sql"),
    },
    Migration {
        version: 18,
        label: "descriptor_metadata",
        sql: include_str!("../migrations/0018_descriptor_metadata.sql"),
    },
    Migration {
        version: 21,
        label: "positional_device_model",
        sql: include_str!("../migrations/0021_positional_device_model.sql"),
    },
    Migration {
        version: 22,
        label: "descriptor_device_model",
        sql: include_str!("../migrations/0022_descriptor_device_model.sql"),
    },
];
