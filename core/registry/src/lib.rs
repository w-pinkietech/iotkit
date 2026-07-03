//! iotkit-core-registry: D6測定レジストリ(標準語彙カタログ+現場レジストリ)。
//! 正本文書: docs/redesign/decisions/D6-measurement-registry.md
pub mod catalog;

pub use catalog::{Catalog, CatalogEntry, ChannelMode, Range, ValueType, standard_catalog};
