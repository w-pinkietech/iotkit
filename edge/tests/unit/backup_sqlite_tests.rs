use super::*;

#[test]
fn capacity_preflight_rejects_a_snapshot_larger_than_available_space() {
    assert!(matches!(
        ensure_snapshot_capacity(99, 100),
        Err(BackupError::InsufficientCapacity)
    ));
    assert!(ensure_snapshot_capacity(100, 100).is_ok());
}
