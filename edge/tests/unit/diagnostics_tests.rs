use super::*;

#[test]
fn certificate_view_marks_expired_and_near_expiry_certificates() {
    let day = 24 * 60 * 60 * 1000;
    assert!(certificate_status(10 * day, 11 * day).needs_action);
    assert!(certificate_status(29 * day, 0).needs_action);
    assert!(!certificate_status(30 * day, 0).needs_action);
    assert_eq!(
        certificate_status(10 * day, 11 * day).days_remaining,
        Some(-1)
    );
}
