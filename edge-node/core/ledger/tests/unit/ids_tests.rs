use super::*;

#[test]
fn generate_produces_unique_ids_and_text_round_trip() {
    let a = SystemId::generate();
    let b = SystemId::generate();
    assert_ne!(a, b);
    let text = a.to_text();
    assert_eq!(text.len(), 36);
    assert_eq!(SystemId::from_text(&text).unwrap(), a);
}

#[test]
fn from_text_rejects_garbage() {
    assert!(SystemId::from_text("not-a-uuid").is_err());
}
