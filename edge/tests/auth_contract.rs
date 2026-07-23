use iotkit_edge::auth::password::{
    Password, PasswordCandidate, PasswordHash, hash_password, normalize_login_id, verify_password,
};

#[test]
fn password_policy_counts_unicode_codepoints() {
    assert!(Password::new("長い安全な秘密の合言葉です").is_ok());
    assert!(Password::new("short").is_err());
    assert!(Password::new("x".repeat(129)).is_err());
}

#[test]
fn login_ids_are_lowercase_and_closed_ascii() {
    assert_eq!(
        normalize_login_id("Plant.Admin").expect("valid login"),
        "plant.admin"
    );
    for invalid in [
        "ab",
        "  Plant.Admin  ",
        "white space",
        "管理者",
        "x@example.com",
        &"x".repeat(65),
    ] {
        assert!(normalize_login_id(invalid).is_err(), "{invalid:?}");
    }
}

#[test]
fn argon2id_hashes_use_the_pinned_parameters_and_verify() {
    let password = Password::new("correct horse battery staple").expect("valid password");
    let encoded = hash_password(&password).expect("hash password");
    let encoded_text = encoded.expose_secret();
    assert!(encoded_text.starts_with("$argon2id$v=19$m=65536,t=3,p=1$"));

    let verified = verify_password(&encoded, &password.candidate()).expect("verify password");
    assert!(verified.matches);
    assert!(!verified.needs_rehash);

    let wrong = PasswordCandidate::new("short");
    assert!(
        !verify_password(&encoded, &wrong)
            .expect("verify wrong password")
            .matches
    );
}

#[test]
fn password_verification_rejects_resource_exhaustion_parameters() {
    let excessive = PasswordHash::new(
        "$argon2id$v=19$m=1048576,t=3,p=1$c2FsdHNhbHRzYWx0c2FsdA$\
         aGFzaGhhc2hoYXNoaGFzaGhhc2hoYXNoaGFzaA",
    );
    let password = PasswordCandidate::new("correct horse battery staple");
    assert!(verify_password(&excessive, &password).is_err());
}
