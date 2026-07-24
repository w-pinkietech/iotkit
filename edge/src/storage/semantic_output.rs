// Keep the public storage operations and types easy to scan while the
// backend-specific SQL remains mechanically separate.
include!("semantic_output/operations.rs");
include!("semantic_output/common.rs");
include!("semantic_output/sqlite.rs");
include!("semantic_output/postgres.rs");
