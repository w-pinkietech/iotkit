#[cfg(target_os = "linux")]
#[test]
fn mqtt_password_reader_accepts_only_one_owner_only_regular_link() {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };

    let directory = tempfile::tempdir().unwrap();
    let password = directory.path().join("password");
    fs::write(&password, b"secret-value\n").unwrap();
    fs::set_permissions(&password, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(super::read_owner_only(&password).unwrap(), "secret-value");

    fs::set_permissions(&password, fs::Permissions::from_mode(0o640)).unwrap();
    assert!(super::read_owner_only(&password).is_err());
    fs::set_permissions(&password, fs::Permissions::from_mode(0o600)).unwrap();

    let link = directory.path().join("password-link");
    fs::hard_link(&password, &link).unwrap();
    assert!(super::read_owner_only(&password).is_err());
    fs::remove_file(&link).unwrap();

    let symlink_path = directory.path().join("password-symlink");
    symlink(&password, &symlink_path).unwrap();
    assert!(super::read_owner_only(&symlink_path).is_err());
}

#[test]
fn activation_can_resume_after_completion_was_durably_stored() {
    assert!(super::activation_mode_allowed(
        &iotkit_core_recovery::RecoveryStartupMode::Recovered {
            recovery_id: "recovery-0123456789abcdef0123456789abcdef".into(),
            candidate_instance_id: "candidate-0123456789abcdef0123456789abcdef".into(),
            new_ledger_epoch: "epoch-0123456789abcdef0123456789abcdef".into(),
        }
    ));
}

#[test]
fn only_the_completion_ack_puback_finishes_activation() {
    let mut tracker = super::PublishReceiptTracker::default();
    tracker.enqueued(super::LocalPublishKind::Result);
    tracker.enqueued(super::LocalPublishKind::Result);
    tracker.enqueued(super::LocalPublishKind::CompletionAck);

    tracker.outgoing(10).unwrap();
    tracker.outgoing(11).unwrap();
    assert!(!tracker.acknowledged(10));
    assert!(!tracker.acknowledged(11));

    tracker.outgoing(12).unwrap();
    assert!(tracker.acknowledged(12));
}
