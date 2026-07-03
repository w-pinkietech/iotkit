pub mod actor;
pub mod registry_policy;

pub use actor::{Collector, CollectorClosed, IngestRequest, MAX_ITEMS_PER_ENVELOPE};
pub use registry_policy::{PermissiveRegistry, RegistryPolicy, RegistryVerdict};
