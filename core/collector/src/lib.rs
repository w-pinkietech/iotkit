pub mod actor;
pub mod registry_policy;

pub use actor::{Collector, IngestRequest, MAX_ITEMS_PER_ENVELOPE, SubmitError};
pub use registry_policy::{PermissiveRegistry, RegistryPolicy, RegistryVerdict, is_series_level};
