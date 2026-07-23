use super::*;

#[test]
fn encryption_capacity_includes_container_overhead() {
    assert!(matches!(
        ensure_encryption_capacity(256 * 1024, 256 * 1024),
        Err(BackupError::InsufficientCapacity)
    ));
    assert!(ensure_encryption_capacity(384 * 1024, 256 * 1024).is_ok());
}
