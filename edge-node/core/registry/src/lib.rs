//! iotkit-core-registry: D6測定レジストリ(標準語彙カタログ+現場レジストリ)。
//! 正本文書: docs/redesign/decisions/D6-measurement-registry.md
pub mod catalog;
pub mod policy;
pub mod store;

pub use catalog::{Catalog, CatalogEntry, ChannelMode, Range, ValueType, standard_catalog};
pub use policy::SqliteRegistry;
pub use store::{
    AliasKind, AliasRow, CustomEntrySpec, EntryRow, LEGACY_SENSOR_MAP, RegistryError, Resolution,
    define_alias, define_custom_entry, enable_entry, find_resolution, get_entry, list_aliases,
    list_entries, lookup_legacy, seed_legacy_sensor_map, validate_custom_entry_spec,
    validate_measurement_key,
};

use iotkit_core_storage::Migration;

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 6,
        label: "registry",
        sql: include_str!("../migrations/0006_registry.sql"),
    },
    Migration {
        version: 19,
        label: "descriptor_revision",
        sql: include_str!("../migrations/0019_descriptor_revision.sql"),
    },
];

#[cfg(test)]
#[path = "../tests/unit/migration_tests.rs"]
mod migration_tests;
