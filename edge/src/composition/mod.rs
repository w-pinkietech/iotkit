mod output_adapters;
pub mod runtime;
pub mod runtime_config;
mod web;

pub use output_adapters::{OutputAdapterRegistration, registered_output_adapters};
pub use web::{LoginPolicy, StorageWebApplication};
